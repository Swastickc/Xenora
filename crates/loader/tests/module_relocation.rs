// SPDX-License-Identifier: GPL-3.0-or-later

//! Transactional explicit-module batch relocation tests (#893).
//!
//! The PE images are original byte-built synthetic fixtures derived only from
//! the public PE/COFF format: data directory 5 (Base Relocation Table) at
//! optional-header offset 152, and blocks made of a 4-byte page RVA, a 4-byte
//! block size and 2-byte entries whose top 4 bits are the type (0 = ABSOLUTE
//! padding, 3 = HIGHLOW, 10 = DIR64). No host library or proprietary module
//! takes part in these tests.

use xenora_loader::module_layout::{
    AddressInterval, ModuleImageDescriptor, ModuleLayoutArena, ModuleLayoutPlan,
};
use xenora_loader::module_memory::ExplicitModuleImage;
use xenora_loader::{
    GuestMemory, MemoryError, MemoryPermissions, ModuleRelocationError, ModuleReservation,
    ParsedPe, RelocationError, map_module_batch, relocate_module_batch,
};

const PREFERRED: u64 = 0x1_4000_0000;
const SECOND: u64 = 0x1_8000_0000;
const RELOC_FILE: usize = 0x400;
const SIZE_OF_IMAGE: u32 = 0x3000;
const SCRATCH: u64 = 0x1_A000_0000;
// File offset of the optional header's `ImageBase` field: e_lfanew 0x40,
// optional header at 0x40 + 4 + 20, field at optional-header offset 24.
const IMAGE_BASE_FIELD: u64 = 0x58 + 24;

/// `.text` at RVA 0x1000 (file 0x200, raw size 0x200) holding two pointers at
/// RVAs 0x1010 and 0x1018, `.reloc` at RVA 0x2000 (file 0x400) holding the
/// relocation blocks. The `.text` slots are read-only/execute in memory, so a
/// successful rebase also demonstrates the primitive's permission bypass.
fn build(preferred: u64, table: &[u8], dir_size: u32) -> Vec<u8> {
    let mut pe = vec![0u8; 0x600];
    pe[0..2].copy_from_slice(b"MZ");
    pe[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    pe[0x40..0x44].copy_from_slice(b"PE\0\0");
    let coff = 0x44;
    pe[coff..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes());
    pe[coff + 2..coff + 4].copy_from_slice(&2u16.to_le_bytes());
    pe[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes());
    let opt = coff + 20;
    pe[opt..opt + 2].copy_from_slice(&0x020Bu16.to_le_bytes());
    pe[opt + 16..opt + 20].copy_from_slice(&0x1000u32.to_le_bytes());
    pe[opt + 24..opt + 32].copy_from_slice(&preferred.to_le_bytes());
    pe[opt + 32..opt + 36].copy_from_slice(&0x1000u32.to_le_bytes());
    pe[opt + 36..opt + 40].copy_from_slice(&0x200u32.to_le_bytes());
    pe[opt + 56..opt + 60].copy_from_slice(&SIZE_OF_IMAGE.to_le_bytes());
    pe[opt + 60..opt + 64].copy_from_slice(&0x200u32.to_le_bytes());
    pe[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
    pe[opt + 152..opt + 156].copy_from_slice(&0x2000u32.to_le_bytes());
    pe[opt + 156..opt + 160].copy_from_slice(&dir_size.to_le_bytes());
    for (i, (name, va, vsize, raw, flags)) in [
        (&b".text"[..], 0x1000u32, 0x200u32, 0x200u32, 0x6000_0020u32),
        (
            &b".reloc"[..],
            0x2000,
            0x1000,
            RELOC_FILE as u32,
            0x4200_0040,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let h = opt + 240 + i * 40;
        pe[h..h + name.len()].copy_from_slice(name);
        pe[h + 8..h + 12].copy_from_slice(&vsize.to_le_bytes());
        pe[h + 12..h + 16].copy_from_slice(&va.to_le_bytes());
        pe[h + 16..h + 20].copy_from_slice(&0x200u32.to_le_bytes());
        pe[h + 20..h + 24].copy_from_slice(&raw.to_le_bytes());
        pe[h + 36..h + 40].copy_from_slice(&flags.to_le_bytes());
    }
    // Two absolute pointers into the image, at RVAs 0x1010 and 0x1018.
    pe[0x210..0x218].copy_from_slice(&(preferred + 0x1100).to_le_bytes());
    pe[0x218..0x220].copy_from_slice(&(preferred + 0x2000).to_le_bytes());
    pe[RELOC_FILE..RELOC_FILE + table.len()].copy_from_slice(table);
    pe
}

fn block(page_rva: u32, entries: &[u16]) -> Vec<u8> {
    let mut b = page_rva.to_le_bytes().to_vec();
    b.extend_from_slice(&(8 + 2 * entries.len() as u32).to_le_bytes());
    for e in entries {
        b.extend_from_slice(&e.to_le_bytes());
    }
    b
}

/// DIR64 at 0x1010 and 0x1018, plus one ABSOLUTE padding entry.
fn standard_table() -> Vec<u8> {
    block(0x1000, &[0xA010, 0xA018, 0])
}

fn fixture(preferred: u64, table: Vec<u8>) -> (Vec<u8>, ParsedPe) {
    let size = table.len() as u32;
    let bytes = build(preferred, &table, size);
    let pe = ParsedPe::parse(&bytes).expect("synthetic image parses");
    (bytes, pe)
}

fn plan_for(images: &[&ParsedPe], occupied: &[AddressInterval]) -> ModuleLayoutPlan {
    let descriptors: Vec<_> = images
        .iter()
        .map(|pe| ModuleImageDescriptor::from_parsed_pe(pe))
        .collect();
    ModuleLayoutPlan::build(
        ModuleLayoutArena {
            base: 0x1_0000_0000,
            size: 0x1_0000_0000,
        },
        occupied,
        &descriptors,
    )
    .unwrap()
}

fn explicit<'a>(image: &'a (Vec<u8>, ParsedPe)) -> ExplicitModuleImage<'a> {
    ExplicitModuleImage {
        pe: &image.1,
        bytes: &image.0,
    }
}

fn map_two<'a>(
    first: &'a (Vec<u8>, ParsedPe),
    second: &'a (Vec<u8>, ParsedPe),
    occupied: &[AddressInterval],
    memory: &mut GuestMemory,
) -> ([ExplicitModuleImage<'a>; 2], Vec<ModuleReservation>) {
    let layout = plan_for(&[&first.1, &second.1], occupied);
    let images = [explicit(first), explicit(second)];
    let reservations = map_module_batch(&images, &layout, memory).unwrap();
    (images, reservations)
}

fn occupied_span(base: u64) -> AddressInterval {
    AddressInterval::new(base, base + u64::from(SIZE_OF_IMAGE))
}

fn scratch_memory() -> GuestMemory {
    let mut memory = GuestMemory::new();
    memory
        .map_region_with_data(SCRATCH, 0x40, MemoryPermissions::READ_WRITE, &[0x77; 0x40])
        .unwrap();
    memory
}

/// The two pointer slot values as mapped, before any relocation.
fn unrelocated_slots(preferred: u64) -> [u64; 2] {
    [preferred + 0x1100, preferred + 0x2000]
}

fn slots(memory: &GuestMemory, base: u64) -> [u64; 2] {
    [
        memory.read_u64_bypass(base + 0x1010).unwrap(),
        memory.read_u64_bypass(base + 0x1018).unwrap(),
    ]
}

fn assert_scratch_intact(memory: &mut GuestMemory) {
    let mut retained = [0; 0x40];
    memory.read_bytes(SCRATCH, &mut retained).unwrap();
    assert_eq!(retained, [0x77; 0x40]);
    memory.write_bytes(SCRATCH, &[0x78]).unwrap();
    assert_eq!(memory.read_u8(SCRATCH), Ok(0x78));
    memory.write_bytes(SCRATCH, &[0x77]).unwrap();
}

#[test]
fn preferred_base_batch_reports_zero_delta_and_leaves_bytes_untouched() {
    let first = fixture(PREFERRED, standard_table());
    let second = fixture(SECOND, standard_table());
    let mut memory = scratch_memory();
    let (images, reservations) = map_two(&first, &second, &[], &mut memory);
    assert!(reservations.iter().all(|r| r.used_preferred_base));

    let outcomes = relocate_module_batch(&images, &reservations, &mut memory).unwrap();
    assert_eq!(outcomes.len(), 2);
    for (outcome, &reservation) in outcomes.iter().zip(reservations.iter()) {
        assert_eq!(outcome.reservation, reservation);
        assert_eq!(outcome.report.delta, 0);
        assert_eq!(outcome.report.total_entries, 0);
        assert_eq!(outcome.report.applied_dir64, 0);
    }
    assert_eq!(slots(&memory, PREFERRED), unrelocated_slots(PREFERRED));
    assert_eq!(slots(&memory, SECOND), unrelocated_slots(SECOND));
    // Relocation does not patch the mapped header's `ImageBase` field.
    assert_eq!(
        memory
            .read_u64_bypass(PREFERRED + IMAGE_BASE_FIELD)
            .unwrap(),
        PREFERRED
    );
    assert_scratch_intact(&mut memory);
}

#[test]
fn nonpreferred_batch_rebases_every_image_in_input_order() {
    let first = fixture(PREFERRED, standard_table());
    let second = fixture(SECOND, standard_table());
    let occupied = [occupied_span(PREFERRED), occupied_span(SECOND)];
    let mut memory = scratch_memory();
    let (images, reservations) = map_two(&first, &second, &occupied, &mut memory);
    assert!(reservations.iter().all(|r| !r.used_preferred_base));

    let outcomes = relocate_module_batch(&images, &reservations, &mut memory).unwrap();
    assert_eq!(outcomes.len(), 2);
    for (image_index, (outcome, (preferred, reservation))) in outcomes
        .iter()
        .zip([PREFERRED, SECOND].iter().zip(reservations.iter()))
        .enumerate()
    {
        assert_eq!(outcome.reservation, *reservation);
        assert_eq!(outcome.reservation.image_index, image_index);
        let delta = reservation.base as i64 - *preferred as i64;
        assert_eq!(outcome.report.delta, delta);
        assert_eq!(outcome.report.applied_dir64, 2);
        assert_eq!(outcome.report.skipped_absolute, 1);
        assert_eq!(outcome.report.total_entries, 3);
        assert_eq!(
            slots(&memory, reservation.base),
            [reservation.base + 0x1100, reservation.base + 0x2000]
        );
    }
    assert_scratch_intact(&mut memory);
}

#[test]
fn shuffled_reservation_order_is_matched_by_image_index() {
    let first = fixture(PREFERRED, standard_table());
    let second = fixture(SECOND, standard_table());
    let occupied = [occupied_span(PREFERRED), occupied_span(SECOND)];
    let mut memory = GuestMemory::new();
    let (images, reservations) = map_two(&first, &second, &occupied, &mut memory);
    let shuffled = [reservations[1], reservations[0]];

    let outcomes = relocate_module_batch(&images, &shuffled, &mut memory).unwrap();
    assert_eq!(outcomes[0].reservation, reservations[0]);
    assert_eq!(outcomes[1].reservation, reservations[1]);
    for (preferred, reservation) in [PREFERRED, SECOND].iter().zip(reservations.iter()) {
        assert_eq!(
            slots(&memory, reservation.base),
            [reservation.base + 0x1100, reservation.base + 0x2000],
            "image preferred at {preferred:#x}"
        );
    }
}

#[test]
fn image_without_relocation_directory_is_an_honest_noop() {
    let first = fixture(PREFERRED, standard_table());
    let second = fixture(SECOND, Vec::new());
    let occupied = [occupied_span(SECOND)];
    let mut memory = GuestMemory::new();
    let (images, reservations) = map_two(&first, &second, &occupied, &mut memory);
    assert!(!reservations[1].used_preferred_base);

    let outcomes = relocate_module_batch(&images, &reservations, &mut memory).unwrap();
    // The delta is reported honestly even though no fixups exist.
    assert_eq!(
        outcomes[1].report.delta,
        reservations[1].base as i64 - SECOND as i64
    );
    assert_eq!(outcomes[1].report.total_entries, 0);
    assert_eq!(outcomes[1].report.applied_dir64, 0);
    assert_eq!(
        slots(&memory, reservations[1].base),
        unrelocated_slots(SECOND)
    );
}

#[test]
fn reservation_count_mismatch_is_rejected_before_memory_mutation() {
    let first = fixture(PREFERRED, standard_table());
    let second = fixture(SECOND, standard_table());
    let occupied = [occupied_span(PREFERRED), occupied_span(SECOND)];
    let mut memory = scratch_memory();
    let (images, reservations) = map_two(&first, &second, &occupied, &mut memory);
    let before = slots(&memory, reservations[0].base);

    for shortage in [&reservations[..1], &reservations[..0]] {
        assert_eq!(
            relocate_module_batch(&images, shortage, &mut memory),
            Err(ModuleRelocationError::ReservationInputMismatch {
                reservations: shortage.len(),
                module_images: 2,
            })
        );
    }
    let mut surplus = reservations.clone();
    surplus.push(reservations[0]);
    assert_eq!(
        relocate_module_batch(&images, &surplus, &mut memory),
        Err(ModuleRelocationError::ReservationInputMismatch {
            reservations: 3,
            module_images: 2,
        })
    );
    // No fixup ran: every image keeps its unrelocated preferred-base bytes.
    assert_eq!(slots(&memory, reservations[0].base), before);
    assert_eq!(
        slots(&memory, reservations[0].base),
        unrelocated_slots(PREFERRED)
    );
    assert_scratch_intact(&mut memory);
}

#[test]
fn invalid_reservation_index_is_rejected_before_memory_mutation() {
    let first = fixture(PREFERRED, standard_table());
    let second = fixture(SECOND, standard_table());
    let occupied = [occupied_span(PREFERRED), occupied_span(SECOND)];
    let mut memory = scratch_memory();
    let (images, reservations) = map_two(&first, &second, &occupied, &mut memory);

    let mut repeated = reservations.clone();
    repeated[1].image_index = 0;
    assert_eq!(
        relocate_module_batch(&images, &repeated, &mut memory),
        Err(ModuleRelocationError::InvalidReservationIndex {
            reservation_position: 1,
            image_index: 0,
        })
    );

    let mut out_of_batch = reservations.clone();
    out_of_batch[0].image_index = 2;
    assert_eq!(
        relocate_module_batch(&images, &out_of_batch, &mut memory),
        Err(ModuleRelocationError::InvalidReservationIndex {
            reservation_position: 0,
            image_index: 2,
        })
    );
    assert_eq!(
        slots(&memory, reservations[0].base),
        unrelocated_slots(PREFERRED)
    );
    assert_eq!(
        slots(&memory, reservations[1].base),
        unrelocated_slots(SECOND)
    );
    assert_scratch_intact(&mut memory);
}

#[test]
fn empty_and_reversed_reservation_spans_are_rejected_before_memory_mutation() {
    let first = fixture(PREFERRED, standard_table());
    let second = fixture(SECOND, standard_table());
    let occupied = [occupied_span(PREFERRED), occupied_span(SECOND)];
    let mut memory = scratch_memory();
    let (images, reservations) = map_two(&first, &second, &occupied, &mut memory);

    for (base, end) in [
        (reservations[0].base, reservations[0].base),
        (reservations[0].end, reservations[0].base),
    ] {
        let mut bad = reservations.clone();
        bad[0].base = base;
        bad[0].end = end;
        assert_eq!(
            relocate_module_batch(&images, &bad, &mut memory),
            Err(ModuleRelocationError::InvalidReservationSpan {
                image_index: 0,
                base,
                end,
            })
        );
    }
    assert_eq!(
        slots(&memory, reservations[0].base),
        unrelocated_slots(PREFERRED)
    );
    assert_scratch_intact(&mut memory);
}

#[test]
fn inconsistent_reservation_extent_is_rejected_before_memory_mutation() {
    let first = fixture(PREFERRED, standard_table());
    let second = fixture(SECOND, standard_table());
    let occupied = [occupied_span(PREFERRED)];
    let mut memory = GuestMemory::new();
    let (images, reservations) = map_two(&first, &second, &occupied, &mut memory);
    let mut narrowed = reservations.clone();
    narrowed[0].end = narrowed[0].base + 1;

    assert_eq!(
        relocate_module_batch(&images, &narrowed, &mut memory),
        Err(ModuleRelocationError::ReservationMismatch { image_index: 0 })
    );
    assert_eq!(
        slots(&memory, reservations[0].base),
        unrelocated_slots(PREFERRED)
    );
}

#[test]
fn malformed_relocation_after_successful_image_rolls_back_exact_state() {
    let first = fixture(PREFERRED, standard_table());
    // HIGHLOW (type 3) is not a PE32+ fixup here, so the second image fails
    // after the first image's fixups have already been applied.
    let malformed = block(0x1000, &[0x3010, 0]);
    let second = fixture(SECOND, malformed);
    let occupied = [occupied_span(PREFERRED), occupied_span(SECOND)];
    let mut memory = scratch_memory();
    let (images, reservations) = map_two(&first, &second, &occupied, &mut memory);
    let first_base = reservations[0].base;
    assert_ne!(first_base, PREFERRED);
    let second_base = reservations[1].base;

    // Control evidence: the same batch shape without the malformation does
    // apply the first image's fixups, so the rollback below really reverts
    // writes rather than merely observing their absence.
    let mut control = memory.clone();
    let valid_second = fixture(SECOND, standard_table());
    let control_images = [explicit(&first), explicit(&valid_second)];
    relocate_module_batch(&control_images, &reservations, &mut control).unwrap();
    assert_eq!(
        slots(&control, first_base),
        [first_base + 0x1100, first_base + 0x2000]
    );

    let error = relocate_module_batch(&images, &reservations, &mut memory).unwrap_err();
    assert_eq!(
        error,
        ModuleRelocationError::RelocationFailed {
            image_index: 1,
            source: RelocationError::UnsupportedRelocationType(3),
        }
    );

    // The exact pre-relocation state is restored: both images remain mapped
    // with their original unrelocated bytes, and the earlier image shows no
    // trace of the fixups the control proves were applied.
    assert_eq!(slots(&memory, first_base), unrelocated_slots(PREFERRED));
    assert_eq!(slots(&memory, second_base), unrelocated_slots(SECOND));
    assert_eq!(memory.read_u8(first_base), Ok(b'M'));
    assert_eq!(memory.read_u8(second_base), Ok(b'M'));
    assert_eq!(
        memory
            .read_u64_bypass(first_base + IMAGE_BASE_FIELD)
            .unwrap(),
        PREFERRED
    );
    assert_scratch_intact(&mut memory);
}

#[test]
fn malformed_relocation_inputs_fail_the_whole_batch_without_writes() {
    let occupied = [occupied_span(PREFERRED)];
    for (table, dir_size, source) in [
        // A block size of 7 is not an 8-byte header plus whole 2-byte entries.
        (
            {
                let mut table = 0x1000u32.to_le_bytes().to_vec();
                table.extend_from_slice(&7u32.to_le_bytes());
                table.extend_from_slice(&[0; 8]);
                table
            },
            16,
            RelocationError::MalformedBlockHeader { offset: RELOC_FILE },
        ),
        // HIGHLOW is not a supported PE32+ fixup type in this loader.
        (
            block(0x1000, &[0x3010, 0]),
            12,
            RelocationError::UnsupportedRelocationType(3),
        ),
        // The standard 14-byte block cannot fit in a 12-byte directory.
        (
            standard_table(),
            12,
            RelocationError::BlockSizeOutOfBounds {
                block_size: 14,
                max_size: 12,
            },
        ),
    ] {
        let bytes = build(PREFERRED, &table, dir_size);
        let pe = ParsedPe::parse(&bytes).unwrap();
        let images = [ExplicitModuleImage {
            pe: &pe,
            bytes: &bytes,
        }];
        let layout = plan_for(&[&pe], &occupied);
        let mut memory = scratch_memory();
        let reservations = map_module_batch(&images, &layout, &mut memory).unwrap();
        assert!(!reservations[0].used_preferred_base);
        let base = reservations[0].base;

        assert_eq!(
            relocate_module_batch(&images, &reservations, &mut memory),
            Err(ModuleRelocationError::RelocationFailed {
                image_index: 0,
                source: source.clone(),
            })
        );
        assert_eq!(slots(&memory, base), unrelocated_slots(PREFERRED));
        assert_scratch_intact(&mut memory);
    }
}

#[test]
fn read_only_targets_are_rebased_without_changing_permissions() {
    let first = fixture(PREFERRED, standard_table());
    let occupied = [occupied_span(PREFERRED)];
    let mut memory = GuestMemory::new();
    let layout = plan_for(&[&first.1], &occupied);
    let images = [explicit(&first)];
    let reservations = map_module_batch(&images, &layout, &mut memory).unwrap();
    let base = reservations[0].base;

    let outcomes = relocate_module_batch(&images, &reservations, &mut memory).unwrap();
    assert_eq!(outcomes[0].report.applied_dir64, 2);
    // The slots live in the read-only/execute `.text` section: the rebase
    // succeeded through the primitive's bypass, proving no permission had to
    // be relaxed for relocation.
    assert_eq!(
        slots(&memory, base),
        [base + 0x1100, base + 0x2000],
        "rebased through bypass"
    );

    // Permission enforcement is unchanged afterwards: ordinary writes to the
    // relocated slots, the read-only headers and `.reloc` still fault, while
    // `.text` remains executable and `.reloc` remains non-executable.
    for address in [base + 0x1010, base + 0x1018, base, base + 0x2000] {
        assert_eq!(
            memory.write_bytes(address, &[0]),
            Err(MemoryError::PermissionViolation {
                address,
                required: "write",
            }),
            "address {address:#x}"
        );
    }
    assert_eq!(
        memory.fetch_instruction_byte(base + 0x1010),
        Ok((base + 0x1100) as u8)
    );
    assert_eq!(
        memory.fetch_instruction_byte(base + 0x2000),
        Err(MemoryError::PermissionViolation {
            address: base + 0x2000,
            required: "execute",
        })
    );
    // The rebased slot bytes are readable through the ordinary read path too.
    assert_eq!(memory.read_u64(base + 0x1010).unwrap(), base + 0x1100);
}

#[test]
fn empty_batch_is_successful_and_memory_is_unchanged() {
    let mut memory = scratch_memory();
    assert_eq!(relocate_module_batch(&[], &[], &mut memory), Ok(Vec::new()));
    assert_scratch_intact(&mut memory);
    assert_eq!(
        memory.read_u8(SCRATCH + 0x40),
        Err(MemoryError::UnmappedAddress(SCRATCH + 0x40))
    );
}
