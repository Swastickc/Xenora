// SPDX-License-Identifier: GPL-3.0-or-later

//! What a physical controller means to a guest.
//!
//! A profile is the translation between the elements a host controller
//! reports and the public XInput layout a guest expects. It names the family
//! the profile was written for, maps the positional elements to buttons,
//! triggers and stick directions, shapes the two thumb sticks, shapes the two
//! triggers and scales the vibration motors.
//!
//! # Dead zones
//!
//! The dead zone of a thumb stick defaults to zero. XInput applications
//! apply their own dead zones to the axis values they read, so a profile that
//! removed part of the range by itself would make the stick feel weaker than
//! the application expects. A profile that wants a dead zone asks for one
//! explicitly.
//!
//! # Text format
//!
//! A profile can be written as text and read back. The format is line
//! oriented, versioned and documented in the `text` module.
//!
//! # Evidence
//!
//! The element names, the family names and the product categories come from
//! the public macOS SDK header documentation for GameController. That is
//! documentation evidence, not hardware evidence: no controller has been
//! connected to the machine that wrote this crate, and no profile has been
//! tested against a physical pad.

mod text;

use crate::buttons;
use crate::controller::{Family, RawController, element};
use crate::pad::{Direction, PadState, Target, axis, is_valid_button};
use crate::{ConfigError, merge};

/// The version of the profile text format this build reads and writes.
pub const PROFILE_VERSION: u32 = 1;

/// The shape of the response of a thumb stick.
///
/// The curve shapes the magnitude of the stick after the dead zone, not its
/// direction: a quadratic curve moves halfway to the edge at half the
/// deflection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Curve {
    /// The deflection reaches the edge at the same rate.
    #[default]
    Linear,
    /// The deflection grows more slowly near the centre.
    Quadratic,
    /// The deflection grows more slowly still near the centre.
    Cubic,
}

impl Curve {
    /// The stable text identifier of the curve.
    pub fn id(self) -> &'static str {
        match self {
            Curve::Linear => "linear",
            Curve::Quadratic => "quadratic",
            Curve::Cubic => "cubic",
        }
    }

    /// The curve with the given text identifier.
    pub fn from_id(id: &str) -> Option<Curve> {
        match id {
            "linear" => Some(Curve::Linear),
            "quadratic" => Some(Curve::Quadratic),
            "cubic" => Some(Curve::Cubic),
            _ => None,
        }
    }
}

/// How one thumb stick is read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StickSettings {
    /// The radial dead zone, from 0 up to the outer edge.
    pub dead_zone: f64,
    /// The deflection that reaches the edge of the range.
    pub outer: f64,
    /// The shape of the response between the dead zone and the edge.
    pub curve: Curve,
    /// True when the X axis is mirrored.
    pub invert_x: bool,
    /// True when the Y axis is mirrored.
    pub invert_y: bool,
    /// The resting reading of the stick, removed before anything else.
    pub center: [f64; 2],
    /// The scale applied after the curve, from above 0 up to 2.
    pub sensitivity: f64,
}

impl Default for StickSettings {
    fn default() -> Self {
        Self {
            dead_zone: 0.0,
            outer: 1.0,
            curve: Curve::Linear,
            invert_x: false,
            invert_y: false,
            center: [0.0, 0.0],
            sensitivity: 1.0,
        }
    }
}

/// How one trigger is read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TriggerSettings {
    /// The reading below which the trigger reads as released.
    pub threshold: f64,
    /// True when the trigger reports fully pressed or fully released only.
    pub digital: bool,
}

/// What a physical controller means to a guest.
///
/// The profile is the whole translation: the family it was written for, the
/// elements it binds, the shape of the two thumb sticks, the shape of the two
/// triggers, the order of the thumb sticks and the strength of the vibration
/// motors.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// The name shown to a person choosing a profile.
    pub name: String,
    /// The family the profile was written for.
    pub family: Family,
    /// The element bindings, in the order they are applied.
    pub bindings: Vec<(u32, Target)>,
    /// How the guest's left thumb stick is read.
    pub left_stick: StickSettings,
    /// How the guest's right thumb stick is read.
    pub right_stick: StickSettings,
    /// True when the two thumb sticks trade places.
    pub swap_sticks: bool,
    /// How the left trigger is read.
    pub left_trigger: TriggerSettings,
    /// How the right trigger is read.
    pub right_trigger: TriggerSettings,
    /// The strength of the vibration motors, from 0 up to 1.
    pub vibration: f64,
}

impl Profile {
    /// The profile of a family that follows the positional layout.
    ///
    /// Every family binds the same fifteen elements: the four face buttons
    /// by position, the two shoulders, the two thumb stick clicks, the menu,
    /// the view and the home buttons and the four directions of the
    /// directional pad. The paddle buttons, the touchpad and the Share
    /// button are left unbound, because a guest that expects them asks for
    /// them in a profile of its own.
    pub fn default_for(family: Family) -> Profile {
        let name = match family {
            Family::XboxSeries => "Xbox Series X|S controller",
            Family::XboxOne => "Xbox One controller",
            Family::XboxElite => "Xbox Elite controller",
            Family::DualShock4 => "DualShock 4",
            Family::DualSense => "DualSense",
            Family::SwitchPro => "Switch Pro controller",
            Family::GenericExtended => "Generic controller",
            Family::Micro => "Micro gamepad",
        };
        let bindings = vec![
            (element::SOUTH, Target::Button(buttons::A)),
            (element::EAST, Target::Button(buttons::B)),
            (element::WEST, Target::Button(buttons::X)),
            (element::NORTH, Target::Button(buttons::Y)),
            (
                element::LEFT_SHOULDER,
                Target::Button(buttons::LEFT_SHOULDER),
            ),
            (
                element::RIGHT_SHOULDER,
                Target::Button(buttons::RIGHT_SHOULDER),
            ),
            (element::LEFT_STICK, Target::Button(buttons::LEFT_THUMB)),
            (element::RIGHT_STICK, Target::Button(buttons::RIGHT_THUMB)),
            (element::MENU, Target::Button(buttons::START)),
            (element::VIEW, Target::Button(buttons::BACK)),
            (element::HOME, Target::Button(buttons::GUIDE)),
            (element::DPAD_UP, Target::Button(buttons::DPAD_UP)),
            (element::DPAD_DOWN, Target::Button(buttons::DPAD_DOWN)),
            (element::DPAD_LEFT, Target::Button(buttons::DPAD_LEFT)),
            (element::DPAD_RIGHT, Target::Button(buttons::DPAD_RIGHT)),
        ];
        Profile {
            name: name.to_string(),
            family,
            bindings,
            left_stick: StickSettings::default(),
            right_stick: StickSettings::default(),
            swap_sticks: false,
            left_trigger: TriggerSettings::default(),
            right_trigger: TriggerSettings::default(),
            vibration: 1.0,
        }
    }

    /// The Switch Pro profile that follows the labels printed on the pad.
    ///
    /// The framework reports the face buttons by position, so the default
    /// Switch Pro profile reaches the guest in the Xbox layout. This profile
    /// instead follows the letters printed on the Nintendo pad, which swaps
    /// the bottom and the right buttons and the left and the top buttons.
    pub fn nintendo_labels() -> Profile {
        let mut profile = Profile::default_for(Family::SwitchPro);
        profile.name = "Switch Pro controller (printed labels)".to_string();
        for binding in &mut profile.bindings {
            binding.1 = match binding.0 {
                element::SOUTH => Target::Button(buttons::B),
                element::EAST => Target::Button(buttons::A),
                element::WEST => Target::Button(buttons::Y),
                element::NORTH => Target::Button(buttons::X),
                _ => binding.1,
            };
        }
        profile
    }

    /// Checks that the profile is one a caller can use.
    ///
    /// The error says which rule was broken.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.name.is_empty() || self.name.chars().count() > 64 {
            return Err(ConfigError::new(
                "the profile name must be 1 to 64 characters long",
            ));
        }
        if self.name.trim() != self.name {
            return Err(ConfigError::new(
                "the profile name must not begin or end with whitespace",
            ));
        }
        if self.name.chars().any(char::is_control) {
            return Err(ConfigError::new(
                "the profile name must not hold a control character",
            ));
        }
        if self.bindings.len() > 64 {
            return Err(ConfigError::new("a profile holds at most 64 bindings"));
        }
        for (bit, target) in &self.bindings {
            if *bit == 0 || bit.count_ones() != 1 || (bit & !element::ALL) != 0 {
                return Err(ConfigError::new(
                    "a binding names exactly one known controller element",
                ));
            }
            check_target(target)?;
        }
        check_stick(&self.left_stick, "left")?;
        check_stick(&self.right_stick, "right")?;
        check_trigger(&self.left_trigger, "left")?;
        check_trigger(&self.right_trigger, "right")?;
        if !self.vibration.is_finite() || !(0.0..=1.0).contains(&self.vibration) {
            return Err(ConfigError::new(
                "the vibration strength must be a number from 0 up to 1",
            ));
        }
        Ok(())
    }

    /// Turns one host snapshot into the guest's gamepad snapshot.
    ///
    /// A host reading that is not a number reads as zero. The thumb sticks are
    /// shaped by their own settings, the triggers by their own settings and
    /// the pressed elements by the bindings. A binding that drives a thumb
    /// stick contributes a digital direction, and the digital direction wins
    /// over a smaller analog deflection of the same stick.
    pub fn translate(&self, raw: &RawController) -> PadState {
        let elements = raw.elements & element::ALL;
        let host_left = sanitize(raw.left_stick);
        let host_right = sanitize(raw.right_stick);
        let (first, second) = if self.swap_sticks {
            (host_right, host_left)
        } else {
            (host_left, host_right)
        };

        let left = apply_stick(&self.left_stick, first);
        let right = apply_stick(&self.right_stick, second);
        let mut state = PadState {
            left_x: left.0,
            left_y: left.1,
            right_x: right.0,
            right_y: right.1,
            ..PadState::default()
        };

        let mut digital_left = [0.0_f64, 0.0_f64];
        let mut digital_right = [0.0_f64, 0.0_f64];
        for (bit, target) in &self.bindings {
            if elements & bit == 0 {
                continue;
            }
            match target {
                Target::Button(button) => state.buttons |= *button,
                Target::LeftTrigger => state.left_trigger = 255,
                Target::RightTrigger => state.right_trigger = 255,
                Target::LeftStick(direction) => push_direction(&mut digital_left, *direction),
                Target::RightStick(direction) => push_direction(&mut digital_right, *direction),
            }
        }

        state.left_trigger = state
            .left_trigger
            .max(trigger_value(&self.left_trigger, raw.left_trigger));
        state.right_trigger = state
            .right_trigger
            .max(trigger_value(&self.right_trigger, raw.right_trigger));

        let digital_left = digital_axis(digital_left);
        let digital_right = digital_axis(digital_right);
        let digital = PadState {
            left_x: digital_left.0,
            left_y: digital_left.1,
            right_x: digital_right.0,
            right_y: digital_right.1,
            ..PadState::default()
        };

        merge(&[state, digital])
    }

    /// Scales a motor speed by the strength of the profile.
    ///
    /// The result is the speed the guest asked for, rounded and limited to
    /// the range of a motor speed.
    pub fn scale_motor(&self, speed: u16) -> u16 {
        let scaled = (f64::from(speed) * self.vibration).round();
        scaled.clamp(0.0, f64::from(u16::MAX)) as u16
    }

    /// Writes the profile as text.
    pub fn to_text(&self) -> String {
        text::to_text(self)
    }

    /// Reads a profile from text.
    ///
    /// The text is checked for size, then parsed, then validated. The error
    /// says which step refused the text.
    pub fn from_text(text: &str) -> Result<Profile, ConfigError> {
        text::from_text(text)
    }
}

fn check_target(target: &Target) -> Result<(), ConfigError> {
    let button = match target {
        Target::Button(button) => *button,
        _ => return Ok(()),
    };
    if is_valid_button(button) {
        Ok(())
    } else {
        Err(ConfigError::new(
            "a button binding must name exactly one public button bit",
        ))
    }
}

fn check_stick(settings: &StickSettings, label: &str) -> Result<(), ConfigError> {
    if !settings.dead_zone.is_finite() || settings.dead_zone < 0.0 {
        return Err(ConfigError::new(format!(
            "the {label} dead zone must be a number from 0 up"
        )));
    }
    if !settings.outer.is_finite() || settings.outer > 1.0 || settings.outer <= settings.dead_zone {
        return Err(ConfigError::new(format!(
            "the {label} outer edge must be above the dead zone and no more than 1"
        )));
    }
    if !settings.sensitivity.is_finite()
        || settings.sensitivity <= 0.0
        || settings.sensitivity > 2.0
    {
        return Err(ConfigError::new(format!(
            "the {label} sensitivity must be a number above 0 and no more than 2"
        )));
    }
    for reading in settings.center {
        if !reading.is_finite() || reading.abs() > 0.5 {
            return Err(ConfigError::new(format!(
                "the {label} centre reading must be a number no larger than 0.5 in either direction"
            )));
        }
    }
    Ok(())
}

fn check_trigger(settings: &TriggerSettings, label: &str) -> Result<(), ConfigError> {
    if !settings.threshold.is_finite() || settings.threshold < 0.0 || settings.threshold >= 1.0 {
        return Err(ConfigError::new(format!(
            "the {label} trigger threshold must be a number from 0 up to below 1"
        )));
    }
    Ok(())
}

fn sanitize(raw: [f64; 2]) -> [f64; 2] {
    let mut out = [0.0_f64, 0.0_f64];
    for (index, value) in raw.iter().enumerate() {
        if value.is_finite() {
            out[index] = *value;
        }
    }
    out
}

fn apply_stick(settings: &StickSettings, raw: [f64; 2]) -> (i16, i16) {
    let mut value = [raw[0] - settings.center[0], raw[1] - settings.center[1]];
    if settings.invert_x {
        value[0] = -value[0];
    }
    if settings.invert_y {
        value[1] = -value[1];
    }
    let length = value[0].hypot(value[1]);
    if length <= settings.dead_zone {
        return (0, 0);
    }
    // The factor that takes the corrected reading to the output. It is
    // computed as one ratio so that the default settings give exactly 1.0
    // (x / x is exact), which makes the default profile a bit-exact
    // per-axis passthrough of the host values.
    let scale = if length <= settings.outer {
        let ratio = (length - settings.dead_zone) / (settings.outer - settings.dead_zone);
        let shaped = match settings.curve {
            Curve::Linear => ratio,
            Curve::Quadratic => ratio * ratio,
            Curve::Cubic => ratio * ratio * ratio,
        };
        shaped / length
    } else if length >= 1.0 {
        // Past the outer edge and outside the unit circle: keep the reading.
        1.0
    } else {
        // Past the outer edge but inside the unit circle: saturate to length 1.
        1.0 / length
    } * settings.sensitivity;
    (axis(value[0] * scale), axis(value[1] * scale))
}

fn trigger_value(settings: &TriggerSettings, raw: f64) -> u8 {
    let value = if raw.is_finite() {
        raw.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if value <= settings.threshold {
        return 0;
    }
    if settings.digital {
        return 255;
    }
    let scaled = ((value - settings.threshold) / (1.0 - settings.threshold) * 255.0).round();
    scaled.clamp(0.0, 255.0) as u8
}

fn push_direction(vector: &mut [f64; 2], direction: Direction) {
    match direction {
        Direction::Up => vector[1] += 1.0,
        Direction::Down => vector[1] -= 1.0,
        Direction::Left => vector[0] -= 1.0,
        Direction::Right => vector[0] += 1.0,
    }
}

fn digital_axis(vector: [f64; 2]) -> (i16, i16) {
    let length = vector[0].hypot(vector[1]);
    if length == 0.0 {
        return (0, 0);
    }
    let scale = if length > 1.0 { 1.0 / length } else { 1.0 };
    (axis(vector[0] * scale), axis(vector[1] * scale))
}
