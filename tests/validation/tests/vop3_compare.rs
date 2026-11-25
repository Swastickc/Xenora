// SPDX-License-Identifier: GPL-3.0-or-later

//! Wave32 VOP3A integer compares. Guide §3.9 permits an SGPR destination in
//! the 64-bit encoding and describes the fully rewritten per-lane mask.
//! A Wave32 mask has 32 bits (§2.1), so SDST consumes one 32-bit SGPR (§5.2).
//! The opcode relations are in §12.9, the re-encoding in §12.9.1, and the
//! VOP3A fields in §13.3.4. These are spec-derived expectations.

#[path = "operand_domains/harness.rs"]
#[allow(dead_code)]
mod harness;
#[path = "../independent_ref.rs"]
mod independent_ref;

use harness::{Case, words};

const FORMS: [(u32, &str); 16] = [
    (128, "v_cmp_f_i32"),
    (129, "v_cmp_lt_i32"),
    (130, "v_cmp_eq_i32"),
    (131, "v_cmp_le_i32"),
    (132, "v_cmp_gt_i32"),
    (133, "v_cmp_ne_i32"),
    (134, "v_cmp_ge_i32"),
    (135, "v_cmp_t_i32"),
    (192, "v_cmp_f_u32"),
    (193, "v_cmp_lt_u32"),
    (194, "v_cmp_eq_u32"),
    (195, "v_cmp_le_u32"),
    (196, "v_cmp_gt_u32"),
    (197, "v_cmp_ne_u32"),
    (198, "v_cmp_ge_u32"),
    (199, "v_cmp_t_u32"),
];
const VALUES: [u32; 10] = [
    0,
    1,
    0x7fff_ffff,
    0x8000_0000,
    u32::MAX,
    0x00ff_ffff,
    0x0080_0000,
    0xff80_0000,
    0x0100_0001,
    0xff00_0003,
];
const MASKS: [u32; 4] = [u32::MAX, 0x5555_5555, 0x8000_0001, 0];

fn passes(op: u32, a: u32, b: u32) -> bool {
    let operand = |value: u32| {
        if op < 192 {
            i64::from(value as i32)
        } else {
            i64::from(value)
        }
    };
    let (a, b) = (operand(a), operand(b));
    match op & 7 {
        // D[threadId] = 0.
        0 => false,
        // D[threadId] = (S0 < S1).
        1 => a < b,
        // D[threadId] = (S0 == S1).
        2 => a == b,
        // D[threadId] = (S0 <= S1).
        3 => a <= b,
        // D[threadId] = (S0 > S1).
        4 => a > b,
        // D[threadId] = (S0 <> S1).
        5 => a != b,
        // D[threadId] = (S0 >= S1).
        6 => a >= b,
        // D[threadId] = 1.
        _ => true,
    }
}

fn expected_mask(op: u32, exec: u32, wave: &[u32], bound: u32, reverse: bool) -> u32 {
    // §3.9: VCC[n] = EXEC[n] & (test passed for thread[n]).
    // §3.9 uses the same result for the alternate scalar destination.
    wave.iter().enumerate().fold(0, |mask, (n, &x)| {
        let (a, b) = if reverse { (bound, x) } else { (x, bound) };
        mask | (exec & (u32::from(passes(op, a, b)) << n))
    })
}

fn vop3(op: u32, dst: u32, reverse: bool) -> [u32; 2] {
    let (a, b) = if reverse { (511, 257) } else { (257, 511) };
    [0xd400_0000 | op << 16 | dst, a | b << 9 | 128 << 18]
}

fn smov(dst: u32, value: u32) -> [u32; 2] {
    [0xbe80_03ff | dst << 16, value]
}

fn setup(bound: u32, exec: u32) -> Vec<u32> {
    let mut code = vec![0xe030_2000, 0x8000_0100, 0xbf8c_3f70]; // load v1
    code.extend([0x7ffe_02ff, bound]); // v255 = literal
    code.extend(smov(126, exec));
    code
}

fn store(mut code: Vec<u32>) -> Vec<u8> {
    code.extend([0xe070_2000, 0x8001_0200, 0xbbfd_0000, 0xbf81_0000]);
    words(&code)
}

fn mask_cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for (op, name) in FORMS {
        for bound in VALUES {
            for reverse in [false, true] {
                for exec in MASKS {
                    for (lanes, dst) in [(7, 106), (32, 106), (45, 106), (45, 0), (45, 105)] {
                        let mut code = setup(bound, exec);
                        // Give the old VCC bits on both sides of EXEC.
                        code.extend(smov(106, 0xa5a5_5a5a));
                        code.extend(vop3(op, dst, reverse));
                        code.extend([0xbefe_03c1, 0x7e04_0200 | dst]);
                        let input: Vec<_> = VALUES.into_iter().cycle().take(lanes).collect();
                        let expected = input
                            .chunks(32)
                            .flat_map(|wave| {
                                let mask = expected_mask(op, exec, wave, bound, reverse);
                                std::iter::repeat_n(mask, wave.len())
                            })
                            .collect();
                        cases.push(Case {
                            form: name,
                            family: if dst == 106 { "vcc_lo" } else { "sgpr" },
                            label: format!("{name} s{dst}, bound={bound:#x}, reverse={reverse}, EXEC={exec:#x}, {lanes} lanes"),
                            code: store(code),
                            input,
                            expected,
                        });
                    }
                }
            }
        }
    }
    cases
}

fn branch_cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for (op, name) in FORMS {
        for dst in [0, 105, 106] {
            for exec in MASKS {
                for old_vcc in [0, 0xa5a5_5a5a] {
                    for nonzero in [false, true] {
                        let scc = old_vcc != 0;
                        let mut code = setup(0, u32::MAX);
                        // Seed s1 to detect an accidental second SGPR write to s0:s1.
                        code.extend(vop3(199, 1, false));
                        code.extend(smov(106, old_vcc));
                        code.push(if scc { 0xbf06_8080 } else { 0xbf06_8180 });
                        code.extend([0x7e04_0280, 0x7e08_0280, 0x7e0c_0284]); // v2 = v4 = 0, v6 = 4
                        code.extend(smov(126, exec));
                        code.extend(vop3(op, dst, false));
                        code.extend([
                            0x7e08_0281, // v4 = 1 only where EXEC was set
                            0xbefe_03c1,
                            // Independent bits for the VCC edge, SCC, and EXEC.
                            if nonzero { 0xbf87_0001 } else { 0xbf86_0001 },
                            0x7e04_0282, // v2 = 2 on fallthrough
                            0xbf84_0001,
                            0x3a04_0d02, // v2 ^= 4 if SCC = 1
                            0x3a04_0902, // v2 ^= v4
                            0x7e06_0201,
                            0x3a04_0702, // v2 ^= s1
                            0x7e06_026a,
                            0x3a04_0702, // v2 ^= VCC_LO
                        ]);
                        // The two waves give opposite results for several relations.
                        let input: Vec<_> = std::iter::repeat_n(1, 32)
                            .chain(std::iter::repeat_n(u32::MAX, 13))
                            .collect();
                        let mut expected = Vec::new();
                        for wave in input.chunks(32) {
                            let mask = expected_mask(op, exec, wave, 0, false);
                            let vcc = if dst == 106 { mask } else { old_vcc };
                            // §12.5: if (VCC == 0) / if (VCC != 0), PC = PC + signext(SIMM16 * 4) + 4.
                            let taken = (vcc != 0) == nonzero;
                            let s1 = u32::MAX >> (32 - wave.len());
                            let marker = (u32::from(scc) << 2) | (u32::from(!taken) << 1);
                            expected.extend(
                                wave.iter()
                                    .enumerate()
                                    .map(|(n, _)| marker ^ ((exec >> n) & 1) ^ s1 ^ vcc),
                            );
                        }
                        cases.push(Case {
                            form: name,
                            family: "vcc branch and flags",
                            label: format!(
                                "{name} s{dst}, EXEC={exec:#x}, VCC={old_vcc:#x}, NZ={nonzero}"
                            ),
                            code: store(code),
                            input,
                            expected,
                        });
                    }
                }
            }
        }
    }
    cases
}

#[test]
fn vop3_compare_matches_the_guide() {
    let masks = mask_cases();
    let branches = branch_cases();
    assert_eq!(masks.len(), 16 * 10 * 2 * 4 * 5);
    assert_eq!(branches.len(), 16 * 3 * 4 * 2 * 2);
    for case in masks.iter().chain(&branches) {
        assert_eq!(
            xenora_ref::execute(&case.code, &case.input).unwrap(),
            case.expected,
            "reference: {}",
            case.label
        );
        assert_eq!(
            independent_ref::execute(&case.code, &case.input).unwrap(),
            case.expected,
            "independent: {}",
            case.label
        );
    }
    eprintln!(
        "vop3_compare: {} mask and {} branch/flag cases per CPU interpreter",
        masks.len(),
        branches.len()
    );
}

#[test]
fn vop3_compare_rejects_excluded_operands_and_modifiers() {
    let mut count = 0;
    for (op, name) in FORMS {
        let setup = setup(7, u32::MAX);
        let offset = setup.len() * 4;
        let [head, tail] = vop3(op, 106, false);
        let mut check = |alu: &[u32]| {
            let mut code = setup.clone();
            code.extend(alu);
            let code = if alu.len() == 1 {
                words(&code)
            } else {
                // A successful decode would read its mask here and complete.
                code.push(0x7e04_026a);
                store(code)
            };
            let input = [1, 2];
            let err = xenora_ref::execute(&code, &input).expect_err(name);
            assert_eq!(err.offset, offset, "{name}: {alu:08x?} {err:?}");
            let err = independent_ref::execute(&code, &input).expect_err(name);
            assert!(
                err.starts_with(&format!("{offset:#x}:")),
                "{name}: {alu:08x?} {err}"
            );
            count += 1;
        };
        // ABS, OP_SEL and CLAMP in word 0. OMOD and NEG in word 1.
        for bit in 8..16 {
            check(&[head | 1 << bit, tail]);
        }
        for bit in 27..32 {
            check(&[head, tail | 1 << bit]);
        }
        for selector in 0..256 {
            check(&[head, (tail & !511) | selector]);
            check(&[head, (tail & !(511 << 9)) | selector << 9]);
        }
        for selector in (0..512).filter(|&s| s != 128) {
            check(&[head, (tail & !(511 << 18)) | selector << 18]);
        }
        for dst in 107..=255 {
            check(&[(head & !255) | dst, tail]);
        }
        check(&[head]);
    }
    assert_eq!(count, 16 * (13 + 512 + 511 + 149 + 1));
    eprintln!("vop3_compare: {count} rejections per CPU interpreter");
}

#[test]
#[ignore = "requires a native Metal device with API and shader validation"]
fn vop3_vcc_compare_matches_the_guide_on_native_metal() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let cases: Vec<_> = mask_cases()
        .into_iter()
        .filter(|case| case.family == "vcc_lo")
        .collect();
    let (failures, programs) = harness::check_metal(&cases);
    failures.assert_none("native Metal disagrees with the guide");
    eprintln!(
        "vop3_compare: {} Metal cases in {programs} programs",
        cases.len()
    );
}
