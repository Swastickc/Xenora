// SPDX-License-Identifier: GPL-3.0-or-later

//! A bounded XInput controller-state surface.
//!
//! Xenora does not yet bridge host controllers into the guest. Claiming that a
//! controller exists would be false, so valid controller slots report the
//! documented device-not-connected result. The exact output layout is reserved
//! for the later bridge; this slice validates indexes and output memory without
//! inventing input state.

use crate::imports::{ImportRegistry, ServiceStatus};
use crate::memory::GuestMemory;

const ERROR_BAD_ARGUMENTS: u32 = 160;
const ERROR_DEVICE_NOT_CONNECTED: u32 = 1167;
const XUSER_MAX_COUNT: u32 = 4;
const XINPUT_STATE_BYTES: usize = 16;
const MODULES: [&str; 3] = ["xinput1_4.dll", "xinput9_1_0.dll", "xinputuap.dll"];

fn get_state(index: u32, output: u64, memory: &GuestMemory) -> u32 {
    if index >= XUSER_MAX_COUNT || output == 0 || !memory.can_write(output, XINPUT_STATE_BYTES) {
        return ERROR_BAD_ARGUMENTS;
    }
    ERROR_DEVICE_NOT_CONNECTED
}

/// Register the state query on the public XInput library names. Vibration,
/// capabilities, keystrokes, headset, battery information and host hotplug
/// bridging remain separate unsupported surfaces.
pub(super) fn register(registry: &mut ImportRegistry) {
    for module in MODULES {
        registry.register_with_status(
            module,
            "XInputGetState",
            ServiceStatus::Partial,
            |args, _, memory| Ok(u64::from(get_state(args.rcx as u32, args.rdx, memory))),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::CpuState;
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::MemoryPermissions;

    fn call(memory: &mut GuestMemory, module: &str, index: u64, output: u64) -> u64 {
        let mut registry = ImportRegistry::new();
        register(&mut registry);
        let mut cpu = CpuState::default();
        cpu.set(crate::cpu::Register::Rcx, index);
        cpu.set(crate::cpu::Register::Rdx, output);
        let descriptor = registry
            .resolve(module, &ImportName::Name("XInputGetState".to_owned()))
            .unwrap();
        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
    }

    fn writable_memory() -> GuestMemory {
        let mut memory = GuestMemory::new();
        memory
            .map_region(0x1000, 0x1000, MemoryPermissions::READ_WRITE)
            .unwrap();
        memory
    }

    #[test]
    fn valid_slots_report_no_connected_controller_without_mutating_output() {
        let mut memory = writable_memory();
        memory.write_bytes(0x1ff0, &[0xaa; 16]).unwrap();
        for module in MODULES {
            for index in 0..4 {
                assert_eq!(
                    call(&mut memory, module, index, 0x1ff0),
                    u64::from(ERROR_DEVICE_NOT_CONNECTED)
                );
            }
        }
        let mut output = [0; 16];
        memory.read_bytes(0x1ff0, &mut output).unwrap();
        assert_eq!(output, [0xaa; 16]);
    }

    #[test]
    fn invalid_indexes_and_output_ranges_are_rejected() {
        let mut memory = writable_memory();
        for (index, output) in [
            (4, 0x1ff0),
            (u64::from(u32::MAX), 0x1ff0),
            (0, 0),
            (0, 0x1ff1),
            (0, 0x5000),
        ] {
            assert_eq!(
                call(&mut memory, "xinput1_4.dll", index, output),
                u64::from(ERROR_BAD_ARGUMENTS)
            );
        }
    }

    #[test]
    fn read_only_output_is_rejected_before_any_mutation() {
        let mut memory = GuestMemory::new();
        memory
            .map_region(0x1000, 0x1000, MemoryPermissions::READ_ONLY)
            .unwrap();
        assert_eq!(
            call(&mut memory, "xinputuap.dll", 0, 0x1ff0),
            u64::from(ERROR_BAD_ARGUMENTS)
        );
    }
}
