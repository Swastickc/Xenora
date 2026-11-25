// SPDX-License-Identifier: GPL-3.0-or-later

//! Assembler encodings against LLVM (`clang -target arm64-apple-macos -c`, read
//! back with `objdump -d`).

use xenora_loader::Arm64Assembler;
use xenora_loader::arm64::Condition;

type Emit = fn(&mut Arm64Assembler);

#[test]
fn flag_and_byte_operations_match_llvm() {
    let cases: &[(&str, Emit, u32)] = &[
        ("cset w0, hs", |a| a.cset_32(0, Condition::Cs), 0x1a9f37e0),
        ("cset w12, vs", |a| a.cset_32(12, Condition::Vs), 0x1a9f77ec),
        ("cset w30, eq", |a| a.cset_32(30, Condition::Eq), 0x1a9f17fe),
        (
            "csel x9, x10, x11, ne",
            |a| a.csel_64(9, 10, 11, Condition::Ne),
            0x9a8b1149,
        ),
        (
            "csel x0, x30, x17, lt",
            |a| a.csel_64(0, 30, 17, Condition::Lt),
            0x9a91b3c0,
        ),
        (
            "strb w0, [x19, #136]",
            |a| a.strb_uoff(0, 19, 136),
            0x39022260,
        ),
        (
            "strb wzr, [x19, #141]",
            |a| a.strb_uoff(31, 19, 141),
            0x3902367f,
        ),
        (
            "ldrb w12, [x19, #140]",
            |a| a.ldrb_uoff(12, 19, 140),
            0x3942326c,
        ),
        (
            "eor w12, w11, w10, lsr #4",
            |a| a.eor_lsr_32(12, 11, 10, 4),
            0x4a4a116c,
        ),
        (
            "eor w1, w2, w3, lsr #0",
            |a| a.eor_lsr_32(1, 2, 3, 0),
            0x4a430041,
        ),
        (
            "orr w12, w13, w14",
            |a| a.orr_reg_32(12, 13, 14),
            0x2a0e01ac,
        ),
        (
            "bic w12, w13, w14",
            |a| a.bic_reg_32(12, 13, 14),
            0x0a2e01ac,
        ),
        ("mvn x11, x9", |a| a.mvn_64(11, 9), 0xaa2903eb),
        ("mvn x0, x30", |a| a.mvn_64(0, 30), 0xaa3e03e0),
        (
            "ubfx w12, w13, #4, #1",
            |a| a.ubfx_bit_32(12, 13, 4),
            0x530411ac,
        ),
        ("rev x10, x9", |a| a.rev(10, 9, true), 0xdac00d2a),
        ("rev w0, w30", |a| a.rev(0, 30, false), 0x5ac00bc0),
        ("lsr x0, x0, #8", |a| a.lsr_imm_64(0, 0, 8), 0xd348fc00),
        ("lsr x9, x30, #63", |a| a.lsr_imm_64(9, 30, 63), 0xd37fffc9),
        ("sxtb w9, w10", |a| a.sxtb_32(9, 10), 0x13001d49),
        ("sxtb w30, w0", |a| a.sxtb_32(30, 0), 0x13001c1e),
        ("sxth w9, w10", |a| a.sxth_32(9, 10), 0x13003d49),
        ("sxth w0, w30", |a| a.sxth_32(0, 30), 0x13003fc0),
    ];
    for (text, emit, word) in cases {
        let mut assembler = Arm64Assembler::new();
        emit(&mut assembler);
        assert_eq!(assembler.instructions(), [*word], "{text}");
    }
}

#[test]
fn vector_operations_match_llvm() {
    let cases: &[(&str, Emit, u32)] = &[
        (
            "ldr q1, [x19, #160]",
            |a| a.ldr_q_uoff(1, 19, 160),
            0x3dc02a61,
        ),
        (
            "str q0, [x19, #400]",
            |a| a.str_q_uoff(0, 19, 400),
            0x3d806660,
        ),
        ("fmov d1, x0", |a| a.fmov_d_from_x(1, 0), 0x9e670001),
        ("mov v1.d[1], x1", |a| a.ins_d1_from_x(1, 1), 0x4e181c21),
        (
            "and v0.16b, v1.16b, v2.16b",
            |a| a.neon3(0x4e20_1c00, 0, 1, 2),
            0x4e221c20,
        ),
        (
            "sub v31.2d, v30.2d, v29.2d",
            |a| a.neon3(0x6ee0_8400, 31, 30, 29),
            0x6efd87df,
        ),
        (
            "shl v0.8h, v1.8h, #0",
            |a| a.shl_vector(0, 1, 16, 0),
            0x4f105420,
        ),
        (
            "shl v0.8h, v1.8h, #15",
            |a| a.shl_vector(0, 1, 16, 15),
            0x4f1f5420,
        ),
        (
            "shl v2.4s, v3.4s, #31",
            |a| a.shl_vector(2, 3, 32, 31),
            0x4f3f5462,
        ),
        (
            "shl v4.2d, v5.2d, #63",
            |a| a.shl_vector(4, 5, 64, 63),
            0x4f7f54a4,
        ),
        (
            "ushr v0.8h, v1.8h, #1",
            |a| a.ushr_vector(0, 1, 16, 1),
            0x6f1f0420,
        ),
        (
            "ushr v0.8h, v1.8h, #16",
            |a| a.ushr_vector(0, 1, 16, 16),
            0x6f100420,
        ),
        (
            "ushr v2.4s, v3.4s, #32",
            |a| a.ushr_vector(2, 3, 32, 32),
            0x6f200462,
        ),
        (
            "ushr v4.2d, v5.2d, #64",
            |a| a.ushr_vector(4, 5, 64, 64),
            0x6f4004a4,
        ),
        (
            "sshr v0.8h, v1.8h, #16",
            |a| a.sshr_vector(0, 1, 16, 16),
            0x4f100420,
        ),
        (
            "sshr v2.4s, v3.4s, #5",
            |a| a.sshr_vector(2, 3, 32, 5),
            0x4f3b0462,
        ),
    ];
    for (text, emit, word) in cases {
        let mut assembler = Arm64Assembler::new();
        emit(&mut assembler);
        assert_eq!(assembler.instructions(), [*word], "{text}");
    }
}

#[test]
fn carry_arithmetic_matches_llvm() {
    // `llvm-mc -triple=aarch64-apple-macos -show-encoding`.
    let cases: &[(&str, Emit, u32)] = &[
        (
            "adcs x11, x9, x10",
            |a| a.adcs_reg_64(11, 9, 10),
            0xba0a012b,
        ),
        (
            "adcs w11, w9, w10",
            |a| a.adcs_reg_32(11, 9, 10),
            0x3a0a012b,
        ),
        (
            "adcs x0, x30, x17",
            |a| a.adcs_reg_64(0, 30, 17),
            0xba1103c0,
        ),
        (
            "sbcs x11, x9, x10",
            |a| a.sbcs_reg_64(11, 9, 10),
            0xfa0a012b,
        ),
        (
            "sbcs w11, w9, w10",
            |a| a.sbcs_reg_32(11, 9, 10),
            0x7a0a012b,
        ),
        (
            "sbcs w0, w30, w17",
            |a| a.sbcs_reg_32(0, 30, 17),
            0x7a1103c0,
        ),
    ];
    for (text, emit, word) in cases {
        let mut assembler = Arm64Assembler::new();
        emit(&mut assembler);
        assert_eq!(assembler.instructions(), [*word], "{text}");
    }
}
