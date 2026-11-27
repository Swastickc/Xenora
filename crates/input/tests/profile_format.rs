// SPDX-License-Identifier: GPL-3.0-or-later

//! Acceptance tests for the versioned profile text format (Campaign F, F4).

use xenora_input::buttons;
use xenora_input::controller::{Family, element};
use xenora_input::profile::{Curve, PROFILE_VERSION, Profile, StickSettings, TriggerSettings};
use xenora_input::{Direction, Target};

#[test]
fn current_version_is_one() {
    assert_eq!(PROFILE_VERSION, 1);
}

#[test]
fn every_default_profile_round_trips() {
    for family in Family::ALL {
        let profile = Profile::default_for(family);
        let text = profile.to_text();
        assert!(text.starts_with("xenora-input-profile 1\n"), "{text}");
        assert_eq!(Profile::from_text(&text), Ok(profile), "{family:?}");
    }
    let labels = Profile::nintendo_labels();
    assert_eq!(Profile::from_text(&labels.to_text()), Ok(labels));
}

#[test]
fn a_customized_profile_round_trips_exactly() {
    let mut profile = Profile {
        name: "Living room pad".to_string(),
        left_stick: StickSettings {
            dead_zone: 0.08,
            outer: 0.95,
            curve: Curve::Quadratic,
            invert_x: false,
            invert_y: true,
            center: [0.013, -0.0042],
            sensitivity: 1.25,
        },
        right_trigger: TriggerSettings {
            threshold: 0.15,
            digital: true,
        },
        swap_sticks: true,
        vibration: 0.3,
        ..Profile::default_for(Family::XboxElite)
    };
    profile
        .bindings
        .push((element::PADDLE_1, Target::Button(buttons::A)));
    profile
        .bindings
        .push((element::PADDLE_2, Target::RightStick(Direction::Down)));
    profile
        .bindings
        .push((element::PADDLE_3, Target::LeftTrigger));
    let text = profile.to_text();
    assert_eq!(Profile::from_text(&text), Ok(profile));
}

#[test]
fn a_hand_written_profile_parses_with_family_defaults_for_missing_keys() {
    let text = "\
# A comment, then the header after blank lines.

xenora-input-profile 1
name = Couch
family = dualsense
left.dead_zone = 0.1
left.curve = cubic
left.center = 0.02 -0.01
right_trigger.threshold = 0.25
vibration = 0.5
";
    let profile = Profile::from_text(text).expect("valid profile");
    let defaults = Profile::default_for(Family::DualSense);
    assert_eq!(profile.name, "Couch");
    assert_eq!(profile.family, Family::DualSense);
    assert_eq!(profile.left_stick.dead_zone, 0.1);
    assert_eq!(profile.left_stick.curve, Curve::Cubic);
    assert_eq!(profile.left_stick.center, [0.02, -0.01]);
    assert_eq!(profile.left_stick.outer, defaults.left_stick.outer);
    assert_eq!(profile.right_trigger.threshold, 0.25);
    assert_eq!(profile.vibration, 0.5);
    assert_eq!(profile.right_stick, defaults.right_stick);
    // Without bind lines the family's default bindings apply.
    assert_eq!(profile.bindings, defaults.bindings);
}

#[test]
fn bind_lines_replace_the_default_bindings() {
    let text = "\
xenora-input-profile 1
name = Two buttons
family = generic-extended
bind south = button b
bind east = button a
bind touchpad = left-stick left
bind share = right-trigger
";
    let profile = Profile::from_text(text).expect("valid profile");
    assert_eq!(
        profile.bindings,
        vec![
            (element::SOUTH, Target::Button(buttons::B)),
            (element::EAST, Target::Button(buttons::A)),
            (element::TOUCHPAD, Target::LeftStick(Direction::Left)),
            (element::SHARE, Target::RightTrigger),
        ]
    );
}

#[test]
fn malformed_profiles_are_rejected_with_a_reason() {
    let header = "xenora-input-profile 1\nname = x\nfamily = dualsense\n";
    let cases: Vec<(String, &str)> = vec![
        (String::new(), "empty"),
        ("name = x\n".to_string(), "missing header"),
        (
            "xenora-input-profile 2\nname = x\nfamily = dualsense\n".to_string(),
            "newer version",
        ),
        (
            "xenora-input-profile 0\nname = x\nfamily = dualsense\n".to_string(),
            "version zero",
        ),
        (
            "xenora-input-profile one\n".to_string(),
            "non-numeric version",
        ),
        (
            "xenora-input-profile 1\nfamily = dualsense\n".to_string(),
            "missing name",
        ),
        (
            "xenora-input-profile 1\nname = x\n".to_string(),
            "missing family",
        ),
        (format!("{header}name = y\n"), "duplicate key"),
        (
            "xenora-input-profile 1\nname = x\nfamily = playstation\n".to_string(),
            "unknown family",
        ),
        (format!("{header}colour = red\n"), "unknown key"),
        (format!("{header}left.dead_zone\n"), "no equals sign"),
        (format!("{header}left.dead_zone = lots\n"), "not a number"),
        (
            format!("{header}left.dead_zone = NaN\n"),
            "non-finite number",
        ),
        (
            format!("{header}left.dead_zone = 0.99\nleft.outer = 0.5\n"),
            "dead zone past outer edge",
        ),
        (format!("{header}left.invert_x = yes\n"), "not a boolean"),
        (format!("{header}left.curve = sigmoid\n"), "unknown curve"),
        (
            format!("{header}left.center = 0.1\n"),
            "center needs two numbers",
        ),
        (format!("{header}bind south = button z\n"), "unknown button"),
        (format!("{header}bind chin = button a\n"), "unknown element"),
        (
            format!("{header}bind south = left-stick sideways\n"),
            "unknown direction",
        ),
        (format!("{header}vibration = 2\n"), "vibration out of range"),
        (format!("{header}\u{0}\n"), "control character"),
    ];
    for (text, why) in cases {
        let result = Profile::from_text(&text);
        assert!(result.is_err(), "{why}: {text:?} parsed as {result:?}");
        assert!(!result.unwrap_err().message().is_empty(), "{why}");
    }
}

#[test]
fn newer_versions_say_so() {
    let error =
        Profile::from_text("xenora-input-profile 7\nname = x\nfamily = micro\n").unwrap_err();
    assert!(error.message().contains("version 7"), "{}", error.message());
}

#[test]
fn input_size_is_bounded() {
    let mut text = String::from("xenora-input-profile 1\nname = x\nfamily = micro\n");
    while text.len() <= 16 * 1024 {
        text.push_str("# padding comment line to exceed the size limit\n");
    }
    assert!(Profile::from_text(&text).is_err(), "over 16 KiB");
    let many_binds: String =
        std::iter::once("xenora-input-profile 1\nname = x\nfamily = micro\n".to_string())
            .chain((0..65).map(|_| "bind south = button a\n".to_string()))
            .collect();
    assert!(Profile::from_text(&many_binds).is_err(), "over 64 bindings");
}

#[test]
fn whitespace_and_comments_are_tolerated() {
    let text = "xenora-input-profile 1\r\n  name   =   Spaced name  \r\n\tfamily=micro\n# trailing comment\n";
    let profile = Profile::from_text(text).expect("valid profile");
    assert_eq!(profile.name, "Spaced name");
    assert_eq!(profile.family, Family::Micro);
}
