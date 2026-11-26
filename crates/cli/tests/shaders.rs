// SPDX-License-Identifier: GPL-3.0-or-later

use std::process::{Command, Output};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/rdna2/add.bin");

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
