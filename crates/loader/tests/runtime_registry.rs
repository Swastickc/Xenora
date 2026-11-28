// SPDX-License-Identifier: GPL-3.0-or-later

//! The synthetic empty registry through the guest import table.

mod support;

use support::*;
use xenora_loader::cpu::ExitReason;

const ADVAPI32: &str = "advapi32.dll";
const REGISTRY_API: &str = "api-ms-win-core-registry-l1-1-0.dll";
const KERNEL32: &str = "kernel32.dll";

const HKCU: u64 = 0x8000_0001;
const WIN64_HKCU: u64 = 0xffff_ffff_8000_0001;
const KEY_READ: u64 = 0x0002_0019;
const NAME: u64 = SCRATCH + 0x100;
const UNWRITTEN: u64 = 0x1122_3344_5566_7788;
const LEGACY_OPEN: &str = "RegOpenKeyW";
const LEGACY_IMPORTS: &[(&str, &str)] = &[(ADVAPI32, LEGACY_OPEN), (KERNEL32, "GetLastError")];

const fn imports(module: &'static str) -> [(&'static str, &'static str); 6] {
    [
        (module, "RegOpenKeyExA"),
        (module, "RegOpenKeyExW"),
        (module, "RegQueryValueExA"),
        (module, "RegQueryValueExW"),
        (module, "RegCloseKey"),
        (KERNEL32, "GetLastError"),
    ]
}

const CASES: [(&str, &[(&str, &str)]); 2] = [
    (ADVAPI32, &imports(ADVAPI32)),
    (REGISTRY_API, &imports(REGISTRY_API)),
];

fn narrow(guest: &mut Guest, text: &str) {
    guest.memory.write_bytes(NAME, text.as_bytes()).unwrap();
    guest.memory.write_u8(NAME + text.len() as u64, 0).unwrap();
}

fn wide(guest: &mut Guest, text: &str) {
    for (index, unit) in text.encode_utf16().chain([0]).enumerate() {
        guest
            .memory
            .write_u16(NAME + index as u64 * 2, unit)
            .unwrap();
    }
}

#[test]
fn opening_a_predefined_key_returns_the_same_handle() {
    for (module, imports) in CASES {
        let mut guest = Guest::new(imports, &[]);
        wide(&mut guest, "");
        for root in [HKCU, WIN64_HKCU] {
            for (symbol, subkey) in [
                ("RegOpenKeyExA", 0),
                ("RegOpenKeyExA", NAME),
                ("RegOpenKeyExW", 0),
                ("RegOpenKeyExW", NAME),
            ] {
                guest.memory.write_u64(SCRATCH, UNWRITTEN).unwrap();
                assert_eq!(guest.call(symbol, &[root, subkey, 0, KEY_READ, SCRATCH]), 0);
                assert_eq!(guest.u64_at(SCRATCH), root, "{module} {symbol}");
            }
            assert_eq!(guest.call("RegCloseKey", &[root]), 0);
            assert_eq!(guest.call("RegCloseKey", &[root]), 0);
        }
    }
}

#[test]
fn registry_failures_return_the_selected_windows_status() {
    for (module, imports) in CASES {
        let mut guest = Guest::new(imports, &[]);
        for (open, query, write_name) in [
            (
                "RegOpenKeyExA",
                "RegQueryValueExA",
                narrow as fn(&mut Guest, &str),
            ),
            ("RegOpenKeyExW", "RegQueryValueExW", wide),
        ] {
            write_name(&mut guest, "Software\\Xenora");
            guest.memory.write_u64(SCRATCH, UNWRITTEN).unwrap();
            assert_eq!(
                guest.call(open, &[WIN64_HKCU, NAME, 0, KEY_READ, SCRATCH]),
                2
            );
            assert_eq!(guest.u64_at(SCRATCH), UNWRITTEN, "{module} {open}");
            assert_eq!(guest.error(), 2);

            assert_eq!(guest.call(open, &[WIN64_HKCU, 0, 1, KEY_READ, SCRATCH]), 87);
            assert_eq!(guest.u64_at(SCRATCH), UNWRITTEN);
            assert_eq!(guest.call(open, &[WIN64_HKCU, 0, 0, KEY_READ, 0]), 87);
            assert_eq!(guest.error(), 87);

            write_name(&mut guest, "MissingValue");
            assert_eq!(guest.call(query, &[WIN64_HKCU, NAME, 0, 0, 0, 0]), 2);
            assert_eq!(guest.call(query, &[WIN64_HKCU, 0, 0, 0, 0, 0]), 2);

            // Arbitrary upper bits do not turn a foreign handle into a root.
            let foreign = 0x1234_5678_8000_0001;
            assert_eq!(guest.call(open, &[foreign, 0, 0, KEY_READ, SCRATCH]), 6);
            assert_eq!(guest.u64_at(SCRATCH), UNWRITTEN);
            assert_eq!(guest.call(query, &[foreign, 0, 0, 0, 0, 0]), 6);
            assert_eq!(guest.error(), 6);
            assert_eq!(guest.call("RegCloseKey", &[foreign]), 6);
        }
    }
}

#[test]
fn reg_open_key_w_returns_the_predefined_handle_through_its_third_argument() {
    let mut guest = Guest::new(LEGACY_IMPORTS, &[]);
    let output = SCRATCH + 8;
    guest.memory.write_u64(SCRATCH, UNWRITTEN).unwrap();
    guest.memory.write_u64(output + 8, UNWRITTEN).unwrap();
    wide(&mut guest, "");

    for root in [HKCU, WIN64_HKCU] {
        for subkey in [0, NAME] {
            guest.memory.write_u64(output, UNWRITTEN).unwrap();
            assert_eq!(guest.call(LEGACY_OPEN, &[root, subkey, output]), 0);
            assert_eq!(guest.u64_at(output), root);
            assert_eq!(guest.u64_at(SCRATCH), UNWRITTEN);
            assert_eq!(guest.u64_at(output + 8), UNWRITTEN);
        }
    }
}

#[test]
fn reg_open_key_w_reports_synthetic_failures_without_writing_a_handle() {
    let mut guest = Guest::new(LEGACY_IMPORTS, &[]);
    guest.memory.write_u64(SCRATCH, UNWRITTEN).unwrap();
    // The first UTF-16 code unit has a zero low byte, so an ANSI read would
    // mistake this named key for an empty string.
    wide(&mut guest, "\u{0100}\\Xenora");

    let foreign = 0x1234_5678_8000_0001;
    for (args, status) in [
        ([WIN64_HKCU, NAME, SCRATCH], 2),
        ([foreign, 0, SCRATCH], 6),
        ([WIN64_HKCU, 0, 0], 87),
    ] {
        assert_eq!(guest.call(LEGACY_OPEN, &args), status);
        assert_eq!(guest.error(), status);
        assert_eq!(guest.u64_at(SCRATCH), UNWRITTEN);
    }

    let unmapped = 0x1000;
    let fault = guest.memory.read_u16(unmapped).unwrap_err();
    assert_eq!(
        guest.fails(LEGACY_OPEN, &[WIN64_HKCU, unmapped, SCRATCH]),
        ExitReason::MemoryFault(fault)
    );
    assert_eq!(guest.u64_at(SCRATCH), UNWRITTEN);

    let fault = guest.memory.write_u64(unmapped, HKCU).unwrap_err();
    assert_eq!(
        guest.fails(LEGACY_OPEN, &[HKCU, 0, unmapped]),
        ExitReason::MemoryFault(fault)
    );
}
