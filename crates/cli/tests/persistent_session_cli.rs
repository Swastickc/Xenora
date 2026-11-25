// SPDX-License-Identifier: GPL-3.0-or-later

use std::ffi::OsStr;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const EXECUTABLE: &str = env!("CARGO_BIN_EXE_xenora-session");
const CLEAR: [u8; 4] = [19, 43, 93, 251];
const RED: [u8; 4] = [255, 0, 0, 255];
static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        loop {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "xenora persistent session {} {sequence}",
                std::process::id()
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create test directory {}: {error}", path.display()),
            }
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Output {
    Command::new(EXECUTABLE)
        .args(args)
        .output()
        .expect("run the xenora-session CLI")
}

fn text(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).expect("CLI stdout is UTF-8")
}

fn status(output: &Output, expected: i32) {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "stdout:\n{}\nstderr:\n{}",
        text(output),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn pixels(direction: &str) -> [[u8; 4]; 12] {
    let column = match direction {
        "left" => 0,
        "right" => 3,
        _ => panic!("unexpected test direction {direction}"),
    };
    // Calculate all 12 texels independently in row-major order.
    std::array::from_fn(|pixel| if pixel % 4 == column { RED } else { CLEAR })
}

fn verify_run(output: Output, backend: &str, sequence: &[&str], journal: &str, native: usize) {
    status(&output, 0);
    let mut lines = text(&output).lines();
    for (index, direction) in sequence.iter().enumerate() {
        let mut hex = String::new();
        for byte in pixels(direction).into_iter().flatten() {
            write!(hex, "{byte:02x}").unwrap();
        }
        let line = format!(
            "frame sequence={} direction={direction} backend={backend} width=4 height=3 draws=1 rgba={hex}",
            index + 1
        );
        assert_eq!(lines.next(), Some(line.as_str()), "frame {}", index + 1);
    }
    let terminal = format!(
        "exit status=0 frames={} journal={journal} handles=0 native_draws={native}",
        sequence.len()
    );
    assert_eq!(lines.next(), Some(terminal.as_str()));
    assert_eq!(
        lines.next(),
        None,
        "unexpected stdout after the terminal record"
    );
}

fn verify_exports(directory: &Path, sequence: &[&str]) {
    let mut actual: Vec<_> = std::fs::read_dir(directory)
        .expect("read exported frame directory")
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    actual.sort_unstable();

    let expected: Vec<_> = (1..=sequence.len())
        .map(|index| format!("frame-{index:04}.ppm"))
        .collect();
    assert_eq!(actual, expected);

    for (filename, direction) in expected.iter().zip(sequence) {
        let frame = std::fs::read(directory.join(filename)).expect("read exported PPM");
        let rgb = frame
            .strip_prefix(b"P6\n4 3\n255\n")
            .expect("P6 header for 4 by 3 RGB");
        let expected: Vec<_> = pixels(direction)
            .into_iter()
            .flat_map(|[r, g, b, _]| [r, g, b])
            .collect();
        assert_eq!(rgb, expected, "{filename}");
    }
}

#[test]
fn reference_default_sequence_renders_exact_pixels() {
    verify_run(
        run(["--backend", "reference"]),
        "reference",
        &["left", "right", "left"],
        "LRL",
        0,
    );
}

#[test]
fn reference_repeated_directions_preserve_order_and_export_each_frame() {
    let scratch = Scratch::new();
    let frame_dir = scratch.0.join("requested frames");
    let sequence = ["right", "left", "left", "right", "right"];
    let result = Command::new(EXECUTABLE)
        .args([
            "--backend",
            "reference",
            "--steps",
            "1000000",
            "--frame-dir",
        ])
        .arg(&frame_dir)
        .args(sequence)
        .output()
        .expect("run the xenora-session CLI");
    verify_run(result, "reference", &sequence, "RLLRR", 0);
    verify_exports(&frame_dir, &sequence);
}

#[test]
fn instruction_exhaustion_does_not_publish_frames() {
    let scratch = Scratch::new();
    let frame_dir = scratch.0.join("must stay absent");
    let result = Command::new(EXECUTABLE)
        .args(["--backend", "reference", "--steps", "1", "--frame-dir"])
        .arg(&frame_dir)
        .output()
        .expect("run the xenora-session CLI");
    status(&result, 125);
    assert!(!text(&result).lines().any(|line| line.starts_with("frame ")));
    assert!(!frame_dir.exists(), "failed run published a directory");
}

#[test]
fn a_budget_failure_after_a_frame_does_not_publish_partial_results() {
    // This budget is enough to finish with just the first direction.
    verify_run(
        run(["--backend", "reference", "--steps", "512", "left"]),
        "reference",
        &["left"],
        "L",
        0,
    );
    let scratch = Scratch::new();
    let frame_dir = scratch.0.join("late failure");
    let result = Command::new(EXECUTABLE)
        .args(["--backend", "reference", "--steps", "512", "--frame-dir"])
        .arg(&frame_dir)
        .args(["left", "right", "left"])
        .output()
        .expect("run the xenora-session CLI");
    status(&result, 125);
    assert!(String::from_utf8_lossy(&result.stderr).contains("instruction budget exhausted"));
    assert!(
        result.stdout.is_empty(),
        "failed run exposed collected frames"
    );
    assert!(!frame_dir.exists(), "failed run published a directory");
}

#[test]
fn the_full_event_budget_preserves_every_frame_and_journal_byte() {
    let directions: [&str; 32] =
        std::array::from_fn(|index| if index % 2 == 0 { "left" } else { "right" });
    verify_run(
        run(["--backend", "reference"].into_iter().chain(directions)),
        "reference",
        &directions,
        &"LR".repeat(16),
        0,
    );
}

#[test]
fn invalid_arguments_are_refused() {
    for args in [
        &["--backend", "reference", "--steps", "0"][..],
        &["--backend", "reference", "--steps", "1000001"],
        &["--backend", "reference", "--steps", "NaN"],
        &["--backend", "unknown"],
        &["--backend", "reference", "up"],
        &["--backend", "reference", "--frame-dir"],
    ] {
        let result = run(args);
        status(&result, 2);
        assert!(
            result.stdout.is_empty(),
            "invalid command emitted a frame record"
        );
    }

    let result = run(["--backend", "reference"]
        .into_iter()
        .chain(std::iter::repeat_n("left", 33)));
    status(&result, 2);
    assert!(result.stdout.is_empty());
}

#[test]
fn existing_export_destinations_are_not_modified() {
    let scratch = Scratch::new();
    let marker = scratch.0.join("frame-0001.ppm");
    std::fs::write(&marker, b"keep this file").expect("write destination marker");
    for destination in [&scratch.0, &marker] {
        let result = Command::new(EXECUTABLE)
            .args(["--backend", "reference", "--frame-dir"])
            .arg(destination)
            .output()
            .expect("run the xenora-session CLI");
        status(&result, 2);
        assert!(result.stdout.is_empty());
        assert_eq!(
            std::fs::read(&marker).expect("previous destination file is intact"),
            b"keep this file"
        );
    }
    assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 1);
}

#[test]
#[ignore = "requires a native Metal device and validation enabled"]
fn default_metal_backend_draws_every_frame_natively() {
    verify_run(
        run(["right", "left", "right", "right"]),
        "metal",
        &["right", "left", "right", "right"],
        "RLRR",
        4,
    );
}
