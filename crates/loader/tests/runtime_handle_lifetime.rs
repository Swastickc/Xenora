// SPDX-License-Identifier: GPL-3.0-or-later

//! Synthetic guest calls through the standard import registry. Expectations use
//! Microsoft's public CloseHandle, DuplicateHandle and synchronization contracts:
//! https://learn.microsoft.com/windows/win32/api/handleapi/nf-handleapi-closehandle
//! https://learn.microsoft.com/windows/win32/api/handleapi/nf-handleapi-duplicatehandle
//! https://learn.microsoft.com/windows/win32/sync/synchronization-object-security-and-access-rights
//!
//! Capacity, rejected rights escalation and failed-output nonmutation are Xenora
//! subset policies, not claims about every Windows handle implementation. The
//! current runtime avoids stale aliases with increasing IDs, not generations.

mod support;

use support::{Guest, SCRATCH, SCRATCH_LEN};
use xenora_loader::memory::MemoryPermissions;

const IMPORTS: &[(&str, &str)] = &[
    ("kernel32.dll", "GetLastError"),
    ("kernel32.dll", "CreateEventW"),
    ("kernel32.dll", "CreateSemaphoreW"),
    ("kernel32.dll", "SetEvent"),
    ("kernel32.dll", "ReleaseSemaphore"),
    ("kernel32.dll", "WaitForSingleObject"),
    ("kernel32.dll", "CloseHandle"),
    ("kernel32.dll", "DuplicateHandle"),
    ("kernel32.dll", "GetCurrentThread"),
    ("kernel32.dll", "GetCurrentThreadId"),
    ("kernel32.dll", "GetThreadId"),
];

const CURRENT_PROCESS: u64 = u64::MAX;
const SAME_ACCESS: u64 = 2;
const MODIFY: u64 = 2;
const SYNCHRONIZE: u64 = 0x0010_0000;
const WAIT_TIMEOUT: u64 = 258;
const WAIT_FAILED: u64 = u32::MAX as u64;
const INVALID_HANDLE: u64 = 6;
const ACCESS_DENIED: u64 = 5;
const NOACCESS: u64 = 998;
const SENTINEL: u64 = 0xABCD_1234_5678_90EF;
// Xenora's bounded object table, not a Windows limit.
const CAPACITY: usize = 1024;

fn event(guest: &mut Guest) -> u64 {
    let handle = guest.call("CreateEventW", &[0, 0, 0, 0]);
    assert_ne!(handle, 0);
    handle
}

fn duplicate(guest: &mut Guest, source: u64, output: u64, rights: u64, options: u64) -> u64 {
    guest.call(
        "DuplicateHandle",
        &[
            CURRENT_PROCESS,
            source,
            CURRENT_PROCESS,
            output,
            rights,
            0,
            options,
        ],
    )
}

fn copy(guest: &mut Guest, source: u64, rights: u64, options: u64) -> u64 {
    guest.memory.write_u64(SCRATCH, SENTINEL).unwrap();
    assert_eq!(duplicate(guest, source, SCRATCH, rights, options), 1);
    let handle = guest.u64_at(SCRATCH);
    assert_ne!(handle, 0);
    assert_ne!(handle, source);
    assert_ne!(handle, SENTINEL);
    handle
}

#[test]
fn reclaimed_capacity_never_revives_closed_handles() {
    let mut guest = Guest::new(IMPORTS, &[]);
    let handles: Vec<_> = (0..CAPACITY).map(|_| event(&mut guest)).collect();
    assert_eq!(guest.call("CreateEventW", &[0, 0, 0, 0]), 0);
    assert_eq!(guest.error(), 8); // ERROR_NOT_ENOUGH_MEMORY

    // Release one slot at a time while the table stays otherwise full.
    for old in handles {
        assert_eq!(guest.call("CloseHandle", &[old]), 1);
        let replacement = event(&mut guest);
        assert_ne!(replacement, old);
        assert_eq!(guest.call("SetEvent", &[old]), 0);
        assert_eq!(guest.error(), INVALID_HANDLE);
        assert_eq!(guest.call("CloseHandle", &[old]), 0);
        assert_eq!(guest.error(), INVALID_HANDLE);
        // Neither stale operation may signal or destroy the replacement.
        assert_eq!(
            guest.call("WaitForSingleObject", &[replacement, 0]),
            WAIT_TIMEOUT
        );
        assert_eq!(guest.call("SetEvent", &[replacement]), 1);
        assert_eq!(guest.call("WaitForSingleObject", &[replacement, 0]), 0);
    }
}

#[test]
fn duplicates_share_signal_state_but_keep_independent_rights_and_lifetimes() {
    let mut guest = Guest::new(IMPORTS, &[]);
    let source = event(&mut guest);
    let waiter = copy(&mut guest, source, SYNCHRONIZE, 0);
    let setter = copy(&mut guest, source, MODIFY, 0);

    assert_eq!(guest.call("SetEvent", &[waiter]), 0);
    assert_eq!(guest.error(), ACCESS_DENIED);
    assert_eq!(
        guest.call("WaitForSingleObject", &[source, 0]),
        WAIT_TIMEOUT
    );
    assert_eq!(guest.call("SetEvent", &[setter]), 1);
    assert_eq!(guest.call("WaitForSingleObject", &[setter, 0]), WAIT_FAILED);
    assert_eq!(guest.error(), ACCESS_DENIED);
    // A denied wait must not consume the auto-reset event's signal.
    assert_eq!(guest.call("WaitForSingleObject", &[waiter, 0]), 0);
    assert_eq!(
        guest.call("WaitForSingleObject", &[source, 0]),
        WAIT_TIMEOUT
    );

    assert_eq!(guest.call("CloseHandle", &[source]), 1);
    assert_eq!(guest.call("SetEvent", &[setter]), 1);
    assert_eq!(guest.call("WaitForSingleObject", &[waiter, 0]), 0);
    // SAME_ACCESS ignores desired access, and must retain the reduced rights.
    let second_waiter = copy(&mut guest, waiter, u64::MAX, SAME_ACCESS);
    assert_eq!(guest.call("CloseHandle", &[waiter]), 1);
    assert_eq!(guest.call("SetEvent", &[second_waiter]), 0);
    assert_eq!(guest.error(), ACCESS_DENIED);
    assert_eq!(guest.call("SetEvent", &[setter]), 1);
    assert_eq!(guest.call("CloseHandle", &[setter]), 1);
    assert_eq!(guest.call("WaitForSingleObject", &[second_waiter, 0]), 0);
    assert_eq!(
        guest.call("WaitForSingleObject", &[second_waiter, 0]),
        WAIT_TIMEOUT
    );
    assert_eq!(guest.call("CloseHandle", &[second_waiter]), 1);
}

#[test]
fn failed_duplicate_outputs_leave_memory_source_and_capacity_unchanged() {
    let mut guest = Guest::new(IMPORTS, &[]);
    let source = event(&mut guest);
    let readonly = 0x6_0000;
    guest
        .memory
        .map_region(readonly, 8, MemoryPermissions::READ_ONLY)
        .unwrap();
    guest
        .memory
        .write_u64(SCRATCH + SCRATCH_LEN - 8, SENTINEL)
        .unwrap();
    assert_eq!(guest.call("SetEvent", &[source]), 1);

    for output in [0, readonly, 0xDEAD_0000, SCRATCH + SCRATCH_LEN - 4] {
        // Repeated failures must not fill the table with unreachable duplicates.
        for _ in 0..CAPACITY {
            assert_eq!(duplicate(&mut guest, source, output, 0, SAME_ACCESS), 0);
            assert_eq!(guest.error(), NOACCESS);
        }
    }
    assert_eq!(guest.u64_at(readonly), 0);
    assert_eq!(guest.u64_at(SCRATCH + SCRATCH_LEN - 8), SENTINEL);
    assert_eq!(guest.call("WaitForSingleObject", &[source, 0]), 0);
    assert_eq!(
        guest.call("WaitForSingleObject", &[source, 0]),
        WAIT_TIMEOUT
    );
    // All remaining slots must still be available after the failed calls.
    for _ in 1..CAPACITY {
        event(&mut guest);
    }
    assert_eq!(guest.call("CreateEventW", &[0, 0, 0, 0]), 0);
    assert_eq!(guest.error(), 8);
}

#[test]
fn wrong_types_and_rights_escalation_preserve_objects_and_output() {
    let mut guest = Guest::new(IMPORTS, &[]);
    let source = event(&mut guest);
    let semaphore = guest.call("CreateSemaphoreW", &[0, 1, 2, 0]);
    assert_ne!(semaphore, 0);
    assert_eq!(guest.call("SetEvent", &[semaphore]), 0);
    assert_eq!(guest.error(), INVALID_HANDLE);
    assert_eq!(guest.call("ReleaseSemaphore", &[source, 1, 0]), 0);
    assert_eq!(guest.error(), INVALID_HANDLE);
    for handle in [source, semaphore] {
        assert_eq!(guest.call("GetThreadId", &[handle]), 0);
        assert_eq!(guest.error(), INVALID_HANDLE);
    }
    assert_eq!(guest.call("WaitForSingleObject", &[semaphore, 0]), 0);
    assert_eq!(
        guest.call("WaitForSingleObject", &[semaphore, 0]),
        WAIT_TIMEOUT
    );
    assert_eq!(
        guest.call("WaitForSingleObject", &[source, 0]),
        WAIT_TIMEOUT
    );

    let waiter = copy(&mut guest, source, SYNCHRONIZE, 0);
    guest.memory.write_u64(SCRATCH, SENTINEL).unwrap();
    assert_eq!(duplicate(&mut guest, waiter, SCRATCH, MODIFY, 0), 0);
    assert_eq!(guest.error(), ACCESS_DENIED);
    assert_eq!(guest.u64_at(SCRATCH), SENTINEL);
    assert_eq!(guest.call("SetEvent", &[source]), 1);
    assert_eq!(guest.call("WaitForSingleObject", &[waiter, 0]), 0);
    assert_eq!(guest.call("CloseHandle", &[waiter]), 1);
    assert_eq!(duplicate(&mut guest, waiter, SCRATCH, 0, SAME_ACCESS), 0);
    assert_eq!(guest.error(), INVALID_HANDLE);
    assert_eq!(guest.u64_at(SCRATCH), SENTINEL);
}

#[test]
fn thread_pseudo_handle_survives_real_duplicate_closure() {
    let mut guest = Guest::new(IMPORTS, &[]);
    let pseudo = guest.call("GetCurrentThread", &[]);
    let id = guest.call("GetCurrentThreadId", &[]);
    let first = copy(&mut guest, pseudo, 0x0800, 0); // THREAD_QUERY_LIMITED_INFORMATION
    assert_eq!(guest.call("GetThreadId", &[first]), id);
    assert_eq!(guest.call("CloseHandle", &[first]), 1);
    let second = copy(&mut guest, pseudo, 0x0800, 0);
    assert_ne!(first, second);
    assert_eq!(guest.call("GetThreadId", &[first]), 0);
    assert_eq!(guest.error(), INVALID_HANDLE);
    assert_eq!(guest.call("GetThreadId", &[second]), id);
    assert_eq!(guest.call("GetThreadId", &[pseudo]), id);
    assert_eq!(guest.call("CloseHandle", &[second]), 1);
    assert_eq!(guest.call("GetThreadId", &[pseudo]), id);
}
