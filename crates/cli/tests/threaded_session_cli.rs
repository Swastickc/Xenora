// SPDX-License-Identifier: GPL-3.0-or-later

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const DIRECTIONS: [&str; 6] = ["right", "right", "left", "left", "left", "right"];
const COLUMNS: &str = "121030";
const EXIT_STATUS: u32 = 0x73D0_CA11;
const CLEAR: [u8; 4] = [19, 43, 93, 251];
const RED: [u8; 4] = [255, 0, 0, 255];
static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        loop {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "xenora threaded session {} {sequence}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create test directory {}: {error}", path.display()),
            }
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn command(backend: &str, frame_dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xenora-session"));
    command
        .args([
            "--backend",
            backend,
            "--workload",
            "threaded",
            "--frame-dir",
        ])
        .arg(frame_dir);
    command
}

fn assert_status(output: &Output, expected: i32) {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn verify_run(
    output: &Output,
    backend: &str,
    directory: &Path,
    directions: &[&str],
    columns: &str,
    native_draws: usize,
) {
    assert_status(output, 0);
    assert_eq!(directions.len(), columns.len());
    let mut lines = std::str::from_utf8(&output.stdout).unwrap().lines();
    assert_eq!(fs::read_dir(directory).unwrap().count(), directions.len());

    for (index, (&direction, digit)) in directions.iter().zip(columns.bytes()).enumerate() {
        let sequence = index + 1;
        let column = usize::from(digit - b'0');
        let mut rgba = String::new();
        let mut ppm = b"P6\n4 3\n255\n".to_vec();
        for pixel in 0..12 {
            let color = if pixel % 4 == column { RED } else { CLEAR };
            for byte in color {
                write!(rgba, "{byte:02x}").unwrap();
            }
            ppm.extend_from_slice(&color[..3]);
        }
        let record = format!(
            "frame sequence={sequence} direction={direction} backend={backend} width=4 height=3 draws=1 rgba={rgba}"
        );
        assert_eq!(lines.next(), Some(record.as_str()), "frame {sequence}");
        let path = directory.join(format!("frame-{sequence:04}.ppm"));
        assert_eq!(fs::read(&path).unwrap(), ppm, "{}", path.display());
    }

    let terminal = format!(
        "exit status={EXIT_STATUS} frames={} journal={columns} handles=0 native_draws={native_draws} workload=threaded",
        directions.len()
    );
    assert_eq!(lines.next(), Some(terminal.as_str()));
    assert_eq!(
        lines.next(),
        None,
        "unexpected stdout after the terminal record"
    );
}

fn verify_positional_and_replay(
    backend: &str,
    directions: &[&str],
    columns: &str,
    native_draws: usize,
) {
    let scratch = Scratch::new();
    let replay = scratch.0.join("recorded directions.txt");
    fs::write(
        &replay,
        format!("xenora-session-v1\n{}\n", directions.join("\n")),
    )
    .unwrap();

    let positional_frames = scratch.0.join("positional frames");
    let positional = command(backend, &positional_frames)
        .args(directions)
        .output()
        .expect("run xenora-session with positional directions");
    verify_run(
        &positional,
        backend,
        &positional_frames,
        directions,
        columns,
        native_draws,
    );

    let replay_frames = scratch.0.join("replay frames");
    let replayed = command(backend, &replay_frames)
        .arg("--replay")
        .arg(&replay)
        .output()
        .expect("run xenora-session with a replay");
    verify_run(
        &replayed,
        backend,
        &replay_frames,
        directions,
        columns,
        native_draws,
    );
    assert_eq!(replayed.stdout, positional.stdout);
}

#[test]
fn threaded_reference_replay_matches_positional_stdout_and_ppms() {
    verify_positional_and_replay("reference", &DIRECTIONS, COLUMNS, 0);
}

#[test]
fn threaded_reference_wraps_columns_across_all_32_events() {
    verify_positional_and_replay(
        "reference",
        &["left"; 32],
        "32103210321032103210321032103210",
        0,
    );
}

#[test]
fn threaded_budget_failure_after_a_frame_publishes_nothing() {
    let scratch = Scratch::new();
    let short_frames = scratch.0.join("completed frames");
    let short = command("reference", &short_frames)
        .args(["--steps", "1536", "right"])
        .output()
        .expect("run the one-event threaded session");
    verify_run(&short, "reference", &short_frames, &["right"], "1", 0);

    // The scheduled adapter probe for this sequence and budget reported at
    // least one frame before it exhausted the budget.
    let refused_frames = scratch.0.join("refused frames");
    let refused = command("reference", &refused_frames)
        .args([
            "--steps", "1536", "right", "left", "left", "right", "left", "right", "left", "left",
        ])
        .output()
        .expect("run the budget-limited threaded session");
    assert_status(&refused, 125);
    assert!(
        String::from_utf8_lossy(&refused.stderr)
            .contains("guest instruction budget exhausted (1536 steps)")
    );
    assert!(
        refused.stdout.is_empty(),
        "failed run exposed collected frames"
    );
    assert!(!refused_frames.exists(), "failed run published a directory");
    assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 1);
}

#[test]
#[ignore = "requires a native Metal device and validation enabled"]
fn threaded_metal_draws_and_exports_every_frame_for_positional_and_replay() {
    verify_positional_and_replay("metal", &DIRECTIONS, COLUMNS, DIRECTIONS.len());
}
