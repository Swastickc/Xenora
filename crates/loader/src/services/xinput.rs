// SPDX-License-Identifier: GPL-3.0-or-later

//! A bounded XInput controller-state surface.
//!
//! Hosts may supply explicit controller snapshots to [`ServicesState`]. This
//! module only serializes those snapshots in the public XInput layout; it does
//! not detect physical controllers or hotplugging by itself.

use super::ServicesState;
use crate::imports::{ImportRegistry, ServiceStatus};
use crate::memory::GuestMemory;
use std::sync::{Arc, Mutex};

const ERROR_BAD_ARGUMENTS: u32 = 160;
const ERROR_DEVICE_NOT_CONNECTED: u32 = 1167;
const XUSER_MAX_COUNT: u32 = 4;
const XINPUT_STATE_BYTES: usize = 16;
const XINPUT_CAPABILITIES_BYTES: usize = 20;
const XINPUT_FLAG_GAMEPAD: u32 = 0x0001;
const XINPUT_GAMEPAD_BUTTON_MASK: u16 = 0xf3ff;
const XINPUT_DEVTYPE_GAMEPAD: u8 = 0x01;
const XINPUT_DEVSUBTYPE_GAMEPAD: u8 = 0x01;
const MODULES: [&str; 3] = ["xinput1_4.dll", "xinput9_1_0.dll", "xinputuap.dll"];

/// One host-supplied controller snapshot in documented XInput units.
///
/// `buttons` uses the public XInput bit assignments, triggers are 0 through
/// 255 and thumb axes are signed values from -32768 through 32767. No deadzone
/// processing is applied here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct XInputControllerState {
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub left_x: i16,
    pub left_y: i16,
    pub right_x: i16,
    pub right_y: i16,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Slot {
    connected: bool,
    packet_number: u32,
    state: XInputControllerState,
}

pub(crate) type Slots = [Slot; 4];

impl Slot {
    fn set(&mut self, next: Option<XInputControllerState>) {
        match next {
            None => self.connected = false,
            Some(state) => {
                if !self.connected || self.state != state {
                    self.packet_number = self.packet_number.wrapping_add(1);
                    self.state = state;
                }
                self.connected = true;
            }
        }
    }

    fn get(&self) -> Option<(u32, XInputControllerState)> {
        self.connected.then_some((self.packet_number, self.state))
    }
}

impl ServicesState {
    /// Replace a user slot's explicit controller snapshot. `None` disconnects
    /// the slot. Returns false for an index outside the documented 0..4 range.
    /// Physical controller discovery is the host's responsibility.
    pub fn set_xinput_controller(&self, index: u32, state: Option<XInputControllerState>) -> bool {
        if index >= XUSER_MAX_COUNT {
            return false;
        }
        self.xinput.lock().unwrap()[index as usize].set(state);
        true
    }
}

fn serialize(packet: u32, state: XInputControllerState) -> [u8; XINPUT_STATE_BYTES] {
    let mut bytes = [0; XINPUT_STATE_BYTES];
    bytes[0..4].copy_from_slice(&packet.to_le_bytes());
    bytes[4..6].copy_from_slice(&state.buttons.to_le_bytes());
    bytes[6] = state.left_trigger;
    bytes[7] = state.right_trigger;
    for (place, value) in bytes[8..16].as_chunks_mut::<2>().0.iter_mut().zip([
        state.left_x,
        state.left_y,
        state.right_x,
        state.right_y,
    ]) {
        *place = value.to_le_bytes();
    }
    bytes
}

/// Serialize the capabilities of Xenora's virtual XInput gamepad surface. The
/// surface accepts every documented standard input field at full resolution,
/// but this implementation has no vibration output path, so both motor
/// resolutions are honestly zero and no feature flags are reported.
fn serialize_virtual_capabilities() -> [u8; XINPUT_CAPABILITIES_BYTES] {
    let mut bytes = [0; XINPUT_CAPABILITIES_BYTES];
    bytes[0] = XINPUT_DEVTYPE_GAMEPAD;
    bytes[1] = XINPUT_DEVSUBTYPE_GAMEPAD;
    bytes[4..6].copy_from_slice(&XINPUT_GAMEPAD_BUTTON_MASK.to_le_bytes());
    bytes[6] = u8::MAX;
    bytes[7] = u8::MAX;
    for place in bytes[8..16].as_chunks_mut::<2>().0.iter_mut() {
        *place = i16::MAX.to_le_bytes();
    }
    bytes
}

fn get_state(slots: &Mutex<Slots>, index: u32, output: u64, memory: &mut GuestMemory) -> u32 {
    if index >= XUSER_MAX_COUNT || output == 0 || !memory.can_write(output, XINPUT_STATE_BYTES) {
        return ERROR_BAD_ARGUMENTS;
    }
    let Some((packet, state)) = slots.lock().unwrap()[index as usize].get() else {
        return ERROR_DEVICE_NOT_CONNECTED;
    };
    if memory
        .write_bytes(output, &serialize(packet, state))
        .is_err()
    {
        return ERROR_BAD_ARGUMENTS;
    }
    0
}

fn get_capabilities(
    slots: &Mutex<Slots>,
    index: u32,
    flags: u32,
    output: u64,
    memory: &mut GuestMemory,
) -> u32 {
    if index >= XUSER_MAX_COUNT
        || flags & !XINPUT_FLAG_GAMEPAD != 0
        || output == 0
        || !memory.can_write(output, XINPUT_CAPABILITIES_BYTES)
    {
        return ERROR_BAD_ARGUMENTS;
    }
    if slots.lock().unwrap()[index as usize].get().is_none() {
        return ERROR_DEVICE_NOT_CONNECTED;
    }
    if memory
        .write_bytes(output, &serialize_virtual_capabilities())
        .is_err()
    {
        return ERROR_BAD_ARGUMENTS;
    }
    0
}

/// Register the state and capabilities queries on the public XInput library
/// names. Vibration, keystrokes, headset, battery information and physical
/// hotplug discovery remain separate unsupported surfaces.
pub(super) fn register(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    for module in MODULES {
        let slots = Arc::clone(state);
        registry.register_with_status(
            module,
            "XInputGetState",
            ServiceStatus::Partial,
            move |args, _, memory| {
                Ok(u64::from(get_state(
                    &slots.xinput,
                    args.rcx as u32,
                    args.rdx,
                    memory,
                )))
            },
        );
        let slots = Arc::clone(state);
        registry.register_with_status(
            module,
            "XInputGetCapabilities",
            ServiceStatus::Partial,
            move |args, _, memory| {
                Ok(u64::from(get_capabilities(
                    &slots.xinput,
                    args.rcx as u32,
                    args.rdx as u32,
                    args.r8,
                    memory,
                )))
            },
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
    use crate::services::MockClock;

    fn fixture() -> (Arc<ServicesState>, ImportRegistry) {
        let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
        let mut registry = ImportRegistry::new();
        register(&mut registry, &state);
        (state, registry)
    }

    fn call(
        registry: &ImportRegistry,
        memory: &mut GuestMemory,
        module: &str,
        index: u64,
        output: u64,
    ) -> u64 {
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

    fn call_capabilities(
        registry: &ImportRegistry,
        memory: &mut GuestMemory,
        index: u64,
        flags: u64,
        output: u64,
    ) -> u64 {
        let mut cpu = CpuState::default();
        cpu.set(crate::cpu::Register::Rcx, index);
        cpu.set(crate::cpu::Register::Rdx, flags);
        cpu.set(crate::cpu::Register::R8, output);
        let descriptor = registry
            .resolve(
                "xinput1_4.dll",
                &ImportName::Name("XInputGetCapabilities".to_owned()),
            )
            .unwrap();
        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
    }

    #[test]
    fn valid_slots_report_no_connected_controller_without_mutating_output() {
        let (_, registry) = fixture();
        let mut memory = writable_memory();
        memory.write_bytes(0x1ff0, &[0xaa; 16]).unwrap();
        for module in MODULES {
            for index in 0..4 {
                assert_eq!(
                    call(&registry, &mut memory, module, index, 0x1ff0),
                    u64::from(ERROR_DEVICE_NOT_CONNECTED)
                );
            }
        }
        let mut output = [0; 16];
        memory.read_bytes(0x1ff0, &mut output).unwrap();
        assert_eq!(output, [0xaa; 16]);
    }

    #[test]
    fn connected_snapshot_uses_the_exact_public_layout() {
        let (state, registry) = fixture();
        assert!(state.set_xinput_controller(
            2,
            Some(XInputControllerState {
                buttons: 0x9011,
                left_trigger: 17,
                right_trigger: 240,
                left_x: -32768,
                left_y: 32767,
                right_x: -1234,
                right_y: 4321,
            }),
        ));
        let mut memory = writable_memory();
        assert_eq!(call(&registry, &mut memory, "xinput1_4.dll", 2, 0x1200), 0);
        let mut output = [0; 16];
        memory.read_bytes(0x1200, &mut output).unwrap();
        assert_eq!(
            output,
            [
                1, 0, 0, 0, 0x11, 0x90, 17, 240, 0, 0x80, 0xff, 0x7f, 0x2e, 0xfb, 0xe1, 0x10,
            ]
        );
    }

    #[test]
    fn packet_numbers_change_only_for_changed_snapshots() {
        let (state, registry) = fixture();
        let mut memory = writable_memory();
        let one = XInputControllerState {
            buttons: 0x1000,
            ..XInputControllerState::default()
        };
        assert!(state.set_xinput_controller(0, Some(one)));
        assert_eq!(call(&registry, &mut memory, "xinputuap.dll", 0, 0x1200), 0);
        assert_eq!(memory.read_u32(0x1200).unwrap(), 1);
        assert!(state.set_xinput_controller(0, Some(one)));
        assert_eq!(call(&registry, &mut memory, "xinputuap.dll", 0, 0x1200), 0);
        assert_eq!(memory.read_u32(0x1200).unwrap(), 1);
        assert!(state.set_xinput_controller(
            0,
            Some(XInputControllerState {
                buttons: 0x2000,
                ..one
            }),
        ));
        assert_eq!(call(&registry, &mut memory, "xinputuap.dll", 0, 0x1200), 0);
        assert_eq!(memory.read_u32(0x1200).unwrap(), 2);
        assert!(state.set_xinput_controller(0, None));
        assert_eq!(
            call(&registry, &mut memory, "xinputuap.dll", 0, 0x1200),
            u64::from(ERROR_DEVICE_NOT_CONNECTED)
        );
    }

    #[test]
    fn connected_virtual_controller_reports_standard_input_capabilities() {
        let (state, registry) = fixture();
        assert!(state.set_xinput_controller(1, Some(XInputControllerState::default())));
        let mut memory = writable_memory();
        for flags in [0, 1] {
            assert_eq!(
                call_capabilities(&registry, &mut memory, 1, flags, 0x1200),
                0
            );
            let mut output = [0; 20];
            memory.read_bytes(0x1200, &mut output).unwrap();
            assert_eq!(
                output,
                [
                    1, 1, 0, 0, 0xff, 0xf3, 0xff, 0xff, 0xff, 0x7f, 0xff, 0x7f, 0xff, 0x7f, 0xff,
                    0x7f, 0, 0, 0, 0,
                ]
            );
        }
    }

    #[test]
    fn capabilities_reject_bad_input_before_writing_and_disconnected_slots() {
        let (_, registry) = fixture();
        let mut memory = writable_memory();
        memory.write_bytes(0x1fec, &[0xaa; 20]).unwrap();
        for (index, flags, output) in [
            (4, 0, 0x1fec),
            (0, 2, 0x1fec),
            (0, 0, 0),
            (0, 0, 0x1fed),
            (0, 0, 0x5000),
        ] {
            assert_eq!(
                call_capabilities(&registry, &mut memory, index, flags, output),
                u64::from(ERROR_BAD_ARGUMENTS)
            );
        }
        assert_eq!(
            call_capabilities(&registry, &mut memory, 0, 1, 0x1fec),
            u64::from(ERROR_DEVICE_NOT_CONNECTED)
        );
        let mut output = [0; 20];
        memory.read_bytes(0x1fec, &mut output).unwrap();
        assert_eq!(output, [0xaa; 20]);
    }

    #[test]
    fn invalid_indexes_and_output_ranges_are_rejected() {
        let (state, registry) = fixture();
        assert!(!state.set_xinput_controller(4, None));
        let mut memory = writable_memory();
        for (index, output) in [
            (4, 0x1ff0),
            (u64::from(u32::MAX), 0x1ff0),
            (0, 0),
            (0, 0x1ff1),
            (0, 0x5000),
        ] {
            assert_eq!(
                call(&registry, &mut memory, "xinput1_4.dll", index, output),
                u64::from(ERROR_BAD_ARGUMENTS)
            );
        }
    }

    #[test]
    fn read_only_output_is_rejected_before_any_mutation() {
        let (_, registry) = fixture();
        let mut memory = GuestMemory::new();
        memory
            .map_region(0x1000, 0x1000, MemoryPermissions::READ_ONLY)
            .unwrap();
        assert_eq!(
            call(&registry, &mut memory, "xinputuap.dll", 0, 0x1ff0),
            u64::from(ERROR_BAD_ARGUMENTS)
        );
    }
}
