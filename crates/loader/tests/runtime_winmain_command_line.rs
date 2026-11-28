// SPDX-License-Identifier: GPL-3.0-or-later

//! PE-bound checks for the narrow command line passed to WinMain.

mod support;

use support::Guest;
use xenora_loader::services::ProgramArgumentsError;

const WINMAIN: &str = "_get_narrow_winmain_command_line";
const COMMAND_LINE: &str = "GetCommandLineA";
const API_SET: &[(&str, &str)] = &[
    ("api-ms-win-crt-runtime-l1-1-0.dll", WINMAIN),
    ("kernel32.dll", COMMAND_LINE),
];
const UCRT: &[(&str, &str)] = &[("ucrtbase.dll", WINMAIN), ("kernel32.dll", COMMAND_LINE)];

fn read(guest: &Guest, address: u64, length: usize) -> Vec<u8> {
    let mut bytes = vec![0; length];
    guest.memory.read_bytes(address, &mut bytes).unwrap();
    bytes
}

fn assert_command_line(arguments: &[&str], complete_bytes: &[u8], tail_bytes: &[u8]) {
    for imports in [API_SET, UCRT] {
        for (first, second) in [(WINMAIN, COMMAND_LINE), (COMMAND_LINE, WINMAIN)] {
            let mut guest = Guest::new(imports, &[]);
            guest.state.set_program_arguments(arguments).unwrap();
            let first_address = guest.call(first, &[]);
            assert_ne!(first_address, 0);
            assert_eq!(
                guest.state.set_program_arguments(&["too late"]),
                Err(ProgramArgumentsError::AlreadyConfigured)
            );
            let second_address = guest.call(second, &[]);
            let (complete, tail) = if first == WINMAIN {
                (second_address, first_address)
            } else {
                (first_address, second_address)
            };

            assert_eq!(read(&guest, complete, complete_bytes.len()), complete_bytes);
            assert_eq!(read(&guest, tail, tail_bytes.len()), tail_bytes);
            assert_eq!(
                tail - complete,
                (complete_bytes.len() - tail_bytes.len()) as u64
            );
            assert_eq!(guest.call(COMMAND_LINE, &[]), complete);
            assert_eq!(guest.call(WINMAIN, &[]), tail);
        }
    }
}

#[test]
fn winmain_tail_preserves_the_prepared_argument_bytes() {
    assert_command_line(
        &[
            r"C:\Program Files\café.exe",
            "two words",
            "",
            r#"say "hi""#,
            "folder \\",
        ],
        b"\"C:\\Program Files\\caf\xc3\xa9.exe\" \"two words\" \"\" \"say \\\"hi\\\"\" \"folder \\\\\"\0",
        b"\"two words\" \"\" \"say \\\"hi\\\"\" \"folder \\\\\"\0",
    );
    assert_command_line(&["app.exe", "é"], b"app.exe \xc3\xa9\0", b"\xc3\xa9\0");
}

#[test]
fn winmain_tail_handles_an_empty_program_token_or_argument_list() {
    assert_command_line(&[], b"\0", b"\0");
    assert_command_line(&[""], b"\"\"\0", b"\0");
    assert_command_line(&["", "alpha"], b"\"\" alpha\0", b"alpha\0");
    assert_command_line(&["app.exe"], b"app.exe\0", b"\0");
}
