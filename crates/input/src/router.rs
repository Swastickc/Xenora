// SPDX-License-Identifier: GPL-3.0-or-later

//! Routing of host input sources into per-slot gamepad snapshots.

use crate::error::ConfigError;
use crate::keyboard::{KeyboardConfig, KeyboardTranslator};
use crate::mouse::{MouseConfig, MouseTranslator};
use crate::pad::{PadState, merge};

/// The number of gamepad slots a router fills.
const SLOTS: usize = 4;

/// The most sources that can hold an explicit assignment at once.
const MAX_ASSIGNMENTS: usize = 16;

/// Where a source comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SourceId {
    /// The virtual pad that the keyboard and the mouse drive.
    KeyboardMouse,
    /// A physical device, named by its stable host identifier.
    Device(u64),
}

/// How the sources are spread over the slots.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RoutingMode {
    /// Devices keep their own slots and the keyboard and the mouse merge into
    /// slot 0. This is the default.
    #[default]
    Automatic,
    /// Slot 0 holds the keyboard and the mouse. Devices are tracked but
    /// ignored.
    KeyboardMouseOnly,
    /// Devices keep their own slots. The keyboard and the mouse are ignored.
    PhysicalOnly,
    /// Every source merges into slot 0.
    Combined,
    /// Slot 0 follows the source that most recently produced new non-neutral
    /// input.
    LastActive,
    /// The sources assigned to a slot merge there. Unassigned sources are
    /// ignored.
    Explicit,
}

/// The input configuration.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputConfig {
    /// The routing mode.
    pub mode: RoutingMode,
    /// The keyboard configuration.
    pub keyboard: KeyboardConfig,
    /// The mouse configuration.
    pub mouse: MouseConfig,
}

impl InputConfig {
    /// Checks the configuration against the rules of this crate.
    ///
    /// # Errors
    ///
    /// Returns a [ConfigError] when the keyboard or the mouse configuration is
    /// refused.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.keyboard.validate()?;
        self.mouse.validate()?;
        Ok(())
    }
}

/// One tracked device and the snapshot it last reported.
#[derive(Clone, Copy, Debug)]
struct Device {
    id: u64,
    state: PadState,
}

/// Routes host input sources into the gamepad snapshots of up to four slots.
///
/// The router holds no clock of its own: every call carries the current time
/// in milliseconds, so the same event script always produces the same output.
pub struct Router {
    config: InputConfig,
    keyboard: KeyboardTranslator,
    mouse: MouseTranslator,
    devices: [Option<Device>; SLOTS],
    assignments: Vec<(SourceId, u8)>,
    active: SourceId,
    focused: bool,
}

impl Router {
    /// Builds a router from a configuration.
    ///
    /// # Errors
    ///
    /// Returns a [ConfigError] when the configuration is refused by
    /// [InputConfig::validate].
    pub fn try_new(config: InputConfig) -> Result<Self, ConfigError> {
        config.validate()?;
        let keyboard = KeyboardTranslator::try_new(config.keyboard.clone())?;
        let mouse = MouseTranslator::try_new(config.mouse.clone())?;
        Ok(Self {
            config,
            keyboard,
            mouse,
            devices: std::array::from_fn(|_| None),
            assignments: Vec::new(),
            active: SourceId::KeyboardMouse,
            focused: true,
        })
    }

    /// Builds a router from a configuration, panicking on an invalid one.
    ///
    /// # Panics
    ///
    /// Panics when the configuration is refused by [InputConfig::validate].
    pub fn new(config: InputConfig) -> Self {
        match Self::try_new(config) {
            Ok(router) => router,
            Err(error) => panic!("invalid input configuration: {error}"),
        }
    }

    /// The configuration in use.
    pub fn config(&self) -> &InputConfig {
        &self.config
    }

    /// The routing mode.
    pub fn mode(&self) -> RoutingMode {
        self.config.mode
    }

    /// Changes the routing mode.
    ///
    /// The device slots, the device states and the assignments stay as they
    /// are.
    pub fn set_mode(&mut self, mode: RoutingMode) {
        self.config.mode = mode;
    }

    /// True while the router accepts input. A router starts focused.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Sets or clears the focus.
    ///
    /// Losing the focus releases every key and every button, and neutralizes
    /// every tracked device. While the router is unfocused it refuses input.
    /// The active source does not change.
    pub fn set_focus(&mut self, focused: bool) {
        self.focused = focused;
        if focused {
            return;
        }
        self.keyboard.release_all();
        self.mouse.release_all();
        for device in self.devices.iter_mut().flatten() {
            device.state = PadState::default();
        }
    }

    /// Records a key-down event.
    ///
    /// Returns false when the router is unfocused or when the key has no
    /// binding, and then nothing changes.
    pub fn key_down(&mut self, key: u16, now_ms: u64) -> bool {
        if !self.focused {
            return false;
        }
        let was_held = self.keyboard.is_held(key);
        if !self.keyboard.key_down(key, now_ms) {
            return false;
        }
        if !was_held {
            self.active = SourceId::KeyboardMouse;
        }
        true
    }

    /// Records a key-up event.
    ///
    /// Returns false when the router is unfocused or when the key has no
    /// binding, and then nothing changes.
    pub fn key_up(&mut self, key: u16, now_ms: u64) -> bool {
        if !self.focused {
            return false;
        }
        self.keyboard.key_up(key, now_ms)
    }

    /// Records a mouse motion event.
    ///
    /// A motion that arrives while the router is unfocused is ignored.
    pub fn mouse_motion(&mut self, dx: f64, dy: f64, now_ms: u64) {
        if !self.focused {
            return;
        }
        if !dx.is_finite() || !dy.is_finite() {
            return;
        }
        if dx != 0.0 || dy != 0.0 {
            self.active = SourceId::KeyboardMouse;
        }
        self.mouse.motion(dx, dy, now_ms);
    }

    /// Records a mouse button event.
    ///
    /// Returns false when the router is unfocused or when the button has no
    /// binding, and then nothing changes.
    pub fn mouse_button(&mut self, button: u8, down: bool, _now_ms: u64) -> bool {
        // Button events carry no time: only motion decays.
        if !self.focused {
            return false;
        }
        let accepted = self.mouse.button(button, down);
        if accepted && down {
            self.active = SourceId::KeyboardMouse;
        }
        accepted
    }

    /// Tracks a device.
    ///
    /// Returns the slot the device occupies. A device that is already tracked
    /// keeps its slot. A new device takes the lowest free slot, and a device
    /// that cannot be tracked returns nothing.
    pub fn connect_device(&mut self, id: u64) -> Option<u8> {
        if let Some(slot) = self.device_slot(id) {
            return Some(slot);
        }
        for (slot, entry) in self.devices.iter_mut().enumerate() {
            if entry.is_none() {
                *entry = Some(Device {
                    id,
                    state: PadState::default(),
                });
                return Some(slot as u8);
            }
        }
        None
    }

    /// Stops tracking a device.
    ///
    /// Returns false when the device is not tracked. A tracked device that was
    /// the active source hands that role back to the keyboard and the mouse.
    pub fn disconnect_device(&mut self, id: u64) -> bool {
        let Some(slot) = self.device_slot(id) else {
            return false;
        };
        self.devices[slot as usize] = None;
        if self.active == SourceId::Device(id) {
            self.active = SourceId::KeyboardMouse;
        }
        true
    }

    /// Records the snapshot a device reports.
    ///
    /// Returns false when the router is unfocused or when the device is not
    /// tracked, and then nothing changes.
    pub fn device_state(&mut self, id: u64, state: PadState, _now_ms: u64) -> bool {
        // A device snapshot is what the device holds now; its time is unused.
        if !self.focused {
            return false;
        }
        let Some(slot) = self.device_slot(id) else {
            return false;
        };
        let Some(device) = self.devices[slot as usize].as_mut() else {
            return false;
        };
        let changed = device.state != state;
        device.state = state;
        if changed && !state.is_neutral() {
            self.active = SourceId::Device(id);
        }
        true
    }

    /// The slot a device occupies.
    pub fn device_slot(&self, id: u64) -> Option<u8> {
        self.devices
            .iter()
            .position(|device| device.is_some_and(|device| device.id == id))
            .map(|slot| slot as u8)
    }

    /// Assigns a source to a slot.
    ///
    /// A slot above 3 does not exist and is refused. Assigning nothing removes
    /// the assignment. The assignment survives a disconnect. At most
    /// sixteen sources hold an assignment; a new source beyond that is
    /// refused, while a known source can still be moved or removed.
    pub fn assign(&mut self, source: SourceId, slot: Option<u8>) -> bool {
        match slot {
            Some(slot) => {
                if slot as usize >= SLOTS {
                    return false;
                }
                if let Some(entry) = self
                    .assignments
                    .iter_mut()
                    .find(|(other, _)| *other == source)
                {
                    entry.1 = slot;
                    return true;
                }
                if self.assignments.len() >= MAX_ASSIGNMENTS {
                    return false;
                }
                self.assignments.push((source, slot));
                true
            }
            None => {
                self.assignments.retain(|(other, _)| *other != source);
                true
            }
        }
    }

    /// The slot a source is assigned to.
    pub fn assignment(&self, source: SourceId) -> Option<u8> {
        self.assignments
            .iter()
            .find(|(other, _)| *other == source)
            .map(|(_, slot)| *slot)
    }

    /// The source that most recently produced new non-neutral input.
    pub fn active_source(&self) -> SourceId {
        self.active
    }

    /// The gamepad snapshots of the four slots for the given time, in
    /// milliseconds.
    ///
    /// A slot without a connected source reads as nothing.
    pub fn output(&self, now_ms: u64) -> [Option<PadState>; SLOTS] {
        let keyboard_mouse = merge(&[self.keyboard.state(now_ms), self.mouse.state(now_ms)]);
        match self.config.mode {
            RoutingMode::KeyboardMouseOnly => [Some(keyboard_mouse), None, None, None],
            RoutingMode::PhysicalOnly => {
                let mut out = [None; SLOTS];
                for slot in 0..SLOTS {
                    out[slot] = self.devices[slot].map(|device| device.state);
                }
                out
            }
            RoutingMode::Automatic => {
                let mut out = [None; SLOTS];
                let mut states = vec![keyboard_mouse];
                if let Some(device) = self.devices[0] {
                    states.push(device.state);
                }
                out[0] = Some(merge(&states));
                for slot in 1..SLOTS {
                    out[slot] = self.devices[slot].map(|device| device.state);
                }
                out
            }
            RoutingMode::Combined => {
                let mut states = vec![keyboard_mouse];
                for device in self.devices.iter().flatten() {
                    states.push(device.state);
                }
                let mut out = [None; SLOTS];
                out[0] = Some(merge(&states));
                out
            }
            RoutingMode::LastActive => {
                let mut out = [None; SLOTS];
                out[0] = match self.active {
                    SourceId::KeyboardMouse => Some(keyboard_mouse),
                    SourceId::Device(id) => self.device_state_of(id),
                };
                out
            }
            RoutingMode::Explicit => {
                let mut out = [None; SLOTS];
                for (slot, entry) in out.iter_mut().enumerate() {
                    let mut states: Vec<PadState> = Vec::new();
                    if self.assignment(SourceId::KeyboardMouse) == Some(slot as u8) {
                        states.push(keyboard_mouse);
                    }
                    for device in self.devices.iter().flatten() {
                        if self.assignment(SourceId::Device(device.id)) == Some(slot as u8) {
                            states.push(device.state);
                        }
                    }
                    if !states.is_empty() {
                        *entry = Some(merge(&states));
                    }
                }
                out
            }
        }
    }

    /// The snapshot a tracked device holds.
    fn device_state_of(&self, id: u64) -> Option<PadState> {
        self.devices
            .iter()
            .flatten()
            .find(|device| device.id == id)
            .map(|device| device.state)
    }
}
