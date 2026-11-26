// SPDX-License-Identifier: GPL-3.0-or-later

//! Synthetic guest calls for the file type, flush and ANSI-query forms.
//! Expectations use Microsoft's public contracts:
//! https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-getfiletype
//! https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers
//! https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-arefileapisansi
//!
//! FILE_TYPE_CHAR for the console handles is a Xenora-hosted-console fact, not a
//! claim about every Windows console host.

mod support;

use support::{Guest, SCRATCH};

const KERNEL32: &str = "kernel32.dll";

const IMPORTS: &[(&str, &str)] = &[
    (KERNEL32, "GetLastError"),
    (KERNEL32, "GetStdHandle"),
    (KERNEL32, "CreateFileW"),
    (KERNEL32, "CloseHandle"),
    (KERNEL32, "CreateEventW"),
    (KERNEL32, "GetFileType"),
    (KERNEL32, "FlushFileBuffers"),
    (KERNEL32, "AreFileApisANSI"),
    (KERNEL32, "GetLogicalDrives"),
    (KERNEL32, "GetDriveTypeA"),
    (KERNEL32, "GetDiskFreeSpaceExW"),
];

const WIDE: u64 = SCRATCH;
const FILE_TYPE_DISK: u64 = 1;
const FILE_TYPE_CHAR: u64 = 2;
const FILE_TYPE_UNKNOWN: u64 = 0;
const GENERIC_READ: u64 = 0x8000_0000;
const GENERIC_WRITE: u64 = 0x4000_0000;
const CREATE_ALWAYS: u64 = 2;
const INVALID_HANDLE_VALUE: u64 = u64::MAX;
const ERROR_INVALID_HANDLE: u64 = 6;
const ERROR_ACCESS_DENIED: u64 = 5;
const STD_OUTPUT_HANDLE: u64 = 0xFFFF_FFF5; // -11

fn wide(guest: &mut Guest, text: &str) -> u64 {
    let units: Vec<u8> = text
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    guest.memory.write_bytes(WIDE, &units).unwrap();
    WIDE
}

fn create_file(guest: &mut Guest, name: &str, access: u64) -> u64 {
    create_file_disposition(guest, name, access, CREATE_ALWAYS)
}

fn create_file_disposition(guest: &mut Guest, name: &str, access: u64, disposition: u64) -> u64 {
    let name = wide(guest, name);
    guest.call("CreateFileW", &[name, access, 3, 0, disposition, 0, 0])
}

fn last_error(guest: &mut Guest) -> u64 {
    guest.call("GetLastError", &[]) & 0xFFFF_FFFF
}

#[test]
fn narrow_file_operations_are_ansi() {
    let mut guest = Guest::new(IMPORTS, &[]);
    assert_eq!(guest.call("AreFileApisANSI", &[]), 1);
}

#[test]
fn the_file_type_distinguishes_disks_consoles_and_other_objects() {
    let mut guest = Guest::new(IMPORTS, &[]);
    // A disk file answers FILE_TYPE_DISK.
    let handle = create_file(&mut guest, "typed.bin", GENERIC_WRITE);
    assert_ne!(handle, INVALID_HANDLE_VALUE);
    assert_eq!(guest.call("GetFileType", &[handle]), FILE_TYPE_DISK);
    assert_eq!(guest.call("CloseHandle", &[handle]), 1);
    // The console output handle answers FILE_TYPE_CHAR.
    let console = guest.call("GetStdHandle", &[STD_OUTPUT_HANDLE]);
    assert_ne!(console, 0);
    assert_ne!(console, INVALID_HANDLE_VALUE);
    assert_eq!(guest.call("GetFileType", &[console]), FILE_TYPE_CHAR);
    // A synchronization object is a valid handle that is not a file:
    // FILE_TYPE_UNKNOWN, and the last error is not set.
    let event = guest.call("CreateEventW", &[0, 0, 0, 0]);
    assert_ne!(event, 0);
    guest.call("GetLastError", &[]);
    assert_eq!(guest.call("GetFileType", &[event]), FILE_TYPE_UNKNOWN);
    assert_eq!(
        last_error(&mut guest),
        0,
        "a valid non-file is not an error"
    );
    assert_eq!(guest.call("CloseHandle", &[event]), 1);
    // An unknown handle is FILE_TYPE_UNKNOWN with ERROR_INVALID_HANDLE.
    assert_eq!(guest.call("GetFileType", &[0xDEAD]), FILE_TYPE_UNKNOWN);
    assert_eq!(last_error(&mut guest), ERROR_INVALID_HANDLE);
}

#[test]
fn flush_file_buffers_accepts_a_written_file_and_refuses_the_rest() {
    let mut guest = Guest::new(IMPORTS, &[]);
    // A file opened for writing flushes successfully.
    let handle = create_file(&mut guest, "flush.bin", GENERIC_WRITE);
    assert_ne!(handle, INVALID_HANDLE_VALUE);
    assert_eq!(guest.call("FlushFileBuffers", &[handle]), 1);
    assert_eq!(guest.call("CloseHandle", &[handle]), 1);
    // A read-only open is refused with ERROR_ACCESS_DENIED.
    // CREATE_ALWAYS requires write access, so the reader opens existing.
    let reader = create_file_disposition(&mut guest, "flush.bin", GENERIC_READ, 3);
    assert_ne!(reader, INVALID_HANDLE_VALUE);
    assert_eq!(guest.call("FlushFileBuffers", &[reader]), 0);
    assert_eq!(last_error(&mut guest), ERROR_ACCESS_DENIED);
    assert_eq!(guest.call("CloseHandle", &[reader]), 1);
    // An unknown handle is refused with ERROR_INVALID_HANDLE.
    assert_eq!(guest.call("FlushFileBuffers", &[0xDEAD]), 0);
    assert_eq!(last_error(&mut guest), ERROR_INVALID_HANDLE);
}

/// The hosted machine has one fixed disk, letter C: bit 2 of the drive mask,
/// DRIVE_FIXED for every path, and counts that fit a memory drive.
#[test]
fn the_volume_queries_describe_the_one_disk() {
    let mut guest = Guest::new(IMPORTS, &[]);
    assert_eq!(guest.call("GetLogicalDrives", &[]), 1 << 2);
    assert_eq!(guest.call("GetDriveTypeA", &[0]), 3);
    const FREE: u64 = SCRATCH + 0x10;
    const TOTAL: u64 = SCRATCH + 0x18;
    const TOTAL_FREE: u64 = SCRATCH + 0x20;
    wide(&mut guest, "C:\\");
    assert_eq!(
        guest.call("GetDiskFreeSpaceExW", &[WIDE, FREE, TOTAL, TOTAL_FREE]),
        1
    );
    let gig = 1u64 << 30;
    assert_eq!(guest.u64_at(FREE), gig);
    assert_eq!(guest.u64_at(TOTAL), gig);
    assert_eq!(guest.u64_at(TOTAL_FREE), gig);
}
