// SPDX-License-Identifier: GPL-3.0-or-later

//! Synthetic guest calls for the volume enumeration and information.
//! Expectations use Microsoft's public contracts:
//! https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-findfirstvolumew
//! https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-getvolumeinformationw
//!
//! The one-volume machine, its fixed GUID name and its label are Xenora
//! facts, not claims about every Windows host.

mod support;

use support::{Guest, SCRATCH};

const KERNEL32: &str = "kernel32.dll";

const IMPORTS: &[(&str, &str)] = &[
    (KERNEL32, "GetLastError"),
    (KERNEL32, "FindFirstVolumeW"),
    (KERNEL32, "FindFirstVolumeA"),
    (KERNEL32, "FindNextVolumeW"),
    (KERNEL32, "FindVolumeClose"),
    (KERNEL32, "GetVolumeInformationW"),
    (KERNEL32, "GetVolumeInformationA"),
    (KERNEL32, "GetVolumeNameForVolumeMountPointW"),
    (KERNEL32, "GetVolumePathNamesForVolumeNameA"),
];

const BUFFER: u64 = SCRATCH + 0x100;
const ERROR_NO_MORE_FILES: u64 = 18;

fn wide_at(guest: &mut Guest, address: u64, text: &str) {
    for (index, unit) in text.encode_utf16().chain(std::iter::once(0)).enumerate() {
        guest
            .memory
            .write_u16(address + index as u64 * 2, unit)
            .unwrap();
    }
}

fn narrow_at(guest: &mut Guest, address: u64, text: &str) {
    guest
        .memory
        .write_bytes(address, format!("{text}\0").as_bytes())
        .unwrap();
}

fn wide_string(guest: &Guest, address: u64) -> String {
    let mut units = Vec::new();
    let mut index = 0u64;
    loop {
        let unit = guest.memory.read_u16(address + index * 2).unwrap();
        if unit == 0 {
            break;
        }
        units.push(unit);
        index += 1;
    }
    String::from_utf16_lossy(&units)
}

#[test]
fn the_one_volume_enumerates_once_then_ends() {
    let mut guest = Guest::new(IMPORTS, &[]);
    let handle = guest.call("FindFirstVolumeW", &[BUFFER, 260]);
    assert_ne!(handle, 0);
    assert_ne!(handle, u64::MAX);
    let name = wide_string(&guest, BUFFER);
    assert!(
        name.starts_with("\\\\?\\Volume{"),
        "a GUID volume name: {name}"
    );
    // The machine has exactly one volume: the enumeration ends right after.
    assert_eq!(guest.call("FindNextVolumeW", &[handle, BUFFER, 260]), 0);
    assert_eq!(
        guest.call("GetLastError", &[]) & 0xFFFF_FFFF,
        ERROR_NO_MORE_FILES
    );
    // A buffer too small for the GUID name fails the first search.
    assert_eq!(guest.call("FindFirstVolumeA", &[BUFFER, 4]), u64::MAX);
    // Closing ends the handle.
    assert_eq!(guest.call("FindVolumeClose", &[handle]), 1);
    assert_eq!(guest.call("FindVolumeClose", &[handle]), 0);
}

#[test]
fn the_volume_information_is_fixed_and_coherent() {
    const ROOT: u64 = SCRATCH;
    const SERIAL: u64 = SCRATCH + 0x300;
    const COMPONENT: u64 = SCRATCH + 0x308;
    const FLAGS: u64 = SCRATCH + 0x310;
    let mut guest = Guest::new(IMPORTS, &[]);
    wide_at(&mut guest, ROOT, "C:\\");
    assert_eq!(
        guest.call(
            "GetVolumeInformationW",
            &[
                ROOT,
                BUFFER,
                64,
                SERIAL,
                COMPONENT,
                FLAGS,
                BUFFER + 0x80,
                64
            ]
        ),
        1
    );
    assert_eq!(wide_string(&guest, BUFFER), "XENORA");
    assert_ne!(guest.u32_at(SERIAL), 0);
    assert_eq!(guest.u32_at(COMPONENT), 255);
    assert!(wide_string(&guest, BUFFER + 0x80).starts_with("XENORA"));
    // The mount point and the volume name translate to each other.
    assert_eq!(
        guest.call("GetVolumeNameForVolumeMountPointW", &[ROOT, BUFFER, 260]),
        1
    );
    let guid = wide_string(&guest, BUFFER);
    assert!(guid.starts_with("\\\\?\\Volume{"));
    narrow_at(&mut guest, ROOT, &guid);
    assert_eq!(
        guest.call("GetVolumePathNamesForVolumeNameA", &[ROOT, BUFFER, 64]),
        1
    );
    assert_eq!(guest.memory.read_u8(BUFFER).unwrap(), b'C');
    assert_eq!(guest.memory.read_u8(BUFFER + 1).unwrap(), b':');
}
