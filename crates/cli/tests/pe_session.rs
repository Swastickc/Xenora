// SPDX-License-Identifier: GPL-3.0-or-later

//! An authored PE exchanges requests and CPU frames with the shipped CLI.

use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

const EXECUTABLE: &str = env!("CARGO_BIN_EXE_xenora-pe-session");
const TIMEOUT: Duration = Duration::from_secs(20);

fn fixture(name: &str) -> PathBuf {
    static DIRECTORY: OnceLock<PathBuf> = OnceLock::new();
    DIRECTORY
        .get_or_init(|| {
            let directory = Path::new(env!("CARGO_TARGET_TMPDIR"))
                .join(format!("authored-session-{}", std::process::id()));
            let script = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/pe/authored-session/build-authored-session.sh");
            let build = Command::new("sh")
                .arg(script)
                .arg(&directory)
                .output()
                .expect("build the authored PE session fixtures");
            assert!(build.status.success(), "{build:?}");
            directory
        })
        .join(format!("{name}.exe"))
}

fn command(path: &Path, digest: Option<&str>) -> Command {
    let mut command = Command::new(EXECUTABLE);
    let digest = digest.map_or_else(
        || format!("{:x}", Sha256::digest(std::fs::read(path).unwrap())),
        str::to_owned,
    );
    command
        .arg("--pe")
        .arg(path)
        .args(["--sha256", &digest, "--experimental"]);
    command
}

struct Session {
    child: Child,
    input: Option<ChildStdin>,
    lines: Receiver<String>,
    diagnostics: Receiver<String>,
}

struct Completed {
    status: ExitStatus,
    lines: Vec<String>,
    diagnostics: String,
}

impl Completed {
    fn assert_code(&self, expected: i32) {
        assert_eq!(
            self.status.code(),
            Some(expected),
            "stdout: {:?}\nstderr: {}",
            self.lines,
            self.diagnostics,
        );
    }
}

impl Session {
    fn start(command: &mut Command) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start xenora-pe-session");
        let (lines_tx, lines) = mpsc::channel();
        let stdout = child.stdout.take().unwrap();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if lines_tx.send(line.expect("read session stdout")).is_err() {
                    break;
                }
            }
        });
        let (diagnostics_tx, diagnostics) = mpsc::channel();
        let mut stderr = child.stderr.take().unwrap();
        thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr.read_to_end(&mut bytes).expect("read session stderr");
            let _ = diagnostics_tx.send(String::from_utf8_lossy(&bytes).into_owned());
        });
        Self {
            input: child.stdin.take(),
            child,
            lines,
            diagnostics,
        }
    }

    fn next_line(&self) -> String {
        self.lines.recv_timeout(TIMEOUT).unwrap_or_else(|error| {
            panic!(
                "waiting for session output: {error}; stderr: {}",
                self.diagnostics.try_recv().unwrap_or_default(),
            )
        })
    }

    fn send(&mut self, input: u32) {
        writeln!(self.input.as_mut().unwrap(), "{input}").expect("send a live session request");
    }

    fn finish(mut self) -> Completed {
        drop(self.input.take());
        let deadline = Instant::now() + TIMEOUT;
        let status = loop {
            if let Some(status) = self.child.try_wait().expect("wait for session") {
                break status;
            }
            assert!(Instant::now() < deadline, "session did not exit after EOF");
            thread::sleep(Duration::from_millis(10));
        };
        let mut lines = Vec::new();
        loop {
            match self.lines.recv_timeout(TIMEOUT) {
                Ok(line) => lines.push(line),
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => panic!("session stdout did not close"),
            }
        }
        Completed {
            status,
            lines,
            diagnostics: self
                .diagnostics
                .recv_timeout(TIMEOUT)
                .expect("session stderr did not close"),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn expected_frame(sequence: u8, input: u32, pixels: bool) -> String {
    expected_frame_with_blue(sequence, input, 0x3c, pixels)
}

fn expected_frame_with_blue(sequence: u8, input: u32, blue: u8, pixels: bool) -> String {
    let red_column = if input.is_multiple_of(2) { 0 } else { 3 };
    let mut rgba = Vec::new();
    for _ in 0..3 {
        for column in 0..4 {
            let pixel = if column == red_column {
                [255, 0, 0, 255]
            } else {
                [input as u8, sequence, blue, 0xff]
            };
            rgba.extend_from_slice(&pixel);
        }
    }
    let hash = Sha256::digest(&rgba);
    let mut line = format!("frame {sequence} width=4 height=3 sha256={hash:x}");
    if pixels {
        line.push_str(" rgba=");
        for byte in rgba {
            write!(&mut line, "{byte:02x}").unwrap();
        }
    }
    line
}

#[test]
fn live_requests_keep_guest_state_until_clean_eof() {
    let path = fixture("authored-session");
    let mut session =
        Session::start(command(&path, None).args(["--pixels", "--phase-seconds", "1"]));
    assert_eq!(session.next_line(), "ready");

    session.send(10);
    assert_eq!(session.next_line(), expected_frame(1, 10, true));
    // The guest has finished processing. Keeping stdin open here must neither
    // trip the guest phase deadline nor produce a frame without a request.
    assert_eq!(
        session.lines.recv_timeout(Duration::from_millis(1200)),
        Err(RecvTimeoutError::Timeout),
    );

    session.send(21);
    assert_eq!(session.next_line(), expected_frame(2, 21, true));
    session.send(10);
    assert_eq!(session.next_line(), expected_frame(3, 10, true));

    let completed = session.finish();
    completed.assert_code(0);
    assert_eq!(completed.lines, ["done frames=3"]);
}

#[test]
fn deterministic_clock_advances_for_inputs_and_guest_sleep_but_not_host_idle() {
    let path = fixture("authored-session-deterministic-clock");
    for host_idle in [false, true] {
        let mut session =
            Session::start(command(&path, None).args(["--deterministic-clock", "--pixels"]));
        assert_eq!(session.next_line(), "ready");
        // The first input advances by 16 ms. Between frames the guest sleeps
        // for 5 ms, and each next input advances by another 16 ms.
        for (sequence, milliseconds) in [(1, 16), (2, 37), (3, 58)] {
            if host_idle {
                assert_eq!(
                    session.lines.recv_timeout(Duration::from_millis(125)),
                    Err(RecvTimeoutError::Timeout),
                );
            }
            session.send(10);
            assert_eq!(
                session.next_line(),
                expected_frame_with_blue(sequence, 10, milliseconds, true),
            );
        }
        let completed = session.finish();
        completed.assert_code(0);
        assert_eq!(completed.lines, ["done frames=3"]);
    }
}

#[test]
fn admission_and_instruction_budget_failures_publish_no_completion() {
    let path = fixture("authored-session");
    let wrong_digest = "0".repeat(64);
    let rejected = Session::start(&mut command(&path, Some(&wrong_digest))).finish();
    rejected.assert_code(125);
    assert!(rejected.lines.is_empty(), "{:?}", rejected.lines);
    assert!(
        rejected.diagnostics.contains("SHA-256"),
        "{}",
        rejected.diagnostics
    );

    let exhausted = Session::start(command(&path, None).args(["--max-steps", "1"])).finish();
    exhausted.assert_code(125);
    assert!(exhausted.lines.is_empty(), "{:?}", exhausted.lines);
    assert!(
        exhausted.diagnostics.contains("budget"),
        "{}",
        exhausted.diagnostics
    );
}

#[test]
fn guest_protocol_and_cleanup_failures_never_publish_done() {
    for variant in ["missing-present", "extra-present", "leak"] {
        let path = fixture(&format!("authored-session-{variant}"));
        let mut session = Session::start(&mut command(&path, None));
        assert_eq!(session.next_line(), "ready");
        session.send(10);
        if variant == "leak" {
            assert_eq!(session.next_line(), expected_frame(1, 10, false));
        }
        let completed = session.finish();
        completed.assert_code(125);
        assert!(
            completed
                .lines
                .iter()
                .all(|line| !line.starts_with("done ")),
            "{variant}: {:?}",
            completed.lines,
        );
        assert!(!completed.diagnostics.is_empty(), "{variant}");
    }
}

#[test]
fn a_frame_phase_deadline_terminates_the_worker() {
    let path = fixture("authored-session-stall-frame");
    let mut session = Session::start(command(&path, None).args(["--phase-seconds", "1"]));
    assert_eq!(session.next_line(), "ready");
    session.send(10);
    let completed = session.finish();
    completed.assert_code(124);
    assert!(completed.lines.is_empty(), "{:?}", completed.lines);
    assert!(
        completed
            .diagnostics
            .contains("frame exceeded the phase deadline"),
        "{}",
        completed.diagnostics
    );
}

#[test]
fn eof_without_input_and_full_sessions_both_finish() {
    let path = fixture("authored-session");
    for count in [0, 64] {
        let mut session = Session::start(&mut command(&path, None));
        assert_eq!(session.next_line(), "ready");
        for sequence in 1..=count {
            session.send(u32::MAX);
            assert_eq!(
                session.next_line(),
                expected_frame(sequence, u32::MAX, false)
            );
        }
        let completed = session.finish();
        completed.assert_code(0);
        assert_eq!(completed.lines, [format!("done frames={count}")]);
    }
}

#[test]
fn an_invalid_host_line_fails_without_publishing_done() {
    let path = fixture("authored-session");
    let mut session = Session::start(&mut command(&path, None));
    assert_eq!(session.next_line(), "ready");
    writeln!(session.input.as_mut().unwrap(), "4294967296").unwrap();
    let completed = session.finish();
    completed.assert_code(125);
    assert!(completed.lines.is_empty(), "{:?}", completed.lines);
    assert!(
        completed
            .diagnostics
            .contains("unsigned 32-bit decimal value")
    );
}

#[test]
fn required_options_and_invalid_limits_are_usage_errors() {
    let digest = "0".repeat(64);
    for flags in [
        vec!["--experimental", "--sha256", &digest, "--pe", "--pixels"],
        vec!["--experimental", "--pe", "some.exe"],
        vec!["--pe", "some.exe", "--sha256", &digest],
        vec!["--experimental", "--pe", "some.exe", "--sha256", "xyz"],
        vec![
            "--experimental",
            "--pe",
            "some.exe",
            "--sha256",
            &digest,
            "--phase-seconds",
            "0",
        ],
        vec![
            "--experimental",
            "--pe",
            "some.exe",
            "--sha256",
            &digest,
            "--max-steps",
            "10000001",
        ],
    ] {
        let invocation = Command::new(env!("CARGO_BIN_EXE_xenora-pe-session"))
            .args(flags)
            .output()
            .unwrap();
        assert_eq!(invocation.status.code(), Some(2));
        assert!(invocation.stdout.is_empty());
    }
}
