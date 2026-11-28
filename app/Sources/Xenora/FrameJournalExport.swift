// SPDX-License-Identifier: GPL-3.0-or-later

import Foundation
import SwiftUI
import UniformTypeIdentifiers

struct FrameJournalDocument: FileDocument {
    static let readableContentTypes: [UTType] = [.commaSeparatedText]
    let data: Data

    init?(records: [UInt8]) {
        var csv = "frame,column,mode,input\n"
        for (index, record) in records.enumerated() {
            // The low two bits hold the column. The remaining bits encode the
            // mode and input as documented by xenora_session_frame_loop_journal.
            let value: String
            switch record & 0xfc {
            case 0x10: value = "standard,none"
            case 0x20: value = "interactive,neutral"
            case 0x24: value = "interactive,right"
            case 0x28: value = "interactive,left"
            default: return nil
            }
            csv += "\(index + 1),\(record & 0x03),\(value)\n"
        }
        data = Data(csv.utf8)
    }

    init(configuration: ReadConfiguration) throws {
        // The export document comes from a verified guest journal.
        throw CocoaError(.fileReadUnknown)
    }

    func fileWrapper(configuration: WriteConfiguration) throws -> FileWrapper {
        FileWrapper(regularFileWithContents: data)
    }
}

@MainActor
struct FrameJournalExportView: View {
    @ObservedObject var model: FrameLoopModel
    @State private var exportingJournal = false
    @State private var exportDocument: FrameJournalDocument?
    @State private var exportError: String?

    var body: some View {
        VStack(alignment: .leading) {
            Button("Save frame journal…") {
                exportError = nil
                guard let document = model.captureJournalForExport() else { return }
                exportDocument = document
                exportingJournal = true
            }
            .disabled(!model.canExportJournal || exportingJournal)
            .accessibilityIdentifier("frame-journal-save")
            if let journalExportError = model.journalExportError {
                Text(journalExportError).foregroundStyle(.red)
            }
            if let exportError {
                Text(exportError).foregroundStyle(.red)
            }
        }
        .onChange(of: model.canExportJournal) { _, available in
            if !available { exportError = nil }
        }
        .fileExporter(
            isPresented: $exportingJournal,
            document: exportDocument,
            contentTypes: [.commaSeparatedText],
            defaultFilename: "xenora-frame-journal"
        ) { result in
            exportDocument = nil
            if case let .failure(error) = result, (error as? CocoaError)?.code != .userCancelled {
                exportError = "Could not save the frame journal: \(error.localizedDescription)"
            }
        } onCancellation: {
            exportDocument = nil
        }
    }
}
