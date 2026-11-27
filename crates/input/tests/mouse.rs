// SPDX-License-Identifier: GPL-3.0-or-later

//! Acceptance tests for the mouse and trackpad translator (Campaign F, F2, #944).
//!
//! Host deltas use the AppKit convention: +x right, +y down. XInput thumb Y is
//! positive upward, so a downward motion deflects the right stick down.

use xenora_input::PadState;
use xenora_input::buttons;
use xenora_input::mouse::{MouseBinding, MouseConfig, MouseTarget, MouseTranslator};

fn plain() -> MouseConfig {
    MouseConfig {
        sensitivity_x: 0.01,
        sensitivity_y: 0.01,
        invert_x: false,
        invert_y: false,
        dead_zone: 0.0,
        smoothing: 0.0,
        max_magnitude: 1.0,
        decay_ms: 50,
        bindings: Vec::new(),
    }
}

#[test]
fn defaults_are_valid_and_documented() {
    let config = MouseConfig::default();
    assert_eq!(config.validate(), Ok(()));
    assert_eq!(config.decay_ms, 50);
    assert!(!config.invert_x && !config.invert_y);
    // Default buttons: primary click is the right trigger, secondary the left.
    assert_eq!(
        config.bindings,
        vec![
            MouseBinding {
                button: 0,
                target: MouseTarget::RightTrigger
            },
            MouseBinding {
                button: 1,
                target: MouseTarget::LeftTrigger
            },
        ]
    );
}

#[test]
fn starts_neutral_and_motion_deflects_the_right_stick() {
    let mut mouse = MouseTranslator::new(plain());
    assert_eq!(mouse.state(0), PadState::default());
    mouse.motion(50.0, 0.0, 10);
    let state = mouse.state(10);
    // 50 * 0.01 = 0.5; 0.5 * 32767 = 16383.5 -> 16384.
    assert_eq!((state.right_x, state.right_y), (16384, 0));
    assert_eq!((state.left_x, state.left_y), (0, 0));
    mouse.motion(0.0, 25.0, 20);
    // Downward motion: -25 * 0.01 = -0.25; -0.25 * 32768 = -8192.
    assert_eq!(
        (mouse.state(20).right_x, mouse.state(20).right_y),
        (0, -8192)
    );
}

#[test]
fn deflection_is_clamped_to_unit_length() {
    let mut mouse = MouseTranslator::new(plain());
    mouse.motion(300.0, -400.0, 0);
    // (3, 4) has length 5 -> (0.6, 0.8); 0.6 * 32767 = 19660.2, 0.8 * 32767 = 26213.6.
    assert_eq!(
        (mouse.state(0).right_x, mouse.state(0).right_y),
        (19660, 26214)
    );
}

#[test]
fn per_axis_sensitivity_and_inversion() {
    let config = MouseConfig {
        sensitivity_x: 0.02,
        sensitivity_y: 0.005,
        invert_x: true,
        invert_y: true,
        ..plain()
    };
    let mut mouse = MouseTranslator::new(config);
    mouse.motion(10.0, 40.0, 0);
    // x: 10 * 0.02 = 0.2, inverted -0.2 -> -6553.6 -> -6554.
    // y: -(40 * 0.005) = -0.2, inverted +0.2 -> 6553.4 -> 6553.
    assert_eq!(
        (mouse.state(0).right_x, mouse.state(0).right_y),
        (-6554, 6553)
    );
}

#[test]
fn returns_to_neutral_after_the_decay_interval() {
    let mut mouse = MouseTranslator::new(plain());
    mouse.motion(50.0, 0.0, 100);
    assert_eq!(mouse.state(149).right_x, 16384);
    assert_eq!(mouse.state(150), PadState::default());
    assert_eq!(mouse.state(10_000), PadState::default());
}

#[test]
fn radial_dead_zone_suppresses_small_motion() {
    let config = MouseConfig {
        dead_zone: 0.1,
        ..plain()
    };
    let mut mouse = MouseTranslator::new(config);
    mouse.motion(6.0, 6.0, 0);
    // Length of (0.06, -0.06) is 0.0849 < 0.1.
    assert_eq!(mouse.state(0), PadState::default());
    mouse.motion(10.0, 0.0, 1);
    // Exactly on the dead zone boundary passes: 0.1 * 32767 = 3276.7 -> 3277.
    assert_eq!(mouse.state(1).right_x, 3277);
}

#[test]
fn maximum_magnitude_caps_deflection() {
    let config = MouseConfig {
        max_magnitude: 0.5,
        ..plain()
    };
    let mut mouse = MouseTranslator::new(config);
    mouse.motion(-200.0, 0.0, 0);
    // -0.5 * 32768 = -16384.
    assert_eq!(mouse.state(0).right_x, -16384);
}

#[test]
fn smoothing_blends_successive_samples() {
    let config = MouseConfig {
        smoothing: 0.5,
        ..plain()
    };
    let mut mouse = MouseTranslator::new(config);
    mouse.motion(40.0, 0.0, 0);
    // current = 0 * 0.5 + 0.4 * 0.5 = 0.2 -> 6553.4 -> 6553.
    assert_eq!(mouse.state(0).right_x, 6553);
    mouse.motion(40.0, 0.0, 10);
    // current = 0.2 * 0.5 + 0.4 * 0.5 = 0.3 -> 9830.1 -> 9830.
    assert_eq!(mouse.state(10).right_x, 9830);
    // After the decay interval the smoothed value restarts from zero.
    mouse.motion(40.0, 0.0, 100);
    assert_eq!(mouse.state(100).right_x, 6553);
}

#[test]
fn mouse_buttons_follow_their_bindings() {
    let config = MouseConfig {
        bindings: vec![
            MouseBinding {
                button: 0,
                target: MouseTarget::RightTrigger,
            },
            MouseBinding {
                button: 1,
                target: MouseTarget::LeftTrigger,
            },
            MouseBinding {
                button: 2,
                target: MouseTarget::Button(buttons::RIGHT_THUMB),
            },
        ],
        ..plain()
    };
    let mut mouse = MouseTranslator::new(config);
    assert!(mouse.button(0, true));
    assert!(mouse.button(2, true));
    assert!(!mouse.button(4, true), "button 4 is unbound");
    let state = mouse.state(0);
    assert_eq!((state.left_trigger, state.right_trigger), (0, 255));
    assert_eq!(state.buttons, buttons::RIGHT_THUMB);
    // Buttons do not decay with motion.
    assert_eq!(mouse.state(1_000).right_trigger, 255);
    assert!(mouse.is_active());
    mouse.button(0, false);
    mouse.button(2, false);
    assert_eq!(mouse.state(1_000), PadState::default());
    assert!(!mouse.is_active());
}

#[test]
fn release_all_clears_motion_and_buttons() {
    let mut mouse = MouseTranslator::new(MouseConfig::default());
    mouse.motion(30.0, 30.0, 0);
    mouse.button(0, true);
    mouse.release_all();
    assert_eq!(mouse.state(0), PadState::default());
    assert!(!mouse.is_active());
}

#[test]
fn non_finite_deltas_are_ignored() {
    let mut mouse = MouseTranslator::new(plain());
    mouse.motion(50.0, 0.0, 0);
    mouse.motion(f64::NAN, 0.0, 1);
    mouse.motion(0.0, f64::INFINITY, 2);
    assert_eq!(mouse.state(2).right_x, 16384);
    assert_eq!(mouse.state(2).right_y, 0);
}

#[test]
fn validation_rejects_out_of_range_settings() {
    let cases = [
        MouseConfig {
            sensitivity_x: -0.01,
            ..plain()
        },
        MouseConfig {
            sensitivity_y: f64::NAN,
            ..plain()
        },
        MouseConfig {
            sensitivity_x: 1.5,
            ..plain()
        },
        MouseConfig {
            dead_zone: 1.0,
            ..plain()
        },
        MouseConfig {
            dead_zone: -0.1,
            ..plain()
        },
        MouseConfig {
            smoothing: 1.0,
            ..plain()
        },
        MouseConfig {
            max_magnitude: 0.0,
            ..plain()
        },
        MouseConfig {
            max_magnitude: 1.1,
            ..plain()
        },
        MouseConfig {
            decay_ms: 0,
            ..plain()
        },
        MouseConfig {
            decay_ms: 10_001,
            ..plain()
        },
        MouseConfig {
            bindings: vec![MouseBinding {
                button: 0,
                target: MouseTarget::Button(0x0003),
            }],
            ..plain()
        },
        MouseConfig {
            bindings: vec![MouseBinding {
                button: 32,
                target: MouseTarget::LeftTrigger,
            }],
            ..plain()
        },
    ];
    for (index, config) in cases.into_iter().enumerate() {
        assert!(config.validate().is_err(), "case {index}");
        assert!(MouseTranslator::try_new(config).is_err(), "case {index}");
    }
    assert_eq!(plain().validate(), Ok(()));
}
