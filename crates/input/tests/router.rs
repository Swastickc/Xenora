// SPDX-License-Identifier: GPL-3.0-or-later

//! Acceptance tests for the host input router (Campaign F, F3, #944).

use xenora_input::buttons;
use xenora_input::keyboard::keycode;
use xenora_input::router::{InputConfig, Router, RoutingMode, SourceId};
use xenora_input::{PadState, merge};

const PAD_A: u64 = 0xA;
const PAD_B: u64 = 0xB;

fn pad(buttons: u16) -> PadState {
    PadState {
        buttons,
        ..PadState::default()
    }
}

fn stick(left_x: i16, left_y: i16) -> PadState {
    PadState {
        left_x,
        left_y,
        ..PadState::default()
    }
}

fn router(mode: RoutingMode) -> Router {
    Router::new(InputConfig {
        mode,
        ..InputConfig::default()
    })
}

#[test]
fn default_mode_is_automatic_and_keyboard_works_without_hardware() {
    let mut input = Router::new(InputConfig::default());
    assert_eq!(input.mode(), RoutingMode::Automatic);
    // The virtual keyboard and mouse pad is connected on slot 0 from the start.
    assert_eq!(
        input.output(0),
        [Some(PadState::default()), None, None, None]
    );
    assert!(input.key_down(keycode::SPACE, 0));
    assert_eq!(input.output(0)[0], Some(pad(buttons::A)));
    assert!(input.key_up(keycode::SPACE, 1));
    assert_eq!(input.output(1)[0], Some(PadState::default()));
}

#[test]
fn devices_take_the_lowest_free_slot_and_keep_it() {
    let mut input = router(RoutingMode::PhysicalOnly);
    assert_eq!(input.output(0), [None, None, None, None]);
    assert_eq!(input.connect_device(PAD_A), Some(0));
    assert_eq!(input.connect_device(PAD_B), Some(1));
    // Connecting again is idempotent.
    assert_eq!(input.connect_device(PAD_A), Some(0));
    assert_eq!(input.device_slot(PAD_B), Some(1));
    assert!(input.disconnect_device(PAD_A));
    assert_eq!(input.device_slot(PAD_A), None);
    // B keeps slot 1; a new device fills the vacated slot 0.
    assert_eq!(input.connect_device(0xC), Some(0));
    assert_eq!(input.device_slot(PAD_B), Some(1));
    assert!(!input.disconnect_device(PAD_A), "already gone");
}

#[test]
fn at_most_four_devices_are_assigned() {
    let mut input = router(RoutingMode::PhysicalOnly);
    for (id, slot) in (1..=4).zip(0u8..) {
        assert_eq!(input.connect_device(id), Some(slot));
    }
    assert_eq!(input.connect_device(5), None);
    assert_eq!(input.device_slot(5), None);
    assert!(
        !input.device_state(5, pad(buttons::A), 0),
        "an unassigned device has no state"
    );
    input.disconnect_device(3);
    assert_eq!(input.connect_device(5), Some(2));
}

#[test]
fn physical_only_ignores_the_keyboard() {
    let mut input = router(RoutingMode::PhysicalOnly);
    input.connect_device(PAD_A);
    assert!(input.device_state(PAD_A, pad(buttons::B), 0));
    input.key_down(keycode::SPACE, 0);
    assert_eq!(input.output(0), [Some(pad(buttons::B)), None, None, None]);
}

#[test]
fn keyboard_mouse_only_ignores_devices_but_tracks_their_slots() {
    let mut input = router(RoutingMode::KeyboardMouseOnly);
    assert_eq!(input.connect_device(PAD_A), Some(0));
    input.device_state(PAD_A, pad(buttons::B), 0);
    input.key_down(keycode::SPACE, 0);
    assert_eq!(input.output(0), [Some(pad(buttons::A)), None, None, None]);
    // Switching modes keeps the device's slot and state.
    input.set_mode(RoutingMode::PhysicalOnly);
    assert_eq!(input.mode(), RoutingMode::PhysicalOnly);
    assert_eq!(input.output(0), [Some(pad(buttons::B)), None, None, None]);
}

#[test]
fn automatic_merges_the_keyboard_into_slot_zero_and_keeps_devices_separate() {
    let mut input = router(RoutingMode::Automatic);
    input.connect_device(PAD_A);
    input.connect_device(PAD_B);
    input.device_state(PAD_A, pad(buttons::B), 0);
    input.device_state(PAD_B, pad(buttons::X), 0);
    input.key_down(keycode::SPACE, 0);
    assert_eq!(
        input.output(0),
        [
            Some(pad(buttons::A | buttons::B)),
            Some(pad(buttons::X)),
            None,
            None
        ]
    );
}

#[test]
fn one_source_never_releases_a_button_another_source_holds() {
    let mut input = router(RoutingMode::Automatic);
    input.connect_device(PAD_A);
    input.device_state(PAD_A, pad(buttons::A), 0);
    input.key_down(keycode::SPACE, 1);
    input.key_up(keycode::SPACE, 2);
    assert_eq!(
        input.output(2)[0],
        Some(pad(buttons::A)),
        "the pad still holds A"
    );
    input.key_down(keycode::RETURN, 3);
    input.device_state(PAD_A, PadState::default(), 4);
    assert_eq!(
        input.output(4)[0],
        Some(pad(buttons::A)),
        "the keyboard still holds A"
    );
}

#[test]
fn combined_merges_every_source_into_slot_zero() {
    let mut input = router(RoutingMode::Combined);
    input.connect_device(PAD_A);
    input.connect_device(PAD_B);
    input.device_state(
        PAD_A,
        PadState {
            left_trigger: 40,
            ..pad(buttons::B)
        },
        0,
    );
    input.device_state(
        PAD_B,
        PadState {
            left_trigger: 200,
            right_trigger: 9,
            ..pad(buttons::Y)
        },
        0,
    );
    input.key_down(keycode::SPACE, 0);
    let expected = PadState {
        left_trigger: 200,
        right_trigger: 9,
        ..pad(buttons::A | buttons::B | buttons::Y)
    };
    assert_eq!(input.output(0), [Some(expected), None, None, None]);
}

#[test]
fn merge_resolves_analog_conflicts_by_largest_magnitude_then_order() {
    // The larger deflection wins for each stick; triggers take the maximum.
    let merged = merge(&[
        PadState {
            left_x: 1000,
            left_y: 0,
            right_x: -30000,
            right_y: 0,
            left_trigger: 10,
            ..pad(buttons::A)
        },
        PadState {
            left_x: 0,
            left_y: -2000,
            right_x: 100,
            right_y: 100,
            right_trigger: 77,
            ..pad(buttons::B)
        },
    ]);
    assert_eq!(
        merged,
        PadState {
            buttons: buttons::A | buttons::B,
            left_trigger: 10,
            right_trigger: 77,
            left_x: 0,
            left_y: -2000,
            right_x: -30000,
            right_y: 0,
        }
    );
    // Equal magnitudes: the earlier source wins, so the result is deterministic.
    assert_eq!(merge(&[stick(3, 4), stick(-4, 3)]), stick(3, 4));
    assert_eq!(merge(&[stick(-4, 3), stick(3, 4)]), stick(-4, 3));
    // An axis pair is never mixed from two sources.
    assert_eq!(merge(&[stick(20000, 0), stick(0, 19000)]), stick(20000, 0));
    // The extreme negative corner must not overflow the magnitude comparison.
    assert_eq!(
        merge(&[stick(32767, 32767), stick(-32768, -32768)]),
        stick(-32768, -32768)
    );
    assert_eq!(merge(&[]), PadState::default());
}

#[test]
fn keyboard_and_pad_sticks_merge_by_magnitude() {
    let mut input = router(RoutingMode::Automatic);
    input.connect_device(PAD_A);
    input.device_state(PAD_A, stick(5000, 0), 0);
    input.key_down(keycode::ANSI_W, 0);
    assert_eq!(input.output(0)[0], Some(stick(0, 32767)));
    input.key_up(keycode::ANSI_W, 1);
    assert_eq!(input.output(1)[0], Some(stick(5000, 0)));
}

#[test]
fn mouse_motion_drives_the_right_stick_of_the_keyboard_pad() {
    let mut input = router(RoutingMode::KeyboardMouseOnly);
    input.mouse_motion(50.0, 0.0, 0);
    let slot = input.output(0)[0].unwrap();
    // Default sensitivity 0.01: 50 * 0.01 = 0.5 -> 16384.
    assert_eq!((slot.right_x, slot.right_y), (16384, 0));
    // After the default 50 ms decay it is neutral again.
    assert_eq!(input.output(50)[0], Some(PadState::default()));
    assert!(input.mouse_button(0, true, 60));
    assert_eq!(input.output(60)[0].unwrap().right_trigger, 255);
}

#[test]
fn focus_loss_neutralizes_and_ignores_input_until_focus_returns() {
    let mut input = router(RoutingMode::Automatic);
    input.connect_device(PAD_A);
    input.device_state(PAD_A, pad(buttons::B), 0);
    input.key_down(keycode::ANSI_W, 0);
    input.mouse_button(0, true, 0);
    input.mouse_motion(40.0, 0.0, 0);
    input.set_focus(false);
    assert!(!input.is_focused());
    assert_eq!(
        input.output(1),
        [Some(PadState::default()), None, None, None]
    );
    // Events while unfocused are refused, and nothing is stuck afterwards.
    assert!(!input.key_down(keycode::SPACE, 2));
    assert!(!input.mouse_button(1, true, 2));
    assert!(!input.device_state(PAD_A, pad(buttons::Y), 2));
    input.mouse_motion(40.0, 0.0, 2);
    input.set_focus(true);
    assert_eq!(
        input.output(3),
        [Some(PadState::default()), None, None, None]
    );
    // Keys released while unfocused do not linger: a key-up after focus returns is harmless.
    input.key_up(keycode::ANSI_W, 4);
    assert_eq!(input.output(4)[0], Some(PadState::default()));
    // Devices stay connected and report again once they send a fresh state.
    assert_eq!(input.device_slot(PAD_A), Some(0));
    assert!(input.device_state(PAD_A, pad(buttons::Y), 5));
    assert_eq!(input.output(5)[0], Some(pad(buttons::Y)));
}

#[test]
fn disconnect_clears_a_device_without_stale_input() {
    let mut input = router(RoutingMode::Automatic);
    input.connect_device(PAD_A);
    input.connect_device(PAD_B);
    input.device_state(PAD_B, pad(buttons::X), 0);
    input.disconnect_device(PAD_B);
    assert_eq!(input.output(0)[1], None);
    // Reconnecting starts neutral rather than replaying the old state.
    assert_eq!(input.connect_device(PAD_B), Some(1));
    assert_eq!(input.output(0)[1], Some(PadState::default()));
}

#[test]
fn replacing_a_controller_on_a_slot_starts_neutral() {
    let mut input = router(RoutingMode::PhysicalOnly);
    input.connect_device(PAD_A);
    input.device_state(PAD_A, pad(buttons::A), 0);
    input.disconnect_device(PAD_A);
    assert_eq!(input.connect_device(PAD_B), Some(0));
    assert_eq!(
        input.output(1),
        [Some(PadState::default()), None, None, None]
    );
}

#[test]
fn last_active_hands_slot_zero_to_the_most_recent_source() {
    let mut input = router(RoutingMode::LastActive);
    input.connect_device(PAD_A);
    input.connect_device(PAD_B);
    // The keyboard is the initial source; other slots are not connected.
    input.key_down(keycode::SPACE, 0);
    assert_eq!(input.output(0), [Some(pad(buttons::A)), None, None, None]);
    assert_eq!(input.active_source(), SourceId::KeyboardMouse);
    // A neutral report from a pad does not take over.
    input.device_state(PAD_A, PadState::default(), 1);
    assert_eq!(input.active_source(), SourceId::KeyboardMouse);
    input.device_state(PAD_B, pad(buttons::Y), 2);
    assert_eq!(input.active_source(), SourceId::Device(PAD_B));
    assert_eq!(input.output(2), [Some(pad(buttons::Y)), None, None, None]);
    // Repeating the same state is not new activity.
    input.device_state(PAD_A, pad(buttons::X), 3);
    input.device_state(PAD_B, pad(buttons::Y), 4);
    assert_eq!(input.active_source(), SourceId::Device(PAD_A));
    // Disconnecting the active device returns slot zero to the keyboard.
    input.disconnect_device(PAD_A);
    assert_eq!(input.active_source(), SourceId::KeyboardMouse);
    assert_eq!(input.output(5)[0], Some(pad(buttons::A)));
    // Mouse motion is keyboard-and-mouse activity.
    input.device_state(PAD_B, pad(buttons::B), 6);
    input.mouse_motion(10.0, 0.0, 7);
    assert_eq!(input.active_source(), SourceId::KeyboardMouse);
}

#[test]
fn explicit_assignment_routes_sources_to_chosen_slots() {
    let mut input = router(RoutingMode::Explicit);
    assert_eq!(
        input.output(0),
        [None, None, None, None],
        "nothing assigned yet"
    );
    assert!(input.assign(SourceId::KeyboardMouse, Some(2)));
    assert!(input.assign(SourceId::Device(PAD_A), Some(0)));
    assert!(input.assign(SourceId::Device(PAD_B), Some(2)));
    assert!(
        !input.assign(SourceId::Device(PAD_B), Some(4)),
        "slot 4 does not exist"
    );
    // PAD_A is assigned but not connected yet.
    input.key_down(keycode::ANSI_X, 0);
    assert_eq!(input.output(0), [None, None, Some(pad(buttons::X)), None]);
    input.connect_device(PAD_A);
    input.connect_device(PAD_B);
    input.device_state(PAD_A, pad(buttons::A), 1);
    input.device_state(PAD_B, pad(buttons::B), 1);
    assert_eq!(
        input.output(1),
        [
            Some(pad(buttons::A)),
            None,
            Some(pad(buttons::X | buttons::B)),
            None
        ]
    );
    // Unassigning drops the source from routing.
    assert!(input.assign(SourceId::KeyboardMouse, None));
    assert_eq!(input.output(2)[2], Some(pad(buttons::B)));
    assert_eq!(input.assignment(SourceId::Device(PAD_A)), Some(0));
    assert_eq!(input.assignment(SourceId::KeyboardMouse), None);
}

#[test]
fn router_output_is_deterministic_for_a_scripted_sequence() {
    fn run() -> Vec<[Option<PadState>; 4]> {
        let mut input = router(RoutingMode::Automatic);
        let mut outputs = Vec::new();
        input.connect_device(7);
        for step in 0u64..200 {
            match step % 7 {
                0 => {
                    let _ = input.key_down(keycode::ANSI_D, step);
                }
                1 => input.mouse_motion(step as f64, -(step as f64) / 2.0, step),
                2 => {
                    let _ = input.device_state(
                        7,
                        stick((step * 100) as i16, -((step * 50) as i16)),
                        step,
                    );
                }
                3 => {
                    let _ = input.key_up(keycode::ANSI_D, step);
                }
                4 => input.set_focus(step % 2 == 0),
                5 => {
                    let _ = input.mouse_button((step % 3) as u8, step % 2 == 0, step);
                }
                _ => {
                    let _ = input.key_down(keycode::SPACE, step);
                }
            }
            outputs.push(input.output(step));
        }
        outputs
    }
    assert_eq!(run(), run());
}

#[test]
fn invalid_configuration_is_refused() {
    let mut config = InputConfig::default();
    config.keyboard.stick_magnitude = 3.0;
    assert!(Router::try_new(config).is_err());
    let mut config = InputConfig::default();
    config.mouse.decay_ms = 0;
    assert!(Router::try_new(config).is_err());
    assert!(Router::try_new(InputConfig::default()).is_ok());
}
