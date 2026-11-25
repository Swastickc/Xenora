// SPDX-License-Identifier: GPL-3.0-or-later

//! Guest memory boundary regression tests (#82).
//!
//! Expected behavior follows x86-64 data access rules: an access is valid when
//! every byte it touches is mapped with the required permission, whether or not
//! it is aligned or crosses into an adjacent mapping. A faulting access changes
//! nothing and reports the lowest address it may not touch (#175).

use xenora_loader::{GuestMemory, MemoryError, MemoryPermissions as P};

const A: u64 = 0x10_0000;

fn two_regions(first: P, second: P) -> GuestMemory {
    let mut m = GuestMemory::new();
    m.map_region_with_data(A, 0x1000, first, &[0x11; 0x1000])
        .unwrap();
    m.map_region_with_data(A + 0x1000, 0x1000, second, &[0x22; 0x1000])
        .unwrap();
    m
}

fn bypass_bytes(m: &GuestMemory, at: u64) -> u64 {
    m.read_u64_bypass(at).unwrap()
}

#[test]
fn permissions_are_enforced_per_access_kind() {
    let mut m = GuestMemory::new();
    m.map_region(A, 0x1000, P::NO_ACCESS).unwrap();
    m.map_region(A + 0x1000, 0x1000, P::READ_ONLY).unwrap();
    m.map_region(A + 0x2000, 0x1000, P::READ_WRITE).unwrap();
    let violation = |address, required| MemoryError::PermissionViolation { address, required };
    assert_eq!(m.read_u8(A), Err(violation(A, "read")));
    assert_eq!(
        m.write_u8(A + 0x1000, 1),
        Err(violation(A + 0x1000, "write"))
    );
    assert_eq!(
        m.fetch_instruction_byte(A + 0x2000),
        Err(violation(A + 0x2000, "execute"))
    );
    assert!(m.read_u8(A + 0x1000).is_ok());
    assert!(m.write_u8(A + 0x2000, 1).is_ok());
}

#[test]
fn region_end_is_exclusive() {
    let m = two_regions(P::READ_ONLY, P::NO_ACCESS);
    assert!(m.read_u32(A + 0xFFC).is_ok());
    assert!(matches!(
        m.read_u32(A + 0xFFD),
        Err(MemoryError::PermissionViolation { .. })
    ));
    let mut lone = GuestMemory::new();
    lone.map_region(A, 0x1000, P::READ_ONLY).unwrap();
    assert_eq!(
        lone.read_u32(A + 0xFFD),
        Err(MemoryError::UnmappedAddress(A + 0x1000))
    );
    assert_eq!(
        lone.read_u8(A - 1),
        Err(MemoryError::UnmappedAddress(A - 1))
    );
}

#[test]
fn unaligned_accesses_succeed_inside_a_region() {
    let mut m = two_regions(P::READ_WRITE, P::READ_WRITE);
    m.write_u64(A + 3, 0x0102_0304_0506_0708).unwrap();
    assert_eq!(m.read_u64(A + 3), Ok(0x0102_0304_0506_0708));
    assert_eq!(m.read_u16(A + 3), Ok(0x0708));
    assert_eq!(m.read_u32(A + 5), Ok(0x0304_0506));
}

#[test]
fn access_spanning_adjacent_permitted_regions_succeeds() {
    let mut m = two_regions(P::READ_WRITE, P::READ_WRITE);
    m.write_u64(A + 0xFFC, 0x8877_6655_4433_2211).unwrap();
    assert_eq!(m.read_u64(A + 0xFFC), Ok(0x8877_6655_4433_2211));
    assert_eq!(m.read_u32(A + 0xFFC), Ok(0x4433_2211)); // low half in the first region
    assert_eq!(m.read_u32(A + 0x1000), Ok(0x8877_6655)); // high half in the second
}

#[test]
fn spanning_into_a_gap_fails_without_partial_write() {
    let mut m = GuestMemory::new();
    m.map_region_with_data(A, 0x1000, P::READ_WRITE, &[0x11; 0x1000])
        .unwrap();
    m.map_region(A + 0x2000, 0x1000, P::READ_WRITE).unwrap();
    assert_eq!(
        m.write_u64(A + 0xFFC, u64::MAX),
        Err(MemoryError::UnmappedAddress(A + 0x1000))
    );
    assert_eq!(m.read_u32(A + 0xFFC), Ok(0x1111_1111));
}

#[test]
fn spanning_into_a_forbidden_region_fails_atomically() {
    let mut m = two_regions(P::READ_WRITE, P::READ_ONLY);
    assert_eq!(
        m.write_u64(A + 0xFFC, u64::MAX),
        Err(MemoryError::PermissionViolation {
            address: A + 0x1000,
            required: "write"
        })
    );
    assert_eq!(bypass_bytes(&m, A + 0xFFC), 0x2222_2222_1111_1111);

    let m = two_regions(P::READ_EXECUTE, P::READ_WRITE);
    assert!(m.read_u64(A + 0xFFC).is_ok());
    assert!(m.fetch_instruction_byte(A + 0xFFF).is_ok());
    assert!(m.fetch_instruction_byte(A + 0x1000).is_err());
}

#[test]
fn loader_bypass_ignores_permissions_but_not_mapping() {
    let mut m = two_regions(P::READ_ONLY, P::NO_ACCESS);
    m.write_u64_bypass(A + 0xFFC, 0x0A0B_0C0D_0E0F_1011)
        .unwrap();
    assert_eq!(m.read_u64_bypass(A + 0xFFC), Ok(0x0A0B_0C0D_0E0F_1011));
    m.write_u32_bypass(A, 7).unwrap();
    assert_eq!(m.read_u32(A), Ok(7));
    assert_eq!(
        m.write_u64_bypass(A + 0x1FFC, 0),
        Err(MemoryError::UnmappedAddress(A + 0x2000))
    );
}

#[test]
fn address_overflow_is_reported() {
    let mut m = GuestMemory::new();
    m.map_region(u64::MAX - 0xFFF, 0xFFF, P::READ_WRITE)
        .unwrap();
    assert_eq!(m.read_u64(u64::MAX - 3), Err(MemoryError::Overflow));
    assert_eq!(
        m.write_u64_bypass(u64::MAX - 3, 0),
        Err(MemoryError::Overflow)
    );
    assert_eq!(
        m.map_region(u64::MAX, 2, P::READ_WRITE),
        Err(MemoryError::Overflow)
    );
}

#[test]
fn overlapping_mappings_are_rejected_and_adjacent_ones_accepted() {
    let mut m = GuestMemory::new();
    m.map_region(A, 0x1000, P::READ_WRITE).unwrap();
    assert_eq!(
        m.map_region(A + 0xFFF, 0x10, P::READ_WRITE),
        Err(MemoryError::OverlappingRegion {
            base: A + 0xFFF,
            size: 0x10
        })
    );
    assert!(m.map_region(A + 0x1000, 0x10, P::READ_WRITE).is_ok());
    assert!(m.map_region(A - 0x10, 0x10, P::READ_WRITE).is_ok());
}
