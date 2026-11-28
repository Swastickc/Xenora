// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::path::Path;
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use xenora_loader::graphics::{FramePresentation, GraphicsBackendId};
use xenora_loader::startup::{StartupEvent, StartupLayout, run_synthetic_startup_scheduled};
use xenora_loader::{
    Clock, ExitReason, ImportRegistry, MockClock, ServicesState, SystemClock,
    register_standard_services,
};

use super::wire::{MAX_FRAMES, Reply, Request, frame_len, read_request, write_reply};

const MODULE: &str = "xenora-session-v1.dll";
const MAX_EXECUTABLE_BYTES: u64 = 64 * 1024 * 1024;
const QUANTUM: usize = 64;
const REPLAY_FILETIME: u64 = 116_444_736_000_000_000; // 1970-01-01 00:00:00 UTC
const TICKS_PER_INPUT: u64 = 160_000; // 16 milliseconds in 100 ns ticks
const LAYOUT: StartupLayout = StartupLayout {
    image_base: 0x150000000,
    stack_top: 0x200010000,
    tls_base: 0x300000000,
};
// Scheduled startup maps the main TEB 64 KiB above its TLS pointer array.
const MAIN_TEB: u64 = LAYOUT.tls_base + 0x10000;

pub(super) fn run(
    path: &Path,
    digest: [u8; 32],
    steps: usize,
    deterministic_clock: bool,
) -> Result<(), String> {
    let bytes = snapshot(path, digest)?;
    let image_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("the guest PE file name must be UTF-8")?;
    let replay_clock = deterministic_clock.then(|| Arc::new(MockClock::new(REPLAY_FILETIME)));
    let clock: Arc<dyn Clock> = match &replay_clock {
        Some(clock) => clock.clone(),
        None => Arc::new(SystemClock),
    };
    let services = Arc::new(ServicesState::new(clock));
    services
        .set_program_arguments(&[image_name])
        .map_err(|error| format!("could not set guest program arguments: {error}"))?;
    let presentation = services
        .graphics
        .lock()
        .map_err(|_| "the graphics context is poisoned")?
        .last_presentation
        .clone();
    let phase = Arc::new(Mutex::new(Session::default()));
    let mut imports = ImportRegistry::new();
    register_standard_services(&mut imports, Arc::clone(&services));
    register_protocol(&mut imports, &phase, &presentation, &services, replay_clock);

    let result = run_synthetic_startup_scheduled(
        &bytes,
        LAYOUT,
        imports,
        Arc::clone(&services),
        steps,
        QUANTUM,
        |event| {
            if matches!(event, StartupEvent::Entering { .. }) {
                phase.lock().expect("guest session state").entered = true;
            }
        },
    );

    let session = phase.lock().map_err(|_| "the session state is poisoned")?;
    if let Some(error) = &session.failure {
        return Err(error.clone());
    }
    let result = result.map_err(|error| format!("guest startup: {error}"))?;
    match result.execution.exit_reason {
        ExitReason::ProcessExit(0) if services.exit_status() == Some(0) => {}
        ExitReason::ProcessExit(status) => {
            return Err(format!("guest exited with status {status}"));
        }
        ExitReason::StepLimitExceeded => {
            return Err(format!(
                "guest instruction budget exhausted ({steps} steps)"
            ));
        }
        reason => return Err(format!("guest execution stopped: {reason:?}")),
    }
    session.expect(Phase::Finished, "guest exited before XenoraSessionFinish")?;
    session.check_frame_index(&presentation)?;
    send(&Reply::Done)
}

fn snapshot(path: &Path, digest: [u8; 32]) -> Result<Vec<u8>, String> {
    let read_error = |error| format!("could not read guest PE {}: {error}", path.display());
    if !fs::metadata(path).map_err(read_error)?.is_file() {
        return Err("the guest PE must be a regular file".to_owned());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::OpenOptionsExt;

        // If the path changed into a FIFO after stat, open must not wait for
        // a writer before the opened descriptor can be checked.
        const O_NONBLOCK: i32 = 0x00000004;
        options.custom_flags(O_NONBLOCK);
    }
    let file = options.open(path).map_err(read_error)?;
    let metadata = file.metadata().map_err(read_error)?;
    if !metadata.is_file() {
        return Err("the guest PE must be a regular file".to_owned());
    }
    if metadata.len() > MAX_EXECUTABLE_BYTES {
        return Err("the guest PE exceeds the 64 MiB limit".to_owned());
    }
    let mut bytes = Vec::new();
    file.take(MAX_EXECUTABLE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(read_error)?;
    if bytes.len() as u64 > MAX_EXECUTABLE_BYTES {
        return Err("the guest PE exceeds the 64 MiB limit".to_owned());
    }

    let actual: [u8; 32] = Sha256::digest(&bytes).into();
    if actual != digest {
        return Err("guest PE SHA-256 does not match".to_owned());
    }
    Ok(bytes)
}

#[derive(PartialEq, Eq, Default)]
enum Phase {
    #[default]
    Input,
    Present,
    Stopped,
    Finished,
}

#[derive(Default)]
struct Session {
    entered: bool,
    phase: Phase,
    frames: u64,
    failure: Option<String>,
}

impl Session {
    fn expect(&self, phase: Phase, message: &str) -> Result<(), String> {
        if self.phase == phase {
            Ok(())
        } else {
            Err(message.to_owned())
        }
    }

    fn check_frame_index(&self, presentation: &Mutex<FramePresentation>) -> Result<(), String> {
        let frame = presentation
            .lock()
            .map_err(|_| "the presentation is poisoned")?;
        if frame.frame_index != self.frames {
            return Err("guest rendered outside an input and its presentation".to_owned());
        }
        Ok(())
    }
}

fn invoke(
    session: &Mutex<Session>,
    teb: u64,
    operation: impl FnOnce(&mut Session) -> Result<u64, String>,
) -> Result<u64, ExitReason> {
    let mut session = session
        .lock()
        .map_err(|_| ExitReason::ServiceAbiViolation)?;
    let result = if session.entered && teb == MAIN_TEB {
        operation(&mut session)
    } else {
        Err("session imports require the main guest thread after startup".to_owned())
    };
    result.map_err(|message| {
        session.failure = Some(message);
        ExitReason::ServiceAbiViolation
    })
}

fn send(reply: &Reply) -> Result<(), String> {
    write_reply(io::stdout().lock(), reply)
        .map_err(|error| format!("could not send session reply: {error}"))
}

fn register_protocol(
    imports: &mut ImportRegistry,
    session: &Arc<Mutex<Session>>,
    presentation: &Arc<Mutex<FramePresentation>>,
    services: &Arc<ServicesState>,
    replay_clock: Option<Arc<MockClock>>,
) {
    let next = Arc::clone(session);
    let current = Arc::clone(presentation);
    imports.register(
        MODULE,
        "XenoraSessionNextInput",
        move |args, cpu, memory| {
            invoke(&next, cpu.gs_base, |session| {
                session.expect(
                    Phase::Input,
                    "XenoraSessionNextInput called out of sequence",
                )?;
                session.check_frame_index(&current)?;
                send(&Reply::Ready)?;
                match read_request(io::stdin().lock())
                    .map_err(|error| format!("could not read session request: {error}"))?
                {
                    Request::Input(input) => {
                        if session.frames >= MAX_FRAMES as u64 {
                            return Err(format!(
                                "session exceeds the limit of {MAX_FRAMES} inputs"
                            ));
                        }
                        memory.write_u32(args.rcx, input).map_err(|error| {
                            format!("invalid NextInput output address: {error}")
                        })?;
                        if let Some(clock) = &replay_clock {
                            clock.advance(TICKS_PER_INPUT);
                        }
                        session.phase = Phase::Present;
                        Ok(1)
                    }
                    Request::Stop => {
                        session.phase = Phase::Stopped;
                        Ok(0)
                    }
                }
            })
        },
    );

    let presented = Arc::clone(session);
    let current = Arc::clone(presentation);
    imports.register(MODULE, "XenoraSessionPresent", move |_, cpu, _| {
        invoke(&presented, cpu.gs_base, |session| {
            session.expect(
                Phase::Present,
                "XenoraSessionPresent called without a pending input",
            )?;
            let reply = {
                let frame = current.lock().map_err(|_| "the presentation is poisoned")?;
                if frame.frame_index != session.frames + 1
                    || frame.backend != Some(GraphicsBackendId::SoftwareReference)
                {
                    return Err("each input requires exactly one new reference presentation".into());
                }
                let width =
                    u32::try_from(frame.width).map_err(|_| "frame width is out of range")?;
                let height =
                    u32::try_from(frame.height).map_err(|_| "frame height is out of range")?;
                let size = frame_len(width, height).map_err(|error| error.to_string())?;
                if frame.pixels.len() != size {
                    return Err("frame does not have the expected RGBA byte count".to_owned());
                }
                Reply::Frame {
                    width,
                    height,
                    rgba: frame.pixels.clone(),
                }
            };
            send(&reply)?;
            session.frames += 1;
            session.phase = Phase::Input;
            Ok(1)
        })
    });

    let finish = Arc::clone(session);
    let current = Arc::clone(presentation);
    let services = Arc::clone(services);
    imports.register(MODULE, "XenoraSessionFinish", move |_, cpu, _| {
        invoke(&finish, cpu.gs_base, |session| {
            session.expect(
                Phase::Stopped,
                "XenoraSessionFinish requires a Stop request",
            )?;
            session.check_frame_index(&current)?;
            let open_handles = services.open_handle_count();
            if open_handles != 0 {
                return Err(format!(
                    "guest retained {open_handles} open runtime handles"
                ));
            }
            session.phase = Phase::Finished;
            Ok(1)
        })
    });
}
