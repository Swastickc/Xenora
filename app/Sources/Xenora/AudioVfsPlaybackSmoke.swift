// SPDX-License-Identifier: GPL-3.0-or-later

import CXenora
import Foundation

/// The injected capture and player suspend until the smoke finishes each
/// operation, so overlap and retry checks need no sleeps or audio device.
private actor AudioVfsPlaybackProbe {
    enum Event: Equatable, Sendable {
        case capture
        case listen([UInt8])
    }

    struct OutputFailure: LocalizedError {
        var errorDescription: String? { "The audio output is unavailable." }
    }

    let stream: AsyncStream<Event>.Continuation
    private(set) var calls: [Event] = []
    private var captureResult: CheckedContinuation<AudioVfsFrame, Never>?
    private var playbackResult: CheckedContinuation<Void, Error>?

    init(stream: AsyncStream<Event>.Continuation) {
        self.stream = stream
    }

    func capture() async -> AudioVfsFrame {
        await withCheckedContinuation {
            captureResult = $0
            record(.capture)
        }
    }

    func listen(_ pcm: [UInt8]) async throws {
        try await withCheckedThrowingContinuation {
            playbackResult = $0
            record(.listen(pcm))
        }
    }

    func finishCapture(_ frame: AudioVfsFrame) {
        captureResult?.resume(returning: frame)
        captureResult = nil
    }

    func finishListen(failing: Bool = false) {
        if failing {
            playbackResult?.resume(throwing: OutputFailure())
        } else {
            playbackResult?.resume()
        }
        playbackResult = nil
    }

    private func record(_ event: Event) {
        calls.append(event)
        stream.yield(event)
    }
}

@MainActor
func checkAudioVfsPlayback() async -> [String] {
    let (stream, continuation) = AsyncStream.makeStream(of: AudioVfsPlaybackProbe.Event.self)
    var events = stream.makeAsyncIterator()
    let probe = AudioVfsPlaybackProbe(stream: continuation)
    let model = AudioVfsPlaybackModel(
        capture: { await probe.capture() },
        playPreview: { try await probe.listen($0) }
    )
    let pcm = [UInt8](repeating: 0x25, count: 512)
    let rgba: [UInt8] = [255, 128, 0, 255, 128, 0, 255, 255, 0, 255, 128, 255, 255, 0, 128, 255]
    func frame(pcm: [UInt8] = [], captureError: String? = nil, status: Int32 = Int32(XENORA_OK)) -> AudioVfsFrame {
        AudioVfsFrame(status: status, width: 2, height: 2, samples: 256, totalAudioBytes: 512,
                      rgba: rgba, pcm: pcm, audioCaptureError: captureError)
    }

    var failures: [String] = []
    func expect(_ condition: Bool, _ message: String) {
        if !condition { failures.append(message) }
    }

    await model.listen()
    expect(await probe.calls.isEmpty, "Listen started without a captured sample")

    let run = Task { await model.run() }
    expect(await events.next() == .capture, "fixture capture did not start")
    expect(model.phase == .capturing && !model.canRun && !model.canListen, "capture did not occupy the model")
    await model.run()
    await model.listen()
    expect(await probe.calls == [.capture], "another action began during capture")
    await probe.finishCapture(frame(pcm: pcm))
    await run.value
    expect(model.canListen && model.frame?.rgba == rgba && model.captureFailure == nil, "capture did not enable Listen")
    expect(await probe.calls == [.capture], "capturing audio started playback")

    let listen = Task { await model.listen() }
    expect(await events.next() == .listen(pcm), "playback received the wrong PCM")
    expect(model.phase == .playing && !model.canRun && !model.canListen, "playback did not occupy the model")
    await model.listen()
    await model.run()
    expect(await probe.calls == [.capture, .listen(pcm)], "another action began during playback")
    await probe.finishListen(failing: true)
    await listen.value
    expect(model.playbackResult == .failed("The audio output is unavailable.") && model.canListen
           && model.frame?.rgba == rgba, "an output failure hid the frame or disabled retry")

    let retry = Task { await model.listen() }
    expect(await events.next() == .listen(pcm), "retry did not reuse the captured sample")
    await probe.finishListen()
    await retry.value
    expect(model.canListen && model.playbackResult == .completed, "successful retry did not clear the output failure")

    let rerun = Task { await model.run() }
    expect(await events.next() == .capture, "fixture could not rerun after playback")
    expect(model.frame == nil && model.playbackResult == nil && !model.canListen, "rerun left the old sample active")
    await probe.finishCapture(frame(captureError: "The fixture audio sample was incomplete."))
    await rerun.value
    expect(model.captureFailure == "The fixture audio sample was incomplete."
           && model.frame?.image != nil && !model.canListen, "a PCM capture failure hid the valid frame or allowed playback")

    let failedRun = Task { await model.run() }
    expect(await events.next() == .capture, "fixture could not rerun after capture failure")
    await probe.finishCapture(frame(pcm: pcm, status: Int32(XENORA_EXECUTION_FAILED)))
    await failedRun.value
    expect(model.captureFailure == "Audio/VFS fixture failed (status \(XENORA_EXECUTION_FAILED))."
           && model.frame?.image == nil && !model.canListen && model.canRun,
           "a failed fixture run did not report its status and keep retry available")
    expect(await probe.calls == [.capture, .listen(pcm), .listen(pcm), .capture, .capture],
           "an action started more than once")
    continuation.finish()
    return failures
}
