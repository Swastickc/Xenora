// SPDX-License-Identifier: GPL-3.0-or-later

//! Independent expected values for run-time uniform branches (#187):
//! `s_cbranch_scc0` on a compare of a loaded SGPR, with and without an `else`
//! part, next to `s_cbranch_vccz`, whose contract has no `else`.
//!
//! The expectations come only from the public AMD "RDNA 2" Instruction Set
//! Architecture Reference Guide: sections 12.2 and 12.4 (which relation sets
//! SCC), 12.5 (branches), 3.9 (VCCZ) and 7.2.1 (the loaded word). Each program
//! applies plain word operations on the path the relation selects; nothing uses
//! the ISA crate, the decoder or the compiler.

#[path = "operand_domains/harness.rs"]
#[allow(dead_code)]
mod harness;

use harness::{Case, Rejection, words};
use xenora_shader::{Architecture, compile};

const WAIT_SCALAR: u32 = 0xbf8c_c07f;
const END: u32 = 0xbf81_0000;
const NULL: u32 = 125 << 25;

fn s_mov(dst: u32, value: u32) -> [u32; 2] {
    [0xbe80_03ff | dst << 16, value]
}

/// s10 = word 0 and s11 = word 1 of the input.
fn load_pair() -> Vec<u32> {
    let mut code: Vec<u32> = Vec::new();
    code.extend(s_mov(10, 0));
    code.extend(s_mov(11, 0));
    code.extend([0x3d << 26 | 9 << 18 | 10 << 6, NULL, WAIT_SCALAR]);
    code
}

/// v1 = the lane's input word, and v2, v3 and v4 = 2, 3 and 4.
fn lanes() -> Vec<u32> {
    vec![
        0xe030_2000,
        0x8000_0100,
        0xbf8c_3f70,
        0x7e04_0282,
        0x7e06_0283,
        0x7e08_0284,
    ]
}

/// v_mov_b32 dst, literal
fn literal(dst: u32, value: u32) -> [u32; 2] {
    [0x7e00_02ff | dst << 17, value]
}

/// v_xor_b32 dst, dst, src
fn xor(dst: u32, src: u32) -> u32 {
    0x3a00_0000 | dst << 17 | src << 9 | (256 + dst)
}

/// v5 = v2 + v3 + v4 (v_add_nc_u32), stored as the result.
fn finish() -> Vec<u32> {
    let add = |dst: u32, a: u32, b: u32| 0x4a00_0000 | dst << 17 | b << 9 | (256 + a);
    vec![
        add(5, 2, 3),
        add(5, 5, 4),
        0xe070_2000,
        0x8001_0500,
        0xbbfd_0000,
        END,
    ]
}

/// SOPP with a word displacement from the next instruction.
fn branch(opcode: u32, skip: usize) -> u32 {
    0xbf80_0000 | opcode << 16 | u32::try_from(skip).unwrap()
}
const BRANCH: u32 = 2;
const SCC0: u32 = 4;
const VCCZ: u32 = 6;

#[derive(Clone, Copy, Debug)]
enum Relation {
    Eq,
    Lg,
    Gt,
    Ge,
    Lt,
    Le,
}
const RELATIONS: [Relation; 6] = [
    Relation::Eq,
    Relation::Lg,
    Relation::Gt,
    Relation::Ge,
    Relation::Lt,
    Relation::Le,
];

/// Guide 12.4: S0 on the left.
fn holds(relation: Relation, signed: bool, a: u32, b: u32) -> bool {
    let value = |x: u32| {
        if signed && x >= 1 << 31 {
            i64::from(x) - (1 << 32)
        } else {
            i64::from(x)
        }
    };
    let difference = value(a) - value(b);
    match relation {
        Relation::Eq => difference == 0,
        Relation::Lg => difference != 0,
        Relation::Gt => difference > 0,
        Relation::Ge => difference >= 0,
        Relation::Lt => difference < 0,
        Relation::Le => difference <= 0,
    }
}

const THEN: u32 = 0x1111_0000;
const ELSE: u32 = 0x0222_2000;

/// The parts of a region: what they do to v2, v3 and v4 given the input word.
struct Parts {
    then: Vec<u32>,
    otherwise: Option<Vec<u32>>,
}

/// The fall-through part: v2 = THEN, v3 ^= x.
fn then_part() -> Vec<u32> {
    let mut code = literal(2, THEN).to_vec();
    code.push(xor(3, 1));
    code
}

/// The `else` part: v4 = ELSE, v3 ^= v2 (the value from before the branch).
fn else_part() -> Vec<u32> {
    let mut code = literal(4, ELSE).to_vec();
    code.push(xor(3, 2));
    code
}

fn region(opcode: u32, parts: &Parts) -> Vec<u32> {
    let mut then = parts.then.clone();
    if let Some(otherwise) = &parts.otherwise {
        then.push(branch(BRANCH, otherwise.len()));
    }
    let mut code = vec![branch(opcode, then.len())];
    code.extend(then);
    code.extend(parts.otherwise.iter().flatten());
    code
}

/// The sum the program stores for input `x` when the branch is or is not taken.
fn result(taken: bool, has_else: bool, x: u32) -> u32 {
    let [mut v2, mut v3, mut v4] = [2u32, 3, 4];
    if !taken {
        v2 = THEN;
        v3 ^= x;
    } else if has_else {
        v4 = ELSE;
        v3 ^= v2;
    }
    v2.wrapping_add(v3).wrapping_add(v4)
}

const EDGES: [u32; 7] = [0, 1, 2, 0x7fff_ffff, 0x8000_0000, 0xffff_8000, u32::MAX];

fn push(cases: &mut Vec<Case>, label: String, code: &[u32], input: Vec<u32>, expected: Vec<u32>) {
    cases.push(Case {
        form: "runtime branch",
        family: "branch",
        label,
        code: words(code),
        input,
        expected,
    });
}

fn input(first: u32, second: u32) -> Vec<u32> {
    let mut input = vec![first, second];
    input.extend((0..38u32).map(|n| 0x9e37_79b9u32.wrapping_mul(n + 1)));
    input
}

fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for has_else in [false, true] {
        let parts = Parts {
            then: then_part(),
            otherwise: has_else.then(else_part),
        };
        for (index, relation) in RELATIONS.into_iter().enumerate() {
            for signed in [true, false] {
                for a in EDGES {
                    for b in EDGES {
                        // SOPC: s_cmp_<relation>_<i32|u32> s10, s11.
                        let opcode = index as u32 + if signed { 0 } else { 6 };
                        let mut code = load_pair();
                        code.extend(lanes());
                        code.push(0xbf00_0000 | opcode << 16 | 11 << 8 | 10);
                        code.extend(region(SCC0, &parts));
                        code.extend(finish());
                        let input = input(a, b);
                        let taken = !holds(relation, signed, a, b);
                        let expected = input.iter().map(|x| result(taken, has_else, *x)).collect();
                        push(
                            &mut cases,
                            format!("sopc {relation:?} {signed} {a:#x} {b:#x} else {has_else}"),
                            &code,
                            input,
                            expected,
                        );
                    }
                    // SOPK: s_cmpk_<relation>_<i32|u32> s10, simm16.
                    for simm16 in [0u16, 1, 0x7fff, 0x8000, 0xffff] {
                        let opcode = index as u32 + if signed { 3 } else { 9 };
                        let mut code = load_pair();
                        code.extend(lanes());
                        code.push(0xb000_0000 | opcode << 23 | 10 << 16 | u32::from(simm16));
                        code.extend(region(SCC0, &parts));
                        code.extend(finish());
                        let constant = if signed {
                            simm16 as i16 as i32 as u32
                        } else {
                            u32::from(simm16)
                        };
                        let input = input(a, 7);
                        let taken = !holds(relation, signed, a, constant);
                        let expected = input.iter().map(|x| result(taken, has_else, *x)).collect();
                        push(
                            &mut cases,
                            format!(
                                "sopk {relation:?} {signed} {a:#x} {simm16:#x} else {has_else}"
                            ),
                            &code,
                            input,
                            expected,
                        );
                    }
                }
            }
        }
        // The loaded value on either side of an inline constant and a literal.
        for a in EDGES {
            for (label, word, literal, reversed, constant) in [
                ("s10 > 2", 0xbf08_820a, None, false, 2u32),
                ("2 > s10", 0xbf08_0a82, None, true, 2),
                (
                    "s10 > lit",
                    0xbf08_ff0a,
                    Some(0x7fff_fffeu32),
                    false,
                    0x7fff_fffe,
                ),
                (
                    "lit > s10",
                    0xbf08_0aff,
                    Some(0x7fff_fffe),
                    true,
                    0x7fff_fffe,
                ),
            ] {
                let mut code = load_pair();
                code.extend(lanes());
                code.push(word); // s_cmp_gt_u32
                code.extend(literal);
                code.extend(region(SCC0, &parts));
                code.extend(finish());
                let input = input(a, 7);
                let taken = if reversed {
                    constant <= a
                } else {
                    a <= constant
                };
                let expected = input.iter().map(|x| result(taken, has_else, *x)).collect();
                push(
                    &mut cases,
                    format!("{label} {a:#x} else {has_else}"),
                    &code,
                    input,
                    expected,
                );
            }
        }
    }
    let parts = Parts {
        then: then_part(),
        otherwise: None,
    };
    // VCCZ: each wave of 32 takes its own path.
    for limit in [0u32, 0x10_0000, 0xfff0_0000, u32::MAX] {
        let mut code = lanes();
        code.extend(literal(6, limit));
        code.push(0x7d88_0000 | 6 << 9 | 257); // v_cmp_gt_u32 vcc_lo, v1, v6
        code.extend(region(VCCZ, &parts));
        code.extend(finish());
        let input: Vec<u32> = (0..100u32)
            .map(|n| {
                if n < 32 {
                    n
                } else {
                    0x9e37_79b9u32.wrapping_mul(n)
                }
            })
            .collect();
        let expected = input
            .chunks(32)
            .flat_map(|wave| {
                let taken = wave.iter().all(|x| *x <= limit);
                wave.iter().map(move |x| result(taken, false, *x))
            })
            .collect();
        push(
            &mut cases,
            format!("vccz {limit:#x}"),
            &code,
            input,
            expected,
        );
    }
    other_shapes(&mut cases);
    cases
}

fn other_shapes(cases: &mut Vec<Case>) {
    for first in [0u32, 9, 10, u32::MAX] {
        let input = input(first, 3);
        let expected = input
            .iter()
            .map(|x| {
                if first > 9 {
                    x.wrapping_add(100)
                } else {
                    x ^ 0xff
                }
            })
            .collect();
        cases.push(Case {
            form: "runtime branch",
            family: "fixture",
            label: format!("fixture {first}"),
            code: include_bytes!("../../../fixtures/rdna2/runtime-branch.bin").to_vec(),
            input,
            expected,
        });
    }
    for a in [0u32, 1, 5, 6] {
        let input_words = input(a, 5);
        // Two branches in sequence; the second sees the joined registers and
        // compares again.
        let parts = Parts {
            then: then_part(),
            otherwise: Some(else_part()),
        };
        let mut code = load_pair();
        code.extend(lanes());
        code.push(0xbf06_0b0a); // s_cmp_eq_u32 s10, s11
        code.extend(region(SCC0, &parts));
        code.push(0xbf06_810a); // s_cmp_eq_u32 s10, 1
        code.extend(region(
            SCC0,
            &Parts {
                then: vec![xor(2, 3)],
                otherwise: Some(vec![xor(4, 3)]),
            },
        ));
        code.extend(finish());
        let expected = input_words
            .iter()
            .map(|x| {
                let [mut v2, mut v3, mut v4] = [2u32, 3, 4];
                if a == 5 {
                    v2 = THEN;
                    v3 ^= x;
                } else {
                    v4 = ELSE;
                    v3 ^= v2;
                }
                if a == 1 {
                    v2 ^= v3;
                } else {
                    v4 ^= v3;
                }
                v2.wrapping_add(v3).wrapping_add(v4)
            })
            .collect();
        push(
            cases,
            format!("two branches {a}"),
            &code,
            input_words.clone(),
            expected,
        );

        // Empty parts: no then part, no else part, neither.
        for (then, otherwise, label) in [
            (vec![], Some(else_part()), "empty then"),
            (then_part(), Some(vec![]), "empty else"),
            (vec![], Some(vec![]), "both empty"),
            (vec![], None, "empty region"),
        ] {
            let mut code = load_pair();
            code.extend(lanes());
            code.push(0xbf06_850a); // s_cmp_eq_u32 s10, 5
            code.extend(region(
                SCC0,
                &Parts {
                    then: then.clone(),
                    otherwise: otherwise.clone(),
                },
            ));
            code.extend(finish());
            let expected = input_words
                .iter()
                .map(|x| {
                    let [mut v2, mut v3, mut v4] = [2u32, 3, 4];
                    if a == 5 && !then.is_empty() {
                        v2 = THEN;
                        v3 ^= x;
                    }
                    if a != 5 && otherwise.as_ref().is_some_and(|part| !part.is_empty()) {
                        v4 = ELSE;
                        v3 ^= v2;
                    }
                    v2.wrapping_add(v3).wrapping_add(v4)
                })
                .collect();
            push(
                cases,
                format!("{label} {a}"),
                &code,
                input_words.clone(),
                expected,
            );
        }

        // Under partial EXEC only active lanes are written, on either path.
        for exec in [0u32, 0x5555_5555, 1] {
            let mut code = load_pair();
            code.extend(lanes());
            code.extend(s_mov(126, exec));
            code.push(0xbf06_850a);
            code.extend(region(SCC0, &parts));
            code.extend(s_mov(126, u32::MAX));
            code.extend(finish());
            let expected = input_words
                .iter()
                .enumerate()
                .map(|(lane, x)| {
                    if exec >> (lane % 32) & 1 == 1 {
                        result(a != 5, true, *x)
                    } else {
                        9
                    }
                })
                .collect();
            push(
                cases,
                format!("exec {exec:#x} {a}"),
                &code,
                input_words.clone(),
                expected,
            );
        }

        // SCC made known again after a run-time compare: an ordinary branch.
        let mut code = load_pair();
        code.extend(lanes());
        code.push(0xbf06_0b0a);
        code.push(0xbf06_8181); // s_cmp_eq_u32 1, 1
        code.push(branch(SCC0, 2));
        code.extend(literal(2, 100));
        code.extend(finish());
        push(
            cases,
            format!("known again {a}"),
            &code,
            input_words.clone(),
            vec![107; 40],
        );

        // A run-time branch that an earlier known branch skips.
        let mut code = load_pair();
        code.extend(lanes());
        code.push(0xbf06_0b0a);
        let skipped = region(SCC0, &parts);
        code.push(branch(BRANCH, skipped.len()));
        code.extend(skipped);
        code.extend(finish());
        push(
            cases,
            format!("skipped {a}"),
            &code,
            input_words.clone(),
            vec![9; 40],
        );
    }
}

fn rejections() -> Vec<Rejection> {
    let mut out = Vec::new();
    let mut reject = |label: &str, prologue: &[u32], rest: &[u32]| {
        let mut code = [prologue, rest].concat();
        code.extend(finish());
        out.push(Rejection {
            family: "rejection",
            truncated: false,
            label: label.into(),
            offset: 4 * prologue.len(),
            code: words(&code),
        });
    };
    let mut ready = load_pair();
    ready.extend(lanes());
    ready.push(0xbf06_0b0a);
    // Instructions outside the region contract, in the then part and in the else
    // part.
    let bad: &[(&str, &[u32])] = &[
        ("scalar move", &[0xbe94_0381]),
        ("scalar compare", &[0xbf06_8181]),
        ("vector compare", &[0x7d88_0681]),
        ("wait", &[0xbf8c_3f70]),
        ("scalar load", &[0x3d << 26 | 8 << 18 | 20 << 6, NULL]),
        ("vector load", &[0xe030_2000, 0x8000_0600]),
        ("store", &[0xe070_2000, 0x8001_0200]),
        ("nested scc0", &[0xbf84_0000]),
        ("nested vccz", &[0xbf86_0000]),
        ("uninitialized write", &[0x7e10_0280]),
        ("uninitialized read", &[0x7e04_0308]),
        ("index register", &[0x7e00_0280]),
    ];
    for (label, code) in bad {
        let mut prologue = ready.clone();
        prologue.push(branch(SCC0, code.len() + 1));
        reject(
            &format!("{label} in then"),
            &prologue,
            &[code, &[0x7e04_0281][..]].concat(),
        );
        let mut prologue = ready.clone();
        prologue.extend([branch(SCC0, 2), 0x7e04_0281, branch(BRANCH, code.len() + 1)]);
        reject(
            &format!("{label} in else"),
            &prologue,
            &[code, &[0x7e04_0281][..]].concat(),
        );
    }
    // An `s_branch` that does not end the then part, and one in the else part.
    let mut prologue = ready.clone();
    prologue.push(branch(SCC0, 2));
    reject(
        "early s_branch",
        &prologue,
        &[branch(BRANCH, 1), 0x7e04_0281, 0x7e04_0282],
    );
    let mut prologue = ready.clone();
    prologue.extend([branch(SCC0, 2), 0x7e04_0281, branch(BRANCH, 2), 0x7e04_0283]);
    reject("s_branch in else", &prologue, &[branch(BRANCH, 0)]);
    // `s_cbranch_vccz` takes no `else` part.
    let mut prologue = lanes();
    prologue.extend([0x7d88_0681, branch(VCCZ, 2), 0x7e04_0281]);
    reject("vccz else", &prologue, &[branch(BRANCH, 1), 0x7e04_0283]);
    // Outstanding loads at the branch.
    let mut prologue = ready.clone();
    prologue.extend([0xe030_2000, 0x8000_0600]);
    reject(
        "outstanding vector load",
        &prologue,
        &[branch(SCC0, 1), 0x7e04_0281, 0xbf8c_3f70],
    );
    let mut prologue = ready.clone();
    prologue.extend(s_mov(20, 0));
    prologue.extend([0x3d << 26 | 8 << 18 | 20 << 6, NULL]);
    reject(
        "outstanding scalar load",
        &prologue,
        &[branch(SCC0, 1), 0x7e04_0281, WAIT_SCALAR],
    );
    // A compare of an SGPR that is still loading.
    let mut prologue = lanes();
    prologue.extend(s_mov(10, 0));
    prologue.extend([0x3d << 26 | 8 << 18 | 10 << 6, NULL]);
    reject("compare before the wait", &prologue, &[0xbf06_810a]);
    reject(
        "immediate compare before the wait",
        &prologue,
        &[0xb480_0000 | 10 << 16],
    );
    out
}

#[test]
fn runtime_branches_match_the_guide_on_the_reference() {
    let cases = cases();
    assert!(cases.len() > 1800, "{}", cases.len());
    harness::check_reference(&cases).assert_none("the reference disagrees with the guide");
}

#[test]
fn code_outside_the_region_contract_is_refused_at_its_offset() {
    let rejections = rejections();
    assert!(rejections.len() > 28);
    harness::check_rejected(&rejections).assert_none("not refused as expected");
}

/// A backward branch stays refused: there are no loops (#188).
#[test]
fn backward_and_misaligned_targets_are_refused() {
    let mut ready = load_pair();
    ready.extend(lanes());
    ready.push(0xbf06_0b0a);
    for (label, tail) in [
        ("backward scc0", vec![0xbf84_ffff]),
        ("backward else", vec![branch(SCC0, 1), 0xbf82_fffe]),
        // Into the second word of the literal.
        ("into a literal", vec![branch(SCC0, 1), 0x7e04_02ff, 7]),
        (
            "else into a literal",
            vec![branch(SCC0, 1), branch(BRANCH, 1), 0x7e04_02ff, 7],
        ),
        ("past the end", vec![branch(SCC0, 200)]),
    ] {
        let mut code = ready.clone();
        code.extend(tail);
        code.extend(finish());
        let code = words(&code);
        assert!(compile(Architecture::Gfx1030, &code).is_err(), "{label}");
        assert!(xenora_ref::execute(&code, &[0, 1]).is_err(), "{label}");
    }
}

#[test]
#[ignore = "requires a native Metal device with API and shader validation"]
fn runtime_branches_match_the_guide_on_native_metal() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let cases = cases();
    let (failures, programs) = harness::check_metal(&cases);
    failures.assert_none("native Metal disagrees with the guide");
    eprintln!(
        "runtime branch: {} Metal cases in {programs} programs",
        cases.len()
    );
}
