// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const SCENE_PATH: &str = r"C:\hosted-scene.txt";
static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        loop {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "xenora graphics backend {} {sequence}",
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

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn runner(image: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xenora"));
    command.args(["run", "--experimental", "--pe"]).arg(image);
    command
}

fn assert_exit(output: &Output, code: i32) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_graphics_trace(output: &Output, expected: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let graphics: Vec<_> = stderr
        .lines()
        .filter(|line| line.starts_with("trace graphics "))
        .collect();
    assert_eq!(graphics, [expected], "{stderr}");
}

fn build_hosted_guest(scratch: &Scratch) -> PathBuf {
    let output_dir = scratch.0.join("hosted");
    let build = Command::new("sh")
        .arg(root().join("tests/compat_fixtures/build-hosted-graphics.sh"))
        .arg(&output_dir)
        .output()
        .expect("build the original hosted graphics guest");
    assert_exit(&build, 0);
    output_dir.join("hosted-graphics-O2.exe")
}

fn run_hosted_guest(image: &Path, export: &Path, backend: Option<&str>) -> Output {
    let mut command = runner(image);
    command
        .args(["--trace", "--max-steps", "1000000", "--wall-seconds", "15"])
        .args(["--export-guest-file", SCENE_PATH])
        .arg(export);
    if let Some(backend) = backend {
        command.args(["--graphics-backend", backend]);
    }
    command
        .args(["--", SCENE_PATH])
        .output()
        .expect("run the original hosted graphics guest")
}

#[test]
fn default_and_explicit_reference_render_the_same_hosted_guest() {
    let scratch = Scratch::new();
    let image = build_hosted_guest(&scratch);
    for (backend, name) in [(None, "default"), (Some("reference"), "explicit")] {
        let export = scratch.0.join(format!("{name}-scene.txt"));
        let output = run_hosted_guest(&image, &export, backend);
        assert_exit(&output, 0);
        assert_eq!(output.stdout, b"hosted graphics: two frames\r\n");
        if backend.is_some() {
            assert_graphics_trace(
                &output,
                "trace graphics backend=reference frames=2 last_draws=1 width=8 height=8",
            );
        } else {
            assert!(!String::from_utf8_lossy(&output.stderr).contains("trace graphics "));
        }
        assert_eq!(fs::read(export).unwrap(), b"inside\r\noutside\r\n");
    }
}

#[test]
fn packaged_program_with_no_draw_reports_no_presentation() {
    let fixtures = root().join("fixtures/pe/multidll");
    let output = runner(&fixtures.join("multidll-consumer.exe"))
        .args(["--trace", "--graphics-backend", "reference", "--package"])
        .arg(fixtures.join("xenora_chain.dll"))
        .arg("--package")
        .arg(fixtures.join("xenora_math.dll"))
        .output()
        .expect("run the original multi-DLL guest");
    assert_exit(&output, 70);
    assert!(output.stdout.is_empty());
    assert_graphics_trace(
        &output,
        "trace graphics backend=none frames=0 last_draws=0 width=0 height=0",
    );
}

#[test]
fn graphics_backend_rejects_missing_unknown_and_duplicate_values() {
    let image = root().join("fixtures/pe/guest-fence.exe");
    for flags in [
        &["--graphics-backend"][..],
        &["--graphics-backend", "invalid"],
        &[
            "--graphics-backend",
            "reference",
            "--graphics-backend",
            "metal",
        ],
    ] {
        let output = runner(&image).args(flags).output().expect("run xenora");
        assert_exit(&output, 2);
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("--graphics-backend"));
    }
}

#[test]
fn graphics_backend_option_is_refused_for_the_trusted_fixture() {
    let image = root().join("fixtures/pe/trusted-runner.exe");
    for backend in ["reference", "metal"] {
        let output = runner(&image)
            .args(["--graphics-backend", backend])
            .output()
            .expect("run xenora");
        assert_exit(&output, 2);
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("--graphics-backend"));
    }
}

#[test]
#[ignore = "requires a native Metal device and validation enabled"]
fn metal_renders_and_verifies_two_hosted_frames() {
    let scratch = Scratch::new();
    let image = build_hosted_guest(&scratch);
    let export = scratch.0.join("metal-scene.txt");
    let output = run_hosted_guest(&image, &export, Some("metal"));
    assert_exit(&output, 0);
    assert_eq!(output.stdout, b"hosted graphics: two frames\r\n");
    assert_graphics_trace(
        &output,
        "trace graphics backend=metal frames=2 last_draws=1 width=8 height=8",
    );
    assert_eq!(fs::read(export).unwrap(), b"inside\r\noutside\r\n");
}

#[test]
#[ignore = "requires a native Metal device and validation enabled"]
fn metal_runs_the_existing_two_queue_guest_fence_fixture() {
    let image = root().join("fixtures/pe/guest-fence.exe");
    let output = runner(&image)
        .args([
            "--graphics-backend",
            "metal",
            "--trace",
            "--max-steps",
            "1000000",
            "--wall-seconds",
            "15",
        ])
        .output()
        .expect("run the guest fence fixture");
    assert_exit(&output, 0);
    assert!(output.stdout.is_empty());
    assert_graphics_trace(
        &output,
        "trace graphics backend=metal frames=2 last_draws=1 width=4 height=3",
    );
}
