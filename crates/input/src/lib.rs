// SPDX-License-Identifier: GPL-3.0-or-later

//! Host input to per-slot gamepad snapshots.
//!
//! This crate turns keyboard events, mouse events and the snapshots that
//! physical controllers report into gamepad snapshots in the public XInput
//! units: the button bits, the triggers from 0 to 255 and the thumb axes from
//! -32768 to 32767 with the Y axis positive upward.
//!
//! The crate is host-agnostic and deterministic. It reads no input device
//! itself and talks to no guest: the caller supplies the events and the
//! current time in milliseconds, so the same event script always produces the
//! same snapshots. The app wiring and the FFI boundary come later; they feed
//! the existing XInput slots of the runtime services state with the snapshots
//! this crate produces.
//!
//! # Sources
//!
//! There are two kinds of source. The keyboard and the mouse drive one
//! virtual pad that needs no hardware and is always connected. Up to four
//! physical devices are tracked by their stable host identifiers. A device
//! takes the lowest free slot when it connects and keeps that slot until it
//! disconnects, and a device that reconnects starts neutral.
//!
//! # Routing modes
//!
//! The routing mode decides which source fills which slot:
//!
//! - [Automatic](router::RoutingMode::Automatic) keeps the devices on their
//!   own slots and merges the keyboard and the mouse into slot 0.
//! - [KeyboardMouseOnly](router::RoutingMode::KeyboardMouseOnly) fills slot 0
//!   with the keyboard and the mouse and tracks the devices without using
//!   them.
//! - [PhysicalOnly](router::RoutingMode::PhysicalOnly) keeps the devices on
//!   their own slots and ignores the keyboard and the mouse.
//! - [Combined](router::RoutingMode::Combined) merges every source into slot
//!   0.
//! - [LastActive](router::RoutingMode::LastActive) gives slot 0 to the source
//!   that most recently produced new non-neutral input.
//! - [Explicit](router::RoutingMode::Explicit) merges the sources assigned to
//!   each slot there and ignores the unassigned ones.
//!
//! # How sources merge
//!
//! Buttons are combined with OR, so one source never releases a button that
//! another source still holds. Each trigger takes the largest value. Each
//! thumb stick takes the whole axis pair of the source with the largest
//! deflection, and a tie goes to the earlier source, so the result is
//! deterministic. An axis pair is never mixed from two sources.
//!
//! # Bindings
//!
//! Keyboard bindings name the physical macOS virtual key codes of Carbon
//! [Events.h](keyboard::keycode). A binding does not depend on the keyboard
//! layout, on the character a key produces or on the modifier state: the same
//! physical key always drives the same target.
//!
//! The crate has no dependencies, uses no unsafe code and is covered by unit
//! tests only. It has not been tested against a physical controller.

pub mod controller;
mod error;
pub mod keyboard;
pub mod mouse;
mod pad;
pub mod profile;
pub mod router;

pub use error::ConfigError;
pub use pad::buttons;
pub use pad::{Direction, PadState, Target, merge};
