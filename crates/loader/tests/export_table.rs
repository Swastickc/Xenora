// SPDX-License-Identifier: GPL-3.0-or-later

//! Original synthetic byte fixtures for the bounded PE32+ export-directory
//! parser. These bytes are written directly by the tests; no host or
//! proprietary executable is inspected.

// Keep the tests under this prefix so `cargo test -p xenora-loader export_table` selects them all.
mod export_table {
    use xenora_loader::export_table::{
        ExportEntry, ExportField, ExportTable, ExportTableError, ExportTarget,
        MAX_ADDRESS_TABLE_ENTRIES, MAX_NAME_LEN, MAX_NAME_POINTERS, parse_exports,
    };

    const DIRECTORY_RVA: u32 = 0x100;
    const DIRECTORY_SIZE: u32 = 0x200;
    const DIRECTORY_END: u32 = DIRECTORY_RVA + DIRECTORY_SIZE;
    const EAT: u32 = 0x130;
    const NPT: u32 = 0x140;
    const EOT: u32 = 0x150;
    const ALPHA: u32 = 0x160;
    const BETA: u32 = 0x168;
    const ZED: u32 = 0x170;
    const MODULE: u32 = 0x180;

    #[derive(Clone, Copy)]
    struct Directory {
        module_rva: u32,
        ordinal_base: u32,
        address_table_entries: u32,
        name_pointers: u32,
        address_table_rva: u32,
        name_pointer_rva: u32,
        ordinal_table_rva: u32,
    }

    fn put16(bytes: &mut [u8], rva: u32, value: u16) {
        let at = rva as usize;
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put32(bytes: &mut [u8], rva: u32, value: u32) {
        let at = rva as usize;
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn write_bytes(bytes: &mut [u8], rva: u32, value: &[u8]) {
        let at = rva as usize;
        bytes[at..at + value.len()].copy_from_slice(value);
    }

    fn write_directory(bytes: &mut [u8], directory: Directory) {
        put32(bytes, DIRECTORY_RVA, 0);
        put32(bytes, DIRECTORY_RVA + 4, 0x6655_4433);
        put16(bytes, DIRECTORY_RVA + 8, 7);
        put16(bytes, DIRECTORY_RVA + 10, 9);
        put32(bytes, DIRECTORY_RVA + 12, directory.module_rva);
        put32(bytes, DIRECTORY_RVA + 16, directory.ordinal_base);
        put32(bytes, DIRECTORY_RVA + 20, directory.address_table_entries);
        put32(bytes, DIRECTORY_RVA + 24, directory.name_pointers);
        put32(bytes, DIRECTORY_RVA + 28, directory.address_table_rva);
        put32(bytes, DIRECTORY_RVA + 32, directory.name_pointer_rva);
        put32(bytes, DIRECTORY_RVA + 36, directory.ordinal_table_rva);
    }

    fn parse_image(bytes: &[u8]) -> (Option<ExportTable>, Option<ExportTableError>) {
        parse_image_size(bytes, DIRECTORY_SIZE)
    }

    fn parse_image_size(
        bytes: &[u8],
        directory_size: u32,
    ) -> (Option<ExportTable>, Option<ExportTableError>) {
        parse_exports(bytes, DIRECTORY_RVA, directory_size, |rva| {
            let offset = rva as usize;
            (offset < bytes.len()).then_some(offset)
        })
    }

    fn valid_named_image() -> Vec<u8> {
        let mut bytes = vec![0; 0x400];
        write_directory(
            &mut bytes,
            Directory {
                module_rva: MODULE,
                ordinal_base: 7,
                address_table_entries: 4,
                name_pointers: 3,
                address_table_rva: EAT,
                name_pointer_rva: NPT,
                ordinal_table_rva: EOT,
            },
        );
        for (index, value) in [0x4000, 0x4004, 0x4008, 0x400c].into_iter().enumerate() {
            put32(&mut bytes, EAT + index as u32 * 4, value);
        }
        for (index, rva) in [ALPHA, BETA, ZED].into_iter().enumerate() {
            put32(&mut bytes, NPT + index as u32 * 4, rva);
        }
        for (index, ordinal) in [2u16, 0, 3].into_iter().enumerate() {
            put16(&mut bytes, EOT + index as u32 * 2, ordinal);
        }
        write_bytes(&mut bytes, ALPHA, b"Alpha\0");
        write_bytes(&mut bytes, BETA, b"Beta\0");
        write_bytes(&mut bytes, ZED, b"Zed\0");
        write_bytes(&mut bytes, MODULE, b"vendor.dll\0");
        bytes
    }

    fn ordinal_only_image(ordinal_base: u32, entries: &[u32]) -> Vec<u8> {
        let mut bytes = vec![0; 0x400];
        write_directory(
            &mut bytes,
            Directory {
                module_rva: MODULE,
                ordinal_base,
                address_table_entries: entries.len() as u32,
                name_pointers: 0,
                address_table_rva: EAT,
                name_pointer_rva: 0,
                ordinal_table_rva: 0,
            },
        );
        for (index, value) in entries.iter().enumerate() {
            put32(&mut bytes, EAT + index as u32 * 4, *value);
        }
        write_bytes(&mut bytes, MODULE, b"ordinal.dll\0");
        bytes
    }

    fn compact_eat_image() -> Vec<u8> {
        let mut bytes = vec![0; 0x200];
        write_directory(
            &mut bytes,
            Directory {
                module_rva: 0x128,
                ordinal_base: 1,
                address_table_entries: 3,
                name_pointers: 0,
                address_table_rva: EAT,
                name_pointer_rva: 0,
                ordinal_table_rva: 0,
            },
        );
        write_bytes(&mut bytes, 0x128, b"m\0");
        put32(&mut bytes, EAT, 0x4000);
        put32(&mut bytes, EAT + 4, 0x4004);
        put32(&mut bytes, EAT + 8, 0x4008);
        bytes
    }

    fn compact_named_image(name_pointer_rva: u32, ordinal_table_rva: u32) -> Vec<u8> {
        let mut bytes = vec![0; 0x200];
        write_directory(
            &mut bytes,
            Directory {
                module_rva: 0x128,
                ordinal_base: 1,
                address_table_entries: 2,
                name_pointers: 3,
                address_table_rva: EAT,
                name_pointer_rva,
                ordinal_table_rva,
            },
        );
        write_bytes(&mut bytes, 0x128, b"m\0");
        put32(&mut bytes, EAT, 0x4000);
        put32(&mut bytes, EAT + 4, 0x4004);
        write_bytes(&mut bytes, 0x138, b"A\0");
        write_bytes(&mut bytes, 0x13a, b"B\0");
        write_bytes(&mut bytes, 0x13c, b"C\0");
        for (position, rva) in [0x138, 0x13a, 0x13c].into_iter().enumerate() {
            put32(&mut bytes, name_pointer_rva + position as u32 * 4, rva);
        }
        for (position, ordinal) in [0u16, 1, 0].into_iter().enumerate() {
            put16(&mut bytes, ordinal_table_rva + position as u32 * 2, ordinal);
        }
        bytes
    }

    fn aliased_name_image(first_ordinal: u16, second_ordinal: u16, same_name: bool) -> Vec<u8> {
        let mut bytes = vec![0; 0x400];
        write_directory(
            &mut bytes,
            Directory {
                module_rva: MODULE,
                ordinal_base: 1,
                address_table_entries: 3,
                name_pointers: 2,
                address_table_rva: EAT,
                name_pointer_rva: NPT,
                ordinal_table_rva: EOT,
            },
        );
        for index in 0..3 {
            put32(&mut bytes, EAT + index * 4, 0x5000 + index * 4);
        }
        put32(&mut bytes, NPT, ALPHA);
        put32(&mut bytes, NPT + 4, BETA);
        put16(&mut bytes, EOT, first_ordinal);
        put16(&mut bytes, EOT + 2, second_ordinal);
        write_bytes(&mut bytes, ALPHA, b"Alpha\0");
        write_bytes(
            &mut bytes,
            BETA,
            if same_name { b"Alpha\0" } else { b"Beta\0" },
        );
        write_bytes(&mut bytes, MODULE, b"alias.dll\0");
        bytes
    }

    #[test]
    fn valid_named_exports_are_inventoried() {
        let bytes = valid_named_image();
        let (table, error) = parse_image(&bytes);

        assert_eq!(error, None);
        let table = table.unwrap();
        assert_eq!(table.time_date_stamp, 0x6655_4433);
        assert_eq!(table.major_version, 7);
        assert_eq!(table.minor_version, 9);
        assert_eq!(table.module, "vendor.dll");
        assert_eq!(table.ordinal_base, 7);
        assert_eq!(table.address_table_entries, 4);
        assert_eq!(table.name_pointers, 3);
        assert_eq!(table.entries.len(), 4);
        assert_eq!(
            table.entries[0],
            ExportEntry {
                ordinal: 7,
                index: 0,
                names: vec!["Beta".to_owned()],
                target: ExportTarget::ExportRva(0x4000),
                address_table_rva: EAT,
            }
        );
        assert!(table.entries[1].names.is_empty());
        assert_eq!(table.entries[1].ordinal, 8);
        assert_eq!(table.entries[2].names, vec!["Alpha".to_owned()]);
        assert_eq!(table.entries[2].ordinal, 9);
        assert_eq!(table.entries[3].names, vec!["Zed".to_owned()]);
        assert_eq!(table.entries[3].ordinal, 10);
        assert_eq!(table.entries[3].target, ExportTarget::ExportRva(0x400c));
        assert_eq!(table.entries[3].address_table_rva, EAT + 12);
    }

    #[test]
    fn ordinal_only_exports_need_no_name_tables() {
        let bytes = ordinal_only_image(42, &[0x6000, 0x6100]);
        let (table, error) = parse_image(&bytes);

        assert_eq!(error, None);
        let table = table.unwrap();
        assert_eq!(table.module, "ordinal.dll");
        assert_eq!(table.entries.len(), 2);
        assert_eq!(table.entries[0].ordinal, 42);
        assert_eq!(table.entries[1].ordinal, 43);
        assert!(table.entries.iter().all(|entry| entry.names.is_empty()));
        assert_eq!(table.entries[0].target, ExportTarget::ExportRva(0x6000));
        assert_eq!(table.entries[1].target, ExportTarget::ExportRva(0x6100));
    }

    #[test]
    fn zero_address_table_slots_are_explicit_ordinal_gaps() {
        let bytes = ordinal_only_image(9, &[0x6000, 0, 0x6100]);
        let (table, error) = parse_image(&bytes);

        assert_eq!(error, None);
        let entries = table.unwrap().entries;
        assert_eq!(entries[0].target, ExportTarget::ExportRva(0x6000));
        assert_eq!(entries[1].target, ExportTarget::Empty);
        assert_eq!(entries[1].ordinal, 10);
        assert_eq!(entries[2].target, ExportTarget::ExportRva(0x6100));
    }

    #[test]
    fn a_name_can_be_inventoried_for_an_empty_slot_without_becoming_exported_code() {
        let mut bytes = ordinal_only_image(9, &[0x6000, 0, 0x6100]);
        put32(&mut bytes, DIRECTORY_RVA + 24, 1);
        put32(&mut bytes, DIRECTORY_RVA + 32, NPT);
        put32(&mut bytes, DIRECTORY_RVA + 36, EOT);
        put32(&mut bytes, NPT, ALPHA);
        put16(&mut bytes, EOT, 1);
        write_bytes(&mut bytes, ALPHA, b"Gap\0");

        let (table, error) = parse_image(&bytes);

        assert_eq!(error, None);
        let entries = table.unwrap().entries;
        assert_eq!(entries[1].names, vec!["Gap".to_owned()]);
        assert_eq!(entries[1].target, ExportTarget::Empty);
    }

    #[test]
    fn forwarders_are_recognized_only_inside_the_export_directory() {
        let forwarder_rva = 0x1c0;
        let edge_forwarder = DIRECTORY_END - 2;
        let mut bytes = ordinal_only_image(
            1,
            &[
                forwarder_rva,
                DIRECTORY_END,
                DIRECTORY_RVA - 1,
                edge_forwarder,
            ],
        );
        write_bytes(&mut bytes, forwarder_rva, b"other.dll.Forwarded\0");
        write_bytes(&mut bytes, DIRECTORY_RVA - 1, b"X\0");
        write_bytes(&mut bytes, edge_forwarder, b"x\0");

        let (table, error) = parse_image(&bytes);

        assert_eq!(error, None);
        let entries = table.unwrap().entries;
        assert_eq!(
            entries[0].target,
            ExportTarget::Forwarder {
                rva: forwarder_rva,
                target: "other.dll.Forwarded".to_owned(),
            }
        );
        assert_eq!(entries[1].target, ExportTarget::ExportRva(DIRECTORY_END));
        assert_eq!(
            entries[2].target,
            ExportTarget::ExportRva(DIRECTORY_RVA - 1)
        );
        assert_eq!(
            entries[3].target,
            ExportTarget::Forwarder {
                rva: edge_forwarder,
                target: "x".to_owned(),
            }
        );
    }

    #[test]
    fn forwarder_must_end_inside_the_export_directory_and_prior_entries_remain() {
        let forwarder_rva = 0x2f8;
        let mut bytes = ordinal_only_image(1, &[0x6000, forwarder_rva]);
        write_bytes(&mut bytes, forwarder_rva, b"forwarder");
        bytes[0x301] = 0;

        let (table, error) = parse_image(&bytes);

        assert_eq!(
            error,
            Some(ExportTableError::MalformedForwarder { rva: forwarder_rva })
        );
        let entries = table.unwrap().entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].target, ExportTarget::ExportRva(0x6000));

        for byte in [0, b'\t', 0x7f, 0x80] {
            let mut bytes = ordinal_only_image(1, &[0x6000, forwarder_rva]);
            write_bytes(&mut bytes, forwarder_rva, &[byte, 0]);
            let (table, error) = parse_image(&bytes);
            assert_eq!(
                error,
                Some(ExportTableError::MalformedForwarder { rva: forwarder_rva }),
                "byte {byte}"
            );
            assert_eq!(table.unwrap().entries.len(), 1, "byte {byte}");
        }
    }

    #[test]
    fn fixed_directory_requires_declared_length_backing_and_rva_arithmetic() {
        let bytes = ordinal_only_image(1, &[0x6000]);
        let (table, error) = parse_image_size(&bytes, 39);
        assert!(table.is_none());
        assert_eq!(
            error,
            Some(ExportTableError::ExportDirectoryEnd {
                rva: DIRECTORY_RVA + 39,
            })
        );

        let (table, error) = parse_image(&bytes[..DIRECTORY_RVA as usize + 16]);
        assert!(table.is_none());
        assert_eq!(
            error,
            Some(ExportTableError::OutOfBounds { rva: DIRECTORY_RVA })
        );

        let (table, error) = parse_exports(&bytes, u32::MAX - 1, 3, |_| Some(0));
        assert!(table.is_none());
        assert_eq!(
            error,
            Some(ExportTableError::OutOfBounds { rva: u32::MAX - 1 })
        );
    }

    #[test]
    fn truncated_address_table_preserves_entries_before_the_first_error() {
        let mut bytes = compact_eat_image();
        bytes.truncate(EAT as usize + 4);

        let (table, error) = parse_image(&bytes);

        assert_eq!(error, Some(ExportTableError::OutOfBounds { rva: EAT + 4 }));
        let entries = table.unwrap().entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].ordinal, 1);
        assert_eq!(entries[0].target, ExportTarget::ExportRva(0x4000));
    }

    #[test]
    fn ordinal_base_arithmetic_cannot_overflow() {
        let bytes = ordinal_only_image(u32::MAX, &[0x6000, 0x6004]);
        let (table, error) = parse_image(&bytes);

        assert_eq!(
            error,
            Some(ExportTableError::OrdinalOverflow {
                rva: EAT + 4,
                base: u32::MAX,
                index: 1,
            })
        );
        let entries = table.unwrap().entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].ordinal, u32::MAX);

        let bytes = ordinal_only_image(u32::MAX, &[0x6000]);
        let (table, error) = parse_image(&bytes);
        assert_eq!(error, None);
        assert_eq!(table.unwrap().entries[0].ordinal, u32::MAX);
    }

    #[test]
    fn name_ordinals_must_select_an_address_table_entry() {
        let mut bytes = valid_named_image();
        put32(&mut bytes, DIRECTORY_RVA + 20, 1);
        put16(&mut bytes, EOT, 0);
        put16(&mut bytes, EOT + 2, 1);

        let (table, error) = parse_image(&bytes);

        assert_eq!(
            error,
            Some(ExportTableError::NameOrdinalOutOfRange {
                rva: EOT + 2,
                index: 1,
                address_table_entries: 1,
            })
        );
        let entries = table.unwrap().entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].names, vec!["Alpha".to_owned()]);
    }

    #[test]
    fn duplicate_and_inconsistent_name_associations_are_rejected() {
        let bytes = aliased_name_image(1, 1, true);
        let (table, error) = parse_image(&bytes);
        assert_eq!(
            error,
            Some(ExportTableError::DuplicateName {
                rva: NPT + 4,
                index: 1,
            })
        );
        assert_eq!(table.unwrap().entries[1].names, vec!["Alpha".to_owned()]);

        let bytes = aliased_name_image(1, 2, true);
        let (table, error) = parse_image(&bytes);
        assert_eq!(
            error,
            Some(ExportTableError::InconsistentName {
                rva: NPT + 4,
                first_index: 1,
                index: 2,
            })
        );
        assert_eq!(table.unwrap().entries[1].names, vec!["Alpha".to_owned()]);
    }

    #[test]
    fn distinct_names_may_alias_one_export_index() {
        let bytes = aliased_name_image(1, 1, false);
        let (table, error) = parse_image(&bytes);

        assert_eq!(error, None);
        assert_eq!(
            table.unwrap().entries[1].names,
            vec!["Alpha".to_owned(), "Beta".to_owned()]
        );
    }

    #[test]
    fn export_names_must_be_in_lexical_order() {
        let mut bytes = valid_named_image();
        put32(&mut bytes, NPT, ZED);
        put32(&mut bytes, NPT + 4, ALPHA);

        let (table, error) = parse_image(&bytes);

        assert_eq!(
            error,
            Some(ExportTableError::UnsortedNames { rva: NPT + 4 })
        );
        assert_eq!(table.unwrap().entries[2].names, vec!["Zed".to_owned()]);
    }

    #[test]
    fn module_names_must_be_printable_terminated_and_nonempty() {
        for byte in [0, b'\t', 0x7f, 0x80] {
            let mut bytes = valid_named_image();
            bytes[MODULE as usize] = byte;
            let (table, error) = parse_image(&bytes);
            assert!(table.is_none(), "byte {byte}");
            assert_eq!(
                error,
                Some(ExportTableError::MalformedModuleName { rva: MODULE }),
                "byte {byte}"
            );
        }

        let mut bytes = valid_named_image();
        bytes[MODULE as usize..].fill(b'x');
        let (table, error) = parse_image(&bytes);
        assert!(table.is_none());
        assert_eq!(
            error,
            Some(ExportTableError::MalformedModuleName { rva: MODULE })
        );
    }

    #[test]
    fn export_names_must_be_printable_terminated_and_nonempty() {
        for byte in [0, b'\n', 0x7f, 0xff] {
            let mut bytes = valid_named_image();
            bytes[BETA as usize] = byte;
            let (table, error) = parse_image(&bytes);
            assert_eq!(
                error,
                Some(ExportTableError::MalformedExportName { rva: BETA }),
                "byte {byte}"
            );
            assert_eq!(
                table.unwrap().entries[2].names,
                vec!["Alpha".to_owned()],
                "byte {byte}"
            );
        }

        let unterminated = 0x3f0;
        let mut bytes = valid_named_image();
        put32(&mut bytes, NPT + 4, unterminated);
        bytes[unterminated as usize..].fill(b'x');
        let (table, error) = parse_image(&bytes);
        assert_eq!(
            error,
            Some(ExportTableError::MalformedExportName { rva: unterminated })
        );
        assert_eq!(table.unwrap().entries[2].names, vec!["Alpha".to_owned()]);
    }

    #[test]
    fn module_name_length_is_bounded_exactly() {
        for (len, accepted) in [(MAX_NAME_LEN, true), (MAX_NAME_LEN + 1, false)] {
            let end = MODULE as usize + len + 1;
            let mut bytes = vec![0; end];
            write_directory(
                &mut bytes,
                Directory {
                    module_rva: MODULE,
                    ordinal_base: 1,
                    address_table_entries: 0,
                    name_pointers: 0,
                    address_table_rva: 0,
                    name_pointer_rva: 0,
                    ordinal_table_rva: 0,
                },
            );
            bytes[MODULE as usize..MODULE as usize + len].fill(b'x');
            let directory_size = end as u32 - DIRECTORY_RVA;
            let (table, error) = parse_image_size(&bytes, directory_size);
            if accepted {
                assert_eq!(error, None);
                assert_eq!(table.unwrap().module.len(), MAX_NAME_LEN);
            } else {
                assert!(table.is_none());
                assert_eq!(
                    error,
                    Some(ExportTableError::MalformedModuleName { rva: MODULE })
                );
            }
        }
    }

    #[test]
    fn count_limits_and_required_fields_are_enforced() {
        let mut bytes = valid_named_image();
        put32(
            &mut bytes,
            DIRECTORY_RVA + 20,
            (MAX_ADDRESS_TABLE_ENTRIES + 1) as u32,
        );
        let (table, error) = parse_image(&bytes);
        assert!(table.is_none());
        assert_eq!(error, Some(ExportTableError::TooManyAddressTableEntries));

        let mut bytes = valid_named_image();
        put32(
            &mut bytes,
            DIRECTORY_RVA + 20,
            MAX_ADDRESS_TABLE_ENTRIES as u32,
        );
        put32(&mut bytes, DIRECTORY_RVA + 28, 0);
        let (_, error) = parse_image(&bytes);
        assert!(matches!(
            error,
            Some(ExportTableError::FieldMissing {
                field: ExportField::AddressTable,
                ..
            })
        ));

        let mut bytes = valid_named_image();
        put32(
            &mut bytes,
            DIRECTORY_RVA + 24,
            (MAX_NAME_POINTERS + 1) as u32,
        );
        let (table, error) = parse_image(&bytes);
        assert!(table.is_none());
        assert_eq!(error, Some(ExportTableError::TooManyNamePointers));

        let mut bytes = valid_named_image();
        put32(&mut bytes, DIRECTORY_RVA + 24, MAX_NAME_POINTERS as u32);
        put32(&mut bytes, DIRECTORY_RVA + 32, 0);
        let (_, error) = parse_image(&bytes);
        assert!(matches!(
            error,
            Some(ExportTableError::FieldMissing {
                field: ExportField::NamePointerTable,
                ..
            })
        ));

        let cases = [
            (12, ExportField::ModuleName),
            (28, ExportField::AddressTable),
            (32, ExportField::NamePointerTable),
            (36, ExportField::OrdinalTable),
        ];
        for (offset, field) in cases {
            let mut bytes = valid_named_image();
            put32(&mut bytes, DIRECTORY_RVA + offset, 0);
            let (table, error) = parse_image(&bytes);
            assert!(table.is_none(), "{field}");
            assert_eq!(
                error,
                Some(ExportTableError::FieldMissing {
                    rva: DIRECTORY_RVA,
                    field,
                }),
                "{field}"
            );
        }
    }

    #[test]
    fn nonzero_reserved_export_flags_are_ignored_by_consumers() {
        let mut bytes = valid_named_image();
        put32(&mut bytes, DIRECTORY_RVA, 1);

        let (table, error) = parse_image(&bytes);

        assert_eq!(error, None);
        assert_eq!(table.unwrap().entries.len(), 4);
    }

    #[test]
    fn table_entry_rva_arithmetic_is_checked() {
        let high_table = u32::MAX - 3;
        let mut bytes = ordinal_only_image(1, &[0x6000, 0x6004]);
        put32(&mut bytes, DIRECTORY_RVA + 28, high_table);
        bytes[0x3f0..0x3f4].copy_from_slice(&0x6000u32.to_le_bytes());

        let (table, error) = parse_exports(&bytes, DIRECTORY_RVA, DIRECTORY_SIZE, |rva| {
            let identity = rva as usize;
            if identity < bytes.len() {
                Some(identity)
            } else if rva >= high_table {
                Some(0x3f0 + (rva - high_table) as usize)
            } else {
                None
            }
        });

        assert_eq!(
            error,
            Some(ExportTableError::OutOfBounds { rva: high_table })
        );
        assert_eq!(table.unwrap().entries.len(), 1);
    }

    #[test]
    fn truncated_name_tables_preserve_completed_name_associations() {
        let mut bytes = compact_named_image(NPT, EOT);
        bytes.truncate(0x152);
        let (table, error) = parse_image(&bytes);
        assert_eq!(error, Some(ExportTableError::OutOfBounds { rva: 0x152 }));
        let entries = table.unwrap().entries;
        assert_eq!(entries[0].names, vec!["A".to_owned()]);
        assert!(entries[1].names.is_empty());

        let mut bytes = compact_named_image(0x148, 0x140);
        bytes.truncate(0x14c);
        let (table, error) = parse_image(&bytes);
        assert_eq!(error, Some(ExportTableError::OutOfBounds { rva: 0x14c }));
        let entries = table.unwrap().entries;
        assert_eq!(entries[0].names, vec!["A".to_owned()]);
        assert!(entries[1].names.is_empty());
    }

    #[test]
    fn empty_export_table_is_bounded_but_valid() {
        let mut bytes = vec![0; 0x400];
        write_directory(
            &mut bytes,
            Directory {
                module_rva: MODULE,
                ordinal_base: 1,
                address_table_entries: 0,
                name_pointers: 0,
                address_table_rva: 0,
                name_pointer_rva: 0,
                ordinal_table_rva: 0,
            },
        );
        write_bytes(&mut bytes, MODULE, b"empty.dll\0");

        let (table, error) = parse_image(&bytes);

        assert_eq!(error, None);
        let table = table.unwrap();
        assert_eq!(table.module, "empty.dll");
        assert!(table.entries.is_empty());
    }
}
