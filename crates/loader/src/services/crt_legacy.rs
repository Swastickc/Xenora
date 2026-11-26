//! Small legacy CRT services (`msvcrt.dll`) that share no state with the
//! modern runtime's tables: the wall-clock seconds, the timezone and file
//! creation mask state, the process identifier, and the legacy CRT's
//! documented multiplicative congruential `rand`.

use std::sync::{Arc, Mutex};

use crate::imports::ImportRegistry;
use crate::memory::MemoryError;

use super::ServicesState;

/// The legacy CRT module that pre-UCRT mingw binaries import directly.
const MSVCRT: &str = "msvcrt.dll";

/// 100-nanosecond intervals between the Windows and the Unix epochs
/// (Microsoft Learn, "FILETIME").
const FILETIME_TO_UNIX: u64 = 116_444_736_000_000_000;
const TICKS_PER_SECOND: u64 = 10_000_000;

fn fault(error: MemoryError) -> crate::cpu::ExitReason {
    crate::cpu::ExitReason::MemoryFault(error)
}

/// The legacy CRT's `rand` state. Microsoft documents the recurrence in the
/// CRT itself — `hold = hold * 214013 + 2531011` — and the result is bits 16
/// to 30 of it, as the widely published reference sequence for seed 1 shows:
/// 41, 18467, 6334, 26500, 19169. The state is the process's, as the legacy
/// CRT keeps it, and starts at the same value as an unseeded process: 1.
struct RandState(Mutex<u64>);

pub(super) const LEGACY_FILE_BYTES: u64 = 48;

pub(super) fn register(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    // `__iob_func` answers the legacy CRT's array of three `FILE` objects.
    // The legacy struct is 48 bytes — the mingw-w64 header's `_iobuf` fixes
    // it — and the fields a caller reads are the flags at 24 and the file
    // descriptor at 28: stdin, stdout and stderr carry theirs. The buffer
    // fields are zero, so a legacy binary that reads or writes through the
    // struct falls back to `_filbuf`/`_flsbuf` for every byte, exactly what
    // an unbuffered stream does on Windows. The array is allocated once
    // from the process heap.
    let iob = Arc::new(Mutex::new(Option::<u64>::None));
    let array = Arc::clone(&iob);
    registry.register(MSVCRT, "__iob_func", move |_, _, memory| {
        let mut kept = array.lock().expect("legacy iob array");
        if let Some(base) = *kept {
            return Ok(base);
        }
        let base = memory
            .map_heap_allocation(crate::memory::PROCESS_HEAP, 3 * LEGACY_FILE_BYTES as usize)
            .map_err(fault)?;
        for index in 0..3u64 {
            let entry = base + LEGACY_FILE_BYTES * index;
            let flags: u32 = if index == 0 { 1 } else { 2 }; // _IOREAD / _IOWRT
            memory.write_u32(entry + 24, flags).map_err(fault)?;
            memory.write_u32(entry + 28, index as u32).map_err(fault)?;
        }
        *kept = Some(base);
        Ok(base)
    });

    // `_time64` answers the seconds since the Unix epoch and stores them
    // through the argument when it is not null. The hosted clock is UTC, so
    // no timezone conversion applies.
    let s = Arc::clone(state);
    registry.register(MSVCRT, "_time64", move |args, _, memory| {
        let filetime = s.clock.now_filetime();
        let seconds = filetime.saturating_sub(FILETIME_TO_UNIX) / TICKS_PER_SECOND;
        if args.rcx != 0 {
            memory.write_u64(args.rcx, seconds).map_err(fault)?;
        }
        Ok(seconds)
    });

    // `_tzset` computes the timezone variables from the host's rules. The
    // hosted environment is UTC — documented at the `_timezone` data import —
    // so the variables already hold their only values and there is nothing
    // to compute. The function is void: on Windows it returns nothing, so a
    // caller may use whatever RAX it happens to leave; zero is honest here.
    registry.register(MSVCRT, "_tzset", |_, _, _| Ok(0));

    // `_umask` sets the file creation mask and answers the previous one. The
    // hosted runtime's files carry no permission bits yet, so the mask is
    // state without effect today, which is what makes this partial.
    let umask = Arc::new(Mutex::new(0u64));
    let mask = Arc::clone(&umask);
    registry.register(MSVCRT, "_umask", move |args, _, _| {
        let mut current = mask.lock().expect("creation mask");
        Ok(std::mem::replace(&mut *current, args.rcx & 0o777))
    });

    // `_getpid` answers the same identifier as GetCurrentProcessId.
    let pid = state.process_id;
    registry.register(MSVCRT, "_getpid", move |_, _, _| Ok(u64::from(pid)));

    // `rand` and `srand`: the documented recurrence above.
    let rand = Arc::new(RandState(Mutex::new(1)));
    let generator = Arc::clone(&rand);
    registry.register(MSVCRT, "rand", move |_, _, _| {
        let mut hold = generator.0.lock().expect("rand state");
        // The CRT's own error checking does not apply to rand, but the state
        // must never fault the process; the multiply cannot overflow the
        // wrapping arithmetic the documentation describes.
        *hold = hold.wrapping_mul(214_013).wrapping_add(2_531_011);
        Ok((*hold >> 16) & 0x7FFF)
    });
    let generator = Arc::clone(&rand);
    registry.register(MSVCRT, "srand", move |args, _, _| {
        *generator.0.lock().expect("rand state") = args.rcx;
        Ok(0)
    });
}
