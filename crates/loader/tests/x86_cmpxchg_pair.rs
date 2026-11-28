// SPDX-License-Identifier: GPL-3.0-or-later

//! CMPXCHG8B and CMPXCHG16B (issue #971).
//!
//! The expected values come from the operation section of Intel SDM Vol. 2A,
//! "CMPXCHG8B/CMPXCHG16B", and are written here as literal results. That
//! section compares EDX:EAX, or RDX:RAX with REX.W, with the memory operand.
//! On a match, ZF is set and ECX:EBX or RCX:RBX is stored. Otherwise, ZF is
//! cleared, the operand is loaded into the register pair and written back
//! unchanged. A 32-bit register write clears bits 63:32 (Vol. 1, 3.4.1.1).
//! CMPXCHG16B needs a 16-byte aligned operand. The encodings were assembled
//! with LLVM `llvm-mc` for x86-64. Nothing was compared with x86 hardware.
//!
//! Both engines run one guest instruction at a time on one host thread, so
//! these tests fix the single-thread results. They say nothing about
//! concurrent execution.

use xenora_loader::cpu::{CpuState, ExitReason, Interpreter, RFlags};
use xenora_loader::dbt::{DbtEngine, is_native};
use xenora_loader::memory::{GuestMemory, MemoryPermissions};
use xenora_loader::x86_decode::decode;

const ENTRY: u64 = 0x1000;
/// Two writable pages. Nothing is mapped after them unless a test says so.
const DATA: u64 = 0x4000;
const PAGE: u64 = 0x1000;
const END: u64 = DATA + 2 * PAGE;

const RAX: usize = 0;
const RCX: usize = 1;
const RDX: usize = 2;
const RBX: usize = 3;
const RSI: usize = 6;
const RDI: usize = 7;

const CMPXCHG8B: &[u8] = &[0x0F, 0xC7, 0x0E]; // cmpxchg8b [rsi]
const LOCK_CMPXCHG8B: &[u8] = &[0xF0, 0x0F, 0xC7, 0x0E]; // lock cmpxchg8b [rsi]
const CMPXCHG16B: &[u8] = &[0x48, 0x0F, 0xC7, 0x0E]; // cmpxchg16b [rsi]
const LOCK_CMPXCHG16B: &[u8] = &[0xF0, 0x48, 0x0F, 0xC7, 0x0E]; // lock cmpxchg16b [rsi]

fn image(code: &[u8]) -> GuestMemory {
    let mut memory = GuestMemory::new();
    memory
        .map_region_with_data(ENTRY, code.len(), MemoryPermissions::READ_EXECUTE, code)
        .unwrap();
    memory
        .map_region(DATA, 2 * PAGE as usize, MemoryPermissions::READ_WRITE)
        .unwrap();
    memory
}

fn data(memory: &GuestMemory) -> Vec<u8> {
    let mut out = vec![0; 2 * PAGE as usize];
    memory.read_bytes(DATA, &mut out).unwrap();
    out
}

struct Run {
    name: &'static str,
    cpu: CpuState,
    exit: ExitReason,
    memory: GuestMemory,
    /// Instructions the DBT ran on the shared semantics.
    helpers: usize,
}

/// The interpreter and the DBT, each from its own copy of `cpu` and `memory`.
fn both(cpu: &CpuState, memory: &GuestMemory, budget: usize) -> [Run; 2] {
    let mut interpreted = memory.clone();
    let result = Interpreter::new(cpu.clone(), &mut interpreted, budget).run();
    let mut translated = memory.clone();
    let mut engine = DbtEngine::new(64 * 1024).unwrap();
    let mut state = cpu.clone();
    let native = engine.run(&mut state, &mut translated, budget);
    [
        Run {
            name: "interpreter",
            cpu: result.final_state,
            exit: result.exit_reason,
            memory: interpreted,
            helpers: 0,
        },
        Run {
            name: "DBT",
            cpu: native.final_state,
            exit: native.exit_reason,
            memory: translated,
            helpers: engine.metrics.interpreter_fallbacks,
        },
    ]
}

fn flags(cf: bool, zf: bool, sf: bool, of: bool, pf: bool, af: bool) -> RFlags {
    RFlags {
        cf,
        zf,
        sf,
        of,
        pf,
        af,
    }
}

fn decoded(code: &[u8]) -> xenora_loader::x86_decode::Insn {
    decode(ENTRY, |at| {
        code.get((at - ENTRY) as usize).copied().ok_or(())
    })
    .unwrap_or_else(|error| panic!("{code:02X?}: {error:?}"))
}

/// Upper halves that the 8-byte form must ignore and, on a match, keep.
const RAX_IN: u64 = 0x1111_1111_2222_2222;
const RDX_IN: u64 = 0x3333_3333_4444_4444;
const RBX_IN: u64 = 0x5555_5555_6666_6666;
const RCX_IN: u64 = 0x7777_7777_8888_8888;

/// One case: the operand before, the operand after, and RAX and RDX after.
struct Case {
    before: &'static [u8],
    after: &'static [u8],
    rax: u64,
    rdx: u64,
    zf: bool,
}

#[test]
fn each_width_compares_both_halves_and_takes_each_branch() {
    // CMPXCHG8B compares EDX:EAX = 44444444:22222222 with the quadword.
    let eight = [
        // A match stores ECX:EBX = 88888888:66666666 and writes no register.
        Case {
            before: &[0x22, 0x22, 0x22, 0x22, 0x44, 0x44, 0x44, 0x44],
            after: &[0x66, 0x66, 0x66, 0x66, 0x88, 0x88, 0x88, 0x88],
            rax: RAX_IN,
            rdx: RDX_IN,
            zf: true,
        },
        // Bit 0 differs: EAX and EDX are loaded and bits 63:32 cleared.
        Case {
            before: &[0x23, 0x22, 0x22, 0x22, 0x44, 0x44, 0x44, 0x44],
            after: &[0x23, 0x22, 0x22, 0x22, 0x44, 0x44, 0x44, 0x44],
            rax: 0x2222_2223,
            rdx: 0x4444_4444,
            zf: false,
        },
        // Bit 63 differs, so the high half is compared too.
        Case {
            before: &[0x22, 0x22, 0x22, 0x22, 0x44, 0x44, 0x44, 0xC4],
            after: &[0x22, 0x22, 0x22, 0x22, 0x44, 0x44, 0x44, 0xC4],
            rax: 0x2222_2222,
            rdx: 0xC444_4444,
            zf: false,
        },
    ];
    // CMPXCHG16B compares RDX:RAX with the double quadword.
    let sixteen = [
        Case {
            before: &[
                0x22, 0x22, 0x22, 0x22, 0x11, 0x11, 0x11, 0x11, 0x44, 0x44, 0x44, 0x44, 0x33, 0x33,
                0x33, 0x33,
            ],
            after: &[
                0x66, 0x66, 0x66, 0x66, 0x55, 0x55, 0x55, 0x55, 0x88, 0x88, 0x88, 0x88, 0x77, 0x77,
                0x77, 0x77,
            ],
            rax: RAX_IN,
            rdx: RDX_IN,
            zf: true,
        },
        // Bit 63 of the low quadword differs.
        Case {
            before: &[
                0x22, 0x22, 0x22, 0x22, 0x11, 0x11, 0x11, 0x91, 0x44, 0x44, 0x44, 0x44, 0x33, 0x33,
                0x33, 0x33,
            ],
            after: &[
                0x22, 0x22, 0x22, 0x22, 0x11, 0x11, 0x11, 0x91, 0x44, 0x44, 0x44, 0x44, 0x33, 0x33,
                0x33, 0x33,
            ],
            rax: 0x9111_1111_2222_2222,
            rdx: RDX_IN,
            zf: false,
        },
        // Bit 64, the lowest of the high quadword, differs.
        Case {
            before: &[
                0x22, 0x22, 0x22, 0x22, 0x11, 0x11, 0x11, 0x11, 0x45, 0x44, 0x44, 0x44, 0x33, 0x33,
                0x33, 0x33,
            ],
            after: &[
                0x22, 0x22, 0x22, 0x22, 0x11, 0x11, 0x11, 0x11, 0x45, 0x44, 0x44, 0x44, 0x33, 0x33,
                0x33, 0x33,
            ],
            rax: RAX_IN,
            rdx: 0x3333_3333_4444_4445,
            zf: false,
        },
    ];
    let at = DATA + 0x40;
    for (code, cases) in [
        (CMPXCHG8B, &eight),
        (LOCK_CMPXCHG8B, &eight),
        (CMPXCHG16B, &sixteen),
        (LOCK_CMPXCHG16B, &sixteen),
    ] {
        let insn = decoded(code);
        assert_eq!(usize::from(insn.len), code.len(), "{code:02X?}");
        assert_eq!(insn.lock, code[0] == 0xF0, "{code:02X?}");
        assert!(!is_native(&insn), "{code:02X?}");
        for case in cases {
            // Every flag starts opposite to ZF's result and set or clear, so
            // a write to any flag other than ZF shows.
            for others in [false, true] {
                let mut cpu = CpuState::new(ENTRY, 0);
                cpu.gpr[RAX] = RAX_IN;
                cpu.gpr[RDX] = RDX_IN;
                cpu.gpr[RBX] = RBX_IN;
                cpu.gpr[RCX] = RCX_IN;
                cpu.gpr[RSI] = at;
                cpu.flags = flags(others, !case.zf, others, others, others, others);
                let mut memory = image(code);
                // Sentinels on either side of the operand stay as they are.
                memory.write_bytes(at - 8, &[0xEE; 40]).unwrap();
                memory.write_bytes(at, case.before).unwrap();

                let mut wanted = cpu.clone();
                wanted.rip = ENTRY + code.len() as u64;
                wanted.gpr[RAX] = case.rax;
                wanted.gpr[RDX] = case.rdx;
                wanted.flags.zf = case.zf;
                let mut wanted_memory = memory.clone();
                wanted_memory.write_bytes(at, case.after).unwrap();

                for run in both(&cpu, &memory, 1) {
                    let what = format!(
                        "{code:02X?} {:02X?} flags={others} on the {}",
                        case.before, run.name
                    );
                    assert_eq!(run.exit, ExitReason::StepLimitExceeded, "{what}");
                    assert_eq!(run.cpu, wanted, "{what}");
                    assert_eq!(data(&run.memory), data(&wanted_memory), "{what}");
                    if run.name == "DBT" {
                        assert_eq!(run.helpers, 1, "{what}");
                    }
                }
            }
        }
    }
}

#[test]
fn operands_address_like_any_other_memory_operand() {
    struct Addressed {
        code: &'static [u8],
        /// CMPXCHG16B rather than CMPXCHG8B.
        wide: bool,
        rsi: u64,
        rdi: u64,
        fs: u64,
        gs: u64,
        /// The effective address.
        at: u64,
    }
    let plain = |code, wide, rsi, rdi, at| Addressed {
        code,
        wide,
        rsi,
        rdi,
        fs: 0,
        gs: 0,
        at,
    };
    let forms = [
        // cmpxchg8b [rsi + 8*rdi + 16]
        plain(&[0x0F, 0xC7, 0x4C, 0xFE, 0x10], false, DATA, 3, DATA + 0x28),
        // cmpxchg8b [rsi + 1]: CMPXCHG8B needs no alignment.
        plain(
            &[0x0F, 0xC7, 0x4E, 0x01],
            false,
            DATA + 0x40,
            0,
            DATA + 0x41,
        ),
        // cmpxchg16b [rsi + 8] on an aligned sum.
        plain(
            &[0x48, 0x0F, 0xC7, 0x4E, 0x08],
            true,
            DATA + 0x2F8,
            0,
            DATA + 0x300,
        ),
        // cmpxchg8b fs:[rsi]
        Addressed {
            fs: DATA + 0x100,
            ..plain(&[0x64, 0x0F, 0xC7, 0x0E], false, 0x20, 0, DATA + 0x120)
        },
        // lock cmpxchg16b gs:[rsi]
        Addressed {
            gs: DATA + 0x200,
            ..plain(
                &[0x65, 0xF0, 0x48, 0x0F, 0xC7, 0x0E],
                true,
                0x30,
                0,
                DATA + 0x230,
            )
        },
    ];
    for Addressed {
        code,
        wide,
        rsi,
        rdi,
        fs,
        gs,
        at,
    } in forms
    {
        let mut cpu = CpuState::new(ENTRY, 0);
        // The pair matches: zero in RDX:RAX against a zero operand.
        cpu.gpr[RBX] = 0x0102_0304_0506_0708;
        cpu.gpr[RCX] = 0x1112_1314_1516_1718;
        cpu.gpr[RSI] = rsi;
        cpu.gpr[RDI] = rdi;
        cpu.fs_base = fs;
        cpu.gs_base = gs;
        let memory = image(code);
        let stored: &[u8] = if wide {
            &[
                0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0x18, 0x17, 0x16, 0x15, 0x14, 0x13,
                0x12, 0x11,
            ]
        } else {
            &[0x08, 0x07, 0x06, 0x05, 0x18, 0x17, 0x16, 0x15]
        };
        let mut wanted_memory = memory.clone();
        wanted_memory.write_bytes(at, stored).unwrap();
        for run in both(&cpu, &memory, 1) {
            let what = format!("{code:02X?} on the {}", run.name);
            assert_eq!(run.exit, ExitReason::StepLimitExceeded, "{what}");
            assert!(run.cpu.flags.zf, "{what}");
            assert_eq!(data(&run.memory), data(&wanted_memory), "{what}");
        }
    }
}

#[test]
fn an_eight_byte_operand_may_span_two_regions() {
    // The operand's first four bytes are the end of the data, the last four
    // the start of a second writable region.
    let at = END - 4;
    let mut cpu = CpuState::new(ENTRY, 0);
    cpu.gpr[RAX] = 0xDDCC_BBAA;
    cpu.gpr[RDX] = 0x4433_2211;
    cpu.gpr[RBX] = 0x0102_0304;
    cpu.gpr[RCX] = 0x0506_0708;
    cpu.gpr[RSI] = at;
    let mut memory = image(LOCK_CMPXCHG8B);
    memory
        .map_region(END, PAGE as usize, MemoryPermissions::READ_WRITE)
        .unwrap();
    memory
        .write_bytes(at, &[0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44])
        .unwrap();
    for run in both(&cpu, &memory, 1) {
        assert_eq!(run.exit, ExitReason::StepLimitExceeded, "{}", run.name);
        assert!(run.cpu.flags.zf, "{}", run.name);
        let mut stored = [0; 8];
        run.memory.read_bytes(at, &mut stored).unwrap();
        assert_eq!(
            stored,
            [0x04, 0x03, 0x02, 0x01, 0x08, 0x07, 0x06, 0x05],
            "{}",
            run.name
        );
    }
}

#[test]
fn cmpxchg16b_needs_a_sixteen_byte_aligned_operand() {
    // #GP(0) for a misaligned CMPXCHG16B operand, with or without LOCK, even
    // where the pair would match. CMPXCHG8B at the same addresses runs.
    for offset in [1, 4, 8, 15] {
        let at = DATA + 0x100 + offset;
        for (code, faults) in [
            (CMPXCHG16B, true),
            (LOCK_CMPXCHG16B, true),
            (CMPXCHG8B, false),
            (LOCK_CMPXCHG8B, false),
        ] {
            let mut cpu = CpuState::new(ENTRY, 0);
            cpu.gpr[RBX] = 1;
            cpu.gpr[RSI] = at;
            let memory = image(code);
            for run in both(&cpu, &memory, 1) {
                let what = format!("{code:02X?} at {at:#x} on the {}", run.name);
                if faults {
                    assert_eq!(run.exit, ExitReason::GeneralProtection, "{what}");
                    assert_eq!(run.cpu, cpu, "{what}");
                    assert_eq!(data(&run.memory), data(&memory), "{what}");
                } else {
                    assert_eq!(run.exit, ExitReason::StepLimitExceeded, "{what}");
                    assert_eq!(run.memory.read_u64(at).unwrap(), 1, "{what}");
                }
            }
        }
    }
}

#[test]
fn a_noncanonical_operand_is_a_general_protection_fault() {
    // An operand at, or running into, the first noncanonical address.
    for (code, at) in [
        (CMPXCHG16B, 0x0000_8000_0000_0000),
        (CMPXCHG8B, 0x0000_8000_0000_0000),
        (CMPXCHG8B, 0x0000_7FFF_FFFF_FFFC),
        (LOCK_CMPXCHG16B, 0xFFFF_7FFF_FFFF_FFF0),
    ] {
        let mut cpu = CpuState::new(ENTRY, 0);
        cpu.gpr[RSI] = at;
        let memory = image(code);
        for run in both(&cpu, &memory, 1) {
            let what = format!("{code:02X?} at {at:#x} on the {}", run.name);
            assert_eq!(run.exit, ExitReason::GeneralProtection, "{what}");
            assert_eq!(run.cpu, cpu, "{what}");
        }
    }
}

#[test]
fn a_fault_on_any_byte_of_the_operand_changes_nothing() {
    // (code, operand address, a read-only page after the data, writable bytes)
    let cases: &[(&[u8], u64, bool, usize)] = &[
        // The last four bytes of the data, then unmapped or read-only bytes.
        (CMPXCHG8B, END - 4, false, 4),
        (LOCK_CMPXCHG8B, END - 4, true, 4),
        // Wholly unmapped, and wholly read-only.
        (CMPXCHG8B, END, false, 0),
        (LOCK_CMPXCHG8B, END, true, 0),
        (CMPXCHG16B, END, false, 0),
        (LOCK_CMPXCHG16B, END, true, 0),
    ];
    for &(code, at, read_only, writable) in cases {
        // A pair that matches the operand and one that does not: a failed
        // compare writes the operand back, so it needs the write access too
        // and must not load RDX:RAX.
        for matches in [true, false] {
            let mut memory = image(code);
            if read_only {
                memory
                    .map_region(END, PAGE as usize, MemoryPermissions::READ_ONLY)
                    .unwrap();
            }
            if writable > 0 {
                memory.write_bytes(at, &[0; 4][..writable]).unwrap();
            }
            let mut cpu = CpuState::new(ENTRY, 0);
            cpu.gpr[RAX] = u64::from(!matches);
            cpu.gpr[RBX] = 0x1122_3344_5566_7788;
            cpu.gpr[RCX] = 0x99AA_BBCC_DDEE_FF00;
            cpu.gpr[RSI] = at;
            cpu.flags = flags(true, false, true, false, true, false);
            for run in both(&cpu, &memory, 1) {
                let what = format!(
                    "{code:02X?} at {at:#x} read-only={read_only} matches={matches} on the {}",
                    run.name
                );
                assert!(
                    matches!(run.exit, ExitReason::MemoryFault(_)),
                    "{what}: {:?}",
                    run.exit
                );
                assert_eq!(run.cpu, cpu, "{what}");
                assert_eq!(data(&run.memory), data(&memory), "{what}");
                if read_only {
                    let mut tail = [0xFF; 16];
                    run.memory.read_bytes(END, &mut tail).unwrap();
                    assert_eq!(tail, [0; 16], "{what}");
                }
            }
        }
    }
}

#[test]
fn register_operands_and_the_other_group_digits_stay_illegal() {
    let rejected: &[&[u8]] = &[
        &[0x0F, 0xC7, 0xC8],       // 0F C7 /1 with ModRM.mod = 3 (EAX)
        &[0x48, 0x0F, 0xC7, 0xC8], // the same with REX.W (RAX)
        &[0xF0, 0x0F, 0xC7, 0xC8], // and with LOCK
        &[0x0F, 0xC7, 0x06],       // 0F C7 /0 [rsi]
        &[0x0F, 0xC7, 0x16],       // 0F C7 /2 [rsi]
        &[0x0F, 0xC7, 0x1E],       // 0F C7 /3 [rsi]
        &[0x0F, 0xC7, 0x26],       // 0F C7 /4 [rsi]
        &[0x0F, 0xC7, 0x2E],       // 0F C7 /5 [rsi]
        &[0x0F, 0xC7, 0x36],       // 0F C7 /6 [rsi]
        &[0x0F, 0xC7, 0x3E],       // 0F C7 /7 [rsi]
        &[0x0F, 0xC7, 0xF0],       // rdrand eax
        &[0x0F, 0xC7, 0xF8],       // rdseed eax
        &[0xF0, 0x0F, 0xC7, 0x06], // lock with /0
    ];
    for code in rejected {
        let mut padded = code.to_vec();
        padded.extend([0x90; 8]);
        let mut cpu = CpuState::new(ENTRY, 0);
        cpu.gpr[RSI] = DATA;
        let memory = image(&padded);
        for run in both(&cpu, &memory, 1) {
            assert!(
                matches!(run.exit, ExitReason::IllegalInstruction(_)),
                "{code:02X?} on the {}: {:?}",
                run.name,
                run.exit
            );
            assert_eq!(run.cpu, cpu, "{code:02X?}");
            assert_eq!(data(&run.memory), data(&memory), "{code:02X?}");
        }
    }
}

#[test]
fn a_retry_loop_increments_a_128_bit_counter() {
    //        mov rax, [rsi + 16]      ; a stale snapshot of the counter
    //        mov rdx, [rsi + 24]
    // retry: mov rbx, rax
    //        mov rcx, rdx
    //        add rbx, 1
    //        adc rcx, 0
    //        lock cmpxchg16b [rsi]
    //        jnz retry
    //        hlt
    let code = [
        0x48, 0x8B, 0x46, 0x10, 0x48, 0x8B, 0x56, 0x18, 0x48, 0x89, 0xC3, 0x48, 0x89, 0xD1, 0x48,
        0x83, 0xC3, 0x01, 0x48, 0x83, 0xD1, 0x00, 0xF0, 0x48, 0x0F, 0xC7, 0x0E, 0x75, 0xEB, 0xF4,
    ];
    let mut memory = image(&code);
    // The counter is 1:FFFFFFFFFFFFFFFF and the snapshot 0:5.
    memory
        .write_bytes(
            DATA,
            &[
                0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0, 0, 0, 0, 0, 0, 0, 0x05, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
        )
        .unwrap();
    let mut cpu = CpuState::new(ENTRY, 0);
    cpu.gpr[RSI] = DATA;
    for run in both(&cpu, &memory, 64) {
        let what = run.name;
        assert_eq!(run.exit, ExitReason::Halt, "{what}");
        assert_eq!(run.cpu.rip, ENTRY + code.len() as u64, "{what}");
        // The first exchange fails on the stale snapshot and loads the
        // counter. The second stores 1:FFFFFFFFFFFFFFFF + 1 = 2:0.
        assert_eq!(run.cpu.gpr[RAX], 0xFFFF_FFFF_FFFF_FFFF, "{what}");
        assert_eq!(run.cpu.gpr[RDX], 1, "{what}");
        assert_eq!(run.cpu.gpr[RBX], 0, "{what}");
        assert_eq!(run.cpu.gpr[RCX], 2, "{what}");
        let mut counter = [0; 16];
        run.memory.read_bytes(DATA, &mut counter).unwrap();
        assert_eq!(
            counter,
            [0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0],
            "{what}"
        );
        // ADC 1 + 0 + 1 = 2 clears CF, ZF, SF, OF, PF and AF; the exchange
        // then sets ZF alone.
        assert_eq!(
            run.cpu.flags,
            flags(false, true, false, false, false, false),
            "{what}"
        );
        if run.name == "DBT" {
            assert_eq!(run.helpers, 2, "the two exchanges");
        }
    }
}
