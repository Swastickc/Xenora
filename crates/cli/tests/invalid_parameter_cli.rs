// SPDX-License-Identifier: GPL-3.0-or-later

//! Run SDK-header guests that call the CRT invalid-parameter API set.

use std::path::{Path, PathBuf};
use std::process::Command;
use xenora_loader::probe::{ProbeConfig, probe_pe_bytes};

const RUNTIME: &str = "api-ms-win-crt-runtime-l1-1-0.dll";

fn assert_crt_imports(path: &Path, noreturn: bool) {
    let bytes = std::fs::read(path).expect("read fixture PE");
    let report = probe_pe_bytes(&bytes, &ProbeConfig::default()).expect("inspect fixture PE");
    assert_eq!(report.import_error, None);
    for (symbol, expected) in [
        ("_set_invalid_parameter_handler", true),
        ("_invalid_parameter_noinfo", true),
        ("_invalid_parameter_noinfo_noreturn", noreturn),
    ] {
        assert_eq!(
            report
                .imports
                .iter()
                .any(|entry| entry.dll.eq_ignore_ascii_case(RUNTIME) && entry.symbol == symbol),
            expected,
            "{}: {RUNTIME}!{symbol}",
            path.display(),
        );
    }
}

#[test]
fn invalid_parameter_api_set_runs_both_variants_through_the_cli() {
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("invalid-parameter-{}", std::process::id()));
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/compat_fixtures/build-crt-invalid-parameter.sh");
    let build = Command::new("sh")
        .arg(script)
        .arg(&directory)
        .output()
        .expect("run build-crt-invalid-parameter.sh");
    assert!(build.status.success(), "{build:?}");

    for (variant, noreturn, guest_code, host_code) in [
        ("return", false, 0u32, 0),
        ("noreturn", true, 0xc000_0417, 23),
    ] {
        let path = directory.join(format!("crt-invalid-parameter-{variant}.exe"));
        assert_crt_imports(&path, noreturn);
        let run = Command::new(env!("CARGO_BIN_EXE_xenora"))
            .args(["run", "--experimental", "--trace", "--pe"])
            .arg(path)
            .output()
            .expect("run xenora");
        let trace = String::from_utf8_lossy(&run.stderr);
        assert_eq!(run.status.code(), Some(host_code), "{variant}\n{trace}");
        let expected_exit = format!("exit=ProcessExit({guest_code})");
        assert!(
            trace.lines().any(|line| {
                line.starts_with("trace stage=exit ") && line.ends_with(&expected_exit)
            }),
            "{variant}\n{trace}",
        );
    }
}
