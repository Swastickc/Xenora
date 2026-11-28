// SPDX-License-Identifier: GPL-3.0-or-later

//! A freestanding PE uses a stack frame larger than the historic startup stack.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use xenora_loader::cpu::ExitReason;
use xenora_loader::imports::ImportRegistry;
use xenora_loader::services::{MockClock, ServicesState, register_standard_services};
use xenora_loader::startup::{
    StartupEvent, StartupLayout, StartupResult, run_synthetic_startup_with_observer,
};

const IMAGE_BASE: u64 = 0x1_4000_0000;
const STACK_TOP: u64 = 0x2_0100_0000;
const TLS_BASE: u64 = 0x3_0000_0000;
const KIB: u64 = 1024;

fn fixtures() -> &'static Path {
    static OUT: OnceLock<PathBuf> = OnceLock::new();
    OUT.get_or_init(|| {
        let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("pe-startup-stack-{}", std::process::id()));
        let script = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/compat_fixtures/build-pe-startup-stack.sh"
        );
        let build = Command::new("sh").arg(script).arg(&out).output().unwrap();
        assert!(
            build.status.success(),
            "the freestanding stack fixtures did not build:\n{}{}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
        out
    })
}

fn image(name: &str) -> Vec<u8> {
    std::fs::read(fixtures().join(name)).unwrap()
}

// PE32+ places SizeOfStackReserve and SizeOfStackCommit at optional-header
// offsets 72 and 80. Controls alter only those fields in the linked image.
fn stack_fields(bytes: &[u8]) -> usize {
    let signature = u32::from_le_bytes(bytes[0x3c..0x40].try_into().unwrap()) as usize;
    let optional = signature + 4 + 20;
    assert_eq!(&bytes[signature..signature + 4], b"PE\0\0");
    assert_eq!(&bytes[optional..optional + 2], &[0x0b, 0x02]);
    optional + 72
}

fn with_stack(mut bytes: Vec<u8>, reserve: u64, commit: u64) -> Vec<u8> {
    let offset = stack_fields(&bytes);
    bytes[offset..offset + 8].copy_from_slice(&reserve.to_le_bytes());
    bytes[offset + 8..offset + 16].copy_from_slice(&commit.to_le_bytes());
    bytes
}

fn run(
    bytes: &[u8],
    observe: impl FnMut(StartupEvent),
) -> Result<StartupResult, Box<dyn std::error::Error>> {
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let mut imports = ImportRegistry::new();
    register_standard_services(&mut imports, state);
    let layout = StartupLayout {
        image_base: IMAGE_BASE,
        stack_top: STACK_TOP,
        tls_base: TLS_BASE,
    };
    run_synthetic_startup_with_observer(bytes, layout, imports, 20_000, observe)
}

fn assert_stack(result: &StartupResult, expected: u64) {
    assert_eq!(result.execution.exit_reason, ExitReason::ProcessExit(0));
    let teb = result.execution.final_state.gs_base;
    let memory = &result.image.memory;
    let high = memory.read_u64(teb + 0x08).unwrap();
    let low = memory.read_u64(teb + 0x10).unwrap();
    assert_eq!(high, STACK_TOP);
    assert_eq!(low, high - expected);
    assert!(memory.read_u8(low).is_ok());
    assert!(memory.read_u8(low - 1).is_err());
}

#[test]
fn linker_reservation_supports_a_large_compiled_stack_frame() {
    let bytes = image("entry.exe");
    let offset = stack_fields(&bytes);
    assert_eq!(&bytes[offset..offset + 8], &(256 * KIB).to_le_bytes());
    assert_eq!(&bytes[offset + 8..offset + 16], &(128 * KIB).to_le_bytes());
    assert_stack(&run(&bytes, |_| {}).unwrap(), 256 * KIB);

    let short_stack = with_stack(bytes, 64 * KIB, 64 * KIB);
    let result = run(&short_stack, |_| {}).unwrap();
    assert!(
        matches!(result.execution.exit_reason, ExitReason::MemoryFault(_)),
        "large stack frame unexpectedly ended with {:?}",
        result.execution.exit_reason
    );
}

#[test]
fn startup_rounds_reservation_and_preserves_the_zero_field_default() {
    let bytes = image("small_entry.exe");
    let rounded = with_stack(bytes.clone(), 192 * KIB + 1, 128 * KIB);
    assert_stack(&run(&rounded, |_| {}).unwrap(), 256 * KIB);
    let legacy = with_stack(bytes, 0, 0);
    assert_stack(&run(&legacy, |_| {}).unwrap(), 64 * KIB);
}

#[test]
fn invalid_stack_headers_never_enter_the_guest() {
    let bytes = image("small_entry.exe");
    for (reserve, commit) in [
        (64 * KIB + 1, 128 * KIB),
        (8 * 1024 * KIB + 1, 64 * KIB),
        (u64::MAX, 0),
    ] {
        let malformed = with_stack(bytes.clone(), reserve, commit);
        let mut entered = false;
        let result = run(&malformed, |event| {
            entered |= matches!(event, StartupEvent::Entering { .. });
        });
        assert!(result.is_err(), "accepted stack {reserve}/{commit}");
        assert!(!entered, "started guest with stack {reserve}/{commit}");
    }
}
