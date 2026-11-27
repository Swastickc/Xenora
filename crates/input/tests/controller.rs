// SPDX-License-Identifier: GPL-3.0-or-later

//! Acceptance tests for physical controller recognition and translation
//! (Campaign F, F17 and F4).
//!
//! These are virtual-device tests: the raw element values stand in for what
//! Apple's GameController framework reports. No physical controller was
//! connected when they were written, so no model is hardware-verified here.

use xenora_input::buttons;
use xenora_input::controller::{Family, HostDescription, RawController, element, recognize};
use xenora_input::profile::{Curve, Profile, StickSettings, TriggerSettings};
use xenora_input::{PadState, Target};

fn host(category: &str) -> HostDescription<'_> {
    HostDescription {
        product_category: category,
        extended: true,
        has_paddles: false,
        has_share: false,
    }
}

fn raw() -> RawController {
    RawController::default()
}

#[test]
fn recognizes_the_documented_product_categories() {
    // Values of GCProductCategoryXboxOne, GCProductCategoryDualSense and
    // GCProductCategoryDualShock4 read from the macOS SDK at runtime.
    assert_eq!(recognize(&host("Xbox One")), Family::XboxOne);
    assert_eq!(
        recognize(&HostDescription {
            has_share: true,
            ..host("Xbox One")
        }),
        Family::XboxSeries
    );
    assert_eq!(
        recognize(&HostDescription {
            has_paddles: true,
            has_share: true,
            ..host("Xbox One")
        }),
        Family::XboxElite,
        "paddles take precedence over the Share button"
    );
    assert_eq!(recognize(&host("DualSense")), Family::DualSense);
    assert_eq!(recognize(&host("DualShock 4")), Family::DualShock4);
}

#[test]
fn unknown_categories_fall_back_without_guessing() {
    // There is no public GameController category constant for the Switch Pro
    // controller, so it is never recognized automatically.
    assert_eq!(recognize(&host("Pro Controller")), Family::GenericExtended);
    assert_eq!(recognize(&host("MFi")), Family::GenericExtended);
    assert_eq!(recognize(&host("HID")), Family::GenericExtended);
    assert_eq!(recognize(&host("")), Family::GenericExtended);
    assert_eq!(
        recognize(&HostDescription {
            extended: false,
            ..host("Xbox One")
        }),
        Family::Micro
    );
    // Matching is exact, not by substring or case.
    assert_eq!(recognize(&host("dualsense")), Family::GenericExtended);
    assert_eq!(recognize(&host("DualSense Edge")), Family::GenericExtended);
}

#[test]
fn family_names_are_stable_identifiers() {
    let all = [
        (Family::XboxSeries, "xbox-series"),
        (Family::XboxOne, "xbox-one"),
        (Family::XboxElite, "xbox-elite"),
        (Family::DualShock4, "dualshock4"),
        (Family::DualSense, "dualsense"),
        (Family::SwitchPro, "switch-pro"),
        (Family::GenericExtended, "generic-extended"),
        (Family::Micro, "micro"),
    ];
    for (family, id) in all {
        assert_eq!(family.id(), id);
        assert_eq!(Family::from_id(id), Some(family));
    }
    assert_eq!(Family::from_id("xbox"), None);
}

#[test]
fn element_bits_are_distinct() {
    let all = [
        element::SOUTH,
        element::EAST,
        element::WEST,
        element::NORTH,
        element::LEFT_SHOULDER,
        element::RIGHT_SHOULDER,
        element::LEFT_STICK,
        element::RIGHT_STICK,
        element::MENU,
        element::VIEW,
        element::HOME,
        element::DPAD_UP,
        element::DPAD_DOWN,
        element::DPAD_LEFT,
        element::DPAD_RIGHT,
        element::PADDLE_1,
        element::PADDLE_2,
        element::PADDLE_3,
        element::PADDLE_4,
        element::TOUCHPAD,
        element::SHARE,
    ];
    let mut seen = 0u32;
    for bit in all {
        assert_eq!(bit.count_ones(), 1);
        assert_eq!(seen & bit, 0);
        seen |= bit;
    }
    assert_eq!(seen, element::ALL);
}

#[test]
fn every_family_exposes_the_xbox_logical_layout_by_position() {
    // GameController names face buttons by position: A bottom, B right,
    // X left, Y top. PlayStation and Nintendo pads therefore reach the guest
    // as the Xbox layout without any keyboard emulation.
    let cases = [
        (element::SOUTH, buttons::A),
        (element::EAST, buttons::B),
        (element::WEST, buttons::X),
        (element::NORTH, buttons::Y),
        (element::LEFT_SHOULDER, buttons::LEFT_SHOULDER),
        (element::RIGHT_SHOULDER, buttons::RIGHT_SHOULDER),
        (element::LEFT_STICK, buttons::LEFT_THUMB),
        (element::RIGHT_STICK, buttons::RIGHT_THUMB),
        (element::MENU, buttons::START),
        (element::VIEW, buttons::BACK),
        (element::HOME, buttons::GUIDE),
        (element::DPAD_UP, buttons::DPAD_UP),
        (element::DPAD_DOWN, buttons::DPAD_DOWN),
        (element::DPAD_LEFT, buttons::DPAD_LEFT),
        (element::DPAD_RIGHT, buttons::DPAD_RIGHT),
    ];
    for family in Family::ALL {
        let profile = Profile::default_for(family);
        assert_eq!(profile.family, family);
        assert_eq!(profile.validate(), Ok(()));
        for (element, bit) in cases {
            let state = profile.translate(&RawController {
                elements: element,
                ..raw()
            });
            assert_eq!(state.buttons, bit, "{family:?} element {element:#x}");
        }
        // Extra elements are unbound by default.
        for extra in [
            element::PADDLE_1,
            element::PADDLE_4,
            element::TOUCHPAD,
            element::SHARE,
        ] {
            let state = profile.translate(&RawController {
                elements: extra,
                ..raw()
            });
            assert_eq!(state, PadState::default(), "{family:?} extra {extra:#x}");
        }
    }
}

#[test]
fn default_translation_is_full_precision_without_dead_zones() {
    // XInput applications apply their own dead zones, so the default profile
    // passes the full range through, matching the existing host bridge.
    let profile = Profile::default_for(Family::DualSense);
    let state = profile.translate(&RawController {
        left_stick: [1.0, -1.0],
        right_stick: [0.25, 0.001],
        left_trigger: 1.0,
        right_trigger: 0.5,
        ..raw()
    });
    // A full diagonal is passed through unclamped: each axis is independent.
    assert_eq!((state.left_x, state.left_y), (32767, -32768));
    // 0.25 * 32767 = 8191.75; 0.001 * 32767 = 32.767.
    assert_eq!((state.right_x, state.right_y), (8192, 33));
    // 0.5 * 255 = 127.5, rounded half away from zero.
    assert_eq!((state.left_trigger, state.right_trigger), (255, 128));
}

#[test]
fn non_finite_and_out_of_range_host_values_are_contained() {
    let profile = Profile::default_for(Family::XboxSeries);
    let state = profile.translate(&RawController {
        left_stick: [f64::NAN, 2.0],
        right_stick: [-3.0, f64::INFINITY],
        left_trigger: f64::NAN,
        right_trigger: -0.5,
        elements: u32::MAX,
    });
    assert_eq!((state.left_x, state.left_y), (0, 32767));
    assert_eq!((state.right_x, state.right_y), (-32768, 0));
    assert_eq!((state.left_trigger, state.right_trigger), (0, 0));
    // Unknown element bits outside element::ALL are ignored.
    let known = profile.translate(&RawController {
        elements: element::ALL,
        ..raw()
    });
    assert_eq!(state.buttons, known.buttons);
}

fn stick_profile(left: StickSettings) -> Profile {
    Profile {
        left_stick: left,
        ..Profile::default_for(Family::GenericExtended)
    }
}

#[test]
fn radial_dead_zone_and_outer_rescale() {
    let profile = stick_profile(StickSettings {
        dead_zone: 0.2,
        outer: 0.9,
        ..StickSettings::default()
    });
    // Inside the dead zone: length 0.1414 <= 0.2.
    let inside = profile.translate(&RawController {
        left_stick: [0.1, 0.1],
        ..raw()
    });
    assert_eq!((inside.left_x, inside.left_y), (0, 0));
    // On the axis: (0.55 - 0.2) / (0.9 - 0.2) = 0.5 -> 16384.
    let half = profile.translate(&RawController {
        left_stick: [0.55, 0.0],
        ..raw()
    });
    assert_eq!((half.left_x, half.left_y), (16384, 0));
    // Past the outer edge saturates at full scale along the same direction.
    let full = profile.translate(&RawController {
        left_stick: [0.0, -0.95],
        ..raw()
    });
    assert_eq!((full.left_x, full.left_y), (0, -32768));
    // A diagonal keeps its direction: (0.6, 0.8) has length 1 -> saturates.
    // 0.6 * 32767 = 19660.2; 0.8 * 32767 = 26213.6.
    let diagonal = profile.translate(&RawController {
        left_stick: [0.6, 0.8],
        ..raw()
    });
    assert_eq!((diagonal.left_x, diagonal.left_y), (19660, 26214));
    // The right stick keeps its own (default) settings.
    let right = profile.translate(&RawController {
        right_stick: [0.1, 0.0],
        ..raw()
    });
    assert_eq!(right.right_x, 3277);
}

#[test]
fn response_curves_shape_the_radial_magnitude() {
    for (curve, expected) in [
        (Curve::Linear, 16384),
        (Curve::Quadratic, 8192),
        (Curve::Cubic, 4096),
    ] {
        let profile = stick_profile(StickSettings {
            curve,
            ..StickSettings::default()
        });
        let state = profile.translate(&RawController {
            left_stick: [0.5, 0.0],
            ..raw()
        });
        // 0.5, 0.25 and 0.125 of full scale: 16383.5, 8191.75, 4095.875.
        assert_eq!(state.left_x, expected, "{curve:?}");
    }
}

#[test]
fn drift_calibration_recenters_before_the_dead_zone() {
    let profile = stick_profile(StickSettings {
        center: [0.05, -0.02],
        dead_zone: 0.06,
        ..StickSettings::default()
    });
    // The resting drift reads as exactly centred.
    let rest = profile.translate(&RawController {
        left_stick: [0.05, -0.02],
        ..raw()
    });
    assert_eq!((rest.left_x, rest.left_y), (0, 0));
    // (0.55 - 0.05) = 0.5 along x; (0.5 - 0.06) / (1 - 0.06) = 0.4680851 -> 15337.7.
    let moved = profile.translate(&RawController {
        left_stick: [0.55, -0.02],
        ..raw()
    });
    assert_eq!((moved.left_x, moved.left_y), (15338, 0));
}

#[test]
fn inversion_sensitivity_and_stick_swap() {
    let profile = Profile {
        left_stick: StickSettings {
            invert_y: true,
            sensitivity: 0.5,
            ..StickSettings::default()
        },
        right_stick: StickSettings {
            invert_x: true,
            ..StickSettings::default()
        },
        swap_sticks: true,
        ..Profile::default_for(Family::XboxOne)
    };
    let state = profile.translate(&RawController {
        left_stick: [0.0, 1.0],
        right_stick: [0.5, 0.0],
        ..raw()
    });
    // Swapping happens first: the host left stick reaches the guest right
    // stick and uses the right stick's settings, and vice versa.
    assert_eq!((state.right_x, state.right_y), (0, 32767));
    // Host right stick (0.5, 0) -> guest left: sensitivity 0.5 -> 0.25 -> 8192.
    assert_eq!((state.left_x, state.left_y), (8192, 0));
    let unswapped = Profile {
        swap_sticks: false,
        ..profile.clone()
    };
    let state = unswapped.translate(&RawController {
        left_stick: [0.0, 1.0],
        ..raw()
    });
    // invert_y and sensitivity 0.5: -0.5 * 32768 = -16384.
    assert_eq!((state.left_x, state.left_y), (0, -16384));
}

#[test]
fn trigger_threshold_rescales_and_digital_mode_saturates() {
    let profile = Profile {
        left_trigger: TriggerSettings {
            threshold: 0.5,
            digital: false,
        },
        right_trigger: TriggerSettings {
            threshold: 0.1,
            digital: true,
        },
        ..Profile::default_for(Family::DualShock4)
    };
    let below = profile.translate(&RawController {
        left_trigger: 0.5,
        right_trigger: 0.1,
        ..raw()
    });
    assert_eq!((below.left_trigger, below.right_trigger), (0, 0));
    // (0.75 - 0.5) / 0.5 = 0.5 -> 127.5 -> 128.
    let past = profile.translate(&RawController {
        left_trigger: 0.75,
        right_trigger: 0.11,
        ..raw()
    });
    assert_eq!((past.left_trigger, past.right_trigger), (128, 255));
}

#[test]
fn remapping_sends_elements_to_any_target() {
    let mut profile = Profile::default_for(Family::XboxElite);
    profile
        .bindings
        .retain(|binding| binding.0 != element::SOUTH);
    profile
        .bindings
        .push((element::SOUTH, Target::Button(buttons::B)));
    profile
        .bindings
        .push((element::PADDLE_1, Target::Button(buttons::A)));
    profile
        .bindings
        .push((element::PADDLE_2, Target::RightTrigger));
    profile.bindings.push((
        element::TOUCHPAD,
        Target::LeftStick(xenora_input::Direction::Left),
    ));
    assert_eq!(profile.validate(), Ok(()));
    let state = profile.translate(&RawController {
        elements: element::SOUTH | element::PADDLE_1 | element::PADDLE_2,
        ..raw()
    });
    assert_eq!(state.buttons, buttons::A | buttons::B);
    assert_eq!(state.right_trigger, 255);
    // A digital stick target overrides a smaller analog deflection on that stick.
    let state = profile.translate(&RawController {
        elements: element::TOUCHPAD,
        left_stick: [0.3, 0.0],
        ..raw()
    });
    assert_eq!((state.left_x, state.left_y), (-32768, 0));
}

#[test]
fn nintendo_label_layout_is_an_explicit_choice() {
    // The default Switch Pro profile is positional like every other family.
    let positional = Profile::default_for(Family::SwitchPro);
    let state = positional.translate(&RawController {
        elements: element::EAST,
        ..raw()
    });
    assert_eq!(state.buttons, buttons::B);
    // The label layout swaps A with B and X with Y to match printed labels.
    let labels = Profile::nintendo_labels();
    assert_eq!(labels.family, Family::SwitchPro);
    let state = labels.translate(&RawController {
        elements: element::EAST | element::NORTH,
        ..raw()
    });
    assert_eq!(state.buttons, buttons::A | buttons::X);
}

#[test]
fn validation_rejects_unsafe_profiles() {
    let base = Profile::default_for(Family::GenericExtended);
    let bad_sticks = [
        StickSettings {
            dead_zone: -0.1,
            ..StickSettings::default()
        },
        StickSettings {
            dead_zone: 0.5,
            outer: 0.5,
            ..StickSettings::default()
        },
        StickSettings {
            outer: 1.1,
            ..StickSettings::default()
        },
        StickSettings {
            sensitivity: 0.0,
            ..StickSettings::default()
        },
        StickSettings {
            sensitivity: 2.5,
            ..StickSettings::default()
        },
        StickSettings {
            center: [0.6, 0.0],
            ..StickSettings::default()
        },
        StickSettings {
            center: [0.0, f64::NAN],
            ..StickSettings::default()
        },
    ];
    for (index, stick) in bad_sticks.into_iter().enumerate() {
        let profile = Profile {
            right_stick: stick,
            ..base.clone()
        };
        assert!(profile.validate().is_err(), "stick case {index}");
    }
    for threshold in [-0.1, 1.0, f64::NAN] {
        let profile = Profile {
            left_trigger: TriggerSettings {
                threshold,
                digital: false,
            },
            ..base.clone()
        };
        assert!(profile.validate().is_err(), "threshold {threshold}");
    }
    for vibration in [-0.1, 1.01, f64::INFINITY] {
        let profile = Profile {
            vibration,
            ..base.clone()
        };
        assert!(profile.validate().is_err(), "vibration {vibration}");
    }
    let mut unknown_element = base.clone();
    unknown_element
        .bindings
        .push((1 << 30, Target::LeftTrigger));
    assert!(unknown_element.validate().is_err());
    let mut two_elements = base.clone();
    two_elements
        .bindings
        .push((element::SOUTH | element::EAST, Target::LeftTrigger));
    assert!(
        two_elements.validate().is_err(),
        "a binding names one element"
    );
    let mut bad_button = base.clone();
    bad_button
        .bindings
        .push((element::SHARE, Target::Button(0x0800)));
    assert!(bad_button.validate().is_err());
    let mut many = base.clone();
    many.bindings = (0..65)
        .map(|_| (element::SOUTH, Target::LeftTrigger))
        .collect();
    assert!(many.validate().is_err(), "at most 64 bindings");
    for name in ["", "line\nbreak", &"n".repeat(65)] {
        let profile = Profile {
            name: name.to_string(),
            ..base.clone()
        };
        assert!(profile.validate().is_err(), "name {name:?}");
    }
}

#[test]
fn vibration_intensity_scales_motor_speeds() {
    let profile = Profile {
        vibration: 0.5,
        ..Profile::default_for(Family::DualSense)
    };
    assert_eq!(profile.scale_motor(65535), 32768);
    assert_eq!(profile.scale_motor(0), 0);
    let off = Profile {
        vibration: 0.0,
        ..profile.clone()
    };
    assert_eq!(off.scale_motor(65535), 0);
    let full = Profile::default_for(Family::DualSense);
    assert_eq!(full.vibration, 1.0);
    assert_eq!(full.scale_motor(40000), 40000);
}
