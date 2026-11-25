// SPDX-License-Identifier: GPL-3.0-or-later

//! Multiword ADC and SBB carry chains through the interpreter and the native
//! DBT.
//!
//! Encodings were assembled with LLVM (`llvm-mc -triple=x86_64`, Intel syntax).
//! Expected values come from widening `u128` and `i128` arithmetic over the
//! limbs, following the ADC and SBB operation sections of the Intel SDM Vol. 2:
//! DEST = DEST + SRC + CF and DEST = DEST - (SRC + CF). They never read the
//! production semantics. Both engines must also agree on the complete state,
//! including PF and AF, which the reference does not model.

use xenora_loader::cpu::{CpuState, ExitReason, Interpreter};
use xenora_loader::dbt::{DbtEngine, is_native};
use xenora_loader::memory::{GuestMemory, MemoryPermissions};
use xenora_loader::x86_decode::decode;
use xenora_loader::x86_table::Op;

const ENTRY: u64 = 0x1000;
/// `jmp +0`: ends the translated block, so the next instruction reads CF from
/// the guest state stored by the previous block.
const JMP: [u8; 2] = [0xEB, 0x00];
const HLT: u8 = 0xF4;

/// Limb registers: A in RAX, RBX, RCX, RDX and B in R8 to R11.
const A: [usize; 4] = [0, 3, 1, 2];
const B: [usize; 4] = [8, 9, 10, 11];

// add rax, r8; adc rbx, r9; adc rcx, r10; adc rdx, r11 (and the SUB/SBB and
// 32-bit equivalents).
const ADD64: [&[u8]; 4] = [
    &[0x4C, 0x01, 0xC0],
    &[0x4C, 0x11, 0xCB],
    &[0x4C, 0x11, 0xD1],
    &[0x4C, 0x11, 0xDA],
];
const SUB64: [&[u8]; 4] = [
    &[0x4C, 0x29, 0xC0],
    &[0x4C, 0x19, 0xCB],
    &[0x4C, 0x19, 0xD1],
    &[0x4C, 0x19, 0xDA],
];
const ADD32: [&[u8]; 4] = [
    &[0x44, 0x01, 0xC0],
    &[0x44, 0x11, 0xCB],
    &[0x44, 0x11, 0xD1],
    &[0x44, 0x11, 0xDA],
];
const SUB32: [&[u8]; 4] = [
    &[0x44, 0x29, 0xC0],
    &[0x44, 0x19, 0xCB],
    &[0x44, 0x19, 0xD1],
    &[0x44, 0x19, 0xDA],
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Chain {
    Add,
    Sub,
}

/// Expected limbs and CF, ZF, SF, OF of the last limb.
#[derive(PartialEq, Eq, Debug)]
struct Expected {
    limbs: [u64; 4],
    cf: bool,
    zf: bool,
    sf: bool,
    of: bool,
}

fn reference(chain: Chain, bits: u32, a: [u64; 4], b: [u64; 4]) -> Expected {
    let mask = if bits == 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    let modulus = 1u128 << bits;
    let signed = |value: u64| -> i128 {
        let value = i128::from(value & mask);
        if value >= (1i128 << (bits - 1)) {
            value - (1i128 << bits)
        } else {
            value
        }
    };
    let mut limbs = [0; 4];
    let mut carry = 0u128;
    let mut of = false;
    for i in 0..4 {
        let (x, y) = (u128::from(a[i] & mask), u128::from(b[i] & mask));
        let wide = match chain {
            Chain::Add => x + y + carry,
            Chain::Sub => x + modulus - y - carry,
        };
        limbs[i] = (wide % modulus) as u64;
        let exact = match chain {
            Chain::Add => signed(a[i]) + signed(b[i]) + carry as i128,
            Chain::Sub => signed(a[i]) - signed(b[i]) - carry as i128,
        };
        of = exact != signed(limbs[i]);
        carry = match chain {
            Chain::Add => u128::from(wide >= modulus),
            Chain::Sub => u128::from(y + carry > x),
        };
    }
    Expected {
        limbs,
        cf: carry != 0,
        zf: limbs[3] == 0,
        sf: limbs[3] >> (bits - 1) & 1 != 0,
        of,
    }
}

fn program(parts: &[&[u8]], split: bool) -> Vec<u8> {
    let mut code = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        if split && i > 0 {
            code.extend_from_slice(&JMP);
        }
        code.extend_from_slice(part);
    }
    code.push(HLT);
    code
}

fn load(code: &[u8], gpr: [u64; 16], cf: bool) -> (CpuState, GuestMemory) {
    let mut memory = GuestMemory::new();
    memory
        .map_region_with_data(ENTRY, code.len(), MemoryPermissions::READ_EXECUTE, code)
        .unwrap();
    let mut cpu = CpuState::new(ENTRY, 0);
    cpu.gpr = gpr;
    cpu.flags.cf = cf;
    (cpu, memory)
}

/// Run `code` to its HLT in both engines, check they agree on the whole state
/// and that the DBT never left native code, and return the final state.
fn run_both(code: &[u8], gpr: [u64; 16], cf: bool, context: &str) -> CpuState {
    let (cpu, mut memory) = load(code, gpr, cf);
    let interpreted = Interpreter::new(cpu, &mut memory, 64).run();
    let (mut cpu, mut memory) = load(code, gpr, cf);
    let mut engine = DbtEngine::new(16 * 1024).unwrap();
    let translated = engine.run(&mut cpu, &mut memory, 64);
    assert_eq!(interpreted.exit_reason, ExitReason::Halt, "{context}");
    assert_eq!(translated.exit_reason, ExitReason::Halt, "{context}");
    assert_eq!(translated.final_state, interpreted.final_state, "{context}");
    assert_eq!(engine.metrics.interpreter_fallbacks, 0, "{context}");
    interpreted.final_state
}

struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// Limb values at the carry and overflow boundaries of a `bits`-wide limb.
fn edges(bits: u32) -> [u64; 6] {
    let max = if bits == 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    let sign = 1u64 << (bits - 1);
    [0, 1, max - 1, max, sign, sign - 1]
}

fn operand_sets(bits: u32) -> Vec<([u64; 4], [u64; 4])> {
    let edges = edges(bits);
    let mut sets = Vec::new();
    for &x in &edges {
        for &y in &edges {
            sets.push(([x; 4], [y; 4]));
            sets.push(([x, y, x, y], [y, x, y, x]));
        }
    }
    let mut random = Random(0x0195_ADC5_BB00_0001);
    for _ in 0..256 {
        let mut pick = || {
            let raw = random.next();
            if raw & 3 == 0 {
                edges[(raw >> 8) as usize % edges.len()] | raw & !edges[3]
            } else {
                raw
            }
        };
        let a = [pick(), pick(), pick(), pick()];
        let b = [pick(), pick(), pick(), pick()];
        sets.push((a, b));
    }
    sets
}

fn check_chain(chain: Chain, bits: u32, parts: [&[u8]; 4]) {
    for part in &parts[1..] {
        let insn = decode(ENTRY, |address| {
            part.get((address - ENTRY) as usize).copied().ok_or(())
        })
        .unwrap();
        assert!(matches!(insn.op, Op::Adc | Op::Sbb));
        assert!(is_native(&insn), "{:?}", insn.op);
    }
    let mut random = Random(u64::from(bits) << 8 | chain as u64);
    for (a, b) in operand_sets(bits) {
        let expected = reference(chain, bits, a, b);
        for split in [false, true] {
            let code = program(&parts, split);
            let mut gpr: [u64; 16] = std::array::from_fn(|_| random.next());
            for i in 0..4 {
                gpr[A[i]] = a[i];
                gpr[B[i]] = b[i];
            }
            // The leading ADD or SUB must ignore the incoming carry.
            for cf in [false, true] {
                let context = format!("{chain:?}{bits} split={split} cf={cf} a={a:x?} b={b:x?}");
                let state = run_both(&code, gpr, cf, &context);
                let limbs = A.map(|index| state.gpr[index]);
                // A 32-bit destination clears bits 63:32.
                assert_eq!(limbs, expected.limbs, "{context}");
                assert_eq!(B.map(|index| state.gpr[index]), b, "{context}");
                let flags = (
                    state.flags.cf,
                    state.flags.zf,
                    state.flags.sf,
                    state.flags.of,
                );
                let wanted = (expected.cf, expected.zf, expected.sf, expected.of);
                assert_eq!(flags, wanted, "{context}");
            }
        }
    }
}

#[test]
fn reference_matches_native_wide_arithmetic() {
    // 128-bit sanity checks of the reference itself on the low two limbs.
    let cases = [
        (u128::MAX, 1u128),
        (0, 1),
        (1u128 << 64, 1),
        (0x8000_0000_0000_0000_0000_0000_0000_0000, 1),
    ];
    for (x, y) in cases {
        let a = [x as u64, (x >> 64) as u64, 0, 0];
        let b = [y as u64, (y >> 64) as u64, 0, 0];
        let sum = reference(Chain::Add, 64, a, b);
        let expected = x.wrapping_add(y);
        assert_eq!(sum.limbs[0], expected as u64);
        assert_eq!(sum.limbs[1], (expected >> 64) as u64);
        let difference = reference(Chain::Sub, 64, a, b);
        let expected = x.wrapping_sub(y);
        assert_eq!(difference.limbs[0], expected as u64);
        assert_eq!(difference.limbs[1], (expected >> 64) as u64);
        // The upper limbs are zero, so the 256-bit borrow is the 128-bit one.
        assert_eq!(difference.cf, y > x);
    }
}

#[test]
fn add_adc_chains_of_64_bit_limbs_carry_across_blocks() {
    check_chain(Chain::Add, 64, ADD64);
}

#[test]
fn sub_sbb_chains_of_64_bit_limbs_borrow_across_blocks() {
    check_chain(Chain::Sub, 64, SUB64);
}

#[test]
fn add_adc_chains_of_32_bit_limbs_carry_across_blocks() {
    check_chain(Chain::Add, 32, ADD32);
}

#[test]
fn sub_sbb_chains_of_32_bit_limbs_borrow_across_blocks() {
    check_chain(Chain::Sub, 32, SUB32);
}

#[test]
fn split_chains_run_one_block_per_limb() {
    let code = program(&ADD64, true);
    let (mut cpu, mut memory) = load(&code, [0; 16], false);
    let mut engine = DbtEngine::new(16 * 1024).unwrap();
    let result = engine.run(&mut cpu, &mut memory, 64);
    assert_eq!(result.exit_reason, ExitReason::Halt);
    assert_eq!(engine.metrics.blocks_compiled, 4);
    assert_eq!(engine.metrics.interpreter_fallbacks, 0);
}

#[test]
fn immediate_forms_increment_and_decrement_a_128_bit_value() {
    // add rax, 1; adc rbx, 0 and sub rax, 1; sbb rbx, 0.
    let increment: [&[u8]; 2] = [&[0x48, 0x83, 0xC0, 0x01], &[0x48, 0x83, 0xD3, 0x00]];
    let decrement: [&[u8]; 2] = [&[0x48, 0x83, 0xE8, 0x01], &[0x48, 0x83, 0xDB, 0x00]];
    let values = [
        0u128,
        1,
        u64::MAX as u128,
        1u128 << 64,
        u128::MAX,
        u128::MAX - 1,
        1u128 << 127,
        (1u128 << 127) - 1,
    ];
    for value in values {
        for split in [false, true] {
            for (parts, expected, carry, overflow) in [
                (
                    &increment,
                    value.wrapping_add(1),
                    value == u128::MAX,
                    value == (1u128 << 127) - 1,
                ),
                (
                    &decrement,
                    value.wrapping_sub(1),
                    value == 0,
                    value == 1u128 << 127,
                ),
            ] {
                let code = program(parts, split);
                let mut gpr = [0x5A5A_5A5A_5A5A_5A5A; 16];
                gpr[0] = value as u64;
                gpr[3] = (value >> 64) as u64;
                let context = format!("{value:#x} split={split} {:x?}", parts[0]);
                let state = run_both(&code, gpr, true, &context);
                assert_eq!(state.gpr[0], expected as u64, "{context}");
                assert_eq!(state.gpr[3], (expected >> 64) as u64, "{context}");
                assert_eq!(state.flags.cf, carry, "{context}");
                assert_eq!(state.flags.of, overflow, "{context}");
                assert_eq!(state.flags.zf, (expected >> 64) == 0, "{context}");
                assert_eq!(state.flags.sf, expected >> 127 != 0, "{context}");
            }
        }
    }
}

#[test]
fn sbb_of_a_register_with_itself_materializes_the_carry_mask() {
    // sbb esi, esi and sbb rsi, rsi give -CF and borrow exactly when CF was
    // set; adc edi, 0 adds CF to 0x89ABCDEF without a carry out. The 32-bit
    // forms clear bits 63:32.
    // (code, destination register, value from CF, CF out from CF in)
    type Case = (&'static [u8], usize, fn(bool) -> u64, fn(bool) -> bool);
    let cases: [Case; 3] = [
        (
            &[0x19, 0xF6],
            6,
            |cf| if cf { 0xFFFF_FFFF } else { 0 },
            |cf| cf,
        ),
        (
            &[0x48, 0x19, 0xF6],
            6,
            |cf| if cf { u64::MAX } else { 0 },
            |cf| cf,
        ),
        (
            &[0x83, 0xD7, 0x00],
            7,
            |cf| u64::from(cf) + 0x89AB_CDEF,
            |_| false,
        ),
    ];
    for (code, register, value, carry) in cases {
        for cf in [false, true] {
            let mut gpr = [0x0123_4567_89AB_CDEF; 16];
            gpr[6] = 0xDEAD_BEEF_0000_0001;
            let program = [code, &[HLT]].concat();
            let context = format!("{code:x?} cf={cf}");
            let state = run_both(&program, gpr, cf, &context);
            assert_eq!(state.gpr[register], value(cf), "{context}");
            assert_eq!(state.flags.cf, carry(cf), "{context}");
        }
    }
}
