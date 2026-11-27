// SPDX-License-Identifier: GPL-3.0-or-later

//! Gaps found while reviewing PR 2 (#948).

use xenora_input::controller::{Family, RawController};
use xenora_input::profile::Profile;

/// The existing host bridge's axis rule, written out independently: clamp to
/// [-1, 1], scale positive values by 32767 and negative ones by 32768, round
/// half away from zero.
fn bridge_axis(value: f64) -> i16 {
    let clamped = value.clamp(-1.0, 1.0);
    let scaled = if clamped < 0.0 {
        clamped * 32768.0
    } else {
        clamped * 32767.0
    };
    scaled.round() as i16
}

#[test]
fn default_profiles_pass_axis_values_through_exactly() {
    // Every GameController axis value is a 32-bit float, so sweep f32 values,
    // including the ones whose scaled value sits next to a rounding boundary.
    let mut values: Vec<f64> = (-2000..=2000)
        .map(|step| f64::from(step as f32 / 2000.0))
        .collect();
    for unit in [32767.0_f64, 32768.0] {
        for k in [1.0_f64, 2.0, 3.0, 1001.0, 16383.0, 32766.0] {
            for sign in [1.0, -1.0] {
                values.push(f64::from(((k + 0.5) / unit * sign) as f32));
            }
        }
    }
    for family in Family::ALL {
        let profile = Profile::default_for(family);
        for &x in &values {
            for y in [0.0, 0.3, -1.0, x] {
                let state = profile.translate(&RawController {
                    left_stick: [x, y],
                    right_stick: [y, x],
                    ..RawController::default()
                });
                assert_eq!(
                    (state.left_x, state.left_y, state.right_x, state.right_y),
                    (
                        bridge_axis(x),
                        bridge_axis(y),
                        bridge_axis(y),
                        bridge_axis(x)
                    ),
                    "{family:?} at ({x}, {y})"
                );
            }
        }
    }
}

#[test]
fn a_profile_survives_a_text_round_trip_after_every_family_change() {
    for family in Family::ALL {
        let mut profile = Profile::default_for(family);
        profile.left_stick.dead_zone = 0.1;
        profile.right_trigger.threshold = 0.33;
        profile.vibration = 0.125;
        let text = profile.to_text();
        assert_eq!(Profile::from_text(&text), Ok(profile.clone()), "{text}");
        // The text never carries a control character other than line feeds.
        assert!(!text.chars().any(|c| c.is_control() && c != '\n'));
    }
}
