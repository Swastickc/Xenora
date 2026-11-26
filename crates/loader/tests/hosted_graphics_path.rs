// SPDX-License-Identifier: GPL-3.0-or-later

//! A hosted C worker reads CRT file records and submits both guest frames.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use xenora_loader::cpu::ExitReason;
use xenora_loader::imports::ImportRegistry;
use xenora_loader::scheduler::DEFAULT_QUANTUM;
use xenora_loader::services::{MockClock, ServicesState, register_standard_services};
use xenora_loader::startup::{
    StartupLayout, StartupResult, run_synthetic_startup, run_synthetic_startup_scheduled,
};

const LAYOUT: StartupLayout = StartupLayout {
    image_base: 0x2_5000_0000,
    stack_top: 0x2_0001_0000,
    tls_base: 0x3_0000_0000,
};
const BUDGET: usize = 5_000_000;
const VARIANTS: [&str; 2] = ["O0", "O2"];
const SCENE_PATH: &str = r"C:\hosted-scene.txt";
const SCENE_FILE: &[u8] = b"inside\r\noutside\r\n";
const REVERSED_SCENE_FILE: &[u8] = b"outside\r\ninside\r\n";
const OUTPUT: &[u8] = b"hosted graphics: two frames\r\n";
const WORKER_STATUS: u32 = 0x41c0_ffee;
const WIDTH: usize = 8;
const HEIGHT: usize = 8;
const FRAME_BYTES: usize = WIDTH * HEIGHT * 4;
const INSIDE: [u8; 4] = [255, 0, 0, 255];
const OUTSIDE: [u8; 4] = [10, 20, 30, 255];

fn built() -> &'static Path {
    static ARTIFACTS: OnceLock<PathBuf> = OnceLock::new();
    ARTIFACTS.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let out = root
            .join("artifacts/hosted-graphics")
            .join(std::process::id().to_string());
        let build = Command::new("sh")
            .arg(root.join("tests/compat_fixtures/build-hosted-graphics.sh"))
            .arg(&out)
            .output()
            .expect("invoke the hosted graphics build");
        assert!(
            build.status.success(),
            "hosted graphics build failed:\n{}{}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
        out
    })
}

fn services() -> Arc<ServicesState> {
    Arc::new(ServicesState::new(Arc::new(MockClock::new(0))))
}

fn run(
    variant: &str,
    state: &Arc<ServicesState>,
    args: &[&str],
    quantum: Option<usize>,
) -> StartupResult {
    let name = format!("hosted-graphics-{variant}.exe");
    let mut argv = vec![name.as_str(), SCENE_PATH];
    argv.extend_from_slice(args);
    state.set_program_arguments(&argv).expect("set hosted argv");
    let bytes = std::fs::read(built().join(name)).expect("read the original hosted guest PE");
    let mut registry = ImportRegistry::new();
    register_standard_services(&mut registry, Arc::clone(state));
    let result = match quantum {
        Some(quantum) => run_synthetic_startup_scheduled(
            &bytes,
            LAYOUT,
            registry,
            Arc::clone(state),
            BUDGET,
            quantum,
            |_| {},
        ),
        None => run_synthetic_startup(&bytes, LAYOUT, registry, BUDGET),
    };
    result.unwrap_or_else(|error| panic!("{variant}, quantum {quantum:?}: {error}"))
}

struct GuestReport {
    main_thread: u32,
    assigned_worker: u32,
    worker_thread: u32,
    join_result: u32,
    worker_status: u32,
    frame_info: [[u32; 4]; 2],
    frames: [[u8; FRAME_BYTES]; 2],
}

impl GuestReport {
    fn read(result: &StartupResult) -> Self {
        let image = &result.image;
        let section = image
            .pe
            .sections
            .iter()
            .find(|section| section.name == ".hostgfx")
            .expect("the mapped guest contains its report section");
        let addr = LAYOUT.image_base + u64::from(section.virtual_address);
        let word = |index: u64| image.memory.read_u32(addr + index * 4).unwrap();
        assert_eq!(word(0), 0x4847_4658, "hosted main did not report");
        let mut frames = [[0; FRAME_BYTES]; 2];
        for (index, frame) in frames.iter_mut().enumerate() {
            image
                .memory
                .read_bytes(addr + 56 + (index * FRAME_BYTES) as u64, frame)
                .expect("read inline frame bytes from guest memory");
        }
        Self {
            main_thread: word(1),
            assigned_worker: word(2),
            worker_thread: word(3),
            join_result: word(4),
            worker_status: word(5),
            frame_info: std::array::from_fn(|frame| {
                std::array::from_fn(|field| word((6 + frame * 4 + field) as u64))
            }),
            frames,
        }
    }

    fn check_worker(&self, status: u32) {
        assert_ne!(self.assigned_worker, 0);
        assert_ne!(self.assigned_worker, self.main_thread);
        assert_eq!(self.worker_thread, self.assigned_worker);
        assert_eq!(self.join_result, 0, "WAIT_OBJECT_0");
        assert_eq!(self.worker_status, status);
    }

    fn check_no_frames(&self) {
        assert_eq!(self.frame_info, [[0; 4]; 2]);
        assert_eq!(self.frames, [[0; FRAME_BYTES]; 2]);
    }
}

fn check_exit(result: &StartupResult, state: &ServicesState, status: u32) {
    assert_eq!(
        result.execution.exit_reason,
        ExitReason::ProcessExit(status)
    );
    assert_eq!(state.exit_status(), Some(status));
    assert_eq!(state.open_handle_count(), 0, "a guest handle leaked");
    assert!(state.standard_error().is_empty());
}

fn check_rendered(
    result: &StartupResult,
    state: &ServicesState,
    backend: u32,
    scene_file: &[u8],
    frame_colors: [[u8; 4]; 2],
) {
    check_exit(result, state, 0);
    assert!(
        result.relocations.applied_dir64 > 0,
        "the PE must be rebased"
    );
    assert!(result.dbt_metrics.native_bytes_emitted > 0);
    assert_eq!(state.standard_output(), OUTPUT);
    assert_eq!(state.file_contents(SCENE_PATH).as_deref(), Some(scene_file));

    let report = GuestReport::read(result);
    assert_ne!(report.main_thread, 0);
    report.check_worker(WORKER_STATUS);
    assert_eq!(
        report.frame_info,
        [[backend, WIDTH as u32, HEIGHT as u32, 1]; 2],
        "each frame reports one draw"
    );
    // The inside triangle is red. The outside triangle leaves the clear color.
    for (frame, pixel) in report.frames.iter().zip(frame_colors) {
        assert_eq!(frame.as_slice(), pixel.repeat(WIDTH * HEIGHT));
    }
}

#[test]
fn hosted_worker_reads_file_and_renders_two_reference_frames() {
    for variant in VARIANTS {
        for quantum in [23, DEFAULT_QUANTUM] {
            let state = services();
            let result = run(variant, &state, &[], Some(quantum));
            check_rendered(&result, &state, 0, SCENE_FILE, [INSIDE, OUTSIDE]);
        }
    }
}

#[test]
fn changing_the_scene_file_order_changes_the_frame_order() {
    let state = services();
    let result = run("O2", &state, &["reverse"], Some(DEFAULT_QUANTUM));
    check_rendered(&result, &state, 0, REVERSED_SCENE_FILE, [OUTSIDE, INSIDE]);
}

#[test]
fn hosted_application_handles_missing_input() {
    let state = services();
    let result = run("O2", &state, &["missing"], Some(DEFAULT_QUANTUM));
    check_exit(&result, &state, 61);
    assert!(state.file_contents(SCENE_PATH).is_none());
    assert!(state.standard_output().is_empty());
    let report = GuestReport::read(&result);
    assert_ne!(report.main_thread, 0);
    report.check_worker(61);
    report.check_no_frames();
}

#[test]
fn hosted_application_reports_a_missing_scheduler() {
    let state = services();
    let result = run("O2", &state, &[], None);
    check_exit(&result, &state, 62);
    assert_eq!(state.file_contents(SCENE_PATH).as_deref(), Some(SCENE_FILE));
    assert!(state.standard_output().is_empty());
    let report = GuestReport::read(&result);
    assert_ne!(report.main_thread, 0);
    assert_eq!(report.assigned_worker, 0);
    assert_eq!(report.worker_thread, 0);
    report.check_no_frames();
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn hosted_worker_reads_file_and_renders_two_metal_frames() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use xenora_loader::graphics::GraphicsError;
    use xenora_metal::NativeCompute;
    use xenora_shader::{Architecture, PIXEL_ENTRY_POINT, VERTEX_ENTRY_POINT, compile_graphics};

    let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
    let msl = Arc::new(
        compile_graphics(Architecture::Gfx1030, vs, ps)
            .expect("translate the original gfx1030 stages")
            .msl,
    );
    let gpu = Arc::new(NativeCompute::new().expect("native Metal device"));
    for variant in VARIANTS {
        let state = services();
        let completed = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&completed);
        let gpu = Arc::clone(&gpu);
        let msl = Arc::clone(&msl);
        state.graphics.lock().unwrap().set_metal_backend(
            move |vertices, (width, height), clear| {
                let bytes = gpu
                    .render_rgba8(
                        &msl,
                        [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT],
                        vertices,
                        &[[0.0; 4]; 3],
                        [width, height],
                        clear,
                    )
                    .map_err(|error| GraphicsError::BackendError(format!("Metal: {error:?}")))?;
                count.fetch_add(1, Ordering::Relaxed);
                Ok(bytes)
            },
        );
        let result = run(variant, &state, &[], Some(DEFAULT_QUANTUM));
        check_rendered(&result, &state, 1, SCENE_FILE, [INSIDE, OUTSIDE]);
        assert_eq!(completed.load(Ordering::Relaxed), 2, "{variant}");
    }
}
