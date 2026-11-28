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
const ERROR_INVALID_HANDLE: u64 = 6;
const ERROR_NOT_SUPPORTED: u64 = 50;
const ERROR_NOACCESS: u64 = 998;

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
fn peek_zeroes_only_the_fourth_argument_count_for_an_empty_queue() {
    const RECORDS: u64 = SCRATCH + 0x100;
    const DECOY: u64 = SCRATCH + 0x200;
    const SENTINEL: u32 = 0x9ABC_13F7;
    let mut guest = Guest::new(IMPORTS, &[]);
    let input = console_input(&mut guest);
    let records = [0xA5; 20];
    guest.memory.write_bytes(RECORDS, &records).unwrap();
    guest.memory.write_u32(DECOY, SENTINEL).unwrap();

    for symbol in ["PeekConsoleInputA", "PeekConsoleInputW"] {
        guest.memory.write_u32(COUNT, SENTINEL).unwrap();
        // The fifth stack slot carries a decoy pointer. The ABI only has
        // four parameters, so the count belongs in R9 and the decoy is ignored.
        assert_eq!(guest.call(symbol, &[input, RECORDS, 1, COUNT, DECOY]), 1);
        assert_eq!(guest.u32_at(COUNT), 0, "{symbol}");
        assert_eq!(guest.u32_at(DECOY), SENTINEL, "{symbol}");
        let mut observed = [0; 20];
        guest.memory.read_bytes(RECORDS, &mut observed).unwrap();
        assert_eq!(observed, records, "{symbol}");
    }
}

#[test]
fn peek_rejects_a_null_count_or_invalid_input_handle() {
    let mut guest = Guest::new(IMPORTS, &[]);
    let input = console_input(&mut guest);
    for symbol in ["PeekConsoleInputA", "PeekConsoleInputW"] {
        assert_eq!(guest.call(symbol, &[input, SCRATCH + 0x100, 1, 0]), 0);
        assert_eq!(guest.call("GetLastError", &[]), ERROR_NOACCESS);
        guest.memory.write_u32(COUNT, 0xFD).unwrap();
        assert_eq!(guest.call(symbol, &[0, SCRATCH + 0x100, 1, COUNT]), 0);
        assert_eq!(guest.call("GetLastError", &[]), ERROR_INVALID_HANDLE);
        assert_eq!(guest.u32_at(COUNT), 0xFD);
    }
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
