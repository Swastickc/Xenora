// SPDX-License-Identifier: GPL-3.0-or-later

//! Wide printf sizing and precision at Xenora's formatted-output limit.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use xenora_loader::services::{MockClock, ServicesState, register_standard_services};
use xenora_loader::startup::{StartupLayout, run_synthetic_startup};
use xenora_loader::{ExitReason, ImportRegistry};

fn fixture(name: &str) -> Vec<u8> {
    static OUTPUT: OnceLock<PathBuf> = OnceLock::new();
    let output = OUTPUT.get_or_init(|| {
        let output = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("crt-wformat-budget-{}", std::process::id()));
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/compat_fixtures/build-crt-wformat-budget.sh");
        let build = Command::new("sh")
            .arg(script)
            .arg(&output)
            .output()
            .expect("build the original wide printf guests");
        assert!(
            build.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
        output
    });
    std::fs::read(output.join(format!("crt-wformat-budget-{name}.exe"))).unwrap()
}

fn run(name: &str, expected: ExitReason, status: Option<u32>) {
    let bytes = fixture(name);
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let mut imports = ImportRegistry::new();
    register_standard_services(&mut imports, Arc::clone(&state));
    let result = run_synthetic_startup(
        &bytes,
        StartupLayout {
            image_base: 0x1_4000_0000,
            stack_top: 0x2_0001_0000,
            tls_base: 0x3_0000_0000,
        },
        imports,
        100_000,
    )
    .expect("start the wide printf guest");
    assert_eq!(result.execution.exit_reason, expected, "{name}");
    assert_eq!(state.exit_status(), status, "{name}");
}

#[test]
fn wide_printf_accepts_exact_cap_and_explicit_precision() {
    run("controls", ExitReason::ProcessExit(0), Some(0));
}

#[test]
fn wide_printf_rejects_an_oversized_wide_string() {
    run("wide", ExitReason::IllegalInstruction(0xFE), None);
}

#[test]
fn wide_printf_rejects_an_oversized_narrow_string() {
    run("narrow", ExitReason::IllegalInstruction(0xFE), None);
}
