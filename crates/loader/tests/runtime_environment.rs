// SPDX-License-Identifier: GPL-3.0-or-later

//! The environment block services through the guest import table.
//!
//! The environment is empty, so expansion is the identity by the documented
//! rule that an undefined variable is copied verbatim. The mechanics -
//! the terminator-counting sizes, the small-buffer answers, the block
//! ownership - are the documented ones.

mod support;

use support::*;

const KERNEL32: &str = "kernel32.dll";

const IMPORTS: &[(&str, &str)] = &[
    (KERNEL32, "ExpandEnvironmentStringsA"),
    (KERNEL32, "ExpandEnvironmentStringsW"),
    (KERNEL32, "GetEnvironmentStringsW"),
    (KERNEL32, "FreeEnvironmentStringsW"),
    (KERNEL32, "GetLastError"),
    (KERNEL32, "SetEnvironmentVariableA"),
    (KERNEL32, "SetEnvironmentVariableW"),
    (KERNEL32, "GetEnvironmentVariableA"),
];

const BUFFER: u64 = SCRATCH + 0x200;

fn narrow(guest: &mut Guest, at: u64, text: &str) {
    guest.memory.write_bytes(at, text.as_bytes()).unwrap();
    guest.memory.write_u8(at + text.len() as u64, 0).unwrap();
}

fn narrow_string(guest: &Guest, at: u64) -> String {
    let mut text = String::new();
    for i in 0.. {
        let byte = guest.memory.read_u8(at + i).unwrap();
        if byte == 0 {
            return text;
        }
        text.push(byte as char);
    }
    unreachable!()
}

#[test]
fn expansion_is_the_identity_over_the_empty_environment() {
    let mut guest = Guest::new(IMPORTS, &[]);
    // Variables undefined everywhere - including the percent signs of a
    // path a program would expand on Windows - come through verbatim.
    narrow(&mut guest, SCRATCH, "%TEMP%\\file.txt");
    let written = guest.call("ExpandEnvironmentStringsA", &[SCRATCH, BUFFER, 64]);
    // The count includes the terminator, as documented.
    assert_eq!(written, "%TEMP%\\file.txt".len() as u64 + 1);
    assert_eq!(narrow_string(&guest, BUFFER), "%TEMP%\\file.txt");

    // A buffer too small answers the required size and writes nothing useful.
    assert_eq!(
        guest.call("ExpandEnvironmentStringsA", &[SCRATCH, BUFFER, 4]),
        16
    );

    // The wide form counts wide units the same way.
    let wide_text: Vec<u16> = "%PATH%".encode_utf16().chain([0]).collect();
    for (i, unit) in wide_text.iter().enumerate() {
        guest
            .memory
            .write_u16(SCRATCH + 2 * i as u64, *unit)
            .unwrap();
    }
    let written = guest.call("ExpandEnvironmentStringsW", &[SCRATCH, BUFFER, 64]);
    assert_eq!(written, 7);
    assert_eq!(guest.u16_at(BUFFER), u16::from(b'%'));
}

#[test]
fn the_environment_block_is_empty_owned_and_freeable() {
    let mut guest = Guest::new(IMPORTS, &[]);
    let block = guest.call("GetEnvironmentStringsW", &[]);
    assert_ne!(block, 0);
    // An empty block is two null units: the empty string, its terminator.
    assert_eq!(guest.u16_at(block), 0);
    assert_eq!(guest.u16_at(block + 2), 0);

    // Each call hands a fresh owned copy, and freeing it succeeds.
    let second = guest.call("GetEnvironmentStringsW", &[]);
    assert_ne!(second, 0);
    assert_ne!(second, block);
    assert_eq!(guest.call("FreeEnvironmentStringsW", &[block]), 1);
    assert_eq!(guest.call("FreeEnvironmentStringsW", &[second]), 1);

    // Freeing a non-allocation fails the documented way.
    assert_eq!(guest.call("FreeEnvironmentStringsW", &[SCRATCH]), 0);
    assert_eq!(guest.call("GetLastError", &[]), 87);
}

#[test]
fn set_variables_expand_and_fill_the_block() {
    let mut guest = Guest::new(IMPORTS, &[]);

    // Invalid names are refused: null, empty, and the ones holding '='.
    narrow(&mut guest, SCRATCH, "BAD=NAME");
    narrow(&mut guest, BUFFER, "value");
    assert_eq!(guest.call("SetEnvironmentVariableA", &[SCRATCH, BUFFER]), 0);
    assert_eq!(guest.call("GetLastError", &[]), 87);
    assert_eq!(guest.call("SetEnvironmentVariableA", &[0, BUFFER]), 0);
    assert_eq!(guest.call("GetLastError", &[]), 87);

    // A set round-trips case-insensitively through the query.
    narrow(&mut guest, SCRATCH, "ProbeVar");
    narrow(&mut guest, BUFFER, "leaf");
    assert_eq!(guest.call("SetEnvironmentVariableA", &[SCRATCH, BUFFER]), 1);
    narrow(&mut guest, SCRATCH, "probevar");
    let got = guest.call("GetEnvironmentVariableA", &[SCRATCH, BUFFER, 32]);
    assert_eq!(got, 4);
    assert_eq!(narrow_string(&guest, BUFFER), "leaf");

    // Expansion resolves it, the unknown names copying verbatim.
    narrow(&mut guest, SCRATCH, "%ProbeVar%/%missing%");
    let written = guest.call("ExpandEnvironmentStringsA", &[SCRATCH, BUFFER, 64]);
    assert_eq!(written, "leaf/%missing%".len() as u64 + 1);
    assert_eq!(narrow_string(&guest, BUFFER), "leaf/%missing%");

    // The wide set form stores the same table.
    let wide_name: Vec<u16> = "WideVar".encode_utf16().chain([0]).collect();
    for (i, unit) in wide_name.iter().enumerate() {
        guest
            .memory
            .write_u16(SCRATCH + 2 * i as u64, *unit)
            .unwrap();
    }
    let wide_value: Vec<u16> = "wval".encode_utf16().chain([0]).collect();
    for (i, unit) in wide_value.iter().enumerate() {
        guest
            .memory
            .write_u16(BUFFER + 2 * i as u64, *unit)
            .unwrap();
    }
    assert_eq!(guest.call("SetEnvironmentVariableW", &[SCRATCH, BUFFER]), 1);

    // The block lists both variables as NAME=value, upper-case sorted.
    let block = guest.call("GetEnvironmentStringsW", &[]);
    assert_ne!(block, 0);
    // Entries end at single nulls, the block at two in a row.
    let mut text = String::new();
    let mut at = block;
    loop {
        let unit = guest.u16_at(at);
        if unit == 0 {
            if guest.u16_at(at + 2) == 0 {
                break;
            }
            text.push('\n');
            at += 2;
            continue;
        }
        text.push(char::from_u32(u32::from(unit)).unwrap_or('\u{fffd}'));
        at += 2;
    }
    assert!(
        text.starts_with("ProbeVar=leaf"),
        "sorted case-insensitively, original casing kept: {text}"
    );
    assert!(text.contains("WideVar=wval"), "{text}");
    assert_eq!(guest.call("FreeEnvironmentStringsW", &[block]), 1);

    // A null value deletes; the variable then misses and no longer expands.
    narrow(&mut guest, SCRATCH, "ProbeVar");
    assert_eq!(guest.call("SetEnvironmentVariableA", &[SCRATCH, 0]), 1);
    assert_eq!(
        guest.call("GetEnvironmentVariableA", &[SCRATCH, BUFFER, 32]),
        0
    );
    assert_eq!(guest.call("GetLastError", &[]), 203);
}
