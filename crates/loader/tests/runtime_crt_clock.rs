// SPDX-License-Identifier: GPL-3.0-or-later

//! The documented per-thread UCRT time buffer, exercised by an original AMD64 PE.
//! https://learn.microsoft.com/cpp/c-runtime-library/reference/gmtime-gmtime32-gmtime64

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use xenora_loader::services::{MockClock, ServicesState, register_standard_services};
use xenora_loader::startup::{StartupLayout, run_synthetic_startup_scheduled};
use xenora_loader::{ExitReason, ImportRegistry};

#[test]
fn scheduled_threads_keep_separate_time_conversion_results() {
    let output = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("crt-clock-threads-{}", std::process::id()));
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/compat_fixtures/build-crt-clock-threads.sh");
    let build = Command::new("sh")
        .arg(script)
        .arg(&output)
        .output()
        .expect("build the original CRT time guest");
    assert!(
        build.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let bytes = std::fs::read(output.join("crt-clock-threads.exe")).unwrap();

    let services = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let mut imports = ImportRegistry::new();
    register_standard_services(&mut imports, Arc::clone(&services));
    let result = run_synthetic_startup_scheduled(
        &bytes,
        StartupLayout {
            image_base: 0x1_4000_0000,
            stack_top: 0x2_0001_0000,
            tls_base: 0x3_0000_0000,
        },
        imports,
        Arc::clone(&services),
        500_000,
        64,
        |_| {},
    )
    .expect("run the scheduled CRT time guest");
    assert_eq!(result.execution.exit_reason, ExitReason::ProcessExit(0));
    assert_eq!(services.exit_status(), Some(0));
    assert_eq!(services.open_handle_count(), 0);
}
