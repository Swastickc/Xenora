// SPDX-License-Identifier: GPL-3.0-or-later

//! Call the UCRT WinMain command tail from a compiler-linked guest.

use std::path::{Path, PathBuf};
use std::process::Command;
use xenora_loader::probe::{ProbeConfig, probe_pe_bytes};

#[test]
fn winmain_receives_the_raw_command_tail_from_the_cli() {
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("winmain-command-line-{}", std::process::id()));
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/compat_fixtures/build-crt-winmain-command-line.sh");
    let build = Command::new("sh")
        .arg(script)
        .arg(&directory)
        .output()
        .expect("run build-crt-winmain-command-line.sh");
    assert!(build.status.success(), "{build:?}");

    let path = directory.join("crt WinMain command line.exe");
    let bytes = std::fs::read(&path).expect("read fixture PE");
    let report = probe_pe_bytes(&bytes, &ProbeConfig::default()).expect("inspect fixture PE");
    assert_eq!(report.import_error, None);
    for (module, symbol) in [
        (
            "api-ms-win-crt-runtime-l1-1-0.dll",
            "_get_narrow_winmain_command_line",
        ),
        ("kernel32.dll", "GetCommandLineA"),
        ("kernel32.dll", "ExitProcess"),
    ] {
        assert!(
            report
                .imports
                .iter()
                .any(|entry| entry.dll.eq_ignore_ascii_case(module) && entry.symbol == symbol),
            "missing {module}!{symbol}",
        );
    }

    for (arguments, status) in [
        (&[][..], 41),
        (&["two  words", "say\"hello", "", "a\tb", "--trace"][..], 42),
    ] {
        let run = Command::new(env!("CARGO_BIN_EXE_xenora"))
            .args(["run", "--experimental", "--trace", "--pe"])
            .arg(&path)
            .arg("--")
            .args(arguments)
            .output()
            .expect("run xenora");
        let trace = String::from_utf8_lossy(&run.stderr);
        assert_eq!(run.status.code(), Some(status), "{arguments:?}\n{trace}");
        let expected = format!("exit=ProcessExit({status})");
        assert!(
            trace
                .lines()
                .any(|line| { line.starts_with("trace stage=exit ") && line.ends_with(&expected) }),
            "{arguments:?}\n{trace}",
        );
    }
}
