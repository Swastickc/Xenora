// SPDX-License-Identifier: GPL-3.0-or-later

//! Issue #936: the public Itanium C++ ABI array allocation operators run end
//! to end in an original C++ program through the loader and ARM64 dynamic
//! binary translation.
//!
//! `tests/compat_fixtures/build-cxx-array.sh` builds
//! `tests/compat_fixtures/cxx_array.cpp` at four optimization levels with no C
//! or C++ runtime linked. Runtime-sized `new[]` expressions make the compiler
//! emit `_Znay` and `_ZdaPv` rather than calling a Xenora-authored shim.
//! Expected values below are literals derived from the program source and the
//! public operator contract, never from a run.
//!
//! The build script and the fixture are authored for Xenora. This test fails,
//! and does not skip, when the native ARM64 build tools are missing.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use xenora_loader::cpu::ExitReason;
use xenora_loader::imports::ImportRegistry;
use xenora_loader::services::{MockClock, ServicesState, register_standard_services};
use xenora_loader::startup::{StartupLayout, StartupResult, run_synthetic_startup};

/// The load address differs from the preferred base, so relocations matter.
const IMAGE_BASE: u64 = 0x2_6000_0000;
const STACK_TOP: u64 = 0x2_0001_0000;
const TLS_BASE: u64 = 0x3_0000_0000;
const BUDGET: usize = 5_000_000;

const MAGIC: u32 = 0x6378_616e;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository root")
}

fn built() -> &'static Path {
    static OUT: OnceLock<PathBuf> = OnceLock::new();
    OUT.get_or_init(|| {
        let root = repo_root();
        let script = root.join("tests/compat_fixtures/build-cxx-array.sh");
        let build = Command::new("sh")
            .arg(&script)
            .current_dir(&root)
            .output()
            .expect("run tests/compat_fixtures/build-cxx-array.sh");
        assert!(
            build.status.success(),
            "the guest C++ array program did not build:\n{}{}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
        let primary = root.join("artifacts/guest-cxx-array");
        if primary.join("guest-cxx-array-O0.exe").is_file() {
            return primary;
        }
        let artifacts = root.join("artifacts");
        if let Ok(entries) = std::fs::read_dir(&artifacts) {
            for entry in entries.flatten() {
                let candidate = entry.path();
                if candidate.join("guest-cxx-array-O0.exe").is_file() {
                    return candidate;
                }
            }
        }
        panic!(
            "the build script did not produce guest-cxx-array-O0.exe in {}",
            artifacts.display()
        );
    })
}

struct Outcome {
    result: StartupResult,
}

fn run(name: &str) -> Result<Outcome, String> {
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let bytes =
        std::fs::read(built().join(format!("{name}.exe"))).expect("read the built guest program");
    let mut registry = ImportRegistry::new();
    register_standard_services(&mut registry, Arc::clone(&state));
    let layout = StartupLayout {
        image_base: IMAGE_BASE,
        stack_top: STACK_TOP,
        tls_base: TLS_BASE,
    };
    match run_synthetic_startup(&bytes, layout, registry, BUDGET) {
        Ok(result) => Ok(Outcome { result }),
        Err(error) => Err(error.to_string()),
    }
}

/// What the program reports in its `.gate` section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ResultBlock {
    magic: u32,
    first_value: u32,
    second_value: u32,
    distinct: u32,
    zero_nonzero: u32,
    zero_distinct: u32,
    first: u64,
    second: u64,
    zero_first: u64,
    zero_second: u64,
}

fn result_block(outcome: &Outcome) -> ResultBlock {
    let image = &outcome.result.image;
    let section = image
        .pe
        .sections
        .iter()
        .find(|section| section.name == ".gate")
        .expect("a .gate section");
    let at = IMAGE_BASE + u64::from(section.virtual_address);
    let memory = &image.memory;
    ResultBlock {
        magic: memory.read_u32(at).expect("result block"),
        first_value: memory.read_u32(at + 4).expect("result block"),
        second_value: memory.read_u32(at + 8).expect("result block"),
        distinct: memory.read_u32(at + 12).expect("result block"),
        zero_nonzero: memory.read_u32(at + 16).expect("result block"),
        zero_distinct: memory.read_u32(at + 20).expect("result block"),
        // Seven u32 fields are followed by four bytes of alignment padding.
        first: memory.read_u64(at + 32).expect("result block"),
        second: memory.read_u64(at + 40).expect("result block"),
        zero_first: memory.read_u64(at + 48).expect("result block"),
        zero_second: memory.read_u64(at + 56).expect("result block"),
    }
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(
        bytes[at..at + 2]
            .try_into()
            .expect("two bytes in the program"),
    )
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(
        bytes[at..at + 4]
            .try_into()
            .expect("four bytes in the program"),
    )
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(
        bytes[at..at + 8]
            .try_into()
            .expect("eight bytes in the program"),
    )
}

/// A section of the program: virtual address, virtual size, raw pointer, raw
/// size.
type Section = (u32, u32, u32, u32);

fn sections(bytes: &[u8]) -> Vec<Section> {
    let pe = u32_at(bytes, 0x3C) as usize;
    let coff = pe + 4;
    let count = u16_at(bytes, coff + 2) as usize;
    let size_of_optional = u16_at(bytes, coff + 16) as usize;
    let headers = pe + 24 + size_of_optional;
    (0..count)
        .map(|index| {
            let at = headers + index * 40;
            (
                u32_at(bytes, at + 12),
                u32_at(bytes, at + 8),
                u32_at(bytes, at + 20),
                u32_at(bytes, at + 16),
            )
        })
        .collect()
}

fn directory(bytes: &[u8], index: usize) -> Option<(u32, u32)> {
    let pe = u32_at(bytes, 0x3C) as usize;
    let optional = pe + 24;
    if index >= u32_at(bytes, optional + 108) as usize {
        return None;
    }
    let at = optional + 112 + index * 8;
    let (rva, size) = (u32_at(bytes, at), u32_at(bytes, at + 4));
    (size != 0).then_some((rva, size))
}

fn file_offset(sections: &[Section], rva: u32) -> Option<usize> {
    for &(virtual_address, virtual_size, raw_pointer, raw_size) in sections {
        let span = virtual_size.max(raw_size);
        if rva >= virtual_address && rva < virtual_address + span {
            return Some((raw_pointer + (rva - virtual_address)) as usize);
        }
    }
    None
}

fn declared_imports(bytes: &[u8]) -> usize {
    let sections = sections(bytes);
    let Some((rva, _)) = directory(bytes, 1) else {
        return 0;
    };
    let Some(at) = file_offset(&sections, rva) else {
        panic!("the import directory is inside the program");
    };
    let mut count = 0;
    let mut descriptor = at;
    loop {
        let (lookup, address, first_thunk) = (
            u32_at(bytes, descriptor),
            u32_at(bytes, descriptor + 12),
            u32_at(bytes, descriptor + 16),
        );
        if lookup == 0 && address == 0 && first_thunk == 0 {
            break;
        }
        let thunks = if lookup != 0 { lookup } else { first_thunk };
        let mut thunk = file_offset(&sections, thunks).expect("a thunk array in the program");
        while u64_at(bytes, thunk) != 0 {
            count += 1;
            thunk += 8;
        }
        descriptor += 20;
    }
    count
}

#[test]
fn cxx_array_allocation_runs_at_every_optimization_level() {
    for name in [
        "guest-cxx-array-O0",
        "guest-cxx-array-O1",
        "guest-cxx-array-O2",
        "guest-cxx-array-Os",
    ] {
        let outcome = run(name).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            outcome.result.execution.exit_reason,
            ExitReason::ProcessExit(0),
            "{name}"
        );
        let program =
            std::fs::read(built().join(format!("{name}.exe"))).expect("read the built program");
        assert_eq!(
            outcome.result.bound_imports,
            declared_imports(&program),
            "{name}: bound imports"
        );
        assert!(
            outcome.result.dbt_metrics.blocks_compiled > 0,
            "{name}: no block compiled"
        );
        assert!(
            outcome.result.dbt_metrics.service_calls > 0,
            "{name}: no service call"
        );

        let block = result_block(&outcome);
        assert_eq!(block.magic, MAGIC, "{name}: result magic");
        assert_eq!(block.first_value, 0x14, "{name}: first array value");
        assert_eq!(block.second_value, 0x24, "{name}: second array value");
        assert_eq!(block.distinct, 1, "{name}: live arrays overlap");
        assert_eq!(block.zero_nonzero, 1, "{name}: zero-size array is null");
        assert_eq!(block.zero_distinct, 1, "{name}: zero-size arrays overlap");
        assert_ne!(block.first, 0, "{name}: first allocation");
        assert_ne!(block.second, 0, "{name}: second allocation");
        assert_ne!(block.zero_first, 0, "{name}: first zero-size allocation");
        assert_ne!(block.zero_second, 0, "{name}: second zero-size allocation");
    }
}
