// SPDX-License-Identifier: GPL-3.0-or-later

//! Direct CRT invalid-parameter dispatch through a synthetic PE import table.
//! Microsoft Learn's "Invalid parameter functions" describes the empty
//! diagnostic arguments, global handler and termination paths.

mod support;

use support::*;
use xenora_loader::cpu::ExitReason;

const RUNTIME: &str = "api-ms-win-crt-runtime-l1-1-0.dll";
const NOINFO: &str = "_invalid_parameter_noinfo";
const NORETURN: &str = "_invalid_parameter_noinfo_noreturn";
const SET_HANDLER: &str = "_set_invalid_parameter_handler";
const EXIT_PROCESS: &str = "ExitProcess";
const EXIT_INDEX: usize = 3;
const IMPORTS: &[(&str, &str)] = &[
    (RUNTIME, NOINFO),
    (RUNTIME, NORETURN),
    (RUNTIME, SET_HANDLER),
    ("kernel32.dll", EXIT_PROCESS),
];

const INVALID_C_RUNTIME_PARAMETER: u32 = 0xc000_0417;
const HANDLER_EXIT: u32 = 0x1234_5678;
const ARGUMENTS: u64 = SCRATCH + 8;
const CALL_COUNT: u64 = ARGUMENTS + 40;
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;

fn recording_handler() -> Vec<u8> {
    let mut code = vec![0x48, 0xb8]; // mov rax, ARGUMENTS
    code.extend_from_slice(&ARGUMENTS.to_le_bytes());
    code.extend_from_slice(&[
        0x48, 0x89, 0x08, // mov [rax], rcx
        0x48, 0x89, 0x50, 0x08, // mov [rax+8], rdx
        0x4c, 0x89, 0x40, 0x10, // mov [rax+16], r8
        0x4c, 0x89, 0x48, 0x18, // mov [rax+24], r9
        0x48, 0x8b, 0x54, 0x24, 0x28, // mov rdx, [rsp+40]
        0x48, 0x89, 0x50, 0x20, // mov [rax+32], rdx
        0x48, 0xff, 0x40, 0x28, // inc qword ptr [rax+40]
        0xc3, // ret
    ]);
    code
}

#[test]
fn invalid_parameter_dispatch_passes_five_zero_arguments_to_the_global_handler() {
    for (symbol, expected_exit, expected_status) in [
        (NOINFO, ExitReason::Halt, None),
        (
            NORETURN,
            ExitReason::ProcessExit(INVALID_C_RUNTIME_PARAMETER),
            Some(INVALID_C_RUNTIME_PARAMETER),
        ),
    ] {
        let mut guest = Guest::new(IMPORTS, &[recording_handler()]);
        assert_eq!(guest.call(SET_HANDLER, &[routine(0)]), 0);
        guest.memory.write_bytes(SCRATCH, &[0xa5; 64]).unwrap();
        guest.memory.write_u64(CALL_COUNT, 0).unwrap();
        // Poison unused stack bytes so an unwritten fifth argument fails.
        guest.memory.write_bytes(STACK, &[0xa5; 0x800]).unwrap();

        assert_eq!(guest.run(symbol, &[CANARY; 5]).1, expected_exit, "{symbol}");
        for offset in (0..40).step_by(8) {
            assert_eq!(guest.u64_at(ARGUMENTS + offset), 0, "{symbol}: {offset}");
        }
        assert_eq!(guest.u64_at(CALL_COUNT), 1, "{symbol}");
        assert_eq!(guest.u64_at(SCRATCH), CANARY, "{symbol}");
        assert_eq!(guest.u64_at(CALL_COUNT + 8), CANARY, "{symbol}");
        assert_eq!(guest.state.exit_status(), expected_status, "{symbol}");
    }
}

#[test]
fn invalid_parameter_dispatch_without_a_handler_terminates_the_process() {
    for symbol in [NOINFO, NORETURN] {
        let mut guest = Guest::new(IMPORTS, &[]);
        assert_eq!(
            guest.run(symbol, &[]).1,
            ExitReason::ProcessExit(INVALID_C_RUNTIME_PARAMETER),
            "{symbol}"
        );
        assert_eq!(guest.state.exit_status(), Some(INVALID_C_RUNTIME_PARAMETER));
    }
}

#[test]
fn invalid_parameter_dispatch_propagates_exit_process_from_the_handler() {
    let mut handler = vec![0xb9]; // mov ecx, HANDLER_EXIT
    handler.extend_from_slice(&HANDLER_EXIT.to_le_bytes());
    handler.extend_from_slice(&[0x48, 0xb8]); // mov rax, ExitProcess IAT slot
    handler.extend_from_slice(&import_slot(IMPORTS, EXIT_INDEX).to_le_bytes());
    handler.extend_from_slice(&[
        0x48, 0x83, 0xec, 0x28, // sub rsp, 40 (home space and alignment)
        0xff, 0x10, // call qword ptr [rax]
        0xf4, // hlt if ExitProcess returns
    ]);

    for symbol in [NOINFO, NORETURN] {
        let mut guest = Guest::new(IMPORTS, std::slice::from_ref(&handler));
        assert_eq!(guest.call(SET_HANDLER, &[routine(0)]), 0);
        assert_eq!(
            guest.run(symbol, &[]).1,
            ExitReason::ProcessExit(HANDLER_EXIT),
            "{symbol}"
        );
        assert_eq!(guest.state.exit_status(), Some(HANDLER_EXIT));
    }
}
