// SPDX-License-Identifier: GPL-3.0-or-later

//! Keyboard to gamepad translation.

use crate::error::ConfigError;
use crate::pad::{Direction, PadState, Target, axis, buttons, is_valid_button};

/// macOS virtual key codes, as published in Carbon `Events.h`.
///
/// A binding names the physical key, not the character it produces. The same
/// physical key drives the same target on every keyboard layout, and modifier
/// state does not change the binding.
pub mod keycode {
    /// `kVK_ANSI_A`.
    pub const ANSI_A: u16 = 0x00;
    /// `kVK_ANSI_S`.
    pub const ANSI_S: u16 = 0x01;
    /// `kVK_ANSI_D`.
    pub const ANSI_D: u16 = 0x02;
    /// `kVK_ANSI_Z`.
    pub const ANSI_Z: u16 = 0x06;
    /// `kVK_ANSI_X`.
    pub const ANSI_X: u16 = 0x07;
    /// `kVK_ANSI_C`.
    pub const ANSI_C: u16 = 0x08;
    /// `kVK_ANSI_V`.
    pub const ANSI_V: u16 = 0x09;
    /// `kVK_ANSI_Q`.
    pub const ANSI_Q: u16 = 0x0C;
    /// `kVK_ANSI_W`.
    pub const ANSI_W: u16 = 0x0D;
    /// `kVK_ANSI_E`.
    pub const ANSI_E: u16 = 0x0E;
    /// `kVK_ANSI_Y`.
    pub const ANSI_Y: u16 = 0x10;
    /// `kVK_Return`.
    pub const RETURN: u16 = 0x24;
    /// `kVK_Tab`.
    pub const TAB: u16 = 0x30;
    /// `kVK_Space`.
    pub const SPACE: u16 = 0x31;
    /// `kVK_Delete`, the backspace key.
    pub const DELETE: u16 = 0x33;
    /// `kVK_Escape`.
    pub const ESCAPE: u16 = 0x35;
    /// `kVK_Shift`, the left shift key.
    pub const SHIFT: u16 = 0x38;
    /// `kVK_RightShift`.
    pub const RIGHT_SHIFT: u16 = 0x3C;
    /// `kVK_LeftArrow`.
    pub const LEFT_ARROW: u16 = 0x7B;
    /// `kVK_RightArrow`.
    pub const RIGHT_ARROW: u16 = 0x7C;
    /// `kVK_DownArrow`.
    pub const DOWN_ARROW: u16 = 0x7D;
    /// `kVK_UpArrow`.
    pub const UP_ARROW: u16 = 0x7E;
}

/// The largest number of bindings a keyboard configuration may hold.
const MAX_BINDINGS: usize = 64;

/// The largest ramp length a keyboard configuration may hold, in milliseconds.
const MAX_RAMP_MS: u32 = 10_000;

/// The largest macOS virtual key code.
const MAX_KEY_CODE: u16 = 0x7F;

/// One binding: a physical macOS key and the gamepad target it drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyBinding {
    /// The macOS virtual key code of the physical key.
    pub key: u16,
    /// What the key drives while it is held.
    pub target: Target,
}

/// The keyboard configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyboardConfig {
    /// The bindings, in the order the caller listed them.
    pub bindings: Vec<KeyBinding>,
    /// The linear ramp time for stick directions, in milliseconds. Zero means
    /// an instant deflection.
    pub ramp_ms: u32,
    /// The scale applied to keyboard stick deflection, from 0.0 to 1.0.
    pub stick_magnitude: f64,
}

impl Default for KeyboardConfig {
    fn default() -> Self {
        let bindings = vec![
            KeyBinding {
                key: keycode::ANSI_W,
                target: Target::LeftStick(Direction::Up),
            },
            KeyBinding {
                key: keycode::ANSI_A,
                target: Target::LeftStick(Direction::Left),
            },
            KeyBinding {
                key: keycode::ANSI_S,
                target: Target::LeftStick(Direction::Down),
            },
            KeyBinding {
                key: keycode::ANSI_D,
                target: Target::LeftStick(Direction::Right),
            },
            KeyBinding {
                key: keycode::RETURN,
                target: Target::Button(buttons::A),
            },
            KeyBinding {
                key: keycode::SPACE,
                target: Target::Button(buttons::A),
            },
            KeyBinding {
                key: keycode::ESCAPE,
                target: Target::Button(buttons::B),
            },
            KeyBinding {
                key: keycode::DELETE,
                target: Target::Button(buttons::B),
            },
            KeyBinding {
                key: keycode::ANSI_X,
                target: Target::Button(buttons::X),
            },
            KeyBinding {
                key: keycode::ANSI_Y,
                target: Target::Button(buttons::Y),
            },
            KeyBinding {
                key: keycode::UP_ARROW,
                target: Target::Button(buttons::DPAD_UP),
            },
            KeyBinding {
                key: keycode::DOWN_ARROW,
                target: Target::Button(buttons::DPAD_DOWN),
            },
            KeyBinding {
                key: keycode::LEFT_ARROW,
                target: Target::Button(buttons::DPAD_LEFT),
            },
            KeyBinding {
                key: keycode::RIGHT_ARROW,
                target: Target::Button(buttons::DPAD_RIGHT),
            },
            KeyBinding {
                key: keycode::ANSI_Q,
                target: Target::Button(buttons::LEFT_SHOULDER),
            },
            KeyBinding {
                key: keycode::ANSI_E,
                target: Target::Button(buttons::RIGHT_SHOULDER),
            },
            KeyBinding {
                key: keycode::ANSI_Z,
                target: Target::LeftTrigger,
            },
            KeyBinding {
                key: keycode::ANSI_C,
                target: Target::RightTrigger,
            },
            KeyBinding {
                key: keycode::TAB,
                target: Target::Button(buttons::START),
            },
            KeyBinding {
                key: keycode::ANSI_V,
                target: Target::Button(buttons::BACK),
            },
            KeyBinding {
                key: keycode::SHIFT,
                target: Target::Button(buttons::LEFT_THUMB),
            },
            KeyBinding {
                key: keycode::RIGHT_SHIFT,
                target: Target::Button(buttons::RIGHT_THUMB),
            },
        ];
        Self {
            bindings,
            ramp_ms: 0,
            stick_magnitude: 1.0,
        }
    }
}

impl KeyboardConfig {
    /// Checks the configuration against the rules of this crate.
    ///
    /// # Errors
    ///
    /// Returns a [ConfigError] when there are more than 64 bindings, when a key
    /// code is above 0x7F, when a button target does not name exactly one
    /// public XInput bit, when the ramp is longer than ten seconds, or when the
    /// stick magnitude is not a number from 0.0 to 1.0.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.bindings.len() > MAX_BINDINGS {
            return Err(ConfigError::new(format!(
                "a keyboard configuration holds at most {MAX_BINDINGS} bindings, this one holds {}",
                self.bindings.len()
            )));
        }
        for binding in &self.bindings {
            if binding.key > MAX_KEY_CODE {
                return Err(ConfigError::new(format!(
                    "key code {:#04X} is above the macOS virtual key range",
                    binding.key
                )));
            }
            if let Target::Button(bit) = binding.target
                && !is_valid_button(bit)
            {
                return Err(ConfigError::new(format!(
                    "button target {bit:#06X} does not name exactly one public XInput bit"
                )));
            }
        }
        if self.ramp_ms > MAX_RAMP_MS {
            return Err(ConfigError::new(format!(
                "ramp of {} ms is longer than the {MAX_RAMP_MS} ms limit",
                self.ramp_ms
            )));
        }
        if !self.stick_magnitude.is_finite() || !(0.0..=1.0).contains(&self.stick_magnitude) {
            return Err(ConfigError::new(format!(
                "stick magnitude {} is not a number from 0.0 to 1.0",
                self.stick_magnitude
            )));
        }
        Ok(())
    }

    /// The keys that drive more than one target.
    ///
    /// A key bound twice to the same target is not a duplicate. Two keys that
    /// share one target are not duplicates either. The list is sorted and holds
    /// each key once, so a settings screen can warn about it.
    pub fn duplicate_keys(&self) -> Vec<u16> {
        let mut keys: Vec<u16> = Vec::new();
        for binding in &self.bindings {
            let conflicting = self
                .bindings
                .iter()
                .any(|other| other.key == binding.key && other.target != binding.target);
            if conflicting && !keys.contains(&binding.key) {
                keys.push(binding.key);
            }
        }
        keys.sort_unstable();
        keys
    }
}

/// One held key and the time it was pressed, in milliseconds.
#[derive(Clone, Copy, Debug)]
struct Held {
    key: u16,
    pressed_at: u64,
}

/// Turns keyboard events into a gamepad snapshot.
///
/// The translator holds no clock of its own: every call carries the current
/// time in milliseconds, so the same event script always produces the same
/// snapshot.
pub struct KeyboardTranslator {
    config: KeyboardConfig,
    held: Vec<Held>,
}

impl KeyboardTranslator {
    /// Builds a translator from a configuration.
    ///
    /// # Errors
    ///
    /// Returns a [ConfigError] when the configuration is refused by
    /// [KeyboardConfig::validate].
    pub fn try_new(config: KeyboardConfig) -> Result<Self, ConfigError> {
        config.validate()?;
        Ok(Self {
            config,
            held: Vec::new(),
        })
    }

    /// Builds a translator from a configuration, panicking on an invalid one.
    ///
    /// # Panics
    ///
    /// Panics when the configuration is refused by [KeyboardConfig::validate].
    pub fn new(config: KeyboardConfig) -> Self {
        match Self::try_new(config) {
            Ok(translator) => translator,
            Err(error) => panic!("invalid keyboard configuration: {error}"),
        }
    }

    /// The configuration in use.
    pub fn config(&self) -> &KeyboardConfig {
        &self.config
    }

    /// True when the key has a binding.
    pub(crate) fn is_bound(&self, key: u16) -> bool {
        self.config
            .bindings
            .iter()
            .any(|binding| binding.key == key)
    }

    /// True when the key is currently held.
    pub(crate) fn is_held(&self, key: u16) -> bool {
        self.held.iter().any(|held| held.key == key)
    }

    /// Records a key-down event.
    ///
    /// Returns false when the key has no binding, and then nothing changes. A
    /// repeated key-down for a key that is already held keeps the original
    /// press time, so it does not restart the ramp.
    pub fn key_down(&mut self, key: u16, now_ms: u64) -> bool {
        if !self.is_bound(key) {
            return false;
        }
        if self.is_held(key) {
            return true;
        }
        self.held.push(Held {
            key,
            pressed_at: now_ms,
        });
        true
    }

    /// Records a key-up event.
    ///
    /// Returns false when the key has no binding, and then nothing changes. A
    /// key-up for a key that is not held changes nothing.
    pub fn key_up(&mut self, key: u16, _now_ms: u64) -> bool {
        // A released key drives nothing, so the release time is unused.
        if !self.is_bound(key) {
            return false;
        }
        if let Some(index) = self.held.iter().position(|held| held.key == key) {
            self.held.remove(index);
        }
        true
    }

    /// Releases every held key.
    pub fn release_all(&mut self) {
        self.held.clear();
    }

    /// True when at least one bound key is held.
    pub fn is_active(&self) -> bool {
        !self.held.is_empty()
    }

    /// The gamepad snapshot for the given time, in milliseconds.
    pub fn state(&self, now_ms: u64) -> PadState {
        let mut state = PadState::default();
        let mut left = [0.0_f64; 4];
        let mut right = [0.0_f64; 4];

        for held in &self.held {
            let factor = self.ramp_factor(now_ms, held.pressed_at);
            for binding in self.config.bindings.iter().filter(|b| b.key == held.key) {
                match binding.target {
                    Target::Button(bit) => state.buttons |= bit,
                    Target::LeftTrigger => state.left_trigger = u8::MAX,
                    Target::RightTrigger => state.right_trigger = u8::MAX,
                    Target::LeftStick(direction) => {
                        let index = direction_index(direction);
                        left[index] = left[index].max(factor);
                    }
                    Target::RightStick(direction) => {
                        let index = direction_index(direction);
                        right[index] = right[index].max(factor);
                    }
                }
            }
        }

        let (left_x, left_y) = stick_vector(&left, self.config.stick_magnitude);
        let (right_x, right_y) = stick_vector(&right, self.config.stick_magnitude);
        state.left_x = left_x;
        state.left_y = left_y;
        state.right_x = right_x;
        state.right_y = right_y;
        state
    }

    /// The share of a full deflection a key has reached at the given time.
    fn ramp_factor(&self, now_ms: u64, pressed_at: u64) -> f64 {
        if self.config.ramp_ms == 0 {
            return 1.0;
        }
        let elapsed = now_ms.saturating_sub(pressed_at) as f64;
        (elapsed / f64::from(self.config.ramp_ms)).min(1.0)
    }
}

/// The index of a direction in the per-direction factor arrays.
fn direction_index(direction: Direction) -> usize {
    match direction {
        Direction::Up => 0,
        Direction::Down => 1,
        Direction::Left => 2,
        Direction::Right => 3,
    }
}

/// Turns per-direction factors into one thumb stick pair.
///
/// Opposite directions cancel, a diagonal is normalized to unit length and the
/// magnitude scales the result.
fn stick_vector(factors: &[f64; 4], magnitude: f64) -> (i16, i16) {
    let mut x = factors[3] - factors[2];
    let mut y = factors[0] - factors[1];
    let length = x.hypot(y);
    if length > 1.0 {
        x /= length;
        y /= length;
    }
    (axis(x * magnitude), axis(y * magnitude))
}
