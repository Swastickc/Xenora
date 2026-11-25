// SPDX-License-Identifier: GPL-3.0-or-later

//! The injected guest clock read through every time service at once.
//!
//! Units come from the public Windows documentation: a FILETIME and the
//! unbiased interrupt time count 100-nanosecond intervals, the tick counts are
//! milliseconds, and a performance count divided by the reported frequency is
//! seconds. Expected values are worked out by hand from those units, not taken
//! from the service code.

mod support;

use std::sync::Arc;
use support::*;
use xenora_loader::services::{MockClock, ServicesState};

const KERNEL32: &str = "kernel32.dll";

const IMPORTS: &[(&str, &str)] = &[
    (KERNEL32, "QueryPerformanceCounter"),
    (KERNEL32, "QueryPerformanceFrequency"),
    (KERNEL32, "GetTickCount"),
    (KERNEL32, "GetTickCount64"),
    (KERNEL32, "QueryUnbiasedInterruptTime"),
    (KERNEL32, "GetSystemTimeAsFileTime"),
    (KERNEL32, "Sleep"),
];

/// 2026-11-24 12:34:56 UTC as a FILETIME: 1,795,523,696 Unix seconds plus the
/// 11,644,473,600 seconds between 1601 and 1970, times ten million.
const START_FILETIME: u64 = 134_399_972_960_000_000;
/// Clock time that passes before the guest process starts.
const BEFORE_START: u64 = 1_234;

/// One reading of every time service.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Reading {
    counter: u64,
    frequency: u64,
    tick: u64,
    tick64: u64,
    interrupt: u64,
    filetime: u64,
}

fn guest() -> (Guest, Arc<MockClock>) {
    let clock = Arc::new(MockClock::new(START_FILETIME));
    clock.advance(BEFORE_START);
    let guest = Guest::with_state(IMPORTS, &[], ServicesState::new(clock.clone()));
    (guest, clock)
}

fn read(guest: &mut Guest) -> Reading {
    let output = |guest: &mut Guest, symbol: &str| {
        guest.memory.write_u64(SCRATCH, 0).unwrap();
        guest.call(symbol, &[SCRATCH]);
        guest.u64_at(SCRATCH)
    };
    Reading {
        counter: output(guest, "QueryPerformanceCounter"),
        frequency: output(guest, "QueryPerformanceFrequency"),
        interrupt: output(guest, "QueryUnbiasedInterruptTime"),
        filetime: output(guest, "GetSystemTimeAsFileTime"),
        tick: guest.call("GetTickCount", &[]),
        tick64: guest.call("GetTickCount64", &[]),
    }
}

#[test]
fn every_time_service_reports_the_same_elapsed_time_in_its_own_unit() {
    let (mut guest, clock) = guest();
    // Uptime starts with the process; the counter and the calendar do not.
    assert_eq!(
        read(&mut guest),
        Reading {
            counter: 1_234,
            frequency: 10_000_000,
            tick: 0,
            tick64: 0,
            interrupt: 0,
            filetime: 134_399_972_960_001_234,
        }
    );

    // 3 s + 7 ms + 3 intervals.
    clock.advance(30_070_003);
    let after = read(&mut guest);
    assert_eq!(
        after,
        Reading {
            counter: 30_071_237,
            frequency: 10_000_000,
            tick: 3_007,
            tick64: 3_007,
            interrupt: 30_070_003,
            filetime: 134_399_972_990_071_237,
        }
    );
    assert_eq!((after.counter - 1_234) / after.frequency, 3);

    // A guest sleep is the same time as a host advance: 250 ms.
    assert_eq!(guest.call("Sleep", &[250]), 0);
    assert_eq!(
        read(&mut guest),
        Reading {
            counter: 32_571_237,
            frequency: 10_000_000,
            tick: 3_257,
            tick64: 3_257,
            interrupt: 32_570_003,
            filetime: 134_399_972_992_571_237,
        }
    );
}

#[test]
fn no_reading_decreases_up_to_and_past_the_end_of_the_clock() {
    let (mut guest, clock) = guest();
    let mut last = read(&mut guest);
    let steps = [
        0,
        1,
        9_998,
        1,
        10_000_000,
        1 << 40,
        i64::MAX as u64 / 2,
        i64::MAX as u64 / 2,
        u64::MAX / 2,
        u64::MAX,
        1,
    ];
    for step in steps {
        clock.advance(step);
        let now = read(&mut guest);
        assert!(
            now.counter >= last.counter,
            "{step}: {now:?} after {last:?}"
        );
        assert!(now.tick64 >= last.tick64, "{step}: {now:?} after {last:?}");
        assert!(
            now.interrupt >= last.interrupt,
            "{step}: {now:?} after {last:?}"
        );
        assert!(
            now.filetime >= last.filetime,
            "{step}: {now:?} after {last:?}"
        );
        // The 32-bit count is the low half of the 64-bit one and may wrap.
        assert_eq!(now.tick, now.tick64 & 0xFFFF_FFFF, "{step}");
        assert_eq!(now.frequency, 10_000_000, "{step}");
        last = now;
    }

    // Both readings stop at their maximum; neither wraps.
    let end = Reading {
        counter: 0x7FFF_FFFF_FFFF_FFFF,
        frequency: 10_000_000,
        // (2^63 - 1 - 1,234) intervals, in whole milliseconds.
        tick: 922_337_203_685_477 & 0xFFFF_FFFF,
        tick64: 922_337_203_685_477,
        interrupt: 0x7FFF_FFFF_FFFF_FFFF - 1_234,
        filetime: u64::MAX,
    };
    assert_eq!(last, end);

    // Sleeping at the end of the clock still returns and changes nothing.
    assert_eq!(guest.call("Sleep", &[1]), 0);
    assert_eq!(read(&mut guest), end);
}
