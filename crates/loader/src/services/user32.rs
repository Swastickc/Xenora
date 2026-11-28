// SPDX-License-Identifier: GPL-3.0-or-later

//! The USER32 surface of a windowless process. Guest threads can exchange
//! posted messages, but there are no windows, window procedures or host input.

use super::{ERROR_INVALID_PARAMETER, ERROR_NOACCESS, ServicesState};
use crate::cpu::ExitReason;
use crate::imports::{ImportRegistry, ServiceStatus};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

const MODULE: &str = "user32.dll";
const ERROR_INVALID_THREAD_ID: u32 = 1444;
const ERROR_NOT_ENOUGH_QUOTA: u32 = 1816;
const MAX_POSTED_MESSAGES: usize = 10_000;
const MSG_SIZE: usize = 48;
const WM_QUIT: u32 = 0x0012;
const PM_REMOVE: u32 = 0x0001;
const PM_NOYIELD: u32 = 0x0002;

struct PostedMessage {
    message: u32,
    wparam: u64,
    lparam: u64,
    time: u32,
}

impl PostedMessage {
    fn encode(&self) -> [u8; MSG_SIZE] {
        // The x64 MSG has an eight-byte HWND, alignment padding after the
        // message, two pointer-sized parameters, a timestamp and a POINT.
        // Thread messages have no HWND; the windowless runtime has no cursor.
        let mut bytes = [0; MSG_SIZE];
        bytes[8..12].copy_from_slice(&self.message.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.wparam.to_le_bytes());
        bytes[24..32].copy_from_slice(&self.lparam.to_le_bytes());
        bytes[32..36].copy_from_slice(&self.time.to_le_bytes());
        bytes
    }

    fn matches(&self, minimum: u32, maximum: u32) -> bool {
        self.message == WM_QUIT
            || (minimum == 0 && maximum == 0)
            || (minimum..=maximum).contains(&self.message)
    }
}

type MessageQueues = BTreeMap<u32, VecDeque<PostedMessage>>;

fn prune_message_queues(state: &ServicesState, queues: &mut MessageQueues) {
    let threads = state.threads.lock().expect("guest thread table");
    if state.hooks.is_attached() {
        queues.retain(|id, _| threads.threads.contains_key(id));
    } else {
        queues.retain(|id, _| *id == state.thread_id);
    }
}

fn register_message_queues(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    let queues = Arc::new(Mutex::new(MessageQueues::new()));
    for symbol in ["PeekMessageA", "PeekMessageW"] {
        let queues = Arc::clone(&queues);
        let state = Arc::clone(state);
        registry.register_with_status(
            MODULE,
            symbol,
            ServiceStatus::Partial,
            move |args, cpu, memory| {
                // NULL accepts messages from any window and the thread itself;
                // HWND(-1) accepts thread messages only. There are no windows.
                if !matches!(args.rdx, 0 | u64::MAX) {
                    return Ok(0);
                }
                let minimum = args.r8 as u32;
                let maximum = args.r9 as u32;
                let flags = args.get(4, memory).map_err(ExitReason::MemoryFault)? as u32;
                let teb = cpu.gs_base;
                if minimum > u16::MAX as u32
                    || maximum > u16::MAX as u32
                    || flags & !(PM_REMOVE | PM_NOYIELD) != 0
                {
                    return state.fail(teb, ERROR_INVALID_PARAMETER, 0);
                }

                let id = state.thread_id_of(teb);
                let mut queues = queues.lock().expect("guest message queues");
                prune_message_queues(&state, &mut queues);
                let queue = queues.entry(id).or_default();
                let Some(index) = queue
                    .iter()
                    .position(|message| message.matches(minimum, maximum))
                else {
                    return Ok(0);
                };
                let output = args.rcx;
                if output == 0
                    || !memory.can_write(output, MSG_SIZE)
                    || memory.write_bytes(output, &queue[index].encode()).is_err()
                {
                    return state.fail(teb, ERROR_NOACCESS, 0);
                }
                if flags & PM_REMOVE != 0 {
                    queue.remove(index);
                }
                Ok(1)
            },
        );
    }

    for symbol in ["PostThreadMessageA", "PostThreadMessageW"] {
        let queues = Arc::clone(&queues);
        let state = Arc::clone(state);
        registry.register_with_status(
            MODULE,
            symbol,
            ServiceStatus::Partial,
            move |args, cpu, _| {
                let mut queues = queues.lock().expect("guest message queues");
                prune_message_queues(&state, &mut queues);
                let Some(queue) = queues.get_mut(&(args.rcx as u32)) else {
                    return state.fail(cpu.gs_base, ERROR_INVALID_THREAD_ID, 0);
                };
                if queue.len() >= MAX_POSTED_MESSAGES {
                    return state.fail(cpu.gs_base, ERROR_NOT_ENOUGH_QUOTA, 0);
                }
                queue.push_back(PostedMessage {
                    message: args.rdx as u32,
                    wparam: args.r8,
                    lparam: args.r9,
                    time: state.uptime_millis() as u32,
                });
                Ok(1)
            },
        );
    }
}

pub(super) fn register(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    register_message_queues(registry, state);

    // Posted thread messages have no window procedure. No keyboard input
    // exists to translate into character messages.
    registry.register_with_status(
        MODULE,
        "TranslateMessage",
        ServiceStatus::Partial,
        |_, _, _| Ok(0),
    );
    registry.register_with_status(
        MODULE,
        "DispatchMessageA",
        ServiceStatus::Partial,
        |_, _, _| Ok(0),
    );

    // This subset of MsgWaitForMultipleObjects observes kernel objects only.
    // It does not wake for posted thread messages; guests poll them with PeekMessage.
    let wait_state = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "MsgWaitForMultipleObjects",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            use super::events::{WAIT_FAILED, resume_wait, wait_for_objects};
            use super::{ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED};
            use crate::cpu::ExitReason;
            if let Some(result) = resume_wait(&wait_state, cpu.gs_base) {
                return result;
            }
            let count = args.rcx as u32;
            let address = args.rdx;
            let timeout = args.r8 as u32;
            let _wake_mask = args.r9 as u32;
            let flags = args.get(4, memory).map_err(ExitReason::MemoryFault)? as u32;
            const MWMO_WAITALL: u32 = 1;
            const MWMO_ALERTABLE: u32 = 2;
            const MWMO_INPUTAVAILABLE: u32 = 4;
            if flags & !(MWMO_WAITALL | MWMO_ALERTABLE | MWMO_INPUTAVAILABLE) != 0 {
                return wait_state.fail(cpu.gs_base, ERROR_INVALID_PARAMETER, WAIT_FAILED);
            }
            if flags & MWMO_ALERTABLE != 0 {
                return wait_state.fail(cpu.gs_base, ERROR_NOT_SUPPORTED, WAIT_FAILED);
            }
            if !(1..=64).contains(&count) || address == 0 {
                return wait_state.fail(cpu.gs_base, ERROR_INVALID_PARAMETER, WAIT_FAILED);
            }
            let mut handles = [0; 64];
            let handles = &mut handles[..count as usize];
            for (index, handle) in handles.iter_mut().enumerate() {
                let slot = address
                    .checked_add(index as u64 * 8)
                    .ok_or(ExitReason::MemoryFault(
                        crate::memory::MemoryError::Overflow,
                    ))?;
                *handle = memory.read_u64(slot).map_err(ExitReason::MemoryFault)?;
            }
            wait_for_objects(
                &wait_state,
                cpu.gs_base,
                handles,
                flags & MWMO_WAITALL != 0,
                timeout,
            )
        },
    );

    // Every window handle is invalid in a windowless machine: invisible,
    // not iconic, and ShowWindow reports no previous visibility.
    registry.register_with_status(
        MODULE,
        "IsWindowVisible",
        ServiceStatus::Partial,
        |_, _, _| Ok(0),
    );
    registry.register_with_status(MODULE, "IsIconic", ServiceStatus::Partial, |_, _, _| Ok(0));
    registry.register_with_status(MODULE, "ShowWindow", ServiceStatus::Partial, |_, _, _| {
        Ok(0)
    });
}

#[cfg(test)]
#[test]
fn a_new_thread_message_queue_is_empty() {
    use crate::cpu::{CpuState, Register};
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::{GuestMemory, MemoryPermissions};
    use crate::services::MockClock;

    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    register(&mut registry, &state);
    let mut memory = GuestMemory::new();
    memory
        .map_region(0x1000, 8192, MemoryPermissions::READ_WRITE)
        .unwrap();
    for (symbol, expected) in [
        ("PeekMessageA", 0),
        ("TranslateMessage", 0),
        ("DispatchMessageA", 0),
        ("IsWindowVisible", 0),
        ("IsIconic", 0),
        ("ShowWindow", 0),
    ] {
        let mut cpu = CpuState::default();
        if symbol == "PeekMessageA" {
            cpu.set(Register::Rcx, 0x1300);
            cpu.set(Register::Rsp, 0x2000);
            memory.write_u64(0x2028, 0).unwrap();
        }
        let descriptor = registry
            .resolve(MODULE, &ImportName::Name(symbol.to_owned()))
            .unwrap();
        assert_eq!(
            (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, &mut memory).unwrap(),
            expected,
            "{symbol} answers honestly for a windowless machine"
        );
    }

    // MsgWaitForMultipleObjects: invalid counts and handles fail honestly,
    // and the wait machinery serves the object half of the contract.
    {
        let wait = registry
            .resolve(
                MODULE,
                &ImportName::Name("MsgWaitForMultipleObjects".to_owned()),
            )
            .unwrap();
        let call_wait = |memory: &mut GuestMemory, args: &[u64]| {
            let mut cpu = CpuState::default();
            for (reg, &value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
                .into_iter()
                .zip(args)
            {
                cpu.set(reg, value);
            }
            cpu.set(Register::Rsp, 0x2000);
            for (index, &value) in args.iter().enumerate().skip(4) {
                memory
                    .write_u64(0x2000 + 0x28 + (index as u64 - 4) * 8, value)
                    .unwrap();
            }
            (wait.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
        };
        memory.write_u64(0x1300, 0xDEAD).unwrap();
        // A count of zero or beyond 64 is invalid, whatever the handles say.
        assert_eq!(
            call_wait(&mut memory, &[0, 0x1300, 0, 0, 0]),
            u32::MAX as u64
        );
        assert_eq!(
            call_wait(&mut memory, &[65, 0x1300, 0, 0, 0]),
            u32::MAX as u64
        );
        // A handle the kernel does not know is WAIT_FAILED.
        assert_eq!(
            call_wait(&mut memory, &[1, 0x1300, 0, 0, 0]),
            u32::MAX as u64
        );
        // Unknown flags are invalid; the alertable form is honestly refused.
        assert_eq!(
            call_wait(&mut memory, &[1, 0x1300, 0, 0, 8]),
            u32::MAX as u64
        );
        assert_eq!(
            call_wait(&mut memory, &[1, 0x1300, 0, 0, 2]),
            u32::MAX as u64
        );
    }
}
