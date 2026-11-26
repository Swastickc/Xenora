// SPDX-License-Identifier: GPL-3.0-or-later

//! The SSE4.2 string compares PCMPESTRI, PCMPESTRM, PCMPISTRI and PCMPISTRM.
//!
//! The steps and their names follow Intel SDM Vol. 2B, 4.1: compare every pair of
//! elements, override pairs with an invalid element, aggregate to IntRes1, apply
//! the polarity to get IntRes2, and produce an index or a mask. Immediate bit 7
//! is reserved and ignored.

use crate::cpu::{CpuState, ExitReason, RFlags, Register};
use crate::memory::GuestMemory;
use crate::x86_decode::Insn;
use crate::x86_simd::{lane, read_rm};
use crate::x86_table::Op;

/// Element width and signedness, immediate bits 1:0.
#[derive(Clone, Copy)]
struct Elements {
    bits: u32,
    signed: bool,
}

impl Elements {
    fn from_immediate(imm: u64) -> Self {
        Self {
            bits: if imm & 1 != 0 { 16 } else { 8 },
            signed: imm & 2 != 0,
        }
    }

    fn count(self) -> u32 {
        128 / self.bits
    }

    fn get(self, value: u128, index: u32) -> i64 {
        let raw = lane(value, self.bits, index) as i64;
        if self.signed && raw >> (self.bits - 1) != 0 {
            raw - (1 << self.bits)
        } else {
            raw
        }
    }

    /// Elements before the first zero.
    fn implicit_length(self, value: u128) -> u32 {
        (0..self.count())
            .find(|index| self.get(value, *index) == 0)
            .unwrap_or(self.count())
    }

    /// The absolute value of a length register, saturated.
    fn explicit_length(self, register: u64, wide: bool) -> u32 {
        let magnitude = if wide {
            (register as i64).unsigned_abs()
        } else {
            u64::from((register as u32 as i32).unsigned_abs())
        };
        magnitude.min(u64::from(self.count())) as u32
    }
}

/// One string: its elements and how many of them are valid.
#[derive(Clone, Copy)]
struct Fragment {
    value: u128,
    length: u32,
}

/// BoolRes for element `i` of the first operand and `j` of the second, with the
/// overrides for invalid elements of the aggregation in `imm`.
fn matches(elements: Elements, imm: u64, a: Fragment, b: Fragment, i: u32, j: u32) -> bool {
    let aggregation = imm >> 2 & 3;
    match (i < a.length, j < b.length) {
        (true, true) => {
            let (x, y) = (elements.get(a.value, i), elements.get(b.value, j));
            match aggregation {
                // Ranges: the even elements are lower bounds, the odd ones upper.
                1 if i.is_multiple_of(2) => y >= x,
                1 => y <= x,
                _ => x == y,
            }
        }
        (false, true) => aggregation == 3,
        (true, false) => false,
        (false, false) => aggregation >= 2,
    }
}

fn aggregate(elements: Elements, imm: u64, a: Fragment, b: Fragment) -> u32 {
    let count = elements.count();
    let hit = |i, j| matches(elements, imm, a, b, i, j);
    let mut result = 0;
    for j in 0..count {
        let set = match imm >> 2 & 3 {
            0 => (0..count).any(|i| hit(i, j)),
            1 => (0..count).step_by(2).any(|i| hit(i, j) && hit(i + 1, j)),
            2 => hit(j, j),
            _ => (0..count - j).all(|i| hit(i, j + i)),
        };
        result |= u32::from(set) << j;
    }
    result
}

pub(crate) fn execute(
    cpu: &mut CpuState,
    memory: &GuestMemory,
    insn: &Insn,
) -> Result<(), ExitReason> {
    let (explicit, index) = match insn.op {
        Op::Pcmpestri => (true, true),
        Op::Pcmpestrm => (true, false),
        Op::Pcmpistri => (false, true),
        _ => (false, false),
    };
    let imm = insn.imm;
    let elements = Elements::from_immediate(imm);
    let count = elements.count();
    let second = read_rm(cpu, memory, insn)?;
    let first = cpu.xmm[usize::from(insn.reg.index)];
    let [a, b] = [(first, Register::Rax), (second, Register::Rdx)].map(|(value, register)| {
        let length = if explicit {
            elements.explicit_length(cpu.gpr[register as usize], insn.size == 8)
        } else {
            elements.implicit_length(value)
        };
        Fragment { value, length }
    });

    let ones = (1u32 << count) - 1;
    // Negative polarity negates every bit, masked negative only those of valid
    // elements of the second operand.
    let negated = match imm >> 4 & 3 {
        1 => ones,
        3 => (1 << b.length) - 1,
        _ => 0,
    };
    let result = aggregate(elements, imm, a, b) ^ negated;

    if index {
        let position = if result == 0 {
            count
        } else if imm & 0x40 != 0 {
            31 - result.leading_zeros()
        } else {
            result.trailing_zeros()
        };
        cpu.gpr[Register::Rcx as usize] = u64::from(position);
    } else if imm & 0x40 != 0 {
        let element = (1u128 << elements.bits) - 1;
        cpu.xmm[0] = (0..count)
            .filter(|at| result >> at & 1 != 0)
            .fold(0, |mask, at| mask | element << (at * elements.bits));
    } else {
        cpu.xmm[0] = u128::from(result);
    }
    cpu.flags = RFlags {
        cf: result != 0,
        zf: b.length < count,
        sf: a.length < count,
        of: result & 1 != 0,
        pf: false,
        af: false,
    };
    Ok(())
}
