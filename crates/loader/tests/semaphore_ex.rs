// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(target_os = "macos")]
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use xenora_loader::cpu::ExitReason;
use xenora_loader::import_table::ImportName;
use xenora_loader::imports::{ImportRegistry, ServiceStatus};
use xenora_loader::services::{MockClock, ServicesState, register_standard_services};
use xenora_loader::startup::{StartupLayout, run_synthetic_startup};

fn guest_compiler() -> PathBuf {
    let gcc = std::env::var_os("MINGW_GCC")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let homebrew = PathBuf::from("/opt/homebrew/bin/x86_64-w64-mingw32-gcc");
            if homebrew.is_file() {
                homebrew
            } else {
                PathBuf::from("x86_64-w64-mingw32-gcc")
            }
        });
    #[cfg(target_os = "macos")]
    {
        let native_gcc = if gcc.components().count() == 1 {
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .map(|dir| dir.join(&gcc))
                .find(|path| path.is_file())
                .expect("find a native mingw-w64 compiler (MINGW_GCC)")
        } else {
            gcc
        };
        require_arm64(&native_gcc);
        for program in ["cc1", "as", "ld", "collect2"] {
            let output = Command::new(&native_gcc)
                .arg(format!("-print-prog-name={program}"))
                .output()
                .unwrap();
            assert!(output.status.success(), "locate mingw-w64 {program}");
            require_arm64(Path::new(String::from_utf8(output.stdout).unwrap().trim()));
        }
        native_gcc
    }
    #[cfg(not(target_os = "macos"))]
    gcc
}

#[cfg(target_os = "macos")]
fn require_arm64(tool: &Path) {
    let archs = Command::new("lipo")
        .arg("-archs")
        .arg(tool)
        .output()
        .unwrap();
    assert!(
        archs.status.success()
            && String::from_utf8_lossy(&archs.stdout)
                .split_whitespace()
                .any(|arch| arch == "arm64"),
        "ARM64 host tool required: {}",
        tool.display()
    );
}

#[test]
fn documented_api_sets_share_the_canonical_partial_services() {
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let mut registry = ImportRegistry::new();
    register_standard_services(&mut registry, state);
    for (module, symbol) in [
        (
            "api-ms-win-core-synch-ansi-l1-1-0.dll",
            "CreateSemaphoreExA",
        ),
        ("api-ms-win-core-synch-l1-1-0.dll", "CreateSemaphoreExW"),
        ("api-ms-win-core-synch-l1-2-0.dll", "CreateSemaphoreExW"),
        ("api-ms-win-core-synch-l1-2-1.dll", "CreateSemaphoreExW"),
    ] {
        let name = ImportName::Name(symbol.into());
        let canonical = registry.resolve("kernel32.dll", &name).unwrap();
        assert_eq!(canonical.status, ServiceStatus::Partial);
        let alias = registry.resolve(module, &name).unwrap();
        assert!(std::ptr::eq(alias, canonical), "{module}!{symbol}");
    }
}

#[test]
fn original_guest_checks_extended_semaphore_counts_and_rights() {
    let source = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/compat_fixtures/semaphore_ex.c"
    );
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("semaphore-ex-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let gcc = guest_compiler();

    for level in ["-O0", "-O1", "-O2", "-Os"] {
        let executable = out.join(format!("semaphore-ex{level}.exe"));
        let build = Command::new(&gcc)
            .args([
                level,
                "-Wall",
                "-Wextra",
                "-Werror",
                "-ffreestanding",
                "-fno-stack-protector",
                "-nostdlib",
                "-nostartfiles",
                "-Wl,-e,start",
                source,
                "-lkernel32",
                "-o",
            ])
            .arg(&executable)
            .output()
            .expect("run a native mingw-w64 compiler (MINGW_GCC)");
        assert!(
            build.status.success(),
            "{level} fixture build: {}",
            String::from_utf8_lossy(&build.stderr)
        );

        let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
        let mut registry = ImportRegistry::new();
        register_standard_services(&mut registry, state);
        let layout = StartupLayout {
            image_base: 0x2_5000_0000,
            stack_top: 0x2_0001_0000,
            tls_base: 0x3_0000_0000,
        };
        let program = std::fs::read(executable).unwrap();
        let result = run_synthetic_startup(&program, layout, registry, 100_000)
            .unwrap_or_else(|error| panic!("{level}: {error}"));
        assert_eq!(
            result.execution.exit_reason,
            ExitReason::ProcessExit(0),
            "{level}: a nonzero exit code identifies the failed C assertion's source line"
        );
    }
}
