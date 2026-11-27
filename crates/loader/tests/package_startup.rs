// SPDX-License-Identifier: GPL-3.0-or-later

//! End-to-end startup of an original multi-DLL package.
//!
//! The three images are built from `fixtures/pe/multidll/*.c` by the native
//! ARM64 LLVM toolchain. The consumer imports a function and a data variable
//! from two package DLLs and `ExitProcess` from kernel32. Its exit status is a
//! checksum of the function results, the data import and both provider
//! `DllMain` markers, so a missing binding or an inverted initialization order
//! changes the observed status instead of passing silently.

use std::sync::Arc;

use xenora_loader::module_graph::PackageModule;
use xenora_loader::module_layout::ModuleLayoutArena;
use xenora_loader::services::{MockClock, ServicesState, register_standard_services};
use xenora_loader::{
    ExitReason, ImportRegistry, PackageStartupError, PackageStartupLayout, run_package_startup,
};

const CONSUMER: &[u8] = include_bytes!("../../../fixtures/pe/multidll/multidll-consumer.exe");
const CHAIN: &[u8] = include_bytes!("../../../fixtures/pe/multidll/xenora_chain.dll");
const MATH: &[u8] = include_bytes!("../../../fixtures/pe/multidll/xenora_math.dll");

/// 42 from `XenoraAdd`, 8 from `XenoraQuad` through the chain, 10 from the
/// `XenoraScale` data import, 1 from the leaf `DllMain`, 2 from the middle
/// `DllMain`.
const EXPECTED_EXIT: u32 = 63;

fn layout() -> PackageStartupLayout {
    PackageStartupLayout {
        arena: ModuleLayoutArena {
            base: 0x1_0000_0000,
            size: 0x1000_0000,
        },
        stack_top: 0x2_0001_0000,
        thread_tls_base: 0x3_0000_0000,
    }
}

fn services() -> Arc<ServicesState> {
    Arc::new(ServicesState::new(Arc::new(MockClock::new(0))))
}

fn registry() -> ImportRegistry {
    let mut registry = ImportRegistry::new();
    register_standard_services(&mut registry, services());
    registry
}

#[test]
fn original_multi_dll_package_runs_its_entry_point() {
    // The consumer is first on purpose: dependency order must come from the
    // import graph, not from the caller's list order.
    let package = [
        PackageModule {
            file_name: "multidll-consumer.exe",
            bytes: CONSUMER,
        },
        PackageModule {
            file_name: "xenora_chain.dll",
            bytes: CHAIN,
        },
        PackageModule {
            file_name: "xenora_math.dll",
            bytes: MATH,
        },
    ];
    let result = run_package_startup(&package, layout(), 0, registry(), 2_000_000, None)
        .expect("the explicit package starts");

    assert_eq!(result.diagnostics.unresolved_imports().count(), 0);
    assert_eq!(
        result.execution.exit_reason,
        ExitReason::ProcessExit(EXPECTED_EXIT)
    );
    assert_eq!(
        result
            .diagnostics
            .failure_counts()
            .into_iter()
            .filter(|(_, count)| *count != 0)
            .count(),
        0
    );

    // Both provider DllMains ran, the leaf before the middle one, and every
    // import was bound to a real package export or a registered service.
    let dll_mains: Vec<&str> = result
        .diagnostics
        .initialization
        .iter()
        .filter(|record| record.phase == xenora_loader::InitializationPhase::DllMain)
        .map(|record| record.file_name.as_str())
        .collect();
    assert_eq!(dll_mains, ["xenora_math.dll", "xenora_chain.dll"]);

    let math = result
        .diagnostics
        .modules
        .iter()
        .find(|module| module.file_name == "xenora_math.dll")
        .expect("math module report");
    assert_eq!(math.imports, 0);
    let chain = result
        .diagnostics
        .modules
        .iter()
        .find(|module| module.file_name == "xenora_chain.dll")
        .expect("chain module report");
    assert_eq!(chain.imports, 2);
    assert_eq!(chain.resolved_imports, 2);
    let consumer = result
        .diagnostics
        .modules
        .iter()
        .find(|module| module.file_name == "multidll-consumer.exe")
        .expect("consumer module report");
    assert_eq!(consumer.imports, 6);
    assert_eq!(consumer.resolved_imports, 6);
    assert_eq!(consumer.dependencies.len(), 2);

    // Package exports are bound to provider addresses, not to host services.
    let package_imports = result
        .diagnostics
        .imports
        .iter()
        .filter(|import| {
            matches!(
                import.disposition,
                xenora_loader::ImportDisposition::Package { .. }
            )
        })
        .count();
    // Seven package imports across both consumers; the eighth import is the
    // kernel32 service.
    assert_eq!(package_imports, 7);
}

#[test]
fn missing_provider_fails_closed_with_structured_diagnostics() {
    let package = [
        PackageModule {
            file_name: "multidll-consumer.exe",
            bytes: CONSUMER,
        },
        PackageModule {
            file_name: "xenora_math.dll",
            bytes: MATH,
        },
    ];
    let error = run_package_startup(&package, layout(), 0, registry(), 2_000_000, None)
        .expect_err("a package without its middle provider must not start");
    let diagnostics = error
        .diagnostics()
        .unwrap_or_else(|| panic!("unresolved imports keep structured evidence: {error:?}"));
    let unresolved: Vec<_> = diagnostics.unresolved_imports().collect();
    assert!(!unresolved.is_empty());
    assert!(
        unresolved
            .iter()
            .any(|import| import.requested_module == "xenora_chain.dll")
    );
    let counts = diagnostics.failure_counts();
    assert!(
        counts
            .iter()
            .any(|(name, count)| *name == "missing-module" && *count != 0)
    );
    assert!(matches!(error, PackageStartupError::UnresolvedImports(_)));
}
