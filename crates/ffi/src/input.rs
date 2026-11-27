// SPDX-License-Identifier: GPL-3.0-or-later

//! The session input router behind the C interface (Campaign F, #942).
//!
//! Every session owns one host input router. The macOS app feeds it the
//! keyboard, the mouse and the physical controllers it sees; the router turns
//! those events into the four XInput-unit gamepad snapshots the runtime
//! services already carry. The router holds no clock of its own: every call
//! carries the current time in milliseconds, so the same event script always
//! produces the same output.
//!
//! The routing rules and the profile translation are the host policy of
//! Xenora, described in `docs/coordination/input-campaign.md`, not Windows or
//! Xbox assignment behavior. `include/xenora.h` states the contract of every
//! function in plain words.

use crate::frame_loop::{XINPUT_FEATURE_VIBRATION, XInputBattery, XInputSnapshot};
use crate::{SESSIONS, guarded, is_live, split};
use xenora_input::controller::{Family, RawController};
use xenora_input::profile::Profile;
use xenora_input::router::{InputConfig, Router, RoutingMode};

/// Devices keep their own slots and the keyboard and mouse merge into slot 0.
pub const XENORA_INPUT_MODE_AUTOMATIC: u32 = 0;
/// Slot 0 holds the keyboard and the mouse; the devices are tracked, not used.
pub const XENORA_INPUT_MODE_KEYBOARD_MOUSE_ONLY: u32 = 1;
/// Devices keep their own slots; the keyboard and the mouse are ignored.
pub const XENORA_INPUT_MODE_PHYSICAL_ONLY: u32 = 2;
/// Every source merges into slot 0.
pub const XENORA_INPUT_MODE_COMBINED: u32 = 3;
/// Slot 0 follows the source that most recently produced new non-neutral input.
pub const XENORA_INPUT_MODE_LAST_ACTIVE: u32 = 4;
/// The sources assigned to a slot merge there; unassigned sources are ignored.
pub const XENORA_INPUT_MODE_EXPLICIT: u32 = 5;

/// An Xbox Series X|S controller.
pub const XENORA_CONTROLLER_FAMILY_XBOX_SERIES: u32 = 0;
/// An Xbox One controller.
pub const XENORA_CONTROLLER_FAMILY_XBOX_ONE: u32 = 1;
/// An Xbox Elite controller.
pub const XENORA_CONTROLLER_FAMILY_XBOX_ELITE: u32 = 2;
/// A DualShock 4 controller.
pub const XENORA_CONTROLLER_FAMILY_DUALSHOCK4: u32 = 3;
/// A DualSense controller.
pub const XENORA_CONTROLLER_FAMILY_DUALSENSE: u32 = 4;
/// A Switch Pro controller.
pub const XENORA_CONTROLLER_FAMILY_SWITCH_PRO: u32 = 5;
/// An extended gamepad of an unrecognized family.
pub const XENORA_CONTROLLER_FAMILY_GENERIC_EXTENDED: u32 = 6;
/// A micro gamepad.
pub const XENORA_CONTROLLER_FAMILY_MICRO: u32 = 7;

/// The largest macOS virtual key code the router accepts.
const MAX_KEY_CODE: u16 = 0x7F;
/// The largest mouse button number an AppKit event may carry.
const MAX_MOUSE_BUTTON: u8 = 31;
/// The largest profile text the interface accepts, in bytes.
const MAX_PROFILE_BYTES: u64 = 16 * 1024;
/// The number of XInput slots the router fills.
const SLOTS: usize = 4;
/// The routing modes, in the order of their wire values.
const ROUTING_MODES: [RoutingMode; 6] = [
    RoutingMode::Automatic,
    RoutingMode::KeyboardMouseOnly,
    RoutingMode::PhysicalOnly,
    RoutingMode::Combined,
    RoutingMode::LastActive,
    RoutingMode::Explicit,
];

/// The raw host reading of one physical controller.
///
/// `elements` holds the positional element bits of the host framework, the
/// sticks run from -1 to 1 with the Y axis pointing up and the triggers from
/// 0 to 1. `features` may hold only the vibration bit, and the reserved bytes
/// must be zero. The field names carry the same meaning as the header.
#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct xenora_controller_raw {
    pub elements: u32,
    pub left_x: f32,
    pub left_y: f32,
    pub right_x: f32,
    pub right_y: f32,
    pub left_trigger: f32,
    pub right_trigger: f32,
    pub features: u8,
    pub reserved: [u8; 3],
}

/// What the host reported about one connected device.
pub(crate) struct DeviceInfo {
    id: u64,
    profile: Profile,
    vibration: bool,
    battery: Option<XInputBattery>,
}

/// The input router one session owns, with the host evidence behind it.
pub(crate) struct InputHost {
    router: Router,
    devices: Vec<DeviceInfo>,
}

impl Default for InputHost {
    fn default() -> Self {
        Self {
            // The default configuration is valid by construction.
            router: Router::new(InputConfig::default()),
            devices: Vec::new(),
        }
    }
}

impl InputHost {
    /// The snapshot of one output slot, in the public XInput units.
    fn slot_snapshot(&self, slot: usize, time_ms: u64) -> XInputSnapshot {
        let output = self.router.output(time_ms);
        let Some(state) = output[slot] else {
            return XInputSnapshot::default();
        };
        let (vibration, _) = self.slot_metadata(slot);
        XInputSnapshot {
            buttons: state.buttons,
            left_trigger: state.left_trigger,
            right_trigger: state.right_trigger,
            left_x: state.left_x,
            left_y: state.left_y,
            right_x: state.right_x,
            right_y: state.right_y,
            connected: 1,
            features: if vibration {
                XINPUT_FEATURE_VIBRATION
            } else {
                0
            },
            reserved: [0; 2],
        }
    }

    /// The vibration and battery evidence of the device that owns a slot.
    ///
    /// Only the automatic and the physical-only modes give an output slot to
    /// one device, so only in those modes does that device report its own
    /// capabilities. In every other mode a slot merges several sources and no
    /// single device owns it, so the slot reports no vibration motor and no
    /// battery. This is Xenora host policy.
    fn slot_metadata(&self, slot: usize) -> (bool, Option<XInputBattery>) {
        if !matches!(
            self.router.mode(),
            RoutingMode::Automatic | RoutingMode::PhysicalOnly
        ) {
            return (false, None);
        }
        let Some(device) = self
            .devices
            .iter()
            .find(|device| self.router.device_slot(device.id) == Some(slot as u8))
        else {
            return (false, None);
        };
        (device.vibration, device.battery)
    }
}

/// Drives the input router of a live session, with the session table locked.
fn with_host(session: u64, body: impl FnOnce(&mut InputHost) -> i32) -> i32 {
    let Ok(mut slots) = SESSIONS.lock() else {
        return crate::XENORA_INTERNAL_ERROR;
    };
    if !is_live(&slots, session) {
        return crate::XENORA_INVALID_HANDLE;
    }
    let index = split(session).0;
    body(&mut slots[index].input)
}

/// Writes the consumed flag, when the caller asked for it.
fn write_consumed(out_consumed: *mut u8, consumed: bool) {
    if out_consumed.is_null() {
        return;
    }
    // SAFETY: checked non-null above; the caller guarantees the write.
    unsafe { out_consumed.write(u8::from(consumed)) };
}

/// Chooses the routing mode of the session's input router.
///
/// # Safety
/// The call takes no pointers, so it is safe for any handle value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xenora_session_input_set_mode(session: u64, mode: u32) -> i32 {
    guarded(|| {
        with_host(session, |host| {
            let Some(routing) = ROUTING_MODES.get(mode as usize).copied() else {
                return crate::XENORA_INVALID_ARGUMENT;
            };
            host.router.set_mode(routing);
            crate::XENORA_OK
        })
    })
}

/// Records a key press or a key release in the session's input router.
///
/// # Safety
/// `out_consumed` must be null or valid for one aligned `u8` write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xenora_session_input_key(
    session: u64,
    key_code: u16,
    down: u8,
    time_ms: u64,
    out_consumed: *mut u8,
) -> i32 {
    guarded(|| {
        with_host(session, |host| {
            if down > 1 || key_code > MAX_KEY_CODE {
                return crate::XENORA_INVALID_ARGUMENT;
            }
            let consumed = if down != 0 {
                host.router.key_down(key_code, time_ms)
            } else {
                host.router.key_up(key_code, time_ms)
            };
            write_consumed(out_consumed, consumed);
            crate::XENORA_OK
        })
    })
}

/// Records a mouse motion in the session's input router.
///
/// # Safety
/// The call takes no pointers, so it is safe for any handle value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xenora_session_input_mouse_motion(
    session: u64,
    dx: f64,
    dy: f64,
    time_ms: u64,
) -> i32 {
    guarded(|| {
        with_host(session, |host| {
            if !dx.is_finite() || !dy.is_finite() {
                return crate::XENORA_INVALID_ARGUMENT;
            }
            host.router.mouse_motion(dx, dy, time_ms);
            crate::XENORA_OK
        })
    })
}

/// Records a mouse button press or release in the session's input router.
///
/// # Safety
/// `out_consumed` must be null or valid for one aligned `u8` write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xenora_session_input_mouse_button(
    session: u64,
    button: u8,
    down: u8,
    time_ms: u64,
    out_consumed: *mut u8,
) -> i32 {
    guarded(|| {
        with_host(session, |host| {
            if down > 1 || button > MAX_MOUSE_BUTTON {
                return crate::XENORA_INVALID_ARGUMENT;
            }
            let consumed = host.router.mouse_button(button, down != 0, time_ms);
            write_consumed(out_consumed, consumed);
            crate::XENORA_OK
        })
    })
}

/// Sets or clears the focus of the session's input router.
///
/// # Safety
/// The call takes no pointers, so it is safe for any handle value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xenora_session_input_focus(session: u64, focused: u8) -> i32 {
    guarded(|| {
        with_host(session, |host| {
            if focused > 1 {
                return crate::XENORA_INVALID_ARGUMENT;
            }
            host.router.set_focus(focused != 0);
            crate::XENORA_OK
        })
    })
}

/// Connects a physical controller to the session's input router.
///
/// # Safety
/// `out_slot` must be null or valid for one aligned `u32` write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xenora_session_input_connect_device(
    session: u64,
    device_id: u64,
    family: u32,
    out_slot: *mut u32,
) -> i32 {
    guarded(|| {
        with_host(session, |host| {
            let Some(family) = Family::ALL.get(family as usize).copied() else {
                return crate::XENORA_INVALID_ARGUMENT;
            };
            if out_slot.is_null() {
                return crate::XENORA_INVALID_ARGUMENT;
            }
            let Some(slot) = host.router.connect_device(device_id) else {
                return crate::XENORA_LIMIT_EXCEEDED;
            };
            if !host.devices.iter().any(|device| device.id == device_id) {
                host.devices.push(DeviceInfo {
                    id: device_id,
                    profile: Profile::default_for(family),
                    vibration: false,
                    battery: None,
                });
            }
            // SAFETY: checked non-null above; the caller guarantees the write.
            unsafe { out_slot.write(u32::from(slot)) };
            crate::XENORA_OK
        })
    })
}

/// Disconnects a physical controller from the session's input router.
///
/// # Safety
/// The call takes no pointers, so it is safe for any handle value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xenora_session_input_disconnect_device(
    session: u64,
    device_id: u64,
) -> i32 {
    guarded(|| {
        with_host(session, |host| {
            if host.router.device_slot(device_id).is_none() {
                return crate::XENORA_INVALID_ARGUMENT;
            }
            host.router.disconnect_device(device_id);
            host.devices.retain(|device| device.id != device_id);
            crate::XENORA_OK
        })
    })
}

/// Replaces the profile of a connected physical controller.
///
/// # Safety
/// `text` must be null or valid for `length` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xenora_session_input_set_device_profile(
    session: u64,
    device_id: u64,
    text: *const u8,
    length: u64,
) -> i32 {
    guarded(|| {
        with_host(session, |host| {
            if text.is_null() || length > MAX_PROFILE_BYTES {
                return crate::XENORA_INVALID_ARGUMENT;
            }
            // SAFETY: checked non-null above and the length is bounded; the
            // caller guarantees the bytes are readable.
            let bytes = unsafe { std::slice::from_raw_parts(text, length as usize) };
            let Ok(source) = std::str::from_utf8(bytes) else {
                return crate::XENORA_INVALID_ARGUMENT;
            };
            let Ok(profile) = Profile::from_text(source) else {
                return crate::XENORA_INVALID_ARGUMENT;
            };
            let Some(device) = host
                .devices
                .iter_mut()
                .find(|device| device.id == device_id)
            else {
                return crate::XENORA_INVALID_ARGUMENT;
            };
            device.profile = profile;
            crate::XENORA_OK
        })
    })
}

/// Records the raw reading of a connected physical controller.
///
/// # Safety
/// `raw` must be null or valid for one initialized `xenora_controller_raw`,
/// and `battery` must be null or valid for one initialized
/// `xenora_xinput_battery`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xenora_session_input_device_raw(
    session: u64,
    device_id: u64,
    raw: *const xenora_controller_raw,
    battery: *const XInputBattery,
    time_ms: u64,
) -> i32 {
    guarded(|| {
        with_host(session, |host| {
            if raw.is_null() {
                return crate::XENORA_INVALID_ARGUMENT;
            }
            // SAFETY: checked non-null above; the caller guarantees the read.
            let raw = unsafe { *raw };
            if raw.features & !XINPUT_FEATURE_VIBRATION != 0 || raw.reserved != [0; 3] {
                return crate::XENORA_INVALID_ARGUMENT;
            }
            let battery = if battery.is_null() {
                None
            } else {
                // SAFETY: checked non-null above; the caller guarantees the read.
                let battery = unsafe { *battery };
                if !battery.valid_wire_shape() {
                    return crate::XENORA_INVALID_ARGUMENT;
                }
                Some(battery)
            };
            let Some(device) = host
                .devices
                .iter_mut()
                .find(|device| device.id == device_id)
            else {
                return crate::XENORA_INVALID_ARGUMENT;
            };
            let reading = RawController {
                elements: raw.elements,
                left_stick: [f64::from(raw.left_x), f64::from(raw.left_y)],
                right_stick: [f64::from(raw.right_x), f64::from(raw.right_y)],
                left_trigger: f64::from(raw.left_trigger),
                right_trigger: f64::from(raw.right_trigger),
            };
            let state = device.profile.translate(&reading);
            device.vibration = raw.features & XINPUT_FEATURE_VIBRATION != 0;
            device.battery = battery;
            host.router.device_state(device_id, state, time_ms);
            crate::XENORA_OK
        })
    })
}

/// Reads the four XInput slots of the session's input router.
///
/// # Safety
/// `out_slots` must be null or valid for four aligned `xenora_xinput_state`
/// writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xenora_session_input_snapshot(
    session: u64,
    time_ms: u64,
    out_slots: *mut XInputSnapshot,
) -> i32 {
    guarded(|| {
        with_host(session, |host| {
            if out_slots.is_null() {
                return crate::XENORA_INVALID_ARGUMENT;
            }
            for slot in 0..SLOTS {
                let snapshot = host.slot_snapshot(slot, time_ms);
                // SAFETY: checked non-null above; the caller guarantees the writes.
                unsafe { out_slots.add(slot).write(snapshot) };
            }
            crate::XENORA_OK
        })
    })
}

/// Publishes the four XInput slots of the session's input router into the
/// running frame loop.
///
/// # Safety
/// The call takes no pointers, so it is safe for any handle value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xenora_session_input_publish(session: u64, time_ms: u64) -> i32 {
    guarded(|| {
        let Ok(slots) = SESSIONS.lock() else {
            return crate::XENORA_INTERNAL_ERROR;
        };
        if !is_live(&slots, session) {
            return crate::XENORA_INVALID_HANDLE;
        }
        let index = split(session).0;
        let Some(running) = slots[index].frame_loop.clone() else {
            return crate::XENORA_INVALID_STATE;
        };
        let mut snapshots = [XInputSnapshot::default(); SLOTS];
        let mut batteries = [XInputBattery::default(); SLOTS];
        for slot in 0..SLOTS {
            snapshots[slot] = slots[index].input.slot_snapshot(slot, time_ms);
            batteries[slot] = slots[index].input.slot_metadata(slot).1.unwrap_or_default();
        }
        drop(slots);
        // The frame loop is called without the session table held, the way
        // the rest of the interface reaches it.
        for slot in 0..SLOTS {
            if !running.send_xinput_with_battery(slot as u32, snapshots[slot], batteries[slot]) {
                return crate::XENORA_INTERNAL_ERROR;
            }
        }
        crate::XENORA_OK
    })
}
