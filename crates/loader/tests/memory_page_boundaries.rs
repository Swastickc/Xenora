// SPDX-License-Identifier: GPL-3.0-or-later

//! Guest 4 KiB page boundaries on a host with larger pages (#175).
//!
//! Apple Silicon uses 16 KiB host pages, so four guest pages share one host
//! page and host page protection cannot separate them. Guest memory is checked
//! in software, byte by byte, and is never mapped with host protections. These
//! tests lay 4 KiB guest pages across two spans of the detected host page size
//! and require the same results at every guest page boundary, whether or not it
//! is also a host page boundary.
//!
//! The expected fault address is the first byte, in address order, that the
//! access may not touch: the linear address with no translation or without the
//! needed right. This is the reading of the Intel SDM used by the page-fault
//! cases in `cpu_known_answers.rs` (Volume 3A, Sections 5.7 and 7.15, Interrupt
//! 14). Every expectation below is a literal address or a table written from
//! that rule, not a value read back from the memory model. Nothing here was
//! compared with x86 hardware.

use xenora_loader::services::sysinfo::HostCapabilities;
use xenora_loader::{GuestMemory, MemoryError, MemoryPermissions as P};

const GUEST_PAGE: u64 = 0x1000;
/// Aligned to every host page size up to 1 GiB.
const BASE: u64 = 0x4000_0000;

fn host_page() -> u64 {
    let size = HostCapabilities::detect()
        .page_size
        .expect("the host reports a page size");
    assert_eq!(size % GUEST_PAGE, 0, "host page size {size:#x}");
    size
}

/// Guest page boundaries strictly inside two host pages starting at `BASE`.
/// On a 16 KiB host that is seven boundaries, one of them a host page boundary.
fn boundaries() -> Vec<u64> {
    let pages = 2 * host_page() / GUEST_PAGE;
    (1..pages).map(|page| BASE + page * GUEST_PAGE).collect()
}

fn unmapped(address: u64) -> MemoryError {
    MemoryError::UnmappedAddress(address)
}

fn violation(address: u64, required: &'static str) -> MemoryError {
    MemoryError::PermissionViolation { address, required }
}

/// The page below `boundary`, filled with 0x5a, and optionally the page above
/// it, filled with 0xa5.
fn pages_around(boundary: u64, below: P, above: Option<P>) -> GuestMemory {
    let mut memory = GuestMemory::new();
    memory
        .map_region_with_data(boundary - GUEST_PAGE, 0x1000, below, &[0x5a; 0x1000])
        .unwrap();
    if let Some(above) = above {
        memory
            .map_region_with_data(boundary, 0x1000, above, &[0xa5; 0x1000])
            .unwrap();
    }
    memory
}

/// The 16 bytes centred on `boundary`, read without permission checks.
fn bytes_around(memory: &GuestMemory, boundary: u64) -> (u64, u64) {
    (
        memory.read_u64_bypass(boundary - 8).unwrap(),
        memory.read_u64_bypass(boundary).unwrap(),
    )
}

const BELOW: u64 = 0x5a5a_5a5a_5a5a_5a5a;
const ABOVE: u64 = 0xa5a5_a5a5_a5a5_a5a5;

#[test]
fn apple_silicon_host_pages_hold_four_guest_pages() {
    let size = host_page();
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        assert_eq!(size, 0x4000);
        assert_eq!(boundaries().len(), 7);
        assert!(boundaries().contains(&(BASE + 0x4000)));
    }
}

#[test]
fn each_guest_page_in_a_host_page_keeps_its_own_permissions() {
    // (permissions, read allowed, write allowed, execute allowed)
    let kinds = [
        (P::READ_WRITE, true, true, false),
        (P::READ_ONLY, true, false, false),
        (P::NO_ACCESS, false, false, false),
        (P::READ_EXECUTE, true, false, true),
    ];
    let pages = 2 * host_page() / GUEST_PAGE;
    let mut memory = GuestMemory::new();
    for page in 0..pages {
        let fill = [page as u8 + 1; 0x1000];
        let kind = kinds[page as usize % kinds.len()].0;
        memory
            .map_region_with_data(BASE + page * GUEST_PAGE, 0x1000, kind, &fill)
            .unwrap();
    }
    for page in 0..pages {
        let (_, read, write, execute) = kinds[page as usize % kinds.len()];
        let fill = page as u8 + 1;
        let first = BASE + page * GUEST_PAGE;
        for address in [first, first + 0xFFF] {
            let expected = |allowed, required| {
                if allowed {
                    Ok(fill)
                } else {
                    Err(violation(address, required))
                }
            };
            assert_eq!(memory.read_u8(address), expected(read, "read"));
            assert_eq!(
                memory.fetch_instruction_byte(address),
                expected(execute, "execute")
            );
            assert_eq!(
                memory.write_u8(address, fill),
                expected(write, "write").map(|_| ())
            );
        }
    }
    assert_eq!(memory.read_u8(BASE - 1), Err(unmapped(BASE - 1)));
    let end = BASE + pages * GUEST_PAGE;
    assert_eq!(memory.read_u8(end), Err(unmapped(end)));
}

#[test]
fn access_across_adjacent_guest_pages_lands_in_both() {
    for boundary in boundaries() {
        let mut memory = pages_around(boundary, P::READ_WRITE, Some(P::READ_WRITE));
        memory
            .write_u64(boundary - 3, 0x8877_6655_4433_2211)
            .unwrap();
        assert_eq!(memory.read_u64(boundary - 3), Ok(0x8877_6655_4433_2211));
        // Little endian: three bytes below the boundary, five above it.
        assert_eq!(
            bytes_around(&memory, boundary),
            (0x3322_115a_5a5a_5a5a, 0xa5a5_a588_7766_5544),
            "boundary {boundary:#x}"
        );
    }
}

#[test]
fn crossing_into_an_unmapped_page_faults_at_its_first_byte() {
    for boundary in boundaries() {
        let mut memory = pages_around(boundary, P::READ_WRITE, None);
        let fault = Err(unmapped(boundary));
        for back in 1..8u64 {
            let start = boundary - back;
            let context = format!("boundary {boundary:#x}, start -{back}");
            assert_eq!(memory.read_u64(start).map(|_| ()), fault, "{context}");
            assert_eq!(memory.write_u64(start, 0), fault, "{context}");
            assert_eq!(memory.write_u64_bypass(start, 0), fault, "{context}");
            assert_eq!(memory.read_u64_bypass(start).map(|_| ()), fault);
        }
        for (back, width) in [(1u64, 2usize), (3, 4), (15, 16), (1, 0x1001)] {
            let mut buffer = vec![0u8; width];
            assert_eq!(memory.read_bytes(boundary - back, &mut buffer), fault);
            assert_eq!(memory.write_bytes(boundary - back, &buffer), fault);
        }
        // The failed stores wrote nothing below the boundary, and the last
        // access that fits still succeeds.
        assert_eq!(memory.read_u64(boundary - 8), Ok(BELOW));
        // An access that does not touch the mapped page faults at its own
        // first byte.
        assert_eq!(memory.read_u64(boundary).map(|_| ()), fault);
        assert_eq!(
            memory.read_u64(boundary + 5).map(|_| ()),
            Err(unmapped(boundary + 5))
        );
    }
}

#[test]
fn starting_in_an_unmapped_page_faults_at_the_access_start() {
    for boundary in boundaries() {
        // Only the page above the boundary exists.
        let mut memory = GuestMemory::new();
        memory
            .map_region_with_data(boundary, 0x1000, P::READ_WRITE, &[0xa5; 0x1000])
            .unwrap();
        for back in 1..8u64 {
            let start = boundary - back;
            assert_eq!(memory.read_u64(start).map(|_| ()), Err(unmapped(start)));
            assert_eq!(memory.write_u64(start, 0), Err(unmapped(start)));
        }
        assert_eq!(memory.read_u64(boundary), Ok(ABOVE));
    }
}

#[test]
fn crossing_into_a_page_without_the_right_faults_at_its_first_byte() {
    for boundary in boundaries() {
        // A store into a read-only page.
        let mut memory = pages_around(boundary, P::READ_WRITE, Some(P::READ_ONLY));
        for back in 1..8u64 {
            assert_eq!(
                memory.write_u64(boundary - back, 0),
                Err(violation(boundary, "write")),
                "boundary {boundary:#x}, start -{back}"
            );
            assert!(memory.read_u64(boundary - back).is_ok());
        }
        assert_eq!(bytes_around(&memory, boundary), (BELOW, ABOVE));

        // A load from a page with no access.
        let memory = pages_around(boundary, P::READ_ONLY, Some(P::NO_ACCESS));
        for back in 1..8u64 {
            assert_eq!(
                memory.read_u64(boundary - back).map(|_| ()),
                Err(violation(boundary, "read")),
                "boundary {boundary:#x}, start -{back}"
            );
        }
        assert_eq!(memory.read_u64(boundary - 8), Ok(BELOW));
    }
}

#[test]
fn the_lowest_failing_byte_decides_the_fault() {
    for boundary in boundaries() {
        // Forbidden first page, unmapped second page: the first byte fails.
        let mut memory = pages_around(boundary, P::READ_ONLY, None);
        assert_eq!(
            memory.write_u64(boundary - 4, 0),
            Err(violation(boundary - 4, "write"))
        );
        assert_eq!(
            memory.read_u64(boundary - 4).map(|_| ()),
            Err(unmapped(boundary))
        );

        // Writable, then read-only, then unmapped: a store over all three
        // stops at the read-only page, a load at the unmapped one.
        let mut memory = pages_around(boundary, P::READ_WRITE, Some(P::READ_ONLY));
        let mut buffer = vec![0u8; 0x1008];
        assert_eq!(
            memory.write_bytes(boundary - 4, &buffer),
            Err(violation(boundary, "write"))
        );
        assert_eq!(
            memory.read_bytes(boundary - 4, &mut buffer),
            Err(unmapped(boundary + GUEST_PAGE))
        );
        assert_eq!(bytes_around(&memory, boundary), (BELOW, ABOVE));
    }
}

#[test]
fn the_highest_guest_page_boundaries_do_not_wrap() {
    // The last page that can be mapped ends one page below 2^64.
    let top = u64::MAX - 0xFFF; // 0xffff_ffff_ffff_f000
    let mut memory = pages_around(top, P::READ_WRITE, None);
    assert_eq!(
        memory.map_region(top, 0x1000, P::READ_WRITE),
        Err(MemoryError::Overflow)
    );
    assert_eq!(memory.read_u64(top - 8), Ok(BELOW));
    assert_eq!(memory.read_u64(top - 4).map(|_| ()), Err(unmapped(top)));
    assert_eq!(memory.write_u64(top - 4, 0), Err(unmapped(top)));
    // An access whose end passes 2^64 is an overflow wherever it starts.
    assert_eq!(memory.read_u64(u64::MAX - 6), Err(MemoryError::Overflow));
    assert_eq!(
        memory.write_u64(u64::MAX - 6, 0),
        Err(MemoryError::Overflow)
    );
    assert_eq!(memory.read_u64(top - 8), Ok(BELOW));
}
