// SPDX-License-Identifier: GPL-3.0-or-later

//! Synthetic guest calls for console input events and control handlers.
//! Expectations use Microsoft's public contracts:
//! https://learn.microsoft.com/windows/console/getnumberofconsoleinputevents
//! https://learn.microsoft.com/windows/console/peekconsoleinput
//!
//! The empty input buffer and UTF-8-only code pages are Xenora facts of a
//! machine with no keyboard, not claims about a desktop Windows console.

mod support;

use support::{Guest, SCRATCH};

const KERNEL32: &str = "kernel32.dll";

const IMPORTS: &[(&str, &str)] = &[
    (KERNEL32, "GetLastError"),
    (KERNEL32, "GetNumberOfConsoleInputEvents"),
    (KERNEL32, "PeekConsoleInputA"),
    (KERNEL32, "PeekConsoleInputW"),
    (KERNEL32, "ReadConsoleInputA"),
    (KERNEL32, "SetConsoleCP"),
    (KERNEL32, "SetConsoleOutputCP"),
    (KERNEL32, "SetConsoleCtrlHandler"),
    (KERNEL32, "GenerateConsoleCtrlEvent"),
    (KERNEL32, "GetStdHandle"),
];

const COUNT: u64 = SCRATCH;
const STD_INPUT_HANDLE: u64 = 0xFFFF_FFF6;
const ERROR_NOT_SUPPORTED: u64 = 50;

fn console_input(guest: &mut Guest) -> u64 {
    guest.call("GetStdHandle", &[STD_INPUT_HANDLE])
}

#[test]
fn the_input_buffer_is_empty_and_countable() {
    let mut guest = Guest::new(IMPORTS, &[]);
    let input = console_input(&mut guest);
    assert_eq!(
        guest.call("GetNumberOfConsoleInputEvents", &[input, COUNT]),
        1
    );
    assert_eq!(guest.u32_at(COUNT), 0, "the machine has no keyboard input");
    // Peeking succeeds and reads nothing.
    assert_eq!(
        guest.call("PeekConsoleInputW", &[input, SCRATCH + 0x100, 8, COUNT]),
        1
    );
    assert_eq!(guest.u32_at(COUNT), 0);
    assert_eq!(
        guest.call("PeekConsoleInputA", &[input, SCRATCH + 0x100, 8, COUNT]),
        1
    );
    assert_eq!(guest.u32_at(COUNT), 0);
    // Reading would block forever; the machine refuses honestly.
    assert_eq!(
        guest.call("ReadConsoleInputA", &[input, SCRATCH + 0x100, 8, COUNT]),
        0
    );
    assert_eq!(
        guest.call("GetLastError", &[]) & 0xFFFF_FFFF,
        ERROR_NOT_SUPPORTED
    );
}

#[test]
fn code_pages_are_utf8_and_control_handlers_are_inert() {
    let mut guest = Guest::new(IMPORTS, &[]);
    // UTF-8 is accepted; any other code page contradicts the capture.
    assert_eq!(guest.call("SetConsoleCP", &[65001]), 1);
    assert_eq!(guest.call("SetConsoleOutputCP", &[65001]), 1);
    assert_eq!(guest.call("SetConsoleCP", &[437]), 0);
    // A control handler is accepted and never fires: the machine generates
    // no control events, which is correct rather than missing.
    assert_eq!(guest.call("SetConsoleCtrlHandler", &[SCRATCH, 1]), 1);
    assert_eq!(guest.call("SetConsoleCtrlHandler", &[SCRATCH, 0]), 1);
    // A null handler cannot be added.
    assert_eq!(guest.call("SetConsoleCtrlHandler", &[0, 1]), 0);
    // Generating an event needs delivery machinery the machine lacks.
    assert_eq!(guest.call("GenerateConsoleCtrlEvent", &[0, 0]), 0);
    assert_eq!(
        guest.call("GetLastError", &[]) & 0xFFFF_FFFF,
        ERROR_NOT_SUPPORTED
    );
}
