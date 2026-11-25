// SPDX-License-Identifier: GPL-3.0-or-later

//! Semantic primitives for the SSE and SSE2 moves and packed integer forms.
//!
//! Lane 0 is the least significant. Every instruction is all-or-nothing and none
//! changes RFLAGS. Floating-point arithmetic is in [`crate::x86_float`].

use crate::cpu::{CpuState, ExitReason, Mxcsr};
use crate::memory::GuestMemory;
use crate::x86_decode::{Encoding, Insn, RegOperand, RmOperand};
use crate::x86_exec::{
    address, canonical_access, read_mem, read_reg, rm_operand, write_mem, write_reg,
};
use crate::x86_float;
use crate::x86_table::{FORMS, Op};

fn lane_mask(bits: u32) -> u128 {
    (1 << bits) - 1
}

pub(crate) fn lane(value: u128, bits: u32, index: u32) -> u64 {
    ((value >> (bits * index)) & lane_mask(bits)) as u64
}

/// Signed value of a lane of `bits`.
fn signed(value: u64, bits: u32) -> i64 {
    (value << (64 - bits)) as i64 >> (64 - bits)
}

/// Build a vector from a function of the lane index.
fn build(bits: u32, f: impl Fn(u32) -> u64) -> u128 {
    (0..128 / bits).fold(0, |out, index| {
        out | (u128::from(f(index)) & lane_mask(bits)) << (bits * index)
    })
}

fn map(a: u128, b: u128, bits: u32, f: impl Fn(u64, u64) -> u64) -> u128 {
    build(bits, |index| f(lane(a, bits, index), lane(b, bits, index)))
}

fn map_signed(a: u128, b: u128, bits: u32, f: impl Fn(i64, i64) -> i64) -> u128 {
    map(a, b, bits, |x, y| {
        f(signed(x, bits), signed(y, bits)) as u64
    })
}

fn all_ones(condition: bool) -> u64 {
    if condition { u64::MAX } else { 0 }
}

/// Clamp to the signed range of `bits`.
fn saturate(value: i64, bits: u32) -> i64 {
    let limit = 1i64 << (bits - 1);
    value.clamp(-limit, limit - 1)
}

/// Clamp to the unsigned range of `bits`.
fn saturate_unsigned(value: i64, bits: u32) -> i64 {
    value.clamp(0, (1 << bits) - 1)
}

/// Narrow the lanes of `a` then `b`, each `bits` wide, to half width.
fn pack(a: u128, b: u128, bits: u32, narrow: impl Fn(i64, u32) -> i64) -> u128 {
    let count = 128 / bits;
    build(bits / 2, |index| {
        let source = if index < count { a } else { b };
        narrow(signed(lane(source, bits, index % count), bits), bits / 2) as u64
    })
}

/// Interleave the low or high halves of `a` and `b`.
fn unpack(a: u128, b: u128, bits: u32, high: bool) -> u128 {
    let start = if high { 64 / bits } else { 0 };
    build(bits, |index| {
        let source = if index % 2 == 0 { a } else { b };
        lane(source, bits, start + index / 2)
    })
}

/// Select lanes of `bits` by two-bit fields of `imm`, reading `a` for the low
/// half of the result and `b` for the high half.
fn shuffle_32(a: u128, b: u128, imm: u64) -> u128 {
    build(32, |index| {
        let source = if index < 2 { a } else { b };
        lane(source, 32, (imm >> (2 * index) & 3) as u32)
    })
}

/// Shuffle the four words starting at word `start`, keeping the other four.
fn shuffle_16(value: u128, imm: u64, start: u32) -> u128 {
    build(16, |index| {
        if (start..start + 4).contains(&index) {
            lane(value, 16, start + (imm >> (2 * (index - start)) & 3) as u32)
        } else {
            lane(value, 16, index)
        }
    })
}

/// Logical or arithmetic shift of every lane; counts past the width give zero
/// or, for arithmetic shifts, copies of the sign.
fn shift(op: Op, value: u128, count: u64) -> u128 {
    let (bits, kind) = match op {
        Op::Psllw => (16, 0),
        Op::Pslld => (32, 0),
        Op::Psllq => (64, 0),
        Op::Psrlw => (16, 1),
        Op::Psrld => (32, 1),
        Op::Psrlq => (64, 1),
        Op::Psraw => (16, 2),
        _ => (32, 2),
    };
    let over = count >= u64::from(bits);
    map(value, 0, bits, |x, _| match kind {
        0 if !over => x << count,
        1 if !over => x >> count,
        2 => (signed(x, bits) >> count.min(u64::from(bits) - 1)) as u64,
        _ => 0,
    })
}

fn shift_bytes(value: u128, count: u64, left: bool) -> u128 {
    if count > 15 {
        0
    } else if left {
        value << (8 * count)
    } else {
        value >> (8 * count)
    }
}

/// Result of a two-source packed operation.
fn packed(op: Op, a: u128, b: u128) -> u128 {
    match op {
        Op::Pand => a & b,
        Op::Pandn => !a & b,
        Op::Por => a | b,
        Op::Pxor => a ^ b,
        Op::Paddb => map(a, b, 8, u64::wrapping_add),
        Op::Paddw => map(a, b, 16, u64::wrapping_add),
        Op::Paddd => map(a, b, 32, u64::wrapping_add),
        Op::Paddq => map(a, b, 64, u64::wrapping_add),
        Op::Psubb => map(a, b, 8, u64::wrapping_sub),
        Op::Psubw => map(a, b, 16, u64::wrapping_sub),
        Op::Psubd => map(a, b, 32, u64::wrapping_sub),
        Op::Psubq => map(a, b, 64, u64::wrapping_sub),
        Op::Paddsb => map_signed(a, b, 8, |x, y| saturate(x + y, 8)),
        Op::Paddsw => map_signed(a, b, 16, |x, y| saturate(x + y, 16)),
        Op::Psubsb => map_signed(a, b, 8, |x, y| saturate(x - y, 8)),
        Op::Psubsw => map_signed(a, b, 16, |x, y| saturate(x - y, 16)),
        Op::Paddusb => map(a, b, 8, |x, y| (x + y).min(0xFF)),
        Op::Paddusw => map(a, b, 16, |x, y| (x + y).min(0xFFFF)),
        Op::Psubusb => map(a, b, 8, u64::saturating_sub),
        Op::Psubusw => map(a, b, 16, u64::saturating_sub),
        Op::Pcmpeqb => map(a, b, 8, |x, y| all_ones(x == y)),
        Op::Pcmpeqw => map(a, b, 16, |x, y| all_ones(x == y)),
        Op::Pcmpeqd => map(a, b, 32, |x, y| all_ones(x == y)),
        Op::Pcmpgtb => map_signed(a, b, 8, |x, y| all_ones(x > y) as i64),
        Op::Pcmpgtw => map_signed(a, b, 16, |x, y| all_ones(x > y) as i64),
        Op::Pcmpgtd => map_signed(a, b, 32, |x, y| all_ones(x > y) as i64),
        Op::Pminub => map(a, b, 8, u64::min),
        Op::Pmaxub => map(a, b, 8, u64::max),
        Op::Pminsw => map_signed(a, b, 16, i64::min),
        Op::Pmaxsw => map_signed(a, b, 16, i64::max),
        Op::Pavgb => map(a, b, 8, |x, y| (x + y + 1) >> 1),
        Op::Pavgw => map(a, b, 16, |x, y| (x + y + 1) >> 1),
        Op::Pmullw => map(a, b, 16, u64::wrapping_mul),
        Op::Pmulhw => map_signed(a, b, 16, |x, y| (x * y) >> 16),
        Op::Pmulhuw => map(a, b, 16, |x, y| (x * y) >> 16),
        Op::Pmuludq => map(a, b, 64, |x, y| (x & 0xFFFF_FFFF) * (y & 0xFFFF_FFFF)),
        Op::Pmaddwd => build(32, |index| {
            let product = |at| signed(lane(a, 16, at), 16) * signed(lane(b, 16, at), 16);
            (product(2 * index) + product(2 * index + 1)) as u64
        }),
        Op::Psadbw => build(64, |half| {
            (0..8)
                .map(|at| lane(a, 8, 8 * half + at).abs_diff(lane(b, 8, 8 * half + at)))
                .sum()
        }),
        Op::Packsswb => pack(a, b, 16, saturate),
        Op::Packssdw => pack(a, b, 32, saturate),
        Op::Packuswb => pack(a, b, 16, saturate_unsigned),
        Op::Punpcklbw => unpack(a, b, 8, false),
        Op::Punpcklwd => unpack(a, b, 16, false),
        Op::Punpckldq => unpack(a, b, 32, false),
        Op::Punpcklqdq => unpack(a, b, 64, false),
        Op::Punpckhbw => unpack(a, b, 8, true),
        Op::Punpckhwd => unpack(a, b, 16, true),
        Op::Punpckhdq => unpack(a, b, 32, true),
        Op::Punpckhqdq => unpack(a, b, 64, true),
        // Vector counts are the low 64 bits of the source.
        _ => shift(op, a, b as u64),
    }
}

fn low_mask(size: u8) -> u128 {
    if size == 16 {
        u128::MAX
    } else {
        (1 << (8 * u32::from(size))) - 1
    }
}

/// Address of the memory operand, checked for 16-byte alignment where required.
fn aligned_address(cpu: &CpuState, insn: &Insn) -> Result<Option<u64>, ExitReason> {
    let Some(operand) = insn.memory() else {
        return Ok(None);
    };
    let address = address(cpu, &operand);
    let unaligned = FORMS[usize::from(insn.form)].unaligned;
    if insn.rm_size == 16 && !unaligned && !address.is_multiple_of(16) {
        return Err(ExitReason::GeneralProtection);
    }
    Ok(Some(address))
}

fn read_vector(memory: &GuestMemory, address: u64, size: u8) -> Result<u128, ExitReason> {
    if size < 16 {
        return read_mem(memory, address, size).map(u128::from);
    }
    if !canonical_access(address, size) {
        return Err(ExitReason::GeneralProtection);
    }
    let mut bytes = [0; 16];
    memory
        .read_bytes(address, &mut bytes)
        .map_err(ExitReason::MemoryFault)?;
    Ok(u128::from_le_bytes(bytes))
}

fn write_vector(
    memory: &mut GuestMemory,
    address: u64,
    size: u8,
    value: u128,
) -> Result<(), ExitReason> {
    if size < 16 {
        return write_mem(memory, address, size, value as u64);
    }
    if !canonical_access(address, size) {
        return Err(ExitReason::GeneralProtection);
    }
    memory
        .write_bytes(address, &value.to_le_bytes())
        .map_err(ExitReason::MemoryFault)
}

fn xmm(reg: RegOperand) -> usize {
    usize::from(reg.index)
}

/// Full XMM register, or zero-extended memory, named by ModRM `rm`.
pub(crate) fn read_rm(
    cpu: &CpuState,
    memory: &GuestMemory,
    insn: &Insn,
) -> Result<u128, ExitReason> {
    match (aligned_address(cpu, insn)?, rm_operand(insn)?) {
        (Some(address), _) => read_vector(memory, address, insn.rm_size),
        (None, RmOperand::Reg(reg)) => Ok(cpu.xmm[xmm(reg)]),
        (None, RmOperand::Mem(_)) => Err(ExitReason::IllegalInstruction(0)),
    }
}

/// Execute a vector instruction. RIP is advanced by the caller.
pub(crate) fn execute(
    cpu: &mut CpuState,
    memory: &mut GuestMemory,
    insn: &Insn,
) -> Result<(), ExitReason> {
    if x86_float::execute(cpu, memory, insn)? {
        return Ok(());
    }
    let reg = xmm(insn.reg);
    let in_memory = insn.memory().is_some();
    match (insn.op, insn.encoding) {
        (Op::Ldmxcsr, _) => {
            let value = read_rm(cpu, memory, insn)? as u32;
            if value & !Mxcsr::WRITABLE != 0 {
                return Err(ExitReason::GeneralProtection);
            }
            cpu.mxcsr = Mxcsr(value);
        }
        (Op::Stmxcsr, _) => {
            if let Some(address) = aligned_address(cpu, insn)? {
                write_mem(memory, address, 4, u64::from(cpu.mxcsr.0))?;
            }
        }
        (Op::Movd, Encoding::XmmGpr) => {
            let value = match rm_operand(insn)? {
                RmOperand::Reg(source) => read_reg(cpu, source, insn.size),
                RmOperand::Mem(operand) => read_mem(memory, address(cpu, &operand), insn.size)?,
            };
            cpu.xmm[reg] = u128::from(value);
        }
        (Op::Movd, _) => {
            let value = cpu.xmm[reg] as u64;
            match rm_operand(insn)? {
                RmOperand::Reg(target) => write_reg(cpu, target, insn.size, value),
                RmOperand::Mem(operand) => {
                    write_mem(memory, address(cpu, &operand), insn.size, value)?;
                }
            }
        }
        (Op::Pinsrw, _) => {
            let value = match rm_operand(insn)? {
                RmOperand::Reg(source) => read_reg(cpu, source, 2),
                RmOperand::Mem(operand) => read_mem(memory, address(cpu, &operand), 2)?,
            };
            let shift = 16 * (insn.imm & 7);
            cpu.xmm[reg] = cpu.xmm[reg] & !(0xFFFF << shift) | u128::from(value) << shift;
        }
        (Op::Pextrw, _) => {
            let value = lane(read_rm(cpu, memory, insn)?, 16, (insn.imm & 7) as u32);
            write_reg(cpu, insn.reg, insn.size, value);
        }
        (Op::Movmskps | Op::Movmskpd | Op::Pmovmskb, _) => {
            let bits = match insn.op {
                Op::Movmskps => 32,
                Op::Movmskpd => 64,
                _ => 8,
            };
            let value = read_rm(cpu, memory, insn)?;
            let signs = (0..128 / bits).fold(0, |out, index| {
                out | (lane(value, bits, index) >> (bits - 1)) << index
            });
            write_reg(cpu, insn.reg, insn.size, signs);
        }
        // MOVHLPS and MOVLHPS are the register forms of these opcodes.
        (Op::Movlps, Encoding::XmmRm) => {
            let source = read_rm(cpu, memory, insn)?;
            let low = if in_memory { source } else { source >> 64 };
            cpu.xmm[reg] = cpu.xmm[reg] & !low_mask(8) | low;
        }
        (Op::Movhps, Encoding::XmmRm) => {
            let source = read_rm(cpu, memory, insn)?;
            cpu.xmm[reg] = cpu.xmm[reg] & low_mask(8) | source << 64;
        }
        (Op::Movhps, _) => {
            if let Some(address) = aligned_address(cpu, insn)? {
                write_vector(memory, address, 8, cpu.xmm[reg] >> 64)?;
            }
        }
        // Scalar loads from memory clear the rest of the register; between
        // registers MOVSS and MOVSD merge and MOVQ clears.
        (Op::Movss | Op::Movsd, Encoding::XmmRm) if !in_memory => {
            let mask = low_mask(insn.rm_size);
            let source = read_rm(cpu, memory, insn)?;
            cpu.xmm[reg] = cpu.xmm[reg] & !mask | source & mask;
        }
        (
            Op::Movaps | Op::Movups | Op::Movdqa | Op::Movdqu | Op::Movss | Op::Movsd | Op::Movq,
            Encoding::XmmRm,
        ) => {
            cpu.xmm[reg] = read_rm(cpu, memory, insn)? & low_mask(insn.rm_size);
        }
        (_, Encoding::RmXmm) => {
            let mask = low_mask(insn.rm_size);
            let value = cpu.xmm[reg] & mask;
            match (aligned_address(cpu, insn)?, rm_operand(insn)?) {
                (Some(address), _) => write_vector(memory, address, insn.rm_size, value)?,
                (None, RmOperand::Reg(target)) => {
                    let target = &mut cpu.xmm[xmm(target)];
                    *target = match insn.op {
                        Op::Movq => value,
                        _ => *target & !mask | value,
                    };
                }
                (None, RmOperand::Mem(_)) => return Err(ExitReason::IllegalInstruction(0)),
            }
        }
        (_, Encoding::RmImmXmm) => {
            if let RmOperand::Reg(target) = rm_operand(insn)? {
                let target = &mut cpu.xmm[xmm(target)];
                *target = match insn.op {
                    Op::Psrldq => shift_bytes(*target, insn.imm, false),
                    Op::Pslldq => shift_bytes(*target, insn.imm, true),
                    op => shift(op, *target, insn.imm),
                };
            }
        }
        (_, Encoding::XmmRmImm) => {
            let source = read_rm(cpu, memory, insn)?;
            let old = cpu.xmm[reg];
            let imm = insn.imm;
            cpu.xmm[reg] = match insn.op {
                Op::Pshufd => shuffle_32(source, source, imm),
                Op::Pshuflw => shuffle_16(source, imm, 0),
                Op::Pshufhw => shuffle_16(source, imm, 4),
                Op::Shufps => shuffle_32(old, source, imm),
                _ => build(64, |index| {
                    let from = if index == 0 { old } else { source };
                    lane(from, 64, (imm >> index & 1) as u32)
                }),
            };
        }
        (op, _) => {
            let source = read_rm(cpu, memory, insn)?;
            cpu.xmm[reg] = packed(op, cpu.xmm[reg], source);
        }
    }
    Ok(())
}
