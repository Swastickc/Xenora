// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::process::{Command, Output};

const DELAY_IMPORT_FIXTURE: &[u8] = include_bytes!("../../../fixtures/pe/delay-imports.exe");

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

#[test]
fn inspects_sparse_pe_with_distant_headers() {
    const PE_OFFSET: u32 = 1024 * 1024 * 1024;
    const FILE_SIZE: u64 = 4 * 1024 * 1024 * 1024;
    const FIRST_RAW_SECTION: usize = 0x400;

    let directory = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("inspect-sparse-{}", std::process::id()));
    fs::create_dir_all(&directory).expect("create test input directory");
    let path = directory.join("sparse.exe");
    let mut file = fs::File::create(&path).expect("create sparse PE");

    let mut dos_header: [u8; 64] = DELAY_IMPORT_FIXTURE[..64].try_into().unwrap();
    let old_offset = u32::from_le_bytes(dos_header[0x3c..].try_into().unwrap()) as usize;
    dos_header[0x3c..].copy_from_slice(&PE_OFFSET.to_le_bytes());
    file.write_all(&dos_header).expect("write DOS header");
    file.seek(SeekFrom::Start(FIRST_RAW_SECTION as u64))
        .expect("seek to original section data");
    file.write_all(&DELAY_IMPORT_FIXTURE[FIRST_RAW_SECTION..])
        .expect("write original section data");
    file.seek(SeekFrom::Start(u64::from(PE_OFFSET)))
        .expect("seek to relocated PE headers");
    file.write_all(&DELAY_IMPORT_FIXTURE[old_offset..FIRST_RAW_SECTION])
        .expect("write relocated PE headers");
    file.set_len(FILE_SIZE).expect("extend sparse PE");
    assert_eq!(
        file.metadata().expect("read sparse PE length").len(),
        FILE_SIZE
    );
    drop(file);

    let result = inspect(&path);
    fs::remove_dir_all(&directory).expect("remove test inputs");

    assert_eq!(result.status.code(), Some(0), "{:?}", result.stderr);
    assert!(result.stderr.is_empty());
    let report = String::from_utf8(result.stdout).expect("UTF-8 report");
    assert!(
        report.starts_with("PE32+ Binary Triage Report:\n"),
        "{report}"
    );
    for line in [
        "  File Size:         4294967296 bytes",
        "  Machine:           0x8664 (AMD64: true)",
        "  Entry Point RVA:   0x00001000",
        "  Sections (3):",
        "    - .data    RVA: 0x00003000 Size: 0x00000200 Raw: 0x00000200 Perms: [RW-]",
    ] {
        assert!(report.lines().any(|actual| actual == line), "{report}");
    }
}

#[test]
fn rejects_section_data_past_the_end_of_the_file() {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("inspect-truncated-section-{}", std::process::id()));
    fs::create_dir_all(&directory).expect("create test input directory");
    let path = directory.join("truncated.exe");
    // The fixture's final section ends at offset 0xc00, exactly at EOF.
    fs::write(&path, &DELAY_IMPORT_FIXTURE[..0xbff]).expect("write truncated PE");

    let result = inspect(&path);
    fs::remove_dir_all(&directory).expect("remove test inputs");

    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    assert_eq!(
        result.stderr,
        b"inspect failed: invalid or corrupt section headers\n"
    );
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
