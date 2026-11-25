// SPDX-License-Identifier: GPL-3.0-or-later

//! Process exit (issue #329): `ExitProcess`, a returning entry point, and the
//! rule that an exited guest runs nothing further.
//!
//! Expected behaviour is from the public Win32 documentation for
//! `ExitProcess` (processthreadsapi.h): it does not return, the status is a
//! `UINT`, and the process's handles are closed. The guest code is original.

mod support;

use std::sync::Arc;
use support::*;
use xenora_loader::cpu::{CpuState, ExitReason};
use xenora_loader::import_table::ImportName;
use xenora_loader::imports::{ENTRY_RETURN, ImportRegistry, RETURN_SENTINEL_PAGE, Win64Args};
use xenora_loader::memory::{GuestMemory, MemoryError, MemoryPermissions};
use xenora_loader::services::{MockClock, ServicesState, register_standard_services};
use xenora_loader::startup::{StartupLayout, run_synthetic_startup};

const KERNEL32: &str = "kernel32.dll";
const THREADS: &str = "api-ms-win-core-processthreads-l1-1-0.dll";
const UCRT: &str = "ucrtbase.dll";

const IMPORTS: &[(&str, &str)] = &[
    (KERNEL32, "ExitProcess"),
    (KERNEL32, "SetLastError"),
    (KERNEL32, "CreateFileW"),
    (KERNEL32, "CreateEventW"),
    (UCRT, "qsort"),
    ("ws2_32.dll", "WSAStartup"),
    ("ws2_32.dll", "socket"),
    ("xgameruntime.dll", "XTaskQueueCreate"),
];
const ALIAS: &[(&str, &str)] = &[(THREADS, "ExitProcess")];

/// Guest address of the stub `call [slot]; hlt` for import `index`.
fn stub(index: usize) -> u64 {
    BASE + u64::from(CODE_RVA) + 16 * index as u64
}

#[test]
fn exit_process_stops_the_guest_with_its_whole_status() {
    // The status is a 32-bit UINT: values above 255 survive, and the upper
    // half of RCX is not part of it.
    for (argument, status) in [
        (0, 0),
        (1, 1),
        (150, 150),
        (0x1234_5678, 0x1234_5678),
        (0xFFFF_FFFF, 0xFFFF_FFFF),
        (0xAAAA_BBBB_0000_002A, 0x2A),
    ] {
        for imports in [IMPORTS, ALIAS] {
            let mut guest = Guest::new(imports, &[]);
            assert_eq!(guest.state.exit_status(), None);
            // The stub is `call [ExitProcess]; hlt`. A service that returned
            // would reach the HLT and report Halt.
            let (_, exit) = guest.run("ExitProcess", &[argument]);
            assert_eq!(exit, ExitReason::ProcessExit(status), "{argument:#x}");
            assert_eq!(guest.state.exit_status(), Some(status), "{argument:#x}");
        }
    }
}

#[test]
fn an_exited_guest_runs_nothing_further() {
    let mut guest = Guest::new(IMPORTS, &[]);
    assert_eq!(guest.run("ExitProcess", &[3]).1, ExitReason::ProcessExit(3));
    // Neither another service nor a second exit takes effect: the engine
    // reports the first exit again without executing anything.
    for (symbol, argument) in [("SetLastError", 5), ("ExitProcess", 9)] {
        let (_, exit) = guest.run(symbol, &[argument]);
        assert_eq!(exit, ExitReason::ProcessExit(3), "{symbol}");
    }
    assert_eq!(guest.state.last_error(THREAD_A), 0);
    assert_eq!(guest.state.exit_status(), Some(3));
}

/// Under the guest thread scheduler the engine runs in slices. A process exit
/// from the scheduled main thread stops the schedule with that exit as its
/// reason, and sets the engine's exit latch through the sliced entry point, so
/// nothing runs afterwards. The scheduler has no outcome of its own for a
/// process exit and reports it as the reason of a `Fault`.
#[test]
fn exit_process_under_the_scheduler_ends_the_run() {
    use xenora_loader::scheduler::Outcome;
    for quantum in [1, 64, 1_000_000] {
        let mut guest = Guest::new(IMPORTS, &[]);
        assert_ne!(guest.call("CreateEventW", &[0, 1, 0, 0]), 0);
        let report = guest.scheduled("ExitProcess", &[0xAAAA_BBBB_0000_0015], quantum, 10_000);
        assert!(
            matches!(
                report.outcome,
                Outcome::Fault {
                    reason: ExitReason::ProcessExit(0x15),
                    ..
                }
            ),
            "quantum {quantum}: {:?}",
            report.outcome
        );
        assert_eq!(guest.state.exit_status(), Some(0x15));
        assert_eq!(guest.state.open_handle_count(), 0);
        // The engine is latched: an unsliced run executes nothing either.
        let (_, exit) = guest.run("SetLastError", &[5]);
        assert_eq!(exit, ExitReason::ProcessExit(0x15), "quantum {quantum}");
        assert_eq!(guest.state.last_error(THREAD_A), 0);
    }
}

#[test]
fn the_first_exit_decides_the_status() {
    // Two calls of the service itself, with no engine in between to stop the
    // second one.
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let mut registry = ImportRegistry::new();
    register_standard_services(&mut registry, Arc::clone(&state));
    let exit = &registry
        .resolve(KERNEL32, &ImportName::Name("ExitProcess".into()))
        .expect("ExitProcess is registered")
        .handler;
    let mut cpu = CpuState::new(0, 0);
    let mut memory = GuestMemory::new();
    for status in [41, 42] {
        let mut args = Win64Args::new(&cpu);
        args.rcx = status;
        assert_eq!(
            exit(&args, &mut cpu, &mut memory),
            Err(ExitReason::ProcessExit(41))
        );
    }
    assert_eq!(state.exit_status(), Some(41));
}

#[test]
fn exit_from_inside_a_service_callback_ends_the_process() {
    // A comparator for qsort that exits instead of comparing:
    //   sub rsp, 0x28
    //   mov ecx, 9
    //   call <ExitProcess stub>
    //   mov byte ptr [SCRATCH], 1      ; must never run
    //   add rsp, 0x28
    //   ret
    let call_end = routine(0) + 14;
    let displacement = (stub(0).wrapping_sub(call_end) as u32).to_le_bytes();
    let mut comparator = vec![0x48, 0x83, 0xEC, 0x28, 0xB9, 9, 0, 0, 0, 0xE8];
    comparator.extend(displacement);
    comparator.extend([0xC6, 0x04, 0x25]);
    comparator.extend((SCRATCH as u32).to_le_bytes());
    comparator.extend([0x01, 0x48, 0x83, 0xC4, 0x28, 0xC3]);

    let mut guest = Guest::new(IMPORTS, &[comparator]);
    let items = SCRATCH + 0x100;
    guest.memory.write_u64(items, 2).unwrap();
    guest.memory.write_u64(items + 8, 1).unwrap();
    let (_, exit) = guest.run("qsort", &[items, 2, 8, routine(0)]);
    assert_eq!(exit, ExitReason::ProcessExit(9));
    assert_eq!(guest.memory.read_u8(SCRATCH).unwrap(), 0);
    // The sort never resumed, so the items are as the guest left them.
    assert_eq!((guest.u64_at(items), guest.u64_at(items + 8)), (2, 1));
    assert_eq!(guest.state.exit_status(), Some(9));
}

#[test]
fn exit_closes_every_handle_and_keeps_the_files() {
    let mut guest = Guest::new(IMPORTS, &[]);
    let name: Vec<u8> = "C:\\exit.bin\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    guest.memory.write_bytes(SCRATCH, &name).unwrap();
    // CreateFileW(name, GENERIC_READ | GENERIC_WRITE, no sharing, null,
    // CREATE_ALWAYS, 0, null) and an unnamed event.
    let file = guest.call("CreateFileW", &[SCRATCH, 0xC000_0000, 0, 0, 2, 0, 0]);
    assert_ne!(file, u64::MAX);
    assert_ne!(guest.call("CreateEventW", &[0, 1, 0, 0]), 0);
    assert_eq!(guest.state.open_handle_count(), 2);

    // Sockets and task queues live in tables of their own. WSAStartup(2.2),
    // two UDP sockets, and XTaskQueueCreate(manual, manual, &handle).
    assert_eq!(guest.call("WSAStartup", &[0x0202, SCRATCH + 0x200]), 0);
    let first = guest.call("socket", &[2, 2, 17]);
    let second = guest.call("socket", &[2, 2, 17]);
    assert!(first != u64::MAX && second != u64::MAX && first != second);
    assert_eq!(guest.state.open_handle_count(), 4);
    assert_eq!(guest.call("XTaskQueueCreate", &[0, 0, SCRATCH + 0x400]), 0);
    assert_ne!(guest.u64_at(SCRATCH + 0x400), 0);
    assert_eq!(guest.state.open_handle_count(), 5);

    assert_eq!(guest.run("ExitProcess", &[0]).1, ExitReason::ProcessExit(0));
    assert_eq!(guest.state.open_handle_count(), 0);
    // The drive is not the process: the file it created is still there.
    assert_eq!(guest.state.file_contents("C:\\exit.bin"), Some(Vec::new()));
    assert_eq!(guest.state.file_contents("C:\\absent.bin"), None);
}

/// A PE32+ image with one executable section and no imports whose entry
/// point is `mov eax, status; ret`.
fn returning_image(status: u32) -> Vec<u8> {
    let mut pe = vec![0u8; 0x400];
    let mut put = |at: usize, bytes: &[u8]| pe[at..at + bytes.len()].copy_from_slice(bytes);
    put(0, b"MZ");
    put(0x3C, &0x80u32.to_le_bytes());
    put(0x80, b"PE\0\0");
    put(0x84, &0x8664u16.to_le_bytes()); // AMD64
    put(0x86, &1u16.to_le_bytes()); // one section
    put(0x94, &240u16.to_le_bytes()); // optional header size
    put(0x96, &0x0022u16.to_le_bytes()); // executable, large-address-aware
    let optional = 0x98;
    put(optional, &0x20Bu16.to_le_bytes()); // PE32+
    put(optional + 16, &0x1000u32.to_le_bytes()); // entry point
    put(optional + 24, &0x1_4000_0000u64.to_le_bytes()); // image base
    put(optional + 32, &0x1000u32.to_le_bytes()); // section alignment
    put(optional + 36, &0x200u32.to_le_bytes()); // file alignment
    put(optional + 56, &0x2000u32.to_le_bytes()); // size of image
    put(optional + 60, &0x200u32.to_le_bytes()); // size of headers
    put(optional + 108, &16u32.to_le_bytes()); // data directories
    let section = optional + 240;
    put(section, b".text");
    put(section + 8, &0x200u32.to_le_bytes()); // virtual size
    put(section + 12, &0x1000u32.to_le_bytes()); // RVA
    put(section + 16, &0x200u32.to_le_bytes()); // raw size
    put(section + 20, &0x200u32.to_le_bytes()); // raw offset
    put(section + 36, &0x6000_0020u32.to_le_bytes()); // code, execute, read
    put(0x200, &[0xB8]);
    put(0x201, &status.to_le_bytes());
    put(0x205, &[0xC3]);
    pe
}

fn layout() -> StartupLayout {
    StartupLayout {
        image_base: 0x1_4000_0000,
        stack_top: 0x2_0001_0000,
        tls_base: 0x3_0000_0000,
    }
}

#[test]
fn a_returning_entry_point_exits_with_eax() {
    // With no ExitProcess registered the return still ends the process.
    let bare = run_synthetic_startup(
        &returning_image(0x1234_5678),
        layout(),
        ImportRegistry::new(),
        100,
    )
    .unwrap();
    assert_eq!(
        bare.execution.exit_reason,
        ExitReason::ProcessExit(0x1234_5678)
    );
    // `mov eax, imm32` and `ret`, then nothing.
    assert_eq!(bare.execution.instructions_executed, 2);

    // With the standard services it goes through ExitProcess, so the status
    // is recorded where a host can read it.
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let mut registry = ImportRegistry::new();
    register_standard_services(&mut registry, Arc::clone(&state));
    let standard = run_synthetic_startup(&returning_image(77), layout(), registry, 100).unwrap();
    assert_eq!(standard.execution.exit_reason, ExitReason::ProcessExit(77));
    assert_eq!(state.exit_status(), Some(77));
}

// ---------------------------------------------------------------------------
// Exit before the entry point, and the reserved return addresses
// ---------------------------------------------------------------------------

const IMAGE_BASE: u64 = 0x1_4000_0000;
const ENTRY_RVA: u32 = 0x1000;
/// TLS callback `n` is at this RVA plus 0x20 for each one before it.
const CALLBACK_RVA: u32 = 0x1040;
const IAT_RVA: u32 = 0x2070;
/// One byte per marker: a routine that ran sets its own to 1.
const MARKER_RVA: u32 = 0x2100;

/// A PE32+ image that imports one kernel32 function. `entry` is its entry
/// point and each of `callbacks` is a TLS callback.
fn image(symbol: &str, entry: &[u8], callbacks: &[Vec<u8>]) -> Vec<u8> {
    assert!(entry.len() <= 0x40 && callbacks.len() <= 4);
    let mut pe = vec![0u8; 0x600];
    let mut put = |at: usize, bytes: &[u8]| pe[at..at + bytes.len()].copy_from_slice(bytes);
    put(0, b"MZ");
    put(0x3C, &0x80u32.to_le_bytes());
    put(0x80, b"PE\0\0");
    put(0x84, &0x8664u16.to_le_bytes());
    put(0x86, &2u16.to_le_bytes());
    put(0x94, &240u16.to_le_bytes());
    put(0x96, &0x0022u16.to_le_bytes());
    let optional = 0x98;
    put(optional, &0x20Bu16.to_le_bytes());
    put(optional + 16, &ENTRY_RVA.to_le_bytes());
    put(optional + 24, &IMAGE_BASE.to_le_bytes());
    put(optional + 32, &0x1000u32.to_le_bytes());
    put(optional + 36, &0x200u32.to_le_bytes());
    put(optional + 56, &0x3000u32.to_le_bytes());
    put(optional + 60, &0x200u32.to_le_bytes());
    put(optional + 108, &16u32.to_le_bytes());
    // Data directory 1: imports. Data directory 9: TLS.
    put(optional + 120, &0x2000u32.to_le_bytes());
    put(optional + 124, &0x28u32.to_le_bytes());
    if !callbacks.is_empty() {
        put(optional + 184, &0x2080u32.to_le_bytes());
        put(optional + 188, &40u32.to_le_bytes());
    }
    for (index, (name, rva, raw, flags)) in [
        (b".text", 0x1000u32, 0x200u32, 0x6000_0020u32),
        (b".data", 0x2000, 0x400, 0xC000_0040),
    ]
    .into_iter()
    .enumerate()
    {
        let section = optional + 240 + 40 * index;
        put(section, name);
        put(section + 8, &0x200u32.to_le_bytes());
        put(section + 12, &rva.to_le_bytes());
        put(section + 16, &0x200u32.to_le_bytes());
        put(section + 20, &raw.to_le_bytes());
        put(section + 36, &flags.to_le_bytes());
    }
    // `.data`, at file offset 0x400: the import descriptor, its strings, the
    // lookup table and the address table.
    let data = |rva: u32| 0x400 + (rva - 0x2000) as usize;
    put(data(0x2000), &0x2060u32.to_le_bytes()); // lookup table
    put(data(0x200C), &0x2040u32.to_le_bytes()); // DLL name
    put(data(0x2010), &IAT_RVA.to_le_bytes());
    put(data(0x2040), KERNEL32.as_bytes());
    put(data(0x2052), symbol.as_bytes());
    put(data(0x2060), &0x2050u64.to_le_bytes());
    put(data(IAT_RVA), &0x2050u64.to_le_bytes());
    if !callbacks.is_empty() {
        // The TLS directory: no template, an index slot, and the callbacks.
        put(data(0x2080) + 16, &(IMAGE_BASE + 0x20C0).to_le_bytes());
        put(data(0x2080) + 24, &(IMAGE_BASE + 0x20D0).to_le_bytes());
    }
    for (index, code) in callbacks.iter().enumerate() {
        assert!(code.len() <= 0x20);
        let rva = CALLBACK_RVA + 0x20 * index as u32;
        put(
            data(0x20D0) + 8 * index,
            &(IMAGE_BASE + u64::from(rva)).to_le_bytes(),
        );
        put(0x200 + (rva - 0x1000) as usize, code);
    }
    put(0x200, entry);
    pe
}

/// `mov byte [marker], 1`, for code that starts at `rva`.
fn set_marker(rva: u32, marker: u32) -> Vec<u8> {
    let mut code = vec![0xC6, 0x05];
    code.extend((MARKER_RVA + marker - (rva + 7)).to_le_bytes());
    code.push(1);
    code
}

/// `mov ecx, status; call [import]; hlt`, for code that starts at `rva`.
fn call_import_with(rva: u32, status: u32) -> Vec<u8> {
    let mut code = vec![0xB9];
    code.extend(status.to_le_bytes());
    code.extend([0xFF, 0x15]);
    code.extend((IAT_RVA - (rva + 11)).to_le_bytes());
    code.push(0xF4);
    code
}

fn then(mut code: Vec<u8>, last: u8) -> Vec<u8> {
    code.push(last);
    code
}

const RET: u8 = 0xC3;
const HLT: u8 = 0xF4;

struct Started {
    result: xenora_loader::startup::StartupResult,
    state: Arc<ServicesState>,
}

impl Started {
    fn markers(&self) -> [u8; 3] {
        let mut markers = [0; 3];
        self.result
            .image
            .memory
            .read_bytes(IMAGE_BASE + u64::from(MARKER_RVA), &mut markers)
            .unwrap();
        markers
    }
}

fn start(bytes: &[u8]) -> Result<Started, String> {
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let mut registry = ImportRegistry::new();
    register_standard_services(&mut registry, Arc::clone(&state));
    run_synthetic_startup(bytes, layout(), registry, 1_000)
        .map(|result| Started { result, state })
        .map_err(|error| error.to_string())
}

#[test]
fn exit_in_a_tls_callback_is_an_exit_and_nothing_after_it_runs() {
    // Markers: 0 and 1 for the two callbacks, 2 for the entry point.
    let entry = then(set_marker(ENTRY_RVA, 2), HLT);
    let first = CALLBACK_RVA;
    let second = CALLBACK_RVA + 0x20;

    // Control: no callback exits. Both run, then the entry point.
    let quiet = start(&image(
        "ExitProcess",
        &entry,
        &[
            then(set_marker(first, 0), RET),
            then(set_marker(second, 1), RET),
        ],
    ))
    .unwrap();
    assert_eq!(quiet.result.execution.exit_reason, ExitReason::Halt);
    assert_eq!(quiet.result.tls_callbacks, 2);
    assert_eq!(quiet.markers(), [1, 1, 1]);
    assert_eq!(quiet.state.exit_status(), None);

    // The first callback exits: the second and the entry point never run.
    let early = start(&image(
        "ExitProcess",
        &entry,
        &[
            call_import_with(first, 77),
            then(set_marker(second, 1), RET),
        ],
    ))
    .unwrap();
    assert_eq!(
        early.result.execution.exit_reason,
        ExitReason::ProcessExit(77)
    );
    assert_eq!(early.result.execution.instructions_executed, 0);
    assert_eq!(early.result.tls_callbacks, 0);
    assert_eq!(early.markers(), [0, 0, 0]);
    assert_eq!(early.state.exit_status(), Some(77));

    // The second callback exits: the first finished, the entry point never runs.
    let late = start(&image(
        "ExitProcess",
        &entry,
        &[
            then(set_marker(first, 0), RET),
            call_import_with(second, 0x1_0002),
        ],
    ))
    .unwrap();
    assert_eq!(
        late.result.execution.exit_reason,
        ExitReason::ProcessExit(0x1_0002)
    );
    assert_eq!(late.result.tls_callbacks, 1);
    assert_eq!(late.markers(), [1, 0, 0]);
    assert_eq!(late.state.exit_status(), Some(0x1_0002));
}

#[test]
fn a_fault_in_a_tls_callback_is_still_a_startup_error() {
    let entry = then(set_marker(ENTRY_RVA, 2), HLT);
    let error = start(&image("ExitProcess", &entry, &[vec![HLT]]))
        .err()
        .expect("a callback that halts is a fault");
    assert_eq!(
        error,
        "fault during TLS callback execution: guest executed HLT"
    );
}

#[test]
fn an_image_over_the_reserved_return_addresses_is_refused() {
    // The entry point of this image would be exactly the address that means
    // "the entry point returned". Unreserved, the engine would report an exit
    // with status 0 before running one instruction of it.
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let mut registry = ImportRegistry::new();
    register_standard_services(&mut registry, Arc::clone(&state));
    let over = StartupLayout {
        image_base: ENTRY_RETURN - u64::from(ENTRY_RVA),
        ..layout()
    };
    let error = run_synthetic_startup(&returning_image(5), over, registry, 100)
        .err()
        .expect("refused")
        .to_string();
    assert_eq!(
        error,
        format!("overlapping memory region: base 0x{RETURN_SENTINEL_PAGE:016x}, size 0x1000")
    );
    assert_eq!(state.exit_status(), None);
}

#[test]
fn the_guest_cannot_map_the_reserved_return_addresses() {
    // The page is there after startup, and nothing can read, write or run it.
    let mut started = start(&image("ExitProcess", &[HLT], &[])).unwrap();
    let memory = &mut started.result.image.memory;
    assert!(matches!(
        memory.read_u8(ENTRY_RETURN),
        Err(MemoryError::PermissionViolation { .. })
    ));
    assert!(matches!(
        memory.map_region(RETURN_SENTINEL_PAGE, 0x1000, MemoryPermissions::READ_WRITE),
        Err(MemoryError::OverlappingRegion { .. })
    ));

    // The entry point calls VirtualAlloc(address, 0x1000, MEM_COMMIT |
    // MEM_RESERVE, PAGE_READWRITE) and returns 1 if it got a null pointer.
    let allocate_at = |address: u64| {
        let mut code = vec![0x48, 0xB9]; // mov rcx, address
        code.extend(address.to_le_bytes());
        code.extend([0xBA, 0x00, 0x10, 0x00, 0x00]); // mov edx, 0x1000
        code.extend([0x41, 0xB8, 0x00, 0x30, 0x00, 0x00]); // mov r8d, 0x3000
        code.extend([0x41, 0xB9, 0x04, 0x00, 0x00, 0x00]); // mov r9d, 4
        code.extend([0x48, 0x83, 0xEC, 0x28]); // sub rsp, 0x28
        code.extend([0xFF, 0x15]); // call [VirtualAlloc]
        code.extend((IAT_RVA - (ENTRY_RVA + code.len() as u32 + 4)).to_le_bytes());
        code.extend([0x48, 0x83, 0xC4, 0x28]); // add rsp, 0x28
        code.extend([0x48, 0x85, 0xC0]); // test rax, rax
        code.extend([0x0F, 0x94, 0xC0]); // setz al
        code.extend([0x0F, 0xB6, 0xC0]); // movzx eax, al
        code.push(RET);
        start(&image("VirtualAlloc", &code, &[]))
            .unwrap()
            .result
            .execution
            .exit_reason
    };
    // Control: a free address is allocated.
    assert_eq!(allocate_at(0x6_0000_0000), ExitReason::ProcessExit(0));
    assert_eq!(
        allocate_at(RETURN_SENTINEL_PAGE),
        ExitReason::ProcessExit(1)
    );
}
