// SPDX-License-Identifier: GPL-3.0-or-later

//! Static probe reports for delay-loaded PE imports.
//!
//! The expected names and slot RVAs are read off the layout that
//! `fixtures/pe/delay-imports.py` writes, not from the parser.
//! `llvm-readobj --coff-imports` from LLVM 23.1.6 lists the same imports in
//! the same order, with the address tables at RVA 0x2050, 0x3000 and 0x3020.

use std::collections::BTreeMap;
use xenora_loader::import_table::{DelayField, ImportTableError, parse_delay_imports};
use xenora_loader::imports::{ImportError, ImportRegistry};
use xenora_loader::pe::ParsedPe;
use xenora_loader::probe::{
    ImportedSymbol, ProbeConfig, ProbeReport, probe_pe_bytes, run_probe_and_export,
};

const FIXTURE: &[u8] = include_bytes!("../../../fixtures/pe/delay-imports.exe");

/// DLL, symbol or ordinal, and import address table slot RVA.
type Row = (&'static str, &'static str, u32);

const ORDINARY: [Row; 2] = [
    ("fixture-ordinary.dll", "OrdinaryByName", 0x2050),
    ("fixture-ordinary.dll", "Ordinal#3", 0x2058),
];
const DELAYED: [Row; 3] = [
    ("fixture-delay-a.dll", "DelayedAlpha", 0x3000),
    ("fixture-delay-a.dll", "Ordinal#7", 0x3008),
    ("fixture-delay-b.dll", "DelayedBeta", 0x3020),
];

// File offsets in the fixture.
const NUMBER_OF_RVA_AND_SIZES: usize = 0x98 + 108;
const DELAY_DIRECTORY_ENTRY: usize = 0x98 + 216;
const DELAY_DIRECTORY_SIZE: usize = DELAY_DIRECTORY_ENTRY + 4;
const RDATA_RAW_SIZE: usize = 0x188 + 40 + 16;
/// The delay-load directory is at RVA 0x2200; `.rdata` maps RVA 0x2000 to 0x600.
const FIRST_DESCRIPTOR: usize = 0x800;
const SECOND_DESCRIPTOR: usize = 0x820;

fn import_strings_with_spaces() -> Vec<u8> {
    // These offsets point to the fixture's module and hint/name strings.
    // Equal-length replacements leave the terminators and tables intact.
    let mut pe = FIXTURE.to_vec();
    pe[0x670..0x684].copy_from_slice(b"fixture ordinary.dll");
    pe[0x692..0x6A0].copy_from_slice(b"Ordinary  Name");
    pe[0x730..0x743].copy_from_slice(b"fixture delay-a.dll");
    pe[0x772..0x77E].copy_from_slice(b"Delayed Name");
    pe
}

fn rows(imports: &[ImportedSymbol]) -> Vec<(&str, &str, u32)> {
    imports
        .iter()
        .map(|import| (import.dll.as_str(), import.symbol.as_str(), import.iat_rva))
        .collect()
}

fn put(pe: &mut [u8], offset: usize, value: u32) {
    pe[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// Probe `pe` and return the report with the text files it exports.
fn export(pe: &[u8], name: &str) -> (ProbeReport, BTreeMap<&'static str, String>) {
    let dir =
        std::env::temp_dir().join(format!("xenora-probe-delay-{name}-{}", std::process::id()));
    let report = run_probe_and_export(pe, &dir, &ProbeConfig::default()).unwrap();
    let files = [
        "imports.csv",
        "missing_imports.csv",
        "candidate_static_report.json",
        "candidate_static_report.md",
        "runtime_gap_matrix.md",
    ]
    .into_iter()
    .map(|file| (file, std::fs::read_to_string(dir.join(file)).unwrap()))
    .collect();
    std::fs::remove_dir_all(dir).unwrap();
    (report, files)
}

fn json_field(json: &str, name: &str) -> String {
    let prefix = format!("\"{name}\": ");
    json.lines()
        .find_map(|line| line.trim().strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("no {name} in the JSON report"))
        .trim_end_matches(',')
        .trim_matches('"')
        .to_string()
}

#[test]
fn ordinary_and_delayed_imports_are_reported_apart() {
    let (report, files) = export(FIXTURE, "fixture");

    assert_eq!(rows(&report.imports), ORDINARY);
    assert_eq!(rows(&report.delayed_imports), DELAYED);
    assert_eq!(report.import_error, None);
    assert_eq!(report.delay_import_error, None);

    let mut lines = files["imports.csv"].lines();
    let header = lines.next().unwrap();
    assert!(header.starts_with("module,symbol_or_ordinal,iat_rva,import_table,"));
    assert!(header.ends_with(",import_kind"));
    let columns = header.split(',').count();
    // The first four columns and the last one of every row.
    let listed: Vec<String> = lines
        .map(|line| {
            let fields: Vec<&str> = line.split(',').collect();
            assert_eq!(fields.len(), columns, "{line}");
            format!("{},{}", fields[..4].join(","), fields[columns - 1])
        })
        .collect();
    assert_eq!(
        listed,
        [
            "\"fixture-ordinary.dll\",\"OrdinaryByName\",0x00002050,ordinary,name",
            "\"fixture-ordinary.dll\",\"Ordinal#3\",0x00002058,ordinary,ordinal",
            "\"fixture-delay-a.dll\",\"DelayedAlpha\",0x00003000,delayed,name",
            "\"fixture-delay-a.dll\",\"Ordinal#7\",0x00003008,delayed,ordinal",
            "\"fixture-delay-b.dll\",\"DelayedBeta\",0x00003020,delayed,name",
        ]
    );

    // Neither fixture DLL has a handler, so both ordinary imports are missing.
    // The delayed ones are not the binder's to resolve and stay off its list.
    assert_eq!(
        files["missing_imports.csv"].lines().collect::<Vec<_>>(),
        [
            "module,symbol_or_ordinal,iat_rva,import_kind",
            "\"fixture-ordinary.dll\",\"OrdinaryByName\",0x00002050,name",
            "\"fixture-ordinary.dll\",\"Ordinal#3\",0x00002058,ordinal",
        ]
    );

    let json = &files["candidate_static_report.json"];
    for (name, expected) in [
        ("total_imports", "2"),
        ("imported_dlls_count", "1"),
        ("unresolved_imports", "2"),
        ("distinct_import_keys", "2"),
        ("delayed_imports", "3"),
        ("delayed_imported_dlls_count", "2"),
        ("delay_import_error", "none"),
    ] {
        assert_eq!(json_field(json, name), expected, "{name}");
    }

    let md = &files["candidate_static_report.md"];
    assert!(md.contains("- **Total Imported Thunks**: `2`"));
    assert!(md.contains("- **Delay-Loaded Import Slots**: `3` across `2` DLLs."));
    assert!(!md.contains("Delay-load import table is malformed"));
    assert!(
        files["runtime_gap_matrix.md"]
            .contains("The delay-load import tables contain 3 more slots across 2 DLLs.")
    );
}

#[test]
fn printable_spaces_survive_ordinary_and_delayed_import_reports() {
    let (report, files) = export(&import_strings_with_spaces(), "spaces");
    assert_eq!(report.import_error, None);
    assert_eq!(report.delay_import_error, None);
    assert_eq!(
        rows(&report.imports),
        [
            ("fixture ordinary.dll", "Ordinary  Name", 0x2050),
            ("fixture ordinary.dll", "Ordinal#3", 0x2058),
        ]
    );
    assert_eq!(
        rows(&report.delayed_imports),
        [
            ("fixture delay-a.dll", "Delayed Name", 0x3000),
            ("fixture delay-a.dll", "Ordinal#7", 0x3008),
            DELAYED[2],
        ]
    );
    assert!(files["imports.csv"].lines().any(|row| {
        row.starts_with("\"fixture delay-a.dll\",\"Delayed Name\",0x00003000,delayed,")
    }));
    assert_eq!(
        files["missing_imports.csv"]
            .lines()
            .skip(1)
            .collect::<Vec<_>>(),
        [
            "\"fixture ordinary.dll\",\"Ordinary  Name\",0x00002050,name",
            "\"fixture ordinary.dll\",\"Ordinal#3\",0x00002058,ordinal",
        ]
    );
}

#[test]
fn an_import_with_spaces_and_no_service_is_unresolved() {
    let bytes = import_strings_with_spaces();
    let pe = ParsedPe::parse(&bytes).unwrap();
    let mut memory = pe.load_into_memory(&bytes).unwrap().memory;
    let base = pe.image_base;
    let before = memory.read_u64(base + 0x2050).unwrap();
    let error = ImportRegistry::new()
        .bind_imports(&pe, &bytes, base, &mut memory)
        .unwrap_err();
    assert!(
        matches!(
            error,
            ImportError::UnresolvedImport { ref module, ref symbol }
                if module == "fixture ordinary.dll" && symbol == "Ordinary  Name"
        ),
        "{error:?}"
    );
    assert_eq!(memory.read_u64(base + 0x2050).unwrap(), before);
}

#[test]
fn inspection_decodes_the_entry_point_and_follows_no_delay_slot() {
    let report = probe_pe_bytes(FIXTURE, &ProbeConfig::default()).unwrap();

    // The entry point is a lone RET at RVA 0x1000. The three delay-load slots
    // hold the addresses of INT3 stubs at 0x1010, 0x1018 and 0x1020. Reaching
    // one would mean the probe followed a slot, so none may be decoded.
    let decoded: Vec<(u32, &[u8])> = report
        .instruction_records
        .iter()
        .map(|record| (record.rva, record.bytes.as_slice()))
        .collect();
    assert_eq!(decoded, [(0x1000, &[0xc3][..])]);
}

#[test]
fn image_without_a_delay_directory_keeps_its_ordinary_imports() {
    // `fixtures/pe/trusted-runner.py` writes one import, `xenora.dll!fixture_draw`,
    // with its slot at RVA 0x2040 and no delay import descriptor.
    let runner = include_bytes!("../../../fixtures/pe/trusted-runner.exe");
    let (report, files) = export(runner, "runner");
    assert_eq!(
        rows(&report.imports),
        [("xenora.dll", "fixture_draw", 0x2040)]
    );
    assert_eq!(report.import_error, None);
    assert!(report.delayed_imports.is_empty());
    assert_eq!(report.delay_import_error, None);

    let listed: Vec<&str> = files["imports.csv"].lines().skip(1).collect();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].starts_with("\"xenora.dll\",\"fixture_draw\",0x00002040,ordinary,"));
    let json = &files["candidate_static_report.json"];
    assert_eq!(json_field(json, "total_imports"), "1");
    assert_eq!(json_field(json, "imported_dlls_count"), "1");
    assert_eq!(json_field(json, "delayed_imports"), "0");
    assert_eq!(json_field(json, "delayed_imported_dlls_count"), "0");
    assert_eq!(json_field(json, "delay_import_error"), "none");
    assert!(!files["runtime_gap_matrix.md"].contains("delay-load"));
}

#[test]
fn delay_directory_beyond_the_declared_count_is_not_read() {
    // With 13 data directories the header has no delay import descriptor, so
    // the bytes at its offset mean nothing.
    let mut pe = FIXTURE.to_vec();
    put(&mut pe, NUMBER_OF_RVA_AND_SIZES, 13);
    let report = probe_pe_bytes(&pe, &ProbeConfig::default()).unwrap();
    assert_eq!(rows(&report.imports), ORDINARY);
    assert!(report.delayed_imports.is_empty());
    assert_eq!(report.delay_import_error, None);
}

#[test]
fn malformed_delay_tables_are_diagnosed_and_ordinary_imports_kept() {
    type Case = (
        &'static str,
        fn(&mut Vec<u8>),
        usize,
        ImportTableError,
        &'static str,
    );
    // (name, damage, delayed imports still listed, error, message)
    let cases: [Case; 13] = [
        (
            "directory-out-of-range",
            |pe| put(pe, DELAY_DIRECTORY_ENTRY, 0x9000),
            0,
            ImportTableError::OutOfBounds { rva: 0x9000 },
            "import data at RVA 0x00009000 is truncated or outside the section data",
        ),
        (
            // `.rdata` keeps 0x230 bytes: the first descriptor and half the second.
            "section-data-ends-inside-descriptor",
            |pe| put(pe, RDATA_RAW_SIZE, 0x230),
            2,
            ImportTableError::OutOfBounds { rva: 0x2220 },
            "import data at RVA 0x00002220 is truncated or outside the section data",
        ),
        (
            // The file ends where `.data` and its address tables would begin.
            "file-ends-before-address-table",
            |pe| pe.truncate(0xa00),
            0,
            ImportTableError::OutOfBounds { rva: 0x3000 },
            "import data at RVA 0x00003000 is truncated or outside the section data",
        ),
        (
            "name-table-out-of-range",
            |pe| put(pe, FIRST_DESCRIPTOR + 16, 0x9000),
            0,
            ImportTableError::OutOfBounds { rva: 0x9000 },
            "import data at RVA 0x00009000 is truncated or outside the section data",
        ),
        (
            "address-table-out-of-range",
            |pe| put(pe, SECOND_DESCRIPTOR + 12, 0x9000),
            2,
            ImportTableError::OutOfBounds { rva: 0x9000 },
            "import data at RVA 0x00009000 is truncated or outside the section data",
        ),
        (
            // The second DLL's name table becomes the empty entry that ends the
            // first one, at RVA 0x2110, so no import reads the address table.
            "address-table-out-of-range-without-imports",
            |pe| {
                put(pe, SECOND_DESCRIPTOR + 16, 0x2110);
                put(pe, SECOND_DESCRIPTOR + 12, 0x9000);
            },
            2,
            ImportTableError::OutOfBounds { rva: 0x9000 },
            "import data at RVA 0x00009000 is truncated or outside the section data",
        ),
        (
            "dll-name-out-of-range",
            |pe| put(pe, SECOND_DESCRIPTOR + 4, 0x9000),
            2,
            ImportTableError::OutOfBounds { rva: 0x9000 },
            "import data at RVA 0x00009000 is truncated or outside the section data",
        ),
        (
            // RVA 0x2028 is zero padding after the ordinary import directory.
            "dll-name-empty",
            |pe| put(pe, SECOND_DESCRIPTOR + 4, 0x2028),
            2,
            ImportTableError::MalformedName { rva: 0x2028 },
            "malformed import name at RVA 0x00002028",
        ),
        (
            "dll-name-missing",
            |pe| put(pe, SECOND_DESCRIPTOR + 4, 0),
            2,
            ImportTableError::DelayFieldMissing {
                rva: 0x2220,
                field: DelayField::DllName,
            },
            "delay-load descriptor at RVA 0x00002220 has no DLL name",
        ),
        (
            "address-table-missing",
            |pe| put(pe, FIRST_DESCRIPTOR + 12, 0),
            0,
            ImportTableError::DelayFieldMissing {
                rva: 0x2200,
                field: DelayField::AddressTable,
            },
            "delay-load descriptor at RVA 0x00002200 has no import address table",
        ),
        (
            "name-table-missing",
            |pe| put(pe, SECOND_DESCRIPTOR + 16, 0),
            2,
            ImportTableError::DelayFieldMissing {
                rva: 0x2220,
                field: DelayField::NameTable,
            },
            "delay-load descriptor at RVA 0x00002220 has no import name table",
        ),
        (
            "reserved-attribute-bit",
            |pe| put(pe, FIRST_DESCRIPTOR, 3),
            0,
            ImportTableError::DelayReservedBits { rva: 0x2200 },
            "reserved attribute bits set in the delay-load descriptor at RVA 0x00002200",
        ),
        (
            "virtual-address-form",
            |pe| put(pe, SECOND_DESCRIPTOR, 0),
            2,
            ImportTableError::DelayVirtualAddresses { rva: 0x2220 },
            "delay-load descriptor at RVA 0x00002220 holds virtual addresses, not RVAs, and is not read",
        ),
    ];

    for (name, damage, listed, error, message) in cases {
        let mut pe = FIXTURE.to_vec();
        damage(&mut pe);
        let (report, files) = export(&pe, name);

        assert_eq!(rows(&report.imports), ORDINARY, "{name}");
        assert_eq!(report.import_error, None, "{name}");
        assert_eq!(rows(&report.delayed_imports), DELAYED[..listed], "{name}");
        assert_eq!(report.delay_import_error, Some(error), "{name}");
        assert_eq!(error.to_string(), message, "{name}");

        let json = &files["candidate_static_report.json"];
        assert_eq!(json_field(json, "total_imports"), "2", "{name}");
        assert_eq!(
            json_field(json, "delayed_imports"),
            listed.to_string(),
            "{name}"
        );
        assert_eq!(json_field(json, "delay_import_error"), message, "{name}");
        assert!(
            files["candidate_static_report.md"].contains(&format!(
                "- **Delay-load import table is malformed**: {message}. Only the delay-loaded imports before it are listed."
            )),
            "{name}"
        );
    }
}

#[test]
fn declared_directory_size_bounds_the_table() {
    // The fixture declares 96 bytes at RVA 0x2200: two descriptors and the
    // empty one. A descriptor counts only if the declared size holds all of
    // it, and the size must reach the empty descriptor.
    let end = |rva| Some(ImportTableError::DelayDirectoryEnd { rva });
    for (size, listed, error) in [
        (0, 0, end(0x2200)),
        (31, 0, end(0x221f)),
        (32, 2, end(0x2220)),
        (63, 2, end(0x223f)),
        (64, 3, end(0x2240)),
        (95, 3, end(0x225f)),
        (96, 3, None),
        (97, 3, None),
        (0x1000, 3, None),
        // The directory would run past the end of the address space.
        (
            u32::MAX,
            0,
            Some(ImportTableError::OutOfBounds { rva: 0x2200 }),
        ),
    ] {
        let mut pe = FIXTURE.to_vec();
        put(&mut pe, DELAY_DIRECTORY_SIZE, size);
        let report = probe_pe_bytes(&pe, &ProbeConfig::default()).unwrap();
        assert_eq!(rows(&report.imports), ORDINARY, "size {size}");
        assert_eq!(
            rows(&report.delayed_imports),
            DELAYED[..listed],
            "size {size}"
        );
        assert_eq!(report.delay_import_error, error, "size {size}");
    }
    assert_eq!(
        ImportTableError::DelayDirectoryEnd { rva: 0x2240 }.to_string(),
        "delay-load directory ends at RVA 0x00002240 before its empty descriptor"
    );
}

#[test]
fn endless_delay_table_is_refused() {
    // Every descriptor RVA maps onto the same 32 bytes, so the table never
    // reaches its empty entry within the megabyte it declares. The descriptor
    // names module "a" at RVA 0x1000, an empty name table at RVA 0x2000 and an
    // address table at RVA 0x3000.
    let mut bytes = vec![0u8; 48];
    put(&mut bytes, 0, 1);
    put(&mut bytes, 4, 0x1000);
    put(&mut bytes, 12, 0x3000);
    put(&mut bytes, 16, 0x2000);
    bytes[32] = b'a';
    let (imports, error) = parse_delay_imports(&bytes, 0x10_0000, 0x10_0000, |rva| match rva {
        0x1000..0x1002 => Some(32 + (rva - 0x1000) as usize),
        0x2000..0x2008 => Some(40 + (rva - 0x2000) as usize),
        0x3000..0x3008 => Some(40 + (rva - 0x3000) as usize),
        0x10_0000.. => Some((rva % 32) as usize),
        _ => None,
    });
    assert!(imports.is_empty());
    assert_eq!(error, Some(ImportTableError::TooManyDescriptors));
}

#[test]
fn probe_never_panics_on_a_truncated_delay_fixture() {
    for len in 1..=FIXTURE.len() {
        // The probe may reject the file or report unreadable imports, but it
        // must not panic.
        let _ = probe_pe_bytes(&FIXTURE[..len], &ProbeConfig::default());
    }
}
