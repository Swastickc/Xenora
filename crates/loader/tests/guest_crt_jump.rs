// SPDX-License-Identifier: GPL-3.0-or-later

//! `_setjmp` and `longjmp` through real guest programs (#632): a C program
//! built with mingw-w64 jumps back and forth at two optimisation levels. At
//! -O2 the surviving locals live in the callee-saved registers the jump
//! buffer carries, so the exact output is the register preservation check.
//!
//! `tests/compat_fixtures/build-crt-jump.sh` builds
//! `tests/compat_fixtures/crt_jump.c`. The expected output is the literal
//! written in the program next to each check. Nothing here is taken from a
//! run. The test fails, and does not skip, when the toolchain is missing.

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, OnceLock};
use xenora_loader::cpu::ExitReason;
use xenora_loader::imports::ImportRegistry;
use xenora_loader::services::{MockClock, ServicesState, register_standard_services};
use xenora_loader::startup::{StartupLayout, StartupResult, run_synthetic_startup};

const BUDGET: usize = 20_000_000;

const EXPECTED_OUTPUT: &str = concat!(
    "crt jump: value 42\r\n",
    "crt jump: stage 1\r\n",
    "crt jump: zero 1\r\n",
    "crt jump: calm 0\r\n",
    "crt jump: passes 5\r\n",
    "crt jump: failures 0\r\n",
);

fn built() -> &'static PathBuf {
    static OUT: OnceLock<PathBuf> = OnceLock::new();
    OUT.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let build = Command::new("sh")
            .arg(root.join("tests/compat_fixtures/build-crt-jump.sh"))
            .current_dir(&root)
            .output()
            .expect("run tests/compat_fixtures/build-crt-jump.sh");
        assert!(
            build.status.success(),
            "the guest jump programs did not build:\n{}{}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
        root.join("artifacts/crt-jump")
    })
}

fn run(name: &str, arguments: &[&str]) -> (StartupResult, Arc<ServicesState>) {
    let bytes = std::fs::read(built().join(format!("{name}.exe"))).expect("read the guest program");
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    state.set_program_arguments(arguments).unwrap();
    let mut registry = ImportRegistry::new();
    register_standard_services(&mut registry, Arc::clone(&state));
    let layout = StartupLayout {
        image_base: 0x2_5000_0000,
        stack_top: 0x2_0001_0000,
        tls_base: 0x3_0000_0000,
    };
    let result = run_synthetic_startup(&bytes, layout, registry, BUDGET)
        .unwrap_or_else(|error| panic!("{name}: {error}"));
    (result, state)
}

fn output(state: &ServicesState) -> String {
    String::from_utf8(state.standard_output()).expect("ASCII output")
}

#[test]
fn a_c_program_jumps_back_and_forth_at_both_optimisation_levels() {
    for level in ["O0", "O2"] {
        let name = format!("crt-jump-{level}");
        let (result, state) = run(&name, &["crt-jump.exe"]);
        assert_eq!(
            result.execution.exit_reason,
            ExitReason::ProcessExit(0),
            "{name}: output so far:\n{}",
            output(&state)
        );
        assert_eq!(output(&state), EXPECTED_OUTPUT, "{name}: standard output");
        let metrics = &result.dbt_metrics;
        assert!(metrics.blocks_compiled > 0, "{name}: no block compiled");
        assert!(metrics.service_calls > 0, "{name}: no service call");
    }
}
