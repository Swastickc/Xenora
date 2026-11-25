// SPDX-License-Identifier: GPL-3.0-or-later

//! Deterministic Win32/CRT import binding generator for Xenora.
//!
//! Reads an import list (PE file or CSV produced by `xenora probe`), classifies each
//! symbol against a built-in public-API database derived from documented Win32/CRT
//! specifications and MinGW-w64 / POSIX equivalents, then generates:
//!
//! - `generated_crt.rs`      — CRT functions mapped to native libc via `extern "C"` FFI
//! - `generated_stubs.rs`    — Logged stubs for functions that cannot be automatically mapped
//! - `generated_tests.rs`    — Conformance tests for every generated binding
//! - `generated_manifest.csv`— Machine-readable coverage report
//!
//! The generator is completely offline. It consumes no AI tokens and has no network
//! dependencies. Output is deterministic and idempotent given the same inputs.
//!
//! # Classification tiers
//!
//! | Tier | Meaning |
//! |------|---------|
//! | `CrtNative`  | Directly maps to a libc symbol with identical semantics |
//! | `CrtWrapper` | Maps to libc with argument/return marshalling |
//! | `Win32Stub`  | Windows-specific; generated as a logged stub |
//! | `Unsupported`| Proprietary/unavailable; noted in manifest, no code emitted |
//! | `Manual`     | Requires hand-written implementation; flagged in report |

pub mod apiset;
pub mod classify;
pub mod codegen;
pub mod demangle;
pub mod ingest;
pub mod manifest;
pub mod metadata;
pub mod parse;
pub mod typed;
pub mod verify;

pub use apiset::{get_api_sets_for_canonical, resolve_api_set_canonical};
pub use classify::{Classification, SymbolDb};
pub use codegen::{GeneratedFiles, generate};
pub use demangle::demangle;
pub use ingest::{ingest_header_file, ingest_standard_system_headers};
pub use manifest::Manifest;
pub use metadata::{AbiType, CallingConvention, FunctionSignature, MetadataCatalog};
pub use parse::{ImportRecord, parse_csv};
pub use verify::{VerificationResult, run_verification};

/// Top-level entry point.  Reads `csv_path`, classifies every symbol, generates
/// Rust source into `out_dir`, and writes `generated_manifest.csv`.
pub fn run(csv_path: &str, out_dir: &str) -> Result<Manifest, String> {
    let records = parse_csv(csv_path)?;
    let db = SymbolDb::builtin();
    let manifest = Manifest::build(&records, &db);
    let files = generate(&manifest, &db);
    files.write_all(out_dir)?;
    manifest.write_csv(&format!("{out_dir}/generated_manifest.csv"))?;
    Ok(manifest)
}
