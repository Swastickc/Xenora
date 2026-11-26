// SPDX-License-Identifier: GPL-3.0-or-later

//! A startup report must leave the hosted program's standard input alone.

use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const INPUT: &[u8] = b"alpha\r\nbeta\r\n0123456789\r\nlast";

fn fixture() -> &'static Path {
    static FIXTURE: OnceLock<PathBuf> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("startup-stdin-{}", std::process::id()));
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/compat_fixtures/build-crt-stdin.sh");
        let build = Command::new("sh")
            .arg(&script)
            .arg(&directory)
            .output()
            .expect("run build-crt-stdin.sh");
        assert!(
            build.status.success(),
            "{} failed:\n{}{}",
            script.display(),
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr),
        );
        directory.join("crt-stdin-O2.exe")
    })
}

fn guest_args(command: &mut Command, options: &[&str], guest: &[&str]) {
    command
        .args(["run", "--experimental", "--pe"])
        .arg(fixture())
        .args(options)
        .arg("--")
        .args(guest);
}

fn spawn(options: &[&str]) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xenora"));
    guest_args(&mut command, options, &[]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn xenora")
}

fn run_with_input(options: &[&str], bytes: &[u8]) -> Output {
    let mut child = spawn(options);
    let written = child.stdin.take().expect("piped stdin").write_all(bytes);
    let output = child.wait_with_output().expect("wait for xenora");
    // An early-rejecting worker can close its end before the writer finishes.
    // Preserve the actual exit status and report for the assertions below.
    if let Err(error) = written {
        assert_eq!(error.kind(), ErrorKind::BrokenPipe, "{error}");
    }
    output
}

fn report(stderr: &[u8]) -> &str {
    let stderr = std::str::from_utf8(stderr).expect("UTF-8 diagnostics");
    let mut reports = stderr
        .lines()
        .filter_map(|line| line.strip_prefix("xenora-startup "));
    let first = reports
        .next()
        .unwrap_or_else(|| panic!("no report: {stderr}"));
    assert!(reports.next().is_none(), "multiple reports: {stderr}");
    first
}

fn assert_success_report(stderr: &[u8]) {
    let json = report(stderr);
    assert!(
        json.contains(r#""last_completed_stage":"entry_returned","protocol":"complete""#),
        "{json}"
    );
    assert!(
        json.contains(r#""outcome":{"kind":"guest_exit","guest_status":0,"worker_code":0}"#),
        "{json}"
    );
}

#[test]
fn piped_input_and_guest_output_are_unchanged_by_the_report() {
    let plain = run_with_input(&[], INPUT);
    let reported = run_with_input(&["--startup-report"], INPUT);
    assert_eq!(plain.status.code(), Some(0), "{plain:?}");
    assert_eq!(plain.stdout, b"crt_stdin: 7 of 7 checks passed\r\n");
    assert!(plain.stderr.is_empty(), "{plain:?}");
    assert_eq!(reported.status.code(), plain.status.code(), "{reported:?}");
    assert_eq!(reported.stdout, plain.stdout);
    assert_success_report(&reported.stderr);
}

#[test]
fn oversized_input_fails_before_guest_entry_with_or_without_the_report() {
    let bytes = vec![b'x'; (1 << 20) + 1];
    for options in [&[][..], &["--startup-report"][..]] {
        let output = run_with_input(options, &bytes);
        assert_eq!(output.status.code(), Some(125), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        let stderr = std::str::from_utf8(&output.stderr).unwrap();
        assert!(
            stderr.contains("exceeds the 1048576 byte limit"),
            "{stderr}"
        );
        if !options.is_empty() {
            let json = report(&output.stderr);
            assert!(json.contains(r#""events":[]"#), "{json}");
            assert!(
                json.contains(r#""outcome":{"kind":"known_failure","phase":"worker""#),
                "{json}"
            );
        }
    }
}

#[test]
fn deadline_includes_waiting_for_input_eof_when_the_report_is_enabled() {
    let mut child = spawn(&["--startup-report", "--wall-seconds", "1"]);
    let mut writer = child.stdin.take().expect("piped stdin");
    // Send valid input but retain the pipe's write end. If the worker
    // ignores stdin, it can exit instead of waiting for EOF.
    writer.write_all(INPUT).expect("write input");
    let until = Instant::now() + Duration::from_secs(8);
    loop {
        if child.try_wait().expect("poll xenora").is_some() {
            break;
        }
        if Instant::now() >= until {
            let _ = child.kill();
            drop(writer);
            let output = child.wait_with_output().expect("reap xenora");
            panic!("xenora outlived its one-second input deadline: {output:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().expect("read xenora output");
    drop(writer);
    assert_eq!(output.status.code(), Some(124), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let json = report(&output.stderr);
    assert!(json.contains(r#""events":[]"#), "{json}");
    assert!(
        json.contains(r#""outcome":{"kind":"wall_timeout""#),
        "{json}"
    );
}

#[test]
fn closed_standard_input_is_eof_with_the_report() {
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", r#"exec 0<&-; exec "$@""#, "closed-stdin"])
        .arg(env!("CARGO_BIN_EXE_xenora"));
    guest_args(&mut command, &["--startup-report"], &["empty"]);
    let output = command.output().expect("run xenora with closed stdin");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, b"crt_stdin: 5 of 5 checks passed\r\n");
    assert_success_report(&output.stderr);
}

#[test]
fn worker_rejects_standard_or_closed_diagnostic_descriptors() {
    for fd in [0, i32::MAX] {
        let output = Command::new(env!("CARGO_BIN_EXE_xenora"))
            .args(["run", "--experimental", "--worker", "--program"])
            .arg("--startup-report-worker")
            .arg(fd.to_string())
            .arg("--pe")
            .arg(fixture())
            .output()
            .expect("run xenora worker directly");
        let stderr = std::str::from_utf8(&output.stderr).unwrap();
        assert_eq!(output.status.code(), Some(125), "fd={fd}: {output:?}");
        assert!(output.stdout.is_empty(), "fd={fd}: {output:?}");
        assert!(
            stderr.contains("cannot open startup diagnostic channel"),
            "fd={fd}: {stderr}"
        );
    }
}
