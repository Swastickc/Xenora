// SPDX-License-Identifier: GPL-3.0-or-later

//! An original Win64 guest orders two graphics queues through a timeline fence.

use std::sync::Arc;

use xenora_loader::graphics::GraphicsBackendId;
use xenora_loader::services::{MockClock, ServicesState, register_standard_services};
use xenora_loader::startup::{StartupLayout, run_synthetic_startup};
use xenora_loader::{ExitReason, ImportRegistry};

const GUEST: &[u8] = include_bytes!("../../../fixtures/pe/guest-fence.exe");
const IMAGE_BASE: u64 = 0x1_4000_0000;
const INITIAL: u64 = 0x1_0000_0003;
const READY: u64 = 0x1_0000_0007;
const DONE: u64 = 0x1_0000_000d;
const RED: [u8; 4] = [255, 0, 0, 255];
const CLEAR: [u8; 4] = [19, 43, 93, 251];
const WIDTH: usize = 4;
const HEIGHT: usize = 3;
const FRAME_BYTES: usize = WIDTH * HEIGHT * 4;

fn expected_frame(red_column: usize) -> Vec<u8> {
    (0..WIDTH * HEIGHT)
        .flat_map(|pixel| {
            if pixel % WIDTH == red_column {
                RED
            } else {
                CLEAR
            }
        })
        .collect()
}

fn services() -> Arc<ServicesState> {
    Arc::new(ServicesState::new(Arc::new(MockClock::new(0))))
}

fn verify_guest(state: &Arc<ServicesState>, backend: GraphicsBackendId, guest_backend: u32) {
    let mut registry = ImportRegistry::new();
    register_standard_services(&mut registry, Arc::clone(state));
    let outcome = run_synthetic_startup(
        GUEST,
        StartupLayout {
            image_base: IMAGE_BASE,
            stack_top: 0x2_0001_0000,
            tls_base: 0x3_0000_0000,
        },
        registry,
        100_000,
    )
    .expect("start the original graphics fence guest");
    assert_eq!(outcome.execution.exit_reason, ExitReason::ProcessExit(0));
    assert_eq!(state.exit_status(), Some(0));
    assert!(outcome.dbt_metrics.dbt_dispatches > 0);

    let section = outcome
        .image
        .pe
        .sections
        .iter()
        .find(|section| section.name == ".fence")
        .expect("guest fence result section");
    let mut result = [0; 40 + 2 * FRAME_BYTES];
    outcome
        .image
        .memory
        .read_bytes(IMAGE_BASE + u64::from(section.virtual_address), &mut result)
        .expect("read the guest's fence and framebuffer results");
    let word = |offset: usize| u32::from_le_bytes(result[offset..offset + 4].try_into().unwrap());
    let wide = |offset: usize| u64::from_le_bytes(result[offset..offset + 8].try_into().unwrap());
    assert_eq!(word(0), 0x4645_4e43);
    assert_eq!(
        [word(4), word(8), word(12)],
        [guest_backend, guest_backend, 0]
    );
    assert_eq!([wide(16), wide(24), wide(32)], [INITIAL, READY, DONE]);

    let producer = expected_frame(0);
    let consumer = expected_frame(3);
    assert_eq!(&result[40..40 + FRAME_BYTES], producer.as_slice());
    assert_eq!(&result[40 + FRAME_BYTES..], consumer.as_slice());
    let graphics = state.graphics.lock().unwrap();
    let presentation = graphics.last_presentation.lock().unwrap();
    assert_eq!(presentation.frame_index, 2);
    assert_eq!(presentation.draw_count, 1);
    assert_eq!(presentation.backend, Some(backend));
    assert_eq!((presentation.width, presentation.height), (WIDTH, HEIGHT));
    assert_eq!(presentation.pixels, consumer);
}

#[test]
fn guest_timeline_fence_orders_two_reference_draws() {
    verify_guest(&services(), GraphicsBackendId::SoftwareReference, 0);
}

#[cfg(feature = "gpu-tests")]
#[test]
fn guest_timeline_fence_orders_two_native_metal_draws() {
    use std::sync::Mutex;
    use xenora_loader::graphics::{FenceOperation, GraphicsError, NativeFence};
    use xenora_metal::NativeCompute;
    use xenora_shader::{Architecture, PIXEL_ENTRY_POINT, VERTEX_ENTRY_POINT, compile_graphics};

    #[derive(Default)]
    struct MetalTrace {
        created: Vec<u64>,
        signals: Vec<u64>,
        queue_waits: Vec<u64>,
        draws: usize,
    }

    let gpu = Arc::new(NativeCompute::new().expect("native Metal device"));
    eprintln!("guest graphics timeline fence on {}", gpu.device_name());
    let msl = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin"),
        include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin"),
    )
    .expect("translate the original shader stages")
    .msl;
    let trace = Arc::new(Mutex::new(MetalTrace::default()));
    let state = services();
    {
        let mut graphics = state.graphics.lock().unwrap();
        let renderer = Arc::clone(&gpu);
        let draw_trace = Arc::clone(&trace);
        graphics.set_metal_backend(move |vertices, (width, height), clear| {
            let pixels = renderer
                .render_rgba8(
                    &msl,
                    [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT],
                    vertices,
                    &[[0.0; 4]; 3],
                    [width, height],
                    clear,
                )
                .map_err(|error| GraphicsError::BackendError(error.to_string()))?;
            draw_trace.lock().unwrap().draws += 1;
            Ok(pixels)
        });
        let fence_trace = Arc::clone(&trace);
        graphics
            .set_native_fence_factory(move |initial| {
                let fence = gpu
                    .new_render_fence(initial)
                    .map_err(|error| GraphicsError::BackendError(error.to_string()))?;
                fence_trace.lock().unwrap().created.push(initial);
                let trace = Arc::clone(&fence_trace);
                let adapter: NativeFence = Box::new(move |operation| {
                    let completed = match operation {
                        FenceOperation::CompletedValue => fence.completed_value(),
                        FenceOperation::Signal(value) => fence.signal(value),
                        FenceOperation::QueueWait { value, timeout } => {
                            fence.queue_wait(value, timeout)
                        }
                        FenceOperation::HostWait { value, timeout } => {
                            fence.host_wait(value, timeout)
                        }
                    }
                    .map_err(|error| GraphicsError::BackendError(error.to_string()))?;
                    match operation {
                        FenceOperation::Signal(value) => trace.lock().unwrap().signals.push(value),
                        FenceOperation::QueueWait { value, .. } => {
                            trace.lock().unwrap().queue_waits.push(value);
                        }
                        FenceOperation::CompletedValue | FenceOperation::HostWait { .. } => {}
                    }
                    Ok(completed)
                });
                Ok(adapter)
            })
            .unwrap();
    }

    verify_guest(&state, GraphicsBackendId::MetalNative, 1);
    let trace = trace.lock().unwrap();
    assert_eq!(trace.created, [INITIAL]);
    assert_eq!(trace.signals, [READY, DONE, DONE]);
    assert_eq!(trace.queue_waits, [READY, DONE]);
    assert_eq!(trace.draws, 2);
}
