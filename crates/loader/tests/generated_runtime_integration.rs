// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::Arc;
use xenora_loader::cpu::{CpuState, ExitReason};
use xenora_loader::imports::{ImportRegistry, ServiceStatus, Win64Args};
use xenora_loader::memory::{GuestMemory, MemoryError, MemoryPermissions};
use xenora_loader::services::{MockClock, ServicesState, register_standard_services};

#[test]
fn test_crt_math_resolves_through_its_api_set_and_uses_xmm0() {
    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(100))));
    register_standard_services(&mut registry, state);

    let mut cpu = CpuState::new(0x1000, 0x8000);
    let mut mem = GuestMemory::new();
    mem.map_region(0x1000, 0x10000, MemoryPermissions::READ_WRITE_EXECUTE)
        .unwrap();
    cpu.xmm[0] = u128::from(16.0f64.to_bits());
    let args = Win64Args {
        rcx: 9.0f64.to_bits(),
        rdx: 0,
        r8: 0,
        r9: 0,
        rsp: 0x8000,
    };

    let desc = registry
        .binding("api-ms-win-crt-math-l1-1-0.dll", "sqrt")
        .unwrap();
    (desc.handler)(&args, &mut cpu, &mut mem).unwrap();
    assert_eq!(f64::from_bits(cpu.xmm[0] as u64), 4.0);
}

#[test]
fn test_unsupported_stub_fails_explicitly() {
    let mut registry = ImportRegistry::new();
    registry.register_with_status(
        "kernel32.dll",
        "UnsupportedApi",
        ServiceStatus::Stub,
        |_args: &Win64Args,
         _cpu: &mut CpuState,
         _mem: &mut GuestMemory|
         -> Result<u64, ExitReason> { Err(ExitReason::IllegalInstruction(0xFE)) },
    );

    let mut cpu = CpuState::new(0x1000, 0x8000);
    let mut mem = GuestMemory::new();
    mem.map_region(0x1000, 0x10000, MemoryPermissions::READ_WRITE_EXECUTE)
        .unwrap();

    let args = Win64Args {
        rcx: 0,
        rdx: 0,
        r8: 0,
        r9: 0,
        rsp: 0x8000,
    };
    let desc = registry.binding("kernel32.dll", "UnsupportedApi").unwrap();
    let res = (desc.handler)(&args, &mut cpu, &mut mem);
    assert_eq!(res, Err(ExitReason::IllegalInstruction(0xFE)));
}

#[test]
fn crt_handlers_validate_memory_boundaries() {
    let mut registry = ImportRegistry::new();
    register_standard_services(
        &mut registry,
        Arc::new(ServicesState::new(Arc::new(MockClock::new(0)))),
    );
    let mut cpu = CpuState::new(0x1000, 0x8000);
    let mut memory = GuestMemory::new();
    memory
        .map_region_with_data(0x1000, 4, MemoryPermissions::READ_WRITE, b"abcd")
        .unwrap();
    memory
        .map_region_with_data(0x1004, 4, MemoryPermissions::READ_WRITE, b"efgh")
        .unwrap();
    let mut call = |symbol, rcx, rdx, r8, memory: &mut GuestMemory| {
        let args = Win64Args {
            rcx,
            rdx,
            r8,
            r9: 0,
            rsp: 0x8000,
        };
        let service = registry.binding("ucrtbase.dll", symbol).unwrap();
        (service.handler)(&args, &mut cpu, memory)
    };

    assert_eq!(call("strnlen", 0x1000, 8, 0, &mut memory), Ok(8));
    assert_eq!(
        call("strlen", 0x1000, 0, 0, &mut memory),
        Err(ExitReason::MemoryFault(MemoryError::UnmappedAddress(
            0x1008
        )))
    );
    // The destination 0x1005..0x1009 is mapped up to 0x1008.
    assert_eq!(
        call("memmove", 0x1005, 0x1000, 4, &mut memory),
        Err(ExitReason::MemoryFault(MemoryError::UnmappedAddress(
            0x1008
        )))
    );
    let mut bytes = [0; 8];
    memory.read_bytes(0x1000, &mut bytes).unwrap();
    assert_eq!(&bytes, b"abcdefgh");
    assert_eq!(call("memmove", 0x1002, 0x1000, 6, &mut memory), Ok(0x1002));
    memory.read_bytes(0x1000, &mut bytes).unwrap();
    assert_eq!(&bytes, b"ababcdef");
}
