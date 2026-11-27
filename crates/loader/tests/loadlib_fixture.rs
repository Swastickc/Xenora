// SPDX-License-Identifier: GPL-3.0-or-later

//! Static verification of the original dynamic-load fixture.
//!
//! These tests inspect only the committed PE32+ images and prepare provider TLS
//! metadata. They intentionally do not wire `LoadLibrary`, execute initializers
//! or claim end-to-end runtime behavior.

use std::collections::BTreeSet;

use xenora_loader::export_table::{ExportTable, parse_exports};
use xenora_loader::import_table::{ImportEntry, ImportName, parse_imports};
use xenora_loader::module_memory::ModuleReservation;
use xenora_loader::module_tls::prepare_module_tls;
use xenora_loader::pe::{IMAGE_FILE_DLL, IMAGE_FILE_MACHINE_AMD64};
use xenora_loader::{ExplicitModuleImage, GuestMemory, ParsedPe};

const CONSUMER: &[u8] = include_bytes!("../../../fixtures/pe/loadlib/loadlib-consumer.exe");
const PROVIDER: &[u8] = include_bytes!("../../../fixtures/pe/loadlib/xenora_loadlib.dll");
const MISSING: &[u8] = include_bytes!("../../../fixtures/pe/loadlib/xenora_loadlib_missing.dll");
const ATTACH_FAIL: &[u8] = include_bytes!("../../../fixtures/pe/loadlib/xenora_loadlib_fail.dll");

fn parse(bytes: &[u8]) -> ParsedPe {
    ParsedPe::parse(bytes).expect("fixture PE32+ image")
}

fn data_directory(bytes: &[u8], index: usize) -> (u32, u32) {
    let pe_offset = u32::from_le_bytes(bytes[0x3c..0x40].try_into().unwrap()) as usize;
    let optional = pe_offset + 24;
    let count =
        u32::from_le_bytes(bytes[optional + 108..optional + 112].try_into().unwrap()) as usize;
    assert!(index < count, "missing data directory {index}");
    let at = optional + 112 + index * 8;
    (
        u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()),
        u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()),
    )
}

fn imports(bytes: &[u8], pe: &ParsedPe) -> Vec<ImportEntry> {
    let (rva, size) = data_directory(bytes, 1);
    let (entries, error) = parse_imports(bytes, rva, size, |rva| pe.rva_to_file_offset(rva));
    assert_eq!(error, None, "fixture imports must be well formed");
    entries
}

fn exports(bytes: &[u8], pe: &ParsedPe) -> ExportTable {
    let (rva, size) = data_directory(bytes, 0);
    let (table, error) = parse_exports(bytes, rva, size, |rva| pe.rva_to_file_offset(rva));
    assert_eq!(error, None, "fixture exports must be well formed");
    table.expect("fixture DLL export table")
}

fn export_names(table: &ExportTable) -> BTreeSet<String> {
    table
        .entries
        .iter()
        .flat_map(|entry| entry.names.iter().cloned())
        .collect()
}

fn import_names(entries: &[ImportEntry]) -> BTreeSet<String> {
    entries
        .iter()
        .map(|entry| match &entry.name {
            ImportName::Name(name) => name.clone(),
            ImportName::Ordinal(ordinal) => format!("Ordinal#{ordinal}"),
        })
        .collect()
}

#[test]
fn fixture_images_are_pe32_plus_and_use_their_expected_image_kinds() {
    let consumer = parse(CONSUMER);
    assert_eq!(consumer.machine, IMAGE_FILE_MACHINE_AMD64);
    assert_eq!(consumer.characteristics & IMAGE_FILE_DLL, 0);
    assert_ne!(consumer.entry_point_rva, 0);

    for bytes in [PROVIDER, MISSING, ATTACH_FAIL] {
        let pe = parse(bytes);
        assert_eq!(pe.machine, IMAGE_FILE_MACHINE_AMD64);
        assert_ne!(pe.characteristics & IMAGE_FILE_DLL, 0);
        assert_ne!(pe.entry_point_rva, 0);
        assert_eq!(exports(bytes, &pe).module, "xenora_loadlib.dll");
    }
}

#[test]
fn consumer_imports_loader_services_and_provider_exports_evidence() {
    let consumer = parse(CONSUMER);
    let imports = imports(CONSUMER, &consumer);
    assert!(
        imports
            .iter()
            .all(|entry| entry.module.eq_ignore_ascii_case("kernel32.dll"))
    );
    assert_eq!(
        import_names(&imports),
        [
            "ExitProcess",
            "FreeLibrary",
            "GetProcAddress",
            "LoadLibraryA"
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    );

    let provider = parse(PROVIDER);
    assert_eq!(
        export_names(&exports(PROVIDER, &provider)),
        [
            "XenoraLibDetach",
            "XenoraLibInit",
            "XenoraLibTls",
            "XenoraTriple"
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    );
}

#[test]
fn control_images_have_their_negative_export_shapes() {
    let missing = parse(MISSING);
    let missing_names = export_names(&exports(MISSING, &missing));
    assert!(!missing_names.contains("XenoraTriple"));
    assert!(missing_names.contains("XenoraLibInit"));
    assert!(missing_names.contains("XenoraLibTls"));
    assert!(missing_names.contains("XenoraLibDetach"));

    let fail = parse(ATTACH_FAIL);
    assert_eq!(
        export_names(&exports(ATTACH_FAIL, &fail)),
        [
            "XenoraLibDetach",
            "XenoraLibInit",
            "XenoraLibTls",
            "XenoraTriple"
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    );
}

#[test]
fn provider_tls_preparation_discovers_but_does_not_execute_its_callback() {
    let pe = parse(PROVIDER);
    let base = pe.image_base;
    let mut memory = GuestMemory::new();
    let entry_point = pe
        .map_into_memory(PROVIDER, &mut memory, base)
        .expect("map provider");
    let images = [ExplicitModuleImage {
        pe: &pe,
        bytes: PROVIDER,
    }];
    let reservations = [ModuleReservation {
        image_index: 0,
        base,
        end: base + u64::from(pe.size_of_image),
        used_preferred_base: true,
        entry_point,
    }];

    let reports = prepare_module_tls(&images, &reservations, &mut memory, 0x3000_0000)
        .expect("prepare provider TLS");
    let prepared = reports[0].prepared.as_ref().expect("provider has TLS");
    assert_eq!(prepared.tls_index, 0);
    assert_eq!(prepared.callbacks.len(), 1);
    assert_eq!(prepared.storage.expect("TLS storage").template_bytes, 8);

    // Preparation validates and records the callback but does not run it: the
    // exported evidence variable still holds its initial zero value.
    let exports = exports(PROVIDER, &pe);
    let tls_rva = exports
        .entries
        .iter()
        .find(|entry| entry.names.iter().any(|name| name == "XenoraLibTls"))
        .and_then(|entry| match entry.target {
            xenora_loader::export_table::ExportTarget::ExportRva(rva) => Some(rva),
            _ => None,
        })
        .expect("XenoraLibTls export RVA");
    assert_eq!(memory.read_u32(base + u64::from(tls_rva)).unwrap(), 0);
}
