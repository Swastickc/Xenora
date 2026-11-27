// SPDX-License-Identifier: GPL-3.0-or-later

//! A compiled PE exercises isolation between live and recycled guest threads.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use xenora_loader::probe::{ProbeConfig, probe_pe_bytes};
use xenora_loader::startup::{StartupLayout, run_synthetic_startup_scheduled};
use xenora_loader::{
    ExitReason, ImportRegistry, MockClock, ServicesState, register_standard_services,
};

#[test]
fn scheduled_msvcrt_rand_is_per_thread_and_drops_the_seed_on_exit() {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("crt-thread-rand-{}", std::process::id()));
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/compat_fixtures/build-crt-thread-rand.sh");
    let build = Command::new("sh")
        .arg(script)
        .arg(&directory)
        .output()
        .expect("build the original rand guest");
    assert!(build.status.success(), "{build:?}");

    let bytes = std::fs::read(directory.join("crt-thread-rand.exe")).unwrap();
    let inspected = probe_pe_bytes(&bytes, &ProbeConfig::default()).expect("inspect the guest PE");
    assert_eq!(inspected.import_error, None);
    for symbol in ["rand", "srand"] {
        assert!(
            inspected
                .imports
                .iter()
                .any(|entry| entry.dll.eq_ignore_ascii_case("msvcrt.dll") && entry.symbol == symbol),
            "guest must import msvcrt.dll!{symbol}"
        );
    }

    for quantum in [1, 64, 1024] {
        let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
        let mut registry = ImportRegistry::new();
        register_standard_services(&mut registry, Arc::clone(&state));
        let result = run_synthetic_startup_scheduled(
            &bytes,
            StartupLayout {
                image_base: 0x1_4000_0000,
                stack_top: 0x2_0001_0000,
                tls_base: 0x3_0000_0000,
            },
            registry,
            Arc::clone(&state),
            250_000,
            quantum,
            |_| {},
        )
        .expect("start the original rand guest");

        assert_eq!(
            result.execution.exit_reason,
            ExitReason::ProcessExit(0),
            "quantum {quantum}",
        );
        assert_eq!(state.exit_status(), Some(0));
    }
}
