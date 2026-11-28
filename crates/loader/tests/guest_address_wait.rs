// SPDX-License-Identifier: GPL-3.0-or-later

//! Original C guests exercising address waits through the scheduler and API set.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use xenora_loader::cpu::ExitReason;
use xenora_loader::imports::ImportRegistry;
use xenora_loader::memory::MemoryError;
use xenora_loader::probe::{ProbeConfig, probe_pe_bytes};
use xenora_loader::services::{Clock, MockClock, ServicesState, register_standard_services};
use xenora_loader::startup::{
    StartupLayout, run_synthetic_startup, run_synthetic_startup_scheduled,
};

const API_SET: &str = "api-ms-win-core-synch-l1-2-0.dll";
const LAYOUT: StartupLayout = StartupLayout {
    image_base: 0x1_4000_0000,
    stack_top: 0x2_0001_0000,
    tls_base: 0x3_0000_0000,
};

fn fixture(name: &str) -> Vec<u8> {
    static OUTPUT: OnceLock<PathBuf> = OnceLock::new();
    let output = OUTPUT.get_or_init(|| {
        let output = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("address-wait-{}", std::process::id()));
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/compat_fixtures/build-address-wait.sh");
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

fn run(bytes: &[u8], quantum: usize) -> (ExitReason, u64, usize) {
    let clock = Arc::new(MockClock::new(0));
    let state = Arc::new(ServicesState::new(clock.clone()));
    let mut imports = ImportRegistry::new();
    register_standard_services(&mut imports, Arc::clone(&state));
    let result = run_synthetic_startup_scheduled(
        bytes,
        LAYOUT,
        imports,
        Arc::clone(&state),
        2_000_000,
        quantum,
        |_| {},
    )
    .expect("start the address-wait guest");
    (
        result.execution.exit_reason,
        clock.performance_count(),
        state.open_handle_count(),
    )
}

#[test]
fn compiled_guest_waits_and_wakes_under_the_scheduler() {
    let bytes = fixture("address-wait.exe");
    let report = probe_pe_bytes(&bytes, &ProbeConfig::default()).unwrap();
    assert_eq!(report.import_error, None);
    for name in ["WaitOnAddress", "WakeByAddressSingle", "WakeByAddressAll"] {
        let modules: Vec<_> = report
            .imports
            .iter()
            .filter(|entry| entry.symbol == name)
            .map(|entry| entry.dll.as_str())
            .collect();
        assert_eq!(modules, [API_SET], "{name}");
    }
    for quantum in [1, 64, 1024] {
        // One 7 ms timeout and six 1 ms intervals to park the workers.
        assert_eq!(
            run(&bytes, quantum),
            (ExitReason::ProcessExit(0), 130_000, 0),
            "quantum {quantum}"
        );
    }
}

#[test]
fn invalid_comparison_pointer_faults_without_parking_the_thread() {
    let bytes = fixture("address-wait-bad-compare.exe");
    assert_eq!(
        run(&bytes, 64),
        (
            ExitReason::MemoryFault(MemoryError::UnmappedAddress(0)),
            0,
            0,
        )
    );
}

#[test]
fn a_blocking_wait_without_a_scheduler_is_refused() {
    let bytes = fixture("address-wait-unscheduled.exe");
    let clock = Arc::new(MockClock::new(0));
    let state = Arc::new(ServicesState::new(clock.clone()));
    let mut imports = ImportRegistry::new();
    register_standard_services(&mut imports, state);
    let result = run_synthetic_startup(&bytes, LAYOUT, imports, 2_000_000).unwrap();
    assert_eq!(result.execution.exit_reason, ExitReason::ProcessExit(0));
    assert_eq!(clock.performance_count(), 0);
}
