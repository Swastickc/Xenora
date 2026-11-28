// SPDX-License-Identifier: GPL-3.0-or-later

//! Run original PE images that only fit when the DLL keeps its fixed base.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use xenora_loader::module_graph::PackageModule;
use xenora_loader::module_layout::{ModuleImageDescriptor, ModuleLayoutArena};
use xenora_loader::{
    ExitReason, ImportRegistry, MockClock, PackageStartupLayout, ParsedPe, ServicesState,
    register_standard_services, run_package_startup,
};

const FIXED_BASE: u64 = 0x1_5000_0000;
const SLOT: u64 = 0x1_0000;

#[test]
fn a_movable_root_rebases_around_an_original_fixed_dll() {
    let output = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("fixed-package-{}", std::process::id()));
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/compat_fixtures/build-module-fixed-package.sh");
    let build = Command::new("sh")
        .arg(script)
        .arg(&output)
        .output()
        .unwrap();
    assert!(build.status.success(), "{build:?}");

    let consumer = std::fs::read(output.join("movable-root.exe")).unwrap();
    let provider = std::fs::read(output.join("fixed-provider.dll")).unwrap();
    for (bytes, stripped) in [(&consumer, false), (&provider, true)] {
        let pe = ParsedPe::parse(bytes).unwrap();
        let image = ModuleImageDescriptor::from_parsed_pe(&pe);
        assert_eq!(image.preferred_base, FIXED_BASE);
        assert_eq!(image.relocations_stripped, stripped);
        assert!((1..=SLOT).contains(&u64::from(image.size_of_image)));
    }
    let root = PackageModule {
        file_name: "movable-root.exe",
        bytes: &consumer,
    };
    let dll = PackageModule {
        file_name: "fixed-provider.dll",
        bytes: &provider,
    };
    for (package, entry) in [([root, dll], 0), ([dll, root], 1)] {
        let layout = PackageStartupLayout {
            arena: ModuleLayoutArena {
                base: FIXED_BASE,
                size: 2 * SLOT,
            },
            stack_top: 0x2_0001_0000,
            thread_tls_base: 0x3_0000_0000,
        };
        let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
        let mut services = ImportRegistry::new();
        register_standard_services(&mut services, Arc::clone(&state));
        let result = run_package_startup(&package, layout, entry, services, 100_000, None)
            .expect("both PE images fit in their own slots");
        assert_eq!(result.execution.exit_reason, ExitReason::ProcessExit(42));
        assert_eq!(state.exit_status(), Some(42));
        assert_eq!(result.diagnostics.unresolved_imports().count(), 0);
        assert_eq!(result.reservations[entry].image_index, entry);
        assert_eq!(result.reservations[entry].base, FIXED_BASE + SLOT);
        assert_eq!(result.reservations[1 - entry].image_index, 1 - entry);
        assert_eq!(result.reservations[1 - entry].base, FIXED_BASE);
        assert!(result.relocations[entry].applied_dir64 > 0);
    }
}
