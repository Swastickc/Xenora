// SPDX-License-Identifier: GPL-3.0-or-later

//! The versioned text form of a controller profile.
//!
//! The format is line oriented UTF-8 text. A file holds one profile:
//!
//! ```text
//! # A comment. Blank lines and comments are ignored everywhere.
//! xenora-input-profile 1
//! name = Couch pad
//! family = dualsense
//! swap_sticks = false
//! vibration = 1
//! left.dead_zone = 0
//! left.outer = 1
//! left.curve = linear
//! left.invert_x = false
//! left.invert_y = false
//! left.center = 0 0
//! left.sensitivity = 1
//! bind south = button a
//! ```
//!
//! The first line that is neither blank nor a comment must be the header
//! `xenora-input-profile <version>`. This build reads version 1. A version
//! above the version this build supports is refused, and so is version zero
//! and a version that is not a number.
//!
//! Every other line is a `key = value` pair, split at the first equals sign
//! with both sides trimmed. A line without an equals sign is refused. A key
//! may appear once; the `bind` lines are the exception and may appear as
//! often as the profile needs.
//!
//! The `name` and `family` keys are required. Every other key that is absent
//! takes the value it has in the default profile of the family.
//!
//! - `name` is the name of the profile.
//! - `family` is one of the family identifiers.
//! - `swap_sticks` is `true` or `false`.
//! - `vibration` is a number from 0 up to 1.
//! - `left` and `right` each take `dead_zone`, `outer`, `curve`, `invert_x`,
//!   `invert_y`, `center` and `sensitivity`. The centre is two numbers.
//! - `left_trigger` and `right_trigger` each take `threshold` and `digital`.
//! - `bind <element> = <target>` binds one element to one target. The element
//!   is one of `south`, `east`, `west`, `north`, `left-shoulder`,
//!   `right-shoulder`, `left-stick`, `right-stick`, `menu`, `view`, `home`,
//!   `dpad-up`, `dpad-down`, `dpad-left`, `dpad-right`, `paddle1`, `paddle2`,
//!   `paddle3`, `paddle4`, `touchpad` and `share`. The target is `button
//!   <name>`, `left-trigger`, `right-trigger`, `left-stick <direction>` or
//!   `right-stick <direction>`.
//!
//! A file with at least one bind line holds exactly those bindings, in the
//! order the file lists them. A file with no bind line holds the bindings of
//! the default profile of its family.
//!
//! A profile is written back in the same order every time, so a profile that
//! is written and read again is the same profile.

use super::{Curve, PROFILE_VERSION, Profile};
use crate::ConfigError;
use crate::buttons;
use crate::controller::{Family, element};
use crate::pad::{Direction, Target};

/// The largest text a profile file may hold, in bytes.
const MAX_BYTES: usize = 16 * 1024;

/// The largest number of lines a profile file may hold.
const MAX_LINES: usize = 256;

/// The text identifier of every element.
const ELEMENTS: [(u32, &str); 21] = [
    (element::SOUTH, "south"),
    (element::EAST, "east"),
    (element::WEST, "west"),
    (element::NORTH, "north"),
    (element::LEFT_SHOULDER, "left-shoulder"),
    (element::RIGHT_SHOULDER, "right-shoulder"),
    (element::LEFT_STICK, "left-stick"),
    (element::RIGHT_STICK, "right-stick"),
    (element::MENU, "menu"),
    (element::VIEW, "view"),
    (element::HOME, "home"),
    (element::DPAD_UP, "dpad-up"),
    (element::DPAD_DOWN, "dpad-down"),
    (element::DPAD_LEFT, "dpad-left"),
    (element::DPAD_RIGHT, "dpad-right"),
    (element::PADDLE_1, "paddle1"),
    (element::PADDLE_2, "paddle2"),
    (element::PADDLE_3, "paddle3"),
    (element::PADDLE_4, "paddle4"),
    (element::TOUCHPAD, "touchpad"),
    (element::SHARE, "share"),
];

/// The text identifier of every public button.
const BUTTONS: [(u16, &str); 15] = [
    (buttons::A, "a"),
    (buttons::B, "b"),
    (buttons::X, "x"),
    (buttons::Y, "y"),
    (buttons::LEFT_SHOULDER, "left-shoulder"),
    (buttons::RIGHT_SHOULDER, "right-shoulder"),
    (buttons::LEFT_THUMB, "left-thumb"),
    (buttons::RIGHT_THUMB, "right-thumb"),
    (buttons::START, "start"),
    (buttons::BACK, "back"),
    (buttons::GUIDE, "guide"),
    (buttons::DPAD_UP, "dpad-up"),
    (buttons::DPAD_DOWN, "dpad-down"),
    (buttons::DPAD_LEFT, "dpad-left"),
    (buttons::DPAD_RIGHT, "dpad-right"),
];

/// The text identifier of every direction.
const DIRECTIONS: [(Direction, &str); 4] = [
    (Direction::Up, "up"),
    (Direction::Down, "down"),
    (Direction::Left, "left"),
    (Direction::Right, "right"),
];

/// Writes the profile as text.
pub(crate) fn to_text(profile: &Profile) -> String {
    let mut out = String::new();
    out.push_str(&format!("xenora-input-profile {PROFILE_VERSION}\n"));
    out.push_str(&format!("name = {}\n", profile.name));
    out.push_str(&format!("family = {}\n", profile.family.id()));
    out.push_str(&format!("swap_sticks = {}\n", profile.swap_sticks));
    out.push_str(&format!("vibration = {}\n", profile.vibration));
    for (label, settings) in [("left", profile.left_stick), ("right", profile.right_stick)] {
        out.push_str(&format!("{label}.dead_zone = {}\n", settings.dead_zone));
        out.push_str(&format!("{label}.outer = {}\n", settings.outer));
        out.push_str(&format!("{label}.curve = {}\n", settings.curve.id()));
        out.push_str(&format!("{label}.invert_x = {}\n", settings.invert_x));
        out.push_str(&format!("{label}.invert_y = {}\n", settings.invert_y));
        out.push_str(&format!(
            "{label}.center = {} {}\n",
            settings.center[0], settings.center[1]
        ));
        out.push_str(&format!("{label}.sensitivity = {}\n", settings.sensitivity));
    }
    for (label, settings) in [
        ("left_trigger", profile.left_trigger),
        ("right_trigger", profile.right_trigger),
    ] {
        out.push_str(&format!("{label}.threshold = {}\n", settings.threshold));
        out.push_str(&format!("{label}.digital = {}\n", settings.digital));
    }
    for (bit, target) in &profile.bindings {
        out.push_str(&format!(
            "bind {} = {}\n",
            element_name(*bit),
            target_name(target)
        ));
    }
    out
}

/// Reads a profile from text.
pub(crate) fn from_text(text: &str) -> Result<Profile, ConfigError> {
    if text.len() > MAX_BYTES {
        return Err(ConfigError::new("the profile text is longer than 16 KiB"));
    }
    if text
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\t' | '\r' | '\n'))
    {
        return Err(ConfigError::new(
            "the profile text holds a control character",
        ));
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() > MAX_LINES {
        return Err(ConfigError::new(
            "the profile text holds more than 256 lines",
        ));
    }

    let mut index = 0;
    while index < lines.len() && is_ignored(lines[index]) {
        index += 1;
    }
    if index >= lines.len() {
        return Err(ConfigError::new("the profile text has no header line"));
    }
    read_header(lines[index].trim())?;
    index += 1;

    let mut keys: Vec<String> = Vec::new();
    let mut values: Vec<String> = Vec::new();
    let mut bindings: Vec<(u32, Target)> = Vec::new();
    for line in &lines[index..] {
        if is_ignored(line) {
            continue;
        }
        let (key, value) = line
            .trim()
            .split_once('=')
            .ok_or_else(|| ConfigError::new("every profile line must be a key = value pair"))?;
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() {
            return Err(ConfigError::new("a profile line has no key"));
        }
        if let Some(name) = key.strip_prefix("bind ") {
            let bit = element_by_id(name.trim()).ok_or_else(|| {
                ConfigError::new(format!("unknown controller element {}", name.trim()))
            })?;
            bindings.push((bit, read_target(value)?));
            continue;
        }
        if keys.iter().any(|seen| seen == key) {
            return Err(ConfigError::new(format!("the key {key} appears twice")));
        }
        keys.push(key.to_string());
        values.push(value.to_string());
    }
    if bindings.len() > 64 {
        return Err(ConfigError::new("a profile holds at most 64 bindings"));
    }

    let family = match lookup(&keys, &values, "family") {
        Some(value) => Family::from_id(value)
            .ok_or_else(|| ConfigError::new(format!("unknown controller family {value}")))?,
        None => return Err(ConfigError::new("the profile has no family line")),
    };
    let name = match lookup(&keys, &values, "name") {
        Some(value) => value.to_string(),
        None => return Err(ConfigError::new("the profile has no name line")),
    };

    let mut profile = Profile::default_for(family);
    profile.name = name;
    for (index, key) in keys.iter().enumerate() {
        let value = values[index].as_str();
        match key.as_str() {
            "name" | "family" => {}
            "swap_sticks" => profile.swap_sticks = read_bool(value)?,
            "vibration" => profile.vibration = read_number(value)?,
            other => read_setting(&mut profile, other, value)?,
        }
    }
    if !bindings.is_empty() {
        profile.bindings = bindings;
    }
    profile.validate()?;
    Ok(profile)
}

fn is_ignored(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.is_empty() || trimmed.starts_with('#')
}

fn lookup<'a>(keys: &'a [String], values: &'a [String], key: &str) -> Option<&'a str> {
    keys.iter()
        .position(|seen| seen == key)
        .map(|index| values[index].as_str())
}

fn read_header(line: &str) -> Result<u32, ConfigError> {
    let Some(rest) = line.strip_prefix("xenora-input-profile") else {
        return Err(ConfigError::new(
            "the first line must be the header xenora-input-profile <version>",
        ));
    };
    let rest = rest.trim();
    if rest.is_empty() {
        return Err(ConfigError::new(
            "the profile header needs a version number",
        ));
    }
    let version: u32 = rest
        .parse()
        .map_err(|_| ConfigError::new("the profile header version is not a number"))?;
    if version == 0 {
        return Err(ConfigError::new("profile version zero is not a version"));
    }
    if version > PROFILE_VERSION {
        return Err(ConfigError::new(format!(
            "profile version {version} is newer than this build supports"
        )));
    }
    Ok(version)
}

fn read_setting(profile: &mut Profile, key: &str, value: &str) -> Result<(), ConfigError> {
    let (scope, field) = key
        .split_once('.')
        .ok_or_else(|| ConfigError::new(format!("unknown key {key}")))?;
    match scope {
        "left" | "right" => {
            let settings = if scope == "left" {
                &mut profile.left_stick
            } else {
                &mut profile.right_stick
            };
            match field {
                "dead_zone" => settings.dead_zone = read_number(value)?,
                "outer" => settings.outer = read_number(value)?,
                "curve" => {
                    settings.curve = Curve::from_id(value)
                        .ok_or_else(|| ConfigError::new(format!("unknown curve {value}")))?;
                }
                "invert_x" => settings.invert_x = read_bool(value)?,
                "invert_y" => settings.invert_y = read_bool(value)?,
                "center" => settings.center = read_center(value)?,
                "sensitivity" => settings.sensitivity = read_number(value)?,
                _ => return Err(ConfigError::new(format!("unknown key {key}"))),
            }
        }
        "left_trigger" | "right_trigger" => {
            let settings = if scope == "left_trigger" {
                &mut profile.left_trigger
            } else {
                &mut profile.right_trigger
            };
            match field {
                "threshold" => settings.threshold = read_number(value)?,
                "digital" => settings.digital = read_bool(value)?,
                _ => return Err(ConfigError::new(format!("unknown key {key}"))),
            }
        }
        _ => return Err(ConfigError::new(format!("unknown key {key}"))),
    }
    Ok(())
}

fn read_number(value: &str) -> Result<f64, ConfigError> {
    let parsed: f64 = value
        .parse()
        .map_err(|_| ConfigError::new(format!("{value} is not a number")))?;
    if !parsed.is_finite() {
        return Err(ConfigError::new(format!("{value} is not a finite number")));
    }
    Ok(parsed)
}

fn read_bool(value: &str) -> Result<bool, ConfigError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ConfigError::new(format!("{value} is not true or false"))),
    }
}

fn read_center(value: &str) -> Result<[f64; 2], ConfigError> {
    let mut parts = value.split_whitespace();
    let x = parts
        .next()
        .ok_or_else(|| ConfigError::new("the centre needs two numbers"))?;
    let y = parts
        .next()
        .ok_or_else(|| ConfigError::new("the centre needs two numbers"))?;
    if parts.next().is_some() {
        return Err(ConfigError::new("the centre takes exactly two numbers"));
    }
    Ok([read_number(x)?, read_number(y)?])
}

fn read_target(value: &str) -> Result<Target, ConfigError> {
    let mut parts = value.split_whitespace();
    let kind = parts
        .next()
        .ok_or_else(|| ConfigError::new("a bind line needs a target"))?;
    let argument = parts.next();
    if parts.next().is_some() {
        return Err(ConfigError::new("a bind target takes one argument"));
    }
    match kind {
        "button" => {
            let name =
                argument.ok_or_else(|| ConfigError::new("a button target needs a button name"))?;
            let bit = button_by_id(name)
                .ok_or_else(|| ConfigError::new(format!("unknown button {name}")))?;
            Ok(Target::Button(bit))
        }
        "left-trigger" => match argument {
            Some(_) => Err(ConfigError::new(
                "the left trigger target takes no argument",
            )),
            None => Ok(Target::LeftTrigger),
        },
        "right-trigger" => match argument {
            Some(_) => Err(ConfigError::new(
                "the right trigger target takes no argument",
            )),
            None => Ok(Target::RightTrigger),
        },
        "left-stick" => {
            let direction = read_direction(argument)?;
            Ok(Target::LeftStick(direction))
        }
        "right-stick" => {
            let direction = read_direction(argument)?;
            Ok(Target::RightStick(direction))
        }
        _ => Err(ConfigError::new(format!("unknown target {kind}"))),
    }
}

fn read_direction(argument: Option<&str>) -> Result<Direction, ConfigError> {
    let name = argument.ok_or_else(|| {
        ConfigError::new("a stick target needs a direction: up, down, left or right")
    })?;
    direction_by_id(name).ok_or_else(|| ConfigError::new(format!("unknown direction {name}")))
}

fn element_by_id(id: &str) -> Option<u32> {
    ELEMENTS
        .iter()
        .find(|(_, name)| *name == id)
        .map(|(bit, _)| *bit)
}

fn element_name(bit: u32) -> &'static str {
    ELEMENTS
        .iter()
        .find(|(known, _)| *known == bit)
        .map(|(_, name)| *name)
        .unwrap_or("unknown-element")
}

fn button_by_id(id: &str) -> Option<u16> {
    BUTTONS
        .iter()
        .find(|(_, name)| *name == id)
        .map(|(bit, _)| *bit)
}

fn direction_by_id(id: &str) -> Option<Direction> {
    DIRECTIONS
        .iter()
        .find(|(_, name)| *name == id)
        .map(|(direction, _)| *direction)
}

fn direction_name(direction: Direction) -> &'static str {
    DIRECTIONS
        .iter()
        .find(|(known, _)| *known == direction)
        .map(|(_, name)| *name)
        .unwrap_or("up")
}

fn target_name(target: &Target) -> String {
    match target {
        Target::Button(button) => {
            let name = BUTTONS
                .iter()
                .find(|(known, _)| known == button)
                .map(|(_, name)| *name)
                .unwrap_or("unknown-button");
            format!("button {name}")
        }
        Target::LeftTrigger => "left-trigger".to_string(),
        Target::RightTrigger => "right-trigger".to_string(),
        Target::LeftStick(direction) => format!("left-stick {}", direction_name(*direction)),
        Target::RightStick(direction) => format!("right-stick {}", direction_name(*direction)),
    }
}
