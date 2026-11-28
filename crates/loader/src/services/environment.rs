// SPDX-License-Identifier: GPL-3.0-or-later

//! Environment block services. The environment starts empty - no host
//! variable passes through - and `SetEnvironmentVariable` maintains it:
//! queries answer case-insensitively with the documented size semantics,
//! expansion resolves `%name%` with unresolved names copied verbatim, and
//! the block strings come sorted upper-case first. The C runtime's own
//! `_environ` copy does not follow these writes, which is what Windows
//! programs observe: only the CRT's own `_putenv` would update that copy.

use super::{ERROR_INVALID_PARAMETER, KERNEL32, ServicesState};
use crate::cpu::ExitReason;
use crate::imports::{ImportRegistry, ServiceStatus};
use crate::memory::PROCESS_HEAP;
use std::sync::Arc;

const PROCESSENV_API: &str = "api-ms-win-core-processenvironment-l1-1-0.dll";

pub(super) fn register(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    for (symbol, wide) in [
        ("ExpandEnvironmentStringsW", true),
        ("ExpandEnvironmentStringsA", false),
    ] {
        let service_state = Arc::clone(state);
        registry.register(KERNEL32, symbol, move |args, _, memory| {
            let fault = ExitReason::MemoryFault;
            let source = args.get(0, memory).map_err(fault)?;
            let target = args.get(1, memory).map_err(fault)?;
            let capacity = args.get(2, memory).map_err(fault)?;

            // Read the source in the service's width.
            let mut input: Vec<u16> = Vec::new();
            loop {
                let unit = if wide {
                    memory
                        .read_u16(source + 2 * input.len() as u64)
                        .map_err(fault)?
                } else {
                    u16::from(memory.read_u8(source + input.len() as u64).map_err(fault)?)
                };
                if unit == 0 {
                    break;
                }
                input.push(unit);
                if input.len() > 32768 {
                    return Ok(0);
                }
            }

            // Expand each %name% against the process environment. A name
            // that resolves to nothing keeps its percent signs verbatim,
            // the documented behaviour.
            let mut output: Vec<u16> = Vec::new();
            let mut index = 0;
            while index < input.len() {
                if input[index] == u16::from(b'%')
                    && let Some(end) = input[index + 1..]
                        .iter()
                        .position(|&unit| unit == u16::from(b'%'))
                {
                    let close = index + 1 + end;
                    let name: String = input[index + 1..close]
                        .iter()
                        .map(|&unit| char::from_u32(u32::from(unit)).unwrap_or('\u{fffd}'))
                        .collect();
                    if !name.is_empty()
                        && let Some(value) = service_state.environment_variable_value(&name)
                    {
                        output.extend(value.encode_utf16());
                        index = close + 1;
                        continue;
                    }
                }
                output.push(input[index]);
                index += 1;
            }

            let mut bytes: Vec<u8> = Vec::new();
            for unit in &output {
                if wide {
                    bytes.extend_from_slice(&unit.to_le_bytes());
                } else {
                    bytes.push((*unit).min(0xff) as u8);
                }
            }

            // The count of the copy, terminator included, in the service's
            // units: the documentation counts the null in both the success
            // return and the required size.
            let needed = output.len() as u64 + 1;
            if needed > capacity {
                // A small buffer answers the required size; what it holds
                // is undefined, and nothing is written here.
                return Ok(needed);
            }
            bytes.extend_from_slice(if wide { &[0, 0] } else { &[0] });
            memory.write_bytes(target, &bytes).map_err(fault)?;
            Ok(needed)
        });
    }

    // The environment block: the variables as NAME=value strings sorted
    // upper-case first, each terminated, and the block terminator after the
    // last one. Each call hands the caller its own copy from the process
    // heap, because the documentation requires the caller to free it with
    // FreeEnvironmentStringsW. An empty environment is the two terminators.
    let service_state = Arc::clone(state);
    registry.register(KERNEL32, "GetEnvironmentStringsW", move |_, _, memory| {
        let entries = service_state.environment_block_entries();
        let mut bytes: Vec<u8> = Vec::new();
        for (name, value) in &entries {
            for unit in format!("{name}={value}").encode_utf16() {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            bytes.extend_from_slice(&[0, 0]);
        }
        bytes.extend_from_slice(&[0, 0]);
        if entries.is_empty() {
            // An empty environment is one empty string and the block
            // terminator: two null units.
            bytes.extend_from_slice(&[0, 0]);
        }
        let length = bytes.len();
        let target = memory
            .map_heap_allocation(PROCESS_HEAP, length)
            .unwrap_or(0);
        if target == 0 {
            return Ok(0);
        }
        if memory.write_bytes(target, &bytes).is_err() {
            return Ok(0);
        }
        Ok(target)
    });

    // Set and delete variables. The name must be non-empty and free of '=';
    // a null value deletes the variable. The C runtime's own copy does not
    // follow, which is what Windows programs observe.
    for (symbol, wide) in [
        ("SetEnvironmentVariableW", true),
        ("SetEnvironmentVariableA", false),
    ] {
        let service_state = Arc::clone(state);
        registry.register(KERNEL32, symbol, move |args, cpu, memory| {
            let fault = ExitReason::MemoryFault;
            let name_ptr = args.get(0, memory).map_err(fault)?;
            let value_ptr = args.get(1, memory).map_err(fault)?;
            if name_ptr == 0 {
                return service_state.fail(cpu.gs_base, ERROR_INVALID_PARAMETER, 0);
            }
            let name = if wide {
                read_wide(memory, name_ptr)?
            } else {
                read_narrow(memory, name_ptr)?
            };
            if name.is_empty() || name.contains('=') {
                return service_state.fail(cpu.gs_base, ERROR_INVALID_PARAMETER, 0);
            }
            let value = if value_ptr == 0 {
                None
            } else if wide {
                Some(read_wide(memory, value_ptr)?)
            } else {
                Some(read_narrow(memory, value_ptr)?)
            };
            service_state.set_environment_variable(&name, value.as_deref());
            Ok(1)
        });
    }

    let service_state = Arc::clone(state);
    registry.register(
        KERNEL32,
        "FreeEnvironmentStringsW",
        move |args, cpu, memory| {
            let block = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
            if block != 0 && memory.release_heap_allocation(PROCESS_HEAP, block) {
                Ok(1)
            } else {
                service_state.fail(cpu.gs_base, ERROR_INVALID_PARAMETER, 0)
            }
        },
    );

    for symbol in [
        "ExpandEnvironmentStringsA",
        "ExpandEnvironmentStringsW",
        "FreeEnvironmentStringsW",
        "GetEnvironmentStringsW",
        "SetEnvironmentVariableA",
        "SetEnvironmentVariableW",
    ] {
        registry.alias(PROCESSENV_API, KERNEL32, symbol);
        registry.set_status(KERNEL32, symbol, ServiceStatus::Partial);
    }
}

/// Read a null-terminated wide string as text.
fn read_wide(
    memory: &crate::memory::GuestMemory,
    address: u64,
) -> Result<String, crate::cpu::ExitReason> {
    let mut units = Vec::new();
    loop {
        let unit = memory
            .read_u16(address + 2 * units.len() as u64)
            .map_err(crate::cpu::ExitReason::MemoryFault)?;
        if unit == 0 {
            break;
        }
        units.push(unit);
        if units.len() > 32768 {
            break;
        }
    }
    Ok(String::from_utf16_lossy(&units))
}

/// Read a null-terminated narrow string as text; the console code page is
/// UTF-8, so narrow text is UTF-8.
fn read_narrow(
    memory: &crate::memory::GuestMemory,
    address: u64,
) -> Result<String, crate::cpu::ExitReason> {
    let mut bytes = Vec::new();
    loop {
        let byte = memory
            .read_u8(address + bytes.len() as u64)
            .map_err(crate::cpu::ExitReason::MemoryFault)?;
        if byte == 0 {
            break;
        }
        bytes.push(byte);
        if bytes.len() > 32768 {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
