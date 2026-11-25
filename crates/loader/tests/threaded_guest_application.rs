// SPDX-License-Identifier: GPL-3.0-or-later

//! One original PE drives a persistent Win32 worker, shared file, DBT and
//! guest-authored graphics commands through the scheduled startup path.

use std::sync::{Arc, Mutex};

use xenora_loader::graphics::{FramePresentation, GraphicsBackendId};
use xenora_loader::services::{MockClock, ServicesState, register_standard_services};
use xenora_loader::startup::{
    StartupLayout, StartupResult, run_synthetic_startup, run_synthetic_startup_scheduled,
};
use xenora_loader::{ExitReason, ImportRegistry};

const GUEST: &[u8] = include_bytes!("../../../fixtures/pe/threaded-guest.exe");
const MODULE: &str = "xenora_threaded_app.dll";
const JOURNAL: &str = "C:\\threaded-scene.log";
const WORKER_EXIT: u32 = 0x4C0D_E12A;
const PROCESS_EXIT: u32 = 0x73D0_CA11;
const SCHEDULER_REQUIRED_EXIT: u32 = 0x2000_0001;
const BUDGET: usize = 200_000;
const COMMANDS: [u64; 6] = [1, 1, 2, 2, 2, 1];
// Starting at column zero, two increments, three decrements and an increment.
const COLUMNS: [usize; COMMANDS.len()] = [1, 2, 1, 0, 3, 0];
const WIDTH: usize = 4;
const HEIGHT: usize = 3;
const RED: [u8; 4] = [255, 0, 0, 255];
const CLEAR: [u8; 4] = [19, 43, 93, 251];

#[derive(Debug)]
struct ReportedFrame {
    sequence: u32,
    worker: u32,
    column: u32,
    job_address: u64,
    job_is_readable: bool,
    presentation: FramePresentation,
}

#[derive(Debug)]
struct Cleanup {
    worker: u32,
    worker_status: u32,
    job_address: u64,
    journal_len: u32,
    job_is_unmapped: bool,
    open_handles: usize,
}

#[derive(Debug, Default)]
struct Observations {
    requests: usize,
    frames: Vec<ReportedFrame>,
    cleanup: Option<Cleanup>,
}

struct Run {
    startup: StartupResult,
    observations: Observations,
    state: Arc<ServicesState>,
}

fn state() -> Arc<ServicesState> {
    Arc::new(ServicesState::new(Arc::new(MockClock::new(0))))
}

fn install_observers(
    registry: &mut ImportRegistry,
    state: &Arc<ServicesState>,
    observed: &Arc<Mutex<Observations>>,
    commands: &'static [u64],
) {
    let requests = Arc::clone(observed);
    registry.register(MODULE, "XenoraNextSceneCommand", move |_, _, _| {
        let mut events = requests.lock().expect("scene observations");
        let command = commands.get(events.requests).copied().unwrap_or(0);
        events.requests += 1;
        Ok(command)
    });

    let presentation = Arc::clone(
        &state
            .graphics
            .lock()
            .expect("guest graphics context")
            .last_presentation,
    );
    let frames = Arc::clone(observed);
    registry.register(MODULE, "XenoraReportSceneFrame", move |args, _, memory| {
        let frame = presentation.lock().expect("last frame").clone();
        let mut events = frames.lock().expect("scene observations");
        if events.frames.len() == COMMANDS.len() || events.cleanup.is_some() {
            return Err(ExitReason::ServiceAbiViolation);
        }
        events.frames.push(ReportedFrame {
            sequence: args.rcx as u32,
            worker: args.rdx as u32,
            column: args.r8 as u32,
            job_address: args.r9,
            job_is_readable: memory.read_u8(args.r9).is_ok(),
            presentation: frame,
        });
        Ok(1)
    });

    let cleanup = Arc::clone(observed);
    let services = Arc::clone(state);
    registry.register(
        MODULE,
        "XenoraReportSceneCleanup",
        move |args, _, memory| {
            let mut events = cleanup.lock().expect("scene observations");
            if events.cleanup.is_some() {
                return Err(ExitReason::ServiceAbiViolation);
            }
            events.cleanup = Some(Cleanup {
                worker: args.rcx as u32,
                worker_status: args.rdx as u32,
                job_address: args.r8,
                journal_len: args.r9 as u32,
                job_is_unmapped: memory.read_u8(args.r8).is_err(),
                open_handles: services.open_handle_count(),
            });
            Ok(1)
        },
    );
}

fn run(
    state: Arc<ServicesState>,
    quantum: Option<usize>,
    budget: usize,
    commands: &'static [u64],
) -> Run {
    let observed = Arc::new(Mutex::new(Observations::default()));
    let mut imports = ImportRegistry::new();
    register_standard_services(&mut imports, Arc::clone(&state));
    install_observers(&mut imports, &state, &observed, commands);
    let layout = StartupLayout {
        image_base: 0x1_4000_0000,
        stack_top: 0x2_0001_0000,
        tls_base: 0x3_0000_0000,
    };
    let startup = match quantum {
        Some(quantum) => run_synthetic_startup_scheduled(
            GUEST,
            layout,
            imports,
            Arc::clone(&state),
            budget,
            quantum,
        ),
        None => run_synthetic_startup(GUEST, layout, imports, budget),
    }
    .unwrap_or_else(|error| panic!("boot threaded guest: {error}"));
    let observations = std::mem::take(&mut *observed.lock().expect("scene observations"));
    Run {
        startup,
        observations,
        state,
    }
}

fn expected_pixels(column: usize) -> Vec<u8> {
    (0..WIDTH * HEIGHT)
        .flat_map(|cell| if cell % WIDTH == column { RED } else { CLEAR })
        .collect()
}

fn verify_completed(run: &Run, backend: GraphicsBackendId) {
    assert_eq!(
        run.startup.execution.exit_reason,
        ExitReason::ProcessExit(PROCESS_EXIT),
        "{:?}",
        run.observations
    );
    assert_eq!(run.state.exit_status(), Some(PROCESS_EXIT));
    assert!(run.startup.dbt_metrics.dbt_dispatches > 0);
    assert_eq!(run.observations.requests, COMMANDS.len() + 1);
    assert_eq!(run.observations.frames.len(), COLUMNS.len());
    let first = &run.observations.frames[0];
    assert_ne!(
        first.worker, run.state.thread_id,
        "the main thread did the work"
    );
    assert_ne!(first.worker, 0);
    assert_ne!(first.job_address, 0);
    for (index, (frame, &column)) in run.observations.frames.iter().zip(&COLUMNS).enumerate() {
        assert_eq!(frame.sequence as usize, index + 1);
        assert_eq!(
            frame.worker, first.worker,
            "worker changed at frame {index}"
        );
        assert_eq!(frame.job_address, first.job_address);
        assert!(frame.job_is_readable);
        assert_eq!(frame.column as usize, column);
        assert_eq!(frame.presentation.frame_index as usize, index + 1);
        assert_eq!(frame.presentation.backend, Some(backend));
        assert_eq!(
            (frame.presentation.width, frame.presentation.height),
            (WIDTH, HEIGHT)
        );
        assert_eq!(frame.presentation.draw_count, 1);
        assert_eq!(
            frame.presentation.pixels,
            expected_pixels(column),
            "frame {index}"
        );
    }

    let cleanup = run
        .observations
        .cleanup
        .as_ref()
        .expect("guest reported cleanup");
    assert_eq!(cleanup.worker, first.worker);
    assert_eq!(cleanup.worker_status, WORKER_EXIT);
    assert_eq!(cleanup.job_address, first.job_address);
    assert_eq!(cleanup.journal_len as usize, COLUMNS.len());
    assert!(
        cleanup.job_is_unmapped,
        "VirtualFree did not release the shared job"
    );
    assert_eq!(
        cleanup.open_handles, 0,
        "the guest must close handles before ExitProcess"
    );
    assert_eq!(run.state.open_handle_count(), 0);
    assert_eq!(
        run.state.file_contents(JOURNAL).as_deref(),
        Some(&b"121030"[..])
    );
}

#[test]
fn the_same_worker_drives_every_reference_frame_at_different_quanta() {
    for quantum in [1, 64, 1024] {
        let run = run(state(), Some(quantum), BUDGET, &COMMANDS);
        verify_completed(&run, GraphicsBackendId::SoftwareReference);
    }
}

#[test]
fn the_application_refuses_to_start_without_the_scheduler() {
    let run = run(state(), None, BUDGET, &COMMANDS);
    assert_eq!(
        run.startup.execution.exit_reason,
        ExitReason::ProcessExit(SCHEDULER_REQUIRED_EXIT)
    );
    assert_eq!(run.state.exit_status(), Some(SCHEDULER_REQUIRED_EXIT));
    assert_eq!(run.observations.requests, 0);
    assert!(run.observations.frames.is_empty());
    assert!(run.observations.cleanup.is_none());
}

#[test]
fn an_exhausted_budget_does_not_report_an_application_exit_or_frame() {
    let run = run(state(), Some(64), 1, &COMMANDS);
    assert_eq!(
        run.startup.execution.exit_reason,
        ExitReason::StepLimitExceeded
    );
    assert_eq!(run.state.exit_status(), None);
    assert!(run.observations.frames.is_empty());
    assert!(run.observations.cleanup.is_none());
}

#[test]
fn a_bad_command_after_rendering_does_not_complete_the_application() {
    let run = run(state(), Some(64), BUDGET, &[1, 0xff]);
    assert_eq!(
        run.startup.execution.exit_reason,
        ExitReason::ProcessExit(5)
    );
    assert_eq!(run.state.exit_status(), Some(5));
    assert_eq!(run.observations.requests, 2);
    let [frame] = run.observations.frames.as_slice() else {
        panic!("expected one frame before the error");
    };
    assert_eq!(frame.sequence, 1);
    assert_eq!(frame.presentation.pixels, expected_pixels(1));
    assert!(run.observations.cleanup.is_none());
    assert_eq!(run.state.file_contents(JOURNAL).as_deref(), Some(&b"1"[..]));
    assert_eq!(run.state.open_handle_count(), 0);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires a native Metal device")]
fn the_same_worker_drives_every_frame_through_native_metal() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use xenora_loader::graphics::GraphicsError;
    use xenora_metal::NativeCompute;
    use xenora_shader::{Architecture, PIXEL_ENTRY_POINT, VERTEX_ENTRY_POINT, compile_graphics};

    let gpu = NativeCompute::new().expect("native Metal device");
    eprintln!("render threaded guest on {}", gpu.device_name());
    let msl = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin"),
        include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin"),
    )
    .expect("translate guest graphics stages")
    .msl;
    let state = state();
    let draws = Arc::new(AtomicUsize::new(0));
    let native_draws = Arc::clone(&draws);
    state
        .graphics
        .lock()
        .expect("guest graphics context")
        .set_metal_backend(move |vertices, (width, height), clear| {
            let pixels = gpu
                .render_rgba8(
                    &msl,
                    [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT],
                    vertices,
                    &[[0.0; 4]; 3],
                    [width, height],
                    clear,
                )
                .map_err(|error| GraphicsError::BackendError(format!("Metal draw: {error}")))?;
            native_draws.fetch_add(1, Ordering::Relaxed);
            Ok(pixels)
        });
    let run = run(state, Some(64), BUDGET, &COMMANDS);
    verify_completed(&run, GraphicsBackendId::MetalNative);
    assert_eq!(draws.load(Ordering::Relaxed), COLUMNS.len());
}
