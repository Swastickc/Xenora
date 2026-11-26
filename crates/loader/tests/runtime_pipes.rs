// SPDX-License-Identifier: GPL-3.0-or-later

//! Synthetic guest calls for anonymous pipes. Expectations use Microsoft's
//! public contracts:
//! https://learn.microsoft.com/windows/win32/api/namedpipeapi/nf-namedpipeapi-createpipe
//! https://learn.microsoft.com/windows/win32/api/namedpipeapi/nf-namedpipeapi-peeknamedpipe
//!
//! A read of an empty pipe whose write end is still open fails with
//! ERROR_NOT_SUPPORTED: blocking would deadlock the only executor, which is a
//! Xenora subset policy, not Windows behavior.

mod support;

use support::{Guest, SCRATCH};

const KERNEL32: &str = "kernel32.dll";
const UCRTBASE: &str = "ucrtbase.dll";

const IMPORTS: &[(&str, &str)] = &[
    (KERNEL32, "GetLastError"),
    (KERNEL32, "CreatePipe"),
    (KERNEL32, "ReadFile"),
    (KERNEL32, "WriteFile"),
    (KERNEL32, "CloseHandle"),
    (KERNEL32, "PeekNamedPipe"),
    (KERNEL32, "CreateFileA"),
    (UCRTBASE, "_open_osfhandle"),
    (UCRTBASE, "_get_osfhandle"),
    (UCRTBASE, "_read"),
    (UCRTBASE, "_write"),
    (UCRTBASE, "_close"),
];

const READ_END: u64 = SCRATCH;
const WRITE_END: u64 = SCRATCH + 8;
const BUFFER: u64 = SCRATCH + 0x100;
const COUNT: u64 = SCRATCH + 0x300;
const ERROR_ACCESS_DENIED: u64 = 5;
const ERROR_INVALID_HANDLE: u64 = 6;

fn pipe(guest: &mut Guest) -> (u64, u64) {
    assert_eq!(guest.call("CreatePipe", &[READ_END, WRITE_END, 0, 0]), 1);
    (guest.u64_at(READ_END), guest.u64_at(WRITE_END))
}

fn write(guest: &mut Guest, end: u64, bytes: &[u8]) -> u64 {
    guest.memory.write_bytes(BUFFER, bytes).unwrap();
    guest.call("WriteFile", &[end, BUFFER, bytes.len() as u64, COUNT, 0])
}

fn read(guest: &mut Guest, end: u64, count: u64) -> u64 {
    guest.call("ReadFile", &[end, BUFFER, count, COUNT, 0])
}

#[test]
fn bytes_written_to_a_pipe_read_back_in_order() {
    let mut guest = Guest::new(IMPORTS, &[]);
    let (read_end, write_end) = pipe(&mut guest);
    assert_ne!(read_end, 0);
    assert_ne!(write_end, 0);
    assert_ne!(read_end, write_end);
    // Two writes stream their bytes in order; a short read takes the front.
    assert_eq!(write(&mut guest, write_end, b"abc"), 1);
    assert_eq!(write(&mut guest, write_end, b"defg"), 1);
    assert_eq!(read(&mut guest, read_end, 5), 1);
    assert_eq!(guest.u32_at(COUNT), 5);
    let mut bytes = [0; 5];
    guest.memory.read_bytes(BUFFER, &mut bytes).unwrap();
    assert_eq!(&bytes, b"abcde");
    // The rest is still there for the next read.
    assert_eq!(read(&mut guest, read_end, 8), 1);
    assert_eq!(guest.u32_at(COUNT), 2);
    // Each end refuses the other's direction.
    assert_eq!(write(&mut guest, read_end, b"x"), 0);
    assert_eq!(read(&mut guest, write_end, 1), 0);
    assert_eq!(guest.call("CloseHandle", &[read_end]), 1);
    assert_eq!(guest.call("CloseHandle", &[write_end]), 1);
}

#[test]
fn closing_the_write_end_is_the_end_of_the_stream() {
    let mut guest = Guest::new(IMPORTS, &[]);
    let (read_end, write_end) = pipe(&mut guest);
    assert_eq!(write(&mut guest, write_end, b"xy"), 1);
    assert_eq!(guest.call("CloseHandle", &[write_end]), 1);
    // What remains still drains, then reads answer zero bytes: the end.
    assert_eq!(read(&mut guest, read_end, 8), 1);
    assert_eq!(guest.u32_at(COUNT), 2);
    assert_eq!(read(&mut guest, read_end, 8), 1);
    assert_eq!(guest.u32_at(COUNT), 0);
    assert_eq!(guest.call("CloseHandle", &[read_end]), 1);
}

#[test]
fn peek_reports_without_taking() {
    let mut guest = Guest::new(IMPORTS, &[]);
    let (read_end, write_end) = pipe(&mut guest);
    assert_eq!(write(&mut guest, write_end, b"peek"), 1);
    // Total bytes available at the fifth argument; the pipe keeps its bytes.
    const TOTAL: u64 = SCRATCH + 0x308;
    assert_eq!(
        guest.call("PeekNamedPipe", &[read_end, 0, 0, 0, TOTAL, 0]),
        1
    );
    assert_eq!(guest.u32_at(TOTAL), 4);
    assert_eq!(read(&mut guest, read_end, 8), 1);
    assert_eq!(guest.u32_at(COUNT), 4, "the peek took nothing");
    // The write end cannot peek, and a file handle is not a pipe.
    assert_eq!(
        guest.call("PeekNamedPipe", &[write_end, 0, 0, 0, TOTAL, 0]),
        0
    );
    assert_eq!(
        guest.call("GetLastError", &[]) & 0xFFFF_FFFF,
        ERROR_ACCESS_DENIED
    );
    assert_eq!(guest.call("PeekNamedPipe", &[0xDEAD, 0, 0, 0, TOTAL, 0]), 0);
    assert_eq!(
        guest.call("GetLastError", &[]) & 0xFFFF_FFFF,
        ERROR_INVALID_HANDLE
    );
    assert_eq!(guest.call("CloseHandle", &[read_end]), 1);
    assert_eq!(guest.call("CloseHandle", &[write_end]), 1);
}

/// `_open_osfhandle` of a kernel file object: the CRT descriptor reads and
/// writes the same bytes at the same position the kernel handle tracks, as
/// spawn redirection needs when a file stands in for a pipe.
#[test]
fn osfhandle_wraps_a_kernel_file() {
    const NAME: u64 = SCRATCH + 0x400;
    const COUNT_AT: u64 = SCRATCH + 0x700;
    let mut guest = Guest::new(IMPORTS, &[]);
    // A file with content the kernel wrote.
    guest.memory.write_bytes(NAME, b"inter.bin\0").unwrap();
    let handle = guest.call("CreateFileA", &[NAME, 0x4000_0000, 3, 0, 2, 0, 0]);
    assert_ne!(handle, u64::MAX);
    guest.memory.write_bytes(BUFFER, b"interop!").unwrap();
    assert_eq!(
        guest.call("WriteFile", &[handle, BUFFER, 8, COUNT_AT, 0]),
        1
    );
    assert_eq!(guest.call("CloseHandle", &[handle]), 1);
    // Reopen for reading and writing and wrap the handle as a descriptor.
    let handle = guest.call("CreateFileA", &[NAME, 0xC000_0000, 3, 0, 3, 0, 0]);
    assert_ne!(handle, u64::MAX);
    let fd = guest.call("_open_osfhandle", &[handle, 0]) & 0xFFFF_FFFF;
    assert_ne!(fd as i32, -1);
    assert_eq!(guest.call("_get_osfhandle", &[fd]), handle);
    // Two bytes through the CRT, then the kernel reads on from the shared
    // position — one position, both worlds.
    assert_eq!(guest.call("_read", &[fd, BUFFER, 2]) & 0xFFFF_FFFF, 2);
    let mut verify = [0u8; 8];
    guest.memory.read_bytes(BUFFER, &mut verify).unwrap();
    assert_eq!(&verify[..2], b"in");
    assert_eq!(guest.call("ReadFile", &[handle, BUFFER, 3, COUNT_AT, 0]), 1);
    assert_eq!(guest.u32_at(COUNT_AT), 3);
    guest.memory.read_bytes(BUFFER, &mut verify).unwrap();
    assert_eq!(&verify[..3], b"ter");
    // A CRT write lands where the kernel read ended, and the kernel
    // position climbs with it: one byte remains of the file afterwards.
    guest.memory.write_bytes(BUFFER, b"XY").unwrap();
    assert_eq!(guest.call("_write", &[fd, BUFFER, 2]) & 0xFFFF_FFFF, 2);
    assert_eq!(guest.call("ReadFile", &[handle, BUFFER, 8, COUNT_AT, 0]), 1);
    assert_eq!(guest.u32_at(COUNT_AT), 1);
    guest.memory.read_bytes(BUFFER, &mut verify).unwrap();
    assert_eq!(&verify[..1], b"!");
    // The CRT owns the handle now: closing the descriptor closes it.
    assert_eq!(guest.call("_close", &[fd]) & 0xFFFF_FFFF, 0);
    assert_eq!(guest.call("CloseHandle", &[handle]), 0);
    // And the content is what both worlds wrote, in order.
    let check = guest.call("CreateFileA", &[NAME, 0x8000_0000, 3, 0, 3, 0, 0]);
    assert_ne!(check, u64::MAX);
    assert_eq!(guest.call("ReadFile", &[check, BUFFER, 8, COUNT_AT, 0]), 1);
    assert_eq!(guest.u32_at(COUNT_AT), 8);
    guest.memory.read_bytes(BUFFER, &mut verify).unwrap();
    assert_eq!(&verify, b"interXY!");
    assert_eq!(guest.call("CloseHandle", &[check]), 1);
}
