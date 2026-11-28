// SPDX-License-Identifier: GPL-3.0-or-later

//! Mouse and trackpad to gamepad translation.

use crate::error::ConfigError;
use crate::pad::{PadState, axis, is_valid_button};

/// The largest number of bindings a mouse configuration may hold.
const MAX_BINDINGS: usize = 32;

/// The largest button number an AppKit event may carry.
const MAX_BUTTON: u8 = 31;

/// The largest decay interval a mouse configuration may hold, in milliseconds.
const MAX_DECAY_MS: u32 = 10_000;

/// What a mouse button drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseTarget {
    /// One public button bit.
    Button(u16),
    /// The left trigger.
    LeftTrigger,
    /// The right trigger.
    RightTrigger,
}

/// One binding: an AppKit button number and the gamepad target it drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MouseBinding {
    /// The AppKit button number: 0 is the primary button, 1 the secondary
    /// button and 2 the middle button.
    pub button: u8,
    /// What the button drives while it is held.
    pub target: MouseTarget,
}

/// The mouse configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct MouseConfig {
    /// Right stick deflection per point of horizontal motion.
    pub sensitivity_x: f64,
    /// Right stick deflection per point of vertical motion.
    pub sensitivity_y: f64,
    /// True when horizontal motion is inverted.
    pub invert_x: bool,
    /// True when vertical motion is inverted.
    pub invert_y: bool,
    /// The radial dead zone, as a fraction of a full deflection.
    pub dead_zone: f64,
    /// The share of each new sample that is smoothed away.
    pub smoothing: f64,
    /// The largest deflection the right stick may reach.
    pub max_magnitude: f64,
    /// The time without motion after which the right stick returns to neutral,
    /// in milliseconds.
    pub decay_ms: u32,
    /// The button bindings.
    pub bindings: Vec<MouseBinding>,
}

impl Default for MouseConfig {
    fn default() -> Self {
        Self {
            sensitivity_x: 0.01,
            sensitivity_y: 0.01,
            invert_x: false,
            invert_y: false,
            dead_zone: 0.0,
            smoothing: 0.0,
            max_magnitude: 1.0,
            decay_ms: 50,
            bindings: vec![
                MouseBinding {
                    button: 0,
                    target: MouseTarget::RightTrigger,
                },
                MouseBinding {
                    button: 1,
                    target: MouseTarget::LeftTrigger,
                },
            ],
        }
    }
}

impl MouseConfig {
    /// Checks the configuration against the rules of this crate.
    ///
    /// # Errors
    ///
    /// Returns a [ConfigError] when a sensitivity, the dead zone, the smoothing
    /// or the maximum magnitude is outside its range, when the decay interval
    /// is not between 1 and 10000 milliseconds, when there are more than 32
    /// bindings, when a button number is above 31, or when a button target does
    /// not name exactly one public XInput bit.
    pub fn validate(&self) -> Result<(), ConfigError> {
        check_unit_range("sensitivity_x", self.sensitivity_x, true)?;
        check_unit_range("sensitivity_y", self.sensitivity_y, true)?;
        check_unit_range("dead zone", self.dead_zone, false)?;
        check_unit_range("smoothing", self.smoothing, false)?;
        if !self.max_magnitude.is_finite()
            || !(0.0..=1.0).contains(&self.max_magnitude)
            || self.max_magnitude == 0.0
        {
            return Err(ConfigError::new(format!(
                "maximum magnitude {} is not a number above 0.0 and up to 1.0",
                self.max_magnitude
            )));
        }
        if !(1..=MAX_DECAY_MS).contains(&self.decay_ms) {
            return Err(ConfigError::new(format!(
                "decay interval of {} ms is not between 1 and {MAX_DECAY_MS} ms",
                self.decay_ms
            )));
        }
        if self.bindings.len() > MAX_BINDINGS {
            return Err(ConfigError::new(format!(
                "a mouse configuration holds at most {MAX_BINDINGS} bindings, this one holds {}",
                self.bindings.len()
            )));
        }
        for binding in &self.bindings {
            if binding.button > MAX_BUTTON {
                return Err(ConfigError::new(format!(
                    "button number {} is above the AppKit range",
                    binding.button
                )));
            }
            if let MouseTarget::Button(bit) = binding.target
                && !is_valid_button(bit)
            {
                return Err(ConfigError::new(format!(
                    "button target {bit:#06X} does not name exactly one public XInput bit"
                )));
            }
        }
        Ok(())
    }
}

/// Checks a number that has to be finite and inside the unit range.
///
/// The upper bound is inclusive for a sensitivity and exclusive for the dead
/// zone and the smoothing, which name the `inclusive` flag.
fn check_unit_range(name: &str, value: f64, inclusive: bool) -> Result<(), ConfigError> {
    let in_range = if inclusive {
        (0.0..=1.0).contains(&value)
    } else {
        (0.0..1.0).contains(&value)
    };
    if !value.is_finite() || !in_range {
        return Err(ConfigError::new(format!(
            "{name} of {value} is not a number inside the unit range"
        )));
    }
    Ok(())
}

/// Turns mouse events into a gamepad snapshot.
///
/// The translator holds no clock of its own: every call carries the current
/// time in milliseconds, so the same event script always produces the same
/// snapshot. Motion deltas follow the AppKit convention, with a positive x
/// pointing right and a positive y pointing down.
pub struct MouseTranslator {
    config: MouseConfig,
    current_x: f64,
    current_y: f64,
    last_motion: Option<u64>,
    held: u32,
}

impl MouseTranslator {
    /// Builds a translator from a configuration.
    ///
    /// # Errors
    ///
    /// Returns a [ConfigError] when the configuration is refused by
    /// [MouseConfig::validate].
    pub fn try_new(config: MouseConfig) -> Result<Self, ConfigError> {
        config.validate()?;
        Ok(Self {
            config,
            current_x: 0.0,
            current_y: 0.0,
            last_motion: None,
            held: 0,
        })
    }

    /// Builds a translator from a configuration, panicking on an invalid one.
    ///
    /// # Panics
    ///
    /// Panics when the configuration is refused by [MouseConfig::validate].
    pub fn new(config: MouseConfig) -> Self {
        match Self::try_new(config) {
            Ok(translator) => translator,
            Err(error) => panic!("invalid mouse configuration: {error}"),
        }
    }

    /// The configuration in use.
    pub fn config(&self) -> &MouseConfig {
        &self.config
    }

    /// Records a motion event.
    ///
    /// A delta that is not finite is ignored and changes nothing. A motion that
    /// arrives after the decay interval starts from a neutral stick.
    pub fn motion(&mut self, dx: f64, dy: f64, now_ms: u64) {
        if !dx.is_finite() || !dy.is_finite() {
            return;
        }
        let idle = match self.last_motion {
            Some(last) => now_ms.saturating_sub(last) >= u64::from(self.config.decay_ms),
            None => true,
        };
        if idle {
            self.current_x = 0.0;
            self.current_y = 0.0;
        }

        let mut raw_x = dx * self.config.sensitivity_x;
        let mut raw_y = -dy * self.config.sensitivity_y;
        if self.config.invert_x {
            raw_x = -raw_x;
        }
        if self.config.invert_y {
            raw_y = -raw_y;
        }

        let smoothing = self.config.smoothing;
        self.current_x = self.current_x * smoothing + raw_x * (1.0 - smoothing);
        self.current_y = self.current_y * smoothing + raw_y * (1.0 - smoothing);
        self.last_motion = Some(now_ms);
    }

    /// Records a button event.
    ///
    /// Returns false when the button has no binding, and then nothing changes.
    pub fn button(&mut self, button: u8, down: bool) -> bool {
        if !self.is_bound(button) {
            return false;
        }
        let mask = 1_u32 << u32::from(button);
        if down {
            self.held |= mask;
        } else {
            self.held &= !mask;
        }
        true
    }

    /// Releases every button and clears the motion.
    pub fn release_all(&mut self) {
        self.current_x = 0.0;
        self.current_y = 0.0;
        self.last_motion = None;
        self.held = 0;
    }

    /// True when at least one bound button is held.
    pub fn is_active(&self) -> bool {
        self.held != 0
    }

    /// The gamepad snapshot for the given time, in milliseconds.
    pub fn state(&self, now_ms: u64) -> PadState {
        let mut state = PadState::default();
        for binding in &self.config.bindings {
            if self.held & (1_u32 << u32::from(binding.button)) == 0 {
                continue;
            }
            match binding.target {
                MouseTarget::Button(bit) => state.buttons |= bit,
                MouseTarget::LeftTrigger => state.left_trigger = u8::MAX,
                MouseTarget::RightTrigger => state.right_trigger = u8::MAX,
            }
        }
        let (right_x, right_y) = self.right_stick(now_ms);
        state.right_x = right_x;
        state.right_y = right_y;
        state
    }

    /// The right stick deflection for the given time, in milliseconds.
    fn right_stick(&self, now_ms: u64) -> (i16, i16) {
        let (mut x, mut y) = match self.last_motion {
            Some(last) if now_ms.saturating_sub(last) < u64::from(self.config.decay_ms) => {
                (self.current_x, self.current_y)
            }
            _ => (0.0, 0.0),
        };
        let length = x.hypot(y);
        if length > self.config.max_magnitude {
            let scale = self.config.max_magnitude / length;
            x *= scale;
            y *= scale;
        }
        let length = x.hypot(y);
        if length < self.config.dead_zone {
            return (0, 0);
        }
        (axis(x), axis(y))
    }

    /// True when the button has a binding.
    fn is_bound(&self, button: u8) -> bool {
        self.config
            .bindings
            .iter()
            .any(|binding| binding.button == button)
    }
}
