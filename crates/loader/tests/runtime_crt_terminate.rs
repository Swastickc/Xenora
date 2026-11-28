// SPDX-License-Identifier: GPL-3.0-or-later

//! The supported default action of UCRT terminate, exercised by compiled PE.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use xenora_loader::cpu::ExitReason;
use xenora_loader::import_table::ImportName;
use xenora_loader::imports::{ImportRegistry, ServiceStatus};
use xenora_loader::probe::{ProbeConfig, probe_pe_bytes};
use xenora_loader::services::{MockClock, ServicesState, register_standard_services};
use xenora_loader::startup::{StartupLayout, run_synthetic_startup};

const UCRT: &str = "ucrtbase.dll";
const API_SET: &str = "api-ms-win-crt-runtime-l1-1-0.dll";
const JOURNAL: &str = r"C:\terminate.log";

fn fixture(name: &str) -> Vec<u8> {
    static OUTPUT: OnceLock<PathBuf> = OnceLock::new();
    let output = OUTPUT.get_or_init(|| {
        let output = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("crt-terminate-{}", std::process::id()));
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/compat_fixtures/build-crt-terminate.sh");
        let build = Command::new("sh")
            .arg(script)
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
        output
    });
    std::fs::read(output.join(name)).unwrap()
}

fn run(bytes: &[u8], status: u32, journal: &[u8]) {
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let mut imports = ImportRegistry::new();
    register_standard_services(&mut imports, Arc::clone(&state));
    let result = run_synthetic_startup(
        bytes,
        StartupLayout {
            image_base: 0x1_4000_0000,
            stack_top: 0x2_0001_0000,
            tls_base: 0x3_0000_0000,
        },
        imports,
        100_000,
    )
    .expect("start the terminate guest");
    assert_eq!(
        result.execution.exit_reason,
        ExitReason::ProcessExit(status)
    );
    assert_eq!(state.exit_status(), Some(status));
    assert_eq!(state.file_contents(JOURNAL).as_deref(), Some(journal));
    assert_eq!(state.open_handle_count(), 0);
}

#[test]
fn terminate_skips_the_callback_and_caller_continuation() {
    for (module, file) in [
        (UCRT, "crt-terminate-ucrt.exe"),
        (API_SET, "crt-terminate-apiset.exe"),
    ] {
        let bytes = fixture(file);
        let report = probe_pe_bytes(&bytes, &ProbeConfig::default()).unwrap();
        assert_eq!(report.import_error, None);
        let name = ImportName::Name("terminate".into());
        let mut matches = report
            .imports
            .iter()
            .filter(|entry| entry.import_name == name);
        let entry = matches.next().expect("an exact terminate import");
        assert_eq!(entry.dll, module);
        assert_eq!(entry.service_status, Some(ServiceStatus::Partial));
        assert_eq!(entry.canonical_module.as_deref(), Some(UCRT));
        assert!(matches.next().is_none());

        // A is written before termination, B would be written by atexit, and
        // C would be written after an incorrect return to the caller.
        run(&bytes, 3, b"A");
    }
}

#[test]
fn the_same_callback_runs_on_normal_crt_exit() {
    run(&fixture("crt-terminate-control.exe"), 0, b"AB");
}
