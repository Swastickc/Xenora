// SPDX-License-Identifier: GPL-3.0-or-later

//! Typed runtime services, called by translated guest code through a bound
//! import table, and the import parser and registry behind them.
//!
//! Expected values come from the public Win32, ISO C and Microsoft x64 calling
//! convention documentation, not from the registry or the handlers.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use xenora_loader::cpu::{CpuState, ExitReason, Register};
use xenora_loader::dbt::DbtEngine;
use xenora_loader::import_table::{ImportName, ImportTableError, MAX_IMPORTS, parse_imports};
use xenora_loader::imports::{
    ImportError, ImportRegistry, RegistryError, ServiceStatus, Win64Args,
};
use xenora_loader::memory::{GuestMemory, MemoryPermissions};
use xenora_loader::pe::ParsedPe;
use xenora_loader::probe::{ProbeConfig, ProbeReport, probe_pe_bytes};
use xenora_loader::runtime::{Call, Float32, Float64};
use xenora_loader::runtime_table::SERVICES;
use xenora_loader::services::{
    MAX_CAPTURED_OUTPUT, MAX_PROGRAM_ARGUMENT_BYTES, MockClock, ProgramArgumentsError,
    ServicesState, register_standard_services,
};

const BASE: u64 = 0x1_4000_0000;
const CODE_RVA: u32 = 0x1000;
const IDATA_RVA: u32 = 0x10000;
const IDATA_RAW: usize = 0x2200;
const IDATA_LEN: usize = 0x4000;
const SCRATCH: u64 = 0x5_0000;
const STACK: u64 = 0x8_0000;
const STACK_TOP: u64 = STACK + 0x800;

#[derive(Clone)]
enum Symbol {
    Name(&'static str),
    Ordinal(u16),
}

/// Original PE32+ image importing `imports`, one descriptor each. The code for
/// import `n`, at RVA 0x1000 + 16n, is `call [its slot]; hlt`.
struct Image {
    bytes: Vec<u8>,
    slots: Vec<u32>,
}

fn build(imports: &[(&str, Symbol)]) -> Image {
    let mut pe = vec![0u8; IDATA_RAW + IDATA_LEN];
    fn put(pe: &mut [u8], at: usize, bytes: &[u8]) {
        pe[at..at + bytes.len()].copy_from_slice(bytes);
    }
    put(&mut pe, 0, b"MZ");
    put(&mut pe, 0x3C, &0x80u32.to_le_bytes());
    put(&mut pe, 0x80, b"PE\0\0");
    put(&mut pe, 0x84, &0x8664u16.to_le_bytes());
    put(&mut pe, 0x86, &2u16.to_le_bytes());
    put(&mut pe, 0x94, &240u16.to_le_bytes());
    let optional = 0x98;
    put(&mut pe, optional, &0x20Bu16.to_le_bytes());
    put(&mut pe, optional + 16, &CODE_RVA.to_le_bytes());
    put(&mut pe, optional + 24, &BASE.to_le_bytes());
    put(&mut pe, optional + 32, &0x1000u32.to_le_bytes());
    put(&mut pe, optional + 36, &0x200u32.to_le_bytes());
    put(&mut pe, optional + 56, &0x20000u32.to_le_bytes());
    put(&mut pe, optional + 60, &0x200u32.to_le_bytes());
    put(&mut pe, optional + 108, &16u32.to_le_bytes());
    put(&mut pe, optional + 120, &IDATA_RVA.to_le_bytes());
    put(&mut pe, optional + 124, &0x100u32.to_le_bytes());
    for (index, (name, rva, raw, len, flags)) in [
        (b".text", CODE_RVA, 0x200, 0x2000, 0x6000_0020u32),
        (b".idat", IDATA_RVA, IDATA_RAW, IDATA_LEN, 0xC000_0040),
    ]
    .into_iter()
    .enumerate()
    {
        let section = optional + 240 + 40 * index;
        put(&mut pe, section, name);
        put(&mut pe, section + 8, &(len as u32).to_le_bytes());
        put(&mut pe, section + 12, &rva.to_le_bytes());
        put(&mut pe, section + 16, &(len as u32).to_le_bytes());
        put(&mut pe, section + 20, &(raw as u32).to_le_bytes());
        put(&mut pe, section + 36, &flags.to_le_bytes());
    }

    let file = |rva: u32| IDATA_RAW + (rva - IDATA_RVA) as usize;
    let mut next = IDATA_RVA + 20 * (imports.len() as u32 + 1);
    let mut take = |len: usize| {
        let rva = next;
        next = (next + len as u32 + 7) & !7;
        rva
    };
    let mut slots = Vec::new();
    for (index, (module, symbol)) in imports.iter().enumerate() {
        let module_rva = take(module.len() + 1);
        put(&mut pe, file(module_rva), module.as_bytes());
        let entry = match symbol {
            Symbol::Ordinal(ordinal) => 1 << 63 | u64::from(*ordinal),
            Symbol::Name(name) => {
                let hint = take(name.len() + 3);
                put(&mut pe, file(hint) + 2, name.as_bytes());
                u64::from(hint)
            }
        };
        let (lookup, slot) = (take(16), take(16));
        put(&mut pe, file(lookup), &entry.to_le_bytes());
        put(&mut pe, file(slot), &entry.to_le_bytes());
        let descriptor = file(IDATA_RVA) + 20 * index;
        put(&mut pe, descriptor, &lookup.to_le_bytes());
        put(&mut pe, descriptor + 12, &module_rva.to_le_bytes());
        put(&mut pe, descriptor + 16, &slot.to_le_bytes());
        slots.push(slot);

        let code_rva = CODE_RVA + 16 * index as u32;
        let displacement = slot.wrapping_sub(code_rva + 6);
        let at = 0x200 + 16 * index;
        put(&mut pe, at, &[0xFF, 0x15]); // call qword ptr [rip + displacement]
        put(&mut pe, at + 2, &displacement.to_le_bytes());
        put(&mut pe, at + 6, &[0xF4]); // hlt
    }
    Image { bytes: pe, slots }
}

fn standard(clock: u64) -> (ImportRegistry, Arc<MockClock>) {
    let (registry, clock, _) = standard_with_state(clock);
    (registry, clock)
}

fn standard_with_state(clock: u64) -> (ImportRegistry, Arc<MockClock>, Arc<ServicesState>) {
    let clock = Arc::new(MockClock::new(clock));
    let state = Arc::new(ServicesState::new(clock.clone()));
    let mut registry = ImportRegistry::new();
    register_standard_services(&mut registry, state.clone());
    (registry, clock, state)
}

/// A guest whose image imports every generated service from its own library,
/// and `_errno` after them, which is registered by hand.
struct Guest {
    engine: DbtEngine,
    memory: GuestMemory,
    clock: Arc<MockClock>,
    state: Arc<ServicesState>,
    teb: u64,
    /// Upper halves and XMM lanes of unrelated arguments hold this pattern.
    xmm: [u128; 4],
}

impl Guest {
    fn new() -> Self {
        let mut imports: Vec<(&str, Symbol)> = SERVICES
            .iter()
            .map(|service| (service.library, Symbol::Name(service.symbol)))
            .collect();
        imports.push(("ucrtbase.dll", Symbol::Name("_errno")));
        let image = build(&imports);
        let pe = ParsedPe::parse(&image.bytes).unwrap();
        let mut memory = pe.load_into_memory(&image.bytes).unwrap().memory;
        memory
            .map_region(SCRATCH, 0x100, MemoryPermissions::READ_WRITE)
            .unwrap();
        memory
            .map_region(STACK, 0x1000, MemoryPermissions::READ_WRITE)
            .unwrap();
        let (mut registry, clock, state) = standard_with_state(0x01DA_0000_1234_5678);
        let bound = registry
            .bind_imports(&pe, &image.bytes, BASE, &mut memory)
            .unwrap();
        assert_eq!(bound, SERVICES.len() + 1);
        let mut engine = DbtEngine::new(256 * 1024).unwrap();
        engine.set_import_registry(registry);
        Self {
            engine,
            memory,
            clock,
            state,
            teb: 0,
            xmm: [0xAAAA_BBBB_CCCC_DDDD_EEEE_FFFF_0123_4567; 4],
        }
    }

    fn run(&mut self, symbol: &str, args: &[u64]) -> (CpuState, ExitReason) {
        let index = SERVICES
            .iter()
            .position(|service| service.symbol == symbol)
            .or((symbol == "_errno").then_some(SERVICES.len()))
            .unwrap();
        let mut cpu = CpuState::new(BASE + u64::from(CODE_RVA) + 16 * index as u64, STACK_TOP);
        cpu.gs_base = self.teb;
        for (index, register) in cpu.gpr.iter_mut().enumerate() {
            if index != Register::Rsp as usize {
                *register = 0x0101_0101_0101_0101 * (index as u64 + 1);
            }
        }
        for (register, value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(register, *value);
        }
        for (index, register) in cpu.xmm.iter_mut().enumerate() {
            *register = 0x1111_2222_3333_4444_5555_6666_7777_0000 + index as u128;
        }
        cpu.xmm[..4].copy_from_slice(&self.xmm);
        let before = cpu.clone();
        let result = self.engine.run(&mut cpu, &mut self.memory, 8);
        if result.exit_reason == ExitReason::Halt {
            use Register::{R12, R13, R14, R15, Rbp, Rbx, Rdi, Rsi, Rsp};
            for register in [Rbx, Rbp, Rdi, Rsi, Rsp, R12, R13, R14, R15] {
                assert_eq!(
                    cpu.get(register),
                    before.get(register),
                    "{symbol} {register:?}"
                );
            }
            assert_eq!(cpu.xmm[6..], before.xmm[6..], "{symbol}");
            assert_eq!(cpu.mxcsr, before.mxcsr, "{symbol}");
        }
        (cpu, result.exit_reason)
    }

    fn call(&mut self, symbol: &str, args: &[u64]) -> u64 {
        let (cpu, exit) = self.run(symbol, args);
        assert_eq!(exit, ExitReason::Halt, "{symbol}");
        cpu.get(Register::Rax)
    }

    fn last_error(&mut self) -> u64 {
        self.call("GetLastError", &[])
    }

    /// The calling thread's `errno`, read through the pointer `_errno` returns.
    fn errno(&mut self) -> u32 {
        let address = self.call("_errno", &[]);
        self.memory.read_u32(address).unwrap()
    }

    fn set_errno(&mut self, value: u32) {
        let address = self.call("_errno", &[]);
        self.memory.write_u32(address, value).unwrap();
    }
}

/// Bits above an `int` argument are not part of it.
const JUNK_HIGH: u64 = 0xFFFF_FFFF_0000_0000;

const PROCESS: u64 = u64::MAX;
const THREAD: u64 = u64::MAX - 1;

#[test]
fn identity_services_return_documented_pseudo_handles_and_ids() {
    let mut guest = Guest::new();
    assert_eq!(guest.call("GetCurrentProcess", &[]), PROCESS);
    assert_eq!(guest.call("GetCurrentThread", &[]), THREAD);
    let id = guest.call("GetCurrentProcessId", &[]);
    assert!(id != 0 && id <= u64::from(u32::MAX));
    assert_ne!(guest.call("GetCurrentThreadId", &[]), id);
    assert_eq!(guest.call("GetProcessId", &[PROCESS]), id);
    assert_eq!(guest.call("GetProcessId", &[THREAD]), 0);
    assert_eq!(guest.last_error(), 6); // ERROR_INVALID_HANDLE
}

#[test]
fn priority_boost_round_trips_and_checks_its_handle() {
    let mut guest = Guest::new();
    let read = |guest: &mut Guest| {
        guest.memory.write_u64(SCRATCH, u64::MAX).unwrap();
        assert_eq!(
            guest.call("GetProcessPriorityBoost", &[PROCESS, SCRATCH]),
            1
        );
        // A BOOL is four bytes; the next four stay untouched.
        assert_eq!(guest.memory.read_u32(SCRATCH + 4).unwrap(), u32::MAX);
        guest.memory.read_u32(SCRATCH).unwrap()
    };
    assert_eq!(read(&mut guest), 0);
    // Any nonzero BOOL disables; bits above 32 are not part of the argument.
    assert_eq!(
        guest.call("SetProcessPriorityBoost", &[PROCESS, 0xFFFF_FFFF_0000_0002]),
        1
    );
    assert_eq!(read(&mut guest), 1);
    assert_eq!(
        guest.call("SetProcessPriorityBoost", &[PROCESS, 0xFFFF_FFFF_0000_0000]),
        1
    );
    assert_eq!(read(&mut guest), 0);
    assert_eq!(guest.call("SetProcessPriorityBoost", &[THREAD, 1]), 0);
    assert_eq!(guest.last_error(), 6);
    assert_eq!(
        guest.call("GetProcessPriorityBoost", &[PROCESS, 0x9000_0000]),
        0
    );
    assert_eq!(guest.last_error(), 998); // ERROR_NOACCESS
}

#[test]
fn thread_handles_check_access_and_are_rejected_once_closed() {
    let mut guest = Guest::new();
    let id = guest.call("GetCurrentThreadId", &[]);
    // THREAD_QUERY_INFORMATION only.
    let query = guest.call("OpenThread", &[0x40, 0, id]);
    assert_ne!(query, 0);
    assert_eq!(guest.call("GetThreadPriority", &[query]), 0);
    assert_eq!(guest.call("SetThreadPriority", &[query, 1]), 0);
    assert_eq!(guest.last_error(), 5); // ERROR_ACCESS_DENIED
    assert_eq!(guest.call("GetThreadPriority", &[query]), 0);
    // THREAD_SET_INFORMATION | THREAD_QUERY_INFORMATION.
    let full = guest.call("OpenThread", &[0x60, 0, id]);
    assert_ne!(full, query);
    // A negative int is returned in EAX with the upper half clear.
    assert_eq!(
        guest.call("SetThreadPriority", &[full, 0xABCD_0000_FFFF_FFFE]),
        1
    );
    assert_eq!(guest.call("GetThreadPriority", &[full]), 0xFFFF_FFFE);
    assert_eq!(guest.call("SetThreadPriority", &[full, 3]), 0);
    assert_eq!(guest.last_error(), 87); // ERROR_INVALID_PARAMETER
    assert_eq!(guest.call("CloseHandle", &[full]), 1);
    assert_eq!(guest.call("GetThreadPriority", &[full]), 0x7FFF_FFFF);
    assert_eq!(guest.last_error(), 6);
    assert_eq!(guest.call("CloseHandle", &[full]), 0);
    assert_eq!(guest.call("OpenThread", &[0x40, 0, id + 1]), 0);
    assert_eq!(guest.last_error(), 87);
    assert_eq!(guest.call("OpenThread", &[0x40, 1, id]), 0);
    assert_eq!(guest.last_error(), 50); // ERROR_NOT_SUPPORTED
    assert_ne!(
        guest.call("OpenThread", &[0x40, 0, id]),
        full,
        "handles are not reused"
    );
}

#[test]
fn system_time_comes_from_the_injected_clock() {
    let mut guest = Guest::new();
    guest.call("GetSystemTimeAsFileTime", &[SCRATCH + 3]);
    assert_eq!(
        guest.memory.read_u64(SCRATCH + 3).unwrap(),
        0x01DA_0000_1234_5678
    );
    guest.clock.advance(10);
    guest.call("GetSystemTimeAsFileTime", &[SCRATCH]);
    assert_eq!(
        guest.memory.read_u64(SCRATCH).unwrap(),
        0x01DA_0000_1234_5682
    );
    for pointer in [0, SCRATCH + 0xFC] {
        let before = guest.memory.read_u32(SCRATCH + 0xFC).unwrap();
        let (_, exit) = guest.run("GetSystemTimeAsFileTime", &[pointer]);
        assert!(matches!(exit, ExitReason::MemoryFault(_)), "{exit:?}");
        assert_eq!(guest.memory.read_u32(SCRATCH + 0xFC).unwrap(), before);
    }
}

#[test]
fn guest_udp_loopback_delivers_datagrams_through_bound_imports() {
    let symbols = [
        "WSAStartup",
        "socket",
        "bind",
        "connect",
        "send",
        "recv",
        "WSAGetLastError",
        "WSACleanup",
        "WSAPoll",
        "getsockname",
        "getpeername",
        "getsockopt",
        "inet_pton",
        "inet_ntop",
        "htons",
    ];
    let imports = symbols.map(|symbol| ("ws2_32.dll", Symbol::Name(symbol)));
    let image = build(&imports);
    let pe = ParsedPe::parse(&image.bytes).unwrap();
    let mut memory = pe.load_into_memory(&image.bytes).unwrap().memory;
    memory
        .map_region(SCRATCH, 4096, MemoryPermissions::READ_WRITE)
        .unwrap();
    memory
        .map_region(STACK, 4096, MemoryPermissions::READ_WRITE)
        .unwrap();
    let (mut registry, _) = standard(0);
    assert_eq!(
        registry
            .bind_imports(&pe, &image.bytes, BASE, &mut memory)
            .unwrap(),
        symbols.len()
    );
    let mut engine = DbtEngine::new(256 * 1024).unwrap();
    engine.set_import_registry(registry);
    let mut call = |memory: &mut GuestMemory, symbol, args: &[u64]| {
        let index = symbols.iter().position(|&name| symbol == name).unwrap() as u64;
        let mut cpu = CpuState::new(BASE + u64::from(CODE_RVA) + 16 * index, STACK_TOP);
        for (register, &value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(register, value);
        }
        if let Some(&fifth) = args.get(4) {
            memory.write_u64(STACK_TOP + 32, fifth).unwrap();
        }
        let result = engine.run(&mut cpu, memory, 8);
        assert_eq!(result.exit_reason, ExitReason::Halt, "{symbol}");
        cpu.get(Register::Rax)
    };
    let address = SCRATCH + 0x200;
    let outgoing = SCRATCH + 0x220;
    let incoming = SCRATCH + 0x240;
    let text = SCRATCH + 0x260;
    let poll = SCRATCH + 0x280;
    let formatted = SCRATCH + 0x2a0;
    let address_length = SCRATCH + 0x2b0;
    let peer_address = SCRATCH + 0x2c0;
    memory.write_u16(address, 2).unwrap();
    let port = call(&mut memory, "htons", &[5000]);
    assert_eq!(port, 0x8813);
    memory.write_bytes(text, b"127.0.0.1\0").unwrap();
    assert_eq!(call(&mut memory, "inet_pton", &[2, text, address + 4]), 1);
    assert_eq!(
        memory.read_u32(address + 4).unwrap().to_le_bytes(),
        [127, 0, 0, 1]
    );
    memory.write_bytes(text, b"127.0.0.999\0").unwrap();
    assert_eq!(call(&mut memory, "inet_pton", &[2, text, address + 4]), 0);
    assert_eq!(
        memory.read_u32(address + 4).unwrap().to_le_bytes(),
        [127, 0, 0, 1]
    );
    assert_eq!(
        call(&mut memory, "inet_pton", &[23, text, address + 4]),
        u64::from(u32::MAX)
    );
    assert_eq!(call(&mut memory, "WSAGetLastError", &[]), 10047);
    memory.write_u8(formatted, 0xaa).unwrap();
    assert_eq!(
        call(&mut memory, "inet_ntop", &[2, address + 4, formatted, 9]),
        0
    );
    assert_eq!(call(&mut memory, "WSAGetLastError", &[]), 87);
    assert_eq!(memory.read_u8(formatted).unwrap(), 0xaa);
    assert_eq!(
        call(&mut memory, "inet_ntop", &[2, address + 4, formatted, 10]),
        formatted
    );
    let mut formatted_address = [0; 10];
    memory
        .read_bytes(formatted, &mut formatted_address)
        .unwrap();
    assert_eq!(&formatted_address, b"127.0.0.1\0");
    assert_eq!(call(&mut memory, "WSAStartup", &[0x0202, SCRATCH]), 0);
    let receiver = call(&mut memory, "socket", &[2, 2, 17]);
    let sender = call(&mut memory, "socket", &[2, 2, 17]);
    assert_ne!(receiver, u64::MAX);
    assert_ne!(sender, u64::MAX);
    assert_ne!(sender, receiver);
    for (option, expected) in [(0x1008, 2), (0x2003, 65_507)] {
        memory.write_u32(address_length, 8).unwrap();
        assert_eq!(
            call(
                &mut memory,
                "getsockopt",
                &[sender, 0xffff, option, incoming, address_length]
            ),
            0
        );
        assert_eq!(memory.read_u32(incoming).unwrap(), expected);
        assert_eq!(memory.read_u32(address_length).unwrap(), 4);
    }
    memory.write_u32(address_length, 3).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "getsockopt",
            &[sender, 0xffff, 0x1008, incoming, address_length]
        ),
        u64::from(u32::MAX)
    );
    assert_eq!(call(&mut memory, "WSAGetLastError", &[]), 10014);
    assert_eq!(memory.read_u32(incoming).unwrap(), 65_507);
    assert_eq!(call(&mut memory, "bind", &[receiver, address, 16]), 0);
    memory.write_u32(address_length, 15).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "getsockname",
            &[receiver, address, address_length]
        ),
        u64::from(u32::MAX)
    );
    assert_eq!(call(&mut memory, "WSAGetLastError", &[]), 10014);
    assert_eq!(memory.read_u16(address + 2).unwrap(), 0);
    memory.write_u32(address_length, 32).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "getsockname",
            &[receiver, address, address_length]
        ),
        0
    );
    assert_eq!(memory.read_u32(address_length).unwrap(), 16);
    assert_eq!(memory.read_u16(address + 2).unwrap().swap_bytes(), 49152);
    assert_eq!(call(&mut memory, "connect", &[sender, address, 16]), 0);
    assert_eq!(
        call(
            &mut memory,
            "getpeername",
            &[sender, peer_address, address_length]
        ),
        0
    );
    assert_eq!(
        memory.read_u64(peer_address).unwrap(),
        memory.read_u64(address).unwrap()
    );
    memory.write_u64(poll, receiver).unwrap();
    memory.write_u16(poll + 8, 0x0100).unwrap();
    memory.write_u64(poll + 16, sender).unwrap();
    memory.write_u16(poll + 24, 0x0010).unwrap();
    assert_eq!(call(&mut memory, "WSAPoll", &[poll, 2, 0]), 1);
    assert_eq!(memory.read_u16(poll + 10).unwrap(), 0);
    assert_eq!(memory.read_u16(poll + 26).unwrap(), 0x0010);
    memory.write_bytes(outgoing, b"loop").unwrap();
    for _ in 0..16 {
        assert_eq!(call(&mut memory, "send", &[sender, outgoing, 4, 0]), 4);
    }
    assert_eq!(
        call(&mut memory, "send", &[sender, outgoing, 4, 0]),
        u64::from(u32::MAX)
    );
    assert_eq!(call(&mut memory, "WSAGetLastError", &[]), 10055);
    assert_eq!(call(&mut memory, "WSAPoll", &[poll, 2, 0]), 1);
    assert_eq!(memory.read_u16(poll + 10).unwrap(), 0x0100);
    assert_eq!(memory.read_u16(poll + 26).unwrap(), 0);
    for _ in 0..16 {
        assert_eq!(call(&mut memory, "recv", &[receiver, incoming, 4, 0]), 4);
        let mut received = [0; 4];
        memory.read_bytes(incoming, &mut received).unwrap();
        assert_eq!(&received, b"loop");
        memory.write_u32(incoming, 0).unwrap();
    }
    assert_eq!(
        call(&mut memory, "recv", &[receiver, incoming, 4, 0]),
        u64::from(u32::MAX)
    );
    assert_eq!(call(&mut memory, "WSAGetLastError", &[]), 10035);
    assert_eq!(call(&mut memory, "WSAPoll", &[poll, 2, 0]), 1);
    assert_eq!(memory.read_u16(poll + 10).unwrap(), 0);
    assert_eq!(memory.read_u16(poll + 26).unwrap(), 0x0010);
    let started = std::time::Instant::now();
    assert_eq!(call(&mut memory, "WSAPoll", &[poll, 1, 10]), 0);
    assert!(started.elapsed() >= std::time::Duration::from_millis(10));
    assert_eq!(memory.read_u16(poll + 10).unwrap(), 0);
    assert_eq!(
        call(&mut memory, "WSAPoll", &[poll, 1, 1001]),
        u64::from(u32::MAX)
    );
    assert_eq!(call(&mut memory, "WSAGetLastError", &[]), 10045);
    assert_eq!(call(&mut memory, "WSACleanup", &[]), 0);
}

#[test]
fn last_error_isolated_across_guest_thread_switches() {
    let mut guest = Guest::new();
    guest.teb = 0x10000;
    guest.call("SetLastError", &[123]);
    guest.teb = 0x20000;
    assert_eq!(guest.last_error(), 0);
    assert_eq!(guest.call("CloseHandle", &[123]), 0);
    assert_eq!(guest.last_error(), 6);
    guest.teb = 0x10000;
    guest.call("GetCurrentProcessId", &[]);
    assert_eq!(guest.last_error(), 123);
    guest.call("SetLastError", &[0]);
    assert_eq!(guest.last_error(), 0);
    guest.teb = 0x20000;
    assert_eq!(guest.last_error(), 6);
}

/// The last-error code is documented as kept per thread, and most functions
/// set it only when they fail (errhandlingapi.h GetLastError).
#[test]
fn a_successful_call_keeps_another_threads_error_and_its_own() {
    let mut guest = Guest::new();
    guest.teb = 0x10000;
    assert_eq!(guest.call("VirtualAlloc", &[0, 0, 0x3000, 4]), 0);
    assert_eq!(guest.last_error(), 87); // ERROR_INVALID_PARAMETER
    guest.teb = 0x20000;
    guest.call("SetLastError", &[0xFFFF_FFFF]);
    assert_eq!(guest.call("GetCurrentProcess", &[]), PROCESS);
    assert_eq!(guest.call("GetCurrentThreadId", &[]), 0x4104);
    assert_eq!(
        guest.last_error(),
        0xFFFF_FFFF,
        "success keeps a nonzero error"
    );
    guest.teb = 0x10000;
    assert_eq!(guest.last_error(), 87);
}

#[test]
fn a_retired_thread_starts_clean_when_its_teb_address_is_reused() {
    let mut guest = Guest::new();
    guest.teb = 0x10000;
    guest.call("SetLastError", &[998]);
    guest.teb = 0x20000;
    guest.call("CloseHandle", &[0x1234]);

    assert!(guest.state.retire_thread_last_error(0x10000));
    assert!(!guest.state.retire_thread_last_error(0x10000));
    assert!(
        !guest.state.retire_thread_last_error(0x30000),
        "a thread that never failed holds no value"
    );
    assert_eq!(guest.last_error(), 6, "the other thread is untouched");
    guest.teb = 0x10000;
    assert_eq!(guest.last_error(), 0);
    guest.call("SetLastError", &[5]);
    assert_eq!(guest.last_error(), 5);
}

/// Guest memory for format strings, argument lists and strings.
const TEXT: u64 = 0x100_0000;
const TEXT_LEN: usize = 0x10_0000 + 0x1000;
/// Where `print` puts the `va_list`, and where string arguments start.
const VA_LIST: u64 = TEXT + 0x400;
const STRINGS: u64 = TEXT + 0x800;
/// What a program built with mingw-w64 passes as the options of
/// `__stdio_common_vfprintf`.
const MINGW_OPTIONS: u64 = 0x24;
const LIMIT: ExitReason = ExitReason::IllegalInstruction(0xFE);
const STDIO_EINVAL: u32 = 22;
/// The low 32 bits of an `int` result of -1, which is also `EOF`.
const STDIO_MINUS_ONE: u64 = 0xFFFF_FFFF;

/// A guest with its three standard streams and room for text.
fn stdio_guest() -> (Guest, [u64; 3]) {
    let mut guest = Guest::new();
    guest
        .memory
        .map_region(TEXT, TEXT_LEN, MemoryPermissions::READ_WRITE)
        .unwrap();
    let streams = [0, 1, 2].map(|index| guest.call("__acrt_iob_func", &[index]));
    (guest, streams)
}

/// Put a NUL-terminated string in guest memory and return its address.
fn place(guest: &mut Guest, at: u64, text: &[u8]) -> u64 {
    guest.memory.write_bytes(at, text).unwrap();
    guest.memory.write_u8(at + text.len() as u64, 0).unwrap();
    at
}

/// Call `__stdio_common_vfprintf` as compiled code does: the format string
/// and the `va_list` are in guest memory, and the fifth argument is on the
/// stack above the shadow space.
fn print_with(
    guest: &mut Guest,
    options: u64,
    stream: u64,
    format: &[u8],
    slots: &[u64],
) -> (u64, ExitReason) {
    let format = place(guest, TEXT, format);
    for (index, slot) in slots.iter().enumerate() {
        guest
            .memory
            .write_u64(VA_LIST + 8 * index as u64, *slot)
            .unwrap();
    }
    guest.memory.write_u64(STACK_TOP + 32, VA_LIST).unwrap();
    let (cpu, exit) = guest.run("__stdio_common_vfprintf", &[options, stream, format, 0]);
    (cpu.get(Register::Rax) & 0xFFFF_FFFF, exit)
}

fn print(guest: &mut Guest, stream: u64, format: &[u8], slots: &[u64]) -> u64 {
    let (count, exit) = print_with(guest, MINGW_OPTIONS, stream, format, slots);
    assert_eq!(
        exit,
        ExitReason::Halt,
        "{}",
        String::from_utf8_lossy(format)
    );
    count
}

/// `__acrt_iob_func` (corecrt_wstdio.h) is what `stdin`, `stdout` and
/// `stderr` expand to: indices 0, 1 and 2. A `FILE` is one pointer wide.
#[test]
fn standard_streams_are_three_stable_file_objects() {
    let (mut guest, streams) = stdio_guest();
    let mut sorted = streams;
    sorted.sort_unstable();
    assert!(sorted[0] != 0 && sorted[1] - sorted[0] >= 8 && sorted[2] - sorted[1] >= 8);
    // Each call returns the same object, and the index is an unsigned int.
    for (index, stream) in streams.into_iter().enumerate() {
        let index = index as u64;
        assert_eq!(guest.call("__acrt_iob_func", &[index]), stream);
        assert_eq!(
            guest.call("__acrt_iob_func", &[0xFFFF_FFFF_0000_0000 | index]),
            stream
        );
    }
    // There is no fourth standard stream.
    for index in [3, 4, 0x8000_0000, 0xFFFF_FFFF] {
        assert_eq!(
            guest.run("__acrt_iob_func", &[index]).1,
            LIMIT,
            "{index:#x}"
        );
    }
    // Asking for a stream writes nothing to it.
    assert!(guest.state.standard_output().is_empty());
    assert!(guest.state.standard_error().is_empty());
}

/// `__stdio_common_vfprintf` is what `printf` and `fprintf` call. It returns
/// "the number of characters written" (Microsoft Learn, vfprintf). The
/// standard streams are text-mode streams, where "each line feed is replaced
/// with a carriage return-line feed pair" that "has no effect on the return
/// value" (fwrite). Expected text follows the format specification page.
#[test]
fn formatted_output_reaches_the_captured_standard_streams() {
    let (mut guest, [_, out, err]) = stdio_guest();
    // The calls the Gate A2 program makes, with the options mingw-w64 passes.
    assert_eq!(print(&mut guest, out, b"gate a2: %d arguments\n", &[3]), 21);
    let one = place(&mut guest, STRINGS, b"one");
    assert_eq!(print(&mut guest, out, b"arg %d = %s\n", &[1, one]), 12);
    assert_eq!(
        guest.state.standard_output(),
        b"gate a2: 3 arguments\r\narg 1 = one\r\n"
    );
    assert!(guest.state.standard_error().is_empty());

    // stderr is a separate buffer.
    assert_eq!(print(&mut guest, err, b"%s: %#x\n", &[one, 0xa2]), 10);
    assert_eq!(guest.state.standard_error(), b"one: 0xa2\r\n");
    assert_eq!(guest.state.standard_output().len(), 35);

    // The arguments are 8-byte slots read in order from the va_list: a width
    // from *, an int whose upper half is junk, a 64-bit value, a string with
    // a precision, a character and a pointer.
    let junk = 0xDEAD_BEEF_0000_0000;
    let slots = [
        6,
        junk | 0xFFFF_FFFE,
        u64::MAX,
        one,
        u64::from(b'!'),
        0x1_4000_2000,
    ];
    let (mut fresh, [_, out, _]) = stdio_guest();
    let one = place(&mut fresh, STRINGS, b"one");
    let slots = [slots[0], slots[1], slots[2], one, slots[4], slots[5]];
    let text: &[u8] = b"[%*d] %llu %.2s%c %p";
    // The result is the number of characters in the text below: 50.
    let expected: &[u8] = b"[    -2] 18446744073709551615 on! 0000000140002000";
    assert_eq!(print(&mut fresh, out, text, &slots), 50);
    assert_eq!(expected.len(), 50);
    assert_eq!(fresh.state.standard_output(), expected);

    // Several line feeds in one call, text with none, and an empty format.
    let (mut guest, [_, out, _]) = stdio_guest();
    assert_eq!(print(&mut guest, out, b"a\n\nb\n", &[]), 5);
    assert_eq!(print(&mut guest, out, b"no newline", &[]), 10);
    assert_eq!(print(&mut guest, out, b"", &[]), 0);
    // A carriage return the program wrote itself is kept as it is.
    assert_eq!(print(&mut guest, out, b"\r\n", &[]), 2);
    assert_eq!(
        guest.state.standard_output(),
        b"a\r\n\r\nb\r\nno newline\r\r\n"
    );

    // The option bits that concern wide functions and floating point change
    // nothing here.
    for options in [0, 0x04, 0x10, 0x20, 0x24, 0x34] {
        let (mut guest, [_, out, _]) = stdio_guest();
        let (count, exit) = print_with(&mut guest, options, out, b"%5.3d|%-4x|", &[7, 255]);
        assert_eq!((count, exit), (11, ExitReason::Halt), "{options:#x}");
        assert_eq!(
            guest.state.standard_output(),
            b"  007|ff  |",
            "{options:#x}"
        );
    }
}

/// What `__stdio_common_vfprintf` refuses. A null stream or format returns -1
/// with `errno` `EINVAL` (Microsoft Learn, vfprintf). An invalid format calls
/// the invalid parameter handler on Windows; here it returns the same -1 and
/// writes nothing. What the runtime does not model stops the guest. In every
/// case nothing reaches either stream.
#[test]
fn formatted_output_refuses_what_it_cannot_print_correctly() {
    let (mut guest, [input, out, _]) = stdio_guest();
    let quiet = |guest: &Guest| {
        assert!(guest.state.standard_output().is_empty());
        assert!(guest.state.standard_error().is_empty());
    };

    // Null stream, null format.
    guest.set_errno(0);
    let format = place(&mut guest, TEXT, b"text");
    guest.memory.write_u64(STACK_TOP + 32, VA_LIST).unwrap();
    for args in [[MINGW_OPTIONS, 0, format, 0], [MINGW_OPTIONS, out, 0, 0]] {
        let result = guest.call("__stdio_common_vfprintf", &args);
        assert_eq!(result & 0xFFFF_FFFF, STDIO_MINUS_ONE, "{args:x?}");
        assert_eq!(guest.errno(), STDIO_EINVAL, "{args:x?}");
        guest.set_errno(0);
    }
    quiet(&guest);

    // %n is disabled by default, and a character with no meaning as a type
    // is invalid. The text before it is not written.
    for format in [&b"count%n"[..], b"before %d then %y", b"%", b"100%"] {
        let (count, exit) = print_with(&mut guest, MINGW_OPTIONS, out, format, &[1, 2]);
        assert_eq!((count, exit), (STDIO_MINUS_ONE, ExitReason::Halt));
        assert_eq!(guest.errno(), STDIO_EINVAL);
        guest.set_errno(0);
    }
    quiet(&guest);

    // Floating point and wide strings are valid and not modelled.
    for format in [&b"%f"[..], b"ok %d then %g", b"%ls", b"%S", b"%lc"] {
        let (_, exit) = print_with(&mut guest, MINGW_OPTIONS, out, format, &[1, 2]);
        assert_eq!(exit, LIMIT, "{}", String::from_utf8_lossy(format));
    }
    // Option bits that change how the format is read or that belong to the
    // buffer functions, and unknown bits.
    for options in [0x01, 0x02, 0x08, 0x40, 0x25, 1 << 32, u64::MAX] {
        let (_, exit) = print_with(&mut guest, options, out, b"x", &[]);
        assert_eq!(exit, LIMIT, "{options:#x}");
    }
    // A locale object.
    let format = place(&mut guest, TEXT, b"x");
    let args = [MINGW_OPTIONS, out, format, STRINGS];
    assert_eq!(guest.run("__stdio_common_vfprintf", &args).1, LIMIT);
    // stdin, and pointers that are no stream: beside a FILE object, inside
    // one, and elsewhere.
    for stream in [input, out + 1, out + 24, out - 16, TEXT, 8] {
        let (_, exit) = print_with(&mut guest, MINGW_OPTIONS, stream, b"x", &[]);
        assert_eq!(exit, LIMIT, "{stream:#x}");
    }
    // Unreadable memory is a fault at the address that could not be read.
    let unmapped = 0x9_0000_0000;
    let (_, exit) = print_with(&mut guest, MINGW_OPTIONS, out, b"%s", &[unmapped]);
    assert!(matches!(exit, ExitReason::MemoryFault(_)), "{exit:?}");
    guest.memory.write_u64(STACK_TOP + 32, unmapped).unwrap();
    for args in [
        [MINGW_OPTIONS, out, format, 0],
        [MINGW_OPTIONS, out, unmapped, 0],
    ] {
        let format_reads_arguments = place(&mut guest, TEXT, b"%d");
        assert_eq!(format_reads_arguments, format);
        let exit = guest.run("__stdio_common_vfprintf", &args).1;
        assert!(matches!(exit, ExitReason::MemoryFault(_)), "{exit:?}");
    }
    quiet(&guest);
    // None of the stops set errno.
    assert_eq!(guest.errno(), 0);

    // A guest that never asked for its streams has none to write to.
    let mut fresh = Guest::new();
    fresh
        .memory
        .map_region(TEXT, TEXT_LEN, MemoryPermissions::READ_WRITE)
        .unwrap();
    let (_, exit) = print_with(&mut fresh, MINGW_OPTIONS, out, b"x", &[]);
    assert_eq!(exit, LIMIT);
}

/// `fwrite` "returns the number of full items the function writes" and, for a
/// null buffer or stream, sets `errno` to `EINVAL` and returns 0. `fputs`
/// returns "a nonnegative value if it's successful", copies no terminating
/// null, and returns `EOF` with `EINVAL` for a null pointer (Microsoft Learn).
#[test]
fn fwrite_and_fputs_write_to_the_captured_standard_streams() {
    let (mut guest, [input, out, err]) = stdio_guest();
    let text = place(&mut guest, TEXT, b"cleanup ran\nxyz");
    // Twelve items of one byte, then three items of four bytes to stderr.
    assert_eq!(guest.call("fwrite", &[text, 1, 12, out]), 12);
    assert_eq!(guest.call("fwrite", &[text, 4, 3, err]), 3);
    assert_eq!(guest.state.standard_output(), b"cleanup ran\r\n");
    assert_eq!(guest.state.standard_error(), b"cleanup ran\r\n");
    // No items or no size writes nothing and returns 0.
    for (size, count) in [(0, 5), (5, 0), (0, 0)] {
        assert_eq!(guest.call("fwrite", &[text, size, count, out]), 0);
    }
    assert_eq!(guest.state.standard_output().len(), 13);
    // A byte of zero is data to fwrite.
    assert_eq!(guest.call("fwrite", &[text + 14, 1, 2, out]), 2);
    assert_eq!(guest.state.standard_output()[13..], [b'z', 0]);

    guest.set_errno(0);
    for args in [[0, 1, 4, out], [text, 1, 4, 0]] {
        assert_eq!(guest.call("fwrite", &args), 0, "{args:x?}");
        assert_eq!(guest.errno(), STDIO_EINVAL, "{args:x?}");
        guest.set_errno(0);
    }
    // stdin, another pointer, more than this runtime transfers at once, a
    // product that overflows, and unreadable memory.
    for args in [
        [text, 1, 4, input],
        [text, 1, 4, TEXT],
        [text, 1, (1 << 20) + 1, out],
        [text, 1 << 33, 1 << 33, out],
    ] {
        assert_eq!(guest.run("fwrite", &args).1, LIMIT, "{args:x?}");
    }
    let exit = guest.run("fwrite", &[0x9_0000_0000, 1, 4, out]).1;
    assert!(matches!(exit, ExitReason::MemoryFault(_)), "{exit:?}");
    assert_eq!(guest.state.standard_output().len(), 15);
    assert_eq!(guest.errno(), 0);

    // fputs writes up to the terminator and not the terminator itself.
    let (mut guest, [input, out, err]) = stdio_guest();
    let text = place(&mut guest, TEXT, b"cleanup ran\n");
    assert_eq!(guest.call("fputs", &[text, out]) & 0xFFFF_FFFF, 0);
    let empty = place(&mut guest, STRINGS, b"");
    assert_eq!(guest.call("fputs", &[empty, out]) & 0xFFFF_FFFF, 0);
    assert_eq!(guest.call("fputs", &[text + 8, err]) & 0xFFFF_FFFF, 0);
    assert_eq!(guest.state.standard_output(), b"cleanup ran\r\n");
    assert_eq!(guest.state.standard_error(), b"ran\r\n");
    guest.set_errno(0);
    for args in [[0, out], [text, 0]] {
        assert_eq!(guest.call("fputs", &args) & 0xFFFF_FFFF, STDIO_MINUS_ONE);
        assert_eq!(guest.errno(), STDIO_EINVAL, "{args:x?}");
        guest.set_errno(0);
    }
    for args in [[text, input], [text, TEXT]] {
        assert_eq!(guest.run("fputs", &args).1, LIMIT, "{args:x?}");
    }
    let exit = guest.run("fputs", &[0x9_0000_0000, out]).1;
    assert!(matches!(exit, ExitReason::MemoryFault(_)), "{exit:?}");
    assert_eq!(guest.state.standard_output().len(), 13);
}

/// `fflush` "returns 0 if the buffer was successfully flushed", and a null
/// stream means every open stream. `setvbuf` "returns 0 if successful"; for a
/// null stream or a mode or size that is not valid it returns -1 and sets
/// `errno` to `EINVAL`. The modes are `_IOFBF` (0), `_IOLBF` (0x40) and
/// `_IONBF` (4), and the size must be from 2 to `INT_MAX` unless the mode is
/// `_IONBF` (Microsoft Learn; values from stdio.h).
#[test]
fn fflush_and_setvbuf_return_their_documented_results() {
    let (mut guest, [input, out, err]) = stdio_guest();
    for stream in [0, input, out, err] {
        assert_eq!(
            guest.call("fflush", &[stream]) & 0xFFFF_FFFF,
            0,
            "{stream:#x}"
        );
    }
    for stream in [TEXT, out + 4, 16] {
        assert_eq!(guest.run("fflush", &[stream]).1, LIMIT, "{stream:#x}");
    }

    const INT_MAX: u64 = 0x7FFF_FFFF;
    let junk = 0xFFFF_FFFF_0000_0000;
    guest.set_errno(0);
    for (args, result) in [
        // The call the mingw-w64 startup makes: stderr, unbuffered.
        ([err, 0, 4, 0], 0),
        // No buffer is used, regardless of buffer or size.
        ([out, TEXT, 4, 1], 0),
        ([out, 0, 4, u64::MAX], 0),
        ([out, 0, 0, 4096], 0),
        ([out, TEXT, 0, 2], 0),
        ([input, 0, 0x40, INT_MAX], 0),
        // The mode is an int.
        ([out, 0, junk | 0x40, 2], 0),
        // A size outside 2 to INT_MAX, and modes that are not one of the three.
        ([out, 0, 0, 1], STDIO_MINUS_ONE),
        ([out, 0, 0, 0], STDIO_MINUS_ONE),
        ([out, 0, 0x40, INT_MAX + 1], STDIO_MINUS_ONE),
        ([out, 0, 1, 4096], STDIO_MINUS_ONE),
        ([out, 0, 0x44, 4096], STDIO_MINUS_ONE),
        ([out, 0, 0xFFFF_FFFF, 4096], STDIO_MINUS_ONE),
        ([0, 0, 4, 0], STDIO_MINUS_ONE),
    ] {
        assert_eq!(
            guest.call("setvbuf", &args) & 0xFFFF_FFFF,
            result,
            "{args:x?}"
        );
        let errno = if result == 0 { 0 } else { STDIO_EINVAL };
        assert_eq!(guest.errno(), errno, "{args:x?}");
        guest.set_errno(0);
    }
    assert_eq!(guest.run("setvbuf", &[TEXT, 0, 4, 0]).1, LIMIT);

    // Neither function holds output back or lets any through.
    assert!(guest.state.standard_output().is_empty());
    assert_eq!(print(&mut guest, out, b"now", &[]), 3);
    assert_eq!(guest.state.standard_output(), b"now");
}

/// Each captured stream keeps at most `MAX_CAPTURED_OUTPUT` bytes, counted
/// after the text-mode replacement, and a write that would pass it stops the
/// guest and adds nothing.
#[test]
fn captured_output_is_bounded() {
    let (mut guest, [_, out, err]) = stdio_guest();
    const CHUNK: u64 = 1 << 20;
    assert_eq!(MAX_CAPTURED_OUTPUT as u64, 16 * CHUNK);
    guest
        .memory
        .write_bytes(TEXT, &vec![b'x'; CHUNK as usize])
        .unwrap();
    for _ in 0..15 {
        assert_eq!(guest.call("fwrite", &[TEXT, 1, CHUNK, out]), CHUNK);
    }
    assert_eq!(guest.call("fwrite", &[TEXT, 1, CHUNK - 2, out]), CHUNK - 2);
    // Two bytes of room: a line feed takes both.
    guest.memory.write_bytes(TEXT, b"\n\nxx").unwrap();
    assert_eq!(guest.run("fwrite", &[TEXT, 1, 2, out]).1, LIMIT);
    assert_eq!(guest.run("fwrite", &[TEXT + 1, 1, 2, out]).1, LIMIT);
    assert_eq!(guest.state.standard_output().len(), MAX_CAPTURED_OUTPUT - 2);
    assert_eq!(guest.call("fwrite", &[TEXT, 1, 1, out]), 1);
    assert_eq!(guest.state.standard_output().len(), MAX_CAPTURED_OUTPUT);
    assert_eq!(guest.run("fwrite", &[TEXT + 2, 1, 1, out]).1, LIMIT);
    assert_eq!(guest.state.standard_output().len(), MAX_CAPTURED_OUTPUT);
    // The other stream has its own room.
    assert_eq!(guest.call("fwrite", &[TEXT + 2, 1, 2, err]), 2);
    assert_eq!(guest.state.standard_error(), b"xx");
}

const ENOENT: u32 = 2;
const EBADF: u32 = 9;
const EMFILE: u32 = 24;
const ENOSPC: u32 = 28;
/// Where the file tests put a name, a mode and the buffer `fgets` fills.
const NAME: u64 = TEXT + 0x1000;
const MODE: u64 = TEXT + 0x1200;
const LINE: u64 = TEXT + 0x2000;
/// What the buffer holds before `fgets` is called.
const UNTOUCHED: u8 = 0xEE;

/// `fopen(name, mode)`, with both strings placed in guest memory.
fn open(guest: &mut Guest, name: &str, mode: &str) -> u64 {
    let name = place(guest, NAME, name.as_bytes());
    let mode = place(guest, MODE, mode.as_bytes());
    guest.call("fopen", &[name, mode])
}

/// `fopen` that must succeed.
fn opened(guest: &mut Guest, name: &str, mode: &str) -> u64 {
    let stream = open(guest, name, mode);
    assert_ne!(stream, 0, "fopen({name:?}, {mode:?})");
    stream
}

fn close(guest: &mut Guest, stream: u64) {
    assert_eq!(guest.call("fclose", &[stream]) & 0xFFFF_FFFF, 0);
}

/// Write `text` to a stream with `fwrite`, one item per byte.
fn write(guest: &mut Guest, stream: u64, text: &[u8]) {
    guest.memory.write_bytes(TEXT + 0x4000, text).unwrap();
    let count = text.len() as u64;
    assert_eq!(
        guest.call("fwrite", &[TEXT + 0x4000, 1, count, stream]),
        count
    );
}

/// Create or replace a file with exactly these bytes.
fn store(guest: &mut Guest, name: &str, bytes: &[u8]) {
    let stream = opened(guest, name, "wb");
    write(guest, stream, bytes);
    close(guest, stream);
}

/// `fgets(LINE, count, stream)`: the string it stored, or `None` for a null
/// result. Either way, the bytes it had no reason to write must be untouched.
fn line(guest: &mut Guest, stream: u64, count: u64) -> Option<Vec<u8>> {
    let room = (count as usize).min(0x400) + 16;
    guest
        .memory
        .write_bytes(LINE, &vec![UNTOUCHED; room])
        .unwrap();
    let result = guest.call("fgets", &[LINE, count, stream]);
    let mut after = vec![0; room];
    guest.memory.read_bytes(LINE, &mut after).unwrap();
    if result == 0 {
        assert!(
            after.iter().all(|byte| *byte == UNTOUCHED),
            "buffer written"
        );
        return None;
    }
    assert_eq!(result, LINE, "fgets returns its buffer");
    let end = after
        .iter()
        .position(|byte| *byte == 0)
        .expect("terminator");
    assert!(end < count as usize, "more than count - 1 characters");
    assert!(after[end + 1..].iter().all(|byte| *byte == UNTOUCHED));
    Some(after[..end].to_vec())
}

/// Every line of a file, read with `fgets` in the given mode.
fn lines(guest: &mut Guest, name: &str, mode: &str) -> Vec<Vec<u8>> {
    let stream = opened(guest, name, mode);
    let mut all = Vec::new();
    while let Some(text) = line(guest, stream, 96) {
        all.push(text);
        // A read that does not advance must fail, not loop for ever.
        assert!(all.len() <= 64, "{name}: fgets never reaches the end");
    }
    close(guest, stream);
    all
}

/// What the Gate A2 program does with a file: `fopen` for writing, `fprintf`,
/// `fclose`, `fopen` for reading, `fgets`, `fclose`. `fgets` reads "to and
/// including the first newline character, to the end of the stream, or until
/// the number of characters read is equal to numChars - 1", and returns null
/// at end of file (Microsoft Learn). Expected text is what the test wrote.
#[test]
fn a_file_written_through_a_stream_reads_back_line_by_line() {
    let (mut guest, streams) = stdio_guest();
    let file = opened(&mut guest, "gate-a2.txt", "w");
    assert!(!streams.contains(&file));
    let one = place(&mut guest, STRINGS, b"one");
    let first: &[u8] = b"sum 12345 hex a2 text one\n";
    let count = print(
        &mut guest,
        file,
        b"sum %llu hex %x text %s\n",
        &[12345, 0xa2, one],
    );
    assert_eq!(count, first.len() as u64);
    let second = place(&mut guest, STRINGS + 0x40, b"second line\n");
    assert_eq!(guest.call("fputs", &[second, file]) & 0xFFFF_FFFF, 0);
    write(&mut guest, file, b"third");
    assert_eq!(guest.call("fflush", &[file]) & 0xFFFF_FFFF, 0);
    close(&mut guest, file);
    // File output is not standard output.
    assert!(guest.state.standard_output().is_empty());
    assert!(guest.state.standard_error().is_empty());

    let file = opened(&mut guest, "gate-a2.txt", "r");
    assert_eq!(line(&mut guest, file, 96).as_deref(), Some(first));
    assert_eq!(
        line(&mut guest, file, 96).as_deref(),
        Some(&b"second line\n"[..])
    );
    // The last line has no newline: it ends at the end of the stream.
    assert_eq!(line(&mut guest, file, 96).as_deref(), Some(&b"third"[..]));
    assert_eq!(line(&mut guest, file, 96), None);
    assert_eq!(line(&mut guest, file, 96), None);
    close(&mut guest, file);
    assert_eq!(guest.errno(), 0);
}

/// The access types of the `fopen` page: "r" fails "if the file doesn't
/// exist"; with "w", "if the given file exists, its contents are destroyed";
/// "a" "creates the file if it doesn't exist" and "all write operations occur
/// at the end of the file"; "r+" needs the file to exist; "w+" and "a+" also
/// read. A null or empty name or mode returns null with `EINVAL`.
#[test]
fn fopen_modes_follow_the_documentation() {
    let (mut guest, _) = stdio_guest();
    guest.set_errno(0);
    for mode in ["r", "r+", "rb", "rt"] {
        assert_eq!(open(&mut guest, "missing.txt", mode), 0, "{mode}");
        assert_eq!(guest.errno(), ENOENT, "{mode}");
        guest.set_errno(0);
    }

    // "w" creates the file, and opening it again destroys what it held.
    store(&mut guest, "w.txt", b"old contents\n");
    assert_eq!(lines(&mut guest, "w.txt", "rb"), [b"old contents\n"]);
    let file = opened(&mut guest, "w.txt", "w");
    close(&mut guest, file);
    assert!(lines(&mut guest, "w.txt", "rb").is_empty());

    // "a" creates the file, keeps what it holds and writes at its end.
    for text in [&b"1\n"[..], b"2\n"] {
        let file = opened(&mut guest, "a.txt", "ab");
        write(&mut guest, file, text);
        close(&mut guest, file);
    }
    assert_eq!(lines(&mut guest, "a.txt", "rb"), [b"1\n", b"2\n"]);

    // "r+" keeps the contents and writes from the start.
    store(&mut guest, "plus.txt", b"abcdef\n");
    let file = opened(&mut guest, "plus.txt", "r+b");
    write(&mut guest, file, b"XY");
    // Reading continues where the write left off.
    assert_eq!(line(&mut guest, file, 96).as_deref(), Some(&b"cdef\n"[..]));
    close(&mut guest, file);
    assert_eq!(lines(&mut guest, "plus.txt", "rb"), [b"XYcdef\n"]);

    // "w+" may be read; right after a write it is at the end of the file. A
    // "w" stream may not be read at all.
    let file = opened(&mut guest, "wplus.txt", "w+b");
    write(&mut guest, file, b"data\n");
    assert_eq!(line(&mut guest, file, 96), None);
    assert_eq!(guest.errno(), 0);
    close(&mut guest, file);
    let file = opened(&mut guest, "wplus.txt", "w");
    assert_eq!(line(&mut guest, file, 96), None);
    assert_eq!(guest.errno(), EBADF);
    guest.set_errno(0);
    close(&mut guest, file);

    // "a+" reads from the start (Xenora's choice; ISO C leaves the initial
    // position to the implementation) and still writes at the end.
    store(&mut guest, "aplus.txt", b"one\ntwo\n");
    let file = opened(&mut guest, "aplus.txt", "a+b");
    assert_eq!(line(&mut guest, file, 96).as_deref(), Some(&b"one\n"[..]));
    write(&mut guest, file, b"three\n");
    close(&mut guest, file);
    assert_eq!(
        lines(&mut guest, "aplus.txt", "rb"),
        [&b"one\n"[..], b"two\n", b"three\n"]
    );

    // One file by any spelling the drive accepts: either slash, any case,
    // with or without the drive.
    store(&mut guest, "Name.TXT", b"same\n");
    for name in ["name.txt", "NAME.txt", "C:\\name.txt", "c:/Name.Txt"] {
        assert_eq!(lines(&mut guest, name, "r"), [b"same\n"], "{name}");
    }

    // Null and empty names and modes, modes that are not modes, a name the
    // drive rejects, and a name that is too long.
    guest.set_errno(0);
    let name = place(&mut guest, NAME, b"w.txt");
    let mode = place(&mut guest, MODE, b"r");
    for args in [[0, mode], [name, 0]] {
        assert_eq!(guest.call("fopen", &args), 0, "{args:x?}");
        assert_eq!(guest.errno(), STDIO_EINVAL, "{args:x?}");
        guest.set_errno(0);
    }
    let long = "x".repeat(261);
    for (name, mode) in [
        ("", "r"),
        ("w.txt", ""),
        ("w.txt", "tr"),
        ("w.txt", "br"),
        ("w.txt", "rw"),
        ("w.txt", "rbt"),
        ("w.txt", "r++"),
        ("w.txt", "wx"),
        ("w.txt", "r,ccs=UTF-8"),
        ("w.txt", "R"),
        ("bad*name", "w"),
        ("a\\..\\b", "w"),
        ("con", "w"),
        (long.as_str(), "w"),
    ] {
        assert_eq!(open(&mut guest, name, mode), 0, "{name:?} {mode:?}");
        assert_eq!(guest.errno(), STDIO_EINVAL, "{name:?} {mode:?}");
        guest.set_errno(0);
    }
    // A refused "w" destroyed nothing.
    assert_eq!(lines(&mut guest, "plus.txt", "rb"), [b"XYcdef\n"]);
    // A directory that does not exist.
    for mode in ["w", "a", "r"] {
        assert_eq!(open(&mut guest, "nodir\\file.txt", mode), 0, "{mode}");
        assert_eq!(guest.errno(), ENOENT, "{mode}");
        guest.set_errno(0);
    }
}

/// "LF characters are translated to CR-LF combinations on output", "CR-LF
/// combinations are translated into single line feeds on input", and "CTRL+Z
/// is interpreted as an end-of-file character on input" in text mode; binary
/// mode suppresses the translations. Without `t` or `b` the mode is text,
/// the initial `_fmode` (Microsoft Learn, fopen).
#[test]
fn text_and_binary_streams_translate_as_documented() {
    let (mut guest, _) = stdio_guest();
    for mode in ["w", "wt"] {
        let file = opened(&mut guest, "text.txt", mode);
        write(&mut guest, file, b"a\nb\n");
        // fwrite's result did not count the carriage returns.
        close(&mut guest, file);
        // Read back untranslated, the file holds CR-LF pairs.
        assert_eq!(
            lines(&mut guest, "text.txt", "rb"),
            [b"a\r\n", b"b\r\n"],
            "{mode}"
        );
        for read in ["r", "rt"] {
            assert_eq!(lines(&mut guest, "text.txt", read), [b"a\n", b"b\n"]);
        }
    }
    // Formatted output and fputs are translated like fwrite.
    let file = opened(&mut guest, "text.txt", "w");
    assert_eq!(print(&mut guest, file, b"%d\n", &[7]), 2);
    let text = place(&mut guest, STRINGS, b"s\n");
    assert_eq!(guest.call("fputs", &[text, file]) & 0xFFFF_FFFF, 0);
    close(&mut guest, file);
    assert_eq!(lines(&mut guest, "text.txt", "rb"), [b"7\r\n", b"s\r\n"]);

    // Binary output is stored as written.
    store(&mut guest, "binary.bin", b"x\ny");
    assert_eq!(lines(&mut guest, "binary.bin", "rb"), [&b"x\n"[..], b"y"]);

    // CTRL+Z ends a text stream and is data to a binary one. A carriage
    // return that no line feed follows is kept.
    store(&mut guest, "ctrlz.bin", b"one\r\ntw\ro\x1Athree\n");
    assert_eq!(
        lines(&mut guest, "ctrlz.bin", "r"),
        [&b"one\n"[..], b"tw\ro"]
    );
    assert_eq!(
        lines(&mut guest, "ctrlz.bin", "rb"),
        [&b"one\r\n"[..], b"tw\ro\x1Athree\n"]
    );
    // A text stream that begins with CTRL+Z has nothing to read.
    store(&mut guest, "empty.bin", b"\x1Arest");
    assert!(lines(&mut guest, "empty.bin", "r").is_empty());
}

/// `fgets` stores at most `numChars - 1` characters and a terminator, and
/// "if str or stream is a null pointer, or numChars is less than or equal to
/// zero", sets `errno` to `EINVAL` and returns null (Microsoft Learn).
#[test]
fn fgets_reads_lines_within_its_count() {
    let (mut guest, [input, out, _]) = stdio_guest();
    store(&mut guest, "lines.bin", b"abcdefgh\nxy");
    let file = opened(&mut guest, "lines.bin", "rb");
    assert_eq!(line(&mut guest, file, 4).as_deref(), Some(&b"abc"[..]));
    assert_eq!(line(&mut guest, file, 4).as_deref(), Some(&b"def"[..]));
    assert_eq!(line(&mut guest, file, 2).as_deref(), Some(&b"g"[..]));
    // A count of 1 has room for the terminator only and reads nothing.
    assert_eq!(line(&mut guest, file, 1).as_deref(), Some(&b""[..]));
    // The newline is kept, and reading stops after it. The count is an int.
    let count = 0xFFFF_FFFF_0000_0060;
    assert_eq!(
        line(&mut guest, file, count & 0xFFFF_FFFF).as_deref(),
        Some(&b"h\n"[..])
    );
    guest.memory.write_bytes(LINE, &[UNTOUCHED; 8]).unwrap();
    assert_eq!(guest.call("fgets", &[LINE, count, file]), LINE);
    assert_eq!(
        guest.memory.read_u32(LINE).unwrap(),
        u32::from_le_bytes(*b"xy\0\xEE")
    );
    // At the end of the file every count returns null.
    for count in [96, 2, 1] {
        assert_eq!(line(&mut guest, file, count), None, "{count}");
    }
    assert_eq!(guest.errno(), 0);

    guest.set_errno(0);
    for args in [
        [0, 96, file],
        [LINE, 96, 0],
        [LINE, 0, file],
        [LINE, 0xFFFF_FFFF, file],
        [LINE, 0x8000_0000, file],
    ] {
        assert_eq!(guest.call("fgets", &args), 0, "{args:x?}");
        assert_eq!(guest.errno(), STDIO_EINVAL, "{args:x?}");
        guest.set_errno(0);
    }
    close(&mut guest, file);

    // A stream that was not opened for reading, and standard output.
    let writer = opened(&mut guest, "lines.bin", "ab");
    for stream in [writer, out] {
        assert_eq!(guest.call("fgets", &[LINE, 96, stream]), 0);
        assert_eq!(guest.errno(), EBADF);
        guest.set_errno(0);
    }
    close(&mut guest, writer);
    // Standard input, a closed stream and a pointer that never was one.
    for stream in [input, writer, TEXT] {
        assert_eq!(
            guest.run("fgets", &[LINE, 96, stream]).1,
            LIMIT,
            "{stream:#x}"
        );
    }
    // A buffer that cannot be written is a fault.
    let file = opened(&mut guest, "lines.bin", "rb");
    let exit = guest.run("fgets", &[0x9_0000_0000, 96, file]).1;
    assert!(matches!(exit, ExitReason::MemoryFault(_)), "{exit:?}");
}

/// Output to a stream that was opened only for reading fails with the error
/// result of each function, and so does output the drive has no room for.
/// The `errno` values are Xenora's choice: `EBADF` and `ENOSPC`.
#[test]
fn file_output_that_cannot_be_written_fails_and_writes_nothing() {
    let (mut guest, _) = stdio_guest();
    store(&mut guest, "fixed.bin", b"keep\n");
    let file = opened(&mut guest, "fixed.bin", "rb");
    guest.set_errno(0);
    let (count, exit) = print_with(&mut guest, MINGW_OPTIONS, file, b"%d", &[1]);
    assert_eq!((count, exit), (STDIO_MINUS_ONE, ExitReason::Halt));
    assert_eq!(guest.errno(), EBADF);
    guest.set_errno(0);
    let text = place(&mut guest, STRINGS, b"text");
    assert_eq!(guest.call("fwrite", &[text, 1, 4, file]), 0);
    assert_eq!(guest.errno(), EBADF);
    guest.set_errno(0);
    assert_eq!(
        guest.call("fputs", &[text, file]) & 0xFFFF_FFFF,
        STDIO_MINUS_ONE
    );
    assert_eq!(guest.errno(), EBADF);
    guest.set_errno(0);
    close(&mut guest, file);
    assert_eq!(lines(&mut guest, "fixed.bin", "rb"), [b"keep\n"]);

    // The drive holds 16 MiB in all. Fifteen whole megabytes and most of the
    // sixteenth fit beside the five bytes above.
    const CHUNK: u64 = 1 << 20;
    guest
        .memory
        .write_bytes(TEXT, &vec![b'x'; CHUNK as usize])
        .unwrap();
    let file = opened(&mut guest, "big.bin", "wb");
    for _ in 0..15 {
        assert_eq!(guest.call("fwrite", &[TEXT, 1, CHUNK, file]), CHUNK);
    }
    assert_eq!(guest.call("fwrite", &[TEXT, 1, CHUNK - 5, file]), CHUNK - 5);
    assert_eq!(guest.errno(), 0);
    // Not one more byte, through any of the three.
    assert_eq!(guest.call("fwrite", &[TEXT, 1, 1, file]), 0);
    assert_eq!(guest.errno(), ENOSPC);
    guest.set_errno(0);
    let (count, _) = print_with(&mut guest, MINGW_OPTIONS, file, b"y", &[]);
    assert_eq!(count, STDIO_MINUS_ONE);
    assert_eq!(guest.errno(), ENOSPC);
    guest.set_errno(0);
    close(&mut guest, file);
    // Emptying the big file makes room again.
    let file = opened(&mut guest, "big.bin", "wb");
    assert_eq!(guest.call("fwrite", &[TEXT, 1, 1, file]), 1);
    close(&mut guest, file);
}

/// `fclose` "returns 0 if the stream is successfully closed" and, for a null
/// stream, sets `errno` to `EINVAL` and returns `EOF` (Microsoft Learn). At
/// most 64 streams are open at once (Xenora's bound), and the 65th `fopen`
/// fails with `EMFILE`.
#[test]
fn open_streams_are_bounded_and_fclose_releases_them() {
    let (mut guest, [input, out, err]) = stdio_guest();
    store(&mut guest, "shared.bin", b"x\n");
    let mut open_streams: Vec<u64> = (0..64)
        .map(|_| opened(&mut guest, "shared.bin", "rb"))
        .collect();
    let mut distinct = open_streams.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), 64, "each stream has its own FILE object");
    guest.set_errno(0);
    for mode in ["rb", "wb", "ab"] {
        assert_eq!(open(&mut guest, "shared.bin", mode), 0, "{mode}");
        assert_eq!(guest.errno(), EMFILE, "{mode}");
        guest.set_errno(0);
    }
    // The refused "wb" destroyed nothing, and each stream has its own
    // position.
    assert_eq!(
        line(&mut guest, open_streams[0], 96).as_deref(),
        Some(&b"x\n"[..])
    );
    assert_eq!(line(&mut guest, open_streams[0], 96), None);
    assert_eq!(
        line(&mut guest, open_streams[1], 96).as_deref(),
        Some(&b"x\n"[..])
    );

    // Closing one makes room for one.
    let last = open_streams.pop().unwrap();
    close(&mut guest, last);
    let again = opened(&mut guest, "shared.bin", "rb");
    assert_eq!(open(&mut guest, "shared.bin", "rb"), 0);
    assert_eq!(guest.errno(), EMFILE);
    guest.set_errno(0);

    // fflush and setvbuf accept an open file stream.
    assert_eq!(guest.call("fflush", &[again]) & 0xFFFF_FFFF, 0);
    assert_eq!(guest.call("setvbuf", &[again, 0, 4, 0]) & 0xFFFF_FFFF, 0);

    assert_eq!(guest.call("fclose", &[0]) & 0xFFFF_FFFF, STDIO_MINUS_ONE);
    assert_eq!(guest.errno(), STDIO_EINVAL);
    guest.set_errno(0);
    // A closed stream is no stream to any function, and closing a standard
    // stream is not modelled.
    close(&mut guest, again);
    for symbol in ["fclose", "fflush"] {
        assert_eq!(guest.run(symbol, &[again]).1, LIMIT, "{symbol}");
    }
    let (_, exit) = print_with(&mut guest, MINGW_OPTIONS, again, b"x", &[]);
    assert_eq!(exit, LIMIT);
    for stream in [input, out, err, TEXT] {
        assert_eq!(guest.run("fclose", &[stream]).1, LIMIT, "{stream:#x}");
    }
    for stream in open_streams {
        close(&mut guest, stream);
    }
    assert_eq!(guest.errno(), 0);
}

/// `perror` "prints an error message to stderr": the message "followed by a
/// colon, then by the system error message for the last library call that
/// produced the error, and finally by a newline character"; "if message is a
/// null pointer or a pointer to a null string, perror prints only the system
/// error message". The page's example prints "No such file or directory"
/// for a missing file; the other texts are Xenora's.
#[test]
fn perror_writes_the_message_and_the_error_text_to_stderr() {
    let (mut guest, _) = stdio_guest();
    guest.set_errno(0);
    // The failure the Gate A2 program would report.
    assert_eq!(open(&mut guest, "nosuchf.ile", "r"), 0);
    let message = place(&mut guest, STRINGS, b"perror says open failed");
    guest.call("perror", &[message]);
    assert_eq!(
        guest.state.standard_error(),
        b"perror says open failed: No such file or directory\r\n"
    );
    // perror does not change errno.
    assert_eq!(guest.errno(), ENOENT);
    assert!(guest.state.standard_output().is_empty());

    let empty = place(&mut guest, STRINGS + 0x80, b"");
    let mut expected = guest.state.standard_error();
    for (errno, message, text) in [
        (ENOENT, 0, &b"No such file or directory\r\n"[..]),
        (ENOENT, empty, b"No such file or directory\r\n"),
        (0, message, b"perror says open failed: No error\r\n"),
        (STDIO_EINVAL, 0, b"Invalid argument\r\n"),
        (EBADF, 0, b"Bad file descriptor\r\n"),
        (EMFILE, 0, b"Too many open files\r\n"),
        (ENOSPC, 0, b"No space left on device\r\n"),
        (13, 0, b"Permission denied\r\n"),
        (12, 0, b"Not enough space\r\n"),
        (999, 0, b"Unknown error\r\n"),
    ] {
        guest.set_errno(errno);
        guest.call("perror", &[message]);
        expected.extend_from_slice(text);
        assert_eq!(guest.state.standard_error(), expected, "errno {errno}");
        assert_eq!(guest.errno(), errno);
    }
    // An unreadable message is a fault and prints nothing.
    let exit = guest.run("perror", &[0x9_0000_0000]).1;
    assert!(matches!(exit, ExitReason::MemoryFault(_)), "{exit:?}");
    assert_eq!(guest.state.standard_error(), expected);
}

/// The module and symbol pairs a program built with mingw-w64 14.0.0 (UCRT)
/// imports the four file services by.
#[test]
fn file_services_resolve_as_mingw_w64_imports_them() {
    let (registry, _) = standard(0);
    let named = |name: &str| ImportName::Name(name.to_owned());
    const STDIO: &str = "api-ms-win-crt-stdio-l1-1-0.dll";
    const RUNTIME: &str = "api-ms-win-crt-runtime-l1-1-0.dll";
    for (module, symbol) in [
        (STDIO, "fopen"),
        (STDIO, "fclose"),
        (STDIO, "fgets"),
        (RUNTIME, "perror"),
    ] {
        let service = registry
            .resolve(module, &named(symbol))
            .unwrap_or_else(|| panic!("{module}!{symbol}"));
        let target = registry.resolve("ucrtbase.dll", &named(symbol)).unwrap();
        assert!(std::ptr::eq(service, target), "{symbol}");
        assert_eq!(service.status, ServiceStatus::Partial, "{symbol}");
    }
    assert!(registry.resolve(RUNTIME, &named("fopen")).is_none());
    assert!(registry.resolve(STDIO, &named("perror")).is_none());
}

/// The module and symbol pairs a program built with mingw-w64 14.0.0 (UCRT)
/// imports these six services by, read from such a program's import table.
#[test]
fn standard_stream_services_resolve_as_mingw_w64_imports_them() {
    let (registry, _) = standard(0);
    let named = |name: &str| ImportName::Name(name.to_owned());
    const STDIO: &str = "api-ms-win-crt-stdio-l1-1-0.dll";
    for symbol in [
        "__acrt_iob_func",
        "__stdio_common_vfprintf",
        "fwrite",
        "fputs",
        "fflush",
        "setvbuf",
    ] {
        let service = registry
            .resolve(STDIO, &named(symbol))
            .unwrap_or_else(|| panic!("{symbol}"));
        let target = registry.resolve("ucrtbase.dll", &named(symbol)).unwrap();
        assert!(std::ptr::eq(service, target), "{symbol}");
        assert_eq!(service.status, ServiceStatus::Partial, "{symbol}");
        // Not through the runtime API set, which lists other symbols.
        assert!(
            registry
                .resolve("api-ms-win-crt-runtime-l1-1-0.dll", &named(symbol))
                .is_none(),
            "{symbol}"
        );
    }
}

#[test]
fn last_error_is_set_by_failures_and_by_the_guest() {
    let mut guest = Guest::new();
    assert_eq!(guest.last_error(), 0);
    guest.call("SetLastError", &[0xFFFF_FFFF_8000_0001]);
    assert_eq!(guest.last_error(), 0x8000_0001);
    assert_eq!(guest.call("GetCurrentProcess", &[]), PROCESS);
    assert_eq!(
        guest.last_error(),
        0x8000_0001,
        "success leaves it unchanged"
    );
    guest.call("CloseHandle", &[0x1234]);
    assert_eq!(guest.last_error(), 6);
}

/// What a guest sees in `__argc` and `__argv`: the count, each argument's
/// bytes up to its terminator, and the pointer after the last argument.
fn guest_arguments(guest: &mut Guest) -> (u32, Vec<Vec<u8>>, u64) {
    let argc = guest.call("__p___argc", &[]);
    let argv = guest.call("__p___argv", &[]);
    let count = guest.memory.read_u32(argc).unwrap();
    let table = guest.memory.read_u64(argv).unwrap();
    let mut arguments = Vec::new();
    for index in 0..u64::from(count) {
        let mut at = guest.memory.read_u64(table + 8 * index).unwrap();
        let mut bytes = Vec::new();
        loop {
            match guest.memory.read_u8(at).unwrap() {
                0 => break,
                byte => bytes.push(byte),
            }
            at += 1;
        }
        arguments.push(bytes);
    }
    let after = guest.memory.read_u64(table + 8 * u64::from(count)).unwrap();
    (count, arguments, after)
}

/// The arguments a host supplies are what the guest finds in `__argc` and
/// `__argv` after `_configure_narrow_argv`, the way the mingw-w64 startup code
/// reads them before `main`. `__argc` and `__argv` are documented globals; ISO
/// C (5.1.2.2.1) puts a null pointer after the last argument, lets the program
/// modify the strings, and makes `argv[0]` the empty string when no program
/// name is available. Expected values are the strings this test supplied.
#[test]
fn program_arguments_reach_the_guest_as_argc_and_argv() {
    let mut guest = Guest::new();
    let supplied = ["gate-a2.exe", "one", "two words", "", "caf\u{e9} \u{2713}"];
    guest.state.set_program_arguments(&supplied).unwrap();

    // Before the runtime configures them there are no arguments.
    let argc = guest.call("__p___argc", &[]);
    let argv = guest.call("__p___argv", &[]);
    assert!(argc != 0 && argv != 0 && argc.abs_diff(argv) >= 8);
    assert_eq!(guest.memory.read_u32(argc).unwrap(), 0);
    assert_eq!(guest.memory.read_u64(argv).unwrap(), 0);
    // A mode other than 0, 1 or 2 is refused with EINVAL; mode 0 asks for no
    // arguments. Neither builds anything. The mode is an int.
    for (mode, result) in [(3, 22), (0xFFFF_FFFF, 22), (0x8000_0001, 22), (0, 0)] {
        let returned = guest.call("_configure_narrow_argv", &[JUNK_HIGH | mode]);
        assert_eq!(returned & 0xFFFF_FFFF, result, "mode {mode:#x}");
        assert_eq!(guest.memory.read_u32(argc).unwrap(), 0, "mode {mode:#x}");
        assert_eq!(guest.memory.read_u64(argv).unwrap(), 0, "mode {mode:#x}");
    }

    // _crt_argv_unexpanded_arguments is 1.
    assert_eq!(
        guest.call("_configure_narrow_argv", &[JUNK_HIGH | 1]) & 0xFFFF_FFFF,
        0
    );
    let (count, arguments, after) = guest_arguments(&mut guest);
    assert_eq!(count, 5);
    let wanted: Vec<Vec<u8>> = supplied
        .iter()
        .map(|text| text.as_bytes().to_vec())
        .collect();
    assert_eq!(arguments, wanted);
    assert_eq!(after, 0, "argv[argc] is a null pointer");
    // The variables did not move, and each argument has its own storage.
    assert_eq!(guest.call("__p___argc", &[]), argc);
    assert_eq!(guest.call("__p___argv", &[]), argv);
    let table = guest.memory.read_u64(argv).unwrap();
    let mut starts: Vec<u64> = (0..5)
        .map(|index| guest.memory.read_u64(table + 8 * index).unwrap())
        .collect();
    for (start, argument) in starts.iter().zip(&wanted) {
        assert!(*start >= table + 8 * 6, "a string inside the pointer table");
        // The terminator is the byte after the argument.
        assert_eq!(
            guest.memory.read_u8(start + argument.len() as u64).unwrap(),
            0
        );
    }
    starts.sort_unstable();
    for (pair, argument) in starts.windows(2).zip(&wanted) {
        assert!(pair[1] - pair[0] > argument.len() as u64, "strings overlap");
    }

    // The program may modify its arguments; a second configuration, in either
    // mode, does not rebuild them.
    let first = guest.memory.read_u64(table).unwrap();
    guest.memory.write_u8(first, b'G').unwrap();
    for mode in [1, 2, 0, 3] {
        guest.call("_configure_narrow_argv", &[mode]);
        assert_eq!(guest.memory.read_u64(argv).unwrap(), table, "mode {mode}");
        assert_eq!(guest.memory.read_u8(first).unwrap(), b'G', "mode {mode}");
        assert_eq!(guest.memory.read_u32(argc).unwrap(), 5, "mode {mode}");
    }
    // Once built, the host can no longer change them.
    assert_eq!(
        guest.state.set_program_arguments(&["late"]),
        Err(ProgramArgumentsError::AlreadyConfigured)
    );
    assert_eq!(guest_arguments(&mut guest).0, 5);
}

/// What a guest gets when the host supplies nothing, supplies no arguments at
/// all, or uses `_crt_argv_expanded_arguments` (2), and what a host may not
/// supply.
#[test]
fn program_arguments_have_defaults_and_limits() {
    // No host input: the program name is not available, so ISO C makes
    // argv[0] the empty string.
    let mut guest = Guest::new();
    assert_eq!(guest.call("_configure_narrow_argv", &[1]), 0);
    assert_eq!(guest_arguments(&mut guest), (1, vec![Vec::new()], 0));

    // An empty list is zero arguments: argv is one null pointer.
    let mut guest = Guest::new();
    guest.state.set_program_arguments(&[]).unwrap();
    assert_eq!(guest.call("_configure_narrow_argv", &[1]), 0);
    assert_eq!(guest_arguments(&mut guest), (0, Vec::new(), 0));
    let argv = guest.call("__p___argv", &[]);
    assert_ne!(guest.memory.read_u64(argv).unwrap(), 0);

    // Mode 2 asks for wildcard expansion, which is not performed: a pattern
    // arrives as it was supplied. The last list supplied before configuration
    // is the one used, and configuring first and asking for the pointers
    // afterwards works too.
    let mut guest = Guest::new();
    guest.state.set_program_arguments(&["first"]).unwrap();
    guest
        .state
        .set_program_arguments(&["prog", "*.txt", "a?c"])
        .unwrap();
    assert_eq!(guest.call("_configure_narrow_argv", &[2]), 0);
    let wanted = vec![b"prog".to_vec(), b"*.txt".to_vec(), b"a?c".to_vec()];
    assert_eq!(guest_arguments(&mut guest), (3, wanted, 0));

    // A NUL byte would end the argument early in the guest.
    let guest = Guest::new();
    let state = &guest.state;
    assert_eq!(
        state.set_program_arguments(&["ok", "a\0b"]),
        Err(ProgramArgumentsError::ContainsNul)
    );
    // The bound counts each argument with its terminator, and is inclusive.
    assert_eq!(MAX_PROGRAM_ARGUMENT_BYTES, 32_767);
    let longest = "x".repeat(MAX_PROGRAM_ARGUMENT_BYTES - 1);
    assert_eq!(state.set_program_arguments(&[&longest]), Ok(()));
    let too_long = "x".repeat(MAX_PROGRAM_ARGUMENT_BYTES);
    assert_eq!(
        state.set_program_arguments(&[&too_long]),
        Err(ProgramArgumentsError::TooLong)
    );
    // Two arguments of 16,383 and 16,382 bytes take 32,767 with their
    // terminators; one more empty argument is one byte too many.
    let (half, rest) = ("y".repeat(16_383), "z".repeat(16_382));
    assert_eq!(state.set_program_arguments(&[&half, &rest]), Ok(()));
    assert_eq!(
        state.set_program_arguments(&[&half, &rest, ""]),
        Err(ProgramArgumentsError::TooLong)
    );
    // A refused list replaces nothing: the last accepted one is still used.
    let mut guest = guest;
    assert_eq!(guest.call("_configure_narrow_argv", &[1]), 0);
    let (count, arguments, after) = guest_arguments(&mut guest);
    assert_eq!((count, after), (2, 0));
    assert_eq!(arguments, [half.as_bytes(), rest.as_bytes()]);
}

/// `_environ` is a documented global: a pointer to the array of environment
/// strings. Xenora's environment is empty, so after
/// `_initialize_narrow_environment` the table is one null pointer, the usual
/// end of such a table. Before that call `_environ` is null.
#[test]
fn the_environment_table_is_empty_and_ends_with_a_null_pointer() {
    let mut guest = Guest::new();
    let environ = guest.call("__p__environ", &[]);
    assert_ne!(environ, 0);
    assert_eq!(guest.memory.read_u64(environ).unwrap(), 0);
    assert_eq!(guest.call("_initialize_narrow_environment", &[]), 0);
    let table = guest.memory.read_u64(environ).unwrap();
    assert_ne!(table, 0);
    assert_eq!(guest.memory.read_u64(table).unwrap(), 0);
    // A second call keeps the table, and the pointer does not move.
    assert_eq!(guest.call("_initialize_narrow_environment", &[]), 0);
    assert_eq!(guest.call("__p__environ", &[]), environ);
    assert_eq!(guest.memory.read_u64(environ).unwrap(), table);

    // The three variables are separate, and neither initialization touches
    // the other's: the order the mingw-w64 startup uses is environment first.
    let argc = guest.call("__p___argc", &[]);
    let argv = guest.call("__p___argv", &[]);
    let mut cells = [argc, argv, environ];
    cells.sort_unstable();
    assert!(cells[1] - cells[0] >= 8 && cells[2] - cells[1] >= 8);
    assert_eq!(guest.memory.read_u32(argc).unwrap(), 0);
    assert_eq!(guest.memory.read_u64(argv).unwrap(), 0);
    assert_eq!(guest.call("_configure_narrow_argv", &[1]), 0);
    assert_eq!(guest.memory.read_u64(environ).unwrap(), table);
    assert_eq!(guest.memory.read_u64(table).unwrap(), 0);
    assert_eq!(guest_arguments(&mut guest).0, 1);

    // Arguments first, environment second, on a fresh guest.
    let mut other = Guest::new();
    assert_eq!(other.call("_configure_narrow_argv", &[1]), 0);
    let environ = other.call("__p__environ", &[]);
    assert_eq!(other.memory.read_u64(environ).unwrap(), 0);
    assert_eq!(other.call("_initialize_narrow_environment", &[]), 0);
    let table = other.memory.read_u64(environ).unwrap();
    assert_eq!(other.memory.read_u64(table).unwrap(), 0);
    assert_eq!(guest_arguments(&mut other), (1, vec![Vec::new()], 0));
}

/// The module and symbol pairs a program built with mingw-w64 14.0.0 (UCRT)
/// imports these five services by, read from such a program's import table.
#[test]
fn program_argument_services_resolve_as_mingw_w64_imports_them() {
    let (registry, _) = standard(0);
    let named = |name: &str| ImportName::Name(name.to_owned());
    const RUNTIME: &str = "api-ms-win-crt-runtime-l1-1-0.dll";
    const ENVIRONMENT: &str = "api-ms-win-crt-environment-l1-1-0.dll";
    for (module, symbol) in [
        (RUNTIME, "__p___argc"),
        (RUNTIME, "__p___argv"),
        (ENVIRONMENT, "__p__environ"),
        (RUNTIME, "_configure_narrow_argv"),
        (RUNTIME, "_initialize_narrow_environment"),
    ] {
        let service = registry
            .resolve(module, &named(symbol))
            .unwrap_or_else(|| panic!("{module}!{symbol}"));
        let target = registry.resolve("ucrtbase.dll", &named(symbol)).unwrap();
        assert!(std::ptr::eq(service, target), "{module}!{symbol}");
        assert_eq!(service.status, ServiceStatus::Partial, "{symbol}");
    }
    // An API set forwards only the symbols listed for it.
    assert!(
        registry
            .resolve(ENVIRONMENT, &named("__p___argc"))
            .is_none()
    );
    assert!(registry.resolve(RUNTIME, &named("__p__environ")).is_none());
}

#[test]
fn virtual_memory_is_mapped_released_and_bounded() {
    let mut guest = Guest::new();
    const COMMIT_RESERVE: u64 = 0x3000;
    const RELEASE: u64 = 0x8000;
    let address = guest.call("VirtualAlloc", &[0, 5000, COMMIT_RESERVE, 0x04]);
    assert!(address != 0 && address.is_multiple_of(0x1_0000));
    assert_eq!(
        guest.memory.read_u64(address + 8184).unwrap(),
        0,
        "zeroed, whole pages"
    );
    assert!(guest.memory.read_u8(address + 8192).is_err());
    guest.memory.write_u64(address, 7).unwrap();
    for (args, error) in [
        ([0, 0, COMMIT_RESERVE, 0x04], 87),
        ([0, 4096, 0x1000, 0x04], 50),
        ([0, 4096, COMMIT_RESERVE, 0x40], 87),
        ([address, 4096, COMMIT_RESERVE, 0x04], 487),
        ([0, u64::MAX, COMMIT_RESERVE, 0x04], 87),
        ([0, 1 << 40, COMMIT_RESERVE, 0x04], 8),
    ] {
        assert_eq!(guest.call("VirtualAlloc", &args), 0, "{args:x?}");
        assert_eq!(guest.last_error(), error, "{args:x?}");
    }
    for (args, error) in [
        ([address, 4096, RELEASE], 87),
        ([address, 0, 0x4000], 50),
        ([address + 4096, 0, RELEASE], 487),
    ] {
        assert_eq!(guest.call("VirtualFree", &args), 0, "{args:x?}");
        assert_eq!(guest.last_error(), error, "{args:x?}");
    }
    assert_eq!(guest.call("VirtualFree", &[address, 0, RELEASE]), 1);
    assert!(guest.memory.read_u8(address).is_err());
    assert_eq!(guest.call("VirtualFree", &[address, 0, RELEASE]), 0);
}

/// `VirtualProtect`, `VirtualQuery` and `__C_specific_handler` are stubs: a
/// hosted C program imports them and does not call them on a normal run. A
/// call stops the guest with the runtime-limit fault. The service writes
/// nothing: not the output buffers, not a result, not the last-error value.
#[test]
fn memory_and_exception_stubs_stop_the_guest_and_change_nothing() {
    let mut guest = Guest::new();
    const READ_WRITE: u64 = 0x04;
    const LIMIT: ExitReason = ExitReason::IllegalInstruction(0xFE);
    let scratch: Vec<u8> = (0..0x100).map(|index| index as u8 ^ 0xA5).collect();
    guest.memory.write_bytes(SCRATCH, &scratch).unwrap();
    guest.call("SetLastError", &[0x1234]);

    for (symbol, args) in [
        // The scratch page, read-write as it already is, with the old
        // protection going to the scratch page too.
        (
            "VirtualProtect",
            [SCRATCH, 0x100, READ_WRITE, SCRATCH + 0x80],
        ),
        ("VirtualProtect", [0, 0, 0, 0]),
        // A full MEMORY_BASIC_INFORMATION, 48 bytes, about a mapped address.
        ("VirtualQuery", [SCRATCH, SCRATCH + 0x40, 48, 0]),
        ("VirtualQuery", [0, 0, 0, 0]),
        (
            "__C_specific_handler",
            [SCRATCH, STACK_TOP, SCRATCH + 0x40, SCRATCH + 0x80],
        ),
        ("__C_specific_handler", [0, 0, 0, 0]),
    ] {
        let (cpu, exit) = guest.run(symbol, &args);
        assert_eq!(exit, LIMIT, "{symbol} {args:x?}");
        // Every general register but the stack pointer is as the caller left
        // it: no result in RAX, no argument register cleared.
        for (index, value) in cpu.gpr.iter().enumerate() {
            let argument = [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
                .iter()
                .position(|register| *register as usize == index);
            let wanted = match argument {
                Some(position) => args[position],
                None => 0x0101_0101_0101_0101 * (index as u64 + 1),
            };
            if index != Register::Rsp as usize {
                assert_eq!(*value, wanted, "{symbol} register {index}");
            }
        }
        assert_eq!(cpu.xmm[..4], guest.xmm, "{symbol}");
        let mut after = vec![0; scratch.len()];
        guest.memory.read_bytes(SCRATCH, &mut after).unwrap();
        assert_eq!(after, scratch, "{symbol} {args:x?}");
    }
    // The stopped calls recorded no error, and the guest still runs.
    assert_eq!(guest.last_error(), 0x1234);
    // The scratch page kept its protection: it is still writable.
    guest.memory.write_u8(SCRATCH, 1).unwrap();
}

/// The three stubs resolve by the module and symbol pairs a program built
/// with mingw-w64 14.0.0 (UCRT) imports them by, and say they are stubs.
#[test]
fn memory_and_exception_stubs_bind_and_are_declared_stubs() {
    let (registry, _) = standard(0);
    let named = |name: &str| ImportName::Name(name.to_owned());
    for (module, symbol, canonical) in [
        ("KERNEL32.dll", "VirtualProtect", "kernel32.dll"),
        ("KERNEL32.dll", "VirtualQuery", "kernel32.dll"),
        (
            "api-ms-win-core-memory-l1-1-0.dll",
            "VirtualProtect",
            "kernel32.dll",
        ),
        (
            "api-ms-win-core-memory-l1-1-0.dll",
            "VirtualQuery",
            "kernel32.dll",
        ),
        (
            "api-ms-win-crt-private-l1-1-0.dll",
            "__C_specific_handler",
            "ucrtbase.dll",
        ),
    ] {
        let service = registry
            .resolve(module, &named(symbol))
            .unwrap_or_else(|| panic!("{module}!{symbol}"));
        let target = registry.resolve(canonical, &named(symbol)).unwrap();
        assert!(std::ptr::eq(service, target), "{module}!{symbol}");
        assert_eq!(service.status, ServiceStatus::Stub, "{symbol}");
    }
    // The real services beside them are not stubs, and the API sets forward
    // only the symbols listed for them.
    let alloc = registry
        .resolve("kernel32.dll", &named("VirtualAlloc"))
        .unwrap();
    assert_eq!(alloc.status, ServiceStatus::Partial);
    for (module, symbol) in [
        ("api-ms-win-crt-private-l1-1-0.dll", "VirtualProtect"),
        ("api-ms-win-core-memory-l1-1-0.dll", "__C_specific_handler"),
        ("kernel32.dll", "__C_specific_handler"),
    ] {
        assert!(
            registry.resolve(module, &named(symbol)).is_none(),
            "{module}!{symbol}"
        );
    }
    let stubs: Vec<&str> = SERVICES
        .iter()
        .filter(|service| service.status == ServiceStatus::Stub)
        .map(|service| service.symbol)
        .collect();
    assert_eq!(
        stubs,
        ["VirtualProtect", "VirtualQuery", "__C_specific_handler"]
    );
}

#[test]
fn floating_arguments_and_results_use_xmm_registers() {
    let mut guest = Guest::new();
    let mut float = |symbol: &str, bits: u64| {
        // Only the low lane is the argument, and RCX must be ignored.
        guest.xmm[0] = 0xFFFF_FFFF_FFFF_FFFF << 64 | u128::from(bits);
        let (cpu, exit) = guest.run(symbol, &[0x4022_0000_0000_0000]);
        assert_eq!(exit, ExitReason::Halt);
        cpu.xmm[0]
    };
    assert_eq!(
        float("sqrt", 0x4030_0000_0000_0000) as u64,
        0x4010_0000_0000_0000
    ); // 16 -> 4
    assert_eq!(
        float("sqrt", 0x4000_0000_0000_0000) as u64,
        0x3FF6_A09E_667F_3BCD
    ); // sqrt 2
    assert_eq!(
        float("sqrt", 0x8000_0000_0000_0000) as u64,
        0x8000_0000_0000_0000
    ); // -0
    assert_eq!(
        float("sqrt", 0x7FF0_0000_0000_0000) as u64,
        0x7FF0_0000_0000_0000
    );
    let nan = float("sqrt", 0xBFF0_0000_0000_0000) as u64;
    assert!(f64::from_bits(nan).is_nan());
    assert_eq!(
        float("fabs", 0xC004_0000_0000_0000) as u64,
        0x4004_0000_0000_0000
    ); // -2.5
    assert_eq!(float("fabs", 0x8000_0000_0000_0000) as u64, 0);
    let garbage = 0xDEAD_BEEF_0000_0000;
    assert_eq!(float("sqrtf", garbage | 0x4110_0000) as u32, 0x4040_0000); // 9 -> 3
    assert_eq!(float("fabsf", garbage | 0xBF80_0000) as u32, 0x3F80_0000); // -1
}

#[test]
fn guest_call_through_dbt_preserves_win64_abi_state() {
    use Register::{R8, R9, R12, R13, R14, R15, Rax, Rbp, Rbx, Rcx, Rdi, Rdx, Rsi, Rsp};
    let observed = Arc::new(Mutex::new(None::<(u64, u64, [u64; 4])>));
    let seen = observed.clone();
    let mut registry = ImportRegistry::new();
    registry.register("t.dll", "F", move |_args, cpu, mem| {
        let rsp = cpu.get(Rsp);
        let shadow = [
            mem.read_u64(rsp + 8).unwrap(),
            mem.read_u64(rsp + 16).unwrap(),
            mem.read_u64(rsp + 24).unwrap(),
            mem.read_u64(rsp + 32).unwrap(),
        ];
        *seen.lock().unwrap() = Some((rsp, cpu.rip, shadow));
        cpu.set(Rcx, 0x11);
        cpu.set(Rdx, 0x22);
        cpu.set(R8, 0x33);
        cpu.set(R9, 0x44);
        for (reg, lane) in cpu.xmm[..6].iter_mut().enumerate() {
            *lane = u128::from(0x55_u64 + reg as u64);
        }
        Ok(0x99AA_BBCC_DDEE_F001)
    });
    let image = build(&[("t.dll", Symbol::Name("F"))]);
    let pe = ParsedPe::parse(&image.bytes).unwrap();
    let mut memory = pe.load_into_memory(&image.bytes).unwrap().memory;
    memory
        .map_region(STACK, 0x1000, MemoryPermissions::READ_WRITE)
        .unwrap();
    registry
        .bind_imports(&pe, &image.bytes, BASE, &mut memory)
        .unwrap();
    let mut engine = DbtEngine::new(256 * 1024).unwrap();
    engine.set_import_registry(registry);
    // Before CALL RSP is 16-byte aligned; CALL pushes return address so the
    // callee sees RSP % 16 == 8. Shadow space is [entry_rsp+8, entry_rsp+40).
    let initial_rsp = STACK_TOP;
    assert_eq!(initial_rsp % 16, 0);
    let entry_rsp_expected = initial_rsp - 8;
    for slot in 0..4u64 {
        memory
            .write_u64(entry_rsp_expected + 8 + slot * 8, 0x7000 + slot)
            .unwrap();
    }
    let mut cpu = CpuState::new(BASE + u64::from(CODE_RVA), initial_rsp);
    let nonvolatile_gpr = [Rbx, Rbp, Rdi, Rsi, R12, R13, R14, R15];
    for (index, register) in nonvolatile_gpr.into_iter().enumerate() {
        cpu.set(register, 0x6100 + index as u64);
    }
    for (offset, lane) in cpu.xmm[6..16].iter_mut().enumerate() {
        *lane = u128::from(0x6200_u64 + (offset + 6) as u64);
    }
    let result = engine.run(&mut cpu, &mut memory, 1000);
    assert_eq!(result.exit_reason, ExitReason::Halt);
    let (entry_rsp, _, shadow) = observed.lock().unwrap().expect("service ran");
    assert_eq!(entry_rsp, entry_rsp_expected);
    assert_eq!(entry_rsp % 16, 8);
    assert_eq!(shadow, [0x7000, 0x7001, 0x7002, 0x7003]);
    assert_eq!(cpu.get(Rax), 0x99AA_BBCC_DDEE_F001);
    assert_eq!(cpu.get(Rsp), initial_rsp);
    for (index, register) in nonvolatile_gpr.into_iter().enumerate() {
        assert_eq!(cpu.get(register), 0x6100 + index as u64, "{register:?}");
    }
    for (offset, lane) in cpu.xmm[6..16].iter().enumerate() {
        let index = offset + 6;
        assert_eq!(*lane, u128::from(0x6200_u64 + index as u64), "xmm{index}");
    }
}

#[test]
fn arguments_are_assigned_by_position_and_read_the_stack_after_the_fourth() {
    let mut memory = GuestMemory::new();
    memory
        .map_region(STACK, 0x1000, MemoryPermissions::READ_WRITE)
        .unwrap();
    // [RSP] return address, 32 bytes of shadow space, then arguments 4 and 5.
    memory
        .write_u64(STACK_TOP + 40, 0x1122_3344_5566_7788)
        .unwrap();
    memory
        .write_u64(STACK_TOP + 48, 0x4008_0000_0000_0000)
        .unwrap();
    let mut cpu = CpuState::new(0, STACK_TOP);
    for (index, register) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
        .into_iter()
        .enumerate()
    {
        cpu.set(register, 0xFFFF_FFFF_0000_0010 + index as u64);
        cpu.xmm[index] = (1 << 100) + 0xAAAA_AAAA_0000_0020 + index as u128;
    }
    let args = Win64Args::new(&cpu);
    let call = Call::new(&args, &cpu, &memory);
    assert_eq!(call.u32(0), Ok(0x10));
    assert_eq!(call.f64(1), Ok(Float64(0xAAAA_AAAA_0000_0021)));
    assert_eq!(call.f32(2), Ok(Float32(0x22)));
    assert_eq!(call.u64(3), Ok(0xFFFF_FFFF_0000_0013));
    assert_eq!(call.i32(4), Ok(0x5566_7788));
    assert_eq!(call.f64(5), Ok(Float64(0x4008_0000_0000_0000)));
    cpu.set(Register::Rsp, STACK + 0xFF0);
    let args = Win64Args::new(&cpu);
    let call = Call::new(&args, &cpu, &memory);
    assert!(matches!(call.u64(4), Err(ExitReason::MemoryFault(_))));
    cpu.set(Register::Rsp, u64::MAX - 8);
    let args = Win64Args::new(&cpu);
    assert!(Call::new(&args, &cpu, &memory).f64(4).is_err());
}

// --- registry ----------------------------------------------------------------

/// Bits above an `int` argument are not part of it.
const JUNK: u64 = 0xFFFF_FFFF_0000_0000;
/// The low 32 bits of an `int` result of -1.
const MINUS_ONE: u64 = 0xFFFF_FFFF;
const EINVAL: u32 = 22;

/// `_set_new_mode` (new.h) and `_configthreadlocale` (locale.h), from their
/// Microsoft Learn pages: each returns the setting before the call, and -1 for
/// an argument outside its documented set, with `errno` set to `EINVAL`.
#[test]
fn crt_startup_modes_return_the_previous_setting() {
    let mut guest = Guest::new();
    let result = |value: u64| value & 0xFFFF_FFFF;
    // malloc does not call the new handler by default, so the first mode is 0.
    assert_eq!(result(guest.call("_set_new_mode", &[1])), 0);
    assert_eq!(result(guest.call("_set_new_mode", &[1])), 1);
    assert_eq!(result(guest.call("_set_new_mode", &[JUNK])), 1);
    assert_eq!(result(guest.call("_set_new_mode", &[JUNK | 1])), 0);
    guest.set_errno(0);
    for invalid in [2, 3, MINUS_ONE, 0x8000_0000] {
        assert_eq!(result(guest.call("_set_new_mode", &[invalid])), MINUS_ONE);
        assert_eq!(guest.errno(), EINVAL, "{invalid:#x}");
        guest.set_errno(0);
    }
    // A refused mode changes nothing, and a valid call leaves errno alone.
    assert_eq!(result(guest.call("_set_new_mode", &[0])), 1);
    assert_eq!(guest.errno(), 0);

    // _ENABLE_PER_THREAD_LOCALE is 1, _DISABLE_PER_THREAD_LOCALE is 2, and 0
    // only asks. A program starts with the global locale.
    let locale =
        |guest: &mut Guest, request: u64| result(guest.call("_configthreadlocale", &[request]));
    assert_eq!(locale(&mut guest, 0), 2);
    assert_eq!(locale(&mut guest, 0), 2);
    assert_eq!(locale(&mut guest, 1), 2);
    assert_eq!(locale(&mut guest, 0), 1);
    assert_eq!(locale(&mut guest, JUNK | 2), 1);
    assert_eq!(locale(&mut guest, JUNK), 2);
    assert_eq!(locale(&mut guest, 1), 2);
    assert_eq!(guest.errno(), 0);
    for invalid in [3, 4, MINUS_ONE, 0x8000_0001] {
        assert_eq!(locale(&mut guest, invalid), MINUS_ONE, "{invalid:#x}");
        assert_eq!(guest.errno(), EINVAL, "{invalid:#x}");
        guest.set_errno(0);
    }
    // A refused request changes nothing, and a valid one leaves errno alone.
    assert_eq!(locale(&mut guest, 0), 1);
    assert_eq!(guest.errno(), 0);
    // The two settings are separate.
    assert_eq!(result(guest.call("_set_new_mode", &[0])), 0);
}

/// `_set_invalid_parameter_handler` (stdlib.h), `SetUnhandledExceptionFilter`
/// (errhandlingapi.h) and `signal` (signal.h), from their Microsoft Learn
/// pages: each returns the handler it replaces. The first two start with none,
/// which is a null pointer; a signal starts at `SIG_DFL`, which is zero.
/// `signal` returns `SIG_ERR`, which is -1 as a pointer, and sets `errno` to
/// `EINVAL` for a signal that is not one of the six its page lists.
#[test]
fn crt_startup_handlers_return_the_previous_handler() {
    let mut guest = Guest::new();
    const A: u64 = 0x1_4000_1230;
    const B: u64 = 0x7FFF_FFFF_FFFF_FFF0;
    for symbol in [
        "_set_invalid_parameter_handler",
        "SetUnhandledExceptionFilter",
    ] {
        assert_eq!(guest.call(symbol, &[A]), 0, "{symbol}");
        assert_eq!(guest.call(symbol, &[B]), A, "{symbol}");
        assert_eq!(guest.call(symbol, &[0]), B, "{symbol}");
        assert_eq!(guest.call(symbol, &[0]), 0, "{symbol}");
    }
    // One does not see the other's handler.
    assert_eq!(guest.call("_set_invalid_parameter_handler", &[A]), 0);
    assert_eq!(guest.call("SetUnhandledExceptionFilter", &[B]), 0);
    assert_eq!(guest.call("_set_invalid_parameter_handler", &[0]), A);
    assert_eq!(guest.call("SetUnhandledExceptionFilter", &[0]), B);

    // SIGINT 2, SIGILL 4, SIGFPE 8, SIGSEGV 11, SIGTERM 15, SIGABRT 22.
    const SIGNALS: [u64; 6] = [2, 4, 8, 11, 15, 22];
    guest.set_errno(0);
    for signal in SIGNALS {
        assert_eq!(guest.call("signal", &[signal, A + signal]), 0, "{signal}");
    }
    // Each signal has its own handler, and the number is an int.
    for signal in SIGNALS {
        assert_eq!(
            guest.call("signal", &[JUNK | signal, B]),
            A + signal,
            "{signal}"
        );
    }
    // SIG_DFL is 0 and SIG_IGN is 1; either is stored like a function.
    assert_eq!(guest.call("signal", &[2, 1]), B);
    assert_eq!(guest.call("signal", &[2, 0]), 1);
    assert_eq!(guest.call("signal", &[2, A]), 0);
    assert_eq!(guest.errno(), 0);
    for unlisted in [
        0,
        1,
        3,
        5,
        7,
        9,
        10,
        12,
        14,
        16,
        23,
        64,
        MINUS_ONE,
        0x8000_0002,
    ] {
        assert_eq!(
            guest.call("signal", &[unlisted, B]),
            u64::MAX,
            "{unlisted:#x}"
        );
        assert_eq!(guest.errno(), EINVAL, "{unlisted:#x}");
        guest.set_errno(0);
    }
    // A refused call changed no handler.
    assert_eq!(guest.call("signal", &[2, 0]), A);
    for signal in &SIGNALS[1..] {
        assert_eq!(guest.call("signal", &[*signal, 0]), B, "{signal}");
    }
}

/// `__p__fmode` and `__p__commode`, from their Microsoft Learn pages: each
/// returns a pointer to a global `int`, `_fmode` and `_commode`. The `_fmode`
/// page gives its default as `_O_TEXT`, which fcntl.h defines as 0x4000. No
/// page gives `_commode` an initial value; zero, "no commit", is Xenora's.
#[test]
fn crt_file_mode_variables_are_guest_memory() {
    let mut guest = Guest::new();
    let fmode = guest.call("__p__fmode", &[]);
    let commode = guest.call("__p__commode", &[]);
    assert_ne!(fmode, 0);
    assert_ne!(commode, 0);
    // Two ints that do not overlap.
    assert!(fmode.abs_diff(commode) >= 4);
    assert_eq!(guest.memory.read_u32(fmode).unwrap(), 0x4000);
    assert_eq!(guest.memory.read_u32(commode).unwrap(), 0);
    // The program assigns through the pointers: _O_BINARY is 0x8000 and
    // _IOCOMMIT is 0x4000.
    guest.memory.write_u32(fmode, 0x8000).unwrap();
    assert_eq!(guest.memory.read_u32(commode).unwrap(), 0);
    guest.memory.write_u32(commode, 0x4000).unwrap();
    // The variables stay where they are and keep what was stored.
    assert_eq!(guest.call("__p__fmode", &[]), fmode);
    assert_eq!(guest.call("__p__commode", &[]), commode);
    assert_eq!(guest.memory.read_u32(fmode).unwrap(), 0x8000);
    assert_eq!(guest.memory.read_u32(commode).unwrap(), 0x4000);

    // The order of the first calls does not matter.
    let mut other = Guest::new();
    let commode = other.call("__p__commode", &[]);
    assert_eq!(other.memory.read_u32(commode).unwrap(), 0);
    let fmode = other.call("__p__fmode", &[]);
    assert_eq!(other.memory.read_u32(fmode).unwrap(), 0x4000);
    assert!(fmode.abs_diff(commode) >= 4);
}

/// `_set_app_type` and `__setusermatherr` (corecrt_startup.h) return nothing.
/// What they were given is visible to the host only.
#[test]
fn crt_app_type_and_math_error_handler_are_recorded() {
    let mut guest = Guest::new();
    assert_eq!(guest.state.crt_app_type(), 0); // _crt_unknown_app
    assert_eq!(guest.state.crt_user_math_error(), 0);
    guest.call("_set_app_type", &[JUNK | 1]); // _crt_console_app
    assert_eq!(guest.state.crt_app_type(), 1);
    assert_eq!(guest.state.crt_user_math_error(), 0);
    guest.call("__setusermatherr", &[0x1_4000_2340]);
    assert_eq!(guest.state.crt_user_math_error(), 0x1_4000_2340);
    assert_eq!(guest.state.crt_app_type(), 1);
    guest.call("_set_app_type", &[2]); // _crt_gui_app
    guest.call("__setusermatherr", &[0]);
    assert_eq!(guest.state.crt_app_type(), 2);
    assert_eq!(guest.state.crt_user_math_error(), 0);
    // Neither touched errno or the last error.
    assert_eq!(guest.errno(), 0);
    assert_eq!(guest.last_error(), 0);
}

/// The module and symbol pairs a program built with mingw-w64 14.0.0 (UCRT)
/// imports these services by, read from such a program's import table.
#[test]
fn crt_startup_services_resolve_as_mingw_w64_imports_them() {
    let (registry, _) = standard(0);
    for (module, symbol, canonical) in [
        (
            "api-ms-win-crt-runtime-l1-1-0.dll",
            "_set_app_type",
            "ucrtbase.dll",
        ),
        (
            "api-ms-win-crt-heap-l1-1-0.dll",
            "_set_new_mode",
            "ucrtbase.dll",
        ),
        (
            "api-ms-win-crt-locale-l1-1-0.dll",
            "_configthreadlocale",
            "ucrtbase.dll",
        ),
        (
            "api-ms-win-crt-math-l1-1-0.dll",
            "__setusermatherr",
            "ucrtbase.dll",
        ),
        (
            "api-ms-win-crt-runtime-l1-1-0.dll",
            "_set_invalid_parameter_handler",
            "ucrtbase.dll",
        ),
        (
            "api-ms-win-crt-stdio-l1-1-0.dll",
            "__p__fmode",
            "ucrtbase.dll",
        ),
        (
            "api-ms-win-crt-stdio-l1-1-0.dll",
            "__p__commode",
            "ucrtbase.dll",
        ),
        (
            "api-ms-win-crt-runtime-l1-1-0.dll",
            "signal",
            "ucrtbase.dll",
        ),
        (
            "KERNEL32.dll",
            "SetUnhandledExceptionFilter",
            "kernel32.dll",
        ),
        (
            "api-ms-win-crt-private-l1-1-0.dll",
            "memcpy",
            "ucrtbase.dll",
        ),
    ] {
        let service = registry
            .resolve(module, &named(symbol))
            .unwrap_or_else(|| panic!("{module}!{symbol}"));
        let target = registry.resolve(canonical, &named(symbol)).unwrap();
        assert!(std::ptr::eq(service, target), "{module}!{symbol}");
        assert_eq!(service.status, ServiceStatus::Partial, "{symbol}");
    }
    // An API set forwards only the symbols listed for it.
    for (module, symbol) in [
        ("api-ms-win-crt-heap-l1-1-0.dll", "_set_app_type"),
        ("api-ms-win-crt-runtime-l1-1-0.dll", "_set_new_mode"),
        ("api-ms-win-crt-stdio-l1-1-0.dll", "signal"),
        ("api-ms-win-crt-private-l1-1-0.dll", "_set_app_type"),
    ] {
        assert!(
            registry.resolve(module, &named(symbol)).is_none(),
            "{module}!{symbol}"
        );
    }
}

fn named(name: &str) -> ImportName {
    ImportName::Name(name.to_owned())
}

#[test]
fn generated_metadata_matches_the_live_registry() {
    let (registry, _) = standard(0);
    for service in &SERVICES {
        let descriptor = registry
            .resolve(service.library, &named(service.symbol))
            .unwrap();
        assert_eq!(descriptor.status, service.status, "{}", service.symbol);
        assert!(!service.tests.is_empty());
        for alias in service.aliases {
            let upper = alias.to_ascii_uppercase();
            let aliased = registry.resolve(&upper, &named(service.symbol)).unwrap();
            assert!(
                std::ptr::eq(aliased, descriptor),
                "{alias}!{}",
                service.symbol
            );
        }
        // Symbol names are exact, and unlisted API sets do not resolve.
        assert!(
            registry
                .resolve(
                    service.library,
                    &named(&service.symbol.to_ascii_uppercase())
                )
                .is_none()
                || service.symbol == service.symbol.to_ascii_uppercase()
        );
        assert!(
            registry
                .resolve("api-ms-win-core-file-l1-1-0.dll", &named(service.symbol))
                .is_none()
        );
    }
}

#[test]
fn conflicting_or_invalid_registrations_are_refused() {
    let handler = || Box::new(|_: &Win64Args, _: &mut CpuState, _: &mut GuestMemory| Ok(0));
    let mut registry = ImportRegistry::new();
    let mut add = |module: &str, name: ImportName| {
        registry.try_register(module, name, ServiceStatus::Stub, handler())
    };
    assert_eq!(add("a.dll", named("F")), Ok(()));
    assert_eq!(
        add("a.dll", named("f")),
        Ok(()),
        "symbols are case-sensitive"
    );
    assert_eq!(add("b.dll", named("F")), Ok(()));
    assert!(matches!(
        add("A.DLL", named("F")),
        Err(RegistryError::Duplicate { .. })
    ));
    assert_eq!(add("a.dll", ImportName::Ordinal(7)), Ok(()));
    assert!(matches!(
        add("a.dll", ImportName::Ordinal(7)),
        Err(RegistryError::Duplicate { .. })
    ));
    assert_eq!(
        add("a.dll", named("Ordinal#7")),
        Ok(()),
        "a name is never an ordinal"
    );
    for (module, symbol) in [("", "F"), ("a.dll", ""), ("a b.dll", "F"), ("a.dll", "F\n")] {
        assert!(matches!(
            add(module, named(symbol)),
            Err(RegistryError::InvalidName { .. })
        ));
    }
    assert_eq!(registry.try_alias("set.dll", "a.dll", "F"), Ok(()));
    assert_eq!(
        registry.try_alias("set.dll", "a.dll", "F"),
        Ok(()),
        "repeating is harmless"
    );
    assert!(matches!(
        registry.try_alias("set.dll", "b.dll", "F"),
        Err(RegistryError::Duplicate { .. })
    ));
    assert!(matches!(
        registry.try_alias("b.dll", "a.dll", "F"),
        Err(RegistryError::Duplicate { .. })
    ));
    assert!(matches!(
        registry.try_alias("x.dll", "a.dll", "G"),
        Err(RegistryError::UnknownTarget { .. })
    ));
}

#[test]
#[should_panic(expected = "kernel32.dll!CloseHandle is already registered")]
fn a_host_module_cannot_replace_a_standard_service() {
    let (mut registry, _) = standard(0);
    registry.register("KERNEL32.dll", "CloseHandle", |_, _, _| Ok(1));
}

fn bind(registry: &mut ImportRegistry, image: &Image) -> (Result<usize, ImportError>, GuestMemory) {
    let pe = ParsedPe::parse(&image.bytes).unwrap();
    let mut memory = pe.load_into_memory(&image.bytes).unwrap().memory;
    let result = registry.bind_imports(&pe, &image.bytes, BASE, &mut memory);
    (result, memory)
}

#[test]
fn an_unresolved_import_leaves_every_slot_unbound() {
    let image = build(&[
        ("kernel32.dll", Symbol::Name("GetLastError")),
        ("kernel32.dll", Symbol::Name("NoSuchFunction")),
    ]);
    let (mut registry, _) = standard(0);
    let (result, memory) = bind(&mut registry, &image);
    let expected = ImportError::UnresolvedImport {
        module: "kernel32.dll".into(),
        symbol: "NoSuchFunction".into(),
    };
    assert_eq!(result, Err(expected));
    for slot in &image.slots {
        let file = IDATA_RAW + (slot - IDATA_RVA) as usize;
        let original = u64::from_le_bytes(image.bytes[file..file + 8].try_into().unwrap());
        assert_eq!(memory.read_u64(BASE + u64::from(*slot)).unwrap(), original);
    }
}

#[test]
fn aliases_share_a_trampoline_and_ordinals_bind_only_ordinals() {
    let (mut registry, _) = standard(0);
    registry
        .try_register(
            "o.dll",
            ImportName::Ordinal(7),
            ServiceStatus::Partial,
            Box::new(|_, _, _| Ok(77)),
        )
        .unwrap();
    let image = build(&[
        ("KERNEL32.dll", Symbol::Name("GetLastError")),
        (
            "API-MS-Win-Core-ErrorHandling-L1-1-0.dll",
            Symbol::Name("GetLastError"),
        ),
        ("O.DLL", Symbol::Ordinal(7)),
    ]);
    let (result, memory) = bind(&mut registry, &image);
    assert_eq!(result, Ok(3));
    let slot = |index: usize| {
        memory
            .read_u64(BASE + u64::from(image.slots[index]))
            .unwrap()
    };
    assert_eq!(slot(0), slot(1));
    assert_ne!(slot(0), slot(2));
    for symbol in [Symbol::Name("Ordinal#7"), Symbol::Ordinal(8)] {
        let (result, _) = bind(&mut registry, &build(&[("o.dll", symbol)]));
        assert!(
            matches!(result, Err(ImportError::UnresolvedImport { .. })),
            "{result:?}"
        );
    }
}

/// A registry with one service bound, and its trampoline address.
fn bound_service<F>(handler: F) -> (ImportRegistry, GuestMemory, u64)
where
    F: Fn(&Win64Args, &mut CpuState, &mut GuestMemory) -> Result<u64, ExitReason>
        + Send
        + Sync
        + 'static,
{
    let mut registry = ImportRegistry::new();
    registry.register("t.dll", "F", handler);
    let image = build(&[("t.dll", Symbol::Name("F"))]);
    let (result, mut memory) = bind(&mut registry, &image);
    assert_eq!(result, Ok(1));
    memory
        .map_region(STACK, 0x1000, MemoryPermissions::READ_WRITE)
        .unwrap();
    memory.write_u64(STACK_TOP, 0x1234).unwrap();
    let trampoline = memory.read_u64(BASE + u64::from(image.slots[0])).unwrap();
    (registry, memory, trampoline)
}

#[test]
fn a_service_that_changes_a_nonvolatile_register_stops_the_guest() {
    use Register::{R8, R9, R10, R11, R12, R13, R14, R15, Rbp, Rbx, Rcx, Rdi, Rdx, Rsi, Rsp};
    let dispatch = |change: fn(&mut CpuState, usize), which: usize| {
        let (registry, mut memory, trampoline) = bound_service(move |_, cpu, _| {
            change(cpu, which);
            Ok(5)
        });
        let mut cpu = CpuState::new(trampoline, STACK_TOP);
        let result = registry.dispatch(trampoline, &mut cpu, &mut memory);
        (result, cpu)
    };
    for register in [Rbx, Rbp, Rdi, Rsi, Rsp, R12, R13, R14, R15] {
        let (result, _) = dispatch(|cpu, index| cpu.gpr[index] ^= 8, register as usize);
        assert_eq!(result, Err(ExitReason::ServiceAbiViolation), "{register:?}");
    }
    for register in [Rcx, Rdx, R8, R9, R10, R11] {
        let (result, cpu) = dispatch(|cpu, index| cpu.gpr[index] ^= 8, register as usize);
        assert_eq!(result, Ok(()), "{register:?}");
        assert_eq!(
            (cpu.rip, cpu.get(Rsp), cpu.get(Register::Rax)),
            (0x1234, STACK_TOP + 8, 5)
        );
    }
    for index in 0..16 {
        let (result, _) = dispatch(|cpu, index| cpu.xmm[index] ^= 1 << 127, index);
        let expected = if index < 6 {
            Ok(())
        } else {
            Err(ExitReason::ServiceAbiViolation)
        };
        assert_eq!(result, expected, "xmm{index}");
    }
}

#[test]
fn service_calls_preserve_win64_abi_state() {
    // Microsoft x64 calling convention: at entry RSP % 16 == 8 (return address
    // pushed by CALL). Callee preserves RBX,RBP,RDI,RSI,R12-R15 and XMM6-XMM15.
    // Caller owns 32-byte shadow/home space at [RSP+8, RSP+40). RAX carries the
    // return value; RCX,RDX,R8,R9,R10,R11,XMM0-XMM5 are volatile.
    use Register::{R8, R9, R10, R11, R12, R13, R14, R15, Rax, Rbp, Rbx, Rcx, Rdi, Rdx, Rsi, Rsp};
    let observed = Arc::new(Mutex::new(None::<(u64, u64, [u64; 4], u64)>));
    let seen = observed.clone();
    let (registry, mut memory, trampoline) = bound_service(move |args, cpu, mem| {
        let rsp = cpu.get(Rsp);
        let shadow = [
            mem.read_u64(rsp + 8).unwrap(),
            mem.read_u64(rsp + 16).unwrap(),
            mem.read_u64(rsp + 24).unwrap(),
            mem.read_u64(rsp + 32).unwrap(),
        ];
        *seen.lock().unwrap() = Some((rsp, cpu.rip, shadow, args.rcx));
        // Clobber volatile registers; dispatcher must still return correctly and
        // nonvolatile state must be preserved after return.
        cpu.set(Rcx, 0xA1);
        cpu.set(Rdx, 0xA2);
        cpu.set(R8, 0xA3);
        cpu.set(R9, 0xA4);
        cpu.set(R10, 0xA5);
        cpu.set(R11, 0xA6);
        for (reg, lane) in cpu.xmm[..6].iter_mut().enumerate() {
            *lane = u128::from(0xB0_u64 + reg as u64);
        }
        Ok(0x1122_3344_5566_7788)
    });
    // Win64: before CALL RSP is 16-byte aligned; CALL pushes return address,
    // so callee entry sees RSP % 16 == 8.
    let call_rsp = STACK_TOP - 8;
    assert_eq!(call_rsp % 16, 8);
    memory.write_u64(call_rsp, 0x1234).unwrap();
    for slot in 0..4u64 {
        memory
            .write_u64(call_rsp + 8 + slot * 8, 0xC000 + slot)
            .unwrap();
    }
    let mut cpu = CpuState::new(trampoline, call_rsp);
    let nonvolatile_gpr = [Rbx, Rbp, Rdi, Rsi, R12, R13, R14, R15];
    for (index, register) in nonvolatile_gpr.into_iter().enumerate() {
        cpu.set(register, 0x5AA5_0000_0001_0000 + index as u64);
    }
    for (offset, lane) in cpu.xmm[6..16].iter_mut().enumerate() {
        *lane = u128::from(0xD000_0000_0000_0000_u64 + (offset + 6) as u64);
    }
    cpu.set(Rcx, 0x1357_9BDF_2468_ACE0);
    let result = registry.dispatch(trampoline, &mut cpu, &mut memory);
    assert_eq!(result, Ok(()));
    let (entry_rsp, entry_rip, shadow, entry_rcx) = observed.lock().unwrap().expect("service ran");
    assert_eq!(entry_rip, trampoline);
    assert_eq!(
        entry_rsp, call_rsp,
        "service sees caller RSP at return address"
    );
    assert_eq!(entry_rsp % 16, 8, "Win64 entry RSP must be 8 mod 16");
    assert_eq!(entry_rcx, 0x1357_9BDF_2468_ACE0);
    // Shadow/home space is caller-owned and must be addressable by the service.
    assert_eq!(shadow, [0xC000, 0xC001, 0xC002, 0xC003]);
    for slot in 0..4u64 {
        assert_eq!(
            memory.read_u64(call_rsp + 8 + slot * 8).unwrap(),
            0xC000 + slot,
            "service call must not mutate caller shadow space"
        );
    }
    // Post-return ABI state.
    assert_eq!(cpu.get(Rax), 0x1122_3344_5566_7788);
    assert_eq!(cpu.get(Rsp), call_rsp + 8);
    assert_eq!(cpu.rip, 0x1234);
    for (index, register) in nonvolatile_gpr.into_iter().enumerate() {
        assert_eq!(
            cpu.get(register),
            0x5AA5_0000_0001_0000 + index as u64,
            "nonvolatile {register:?} must be preserved"
        );
    }
    for (offset, lane) in cpu.xmm[6..16].iter().enumerate() {
        let index = offset + 6;
        assert_eq!(
            *lane,
            u128::from(0xD000_0000_0000_0000_u64 + index as u64),
            "nonvolatile xmm{index} must be preserved"
        );
    }
}

#[test]
fn a_call_that_cannot_return_runs_no_service() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let (registry, mut memory, trampoline) = bound_service(move |_, _, _| {
        counter.fetch_add(1, Ordering::SeqCst);
        Ok(0)
    });
    for rsp in [0x9000_0000, STACK + 0xFFC, u64::MAX - 7] {
        let mut cpu = CpuState::new(trampoline, rsp);
        let before = cpu.clone();
        assert!(
            registry
                .dispatch(trampoline, &mut cpu, &mut memory)
                .is_err()
        );
        assert_eq!(cpu, before);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let mut cpu = CpuState::new(trampoline, STACK_TOP);
    assert_eq!(registry.dispatch(trampoline, &mut cpu, &mut memory), Ok(()));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

// --- import table --------------------------------------------------------------

fn inspect(image: &Image) -> ProbeReport {
    probe_pe_bytes(&image.bytes, &ProbeConfig::default()).unwrap()
}

/// File offset of an RVA in the import section.
fn file(rva: u32) -> usize {
    IDATA_RAW + (rva - IDATA_RVA) as usize
}

fn u32_at(image: &Image, at: usize) -> u32 {
    u32::from_le_bytes(image.bytes[at..at + 4].try_into().unwrap())
}

#[test]
fn malformed_import_tables_are_reported_and_never_bound() {
    let base = || {
        build(&[
            ("kernel32.dll", Symbol::Name("GetLastError")),
            ("kernel32.dll", Symbol::Name("CloseHandle")),
        ])
    };
    let end = IDATA_RVA + IDATA_LEN as u32;
    let second = IDATA_RAW + 20;
    type Damage = fn(&mut Image, usize, u32);
    type Expected = fn(&ImportTableError) -> bool;
    // (damage to the second descriptor, expected error)
    let cases: &[(&str, Damage, Expected)] = &[
        (
            "name runs off the section (issue #270)",
            |image, second, end| {
                let lookup = u32_at(image, second);
                image.bytes[file(lookup)..file(lookup) + 8]
                    .copy_from_slice(&u64::from(end - 6).to_le_bytes());
                let at = file(end - 4);
                image.bytes[at..at + 4].copy_from_slice(b"Name");
            },
            |error| matches!(error, ImportTableError::MalformedName { .. }),
        ),
        (
            "name pointer at the last byte",
            |image, second, end| {
                let lookup = u32_at(image, second);
                image.bytes[file(lookup)..file(lookup) + 8]
                    .copy_from_slice(&u64::from(end - 1).to_le_bytes());
            },
            |error| matches!(error, ImportTableError::MalformedName { .. }),
        ),
        (
            "empty name",
            |image, second, _| {
                let lookup = u32_at(image, second);
                let hint = u32_at(image, file(lookup));
                image.bytes[file(hint) + 2] = 0;
            },
            |error| matches!(error, ImportTableError::MalformedName { .. }),
        ),
        (
            "non-ASCII name",
            |image, second, _| {
                let lookup = u32_at(image, second);
                let hint = u32_at(image, file(lookup));
                image.bytes[file(hint) + 4] = 0xC3;
            },
            |error| matches!(error, ImportTableError::MalformedName { .. }),
        ),
        (
            "module name outside every section",
            |image, second, _| {
                image.bytes[second + 12..second + 16].copy_from_slice(&0x7000_0000u32.to_le_bytes())
            },
            |error| matches!(error, ImportTableError::MalformedName { .. }),
        ),
        (
            "lookup table outside every section",
            |image, second, _| {
                image.bytes[second..second + 4].copy_from_slice(&u32::MAX.to_le_bytes())
            },
            |error| matches!(error, ImportTableError::OutOfBounds { .. }),
        ),
        (
            "reserved bits in a name entry",
            |image, second, _| {
                let lookup = u32_at(image, second);
                image.bytes[file(lookup) + 5] = 1;
            },
            |error| matches!(error, ImportTableError::ReservedBits { .. }),
        ),
        (
            "bit 31 in a name entry",
            |image, second, _| {
                let lookup = u32_at(image, second);
                image.bytes[file(lookup) + 3] |= 0x80;
            },
            |error| matches!(error, ImportTableError::ReservedBits { .. }),
        ),
        (
            "name continues in bytes after the section",
            |image, second, end| {
                let lookup = u32_at(image, second);
                image.bytes[file(lookup)..file(lookup) + 8]
                    .copy_from_slice(&u64::from(end - 6).to_le_bytes());
                let at = file(end - 4);
                image.bytes[at..at + 4].copy_from_slice(b"Name");
                image.bytes.extend_from_slice(b"Tail\0");
            },
            |error| matches!(error, ImportTableError::MalformedName { .. }),
        ),
        (
            "reserved bits in an ordinal entry",
            |image, second, _| {
                let lookup = u32_at(image, second);
                image.bytes[file(lookup)..file(lookup) + 8]
                    .copy_from_slice(&(1u64 << 63 | 0x1_0000).to_le_bytes());
            },
            |error| matches!(error, ImportTableError::ReservedBits { .. }),
        ),
    ];
    for (what, damage, expected) in cases {
        let mut image = base();
        damage(&mut image, second, end);
        let (mut registry, _) = standard(0);
        let (result, _) = bind(&mut registry, &image);
        match &result {
            Err(ImportError::Table(error)) => assert!(expected(error), "{what}: {error:?}"),
            other => panic!("{what}: {other:?}"),
        }
        let report = inspect(&image);
        assert_eq!(
            report.import_error.map(ImportError::Table),
            result.err(),
            "{what}"
        );
        assert_eq!(
            report.imports.len(),
            1,
            "{what}: the first import is still listed"
        );
        assert_eq!(report.imports[0].symbol, "GetLastError");
    }
}

#[test]
fn endless_tables_are_bounded() {
    // A descriptor at 0, a module name at 0x40, a hint and name at 0x60 and a
    // lookup entry at 0x80. The mapping repeats one structure without end.
    let mut bytes = vec![0u8; 0x88];
    bytes[0..4].copy_from_slice(&0x80u32.to_le_bytes());
    bytes[12..16].copy_from_slice(&0x40u32.to_le_bytes());
    bytes[16..20].copy_from_slice(&0x80u32.to_le_bytes());
    bytes[0x40..0x46].copy_from_slice(b"m.dll\0");
    bytes[0x62..0x64].copy_from_slice(b"F\0");
    bytes[0x80] = 0x60;

    // Lookup entries repeat from 0x80 on.
    let (entries, error) = parse_imports(&bytes, 0, |rva| {
        let rva = rva as usize;
        Some(if rva < 0x80 { rva } else { 0x80 + rva % 8 })
    });
    assert_eq!(error, Some(ImportTableError::TooManyImports));
    assert_eq!(entries.len(), MAX_IMPORTS);

    // Descriptors repeat from 0x1000 on; each has one symbol.
    bytes.extend([0; 8]);
    let (entries, error) = parse_imports(&bytes, 0x1000, |rva| {
        let rva = rva as usize;
        Some(if rva < 0x1000 {
            rva
        } else {
            (rva - 0x1000) % 20
        })
    });
    assert_eq!(error, Some(ImportTableError::TooManyDescriptors));
    assert_eq!(entries.len(), 1024);
}

#[test]
fn inspection_and_binding_agree_on_damaged_tables() {
    let base = build(&[
        ("kernel32.dll", Symbol::Name("GetLastError")),
        (
            "api-ms-win-core-handle-l1-1-0.dll",
            Symbol::Name("CloseHandle"),
        ),
        ("ucrtbase.dll", Symbol::Name("sqrt")),
        ("o.dll", Symbol::Ordinal(3)),
    ]);
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state.wrapping_mul(0x9E37_79B9_7F4A_7C15)
    };
    let (mut bound, mut malformed, mut unresolved) = (0, 0, 0);
    for _ in 0..6000 {
        let mut image = Image {
            bytes: base.bytes.clone(),
            slots: base.slots.clone(),
        };
        for _ in 0..1 + next() % 3 {
            let at = IDATA_RAW + (next() % 0x220) as usize;
            image.bytes[at] = match next() % 4 {
                0 => 0,
                1 => 0xFF,
                _ => next() as u8,
            };
        }
        let (mut registry, _) = standard(0);
        registry
            .try_register(
                "o.dll",
                ImportName::Ordinal(3),
                ServiceStatus::Stub,
                Box::new(|_, _, _| Ok(0)),
            )
            .unwrap();
        let pe = ParsedPe::parse(&image.bytes).unwrap();
        let mut memory = pe.load_into_memory(&image.bytes).unwrap().memory;
        let result = registry.bind_imports(&pe, &image.bytes, BASE, &mut memory);
        let report = inspect(&image);
        match result {
            Ok(count) => {
                bound += 1;
                assert_eq!(report.import_error, None);
                assert_eq!(report.imports.len(), count);
            }
            Err(ImportError::Table(error)) => {
                malformed += 1;
                assert_eq!(report.import_error, Some(error));
            }
            Err(ImportError::UnresolvedImport { module, symbol }) => {
                unresolved += 1;
                assert_eq!(report.import_error, None);
                assert!(
                    report
                        .imports
                        .iter()
                        .any(|import| import.dll == module && import.symbol == symbol)
                );
            }
            Err(ImportError::Memory(_)) => {}
        }
    }
    assert!(
        bound > 100 && malformed > 100 && unresolved > 100,
        "{bound} {malformed} {unresolved}"
    );
}

#[test]
fn inspection_reports_exactly_what_binding_resolves() {
    let image = build(&[
        ("KERNEL32.dll", Symbol::Name("GetLastError")),
        ("kernel32.dll", Symbol::Name("getlasterror")),
        (
            "api-ms-win-core-errorhandling-l1-1-0.dll",
            Symbol::Name("SetLastError"),
        ),
        (
            "api-ms-win-core-errorhandling-l1-1-3.dll",
            Symbol::Name("SetLastError"),
        ),
        ("kernel32.dll", Symbol::Ordinal(1)),
    ]);
    let report = inspect(&image);
    let supported: Vec<bool> = report
        .imports
        .iter()
        .map(|import| import.is_supported)
        .collect();
    assert_eq!(supported, [true, false, true, false, false]);
    assert_eq!(
        report.imports[2].canonical_module.as_deref(),
        Some("kernel32.dll")
    );
    assert_eq!(report.imports[4].symbol, "Ordinal#1");
}
