// SPDX-License-Identifier: GPL-3.0-or-later

//! Process termination.

use super::{KERNEL32, ServicesState};
use crate::cpu::ExitReason;
use crate::imports::{ImportRegistry, ServiceStatus};
use std::sync::Arc;

const PROCESS_THREADS_API: &str = "api-ms-win-core-processthreads-l1-1-0.dll";
const UCRT: &str = "ucrtbase.dll";
const CRT_RUNTIME_API: &str = "api-ms-win-crt-runtime-l1-1-0.dll";
/// The exit code `abort` ends the process with (Microsoft Learn, "abort").
const ABORT_STATUS: u32 = 3;

pub(super) fn register(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    // ExitProcess does not return (processthreadsapi.h). It stops the guest
    // with its status and never ends the host. It is partial: there are no
    // other threads or loaded DLLs to notify.
    let s = Arc::clone(state);
    registry.register(KERNEL32, "ExitProcess", move |args, _, _| {
        // The status is a UINT, the low half of the first argument.
        Err(ExitReason::ProcessExit(s.exit_process(args.rcx as u32)))
    });
    registry.alias(PROCESS_THREADS_API, KERNEL32, "ExitProcess");
    registry.set_status(KERNEL32, "ExitProcess", ServiceStatus::Partial);

    // _exit ends the process without calling the functions registered with
    // atexit (Microsoft Learn, "exit, _Exit, _exit"). `exit`, which does call
    // them, is beside `_cexit` in crt_callbacks.rs.
    let s = Arc::clone(state);
    registry.register(UCRT, "_exit", move |args, _, _| {
        // The status is an int, the low half of the first argument.
        Err(ExitReason::ProcessExit(s.exit_process(args.rcx as u32)))
    });

    // abort ends the process with exit code 3 and calls no registered
    // function. It is partial: it prints no message, and it does not call a
    // SIGABRT handler, which the documentation says it does when one is set.
    let s = Arc::clone(state);
    registry.register(UCRT, "abort", move |_, _, _| {
        Err(ExitReason::ProcessExit(s.exit_process(ABORT_STATUS)))
    });

    for symbol in ["_exit", "abort"] {
        registry.alias(CRT_RUNTIME_API, UCRT, symbol);
        registry.set_status(UCRT, symbol, ServiceStatus::Partial);
    }
}
