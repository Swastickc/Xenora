// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/rdna2/add.bin");

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "xenora shader inspection {} {}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create shader test directory: {error}"),
            }
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn shaders(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xenora"))
        .arg("shaders")
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn documented_shader_commands_keep_their_output_formats() {
    for (args, output_start) in [
        (&["coverage"][..], "family"),
        (&["coverage", "--json"][..], "{\"architecture\":\"gfx1030\""),
        (&["inspect", FIXTURE][..], "0x0000"),
    ] {
        let output = shaders(args);
        assert!(output.status.success(), "{args:?}: {:?}", output.stderr);
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.starts_with(output_start), "{args:?}: {stdout}");
    }
}

#[test]
fn malformed_shader_commands_return_usage() {
    for args in [
        &[][..],
        &["coverage", "--jsno"][..],
        &["coverage", "--json", "--json"][..],
        &["inspect"][..],
        &["inspect", FIXTURE, "unexpected"][..],
    ] {
        let output = shaders(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {:?}", output.stdout);
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.starts_with("usage: xenora shaders "), "{stderr}");
    }
}

#[test]
fn shader_inspection_accepts_a_regular_file_symlink() {
    let scratch = Scratch::new();
    let link = scratch.0.join("shader link");
    symlink(FIXTURE, &link).unwrap();
    let expected = shaders(&["inspect", FIXTURE]);
    assert!(expected.status.success());
    let actual = shaders(&["inspect", link.to_str().unwrap()]);
    assert!(actual.status.success());
    assert_eq!(actual.stdout, expected.stdout);
    assert_eq!(actual.stderr, expected.stderr);
}

#[test]
fn shader_inspection_enforces_the_exact_byte_limit() {
    const LIMIT: u64 = 1024 * 1024;
    let scratch = Scratch::new();
    let path = scratch.0.join("bounded shader");
    let mut file = File::create(&path).unwrap();
    // One ordinary SOPP instruction, then the unassigned opcode 127.
    // The latter stops decoding before the large sparse tail.
    for word in [0xbf81_0000_u32, 0xbfff_0000] {
        file.write_all(&word.to_le_bytes()).unwrap();
    }
    let args = ["inspect", path.to_str().unwrap()];
    let malformed = shaders(&args);
    assert_eq!(malformed.status.code(), Some(1));
    assert!(malformed.stderr.is_empty());
    let text = std::str::from_utf8(&malformed.stdout).unwrap();
    assert!(text.starts_with("0x0000"));
    assert!(text.contains("s_endpgm"));
    assert!(text.contains("0x0004  decode failure:"));

    file.set_len(LIMIT).unwrap();
    let at_limit = shaders(&args);
    assert_eq!(at_limit.status.code(), Some(1));
    assert_eq!(at_limit.stdout, malformed.stdout);
    assert_eq!(at_limit.stderr, malformed.stderr);

    file.set_len(LIMIT + 1).unwrap();
    let over_limit = shaders(&args);
    assert_eq!(over_limit.status.code(), Some(1));
    assert!(over_limit.stdout.is_empty());
    let error = std::str::from_utf8(&over_limit.stderr).unwrap();
    assert!(
        error.contains("shader file exceeds the 1 MiB inspection limit"),
        "{error}"
    );
}

#[test]
fn shader_inspection_accepts_an_empty_regular_file() {
    let scratch = Scratch::new();
    let path = scratch.0.join("empty shader");
    fs::write(&path, []).unwrap();
    let output = shaders(&["inspect", path.to_str().unwrap()]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let text = std::str::from_utf8(&output.stdout).unwrap();
    assert!(
        text.starts_with("0 instructions inspected against "),
        "{text}"
    );
    assert!(
        text.ends_with("gfx1030 forms; 0 without semantic decoding\n"),
        "{text}"
    );
}

fn inspect_before_deadline(path: &Path) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_xenora"))
        .args(["shaders", "inspect"])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().unwrap(),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "shader inspection did not promptly refuse {}: {result:?}",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn shader_inspection_refuses_nonregular_files_without_waiting() {
    let scratch = Scratch::new();
    let fifo = scratch.0.join("no-writer fifo");
    let link = scratch.0.join("fifo link");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    symlink(&fifo, &link).unwrap();

    for path in [&fifo, &link, &scratch.0] {
        let output = inspect_before_deadline(path);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let error = std::str::from_utf8(&output.stderr).unwrap();
        assert!(
            error.contains("expected a regular shader file"),
            "{}: {error}",
            path.display()
        );
    }
}
