// SPDX-License-Identifier: GPL-3.0-or-later

//! Transactional explicit-module batch mapping tests (#889).
//!
//! The PE images are original byte-built synthetic fixtures. No host library
//! or proprietary module takes part in these tests.

mod module_memory {
    use xenora_loader::module_layout::{
        AddressInterval, ImageLayoutOutcome, ModuleImageDescriptor, ModuleLayoutArena,
        ModuleLayoutPlan,
    };
    use xenora_loader::module_memory::ExplicitModuleImage;
    use xenora_loader::{
        GuestMemory, MemoryError, MemoryPermissions, ModuleMemoryError, ParsedPe, PeError,
        map_module_batch,
    };

    const E_LFANEW: usize = 0x80;
    const COFF: usize = E_LFANEW + 4;
    const OPTIONAL: usize = COFF + 20;
    const OPTIONAL_SIZE: usize = 112;
    const SECTION_TABLE: usize = OPTIONAL + OPTIONAL_SIZE;
    const SECTION_LEN: usize = 0x100;
    const TEXT_FILE: usize = 0x200;
    const DATA_FILE: usize = 0x300;
    const FILE_LEN: usize = 0x400;
    const TEXT_RVA: u32 = 0x1000;
    const DATA_RVA: u32 = 0x2000;
    const SIZE_OF_IMAGE: u32 = 0x3000;
    const SIZE_OF_HEADERS: u32 = 0x200;
    const RX_SECTION: u32 = 0x6000_0020;
    const RW_SECTION: u32 = 0xC000_0040;
    const DLL_CHARACTERISTICS: u16 = 0x2022;
    const RELOCS_STRIPPED: u16 = 0x0001;
    const FIRST_BASE: u64 = 0x1_4000_0000;
    const SECOND_BASE: u64 = 0x1_8000_0000;
    const SCRATCH: u64 = 0x1_A000_0000;

    struct ImageSpec {
        characteristics: u16,
        entry_rva: u32,
        preferred_base: u64,
        text_byte: u8,
        data_byte: u8,
    }

    impl ImageSpec {
        fn dll(entry_rva: u32, preferred_base: u64, text_byte: u8, data_byte: u8) -> Self {
            Self {
                characteristics: DLL_CHARACTERISTICS,
                entry_rva,
                preferred_base,
                text_byte,
                data_byte,
            }
        }

        fn build(&self) -> Vec<u8> {
            let mut pe = vec![0; FILE_LEN];
            pe[0..2].copy_from_slice(b"MZ");
            pe[0x3C..0x40].copy_from_slice(&(E_LFANEW as u32).to_le_bytes());
            pe[E_LFANEW..E_LFANEW + 4].copy_from_slice(b"PE\0\0");
            pe[COFF..COFF + 2].copy_from_slice(&0x8664u16.to_le_bytes());
            pe[COFF + 2..COFF + 4].copy_from_slice(&2u16.to_le_bytes());
            pe[COFF + 16..COFF + 18].copy_from_slice(&(OPTIONAL_SIZE as u16).to_le_bytes());
            pe[COFF + 18..COFF + 20].copy_from_slice(&self.characteristics.to_le_bytes());
            pe[OPTIONAL..OPTIONAL + 2].copy_from_slice(&0x020Bu16.to_le_bytes());
            pe[OPTIONAL + 16..OPTIONAL + 20].copy_from_slice(&self.entry_rva.to_le_bytes());
            pe[OPTIONAL + 24..OPTIONAL + 32].copy_from_slice(&self.preferred_base.to_le_bytes());
            pe[OPTIONAL + 32..OPTIONAL + 36].copy_from_slice(&0x1000u32.to_le_bytes());
            pe[OPTIONAL + 36..OPTIONAL + 40].copy_from_slice(&0x200u32.to_le_bytes());
            pe[OPTIONAL + 56..OPTIONAL + 60].copy_from_slice(&SIZE_OF_IMAGE.to_le_bytes());
            pe[OPTIONAL + 60..OPTIONAL + 64].copy_from_slice(&SIZE_OF_HEADERS.to_le_bytes());
            write_section(
                &mut pe,
                0,
                b".text\0\0\0",
                TEXT_RVA,
                TEXT_FILE as u32,
                RX_SECTION,
            );
            write_section(
                &mut pe,
                1,
                b".data\0\0\0",
                DATA_RVA,
                DATA_FILE as u32,
                RW_SECTION,
            );
            pe[TEXT_FILE..TEXT_FILE + SECTION_LEN].fill(self.text_byte);
            pe[DATA_FILE..DATA_FILE + SECTION_LEN].fill(self.data_byte);
            pe
        }
    }

    fn write_section(
        pe: &mut [u8],
        index: usize,
        name: &[u8; 8],
        rva: u32,
        raw: u32,
        characteristics: u32,
    ) {
        let at = SECTION_TABLE + index * 40;
        pe[at..at + 8].copy_from_slice(name);
        pe[at + 8..at + 12].copy_from_slice(&(SECTION_LEN as u32).to_le_bytes());
        pe[at + 12..at + 16].copy_from_slice(&rva.to_le_bytes());
        pe[at + 16..at + 20].copy_from_slice(&(SECTION_LEN as u32).to_le_bytes());
        pe[at + 20..at + 24].copy_from_slice(&raw.to_le_bytes());
        pe[at + 36..at + 40].copy_from_slice(&characteristics.to_le_bytes());
    }

    fn image(spec: &ImageSpec) -> (Vec<u8>, ParsedPe) {
        let bytes = spec.build();
        let pe = ParsedPe::parse(&bytes).expect("synthetic image parses");
        (bytes, pe)
    }

    fn arena() -> ModuleLayoutArena {
        ModuleLayoutArena {
            base: 0x1_0000_0000,
            size: 0x1_0000_0000,
        }
    }

    fn plan(
        parsed: &[&ParsedPe],
        occupied: &[AddressInterval],
    ) -> Result<ModuleLayoutPlan, xenora_loader::module_layout::ModuleLayoutError> {
        let descriptors: Vec<ModuleImageDescriptor> = parsed
            .iter()
            .map(|pe| ModuleImageDescriptor::from_parsed_pe(pe))
            .collect();
        ModuleLayoutPlan::build(arena(), occupied, &descriptors)
    }

    fn explicit<'a>(bytes: &'a [u8], pe: &'a ParsedPe) -> ExplicitModuleImage<'a> {
        ExplicitModuleImage { pe, bytes }
    }

    fn assert_image(memory: &mut GuestMemory, base: u64, text: u8, data: u8) {
        assert_eq!(memory.read_u8(base), Ok(b'M'));
        assert_eq!(memory.read_u8(base + u64::from(TEXT_RVA)), Ok(text));
        assert_eq!(memory.read_u8(base + u64::from(DATA_RVA)), Ok(data));
        assert_eq!(
            memory.fetch_instruction_byte(base + u64::from(TEXT_RVA)),
            Ok(text)
        );
        assert_eq!(
            memory.fetch_instruction_byte(base + u64::from(DATA_RVA)),
            Err(MemoryError::PermissionViolation {
                address: base + u64::from(DATA_RVA),
                required: "execute",
            })
        );
    }

    fn scratch_memory() -> GuestMemory {
        let mut memory = GuestMemory::new();
        memory
            .map_region_with_data(SCRATCH, 0x40, MemoryPermissions::READ_WRITE, &[0x77; 0x40])
            .unwrap();
        memory
    }

    fn assert_scratch_and_absent_images(memory: &mut GuestMemory, bases: &[u64]) {
        assert_eq!(memory.read_u8(SCRATCH), Ok(0x77));
        memory.write_bytes(SCRATCH, &[0x78]).unwrap();
        assert_eq!(memory.read_u8(SCRATCH), Ok(0x78));
        memory.write_bytes(SCRATCH, &[0x77]).unwrap();
        for &base in bases {
            for offset in [0, TEXT_RVA, DATA_RVA] {
                let address = base + u64::from(offset);
                assert_eq!(
                    memory.read_u8(address),
                    Err(MemoryError::UnmappedAddress(address)),
                    "address {address:#x}"
                );
            }
        }
    }

    #[test]
    fn complete_batch_maps_in_order_and_reports_reservations() {
        let (first_bytes, first_pe) = image(&ImageSpec::dll(0, FIRST_BASE, 0x11, 0x51));
        let (second_bytes, second_pe) = image(&ImageSpec::dll(TEXT_RVA, SECOND_BASE, 0x22, 0x62));
        let layout = plan(&[&first_pe, &second_pe], &[]).unwrap();
        assert!(layout.all_placed());
        let images = [
            explicit(&first_bytes, &first_pe),
            explicit(&second_bytes, &second_pe),
        ];
        let mut memory = scratch_memory();

        let reservations = map_module_batch(&images, &layout, &mut memory).unwrap();
        assert_eq!(reservations.len(), 2);
        assert_eq!(reservations[0].image_index, 0);
        assert_eq!(reservations[0].base, FIRST_BASE);
        assert_eq!(reservations[0].end, FIRST_BASE + u64::from(SIZE_OF_IMAGE));
        assert!(reservations[0].used_preferred_base);
        assert_eq!(reservations[0].entry_point, None);
        assert_eq!(reservations[1].image_index, 1);
        assert_eq!(reservations[1].base, SECOND_BASE);
        assert_eq!(
            reservations[1].entry_point,
            Some(SECOND_BASE + u64::from(TEXT_RVA))
        );
        assert_image(&mut memory, FIRST_BASE, 0x11, 0x51);
        assert_image(&mut memory, SECOND_BASE, 0x22, 0x62);
        assert_eq!(memory.read_u8(SCRATCH), Ok(0x77));
    }

    #[test]
    fn fallback_base_is_preserved_in_reservation_metadata() {
        let (bytes, pe) = image(&ImageSpec::dll(TEXT_RVA, FIRST_BASE, 0x33, 0x63));
        let occupied = [AddressInterval::new(FIRST_BASE, FIRST_BASE + 0x4000)];
        let layout = plan(&[&pe], &occupied).unwrap();
        let (_, placement) = layout.placements().next().unwrap();
        assert!(!placement.used_preferred_base);
        assert_ne!(placement.base, FIRST_BASE);
        assert_eq!(placement.base % 0x1_0000, 0);
        let images = [explicit(&bytes, &pe)];
        let mut memory = GuestMemory::new();

        let reservations = map_module_batch(&images, &layout, &mut memory).unwrap();
        assert_eq!(reservations[0].base, placement.base);
        assert!(!reservations[0].used_preferred_base);
        assert_image(&mut memory, placement.base, 0x33, 0x63);
        assert_eq!(
            memory.read_u8(FIRST_BASE),
            Err(MemoryError::UnmappedAddress(FIRST_BASE))
        );
    }

    #[test]
    fn empty_batch_is_successful_and_does_not_disturb_memory() {
        let layout = ModuleLayoutPlan::build(arena(), &[], &[]).unwrap();
        let mut memory = scratch_memory();
        let reservations = map_module_batch(&[], &layout, &mut memory).unwrap();
        assert!(reservations.is_empty());
        assert_eq!(memory.read_u8(SCRATCH), Ok(0x77));
    }

    #[test]
    fn plan_input_mismatch_is_rejected_before_memory_mutation() {
        let (first_bytes, first_pe) = image(&ImageSpec::dll(0, FIRST_BASE, 0x11, 0x51));
        let (second_bytes, second_pe) = image(&ImageSpec::dll(TEXT_RVA, SECOND_BASE, 0x22, 0x62));
        let layout = plan(&[&first_pe], &[]).unwrap();
        let images = [
            explicit(&first_bytes, &first_pe),
            explicit(&second_bytes, &second_pe),
        ];
        let mut memory = scratch_memory();

        assert_eq!(
            map_module_batch(&images, &layout, &mut memory),
            Err(ModuleMemoryError::PlanInputMismatch {
                plan_images: 1,
                module_images: 2,
            })
        );
        assert_scratch_and_absent_images(&mut memory, &[FIRST_BASE, SECOND_BASE]);
    }

    #[test]
    fn extra_plan_record_is_rejected_before_memory_mutation() {
        let (first_bytes, first_pe) = image(&ImageSpec::dll(0, FIRST_BASE, 0x11, 0x51));
        let (_second_bytes, second_pe) = image(&ImageSpec::dll(TEXT_RVA, SECOND_BASE, 0x22, 0x62));
        let layout = plan(&[&first_pe, &second_pe], &[]).unwrap();
        let images = [explicit(&first_bytes, &first_pe)];
        let mut memory = scratch_memory();

        assert_eq!(
            map_module_batch(&images, &layout, &mut memory),
            Err(ModuleMemoryError::PlanInputMismatch {
                plan_images: 2,
                module_images: 1,
            })
        );
        assert_scratch_and_absent_images(&mut memory, &[FIRST_BASE, SECOND_BASE]);
    }

    #[test]
    fn failed_or_mismatched_layout_is_rejected_before_memory_mutation() {
        let mut fixed = ImageSpec::dll(TEXT_RVA, FIRST_BASE, 0x22, 0x62);
        fixed.characteristics |= RELOCS_STRIPPED;
        let (first_bytes, first_pe) = image(&ImageSpec::dll(0, FIRST_BASE, 0x11, 0x51));
        let (second_bytes, second_pe) = image(&fixed);
        let occupied = [AddressInterval::new(
            FIRST_BASE,
            FIRST_BASE + u64::from(SIZE_OF_IMAGE),
        )];
        let layout = plan(&[&first_pe, &second_pe], &occupied).unwrap();
        assert!(matches!(
            layout.images()[1].outcome,
            ImageLayoutOutcome::Failed(_)
        ));
        let fallback_base = layout.placements().next().unwrap().1.base;
        assert_ne!(fallback_base, FIRST_BASE);
        let images = [
            explicit(&first_bytes, &first_pe),
            explicit(&second_bytes, &second_pe),
        ];
        let mut memory = scratch_memory();

        assert_eq!(
            map_module_batch(&images, &layout, &mut memory),
            Err(ModuleMemoryError::MissingPlacement { image_index: 1 })
        );
        assert_scratch_and_absent_images(&mut memory, &[FIRST_BASE, fallback_base]);

        let wrong_descriptor = [ModuleImageDescriptor::from_parsed_pe(&first_pe)];
        let mismatched_layout = ModuleLayoutPlan::build(arena(), &[], &wrong_descriptor).unwrap();
        assert_eq!(
            map_module_batch(&images[1..], &mismatched_layout, &mut memory),
            Err(ModuleMemoryError::DescriptorMismatch { image_index: 0 })
        );
        assert_scratch_and_absent_images(&mut memory, &[FIRST_BASE]);
    }

    #[test]
    fn invalid_image_bytes_roll_back_the_whole_batch() {
        let (first_bytes, first_pe) = image(&ImageSpec::dll(0, FIRST_BASE, 0x11, 0x51));
        let (second_bytes, second_pe) = image(&ImageSpec::dll(TEXT_RVA, SECOND_BASE, 0x22, 0x62));
        let layout = plan(&[&first_pe, &second_pe], &[]).unwrap();
        let mut truncated = second_bytes;
        truncated.truncate(DATA_FILE + 1);
        let images = [
            explicit(&first_bytes, &first_pe),
            explicit(&truncated, &second_pe),
        ];
        let mut memory = scratch_memory();

        assert_eq!(
            map_module_batch(&images, &layout, &mut memory),
            Err(ModuleMemoryError::MappingFailed {
                image_index: 1,
                source: PeError::InvalidSectionHeaders,
            })
        );
        assert_scratch_and_absent_images(&mut memory, &[FIRST_BASE, SECOND_BASE]);

        // The rollback also removed the first image's regions rather than
        // merely preserving their bytes: the same range can be mapped again.
        memory
            .map_region_with_data(FIRST_BASE, 1, MemoryPermissions::READ_WRITE, &[0x99])
            .unwrap();
        assert_eq!(memory.read_u8(FIRST_BASE), Ok(0x99));
    }

    #[test]
    fn insertion_failure_after_partial_mapping_rolls_back_the_whole_batch() {
        let (first_bytes, first_pe) = image(&ImageSpec::dll(0, FIRST_BASE, 0x11, 0x51));
        let (second_bytes, second_pe) = image(&ImageSpec::dll(TEXT_RVA, SECOND_BASE, 0x22, 0x62));
        let layout = plan(&[&first_pe, &second_pe], &[]).unwrap();
        let images = [
            explicit(&first_bytes, &first_pe),
            explicit(&second_bytes, &second_pe),
        ];
        let mut memory = scratch_memory();
        let blocker = SECOND_BASE + u64::from(DATA_RVA);
        memory
            .map_region_with_data(blocker, 0x80, MemoryPermissions::READ_WRITE, &[0x55; 0x80])
            .unwrap();

        assert_eq!(
            map_module_batch(&images, &layout, &mut memory),
            Err(ModuleMemoryError::MappingFailed {
                image_index: 1,
                source: PeError::SectionMappingError(MemoryError::OverlappingRegion {
                    base: blocker,
                    size: SECTION_LEN as u64,
                }),
            })
        );
        assert_scratch_and_absent_images(&mut memory, &[FIRST_BASE]);
        for offset in [0, TEXT_RVA] {
            let address = SECOND_BASE + u64::from(offset);
            assert_eq!(
                memory.read_u8(address),
                Err(MemoryError::UnmappedAddress(address)),
                "address {address:#x}"
            );
        }

        // The pre-existing blocker is part of the exact prior state: its bytes
        // and write permission survive even though the second image's headers
        // and .text had been inserted before its .data collision was found.
        let mut retained = [0; 0x80];
        memory.read_bytes(blocker, &mut retained).unwrap();
        assert_eq!(retained, [0x55; 0x80]);
        memory.write_bytes(blocker, &[0x56]).unwrap();
        assert_eq!(memory.read_u8(blocker), Ok(0x56));
    }
}
