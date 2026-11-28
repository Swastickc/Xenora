// SPDX-License-Identifier: GPL-3.0-or-later

//! The error reporting services through the guest import table.
//!
//! Message texts come from the public winerror.h documentation, the flag
//! constants from winbase.h. The expected message for error 2 - "The system
//! cannot find the file specified." with its trailing newline - is written
//! here from the documentation, not from the implementation's table.

mod support;

use support::*;

const KERNEL32: &str = "kernel32.dll";

const IMPORTS: &[(&str, &str)] = &[
    (KERNEL32, "FormatMessageA"),
    (KERNEL32, "FormatMessageW"),
    (KERNEL32, "SetErrorMode"),
    (KERNEL32, "GetLastError"),
    (KERNEL32, "SetLastError"),
    (KERNEL32, "LocalFree"),
];

const BUFFER: u64 = SCRATCH + 0x400;
const ARGUMENTS: u64 = SCRATCH + 0x600;
const STRING: u64 = SCRATCH + 0x700;

const ALLOCATE_BUFFER: u64 = 0x100;
const IGNORE_INSERTS: u64 = 0x200;
const FROM_STRING: u64 = 0x400;
const FROM_SYSTEM: u64 = 0x1000;
const ERROR_INSUFFICIENT_BUFFER: u64 = 122;
const ERROR_MR_MID_NOT_FOUND: u64 = 317;
const ERROR_NOT_SUPPORTED: u64 = 50;

fn narrow(guest: &mut Guest, at: u64, text: &str) {
    guest.memory.write_bytes(at, text.as_bytes()).unwrap();
    guest.memory.write_u8(at + text.len() as u64, 0).unwrap();
}

fn narrow_string(guest: &Guest, at: u64) -> String {
    let mut text = String::new();
    let mut i = 0u64;
    loop {
        let byte = guest.memory.read_u8(at + i).unwrap();
        if byte == 0 {
            return text;
        }
        text.push(byte as char);
        i += 1;
    }
}

fn wide_string(guest: &Guest, at: u64) -> String {
    let mut text = String::new();
    let mut i = 0u64;
    loop {
        let unit = guest.memory.read_u16(at + 2 * i).unwrap();
        if unit == 0 {
            return text;
        }
        text.push(char::from_u32(u32::from(unit)).unwrap());
        i += 1;
    }
}

#[test]
fn format_message_answers_the_system_text_in_both_widths() {
    let mut guest = Guest::new(IMPORTS, &[]);
    let expected = "The system cannot find the file specified.\r\n";

    let written = guest.call("FormatMessageA", &[FROM_SYSTEM, 0, 2, 0, BUFFER, 256, 0]);
    assert_eq!(written, expected.len() as u64);
    assert_eq!(narrow_string(&guest, BUFFER), expected);

    let wide = guest.call("FormatMessageW", &[FROM_SYSTEM, 0, 2, 0, BUFFER, 256, 0]);
    assert_eq!(wide, expected.len() as u64);
    assert_eq!(wide_string(&guest, BUFFER), expected);
}

#[test]
fn format_message_failures_are_the_documented_ones() {
    let mut guest = Guest::new(IMPORTS, &[]);

    // An unknown identifier fails with the missing-message error.
    assert_eq!(
        guest.call(
            "FormatMessageA",
            &[FROM_SYSTEM, 0, 999999, 0, BUFFER, 256, 0]
        ),
        0
    );
    assert_eq!(guest.call("GetLastError", &[]), ERROR_MR_MID_NOT_FOUND);

    // A buffer too small fails with the insufficient-buffer error.
    assert_eq!(
        guest.call("FormatMessageA", &[FROM_SYSTEM, 0, 2, 0, BUFFER, 4, 0]),
        0
    );
    assert_eq!(guest.call("GetLastError", &[]), ERROR_INSUFFICIENT_BUFFER);

    // FROM_HMODULE has no module tables yet: the honest not-supported.
    assert_eq!(
        guest.call("FormatMessageA", &[0x800, 0, 2, 0, BUFFER, 256, 0]),
        0
    );
    assert_eq!(guest.call("GetLastError", &[]), ERROR_NOT_SUPPORTED);
}

#[test]
fn format_message_allocate_buffer_frees_with_local_free() {
    let mut guest = Guest::new(IMPORTS, &[]);
    let expected = "The handle is invalid.\r\n";

    // The buffer slot receives the allocated pointer; the text is there.
    guest.memory.write_u64(BUFFER, 0).unwrap();
    let written = guest.call(
        "FormatMessageA",
        &[FROM_SYSTEM | ALLOCATE_BUFFER, 0, 6, 0, BUFFER, 0, 0],
    );
    assert_eq!(written, expected.len() as u64);
    let allocated = guest.memory.read_u64(BUFFER).unwrap();
    assert_ne!(allocated, 0);
    assert_eq!(narrow_string(&guest, allocated), expected);

    // The documentation says to free it with LocalFree - and it works,
    // because both services share the process heap.
    assert_eq!(guest.call("LocalFree", &[allocated]), 0);
}

#[test]
fn format_message_from_string_expands_the_inserts() {
    let mut guest = Guest::new(IMPORTS, &[]);
    narrow(&mut guest, STRING, "alpha.txt");
    narrow(&mut guest, SCRATCH, "cannot open %1!s! code %2!u!");
    guest.memory.write_u64(ARGUMENTS, STRING).unwrap();
    guest.memory.write_u64(ARGUMENTS + 8, 42).unwrap();

    let written = guest.call(
        "FormatMessageA",
        &[FROM_STRING, SCRATCH, 0, 0, BUFFER, 256, ARGUMENTS],
    );
    let expected = "cannot open alpha.txt code 42";
    assert_eq!(written, expected.len() as u64);
    assert_eq!(narrow_string(&guest, BUFFER), expected);

    // IGNORE_INSERTS passes the sequence through unchanged.
    let written = guest.call(
        "FormatMessageA",
        &[FROM_STRING | IGNORE_INSERTS, SCRATCH, 0, 0, BUFFER, 256, 0],
    );
    let expected = "cannot open %1!s! code %2!u!";
    assert_eq!(written, expected.len() as u64);
    assert_eq!(narrow_string(&guest, BUFFER), expected);

    // A wide service reads wide arguments for %1!s!.
    let wide_source = SCRATCH + 0x100;
    let text: Vec<u16> = "got %1!s!".encode_utf16().chain([0]).collect();
    for (i, unit) in text.iter().enumerate() {
        guest
            .memory
            .write_u16(wide_source + 2 * i as u64, *unit)
            .unwrap();
    }
    let wide_arg: Vec<u16> = "wide.txt".encode_utf16().chain([0]).collect();
    for (i, unit) in wide_arg.iter().enumerate() {
        guest
            .memory
            .write_u16(STRING + 2 * i as u64, *unit)
            .unwrap();
    }
    guest.memory.write_u64(ARGUMENTS, STRING).unwrap();
    let written = guest.call(
        "FormatMessageW",
        &[FROM_STRING, wide_source, 0, 0, BUFFER, 256, ARGUMENTS],
    );
    assert_eq!(written, "got wide.txt".len() as u64);
    assert_eq!(wide_string(&guest, BUFFER), "got wide.txt");
}

#[test]
fn the_error_mode_exchanges_and_remembers() {
    let mut guest = Guest::new(IMPORTS, &[]);

    // The first exchange answers the default mode, zero.
    assert_eq!(guest.call("SetErrorMode", &[1]), 0);
    // Each following exchange answers the mode it replaces.
    assert_eq!(guest.call("SetErrorMode", &[0x8000]), 1);
    assert_eq!(guest.call("SetErrorMode", &[3]), 0x8000);
}

fn check_quiet_debug_imports(imports: &'static [(&'static str, &'static str)]) {
    use xenora_loader::MemoryPermissions;

    const ANSI: u64 = SCRATCH + 0x2000;
    const WIDE: u64 = SCRATCH + 0x3000;
    let mut guest = Guest::new(imports, &[]);
    assert_eq!(guest.call("IsDebuggerPresent", &[]), 0);

    // Each empty string ends exactly at its mapping boundary. In particular,
    // the ANSI terminator must not be fetched as a two-byte UTF-16 unit.
    guest
        .memory
        .map_region_with_data(ANSI, 1, MemoryPermissions::READ_ONLY, &[0])
        .unwrap();
    guest
        .memory
        .map_region_with_data(WIDE, 2, MemoryPermissions::READ_ONLY, &[0, 0])
        .unwrap();
    assert!(guest.memory.read_u8(ANSI + 1).is_err());
    assert!(guest.memory.read_u8(WIDE + 2).is_err());
    guest.call("OutputDebugStringA", &[ANSI]);
    guest.call("OutputDebugStringW", &[WIDE]);

    // Nonempty strings must also leave the guest's output streams untouched.
    narrow(&mut guest, STRING, "debug diagnostic");
    guest.call("OutputDebugStringA", &[STRING]);
    guest
        .memory
        .write_bytes(STRING, b"w\0i\0d\0e\0\0\0")
        .unwrap();
    guest.call("OutputDebugStringW", &[STRING]);
    assert!(guest.state.standard_output().is_empty());
    assert!(guest.state.standard_error().is_empty());
}

#[test]
fn canonical_debug_imports_are_quiet_and_honor_character_width() {
    check_quiet_debug_imports(&[
        (KERNEL32, "IsDebuggerPresent"),
        (KERNEL32, "OutputDebugStringA"),
        (KERNEL32, "OutputDebugStringW"),
    ]);
}

#[test]
fn debug_api_set_imports_bind_and_are_quiet() {
    const DEBUG: &str = "api-ms-win-core-debug-l1-1-0.dll";
    check_quiet_debug_imports(&[
        (DEBUG, "IsDebuggerPresent"),
        (DEBUG, "OutputDebugStringA"),
        (DEBUG, "OutputDebugStringW"),
    ]);
}

#[test]
fn legacy_error_handling_debug_imports_remain_available() {
    const ERROR_HANDLING: &str = "api-ms-win-core-errorhandling-l1-1-0.dll";
    check_quiet_debug_imports(&[
        (ERROR_HANDLING, "IsDebuggerPresent"),
        (ERROR_HANDLING, "OutputDebugStringA"),
        (ERROR_HANDLING, "OutputDebugStringW"),
    ]);
}

#[test]
fn compiled_debug_probe_binds_the_exact_api_set_and_exits_quietly() {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;
    use xenora_loader::ExitReason;
    use xenora_loader::import_table::ImportName;
    use xenora_loader::imports::{ImportRegistry, ServiceStatus};
    use xenora_loader::probe::{ProbeConfig, probe_pe_bytes};
    use xenora_loader::services::{MockClock, ServicesState, register_standard_services};
    use xenora_loader::startup::{StartupLayout, run_synthetic_startup};

    const DEBUG: &str = "api-ms-win-core-debug-l1-1-0.dll";
    let output = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("debug-probe-{}", std::process::id()));
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/compat_fixtures/build-debug-probe.sh");
    let build = Command::new("sh")
        .arg(script)
        .arg(&output)
        .output()
        .expect("run the native debug probe builder");
    assert!(
        build.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let bytes = std::fs::read(output.join("debug-probe.exe")).unwrap();

    let report = probe_pe_bytes(&bytes, &ProbeConfig::default()).unwrap();
    assert_eq!(report.import_error, None);
    assert_eq!(
        report
            .imports
            .iter()
            .filter(|entry| entry.dll == DEBUG)
            .count(),
        3
    );
    for symbol in [
        "IsDebuggerPresent",
        "OutputDebugStringA",
        "OutputDebugStringW",
    ] {
        let name = ImportName::Name(symbol.into());
        let mut matches = report
            .imports
            .iter()
            .filter(|entry| entry.import_name == name);
        let entry = matches.next().unwrap_or_else(|| panic!("missing {symbol}"));
        assert_eq!(entry.dll, DEBUG, "{symbol}");
        assert_eq!(
            entry.canonical_module.as_deref(),
            Some(KERNEL32),
            "{symbol}"
        );
        assert_eq!(
            entry.service_status,
            Some(ServiceStatus::Implemented),
            "{symbol}"
        );
        assert!(matches.next().is_none(), "duplicate {symbol}");
    }

    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let mut registry = ImportRegistry::new();
    register_standard_services(&mut registry, Arc::clone(&state));
    let result = run_synthetic_startup(
        &bytes,
        StartupLayout {
            image_base: 0x1_4000_0000,
            stack_top: 0x2_0001_0000,
            tls_base: 0x3_0000_0000,
        },
        registry,
        100_000,
    )
    .expect("start the debug probe guest");
    assert_eq!(result.execution.exit_reason, ExitReason::ProcessExit(0));
    assert!(state.standard_output().is_empty());
    assert!(state.standard_error().is_empty());
}
