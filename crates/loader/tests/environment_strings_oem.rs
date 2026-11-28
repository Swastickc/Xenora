// SPDX-License-Identifier: GPL-3.0-or-later

//! Original compiler-linked PE guests exercise the narrow environment block.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use xenora_loader::probe::{ProbeConfig, probe_pe_bytes};
use xenora_loader::startup::{StartupLayout, run_synthetic_startup};
use xenora_loader::{
    ExitReason, ImportRegistry, MockClock, ServicesState, register_standard_services,
};

const KERNEL32: &str = "kernel32.dll";
const API_SET: &str = "api-ms-win-core-processenvironment-l1-1-0.dll";

#[test]
fn compiler_linked_guests_use_oem_environment_blocks_and_preserve_wide_blocks() {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("environment-strings-oem-{}", std::process::id()));
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/compat_fixtures/build-environment-strings-oem.sh");
    let build = Command::new("sh")
        .arg(script)
        .arg(&directory)
        .output()
        .expect("build the original environment block guests");
    assert!(build.status.success(), "{build:?}");

    for (name, narrow_module, has_suffixed_get) in
        [("kernel32", KERNEL32, true), ("apiset", API_SET, false)]
    {
        let bytes = std::fs::read(directory.join(format!("environment-strings-{name}.exe")))
            .expect("read the original guest PE");
        let inspected =
            probe_pe_bytes(&bytes, &ProbeConfig::default()).expect("inspect fixture PE");
        assert_eq!(inspected.import_error, None, "{name}");
        for (module, symbol, expected) in [
            (narrow_module, "GetEnvironmentStrings", true),
            (narrow_module, "GetEnvironmentStringsA", has_suffixed_get),
            (narrow_module, "FreeEnvironmentStringsA", true),
            (KERNEL32, "GetEnvironmentStringsW", true),
            (KERNEL32, "FreeEnvironmentStringsW", true),
        ] {
            assert_eq!(
                inspected.imports.iter().any(|entry| {
                    entry.dll.eq_ignore_ascii_case(module) && entry.symbol == symbol
                }),
                expected,
                "{name}: {module}!{symbol}",
            );
        }

        let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
        let mut registry = ImportRegistry::new();
        register_standard_services(&mut registry, Arc::clone(&state));
        let result = run_synthetic_startup(
            &bytes,
            StartupLayout {
                image_base: 0x1_5000_0000,
                stack_top: 0x2_0001_0000,
                tls_base: 0x3_0000_0000,
            },
            registry,
            100_000,
        )
        .unwrap_or_else(|error| panic!("start the {name} environment block guest: {error}"));
        assert_eq!(
            result.execution.exit_reason,
            ExitReason::ProcessExit(0),
            "{name}"
        );
        assert!(result.dbt_metrics.dbt_dispatches > 0, "{name}");
        assert_eq!(state.exit_status(), Some(0), "{name}");
        assert_eq!(
            state.environment_variable_value("ALPHA").as_deref(),
            Some("new")
        );
        assert_eq!(
            state.environment_variable_value("BETA").as_deref(),
            Some("")
        );
        assert_eq!(
            state.environment_variable_value("ZULU").as_deref(),
            Some("中")
        );
    }
}
