// SPDX-License-Identifier: GPL-3.0-or-later

//! Synthetic guest calls for the toolhelp process snapshot of a machine with
//! one process. Expectations use Microsoft's public contracts:
//! https://learn.microsoft.com/windows/win32/api/tlhelp32/nf-tlhelp32-createtoolhelp32snapshot
//! https://learn.microsoft.com/windows/win32/api/tlhelp32/nf-tlhelp32-process32firstw
//!
//! The one-process machine is a Xenora fact, not a Windows behavior.

mod support;

use support::{Guest, SCRATCH};

const KERNEL32: &str = "kernel32.dll";

const IMPORTS: &[(&str, &str)] = &[
    (KERNEL32, "GetLastError"),
    (KERNEL32, "CloseHandle"),
    (KERNEL32, "CreateToolhelp32Snapshot"),
    (KERNEL32, "Process32FirstW"),
    (KERNEL32, "Process32NextW"),
    (KERNEL32, "Process32First"),
    (KERNEL32, "Process32Next"),
    (KERNEL32, "IsWow64Process"),
    (KERNEL32, "GetCurrentProcessId"),
];

const ENTRY: u64 = SCRATCH;
const ERROR_NO_MORE_FILES: u64 = 259;
const ERROR_INVALID_HANDLE: u64 = 6;
const TH32CS_SNAPPROCESS: u64 = 2;

#[test]
fn the_snapshot_enumerates_the_one_process() {
    let mut guest = Guest::new(IMPORTS, &[]);
    let own_pid = guest.call("GetCurrentProcessId", &[]) as u32;
    let snapshot = guest.call("CreateToolhelp32Snapshot", &[TH32CS_SNAPPROCESS, 0]);
    assert_ne!(snapshot, 0);
    // First answers the machine's one process with one thread and no parent.
    guest.memory.write_u32(ENTRY, 0x248).unwrap();
    assert_eq!(guest.call("Process32FirstW", &[snapshot, ENTRY]), 1);
    assert_eq!(guest.u32_at(ENTRY + 8), own_pid);
    assert_eq!(guest.u32_at(ENTRY + 0x18), 1, "one thread");
    assert_eq!(guest.u32_at(ENTRY + 0x20), 0, "no parent");
    let mut units = [0u16; 17];
    for (slot, unit) in units.iter_mut().enumerate() {
        *unit = guest.u16_at(ENTRY + 0x28 + slot as u64 * 2);
    }
    let name: String = units
        .iter()
        .take_while(|unit| **unit != 0)
        .map(|unit| char::from(*unit as u8))
        .collect();
    assert_eq!(name, "xenora-guest.exe");
    // Next is the end of the list, honestly: ERROR_NO_MORE_FILES.
    assert_eq!(guest.call("Process32NextW", &[snapshot, ENTRY]), 0);
    assert_eq!(
        guest.call("GetLastError", &[]) & 0xFFFF_FFFF,
        ERROR_NO_MORE_FILES
    );
    assert_eq!(guest.call("CloseHandle", &[snapshot]), 1);
    // The narrow width enumerates the same process.
    let narrow = guest.call("CreateToolhelp32Snapshot", &[TH32CS_SNAPPROCESS, 0]);
    guest.memory.write_u32(ENTRY, 0x128).unwrap();
    assert_eq!(guest.call("Process32First", &[narrow, ENTRY]), 1);
    assert_eq!(guest.u32_at(ENTRY + 8), own_pid);
    assert_eq!(guest.call("Process32Next", &[narrow, ENTRY]), 0);
    assert_eq!(guest.call("CloseHandle", &[narrow]), 1);
    // A file handle is not a snapshot.
    assert_eq!(guest.call("Process32FirstW", &[0xDEAD, ENTRY]), 0);
    assert_eq!(
        guest.call("GetLastError", &[]) & 0xFFFF_FFFF,
        ERROR_INVALID_HANDLE
    );
}

#[test]
fn nothing_runs_under_wow64() {
    let mut guest = Guest::new(IMPORTS, &[]);
    const OUT: u64 = SCRATCH;
    assert_eq!(guest.call("IsWow64Process", &[u64::MAX, OUT]), 1);
    assert_eq!(guest.u32_at(OUT), 0);
}
