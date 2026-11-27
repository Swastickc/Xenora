// SPDX-License-Identifier: GPL-3.0-or-later

//! Acceptance tests for the keyboard translator (Campaign F, F1, #944).
//!
//! Expected values are derived by hand from the rules in the crate
//! documentation: a positive axis scales by 32767, a negative axis by 32768,
//! both rounded half away from zero; diagonals are normalized to unit length.

use xenora_input::buttons;
use xenora_input::keyboard::{KeyBinding, KeyboardConfig, KeyboardTranslator, keycode};
use xenora_input::{Direction, PadState, Target};

fn translator() -> KeyboardTranslator {
    KeyboardTranslator::new(KeyboardConfig::default())
}

#[test]
fn macos_virtual_key_codes_match_carbon_events_h() {
    // Values from Carbon HIToolbox Events.h in the macOS SDK.
    assert_eq!(keycode::ANSI_A, 0x00);
    assert_eq!(keycode::ANSI_S, 0x01);
    assert_eq!(keycode::ANSI_D, 0x02);
    assert_eq!(keycode::ANSI_Z, 0x06);
    assert_eq!(keycode::ANSI_X, 0x07);
    assert_eq!(keycode::ANSI_C, 0x08);
    assert_eq!(keycode::ANSI_V, 0x09);
    assert_eq!(keycode::ANSI_Q, 0x0C);
    assert_eq!(keycode::ANSI_W, 0x0D);
    assert_eq!(keycode::ANSI_E, 0x0E);
    assert_eq!(keycode::ANSI_Y, 0x10);
    assert_eq!(keycode::RETURN, 0x24);
    assert_eq!(keycode::TAB, 0x30);
    assert_eq!(keycode::SPACE, 0x31);
    assert_eq!(keycode::DELETE, 0x33);
    assert_eq!(keycode::ESCAPE, 0x35);
    assert_eq!(keycode::SHIFT, 0x38);
    assert_eq!(keycode::RIGHT_SHIFT, 0x3C);
    assert_eq!(keycode::LEFT_ARROW, 0x7B);
    assert_eq!(keycode::RIGHT_ARROW, 0x7C);
    assert_eq!(keycode::DOWN_ARROW, 0x7D);
    assert_eq!(keycode::UP_ARROW, 0x7E);
}

#[test]
fn xinput_button_bits_are_the_public_values() {
    assert_eq!(buttons::DPAD_UP, 0x0001);
    assert_eq!(buttons::DPAD_DOWN, 0x0002);
    assert_eq!(buttons::DPAD_LEFT, 0x0004);
    assert_eq!(buttons::DPAD_RIGHT, 0x0008);
    assert_eq!(buttons::START, 0x0010);
    assert_eq!(buttons::BACK, 0x0020);
    assert_eq!(buttons::LEFT_THUMB, 0x0040);
    assert_eq!(buttons::RIGHT_THUMB, 0x0080);
    assert_eq!(buttons::LEFT_SHOULDER, 0x0100);
    assert_eq!(buttons::RIGHT_SHOULDER, 0x0200);
    assert_eq!(buttons::GUIDE, 0x0400);
    assert_eq!(buttons::A, 0x1000);
    assert_eq!(buttons::B, 0x2000);
    assert_eq!(buttons::X, 0x4000);
    assert_eq!(buttons::Y, 0x8000);
}

#[test]
fn starts_neutral() {
    let keys = translator();
    assert_eq!(keys.state(0), PadState::default());
    assert!(keys.state(0).is_neutral());
    assert!(!keys.is_active());
}

#[test]
fn default_bindings_cover_the_campaign_table() {
    let cases: [(u16, u16); 15] = [
        (keycode::RETURN, buttons::A),
        (keycode::SPACE, buttons::A),
        (keycode::ESCAPE, buttons::B),
        (keycode::DELETE, buttons::B),
        (keycode::ANSI_X, buttons::X),
        (keycode::ANSI_Y, buttons::Y),
        (keycode::UP_ARROW, buttons::DPAD_UP),
        (keycode::DOWN_ARROW, buttons::DPAD_DOWN),
        (keycode::LEFT_ARROW, buttons::DPAD_LEFT),
        (keycode::RIGHT_ARROW, buttons::DPAD_RIGHT),
        (keycode::ANSI_Q, buttons::LEFT_SHOULDER),
        (keycode::ANSI_E, buttons::RIGHT_SHOULDER),
        (keycode::TAB, buttons::START),
        (keycode::ANSI_V, buttons::BACK),
        (keycode::SHIFT, buttons::LEFT_THUMB),
    ];
    for (key, bit) in cases {
        let mut keys = translator();
        assert!(keys.key_down(key, 0), "key {key:#x} is bound");
        assert_eq!(keys.state(0).buttons, bit, "key {key:#x}");
        assert!(keys.is_active());
        assert!(keys.key_up(key, 1));
        assert_eq!(keys.state(1), PadState::default());
    }
    let mut keys = translator();
    keys.key_down(keycode::RIGHT_SHIFT, 0);
    assert_eq!(keys.state(0).buttons, buttons::RIGHT_THUMB);
}

#[test]
fn triggers_are_full_while_held() {
    let mut keys = translator();
    keys.key_down(keycode::ANSI_Z, 0);
    assert_eq!(keys.state(0).left_trigger, 255);
    assert_eq!(keys.state(0).right_trigger, 0);
    keys.key_down(keycode::ANSI_C, 0);
    assert_eq!(keys.state(0).right_trigger, 255);
    keys.key_up(keycode::ANSI_Z, 1);
    let state = keys.state(1);
    assert_eq!((state.left_trigger, state.right_trigger), (0, 255));
}

#[test]
fn wasd_drives_the_left_stick_at_full_scale() {
    let mut keys = translator();
    keys.key_down(keycode::ANSI_D, 0);
    assert_eq!((keys.state(0).left_x, keys.state(0).left_y), (32767, 0));
    keys.key_up(keycode::ANSI_D, 0);
    keys.key_down(keycode::ANSI_A, 0);
    assert_eq!((keys.state(0).left_x, keys.state(0).left_y), (-32768, 0));
    keys.key_up(keycode::ANSI_A, 0);
    keys.key_down(keycode::ANSI_W, 0);
    // XInput thumb Y is positive upward.
    assert_eq!((keys.state(0).left_x, keys.state(0).left_y), (0, 32767));
    keys.key_up(keycode::ANSI_W, 0);
    keys.key_down(keycode::ANSI_S, 0);
    assert_eq!((keys.state(0).left_x, keys.state(0).left_y), (0, -32768));
    assert_eq!((keys.state(0).right_x, keys.state(0).right_y), (0, 0));
}

#[test]
fn diagonals_are_normalized() {
    let mut keys = translator();
    keys.key_down(keycode::ANSI_W, 0);
    keys.key_down(keycode::ANSI_D, 0);
    // (1, 1) / sqrt(2) = 0.70710678; 0.70710678 * 32767 = 23169.77.
    assert_eq!((keys.state(0).left_x, keys.state(0).left_y), (23170, 23170));
    keys.key_up(keycode::ANSI_W, 0);
    keys.key_up(keycode::ANSI_D, 0);
    keys.key_down(keycode::ANSI_S, 0);
    keys.key_down(keycode::ANSI_A, 0);
    // 0.70710678 * 32768 = 23170.47.
    assert_eq!(
        (keys.state(0).left_x, keys.state(0).left_y),
        (-23170, -23170)
    );
}

#[test]
fn opposite_directions_cancel() {
    let mut keys = translator();
    keys.key_down(keycode::ANSI_A, 0);
    keys.key_down(keycode::ANSI_D, 0);
    keys.key_down(keycode::ANSI_W, 0);
    assert_eq!((keys.state(0).left_x, keys.state(0).left_y), (0, 32767));
    keys.key_down(keycode::ANSI_S, 0);
    assert_eq!((keys.state(0).left_x, keys.state(0).left_y), (0, 0));
}

#[test]
fn simultaneous_buttons_sticks_and_triggers_are_independent() {
    let mut keys = translator();
    for key in [
        keycode::SPACE,
        keycode::ANSI_X,
        keycode::ANSI_Q,
        keycode::UP_ARROW,
        keycode::ANSI_D,
        keycode::ANSI_C,
    ] {
        assert!(keys.key_down(key, 0));
    }
    let state = keys.state(0);
    assert_eq!(
        state.buttons,
        buttons::A | buttons::X | buttons::LEFT_SHOULDER | buttons::DPAD_UP
    );
    assert_eq!((state.left_x, state.left_y), (32767, 0));
    assert_eq!((state.left_trigger, state.right_trigger), (0, 255));
    keys.key_up(keycode::ANSI_X, 1);
    let state = keys.state(1);
    assert_eq!(
        state.buttons,
        buttons::A | buttons::LEFT_SHOULDER | buttons::DPAD_UP
    );
    assert_eq!(state.left_x, 32767);
    assert_eq!(state.right_trigger, 255);
}

#[test]
fn two_keys_on_one_button_hold_it_until_both_are_released() {
    let mut keys = translator();
    keys.key_down(keycode::RETURN, 0);
    keys.key_down(keycode::SPACE, 0);
    keys.key_up(keycode::RETURN, 1);
    assert_eq!(keys.state(1).buttons, buttons::A);
    keys.key_up(keycode::SPACE, 2);
    assert_eq!(keys.state(2).buttons, 0);
}

#[test]
fn unbound_keys_are_ignored() {
    let mut keys = translator();
    // kVK_ANSI_K = 0x28 is not bound by default.
    assert!(!keys.key_down(0x28, 0));
    assert!(!keys.is_active());
    assert_eq!(keys.state(0), PadState::default());
    assert!(!keys.key_up(0x28, 0));
}

#[test]
fn releasing_a_key_that_was_never_pressed_changes_nothing() {
    let mut keys = translator();
    keys.key_down(keycode::ANSI_W, 0);
    assert!(keys.key_up(keycode::ANSI_S, 0));
    assert_eq!(keys.state(0).left_y, 32767);
}

#[test]
fn release_all_clears_every_held_key() {
    let mut keys = translator();
    for key in [
        keycode::ANSI_W,
        keycode::ANSI_A,
        keycode::SPACE,
        keycode::ANSI_Z,
        keycode::TAB,
    ] {
        keys.key_down(key, 0);
    }
    keys.release_all();
    assert!(!keys.is_active());
    assert_eq!(keys.state(0), PadState::default());
    // A key-up arriving after the release must not underflow or re-press anything.
    keys.key_up(keycode::ANSI_W, 1);
    assert_eq!(keys.state(1), PadState::default());
}

#[test]
fn analog_ramp_grows_linearly_and_saturates() {
    let config = KeyboardConfig {
        ramp_ms: 100,
        ..KeyboardConfig::default()
    };
    let mut keys = KeyboardTranslator::new(config);
    keys.key_down(keycode::ANSI_D, 1000);
    assert_eq!(keys.state(1000).left_x, 0);
    // 50 / 100 = 0.5; 0.5 * 32767 = 16383.5, rounded half away from zero.
    assert_eq!(keys.state(1050).left_x, 16384);
    assert_eq!(keys.state(1100).left_x, 32767);
    assert_eq!(keys.state(5000).left_x, 32767);
    // A time before the press reads as no deflection rather than wrapping.
    assert_eq!(keys.state(900).left_x, 0);
    // Buttons and triggers never ramp.
    keys.key_down(keycode::ANSI_Z, 2000);
    keys.key_down(keycode::SPACE, 2000);
    assert_eq!(keys.state(2000).left_trigger, 255);
    assert_eq!(keys.state(2000).buttons, buttons::A);
}

#[test]
fn key_repeat_does_not_restart_the_ramp() {
    let config = KeyboardConfig {
        ramp_ms: 100,
        ..KeyboardConfig::default()
    };
    let mut keys = KeyboardTranslator::new(config);
    keys.key_down(keycode::ANSI_A, 0);
    assert!(keys.key_down(keycode::ANSI_A, 75));
    // 75 / 100 = 0.75; -0.75 * 32768 = -24576.
    assert_eq!(keys.state(75).left_x, -24576);
}

#[test]
fn ramped_diagonal_is_clamped_to_unit_length() {
    let config = KeyboardConfig {
        ramp_ms: 100,
        ..KeyboardConfig::default()
    };
    let mut keys = KeyboardTranslator::new(config);
    keys.key_down(keycode::ANSI_D, 0);
    keys.key_down(keycode::ANSI_W, 50);
    // At 50 ms: (0.5, 0) -> 16384.
    assert_eq!((keys.state(50).left_x, keys.state(50).left_y), (16384, 0));
    // At 100 ms: (1.0, 0.5), length 1.1180340 > 1 -> (0.8944272, 0.4472136).
    // 0.8944272 * 32767 = 29307.8; 0.4472136 * 32767 = 14653.9.
    assert_eq!(
        (keys.state(100).left_x, keys.state(100).left_y),
        (29308, 14654)
    );
}

#[test]
fn stick_magnitude_scales_keyboard_deflection() {
    let config = KeyboardConfig {
        stick_magnitude: 0.5,
        ..KeyboardConfig::default()
    };
    let mut keys = KeyboardTranslator::new(config);
    keys.key_down(keycode::ANSI_A, 0);
    // -0.5 * 32768 = -16384.
    assert_eq!(keys.state(0).left_x, -16384);
}

#[test]
fn custom_bindings_can_drive_the_right_stick() {
    let config = KeyboardConfig {
        bindings: vec![
            KeyBinding {
                key: 0x22,
                target: Target::RightStick(Direction::Up),
            },
            KeyBinding {
                key: 0x26,
                target: Target::RightStick(Direction::Left),
            },
            KeyBinding {
                key: 0x28,
                target: Target::LeftTrigger,
            },
        ],
        ..KeyboardConfig::default()
    };
    let mut keys = KeyboardTranslator::new(config);
    keys.key_down(0x22, 0);
    assert_eq!((keys.state(0).right_x, keys.state(0).right_y), (0, 32767));
    keys.key_down(0x26, 0);
    assert_eq!(
        (keys.state(0).right_x, keys.state(0).right_y),
        (-23170, 23170)
    );
    assert_eq!((keys.state(0).left_x, keys.state(0).left_y), (0, 0));
    // The defaults are replaced, not merged.
    assert!(!keys.key_down(keycode::ANSI_W, 0));
    keys.key_down(0x28, 0);
    assert_eq!(keys.state(0).left_trigger, 255);
}

#[test]
fn one_key_bound_to_two_targets_drives_both_and_is_reported() {
    let config = KeyboardConfig {
        bindings: vec![
            KeyBinding {
                key: keycode::SPACE,
                target: Target::Button(buttons::A),
            },
            KeyBinding {
                key: keycode::SPACE,
                target: Target::Button(buttons::X),
            },
            KeyBinding {
                key: keycode::RETURN,
                target: Target::Button(buttons::A),
            },
            KeyBinding {
                key: keycode::TAB,
                target: Target::RightTrigger,
            },
            KeyBinding {
                key: keycode::TAB,
                target: Target::LeftTrigger,
            },
        ],
        ..KeyboardConfig::default()
    };
    // Two keys sharing one target is normal; one key with two targets is a
    // duplicate binding that the settings UI warns about.
    assert_eq!(config.duplicate_keys(), vec![keycode::TAB, keycode::SPACE]);
    let mut keys = KeyboardTranslator::new(config);
    keys.key_down(keycode::SPACE, 0);
    assert_eq!(keys.state(0).buttons, buttons::A | buttons::X);
}

#[test]
fn default_bindings_have_no_duplicates_and_validate() {
    let config = KeyboardConfig::default();
    assert!(config.duplicate_keys().is_empty());
    assert_eq!(config.validate(), Ok(()));
    assert_eq!(config.ramp_ms, 0);
    assert_eq!(config.stick_magnitude, 1.0);
}

#[test]
fn validation_rejects_bad_targets_and_ranges() {
    let bad_button = KeyboardConfig {
        bindings: vec![KeyBinding {
            key: 0x00,
            target: Target::Button(buttons::A | buttons::B),
        }],
        ..KeyboardConfig::default()
    };
    assert!(
        bad_button.validate().is_err(),
        "a binding names exactly one button bit"
    );
    let unknown_bit = KeyboardConfig {
        bindings: vec![KeyBinding {
            key: 0x00,
            target: Target::Button(0x0800),
        }],
        ..KeyboardConfig::default()
    };
    assert!(
        unknown_bit.validate().is_err(),
        "0x0800 is not a public XInput button"
    );
    let zero = KeyboardConfig {
        bindings: vec![KeyBinding {
            key: 0x00,
            target: Target::Button(0),
        }],
        ..KeyboardConfig::default()
    };
    assert!(zero.validate().is_err());
    for magnitude in [-0.1, 1.5, f64::NAN, f64::INFINITY] {
        let config = KeyboardConfig {
            stick_magnitude: magnitude,
            ..KeyboardConfig::default()
        };
        assert!(config.validate().is_err(), "stick magnitude {magnitude}");
    }
    let slow = KeyboardConfig {
        ramp_ms: 10_001,
        ..KeyboardConfig::default()
    };
    assert!(slow.validate().is_err(), "ramps are bounded at ten seconds");
    let key_range = KeyboardConfig {
        bindings: vec![KeyBinding {
            key: 0x80,
            target: Target::LeftTrigger,
        }],
        ..KeyboardConfig::default()
    };
    assert!(
        key_range.validate().is_err(),
        "macOS virtual key codes stop at 0x7F"
    );
    let too_many = KeyboardConfig {
        bindings: (0..=64)
            .map(|key| KeyBinding {
                key: key % 0x80,
                target: Target::LeftTrigger,
            })
            .collect(),
        ..KeyboardConfig::default()
    };
    assert!(too_many.validate().is_err(), "at most 64 bindings");
}

#[test]
fn translator_from_invalid_config_is_refused() {
    let config = KeyboardConfig {
        stick_magnitude: 2.0,
        ..KeyboardConfig::default()
    };
    assert!(KeyboardTranslator::try_new(config).is_err());
    assert!(KeyboardTranslator::try_new(KeyboardConfig::default()).is_ok());
}
