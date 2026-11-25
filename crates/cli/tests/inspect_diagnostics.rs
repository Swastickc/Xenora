// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn inspect(path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xenora"))
        .arg("inspect")
        .arg(path)
        .output()
        .expect("launch Xenora")
}

#[test]
fn distinguishes_read_errors_from_invalid_pe_bytes() {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("inspect-diagnostics-{}", std::process::id()));
    fs::create_dir(&directory).expect("create test input directory");
    let missing = directory.join("missing.exe");
    let empty = directory.join("empty.exe");
    fs::write(&empty, []).expect("write empty input");

    let cases = [
        (inspect(&missing), "could not read executable: No such file"),
        (
            inspect(&directory),
            "could not read executable: Is a directory",
        ),
        (
            inspect(&empty),
            "executable image is smaller than required headers",
        ),
    ];
    fs::remove_dir_all(&directory).expect("remove test inputs");

    for (result, diagnostic) in cases {
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stdout.is_empty());
        let stderr = String::from_utf8(result.stderr).expect("UTF-8 diagnostic");
        assert!(
            stderr.starts_with(&format!("inspect failed: {diagnostic}")),
            "{stderr}"
        );
        assert!(!stderr.contains(&*directory.to_string_lossy()), "{stderr}");
    }
}

#[test]
fn inspects_the_original_delay_import_fixture() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pe/delay-imports.exe");
    let result = inspect(&fixture);
    assert_eq!(result.status.code(), Some(0));
    assert!(result.stderr.is_empty());
    let report = String::from_utf8(result.stdout).expect("UTF-8 report");
    assert!(
        report.starts_with("PE32+ Binary Triage Report:\n"),
        "{report}"
    );
    assert!(report.contains("Entry Point RVA:   0x00001000"), "{report}");
}

#[cfg(unix)]
#[test]
fn inspects_only_regular_files_and_follows_symlinks() {
    use std::os::unix::fs::symlink;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    fn inspect_bounded(path: &Path) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_xenora"))
            .arg("inspect")
            .arg(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("launch Xenora");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if child.try_wait().expect("poll Xenora").is_some() {
                return child.wait_with_output().expect("collect Xenora output");
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                child.wait().expect("reap stalled Xenora");
                panic!("inspect stalled on {}", path.display());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    let directory = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("inspect-file-types-{}", std::process::id()));
    fs::create_dir_all(&directory).expect("create test input directory");
    let fifo = directory.join("input.fifo");
    let mkfifo = Command::new("mkfifo")
        .current_dir(&directory)
        .arg("input.fifo")
        .status()
        .expect("create FIFO");
    assert!(mkfifo.success());
    let fifo_link = directory.join("fifo-link");
    symlink("input.fifo", &fifo_link).expect("link FIFO");

    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pe/delay-imports.exe");
    let regular_link = directory.join("regular-link.exe");
    symlink(fixture, &regular_link).expect("link original PE fixture");

    for path in [&fifo, &fifo_link] {
        let result = inspect_bounded(path);
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stdout.is_empty());
        let stderr = String::from_utf8(result.stderr).expect("UTF-8 diagnostic");
        assert_eq!(
            stderr,
            "inspect failed: could not read executable: expected a regular file\n"
        );
    }
    let result = inspect_bounded(&regular_link);
    assert_eq!(result.status.code(), Some(0));
    assert!(result.stderr.is_empty());
    assert!(result.stdout.starts_with(b"PE32+ Binary Triage Report:\n"));
    fs::remove_dir_all(&directory).expect("remove test inputs");
}
