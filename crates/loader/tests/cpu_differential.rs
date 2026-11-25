// SPDX-License-Identifier: GPL-3.0-or-later

//! Seeded sequences of original x86-64 instructions for issue #74.
//!
//! Expected states follow the Intel SDM (Vol. 1, §§3.4.1 and 3.4.3,
//! Vol. 2, ADD, SUB, CMP, XOR, MOV, Jcc, CALL, RET, DEC, MOVDQU, PADDB,
//! PSUBW and PXOR, and Vol. 3A, §7.15, Page-Fault Exception). The oracles work from arithmetic operands, branch
//! predicates, array indices and vector lanes. They never decode the emitted
//! bytes or use Xenora's instruction or flag helpers.
//!
//! <https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html>
//!
//! The x86 bytes run as guest code through a synthetic memory map and end at
//! Xenora's HLT sentinel. To replay a failure, use the printed test name and
//! XENORA_CPU_DIFFERENTIAL_CASE index. For example:
//!
//! ```sh
//! XENORA_CPU_DIFFERENTIAL_CASE=0 cargo test --locked -p xenora-loader \
//!   --test cpu_differential seeded_register_sequences -- --exact
//! ```
//!
//! The seed and bytes are printed for reproducing the program outside this
//! harness as well.

use xenora_loader::cpu::{CpuState, ExecutionResult, ExitReason, Interpreter, RFlags, Register};
use xenora_loader::dbt::DbtEngine;
use xenora_loader::memory::{GuestMemory, MemoryError, MemoryPermissions};

const CODE: u64 = 0x40_0000;
const DATA: u64 = 0x50_0000;
const DATA_LEN: usize = 192;
const STACK_TOP: u64 = DATA + DATA_LEN as u64;
const BUDGET: usize = 128;
const TEST_SEED: u64 = 0x74_da7a_c0de_ba5e;

// SplitMix64 with a fresh stream per case. Changing the corpus order does not
// affect replay of an existing index.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut word = self.0;
        word = (word ^ (word >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        word = (word ^ (word >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        word ^ (word >> 31)
    }

    fn below(&mut self, end: usize) -> usize {
        (self.next() % end as u64) as usize
    }
}

#[derive(Clone)]
struct State {
    cpu: CpuState,
    data: [u8; DATA_LEN],
    af_defined: bool,
}

impl State {
    fn seeded(random: &mut Random) -> Self {
        let mut cpu = CpuState::new(CODE, STACK_TOP);
        cpu.gpr = std::array::from_fn(|_| random.next());
        cpu.set(Register::Rsp, STACK_TOP);
        cpu.xmm =
            std::array::from_fn(|_| (u128::from(random.next()) << 64) | u128::from(random.next()));
        let flags = random.next();
        cpu.flags = RFlags {
            cf: flags & 1 != 0,
            pf: flags & 2 != 0,
            af: flags & 4 != 0,
            zf: flags & 8 != 0,
            sf: flags & 16 != 0,
            of: flags & 32 != 0,
        };
        cpu.df = flags & 64 != 0;
        cpu.fs_base = DATA;
        cpu.gs_base = DATA + 64;
        Self {
            cpu,
            data: std::array::from_fn(|_| random.next() as u8),
            af_defined: true,
        }
    }

    fn checkpoint(&mut self, trace: &mut Vec<Self>, offset: usize) {
        self.cpu.rip = CODE + offset as u64;
        trace.push(self.clone());
    }

    fn word(&self, offset: usize) -> u64 {
        u64::from_le_bytes(self.data[offset..offset + 8].try_into().unwrap())
    }

    fn put_word(&mut self, offset: usize, value: u64) {
        self.data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
}

struct Case {
    code: Vec<u8>,
    initial: State,
    // State after each attempted instruction, including the terminal instruction.
    trace: Vec<State>,
    native_only: bool,
    exit: ExitReason,
}

impl Case {
    fn memory(&self) -> GuestMemory {
        let mut memory = GuestMemory::new();
        memory
            .map_region_with_data(
                CODE,
                self.code.len(),
                MemoryPermissions::READ_EXECUTE,
                &self.code,
            )
            .unwrap();
        memory
            .map_region_with_data(
                DATA,
                DATA_LEN,
                MemoryPermissions::READ_WRITE,
                &self.initial.data,
            )
            .unwrap();
        memory
    }

    fn verify(&self, id: &str) {
        let steps = self.trace.len();
        assert!(steps < BUDGET);
        let compare = |engine: &str,
                       result: ExecutionResult,
                       memory: &GuestMemory,
                       budget: usize,
                       offset: usize| {
            let steps = (steps - offset).min(budget);
            let expected = &self.trace[offset + steps - 1];
            let label = || {
                let start_rip = if offset == 0 {
                    self.initial.cpu.rip
                } else {
                    self.trace[offset - 1].cpu.rip
                };
                format!(
                    "{id} {engine} from {start_rip:#x}, after {} guest steps, bytes {:02x?}",
                    offset + steps,
                    self.code
                )
            };
            let finished = offset + steps == self.trace.len();
            let exit = if finished {
                self.exit
            } else {
                ExitReason::StepLimitExceeded
            };
            let retired = steps - usize::from(finished && self.exit != ExitReason::Halt);
            assert_eq!(result.exit_reason, exit, "{}", label());
            assert_eq!(result.instructions_executed, retired, "{}", label());
            let mut actual = result.final_state;
            if !expected.af_defined {
                actual.flags.af = expected.cpu.flags.af;
            }
            assert_eq!(actual, expected.cpu, "{}", label());
            let mut data = [0; DATA_LEN];
            memory.read_bytes(DATA, &mut data).unwrap();
            if let Some(index) = data.iter().zip(expected.data).position(|(&a, b)| a != b) {
                panic!(
                    "{}: guest byte at {:#x}: got {:#04x}, expected {:#04x}",
                    label(),
                    DATA + index as u64,
                    data[index],
                    expected.data[index]
                );
            }
        };

        let mut stepped = DbtEngine::new(128 * 1024).unwrap();
        let mut interpreted_memory = self.memory();
        let mut translated_memory = self.memory();
        let mut interpreted_cpu = self.initial.cpu.clone();
        let mut translated_cpu = self.initial.cpu.clone();
        // Check the first divergent retired instruction before testing full
        // blocks. Each engine continues from its own state at every step.
        for offset in 0..steps {
            let result = Interpreter::new(interpreted_cpu, &mut interpreted_memory, 1).run();
            interpreted_cpu = result.final_state.clone();
            compare(
                "interpreter (1 step)",
                result,
                &interpreted_memory,
                1,
                offset,
            );

            let result = stepped.run(&mut translated_cpu, &mut translated_memory, 1);
            compare("DBT (1 step)", result, &translated_memory, 1, offset);
        }

        // Exercise native block lowering and then reuse those blocks with a
        // fresh copy of the guest input and memory at the same addresses.
        let mut dbt = DbtEngine::new(128 * 1024).unwrap();
        let mut memory = self.memory();
        let result = Interpreter::new(self.initial.cpu.clone(), &mut memory, BUDGET).run();
        compare("interpreter", result, &memory, BUDGET, 0);
        let mut run_full = |engine| {
            let mut memory = self.memory();
            let result = dbt.run(&mut self.initial.cpu.clone(), &mut memory, BUDGET);
            compare(engine, result, &memory, BUDGET, 0);
            dbt.metrics
        };
        let cold = run_full("DBT (cold)");
        let cached = run_full("DBT (cached)");
        assert!(cold.blocks_compiled > 0, "{id}");
        assert_eq!(cold.blocks_compiled, cached.blocks_compiled, "{id}");
        assert!(cached.cache_hits > cold.cache_hits, "{id}");
        assert!(cached.dbt_dispatches > cold.dbt_dispatches, "{id}");

        // These budgets cut the longer programs around a 16-instruction
        // native block boundary, including a resume from the truncated block.
        for budget in [15, 16, 17].into_iter().filter(|&budget| budget < steps) {
            let mut memory = self.memory();
            let mut cpu = self.initial.cpu.clone();
            let result = dbt.run(&mut cpu, &mut memory, budget);
            compare("DBT (limited)", result, &memory, budget, 0);
            let result = dbt.run(&mut cpu, &mut memory, BUDGET);
            compare("DBT (resumed)", result, &memory, BUDGET, budget);
        }

        if self.native_only {
            assert_eq!(dbt.metrics.interpreter_fallbacks, 0, "{id}");
            assert_eq!(stepped.metrics.interpreter_fallbacks, 0, "{id}");
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Alu {
    Add,
    Sub,
    Compare,
    Xor,
    Move,
}

impl Alu {
    const OPS: [Self; 5] = [Self::Add, Self::Sub, Self::Compare, Self::Xor, Self::Move];

    // SDM Vol. 2, §2.1 and the r/m,reg forms for each instruction. A bare
    // REX prefix selects SPL/BPL/SIL/DIL instead of the high byte registers.
    fn encode(self, code: &mut Vec<u8>, bits: u32, dst: usize, src: usize) {
        if bits == 16 {
            code.push(0x66);
        }
        let rex = 0x40 | (u8::from(bits == 64) << 3) | ((src as u8 >> 3) << 2) | (dst as u8 >> 3);
        if rex != 0x40 || (bits == 8 && (src >= 4 || dst >= 4)) {
            code.push(rex);
        }
        let byte_opcode = match self {
            Self::Add => 0x00,
            Self::Sub => 0x28,
            Self::Compare => 0x38,
            Self::Xor => 0x30,
            Self::Move => 0x88,
        };
        code.extend([
            byte_opcode | u8::from(bits != 8),
            0xc0 | ((src as u8 & 7) << 3) | (dst as u8 & 7),
        ]);
    }

    fn expect(self, state: &mut State, bits: u32, dst: usize, src: usize) {
        let mask = u64::MAX >> (64 - bits);
        let left = state.cpu.gpr[dst] & mask;
        let right = state.cpu.gpr[src] & mask;
        let result = match self {
            Self::Move => right,
            Self::Xor => {
                let result = left ^ right;
                state.cpu.flags = result_flags(result, bits);
                state.af_defined = false;
                result
            }
            Self::Add | Self::Compare | Self::Sub => {
                let (result, flags) = arithmetic(left, right, bits, matches!(self, Self::Add));
                state.cpu.flags = flags;
                state.af_defined = true;
                result
            }
        };
        if !matches!(self, Self::Compare) {
            state.cpu.gpr[dst] = if bits >= 32 {
                result
            } else {
                (state.cpu.gpr[dst] & !mask) | result
            };
        }
    }
}

fn result_flags(result: u64, bits: u32) -> RFlags {
    RFlags {
        zf: result == 0,
        sf: result >> (bits - 1) == 1,
        pf: (result as u8).count_ones().is_multiple_of(2),
        ..RFlags::default()
    }
}

// Carry, auxiliary carry and signed overflow are derived by separate
// widened calculations. The exact signed result determines overflow.
fn arithmetic(left: u64, right: u64, bits: u32, add: bool) -> (u64, RFlags) {
    let modulus = 1i128 << bits;
    let signed = |word: u64| {
        let word = i128::from(word);
        if word >= modulus / 2 {
            word - modulus
        } else {
            word
        }
    };
    let operation = |a: i128, b: i128| if add { a + b } else { a - b };
    let wide = operation(i128::from(left), i128::from(right));
    let result = wide.rem_euclid(modulus) as u64;
    let mut flags = result_flags(result, bits);
    flags.cf = !(0..modulus).contains(&wide);
    let nibble = operation(i128::from(left & 15), i128::from(right & 15));
    flags.af = !(0..16).contains(&nibble);
    let signed_result = operation(signed(left), signed(right));
    flags.of = !(-modulus / 2..modulus / 2).contains(&signed_result);
    (result, flags)
}

fn seeded_cases(
    test: &str,
    family: u64,
    count: usize,
    make_case: impl Fn(usize, &mut Random) -> Case,
) {
    let selected = std::env::var("XENORA_CPU_DIFFERENTIAL_CASE")
        .ok()
        .map(|text| {
            let index = text
                .parse::<usize>()
                .expect("XENORA_CPU_DIFFERENTIAL_CASE must be a decimal case index");
            assert!(index < count, "{test} has case indices 0..{count}");
            index
        });
    for index in selected.map_or(0..count, |index| index..index + 1) {
        let seed = TEST_SEED ^ (family << 48) ^ index as u64;
        let case = make_case(index, &mut Random(seed));
        case.verify(&format!(
            "{test}: XENORA_CPU_DIFFERENTIAL_CASE={index} (seed {seed:#018x})"
        ));
    }
}

#[test]
fn seeded_register_sequences() {
    const WIDTHS: [u32; 4] = [8, 16, 32, 64];
    seeded_cases("seeded_register_sequences", 1, 240, |index, random| {
        let mixed_widths = index >= 160;
        let bits = WIDTHS[index % 4];
        let edges = [
            0,
            1,
            15,
            16,
            (1u64 << (bits - 1)) - 1,
            1 << (bits - 1),
            u64::MAX,
        ];
        let mut initial = State::seeded(random);
        // Keep random high bits when the operand is narrower than the GPR.
        let mask = u64::MAX >> (64 - bits);
        for reg in [0, 1, 2, 3, 8, 9, 10, 11] {
            let edge = edges[random.below(edges.len())];
            initial.cpu.gpr[reg] = (initial.cpu.gpr[reg] & !mask) | (edge & mask);
        }
        let mut state = initial.clone();
        let mut code = Vec::new();
        let mut trace = Vec::new();
        for step in 0..24 {
            let bits = if mixed_widths {
                WIDTHS[(index + step) % WIDTHS.len()]
            } else {
                bits
            };
            let op = Alu::OPS[random.below(Alu::OPS.len())];
            let dst = random.below(16);
            let src = random.below(16);
            op.encode(&mut code, bits, dst, src);
            op.expect(&mut state, bits, dst, src);
            state.checkpoint(&mut trace, code.len());
        }
        code.push(0xf4);
        state.checkpoint(&mut trace, code.len());
        Case {
            code,
            initial,
            trace,
            native_only: !mixed_widths && bits >= 32,
            exit: ExitReason::Halt,
        }
    });
}

fn branch_predicate(condition: usize, flags: RFlags) -> bool {
    let base = [
        flags.of,
        flags.cf,
        flags.zf,
        flags.cf || flags.zf,
        flags.sf,
        flags.pf,
        flags.sf != flags.of,
        flags.zf || flags.sf != flags.of,
    ][condition / 2];
    base ^ !condition.is_multiple_of(2)
}

#[test]
fn seeded_compare_branch_pairs() {
    seeded_cases("seeded_compare_branch_pairs", 2, 128, |index, random| {
        let condition = index % 16;
        let taken = index / 16 % 2 != 0;
        let bits = [32, 64][index / 32 % 2];
        let near = index >= 64;
        let sign = 1u64 << (bits - 1);
        let boundary = [0, 1, 2, sign - 1, sign, sign | 1, (sign - 1) | sign];
        let pairs = boundary
            .into_iter()
            .flat_map(|left| boundary.map(|right| (left, right)));
        let eligible: Vec<_> = pairs
            .filter(|&(left, right)| {
                branch_predicate(condition, arithmetic(left, right, bits, false).1) == taken
            })
            .collect();
        let (left, right) = eligible[random.below(eligible.len())];
        let mut initial = State::seeded(random);
        let mask = u64::MAX >> (64 - bits);
        initial.cpu.gpr[0] = (initial.cpu.gpr[0] & !mask) | left;
        initial.cpu.gpr[8] = (initial.cpu.gpr[8] & !mask) | right;
        let mut state = initial.clone();
        let mut code = Vec::new();
        let mut trace = Vec::new();
        Alu::Compare.encode(&mut code, bits, 0, 8);
        Alu::Compare.expect(&mut state, bits, 0, 8);
        let branch = code.len();
        state.checkpoint(&mut trace, branch);
        if near {
            code.extend([0x0f, 0x80 + condition as u8, 3, 0, 0, 0]);
        } else {
            code.extend([0x70 + condition as u8, 3]);
        }
        // The two arms set distinct low bytes of RDX, then stop.
        let fallthrough = code.len();
        code.extend([0xb2, 0x31, 0xf4, 0xb2, 0x73, 0xf4]);
        let arm = fallthrough + if taken { 3 } else { 0 };
        state.checkpoint(&mut trace, arm);
        state.cpu.gpr[2] = (state.cpu.gpr[2] & !255) | if taken { 0x73 } else { 0x31 };
        state.checkpoint(&mut trace, arm + 2);
        state.checkpoint(&mut trace, arm + 3);
        Case {
            code,
            initial,
            trace,
            native_only: true,
            exit: ExitReason::Halt,
        }
    });
}

#[test]
fn seeded_memory_loop_with_calls() {
    seeded_cases("seeded_memory_loop_with_calls", 3, 64, |_, random| {
        let count = random.below(8) + 1;
        let input = random.below(8);
        let output = 80 + random.below(8);
        let mut initial = State::seeded(random);
        initial.cpu.set(Register::Rcx, count as u64);
        initial.cpu.set(Register::Rsi, DATA + input as u64);
        initial.cpu.set(Register::Rdi, DATA + output as u64);
        let code = vec![
            0xe8, 10, 0, 0, 0, //  0: call sum
            0x48, 0x89, 0x44, 0xcf, 0xf8, //  5: mov [rdi + rcx*8 - 8], rax
            0xff, 0xc9, // 10: dec ecx
            0x75, 0xf2, // 12: jnz start
            0xf4, // 14: hlt
            0x48, 0x03, 0x44, 0xce, 0xf8, // 15: sum: add rax, [rsi + rcx*8 - 8]
            0xc3, // 20: ret
        ];
        let mut state = initial.clone();
        let mut trace = Vec::new();
        let mut sum = state.cpu.get(Register::Rax);
        for remaining in (1..=count).rev() {
            state.put_word(DATA_LEN - 8, CODE + 5);
            state.cpu.set(Register::Rsp, STACK_TOP - 8);
            state.checkpoint(&mut trace, 15);

            let index = (remaining - 1) * 8;
            let (next, flags) = arithmetic(sum, initial.word(input + index), 64, true);
            sum = next;
            state.cpu.set(Register::Rax, sum);
            state.cpu.flags = flags;
            state.checkpoint(&mut trace, 20);

            state.cpu.set(Register::Rsp, STACK_TOP);
            state.checkpoint(&mut trace, 5);
            state.put_word(output + index, sum);
            state.checkpoint(&mut trace, 10);

            let (counter, mut flags) = arithmetic(remaining as u64, 1, 32, false);
            flags.cf = state.cpu.flags.cf; // DEC leaves carry intact.
            state.cpu.set(Register::Rcx, counter);
            state.cpu.flags = flags;
            state.checkpoint(&mut trace, 12);
            state.checkpoint(&mut trace, if counter == 0 { 14 } else { 0 });
        }
        state.checkpoint(&mut trace, 15);
        Case {
            code,
            initial,
            trace,
            native_only: true,
            exit: ExitReason::Halt,
        }
    });
}

#[test]
fn seeded_vector_load_arithmetic_store() {
    seeded_cases("seeded_vector_load_arithmetic_store", 4, 64, |_, random| {
        let mut initial = State::seeded(random);
        let input = random.below(16);
        let output = 64 + random.below(16);
        initial.cpu.set(Register::Rsi, DATA + input as u64);
        initial.cpu.set(Register::Rdi, DATA + output as u64);
        let code = vec![
            0xf3, 0x44, 0x0f, 0x6f, 0x0e, //  0: movdqu xmm9, [rsi]
            0x66, 0x44, 0x0f, 0xfc, 0xca, //  5: paddb xmm9, xmm2
            0x66, 0x45, 0x0f, 0xf9, 0xcf, // 10: psubw xmm9, xmm15
            0x66, 0x44, 0x0f, 0xef, 0xc8, // 15: pxor xmm9, xmm0
            0xf3, 0x44, 0x0f, 0x7f, 0x0f, // 20: movdqu [rdi], xmm9
            0xf4,
        ];
        let mut state = initial.clone();
        let mut trace = Vec::new();
        let mut lanes: [u8; 16] = initial.data[input..input + 16].try_into().unwrap();
        state.cpu.xmm[9] = u128::from_le_bytes(lanes);
        state.checkpoint(&mut trace, 5);

        for (lane, addend) in lanes.iter_mut().zip(initial.cpu.xmm[2].to_le_bytes()) {
            *lane = lane.wrapping_add(addend);
        }
        state.cpu.xmm[9] = u128::from_le_bytes(lanes);
        state.checkpoint(&mut trace, 10);

        let subtrahend = initial.cpu.xmm[15].to_le_bytes();
        let (words, _) = lanes.as_chunks_mut::<2>();
        let (subtrahends, _) = subtrahend.as_chunks::<2>();
        for (word, &rhs) in words.iter_mut().zip(subtrahends) {
            let left = u16::from_le_bytes(*word);
            let right = u16::from_le_bytes(rhs);
            *word = left.wrapping_sub(right).to_le_bytes();
        }
        state.cpu.xmm[9] = u128::from_le_bytes(lanes);
        state.checkpoint(&mut trace, 15);

        state.cpu.xmm[9] ^= initial.cpu.xmm[0];
        state.checkpoint(&mut trace, 20);
        state.data[output..output + 16].copy_from_slice(&state.cpu.xmm[9].to_le_bytes());
        state.checkpoint(&mut trace, 25);
        state.checkpoint(&mut trace, 26);
        Case {
            code,
            initial,
            trace,
            native_only: true,
            exit: ExitReason::Halt,
        }
    });
}

#[test]
fn seeded_precise_memory_faults() {
    seeded_cases("seeded_precise_memory_faults", 5, 28, |index, random| {
        // Both branch outcomes and both operand directions at each of the
        // seven possible byte splits of an eight-byte operand.
        let store = index.is_multiple_of(2);
        let taken = (index / 2).is_multiple_of(2);
        let split = index / 4 + 1;
        let mut initial = State::seeded(random);
        let output = random.below(32);
        let value = initial.cpu.get(Register::Rax);
        let rhs = if taken { value } else { value ^ (1 << 63) };
        initial.cpu.set(Register::R8, rhs);
        initial.cpu.set(Register::Rdi, DATA + output as u64);
        initial.cpu.set(Register::Rsi, STACK_TOP - split as u64);
        let code = vec![
            0x48,
            0x89,
            0x07, //  0: mov [rdi], rax
            0x4c,
            0x39,
            0xc0, //  3: cmp rax, r8
            0x74,
            0x03, //  6: je faulting_move
            0x4d,
            0x89,
            0xd1, //  8: mov r9, r10
            0x48,
            if store { 0x89 } else { 0x8b },
            0x06, // 11: mov [rsi], rax or mov rax, [rsi]
            0xf4,
        ];
        let mut state = initial.clone();
        let mut trace = Vec::new();
        state.put_word(output, value);
        state.checkpoint(&mut trace, 3);
        state.cpu.flags = arithmetic(value, rhs, 64, false).1;
        state.checkpoint(&mut trace, 6);
        if taken {
            state.checkpoint(&mut trace, 11);
        } else {
            state.checkpoint(&mut trace, 8);
            state.cpu.set(Register::R9, initial.cpu.get(Register::R10));
            state.checkpoint(&mut trace, 11);
        }
        // The faulting instruction leaves RIP, registers and all mapped bytes
        // intact. The preceding store and optional register move stay retired.
        state.checkpoint(&mut trace, 11);
        Case {
            code,
            initial,
            trace,
            native_only: true,
            exit: ExitReason::MemoryFault(MemoryError::UnmappedAddress(STACK_TOP)),
        }
    });
}
