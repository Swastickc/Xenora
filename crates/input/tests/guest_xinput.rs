// SPDX-License-Identifier: GPL-3.0-or-later

//! End-to-end acceptance for Campaign F (#942): an original PE32+ guest reads
//! routed host input only through its own `XInputGetState` calls, executed by
//! the loader and the ARM64 dynamic binary translator.
//!
//! The guest, `fixtures/pe/xinput-poll.c`, polls user slots 0 to 3 once per
//! frame and prints a line for every change. Frames come from Xenora's
//! `xenora_session.dll!XenoraNextEvent` frame-tick service, which this test
//! registers: on the frame-th tick it applies that frame's scripted host
//! events to a [`Router`], translates virtual controllers through their
//! profiles, and installs the router's output into the runtime services'
//! XInput slots through the public `ServicesState` interface. Nothing feeds
//! the guest any other way.
//!
//! The controllers are virtual devices: raw element values stand in for what
//! GameController would report. No physical controller was connected.
//!
//! The expected output is a literal written by hand from the routing rules,
//! not computed by the code under test.

use std::sync::{Arc, Mutex};

use xenora_input::controller::{Family, RawController, element};
use xenora_input::keyboard::keycode;
use xenora_input::profile::Profile;
use xenora_input::router::{InputConfig, Router, RoutingMode};
use xenora_loader::cpu::ExitReason;
use xenora_loader::dbt::DbtMetrics;
use xenora_loader::imports::ImportRegistry;
use xenora_loader::scheduler::DEFAULT_QUANTUM;
use xenora_loader::services::{
    MockClock, ServicesState, XInputControllerState, register_standard_services,
};
use xenora_loader::startup::{StartupLayout, run_synthetic_startup_scheduled};

const GUEST: &[u8] = include_bytes!("../../../fixtures/pe/xinput-poll.exe");
const BUDGET: usize = 5_000_000;
const FRAME_MS: u64 = 16;
const PAD_ONE: u64 = 7;
const PAD_TWO: u64 = 8;

/// What the guest prints, derived by hand:
///
/// 1. The keyboard-and-mouse pad is connected on slot 0 from the start.
/// 2. W and Space: A and full-up left stick.
/// 3. Two DualSense-profile pads connect. Pad one joins slot 0 neutral, which
///    changes nothing there; pad two takes slot 1 with East (B), the right
///    trigger at half travel (0.5 * 255 = 127.5 -> 128) and the left stick
///    fully left.
/// 4. Pad one presses South (A) while Space is released: slot 0 still holds
///    A, so its packet number does not change and the guest prints nothing.
/// 5. Pad one releases South: A goes away; W still holds the stick up.
/// 6. Focus loss neutralizes every slot.
/// 7. Focus returns and pad two disconnects.
/// 8. Mouse motion of 50 points: 50 * 0.01 = 0.5 -> 16384 on the right stick.
const EXPECTED: &str = "\
f1 s0 p1 b0000 t0,0 l0,0 r0,0
f2 s0 p2 b1000 t0,0 l0,32767 r0,0
f3 s1 p1 b2000 t0,128 l-32768,0 r0,0
f5 s0 p3 b0000 t0,0 l0,32767 r0,0
f6 s0 p4 b0000 t0,0 l0,0 r0,0
f6 s1 p2 b0000 t0,0 l0,0 r0,0
f7 s1 disconnected
f8 s0 p5 b0000 t0,0 l0,0 r16384,0
done
";

/// The same script in physical-only mode, derived by hand: the keyboard and
/// the mouse reach nothing, so slot 0 appears only when pad one connects and
/// then follows pad one alone.
const EXPECTED_PHYSICAL_ONLY: &str = "\
f3 s0 p1 b0000 t0,0 l0,0 r0,0
f3 s1 p1 b2000 t0,128 l-32768,0 r0,0
f4 s0 p2 b1000 t0,0 l0,0 r0,0
f5 s0 p3 b0000 t0,0 l0,0 r0,0
f6 s1 p2 b0000 t0,0 l0,0 r0,0
f7 s1 disconnected
done
";

struct Host {
    router: Router,
    profile: Profile,
    frame: u32,
}

impl Host {
    fn device(&mut self, id: u64, raw: RawController, now: u64) {
        let state = self.profile.translate(&raw);
        assert!(
            self.router.device_state(id, state, now),
            "device {id} accepts state"
        );
    }

    /// Apply the scripted host events for one frame.
    fn step(&mut self, frame: u32) {
        let now = u64::from(frame) * FRAME_MS;
        match frame {
            2 => {
                assert!(self.router.key_down(keycode::ANSI_W, now));
                assert!(self.router.key_down(keycode::SPACE, now));
            }
            3 => {
                assert_eq!(self.router.connect_device(PAD_ONE), Some(0));
                assert_eq!(self.router.connect_device(PAD_TWO), Some(1));
                let raw = RawController {
                    elements: element::EAST,
                    right_trigger: 0.5,
                    left_stick: [-1.0, 0.0],
                    ..RawController::default()
                };
                self.device(PAD_TWO, raw, now);
            }
            4 => {
                let raw = RawController {
                    elements: element::SOUTH,
                    ..RawController::default()
                };
                self.device(PAD_ONE, raw, now);
                assert!(self.router.key_up(keycode::SPACE, now));
            }
            5 => self.device(PAD_ONE, RawController::default(), now),
            6 => self.router.set_focus(false),
            7 => {
                self.router.set_focus(true);
                assert!(self.router.disconnect_device(PAD_TWO));
            }
            8 => self.router.mouse_motion(50.0, 0.0, now),
            _ => {}
        }
    }
}

fn publish(state: &ServicesState, router: &Router, now: u64) {
    for (slot, output) in router.output(now).into_iter().enumerate() {
        let snapshot = output.map(|pad| XInputControllerState {
            buttons: pad.buttons,
            left_trigger: pad.left_trigger,
            right_trigger: pad.right_trigger,
            left_x: pad.left_x,
            left_y: pad.left_y,
            right_x: pad.right_x,
            right_y: pad.right_y,
        });
        assert!(state.set_xinput_controller(slot as u32, snapshot));
    }
}

fn run(last_frame: u32, mode: RoutingMode) -> (ExitReason, String, DbtMetrics) {
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let mut registry = ImportRegistry::new();
    register_standard_services(&mut registry, Arc::clone(&state));
    let host = Arc::new(Mutex::new(Host {
        router: Router::new(InputConfig {
            mode,
            ..InputConfig::default()
        }),
        profile: Profile::default_for(Family::DualSense),
        frame: 0,
    }));
    let tick_state = Arc::clone(&state);
    let tick_host = Arc::clone(&host);
    registry.register("xenora_session.dll", "XenoraNextEvent", move |_, _, _| {
        let mut host = tick_host
            .lock()
            .map_err(|_| ExitReason::ServiceAbiViolation)?;
        if host.frame >= last_frame {
            return Ok(0);
        }
        host.frame += 1;
        let frame = host.frame;
        host.step(frame);
        publish(&tick_state, &host.router, u64::from(frame) * FRAME_MS);
        Ok(u64::from(frame))
    });
    let layout = StartupLayout {
        image_base: 0x2_5000_0000,
        stack_top: 0x2_0001_0000,
        tls_base: 0x3_0000_0000,
    };
    let result = run_synthetic_startup_scheduled(
        GUEST,
        layout,
        registry,
        Arc::clone(&state),
        BUDGET,
        DEFAULT_QUANTUM,
        |_| {},
    )
    .expect("the guest starts");
    let output = String::from_utf8(state.standard_output()).expect("ASCII output");
    (result.execution.exit_reason, output, result.dbt_metrics)
}

#[test]
fn guest_reads_routed_keyboard_mouse_and_virtual_controllers_through_xinput() {
    let (exit, output, metrics) = run(8, RoutingMode::Automatic);
    assert_eq!(exit, ExitReason::ProcessExit(0), "output so far:\n{output}");
    assert_eq!(output, EXPECTED);
    // The guest ran as translated ARM64 code, and its eight frames made at
    // least the 32 XInputGetState calls plus the frame ticks as services.
    assert!(
        metrics.blocks_compiled > 0 && metrics.native_bytes_emitted > 0,
        "{metrics:?}"
    );
    assert!(metrics.service_calls >= 8 * 4 + 9, "{metrics:?}");
}

#[test]
fn guest_run_is_deterministic() {
    let (first_exit, first_output, _) = run(8, RoutingMode::Automatic);
    let (second_exit, second_output, _) = run(8, RoutingMode::Automatic);
    assert_eq!((first_exit, first_output), (second_exit, second_output));
}

#[test]
fn with_no_frames_the_guest_sees_nothing_and_exits() {
    let (exit, output, _) = run(0, RoutingMode::Automatic);
    assert_eq!(exit, ExitReason::ProcessExit(0));
    assert_eq!(output, "done\n");
}

#[test]
fn physical_only_routing_changes_what_the_guest_reads() {
    let (exit, output, _) = run(8, RoutingMode::PhysicalOnly);
    assert_eq!(exit, ExitReason::ProcessExit(0), "output so far:\n{output}");
    assert_eq!(output, EXPECTED_PHYSICAL_ONLY);
}
