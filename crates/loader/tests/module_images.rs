// SPDX-License-Identifier: GPL-3.0-or-later

//! Original synthetic PE32+ images for DLL admission and multi-image mapping (#776).
//!
//! The images are built byte by byte here from the public PE/COFF layout. No
//! host library or proprietary binary takes part in these tests.

mod module_images {
    use xenora_loader::{GuestMemory, MemoryError, MemoryPermissions, ParsedPe, PeError};

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
    const EXECUTABLE_CHARACTERISTICS: u16 = 0x0022;
    const DLL_CHARACTERISTICS: u16 = 0x2022;

    const FIRST_BASE: u64 = 0x1_4000_0000;
    const SECOND_BASE: u64 = 0x1_8000_0000;
    const NONPREFERRED_BASE: u64 = 0x1_9000_0000;

    struct ImageSpec {
        characteristics: u16,
        entry_rva: u32,
        preferred_base: u64,
        text_byte: u8,
        data_byte: u8,
    }

    impl ImageSpec {
        fn executable() -> Self {
            Self {
                characteristics: EXECUTABLE_CHARACTERISTICS,
                entry_rva: TEXT_RVA,
                preferred_base: FIRST_BASE,
                text_byte: 0xC3,
                data_byte: 0x40,
            }
        }

        fn dll(entry_rva: u32, text_byte: u8, data_byte: u8) -> Self {
            Self {
                characteristics: DLL_CHARACTERISTICS,
                entry_rva,
                preferred_base: FIRST_BASE,
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

    fn parsed(spec: &ImageSpec) -> (Vec<u8>, ParsedPe) {
        let bytes = spec.build();
        let pe = ParsedPe::parse(&bytes).expect("synthetic image parses");
        (bytes, pe)
    }

    fn assert_image_bytes_and_permissions(
        memory: &mut GuestMemory,
        base: u64,
        text_byte: u8,
        data_byte: u8,
    ) {
        let text = base + u64::from(TEXT_RVA);
        let data = base + u64::from(DATA_RVA);
        assert_eq!(memory.read_u8(base), Ok(b'M'));
        assert_eq!(memory.read_u8(text), Ok(text_byte));
        assert_eq!(memory.read_u8(data), Ok(data_byte));
        assert_eq!(memory.fetch_instruction_byte(text), Ok(text_byte));
        assert_eq!(
            memory.write_bytes(text, &[0]),
            Err(MemoryError::PermissionViolation {
                address: text,
                required: "write",
            })
        );
        assert_eq!(
            memory.fetch_instruction_byte(data),
            Err(MemoryError::PermissionViolation {
                address: data,
                required: "execute",
            })
        );
    }

    #[test]
    fn zero_entry_dll_is_accepted_and_identified() {
        let spec = ImageSpec::dll(0, 0x11, 0x51);
        let (_, pe) = parsed(&spec);
        assert_eq!(pe.characteristics, DLL_CHARACTERISTICS);
        assert!(pe.is_dll());
        assert_eq!(pe.entry_point_rva, 0);
    }

    #[test]
    fn zero_entry_executable_is_rejected() {
        let mut spec = ImageSpec::executable();
        spec.entry_rva = 0;
        let bytes = spec.build();
        let error = ParsedPe::parse(&bytes).expect_err("an executable needs an entry point");
        assert_eq!(error, PeError::InvalidEntryPoint);
        let (_, executable) = parsed(&ImageSpec::executable());
        assert!(!executable.is_dll());
    }

    #[test]
    fn nonzero_dll_entry_must_name_executable_image_code() {
        for entry_rva in [DATA_RVA, TEXT_RVA + SECTION_LEN as u32, u32::MAX] {
            let bytes = ImageSpec::dll(entry_rva, 0x11, 0x51).build();
            assert_eq!(
                ParsedPe::parse(&bytes).unwrap_err(),
                PeError::InvalidEntryPoint,
                "entry {entry_rva:#x}"
            );
        }

        let (_, pe) = parsed(&ImageSpec::dll(TEXT_RVA + 1, 0x11, 0x51));
        assert!(pe.is_dll());
        assert_eq!(pe.entry_point_rva, TEXT_RVA + 1);
    }

    #[test]
    fn two_images_coexist_with_independent_bytes_and_permissions() {
        let (first_bytes, first) = parsed(&ImageSpec::dll(0, 0x11, 0x51));
        let mut second_spec = ImageSpec::dll(TEXT_RVA, 0x22, 0x62);
        second_spec.preferred_base = SECOND_BASE;
        let (second_bytes, second) = parsed(&second_spec);
        let mut memory = GuestMemory::new();

        assert_eq!(
            first.map_into_memory(&first_bytes, &mut memory, FIRST_BASE),
            Ok(None)
        );
        assert_eq!(
            second.map_into_memory(&second_bytes, &mut memory, SECOND_BASE),
            Ok(Some(SECOND_BASE + u64::from(TEXT_RVA)))
        );

        assert_image_bytes_and_permissions(&mut memory, FIRST_BASE, 0x11, 0x51);
        assert_image_bytes_and_permissions(&mut memory, SECOND_BASE, 0x22, 0x62);

        let first_data = FIRST_BASE + u64::from(DATA_RVA);
        memory.write_bytes(first_data, &[0xAA]).unwrap();
        assert_eq!(memory.read_u8(first_data), Ok(0xAA));
        assert_eq!(memory.read_u8(SECOND_BASE + u64::from(DATA_RVA)), Ok(0x62));
    }

    #[test]
    fn overlapping_placement_is_refused_without_partial_sections() {
        let (first_bytes, first) = parsed(&ImageSpec::dll(0, 0x11, 0x51));
        let mut second_spec = ImageSpec::dll(TEXT_RVA, 0x22, 0x62);
        second_spec.preferred_base = SECOND_BASE;
        let (second_bytes, second) = parsed(&second_spec);
        let mut memory = GuestMemory::new();
        assert_eq!(
            first.map_into_memory(&first_bytes, &mut memory, FIRST_BASE),
            Ok(None)
        );

        let blocker = SECOND_BASE + u64::from(DATA_RVA);
        memory
            .map_region_with_data(blocker, 0x80, MemoryPermissions::READ_WRITE, &[0x77; 0x80])
            .unwrap();
        let error = second
            .map_into_memory(&second_bytes, &mut memory, SECOND_BASE)
            .expect_err("the data section overlaps the blocker");
        assert_eq!(
            error,
            PeError::SectionMappingError(MemoryError::OverlappingRegion {
                base: blocker,
                size: SECTION_LEN as u64,
            })
        );

        // The headers and .text had no overlap and would have been mapped
        // before .data without the transaction. They are absent afterwards.
        for address in [SECOND_BASE, SECOND_BASE + u64::from(TEXT_RVA)] {
            assert_eq!(
                memory.read_u8(address),
                Err(MemoryError::UnmappedAddress(address)),
                "address {address:#x}"
            );
        }
        let mut retained = [0; 0x80];
        memory.read_bytes(blocker, &mut retained).unwrap();
        assert_eq!(retained, [0x77; 0x80]);
        memory.write_bytes(blocker, &[0x78]).unwrap();
        assert_image_bytes_and_permissions(&mut memory, FIRST_BASE, 0x11, 0x51);
    }

    #[test]
    fn malformed_second_image_leaves_memory_unchanged() {
        let (first_bytes, first) = parsed(&ImageSpec::dll(0, 0x11, 0x51));
        let mut second_spec = ImageSpec::dll(TEXT_RVA, 0x22, 0x62);
        second_spec.preferred_base = SECOND_BASE;
        let (second_bytes, second) = parsed(&second_spec);
        let mut memory = GuestMemory::new();
        assert_eq!(
            first.map_into_memory(&first_bytes, &mut memory, FIRST_BASE),
            Ok(None)
        );
        let scratch = 0x1_A000_0000;
        memory
            .map_region_with_data(scratch, 0x40, MemoryPermissions::READ_WRITE, &[0x55; 0x40])
            .unwrap();

        // The parsed image was complete. This truncated presentation still
        // contains the headers and .text, so the missing .data bytes are a
        // mapping input error rather than a parse error.
        let mut truncated = second_bytes;
        truncated.truncate(DATA_FILE + 1);
        let error = second
            .map_into_memory(&truncated, &mut memory, SECOND_BASE)
            .expect_err("truncated section data is invalid");
        assert_eq!(error, PeError::InvalidSectionHeaders);

        for address in [SECOND_BASE, SECOND_BASE + u64::from(TEXT_RVA)] {
            assert_eq!(
                memory.read_u8(address),
                Err(MemoryError::UnmappedAddress(address)),
                "address {address:#x}"
            );
        }
        let mut retained = [0; 0x40];
        memory.read_bytes(scratch, &mut retained).unwrap();
        assert_eq!(retained, [0x55; 0x40]);
        assert_image_bytes_and_permissions(&mut memory, FIRST_BASE, 0x11, 0x51);
    }

    #[test]
    fn extreme_base_address_arithmetic_is_rejected_before_mapping() {
        let (bytes, pe) = parsed(&ImageSpec::dll(TEXT_RVA, 0x33, 0x63));
        for overflowing_base in [
            u64::MAX - 0xFFF,  // header end
            u64::MAX - 0x1000, // first section end
            u64::MAX - 0x1800, // later section base
        ] {
            let mut memory = GuestMemory::new();
            let scratch = 0x1_A000_0000;
            memory
                .map_region_with_data(scratch, 1, MemoryPermissions::READ_WRITE, &[0x99])
                .unwrap();

            assert_eq!(
                pe.map_into_memory(&bytes, &mut memory, overflowing_base),
                Err(PeError::SectionMappingError(MemoryError::Overflow)),
                "base {overflowing_base:#x}"
            );
            assert_eq!(memory.read_u8(scratch), Ok(0x99));
            assert_eq!(
                memory.read_u8(overflowing_base),
                Err(MemoryError::UnmappedAddress(overflowing_base))
            );
        }
    }

    #[test]
    fn nonpreferred_base_placement_uses_the_explicit_layout() {
        let mut spec = ImageSpec::dll(TEXT_RVA, 0x33, 0x63);
        spec.preferred_base = FIRST_BASE;
        let (bytes, pe) = parsed(&spec);
        let mut memory = GuestMemory::new();

        assert_eq!(
            pe.map_into_memory(&bytes, &mut memory, NONPREFERRED_BASE),
            Ok(Some(NONPREFERRED_BASE + u64::from(TEXT_RVA)))
        );
        assert_image_bytes_and_permissions(&mut memory, NONPREFERRED_BASE, 0x33, 0x63);
        assert_eq!(
            memory.read_u8(FIRST_BASE),
            Err(MemoryError::UnmappedAddress(FIRST_BASE))
        );

        // The primitive does not rebase or patch the image: those are separate
        // loader steps. The mapped header still contains the preferred base.
        let image_base_field = NONPREFERRED_BASE + (OPTIONAL + 24) as u64;
        assert_eq!(memory.read_u64(image_base_field), Ok(FIRST_BASE));
    }

    #[test]
    fn fresh_memory_constructor_still_uses_the_preferred_base() {
        let (bytes, pe) = parsed(&ImageSpec::executable());
        let mut loaded = pe.load_into_memory(&bytes).expect("fresh load");
        assert_eq!(loaded.entry_point, FIRST_BASE + u64::from(TEXT_RVA));
        assert_image_bytes_and_permissions(&mut loaded.memory, FIRST_BASE, 0xC3, 0x40);
    }
}
