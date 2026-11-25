// SPDX-License-Identifier: GPL-3.0-or-later

//! Independent expected values for `s_buffer_load_dword`, `_x2` and `_x4` (#189).
//!
//! The expectations come only from the public AMD "RDNA 2" Instruction Set
//! Architecture Reference Guide: section 7.1 (operands), 7.2.1 (address), 7.3
//! (LGKM_CNT), 7.5 (alignment and bounds) and Tables 73 and 74 of section 13.2.1
//! (encoding). Nothing here uses the ISA crate, the decoder or the compiler: a
//! case states the SGPR values before the load and the buffer, and the expected
//! value of each word follows from those.
//!
//! The fixture's descriptor `s[0:3]` is the dispatch's input: base 0, and a word
//! is in range if the dispatch has it.

#[path = "operand_domains/harness.rs"]
#[allow(dead_code)]
mod harness;

use harness::{Case, Rejection, words};
use xenora_shader::{Architecture, CompileError, compile};

const NULL: u32 = 125;
const WAIT_SCALAR: u32 = 0xbf8c_c07f;
const WAIT_VECTOR: u32 = 0xbf8c_3f70;
const END: u32 = 0xbf81_0000;

fn s_mov(dst: u32, value: u32) -> [u32; 2] {
    [0xbe80_03ff | dst << 16, value]
}

/// `words` is 1, 2 or 4.
fn load(words: u32, dst: u32, offset: u32, soffset: u32) -> [u32; 2] {
    let opcode = 8 + words.trailing_zeros();
    [0x3d << 26 | opcode << 18 | dst << 6, soffset << 25 | offset]
}

fn v_mov(dst: u32, src0: u32) -> u32 {
    0x7e00_0200 | dst << 17 | src0
}

fn vgpr(register: u32) -> u32 {
    256 + register
}

/// v1 = the lane's input word.
fn load_lane() -> [u32; 3] {
    [0xe030_2000, 0x8000_0100, WAIT_VECTOR]
}

/// Stores `register` as the lane's result.
fn finish(register: u32) -> [u32; 4] {
    [0xe070_2000, 0x8001_0000 | register << 8, 0xbbfd_0000, END]
}

/// Word `index` of a load, by guide 7.2.1 and 7.5.
fn loaded(buffer: &[u32], offset: u32, soffset: u32, index: u32, old: u32) -> u32 {
    let byte = u128::from(offset) + u128::from(soffset);
    let word = byte / 4 + u128::from(index);
    let mut value = old;
    for (position, candidate) in buffer.iter().enumerate() {
        if position as u128 == word {
            value = *candidate;
        }
    }
    value
}

fn buffer(length: u32) -> Vec<u32> {
    (0..length)
        .map(|index| 0x9e37_79b9u32.wrapping_mul(index + 1) ^ 0x8000_0001)
        .collect()
}

fn old_value(index: u32) -> u32 {
    0xa0b0_c000 + index
}

struct Build {
    cases: Vec<Case>,
}

impl Build {
    fn push(
        &mut self,
        family: &'static str,
        label: String,
        code: &[u32],
        input: Vec<u32>,
        expected: Vec<u32>,
    ) {
        self.cases.push(Case {
            form: "s_buffer_load",
            family,
            label,
            code: words(code),
            input,
            expected,
        });
    }
}

/// Every width, offset class and observed word over several buffer lengths.
fn address_cases(build: &mut Build) {
    let offsets: &[(u32, Option<u32>)] = &[
        (0, None),
        (4, None),
        (0x7c, None),
        (1, None),
        (3, None),
        (6, None),
        (0xf_ffff, None),
        (0xf_fffc, None),
        (0, Some(0)),
        (0, Some(8)),
        (8, Some(4)),
        (3, Some(1)), // the sum is aligned although neither part is
        (2, Some(1)),
        (0, Some(0xffff_ffff)),
        (0xf_ffff, Some(0xffff_ffff)), // the sum does not fit in 32 bits
        (4, Some(0xffff_fffc)),        // wraps to 0 in 32 bits
        (0, Some(0x8000_0000)),
        (0, Some(0x3fff_fffc)),
    ];
    for length in [1, 3, 5, 32, 33, 64] {
        let input = buffer(length);
        for &(offset, soffset) in offsets {
            for width in [1, 2, 4] {
                for observed in 0..width {
                    for dst in [8, 100] {
                        let mut code = Vec::new();
                        for index in 0..width {
                            code.extend(s_mov(dst + index, old_value(index)));
                        }
                        let selector = match soffset {
                            Some(value) => {
                                code.extend(s_mov(20, value));
                                20
                            }
                            None => NULL,
                        };
                        code.extend(load(width, dst, offset, selector));
                        code.push(WAIT_SCALAR);
                        code.push(v_mov(2, dst + observed));
                        code.extend(finish(2));
                        let value = loaded(
                            &input,
                            offset,
                            soffset.unwrap_or(0),
                            observed,
                            old_value(observed),
                        );
                        build.push(
                            "address",
                            format!("x{width} s{dst} {offset:#x}+{soffset:x?} word {observed} of {length}"),
                            &code,
                            input.clone(),
                            vec![value; length as usize],
                        );
                    }
                }
            }
        }
    }
}

/// Loaded SGPRs as vector sources, and what else the load must not disturb.
fn consumer_cases(build: &mut Build) {
    let input = buffer(40);
    let lanes = |f: &dyn Fn(usize, u32) -> u32| -> Vec<u32> {
        input
            .iter()
            .enumerate()
            .map(|(lane, x)| f(lane, *x))
            .collect()
    };
    let start = |dst: u32, offset: u32| {
        let mut code: Vec<u32> = s_mov(dst, 5).to_vec();
        code.extend(load(1, dst, offset, NULL));
        code.push(WAIT_SCALAR);
        code.extend(load_lane());
        code
    };
    let s = input[3];

    let mut code = start(10, 12);
    code.push(0x4a00_0000 | 2 << 17 | 1 << 9 | 10); // v_add_nc_u32 v2, s10, v1
    code.extend(finish(2));
    build.push(
        "consumer",
        "vop2 src0".into(),
        &code,
        input.clone(),
        lanes(&|_, x| x.wrapping_add(s)),
    );

    let mut code = start(10, 12);
    code.push(0x7e00_0000 | 2 << 17 | 55 << 9 | 10); // v_not_b32 v2, s10
    code.extend(finish(2));
    build.push(
        "consumer",
        "vop1 src0".into(),
        &code,
        input.clone(),
        lanes(&|_, _| !s),
    );

    // v_cmp_lt_u32 vcc_lo, s10, v1, then the wave's mask.
    let mut code = start(10, 12);
    code.push(0x7d82_0000 | 1 << 9 | 10);
    code.push(v_mov(2, 106));
    code.extend(finish(2));
    let mask = |lane: usize| -> u32 {
        let base = lane / 32 * 32;
        (0..32)
            .filter(|n| input.get(base + n).is_some_and(|x| s < *x))
            .map(|n| 1 << n)
            .sum()
    };
    build.push(
        "consumer",
        "compare src0".into(),
        &code,
        input.clone(),
        lanes(&|lane, _| mask(lane)),
    );

    // v_cmp_eq_u64 vcc_lo, s[10:11], v[3:4] (VOPC opcode 226).
    for equal in [true, false] {
        let mut code: Vec<u32> = Vec::new();
        code.extend(s_mov(10, 0));
        code.extend(s_mov(11, 0));
        code.extend(load(2, 10, 8, NULL));
        code.push(WAIT_SCALAR);
        code.extend([0x7e06_02ff, input[2]]); // v_mov_b32 v3, literal
        code.extend([0x7e08_02ff, if equal { input[3] } else { !input[3] }]);
        code.push(0x7c00_0000 | 226 << 17 | 3 << 9 | 10);
        code.push(v_mov(2, 106));
        code.extend(finish(2));
        let full = |lane: usize| if lane < 32 { u32::MAX } else { 0xff };
        build.push(
            "consumer",
            format!("64-bit compare {equal}"),
            &code,
            input.clone(),
            lanes(&|lane, _| if equal { full(lane) } else { 0 }),
        );
    }

    // A scalar load runs with EXEC zero, and only active lanes take the move.
    for exec in [0u32, 1, 0x8000_0000, 0x5555_5555] {
        let mut code: Vec<u32> = load_lane().to_vec();
        code.extend(s_mov(10, 5));
        code.extend(s_mov(126, 0));
        code.extend(load(1, 10, 12, NULL));
        code.push(WAIT_SCALAR);
        code.push(v_mov(1, 10)); // no lane
        code.extend(s_mov(126, exec));
        code.push(v_mov(1, 10));
        code.extend(s_mov(126, u32::MAX));
        code.extend(finish(1));
        build.push(
            "exec",
            format!("exec {exec:#x}"),
            &code,
            input.clone(),
            lanes(&|lane, x| if exec >> (lane % 32) & 1 == 1 { s } else { x }),
        );
    }

    // Two loads before one wait; the second reads what the first one wrote.
    let mut code: Vec<u32> = Vec::new();
    code.extend(s_mov(10, 7));
    code.extend(s_mov(11, 9));
    code.extend(load(1, 10, 0, NULL));
    code.extend(load(1, 11, 4, NULL));
    code.push(WAIT_SCALAR);
    code.extend(load(1, 10, 0xf_0000, NULL)); // out of range: keeps the first value
    code.push(WAIT_SCALAR);
    code.push(v_mov(1, 11));
    code.push(0x4a00_0000 | 2 << 17 | 1 << 9 | 10);
    code.extend(finish(2));
    let sum = input[0].wrapping_add(input[1]);
    build.push(
        "order",
        "two loads, one wait".into(),
        &code,
        input.clone(),
        lanes(&|_, _| sum),
    );

    // A wait for vector loads leaves the scalar load outstanding and vice versa.
    let mut code: Vec<u32> = Vec::new();
    code.extend([0xe030_2000, 0x8000_0100]); // v1 = input, no wait yet
    code.extend(s_mov(10, 7));
    code.extend(load(1, 10, 16, NULL));
    code.push(WAIT_SCALAR);
    code.push(v_mov(2, 10));
    code.push(WAIT_VECTOR);
    code.push(0x4a00_0000 | 2 << 17 | 2 << 9 | vgpr(1));
    code.extend(finish(2));
    let t = input[4];
    build.push(
        "order",
        "independent counters".into(),
        &code,
        input.clone(),
        lanes(&|_, x| x.wrapping_add(t)),
    );

    // Under a run-time VCC branch: lanes of a wave with any x > s take it.
    let mut code = start(10, 12);
    code.push(v_mov(2, vgpr(1)));
    code.push(0x7d82_0000 | 1 << 9 | 10); // vcc_lo = s10 < v1
    code.push(0xbf86_0001); // s_cbranch_vccz over one word
    code.push(v_mov(2, 10));
    code.extend(finish(2));
    build.push(
        "branch",
        "loaded value under a run-time branch".into(),
        &code,
        input.clone(),
        lanes(&|lane, x| if mask(lane) != 0 { s } else { x }),
    );
    // A scalar write after the wait replaces the loaded value.
    let mut code = start(10, 12);
    code.extend(s_mov(10, 77));
    code.push(v_mov(2, 10));
    code.extend(finish(2));
    build.push(
        "order",
        "overwritten".into(),
        &code,
        input.clone(),
        lanes(&|_, _| 77),
    );
}

/// The LLVM-assembled fixture, with all, some and none of the words in range.
fn fixture_cases(build: &mut Build) {
    const CODE: &[u8] = include_bytes!("../../../fixtures/rdna2/scalar-memory.bin");
    for length in [1, 2, 5, 8, 70] {
        let input = buffer(length);
        let word = |index: usize| input.get(index).copied().unwrap_or(0);
        let expected = input
            .iter()
            .map(|x| {
                let mut value = !(x.wrapping_add(word(1)) ^ word(2));
                value = !(value.wrapping_add(word(3)) ^ word(4));
                value = !(value.wrapping_add(word(5)) ^ word(6));
                value.wrapping_add(word(7))
            })
            .collect();
        build.cases.push(Case {
            form: "s_buffer_load",
            family: "fixture",
            label: format!("fixture over {length}"),
            code: CODE.to_vec(),
            input,
            expected,
        });
    }
}

fn cases() -> Vec<Case> {
    let mut build = Build { cases: Vec::new() };
    fixture_cases(&mut build);
    address_cases(&mut build);
    consumer_cases(&mut build);
    build.cases
}

fn rejections() -> Vec<Rejection> {
    let mut out = Vec::new();
    let mut reject = |label: &str, prologue: &[u32], rest: &[u32], truncated: bool| {
        let code = [prologue, rest].concat();
        out.push(Rejection {
            family: "rejection",
            truncated,
            label: label.into(),
            offset: 4 * prologue.len(),
            code: words(&code),
        });
    };
    let ready: Vec<u32> = (8..16).flat_map(|register| s_mov(register, 1)).collect();
    let tail = |first: &[u32]| -> Vec<u32> {
        let mut code = first.to_vec();
        code.push(WAIT_SCALAR);
        code.push(v_mov(2, 8));
        code.extend(finish(2));
        code
    };
    let [w0, w1] = load(1, 8, 0, NULL);
    for (label, first) in [
        ("glc", [w0 | 1 << 16, w1]),
        ("dlc", [w0 | 1 << 14, w1]),
        ("reserved bit 13", [w0 | 1 << 13, w1]),
        ("reserved bit 15", [w0 | 1 << 15, w1]),
        ("reserved bit 17", [w0 | 1 << 17, w1]),
        ("reserved bit 53", [w0, w1 | 1 << 21]),
        ("reserved bit 56", [w0, w1 | 1 << 24]),
        ("negative offset", [w0, w1 | 1 << 20]),
        ("sbase not a quad", [w0 | 1, w1]),
        ("sbase past s103", [w0 | 52, w1]),
        ("soffset vcc_lo", load(1, 8, 0, 106)),
        ("soffset ttmp0", load(1, 8, 0, 108)),
        ("soffset m0", load(1, 8, 0, 124)),
        ("soffset exec_lo", load(1, 8, 0, 126)),
        ("sdata vcc_lo", load(1, 106, 0, NULL)),
        ("sdata m0", load(1, 124, 0, NULL)),
        ("sdata exec_lo", load(1, 126, 0, NULL)),
        ("x2 odd", load(2, 9, 0, NULL)),
        ("x4 pair", load(4, 10, 0, NULL)),
        ("x4 past s105", load(4, 104, 0, NULL)),
        ("x8", [0x3d << 26 | 11 << 18 | 8 << 6, w1]),
        ("x16", [0x3d << 26 | 12 << 18 | 8 << 6, w1]),
        ("s_load_dword", [0x3d << 26 | 8 << 6, w1]),
        ("s_load_dwordx2", [0x3d << 26 | 1 << 18 | 8 << 6, w1]),
        ("the output binding", [w0 | 2, w1]),
        ("another binding", [w0 | 4, w1]),
    ] {
        // The binding is checked for the whole program, before it runs.
        reject(label, &ready, &tail(&first), false);
    }
    reject("truncated", &ready, &[w0], true);
    reject("reserved SGPR", &ready, &tail(&load(1, 7, 0, NULL)), false);
    reject("reserved SGPRs", &ready, &tail(&load(4, 4, 0, NULL)), false);
    reject(
        "uninitialized destination",
        &[],
        &tail(&load(1, 8, 0, NULL)),
        false,
    );
    reject(
        "half initialized pair",
        &s_mov(8, 1),
        &tail(&load(2, 8, 0, NULL)),
        false,
    );
    reject(
        "uninitialized soffset",
        &ready,
        &tail(&load(1, 8, 0, 30)),
        false,
    );

    let loading = [&ready[..], &load(2, 8, 0, NULL)].concat();
    let mut pending = |label: &str, extra: &[u32]| {
        let mut rest = extra.to_vec();
        rest.extend(finish(2));
        reject(label, &loading, &rest, false);
    };
    pending("vector read before the wait", &[v_mov(2, 8)]);
    pending("second word before the wait", &[v_mov(2, 9)]);
    pending("scalar read before the wait", &[0xbe94_0308]); // s_mov_b32 s20, s8
    pending("compare before the wait", &[0xbf06_0908]); // s_cmp_eq_u32 s8, s9
    pending("write before the wait", &s_mov(9, 3));
    pending("load over a pending load", &load(1, 9, 0, NULL));
    pending("offset from a pending load", &load(1, 12, 0, 8));
    // A vector wait does not complete the scalar load.
    let waited = [&loading[..], &[WAIT_VECTOR]].concat();
    let mut rest = vec![v_mov(2, 8)];
    rest.extend(finish(2));
    reject("vector wait instead", &waited, &rest, false);
    // Termination with an outstanding load.
    let mut prologue = load_lane().to_vec();
    prologue.extend(&loading);
    prologue.extend(&finish(1)[..3]);
    reject("outstanding at the end", &prologue, &[END], false);
    // A run-time branch with an outstanding load.
    let mut prologue = load_lane().to_vec();
    prologue.push(0x7d82_0000 | 1 << 9 | 128); // vcc_lo = 0 < v1
    prologue.extend(&loading);
    let mut rest = vec![0xbf86_0000, WAIT_SCALAR];
    rest.extend(finish(1));
    reject("outstanding at a run-time branch", &prologue, &rest, false);
    out
}

#[test]
fn scalar_buffer_loads_match_the_guide_on_the_reference() {
    let cases = cases();
    assert!(cases.len() > 1500, "{}", cases.len());
    harness::check_reference(&cases).assert_none("the reference disagrees with the guide");
}

#[test]
fn unsupported_scalar_loads_are_refused_at_their_offset() {
    let rejections = rejections();
    assert!(rejections.len() > 35);
    harness::check_rejected(&rejections).assert_none("not refused as expected");
}

/// The compiler cannot lower scalar operations on run-time values yet, so it
/// refuses them. The reference computes them.
#[test]
fn scalar_consumers_of_a_loaded_sgpr_are_refused_by_the_compiler() {
    let mut prologue: Vec<u32> = s_mov(8, 1).to_vec();
    prologue.extend(load(1, 8, 0, NULL));
    prologue.push(WAIT_SCALAR);
    prologue.extend(s_mov(20, 1));
    let consumers: &[(&str, &[u32])] = &[
        ("s_mov_b32 s20, s8", &[0xbe94_0308]),
        ("s_add_u32 s20, s8, s20", &[0x8014_1408]),
        ("s_not_b32 s20, s8", &[0xbe94_0708]),
        ("s_addk_i32 s8, 1", &[0xb788_0001]),
        ("s_mov_b32 exec_lo, s8", &[0xbefe_0308]),
        ("s_buffer_load_dword s20, s[0:3], s8", &load(1, 20, 0, 8)),
    ];
    for (label, consumer) in consumers {
        let mut code = prologue.clone();
        let offset = 4 * code.len();
        code.extend(*consumer);
        code.extend([0xbf8c_c07f, v_mov(2, 20)]);
        code.extend(finish(2));
        match compile(Architecture::Gfx1030, &words(&code)) {
            Err(CompileError::Verify { offset: at, .. }) => assert_eq!(at, offset, "{label}"),
            other => panic!("{label}: {:?}", other.map(|compiled| compiled.msl)),
        }
    }
}

#[test]
#[ignore = "requires a native Metal device with API and shader validation"]
fn scalar_buffer_loads_match_the_guide_on_native_metal() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let cases = cases();
    let (failures, programs) = harness::check_metal(&cases);
    failures.assert_none("native Metal disagrees with the guide");
    eprintln!("smem: {} Metal cases in {programs} programs", cases.len());
}
