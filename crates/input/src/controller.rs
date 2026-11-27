// SPDX-License-Identifier: GPL-3.0-or-later

//! The controller families Xenora knows and the positional elements they report.
//!
//! Apple's GameController framework names the face buttons by position and
//! never by the label printed on the pad: `buttonA` is the bottom button,
//! `buttonB` the right one, `buttonX` the left one and `buttonY` the top
//! one. Every family uses those names, so a PlayStation or a Nintendo pad
//! reaches a guest with the Xbox layout without any emulation of the printed
//! labels. [`element::SOUTH`] is therefore `buttonA` on every family. The
//! remaining elements keep their framework names: [`element::MENU`] is
//! `buttonMenu`, [`element::VIEW`] is `buttonOptions` and [`element::HOME`]
//! is `buttonHome`.
//!
//! # Evidence
//!
//! The element names and the product category strings come from the public
//! macOS SDK header documentation for GameController: `GCController.h` names
//! the product categories, `GCGamepad.h` names the elements and
//! `GCXboxGamepad.h` documents the paddle buttons of the Elite controller and
//! the Share button of the Series X|S controller. That is documentation
//! evidence, not hardware evidence: no controller has been connected to the
//! machine that wrote this crate, so no family is hardware-tested. The Switch
//! Pro controller and the DualSense Edge have no public product category and
//! are therefore never recognized automatically.

/// The positional elements of a GameController gamepad, as bit masks.
pub mod element {
    /// The bottom face button, `buttonA` in the framework.
    pub const SOUTH: u32 = 1 << 0;
    /// The right face button, `buttonB` in the framework.
    pub const EAST: u32 = 1 << 1;
    /// The left face button, `buttonX` in the framework.
    pub const WEST: u32 = 1 << 2;
    /// The top face button, `buttonY` in the framework.
    pub const NORTH: u32 = 1 << 3;
    /// The left shoulder button, `buttonLeftShoulder`.
    pub const LEFT_SHOULDER: u32 = 1 << 4;
    /// The right shoulder button, `buttonRightShoulder`.
    pub const RIGHT_SHOULDER: u32 = 1 << 5;
    /// The left thumb stick click, `leftThumbstickButton`.
    pub const LEFT_STICK: u32 = 1 << 6;
    /// The right thumb stick click, `rightThumbstickButton`.
    pub const RIGHT_STICK: u32 = 1 << 7;
    /// The menu button, `buttonMenu`.
    pub const MENU: u32 = 1 << 8;
    /// The view or options button, `buttonOptions`.
    pub const VIEW: u32 = 1 << 9;
    /// The home button, `buttonHome`.
    pub const HOME: u32 = 1 << 10;
    /// The up direction of the directional pad.
    pub const DPAD_UP: u32 = 1 << 11;
    /// The down direction of the directional pad.
    pub const DPAD_DOWN: u32 = 1 << 12;
    /// The left direction of the directional pad.
    pub const DPAD_LEFT: u32 = 1 << 13;
    /// The right direction of the directional pad.
    pub const DPAD_RIGHT: u32 = 1 << 14;
    /// The first paddle button of an Xbox Elite controller.
    pub const PADDLE_1: u32 = 1 << 15;
    /// The second paddle button of an Xbox Elite controller.
    pub const PADDLE_2: u32 = 1 << 16;
    /// The third paddle button of an Xbox Elite controller.
    pub const PADDLE_3: u32 = 1 << 17;
    /// The fourth paddle button of an Xbox Elite controller.
    pub const PADDLE_4: u32 = 1 << 18;
    /// The touchpad click of a DualSense controller.
    pub const TOUCHPAD: u32 = 1 << 19;
    /// The Share button of an Xbox Series X|S controller.
    pub const SHARE: u32 = 1 << 20;
    /// Every element this crate knows.
    pub const ALL: u32 = SOUTH
        | EAST
        | WEST
        | NORTH
        | LEFT_SHOULDER
        | RIGHT_SHOULDER
        | LEFT_STICK
        | RIGHT_STICK
        | MENU
        | VIEW
        | HOME
        | DPAD_UP
        | DPAD_DOWN
        | DPAD_LEFT
        | DPAD_RIGHT
        | PADDLE_1
        | PADDLE_2
        | PADDLE_3
        | PADDLE_4
        | TOUCHPAD
        | SHARE;
}

/// A family of physical controller.
///
/// The families name what the host reports, not what a guest expects. Every
/// family reaches the guest through a profile that maps the positional
/// elements to the public XInput layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    /// An Xbox Series X|S controller.
    XboxSeries,
    /// An Xbox One controller.
    XboxOne,
    /// An Xbox Elite controller.
    XboxElite,
    /// A DualShock 4 controller.
    DualShock4,
    /// A DualSense controller.
    DualSense,
    /// A Switch Pro controller.
    SwitchPro,
    /// An extended gamepad of an unrecognized family.
    GenericExtended,
    /// A micro gamepad.
    Micro,
}

impl Family {
    /// Every family this crate knows.
    pub const ALL: [Family; 8] = [
        Family::XboxSeries,
        Family::XboxOne,
        Family::XboxElite,
        Family::DualShock4,
        Family::DualSense,
        Family::SwitchPro,
        Family::GenericExtended,
        Family::Micro,
    ];

    /// The stable text identifier of the family.
    ///
    /// The identifier appears in a profile text file and never changes once
    /// it is published.
    pub fn id(self) -> &'static str {
        match self {
            Family::XboxSeries => "xbox-series",
            Family::XboxOne => "xbox-one",
            Family::XboxElite => "xbox-elite",
            Family::DualShock4 => "dualshock4",
            Family::DualSense => "dualsense",
            Family::SwitchPro => "switch-pro",
            Family::GenericExtended => "generic-extended",
            Family::Micro => "micro",
        }
    }

    /// The family with the given text identifier.
    ///
    /// The match is exact: an identifier that differs in any character names
    /// no family.
    pub fn from_id(id: &str) -> Option<Family> {
        match id {
            "xbox-series" => Some(Family::XboxSeries),
            "xbox-one" => Some(Family::XboxOne),
            "xbox-elite" => Some(Family::XboxElite),
            "dualshock4" => Some(Family::DualShock4),
            "dualsense" => Some(Family::DualSense),
            "switch-pro" => Some(Family::SwitchPro),
            "generic-extended" => Some(Family::GenericExtended),
            "micro" => Some(Family::Micro),
            _ => None,
        }
    }
}

/// What the host reports about a connected controller.
///
/// The caller fills the description from the GameController framework. It is
/// a description of what the host says, not a verdict: [`recognize`] turns it
/// into a family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostDescription<'a> {
    /// The product category string of the framework.
    pub product_category: &'a str,
    /// True when the controller reports an extended gamepad.
    pub extended: bool,
    /// True when the controller reports paddle buttons.
    pub has_paddles: bool,
    /// True when the controller reports a Share button.
    pub has_share: bool,
}

/// One snapshot of a physical controller, in the units the host reports.
///
/// The values are the raw host readings: the axis values run from -1 to 1
/// with the Y axis pointing up, and the trigger values run from 0 to 1. A
/// profile turns them into the public XInput units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RawController {
    /// The pressed elements, as a mask of [`element`] bits.
    pub elements: u32,
    /// The left thumb stick, X first and Y second.
    pub left_stick: [f64; 2],
    /// The right thumb stick, X first and Y second.
    pub right_stick: [f64; 2],
    /// The left trigger.
    pub left_trigger: f64,
    /// The right trigger.
    pub right_trigger: f64,
}

/// The family the host description names.
///
/// A controller that does not report an extended gamepad is a micro gamepad.
/// An extended gamepad is matched on the exact product category string:
/// `"Xbox One"` covers the Xbox family, and the paddle buttons of the Elite
/// controller and the Share button of the Series X|S controller tell the
/// three apart. `"DualSense"` and `"DualShock 4"` name their own families.
/// Every other category is an unrecognized extended gamepad, and the family
/// of the Switch Pro controller is never recognized automatically because the
/// framework publishes no product category for it.
///
/// The categories come from the public macOS SDK header documentation. No
/// family has been tested against hardware.
pub fn recognize(host: &HostDescription<'_>) -> Family {
    if !host.extended {
        return Family::Micro;
    }
    match host.product_category {
        "Xbox One" => {
            if host.has_paddles {
                Family::XboxElite
            } else if host.has_share {
                Family::XboxSeries
            } else {
                Family::XboxOne
            }
        }
        "DualSense" => Family::DualSense,
        "DualShock 4" => Family::DualShock4,
        _ => Family::GenericExtended,
    }
}
