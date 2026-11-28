// SPDX-License-Identifier: GPL-3.0-or-later

//! Run the packaged scene through the supervised CLI and check its export.

use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const GUEST_EXPORT: &str = r"C:\package-frames.bin";

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "xenora-package-scene-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create a package export directory");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/pe")
        .join(path)
}

fn root_and_digest() -> (PathBuf, String) {
    let root = fixture("package-scene/xenora-package-scene.exe");
    let digest = format!("{:x}", Sha256::digest(fs::read(&root).unwrap()));
    (root, digest)
}

fn run(root: &Path, provider: &Path, digest: &str, destination: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xenora"))
        .args(["run", "--experimental", "--pe"])
        .arg(root)
        .arg("--package")
        .arg(provider)
        .args(["--sha256", digest, "--export-guest-file", GUEST_EXPORT])
        .arg(destination)
        .arg("--trace")
        .output()
        .expect("run the packaged scene through xenora")
}

fn assert_status(output: &Output, expected: i32) -> &str {
    let stderr = std::str::from_utf8(&output.stderr).expect("UTF-8 diagnostics");
    assert_eq!(
        output.status.code(),
        Some(expected),
        "stdout: {}\nstderr: {stderr}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(output.stdout.is_empty(), "unexpected guest stdout");
    stderr
}

#[test]
fn packaged_scene_exports_the_same_two_frames_on_repeated_runs() {
    let (root, digest) = root_and_digest();
    let provider = fixture("package-scene/xenora_scene.dll");
    let scratch = Scratch::new();
    let first = scratch.0.join("first.bin");
    let second = scratch.0.join("second.bin");

    for destination in [&first, &second] {
        let output = run(&root, &provider, &digest, destination);
        let stderr = assert_status(&output, 0);
        assert!(stderr.contains("policy=sha256"), "{stderr}");
        assert!(stderr.contains("trace package imports="), "{stderr}");
        assert!(stderr.contains("exit=ProcessExit(0)"), "{stderr}");
    }

    let pixels = fs::read(first).expect("read the first scene export");
    assert_eq!(pixels.len(), 96);
    assert_eq!(pixels[..48], [255, 0, 0, 255].repeat(12));
    assert_eq!(pixels[48..], [10, 20, 30, 255].repeat(12));
    assert_eq!(
        fs::read(second).expect("read the second scene export"),
        pixels
    );
}

#[test]
fn rejected_packages_do_not_export_to_the_host() {
    let (root, digest) = root_and_digest();
    let mut wrong_digest = digest.clone();
    let replacement = if digest.starts_with('0') { "1" } else { "0" };
    wrong_digest.replace_range(..1, replacement);
    let scratch = Scratch::new();

    for (label, provider, digest, diagnostic) in [
        (
            "wrong-digest",
            "package-scene/xenora_scene.dll",
            wrong_digest.as_str(),
            "execution denied: the file's SHA-256 is not the one given",
        ),
        (
            "missing-provider",
            "multidll/xenora_math.dll",
            digest.as_str(),
            "missing-module=",
        ),
        (
            "rejected-provider",
            "package-scene/reject/xenora_scene.dll",
            digest.as_str(),
            "DllMain of xenora_scene.dll returned FALSE",
        ),
    ] {
        let destination = scratch.0.join(format!("{label}.bin"));
        let output = run(&root, &fixture(provider), digest, &destination);
        let stderr = assert_status(&output, 125);
        assert!(stderr.contains(diagnostic), "{label}: {stderr}");
        assert!(!stderr.contains("exit=ProcessExit(0)"), "{label}: {stderr}");
        assert!(!destination.exists(), "{label} published an export");
    }
    assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 0);
}
