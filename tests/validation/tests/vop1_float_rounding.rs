// SPDX-License-Identifier: GPL-3.0-or-later

//! Independent per-lane expected values for the four 32-bit VOP1 binary32
//! rounding forms (#710): `v_trunc_f32`, `v_ceil_f32`, `v_rndne_f32` and
//! `v_floor_f32`, all `_e32`, Wave32.
//!
//! The expectations come from the public AMD "RDNA 2" Instruction Set
//! Architecture Reference Guide only: section 12.8 "VOP1 Instructions" (page
//! 140: the TRUNC, CEIL, RNDNE and FLOOR pseudocode), section 3.5 and 6.4
//! (MODE register, FP_DENORM), section 13.3.2 for the VOP1 word and section 6.2
//! for SRC0. `KNOWN` is a table written by hand from those definitions;
//! `expect` transcribes the pseudocode in exact arithmetic. Nothing here comes
//! from the ISA crate, the registry, the decoder or the shader compiler.
//!
//! Readings of the guide (not hardware evidence):
//! - Pinned mode: MODE.FP_DENORM single-precision input denormals allowed
//!   (modes 1 or 3), so a subnormal input keeps its exact value: FLOOR of the
//!   smallest negative subnormal is -1.0. With input flushing (modes 0 and 2)
//!   it would be read as -0 and give -0; that mode is outside this contract.
//!   Results are integers or signed zeros, never subnormal, so output
//!   flushing cannot change them.
//! - The pseudocode is evaluated exactly: RNDNE's floor(S0 + 0.5) does not
//!   round S0 + 0.5 first, which matches the "round-to-nearest-even semantics"
//!   description for every input except the zero sign below.
//! - Refused: every NaN input (quiet or signalling). The pseudocode has no NaN
//!   case and the result depends on MODE.IEEE and MODE.DX10_CLAMP (section
//!   3.5), so no value is expected. For RNDNE, a negative input of magnitude at
//!   most 0.5 (including -0 and negative subnormals): the pseudocode gives
//!   floor(+x) = +0, while round-to-nearest-even semantics keep the sign.
//! - Refused: integer inline constants (selectors 128-208) as SRC0. The guide
//!   does not say whether a float operation reads them as integer bits or as
//!   converted values. Float inline constants stay outside the VOP1 subset.
//! - None of the four writes VCC or SCC; inactive lanes keep their VGPRs
//!   (section 3.3).

#[path = "operand_domains/harness.rs"]
#[allow(dead_code)]
mod harness;
#[path = "../independent_ref.rs"]
mod independent_ref;

use std::collections::BTreeMap;

use harness::{Case, words};
use xenora_shader::{Architecture, CompileError, compile};

/// The four forms with their ENC_VOP1 opcodes (guide 12.8).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Form {
    Trunc = 33,
    Ceil = 34,
    Rndne = 35,
    Floor = 36,
}

const FORMS: [(Form, &str); 4] = [
    (Form::Trunc, "v_trunc_f32"),
    (Form::Ceil, "v_ceil_f32"),
    (Form::Rndne, "v_rndne_f32"),
    (Form::Floor, "v_floor_f32"),
];

/// A refused input: no expected value.
const NO: Option<u32> = None;

const fn y(bits: u32) -> Option<u32> {
    Some(bits)
}

/// S0 and the expected D for TRUNC, CEIL, RNDNE and FLOOR, by hand.
#[rustfmt::skip]
const KNOWN: [(u32, [Option<u32>; 4]); 43] = [
    (0x0000_0000, [y(0x0000_0000), y(0x0000_0000), y(0x0000_0000), y(0x0000_0000)]), // +0
    (0x8000_0000, [y(0x8000_0000), y(0x8000_0000), NO, y(0x8000_0000)]),             // -0
    (0x7f80_0000, [y(0x7f80_0000), y(0x7f80_0000), y(0x7f80_0000), y(0x7f80_0000)]), // +inf
    (0xff80_0000, [y(0xff80_0000), y(0xff80_0000), y(0xff80_0000), y(0xff80_0000)]), // -inf
    (0x7fc0_0000, [NO, NO, NO, NO]),                                                 // quiet NaN
    (0xffc0_0001, [NO, NO, NO, NO]),                                                 // -quiet NaN
    (0x7f80_0001, [NO, NO, NO, NO]),                                                 // signalling NaN
    (0xffbf_ffff, [NO, NO, NO, NO]),                                                 // -signalling NaN
    (0x0000_0001, [y(0x0000_0000), y(0x3f80_0000), y(0x0000_0000), y(0x0000_0000)]), // min subnormal
    (0x007f_ffff, [y(0x0000_0000), y(0x3f80_0000), y(0x0000_0000), y(0x0000_0000)]), // max subnormal
    (0x8000_0001, [y(0x8000_0000), y(0x8000_0000), NO, y(0xbf80_0000)]),             // -min subnormal
    (0x807f_ffff, [y(0x8000_0000), y(0x8000_0000), NO, y(0xbf80_0000)]),             // -max subnormal
    (0x0080_0000, [y(0x0000_0000), y(0x3f80_0000), y(0x0000_0000), y(0x0000_0000)]), // min normal
    (0x3e80_0000, [y(0x0000_0000), y(0x3f80_0000), y(0x0000_0000), y(0x0000_0000)]), // 0.25
    (0xbe80_0000, [y(0x8000_0000), y(0x8000_0000), NO, y(0xbf80_0000)]),             // -0.25
    (0x3eff_ffff, [y(0x0000_0000), y(0x3f80_0000), y(0x0000_0000), y(0x0000_0000)]), // 0.5 - ulp
    (0x3f00_0000, [y(0x0000_0000), y(0x3f80_0000), y(0x0000_0000), y(0x0000_0000)]), // 0.5
    (0x3f00_0001, [y(0x0000_0000), y(0x3f80_0000), y(0x3f80_0000), y(0x0000_0000)]), // 0.5 + ulp
    (0xbf00_0000, [y(0x8000_0000), y(0x8000_0000), NO, y(0xbf80_0000)]),             // -0.5
    (0xbf00_0001, [y(0x8000_0000), y(0x8000_0000), y(0xbf80_0000), y(0xbf80_0000)]), // -0.5 - ulp
    (0x3f40_0000, [y(0x0000_0000), y(0x3f80_0000), y(0x3f80_0000), y(0x0000_0000)]), // 0.75
    (0xbf40_0000, [y(0x8000_0000), y(0x8000_0000), y(0xbf80_0000), y(0xbf80_0000)]), // -0.75
    (0x3f7f_ffff, [y(0x0000_0000), y(0x3f80_0000), y(0x3f80_0000), y(0x0000_0000)]), // 1 - ulp
    (0xbf7f_ffff, [y(0x8000_0000), y(0x8000_0000), y(0xbf80_0000), y(0xbf80_0000)]), // -1 + ulp
    (0x3f80_0000, [y(0x3f80_0000), y(0x3f80_0000), y(0x3f80_0000), y(0x3f80_0000)]), // 1
    (0x3f80_0001, [y(0x3f80_0000), y(0x4000_0000), y(0x3f80_0000), y(0x3f80_0000)]), // 1 + ulp
    (0xbf80_0001, [y(0xbf80_0000), y(0xbf80_0000), y(0xbf80_0000), y(0xc000_0000)]), // -1 - ulp
    (0x3fc0_0000, [y(0x3f80_0000), y(0x4000_0000), y(0x4000_0000), y(0x3f80_0000)]), // 1.5
    (0xbfc0_0000, [y(0xbf80_0000), y(0xbf80_0000), y(0xc000_0000), y(0xc000_0000)]), // -1.5
    (0x401f_ffff, [y(0x4000_0000), y(0x4040_0000), y(0x4000_0000), y(0x4000_0000)]), // 2.5 - ulp
    (0x4020_0000, [y(0x4000_0000), y(0x4040_0000), y(0x4000_0000), y(0x4000_0000)]), // 2.5
    (0x4020_0001, [y(0x4000_0000), y(0x4040_0000), y(0x4040_0000), y(0x4000_0000)]), // 2.5 + ulp
    (0xc020_0000, [y(0xc000_0000), y(0xc000_0000), y(0xc000_0000), y(0xc040_0000)]), // -2.5
    (0x4060_0000, [y(0x4040_0000), y(0x4080_0000), y(0x4080_0000), y(0x4040_0000)]), // 3.5
    (0x42c9_0000, [y(0x42c8_0000), y(0x42ca_0000), y(0x42c8_0000), y(0x42c8_0000)]), // 100.5
    (0x42cb_0000, [y(0x42ca_0000), y(0x42cc_0000), y(0x42cc_0000), y(0x42ca_0000)]), // 101.5
    (0x4aff_fffd, [y(0x4aff_fffc), y(0x4aff_fffe), y(0x4aff_fffc), y(0x4aff_fffc)]), // 2^23 - 1.5
    (0x4aff_ffff, [y(0x4aff_fffe), y(0x4b00_0000), y(0x4b00_0000), y(0x4aff_fffe)]), // 2^23 - 0.5
    (0xcaff_ffff, [y(0xcaff_fffe), y(0xcaff_fffe), y(0xcb00_0000), y(0xcb00_0000)]), // -(2^23 - 0.5)
    (0x4b00_0000, [y(0x4b00_0000), y(0x4b00_0000), y(0x4b00_0000), y(0x4b00_0000)]), // 2^23
    (0x4b00_0001, [y(0x4b00_0001), y(0x4b00_0001), y(0x4b00_0001), y(0x4b00_0001)]), // 2^23 + 1
    (0x7f7f_ffff, [y(0x7f7f_ffff), y(0x7f7f_ffff), y(0x7f7f_ffff), y(0x7f7f_ffff)]), // max finite
    (0xff7f_ffff, [y(0xff7f_ffff), y(0xff7f_ffff), y(0xff7f_ffff), y(0xff7f_ffff)]), // -max finite
];

fn column(form: Form) -> usize {
    form as usize - 33
}

/// D for S0 = `bits`, transcribed from the guide 12.8 pseudocode in exact
/// arithmetic (binary64 holds every binary32 value and, below 2^23, S0 + 0.5
/// exactly). `None` for a refused input.
fn expect(form: Form, bits: u32) -> Option<u32> {
    let s = f64::from(f32::from_bits(bits));
    if s.is_nan() {
        return None;
    }
    // V_FLOOR_F32: D = trunc(S0); if S0 < 0 and S0 != D then D += -1.
    let floor = |v: f64| {
        let d = v.trunc();
        if v < 0.0 && v != d { d - 1.0 } else { d }
    };
    let trunc = s.trunc();
    let d = match form {
        Form::Trunc => trunc,
        // D = trunc(S0); if S0 > 0 and S0 != D then D += 1.
        Form::Ceil if s > 0.0 && s != trunc => trunc + 1.0,
        Form::Ceil => trunc,
        Form::Floor => floor(s),
        Form::Rndne if s.is_sign_negative() && s >= -0.5 => return None,
        // From 2^23 every binary32 value is an integer (no fraction bits).
        Form::Rndne if s.abs() >= 8_388_608.0 => s,
        // D = floor(S0 + 0.5); if floor(S0) is even and fract(S0) == 0.5 then D -= 1.
        Form::Rndne => {
            let fract = s - floor(s);
            let d = floor(s + 0.5);
            if floor(s) % 2.0 == 0.0 && fract == 0.5 {
                d - 1.0
            } else {
                d
            }
        }
    };
    Some((d as f32).to_bits())
}

const LITERAL: u32 = 255;
const VCC_LO: u32 = 106;

/// ENCODING 0b0111111 in bits 25-31, VDST 17-24, OP 9-16, SRC0 0-8 (13.3.2).
fn vop1(op: u32, dst: u32, src0: u32) -> u32 {
    0x7e00_0000 | dst << 17 | op << 9 | src0
}

const fn vgpr(n: u32) -> u32 {
    256 + n
}

/// `v_mov_b32 v<reg>, literal`.
fn vmov(reg: u32, value: u32) -> [u32; 2] {
    [0x7e00_02ff | reg << 17, value]
}

/// `s_mov_b32 s<reg>, literal`.
fn smov(reg: u32, value: u32) -> [u32; 2] {
    [0xbe80_03ff | reg << 16, value]
}

/// `v_xor_b32 v<dst>, v<a>, v<b>`.
fn vxor(dst: u32, a: u32, b: u32) -> u32 {
    29 << 25 | dst << 17 | b << 9 | vgpr(a)
}

const FULL_EXEC: u32 = 0xbefe_03c1;
const SCC_SET: u32 = 0xbf06_8080;
const SCC_CLEAR: u32 = 0xbf06_8180;
/// v6 = vcc_lo; v2 ^= v6; `s_cbranch_scc0` over v2 ^= v1.
const FLAG_TAIL: [u32; 4] = [0x7e0c_026a, 0x3a04_0506, 0xbf84_0001, 0x3a04_0501];
/// `buffer_load_dword v3, v0, s[0:3], 0 idxen` without the wait.
const PENDING_LOAD_V3: [u32; 2] = [0xe030_2000, 0x8000_0300];
/// XORed with v1 to give each lane its own old VGPR before a masked write.
const OLD: u32 = 0x0bad_f00d;
const EXEC_MASKS: [u32; 5] = [0, 0x5555_5555, 0xaaaa_aaaa, 0x8000_0001, u32::MAX];

/// Load v1 = lane input, `setup`, the ALU words, `after`, v2 = v<result>, store v2.
fn program(setup: &[u32], alu: &[u32], after: &[u32], result: u32) -> Vec<u8> {
    let mut code = vec![0xe030_2000, 0x8000_0100, 0xbf8c_3f70];
    code.extend(setup);
    code.extend(alu);
    code.extend(after);
    if result != 2 {
        code.push(0x7e04_0300 | result);
    }
    code.extend([0xe070_2000, 0x8001_0200, 0xbbfd_0000, 0xbf81_0000]);
    words(&code)
}

fn alu_offset(setup: &[u32]) -> usize {
    12 + 4 * setup.len()
}

/// Accepted inputs of `form`: the known values without refusals, then quarter
/// steps -16..16, values one ulp around each power of two 2^0..2^24 and their
/// negatives, and spread subnormals.
fn lanes(form: Form) -> Vec<u32> {
    let mut lanes: Vec<u32> = KNOWN.iter().map(|&(s, _)| s).collect();
    lanes.extend((-64..=64).map(|k| (k as f32 * 0.25).to_bits()));
    for e in 0..=24 {
        let p = 0x3f80_0000 + (e << 23);
        for bits in [p - 1, p, p + 1] {
            lanes.extend([bits, bits | 0x8000_0000]);
        }
    }
    lanes.extend([0x0000_0100, 0x0040_0000, 0x8000_0100, 0x8040_0000]);
    lanes.retain(|&s| expect(form, s).is_some());
    lanes
}

fn case(
    name: &'static str,
    family: &'static str,
    label: String,
    code: Vec<u8>,
    input: Vec<u32>,
    lane: impl Fn(usize, u32) -> u32,
) -> Case {
    let expected = input.iter().enumerate().map(|(n, &x)| lane(n, x)).collect();
    Case {
        form: name,
        family,
        label: format!("{name} {label}"),
        code,
        input,
        expected,
    }
}

fn masked(exec: u32, n: usize, d: u32, old: u32) -> u32 {
    if exec >> (n % 32) & 1 == 1 { d } else { old }
}

/// Source values for SGPR, vcc_lo and literal SRC0 cases.
const UNIFORM: [u32; 6] = [
    0xbf00_0001,
    0x4020_0000,
    0x8000_0001,
    0xc020_0000,
    0x3f7f_ffff,
    0xff80_0000,
];

fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for (form, name) in FORMS {
        let op = form as u32;
        let f = move |s| expect(form, s).expect("accepted input");
        let all = lanes(form);
        let alu = [vop1(op, 2, vgpr(1))];
        let code = program(&[], &alu, &[], 2);
        let label = format!("{} lanes", all.len());
        cases.push(case(
            name,
            "f32 lanes",
            label,
            code.clone(),
            all.clone(),
            |_, x| f(x),
        ));
        let short = all[..7].to_vec();
        cases.push(case(
            name,
            "f32 lanes",
            "7 lanes".into(),
            code,
            short,
            |_, x| f(x),
        ));
        // VDST equal to SRC0, and v255 as VDST and as SRC0.
        let code = program(&[], &[vop1(op, 1, vgpr(1))], &[], 1);
        cases.push(case(
            name,
            "f32 registers",
            "v1 = v1".into(),
            code,
            all.clone(),
            |_, x| f(x),
        ));
        let code = program(&[], &[vop1(op, 255, vgpr(1))], &[], 255);
        cases.push(case(
            name,
            "f32 registers",
            "v255 = v1".into(),
            code,
            all.clone(),
            |_, x| f(x),
        ));
        let b = 0xc020_0000;
        let code = program(&vmov(255, b), &[vop1(op, 2, vgpr(255))], &[], 2);
        cases.push(case(
            name,
            "f32 registers",
            "v2 = v255".into(),
            code,
            all.clone(),
            |_, _| f(b),
        ));
        // Uniform SRC0: s8, s105, vcc_lo and a literal dword.
        for value in UNIFORM.into_iter().filter(|&v| expect(form, v).is_some()) {
            for reg in [8, 105, VCC_LO] {
                let code = program(&smov(reg, value), &[vop1(op, 2, reg)], &[], 2);
                let label = format!("s{reg} = {value:#x}");
                cases.push(case(
                    name,
                    "f32 uniform source",
                    label,
                    code,
                    all.clone(),
                    |_, _| f(value),
                ));
            }
            let code = program(&[], &[vop1(op, 2, LITERAL), value], &[], 2);
            let label = format!("literal {value:#x}");
            cases.push(case(
                name,
                "f32 uniform source",
                label,
                code,
                all.clone(),
                |_, _| f(value),
            ));
        }
        // EXEC: inactive lanes keep v2 = v1 ^ OLD.
        let mut setup = vmov(5, OLD).to_vec();
        setup.push(vxor(2, 5, 1));
        for exec in EXEC_MASKS {
            let mut before = setup.clone();
            before.extend(smov(126, exec));
            let code = program(&before, &alu, &[FULL_EXEC], 2);
            let label = format!("exec {exec:#x}");
            cases.push(case(name, "f32 exec", label, code, all.clone(), |n, x| {
                masked(exec, n, f(x), x ^ OLD)
            }));
        }
        // VCC_LO and SCC are not written: D ^ VCC_LO, XORed with S0 when SCC is set.
        for vcc in [0, u32::MAX] {
            for scc in [false, true] {
                let mut before = smov(VCC_LO, vcc).to_vec();
                before.push(if scc { SCC_SET } else { SCC_CLEAR });
                let code = program(&before, &alu, &FLAG_TAIL, 2);
                let label = format!("vcc_lo {vcc:#x}, scc {scc}");
                cases.push(case(
                    name,
                    "f32 no vcc or scc write",
                    label,
                    code,
                    all.clone(),
                    |_, x| f(x) ^ vcc ^ if scc { x } else { 0 },
                ));
            }
        }
    }
    cases
}

/// Per form: one VGPR, five uniform sources of which RNDNE refuses one, five
/// EXEC masks and four flag seeds, plus the two lane counts and three
/// register cases.
const COUNTS: [(&str, usize); 5] = [
    ("f32 exec", 4 * 5),
    ("f32 lanes", 4 * 2),
    ("f32 no vcc or scc write", 4 * 4),
    ("f32 registers", 4 * 3),
    ("f32 uniform source", 4 * 4 * 6 - 4),
];

/// Programs the decoders refuse at the ALU word: a missing literal, the
/// integer inline constants and every other selector outside the subset (with
/// the SDWA and DPP extension dword where one belongs), and a read and a write
/// of a VGPR with a load outstanding.
fn rejections() -> Vec<(String, Vec<u8>, usize)> {
    let setup = vmov(3, 0x3fc0_0000);
    let offset = alu_offset(&setup);
    let extension = |src0: u32| match src0 {
        249 => vec![0x0606_0601],
        250 => vec![0xff00_e401],
        233 | 234 => vec![0xfac6_8801],
        _ => vec![],
    };
    let mut all = Vec::new();
    for (form, name) in FORMS {
        let op = form as u32;
        let mut code = vec![0xe030_2000, 0x8000_0100, 0xbf8c_3f70];
        code.extend(setup);
        code.push(vop1(op, 2, LITERAL));
        all.push((format!("{name} missing literal"), words(&code), offset));
        for src0 in 107..=254 {
            let mut alu = vec![vop1(op, 2, src0)];
            alu.extend(extension(src0));
            let code = program(&setup, &alu, &[], 2);
            all.push((format!("{name} src0 {src0}"), code, offset));
        }
        let mut pending = setup.to_vec();
        pending.extend(PENDING_LOAD_V3);
        let at = alu_offset(&pending);
        for (dst, src0, what) in [(2, 3, "read"), (3, 1, "write")] {
            let code = program(&pending, &[vop1(op, dst, vgpr(src0))], &[], 2);
            all.push((
                format!("{name} {what} of v3 with its load outstanding"),
                code,
                at,
            ));
        }
    }
    all
}

const REJECTIONS: usize = 4 * (1 + 148 + 2);

/// Programs both interpreters refuse at run time at the ALU word: a refused
/// input in one lane, or as a literal. The compiler cannot see lane data.
fn refused_inputs() -> Vec<(String, Vec<u8>, Vec<u32>, usize)> {
    let mut all = Vec::new();
    for (form, name) in FORMS {
        let op = form as u32;
        for &(s, expected) in KNOWN.iter().filter(|(_, d)| d[column(form)].is_none()) {
            assert_eq!(expected[column(form)], expect(form, s));
            let mut input = lanes(form)[..5].to_vec();
            input[3] = s;
            let code = program(&[], &[vop1(op, 2, vgpr(1))], &[], 2);
            all.push((format!("{name} lane {s:#x}"), code, input, 12));
            let code = program(&[], &[vop1(op, 2, LITERAL), s], &[], 2);
            all.push((format!("{name} literal {s:#x}"), code, lanes(form), 12));
        }
    }
    all
}

/// Four NaNs per form and five negative RNDNE inputs of magnitude at most 0.5
/// (-0, two negative subnormals, -0.25 and -0.5), each as a lane and as a literal.
const REFUSED: usize = 2 * (4 * 4 + 5);

fn assert_rejected_at(label: &str, code: &[u8], offset: usize, input: &[u32]) {
    match xenora_ref::execute(code, input) {
        Ok(_) => panic!("reference accepted {label}"),
        Err(error) => assert_eq!(
            error.offset, offset,
            "reference rejected {label} elsewhere: {error:?}"
        ),
    }
    match independent_ref::execute(code, input) {
        Ok(_) => panic!("independent accepted {label}"),
        Err(error) => assert!(
            error.starts_with(&format!("{offset:#x}:")),
            "independent rejected {label} elsewhere: {error}"
        ),
    }
}

fn checked_cases() -> Vec<Case> {
    let cases = cases();
    let mut counts = BTreeMap::new();
    for case in &cases {
        *counts.entry(case.family).or_default() += 1;
    }
    assert_eq!(counts, BTreeMap::from(COUNTS));
    cases
}

fn failures(
    cases: &[Case],
    execute: impl Fn(&[u8], &[u32]) -> Result<Vec<u32>, String>,
) -> Vec<String> {
    cases
        .iter()
        .filter_map(|case| {
            let output = execute(&case.code, &case.input);
            (output.as_deref().ok() != Some(case.expected.as_slice()))
                .then(|| format!("  {}: {output:?}", case.label))
        })
        .collect()
}

#[test]
fn pseudocode_transcription_reproduces_the_hand_table() {
    for (s, expected) in KNOWN {
        for (form, name) in FORMS {
            assert_eq!(expect(form, s), expected[column(form)], "{name}({s:#x})");
        }
    }
}

#[test]
fn rounding_forms_match_the_guide_on_the_reference() {
    let cases = checked_cases();
    let failed = failures(&cases, |code, input| {
        xenora_ref::execute(code, input).map_err(|error| format!("{error:?}"))
    });
    assert!(
        failed.is_empty(),
        "reference disagrees with the guide:\n{}",
        failed.join("\n")
    );
    eprintln!("vop1_float_rounding: {} reference cases", cases.len());
}

#[test]
fn rounding_forms_match_the_guide_on_the_independent_interpreter() {
    let cases = checked_cases();
    let failed = failures(&cases, independent_ref::execute);
    assert!(
        failed.is_empty(),
        "independent interpreter disagrees:\n{}",
        failed.join("\n")
    );
    eprintln!(
        "vop1_float_rounding: {} independent-interpreter cases",
        cases.len()
    );
}

#[test]
fn unsupported_rounding_encodings_and_inputs_are_refused() {
    let rejections = rejections();
    assert_eq!(rejections.len(), REJECTIONS);
    let input = lanes(Form::Trunc);
    for (label, code, offset) in &rejections {
        assert_rejected_at(label, code, *offset, &input);
    }
    let refused = refused_inputs();
    assert_eq!(refused.len(), REFUSED);
    for (label, code, input, offset) in &refused {
        assert_rejected_at(label, code, *offset, input);
    }
    eprintln!(
        "vop1_float_rounding: {} rejected encodings, {} refused inputs",
        rejections.len(),
        refused.len()
    );
}

fn check_compiler_rejections() {
    for (label, code, offset) in rejections() {
        let at = match compile(Architecture::Gfx1030, &code) {
            Err(CompileError::Decode(error)) => error.offset,
            Err(CompileError::Verify { offset, .. }) => offset,
            other => panic!("compiler did not reject {label}: {other:?}"),
        };
        assert_eq!(at, offset, "compiler rejected {label} elsewhere");
    }
}

#[test]
fn compiler_matches_the_rounding_operand_domain() {
    for case in &checked_cases() {
        compile(Architecture::Gfx1030, &case.code)
            .unwrap_or_else(|error| panic!("compiler rejected {}: {error:?}", case.label));
    }
    check_compiler_rejections();
}

#[test]
#[ignore = "requires a native Metal device with API and shader validation"]
fn rounding_forms_match_the_guide_on_native_metal() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let cases = checked_cases();
    check_compiler_rejections();
    let (failures, programs) = harness::check_metal(&cases);
    failures.assert_none("native Metal disagrees with the guide");
    eprintln!(
        "vop1_float_rounding: {} Metal cases in {programs} programs",
        cases.len()
    );
}
