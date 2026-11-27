// SPDX-License-Identifier: GPL-3.0-or-later

//! Gamepad snapshots in public XInput units, and the rule that combines them.

/// One gamepad snapshot in public XInput units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PadState {
    /// Public `XINPUT_GAMEPAD_*` button bits.
    pub buttons: u16,
    /// Left trigger, 0 to 255.
    pub left_trigger: u8,
    /// Right trigger, 0 to 255.
    pub right_trigger: u8,
    /// Left thumb X axis, -32768 to 32767.
    pub left_x: i16,
    /// Left thumb Y axis, positive upward.
    pub left_y: i16,
    /// Right thumb X axis, -32768 to 32767.
    pub right_x: i16,
    /// Right thumb Y axis, positive upward.
    pub right_y: i16,
}

impl PadState {
    /// True when no button, trigger or axis carries a value.
    pub fn is_neutral(&self) -> bool {
        *self == Self::default()
    }
}

/// Public `XINPUT_GAMEPAD_*` button bit values, as published by Microsoft.
pub mod buttons {
    /// Directional pad, up.
    pub const DPAD_UP: u16 = 0x0001;
    /// Directional pad, down.
    pub const DPAD_DOWN: u16 = 0x0002;
    /// Directional pad, left.
    pub const DPAD_LEFT: u16 = 0x0004;
    /// Directional pad, right.
    pub const DPAD_RIGHT: u16 = 0x0008;
    /// Menu button.
    pub const START: u16 = 0x0010;
    /// View button.
    pub const BACK: u16 = 0x0020;
    /// Left thumb stick click.
    pub const LEFT_THUMB: u16 = 0x0040;
    /// Right thumb stick click.
    pub const RIGHT_THUMB: u16 = 0x0080;
    /// Left shoulder button.
    pub const LEFT_SHOULDER: u16 = 0x0100;
    /// Right shoulder button.
    pub const RIGHT_SHOULDER: u16 = 0x0200;
    /// Guide button.
    pub const GUIDE: u16 = 0x0400;
    /// A button.
    pub const A: u16 = 0x1000;
    /// B button.
    pub const B: u16 = 0x2000;
    /// X button.
    pub const X: u16 = 0x4000;
    /// Y button.
    pub const Y: u16 = 0x8000;
    /// Every public button bit.
    pub const ALL: u16 = DPAD_UP
        | DPAD_DOWN
        | DPAD_LEFT
        | DPAD_RIGHT
        | START
        | BACK
        | LEFT_THUMB
        | RIGHT_THUMB
        | LEFT_SHOULDER
        | RIGHT_SHOULDER
        | GUIDE
        | A
        | B
        | X
        | Y;
}

/// A direction on one of the thumb sticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Toward the top of the pad.
    Up,
    /// Toward the bottom of the pad.
    Down,
    /// Toward the left of the pad.
    Left,
    /// Toward the right of the pad.
    Right,
}

/// What a keyboard or mouse event drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// One public button bit.
    Button(u16),
    /// The left trigger.
    LeftTrigger,
    /// The right trigger.
    RightTrigger,
    /// One direction of the left thumb stick.
    LeftStick(Direction),
    /// One direction of the right thumb stick.
    RightStick(Direction),
}

/// True when the value names exactly one public XInput button bit.
pub(crate) fn is_valid_button(bit: u16) -> bool {
    bit != 0 && bit.count_ones() == 1 && (bit & !buttons::ALL) == 0
}

/// Converts a unit-range deflection into a public XInput axis value.
///
/// Non-finite input reads as zero. The result is clamped to the axis range:
/// a non-negative value scales by 32767 and a negative value by 32768, both
/// rounded half away from zero.
pub(crate) fn axis(v: f64) -> i16 {
    if !v.is_finite() {
        return 0;
    }
    let clamped = v.clamp(-1.0, 1.0);
    let scaled = if clamped >= 0.0 {
        clamped * 32767.0
    } else {
        clamped * 32768.0
    };
    scaled.round() as i16
}

/// Combines several snapshots into one.
///
/// Buttons are combined with OR, so one source never releases a button that
/// another source still holds. Each trigger takes the largest value. Each thumb
/// stick takes the whole axis pair of the source with the largest deflection;
/// a tie goes to the earlier source, so the result is deterministic.
pub fn merge(states: &[PadState]) -> PadState {
    let mut out = PadState::default();
    let mut left: Option<(i64, i16, i16)> = None;
    let mut right: Option<(i64, i16, i16)> = None;

    for state in states {
        out.buttons |= state.buttons;
        out.left_trigger = out.left_trigger.max(state.left_trigger);
        out.right_trigger = out.right_trigger.max(state.right_trigger);

        let left_magnitude = i64::from(state.left_x) * i64::from(state.left_x)
            + i64::from(state.left_y) * i64::from(state.left_y);
        if left.is_none_or(|(best, _, _)| left_magnitude > best) {
            left = Some((left_magnitude, state.left_x, state.left_y));
        }

        let right_magnitude = i64::from(state.right_x) * i64::from(state.right_x)
            + i64::from(state.right_y) * i64::from(state.right_y);
        if right.is_none_or(|(best, _, _)| right_magnitude > best) {
            right = Some((right_magnitude, state.right_x, state.right_y));
        }
    }

    if let Some((_, x, y)) = left {
        out.left_x = x;
        out.left_y = y;
    }
    if let Some((_, x, y)) = right {
        out.right_x = x;
        out.right_y = y;
    }
    out
}
