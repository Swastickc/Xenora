// SPDX-License-Identifier: GPL-3.0-or-later

//! Restartable C-locale wide conversion progress and memory bounds (#1088).

mod support;

use support::*;
use xenora_loader::memory::MemoryPermissions;

const UCRT: &str = "ucrtbase.dll";
const IMPORTS: &[(&str, &str)] = &[
    (UCRT, "mbtowc"),
    (UCRT, "mbsrtowcs"),
    (UCRT, "wcsrtombs"),
    (UCRT, "_errno"),
];

const SOURCE_SLOT: u64 = SCRATCH + 0x800;
const WIDE_OUT: u64 = SCRATCH + 0x100;
const TEXT_OUT: u64 = SCRATCH + 0x200;
const PAGE: u64 = 0x60_0000;
const PAGE_END: u64 = PAGE + 0x1000;

fn guest() -> Guest {
    Guest::new(IMPORTS, &[])
}

fn wide_bytes(units: &[u16]) -> Vec<u8> {
    units.iter().flat_map(|unit| unit.to_le_bytes()).collect()
}

fn errno(guest: &mut Guest) -> u32 {
    let errno = guest.call("_errno", &[]);
    guest.u32_at(errno)
}

#[test]
fn restartable_wide_conversions_stop_exactly_at_output_capacity() {
    let mut guest = guest();
    guest
        .memory
        .map_region(PAGE, 0x1000, MemoryPermissions::READ_WRITE)
        .unwrap();

    // mbsrtowcs fills both wide slots, leaves the terminator for a second
    // call, then clears the source only when that terminator is copied.
    guest.memory.write_bytes(PAGE_END - 3, b"xy\0").unwrap();
    guest.memory.write_u64(SOURCE_SLOT, PAGE_END - 3).unwrap();
    assert_eq!(guest.call("mbsrtowcs", &[WIDE_OUT, SOURCE_SLOT, 2, 0]), 2);
    assert_eq!(guest.u64_at(SOURCE_SLOT), PAGE_END - 1);
    assert_eq!(guest.call("mbsrtowcs", &[WIDE_OUT, SOURCE_SLOT, 1, 0]), 0);
    assert_eq!(guest.u64_at(SOURCE_SLOT), 0);
    assert_eq!(guest.memory.read_u16(WIDE_OUT).unwrap(), 0);

    // wcsrtombs uses the same progress rule in the opposite direction.
    let wide = wide_bytes(&[u16::from(b'x'), u16::from(b'y'), 0]);
    guest.memory.write_bytes(PAGE_END - 6, &wide).unwrap();
    guest.memory.write_u64(SOURCE_SLOT, PAGE_END - 6).unwrap();
    assert_eq!(guest.call("wcsrtombs", &[TEXT_OUT, SOURCE_SLOT, 2, 0]), 2);
    assert_eq!(guest.u64_at(SOURCE_SLOT), PAGE_END - 2);
    assert_eq!(guest.call("wcsrtombs", &[TEXT_OUT, SOURCE_SLOT, 1, 0]), 0);
    assert_eq!(guest.u64_at(SOURCE_SLOT), 0);
    assert_eq!(guest.memory.read_u8(TEXT_OUT).unwrap(), 0);
}

#[test]
fn output_full_never_reads_the_next_source_unit() {
    let mut guest = guest();
    guest
        .memory
        .map_region(PAGE, 0x1000, MemoryPermissions::READ_WRITE)
        .unwrap();

    // The two input bytes end at the mapped boundary and have no terminator.
    // Because the output is full after them, neither conversion may probe the
    // unmapped byte or wide unit that would come next.
    guest.memory.write_bytes(PAGE_END - 2, b"xy").unwrap();
    guest.memory.write_u64(SOURCE_SLOT, PAGE_END - 2).unwrap();
    assert_eq!(guest.call("mbsrtowcs", &[WIDE_OUT, SOURCE_SLOT, 2, 0]), 2);
    assert_eq!(guest.u64_at(SOURCE_SLOT), PAGE_END);

    let wide = wide_bytes(&[u16::from(b'x'), u16::from(b'y')]);
    guest.memory.write_bytes(PAGE_END - 4, &wide).unwrap();
    guest.memory.write_u64(SOURCE_SLOT, PAGE_END - 4).unwrap();
    assert_eq!(guest.call("wcsrtombs", &[TEXT_OUT, SOURCE_SLOT, 2, 0]), 2);
    assert_eq!(guest.u64_at(SOURCE_SLOT), PAGE_END);

    // A zero-length output is full before the first unit, so it consumes and
    // reads nothing.
    guest.memory.write_u64(SOURCE_SLOT, PAGE_END - 2).unwrap();
    assert_eq!(guest.call("mbsrtowcs", &[WIDE_OUT, SOURCE_SLOT, 0, 0]), 0);
    assert_eq!(guest.u64_at(SOURCE_SLOT), PAGE_END - 2);
}

#[test]
fn sizing_queries_convert_without_output_or_source_progress() {
    let mut guest = guest();
    guest.memory.write_bytes(SCRATCH, b"abc\0").unwrap();
    guest.memory.write_u64(SOURCE_SLOT, SCRATCH).unwrap();
    assert_eq!(guest.call("mbsrtowcs", &[0, SOURCE_SLOT, 0, 0]), 3);
    assert_eq!(guest.u64_at(SOURCE_SLOT), SCRATCH);

    let wide = wide_bytes(&[u16::from(b'a'), u16::from(b'b'), u16::from(b'c'), 0]);
    guest.memory.write_bytes(SCRATCH + 0x20, &wide).unwrap();
    guest.memory.write_u64(SOURCE_SLOT, SCRATCH + 0x20).unwrap();
    assert_eq!(guest.call("wcsrtombs", &[0, SOURCE_SLOT, 0, 0]), 3);
    assert_eq!(guest.u64_at(SOURCE_SLOT), SCRATCH + 0x20);
}

#[test]
fn unrepresentable_wide_units_are_not_consumed() {
    let mut guest = guest();
    let wide = wide_bytes(&[u16::from(b'a'), 0x100, u16::from(b'b'), 0]);
    guest.memory.write_bytes(SCRATCH, &wide).unwrap();
    guest.memory.write_u64(SOURCE_SLOT, SCRATCH).unwrap();
    assert_eq!(
        guest.call("wcsrtombs", &[TEXT_OUT, SOURCE_SLOT, 8, 0]),
        u64::MAX
    );
    assert_eq!(errno(&mut guest), 42, "EILSEQ");
    assert_eq!(guest.u64_at(SOURCE_SLOT), SCRATCH + 2);
    assert_eq!(guest.memory.read_u8(TEXT_OUT).unwrap(), b'a');

    // The caller can repair or skip that unit and resume from it.
    guest
        .memory
        .write_u16(SCRATCH + 2, u16::from(b'b'))
        .unwrap();
    assert_eq!(guest.call("wcsrtombs", &[TEXT_OUT, SOURCE_SLOT, 8, 0]), 2);
    assert_eq!(guest.u64_at(SOURCE_SLOT), 0);
}

#[test]
fn mbtowc_stores_a_converted_nul_wide_character() {
    let mut guest = guest();
    guest.memory.write_u8(SCRATCH, 0).unwrap();
    guest.memory.write_u16(WIDE_OUT, 0xffff).unwrap();
    assert_eq!(guest.call("mbtowc", &[WIDE_OUT, SCRATCH, 1]), 0);
    assert_eq!(guest.memory.read_u16(WIDE_OUT).unwrap(), 0);
}
