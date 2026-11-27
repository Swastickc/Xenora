// SPDX-License-Identifier: GPL-3.0-or-later

//! Gaps found while reviewing PR 1 (#944).

use xenora_input::buttons;
use xenora_input::keyboard::keycode;
use xenora_input::router::{InputConfig, Router, RoutingMode, SourceId};
use xenora_input::{PadState, merge};

#[test]
fn explicit_assignments_are_bounded() {
    let mut input = Router::new(InputConfig {
        mode: RoutingMode::Explicit,
        ..InputConfig::default()
    });
    for id in 0..16u64 {
        assert!(
            input.assign(SourceId::Device(id), Some((id % 4) as u8)),
            "assignment {id}"
        );
    }
    // A seventeenth distinct source is refused instead of growing the table.
    assert!(!input.assign(SourceId::Device(16), Some(0)));
    assert_eq!(input.assignment(SourceId::Device(16)), None);
    // Re-assigning or removing a known source still works when the table is full.
    assert!(input.assign(SourceId::Device(3), Some(1)));
    assert_eq!(input.assignment(SourceId::Device(3)), Some(1));
    assert!(input.assign(SourceId::Device(3), None));
    assert!(input.assign(SourceId::KeyboardMouse, Some(0)));
    assert_eq!(input.assignment(SourceId::KeyboardMouse), Some(0));
}

#[test]
fn every_mode_neutralizes_on_focus_loss() {
    for mode in [
        RoutingMode::Automatic,
        RoutingMode::KeyboardMouseOnly,
        RoutingMode::PhysicalOnly,
        RoutingMode::Combined,
        RoutingMode::LastActive,
        RoutingMode::Explicit,
    ] {
        let mut input = Router::new(InputConfig {
            mode,
            ..InputConfig::default()
        });
        input.assign(SourceId::KeyboardMouse, Some(0));
        input.assign(SourceId::Device(1), Some(1));
        input.connect_device(1);
        input.device_state(
            1,
            PadState {
                buttons: buttons::Y,
                left_x: 9000,
                ..PadState::default()
            },
            0,
        );
        input.key_down(keycode::ANSI_W, 0);
        input.key_down(keycode::ANSI_Z, 0);
        input.mouse_motion(80.0, -20.0, 0);
        input.set_focus(false);
        for (slot, state) in input.output(1).into_iter().enumerate() {
            if let Some(state) = state {
                assert!(state.is_neutral(), "{mode:?} slot {slot}: {state:?}");
            }
        }
    }
}

#[test]
fn merging_one_state_is_the_identity() {
    let state = PadState {
        buttons: buttons::A | buttons::DPAD_LEFT,
        left_trigger: 3,
        right_trigger: 250,
        left_x: -32768,
        left_y: 12,
        right_x: 0,
        right_y: -1,
    };
    assert_eq!(merge(&[state]), state);
    assert_eq!(merge(&[PadState::default(), state]), state);
}
