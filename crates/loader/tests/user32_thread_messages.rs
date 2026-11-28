// SPDX-License-Identifier: GPL-3.0-or-later

//! An original compiled PE exercises message polling on two guest threads.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use xenora_loader::probe::{ProbeConfig, probe_pe_bytes};
use xenora_loader::startup::{StartupLayout, run_synthetic_startup_scheduled};
use xenora_loader::{
    ExitReason, ImportRegistry, MockClock, ServicesState, register_standard_services,
};

#[test]
fn scheduled_guest_posts_and_polls_isolated_thread_messages() {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("user32-thread-messages-{}", std::process::id()));
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/compat_fixtures/build-user32-thread-messages.sh");
    let build = Command::new("sh")
        .arg(script)
        .arg(&directory)
        .output()
        .expect("build the original thread-message guest");
    assert!(build.status.success(), "{build:?}");

    let bytes = std::fs::read(directory.join("user32-thread-messages.exe")).unwrap();
    let inspected = probe_pe_bytes(&bytes, &ProbeConfig::default()).expect("inspect fixture PE");
    assert_eq!(inspected.import_error, None);
    for symbol in [
        "PeekMessageA",
        "PeekMessageW",
        "PostThreadMessageA",
        "PostThreadMessageW",
    ] {
        assert!(
            inspected.imports.iter().any(|entry| {
                entry.dll.eq_ignore_ascii_case("user32.dll") && entry.symbol == symbol
            }),
            "missing user32.dll!{symbol}",
        );
    }
    for quantum in [1, 64, 1024] {
        verify_guest(&bytes, quantum);
    }
}

fn verify_guest(bytes: &[u8], quantum: usize) {
    let clock = Arc::new(MockClock::new(116_444_736_000_000_000));
    let state = Arc::new(ServicesState::new(clock));
    let mut registry = ImportRegistry::new();
    register_standard_services(&mut registry, Arc::clone(&state));

    let cleanup_calls = Arc::new(AtomicUsize::new(0));
    let observed_cleanup = Arc::clone(&cleanup_calls);
    let state_at_cleanup = Arc::clone(&state);
    registry.register("xenora_test.dll", "OpenHandleCount", move |_, _, _| {
        observed_cleanup.fetch_add(1, Ordering::Relaxed);
        Ok(state_at_cleanup.open_handle_count() as u64)
    });

    let result = run_synthetic_startup_scheduled(
        bytes,
        StartupLayout {
            image_base: 0x1_5000_0000,
            stack_top: 0x2_0001_0000,
            tls_base: 0x3_0000_0000,
        },
        registry,
        Arc::clone(&state),
        250_000,
        quantum,
        |_| {},
    )
    .expect("start the original thread-message guest");
    assert_eq!(
        result.execution.exit_reason,
        ExitReason::ProcessExit(0),
        "quantum {quantum}",
    );
    assert!(result.dbt_metrics.dbt_dispatches > 0);
    assert_eq!(state.exit_status(), Some(0));
    assert_eq!(cleanup_calls.load(Ordering::Relaxed), 1);
}
