// SPDX-License-Identifier: GPL-3.0-or-later

//! Wave32 VOP3A integer ALU forms. AMD RDNA 2 ISA guide §§12.7, 12.7.1,
//! 12.12 and 13.3.4. The expectations use the guide's arithmetic per lane.

#[path = "operand_domains/harness.rs"]
#[allow(dead_code)]
mod harness;
#[path = "../independent_ref.rs"]
mod independent_ref;

use harness::{Case, words};

const FORMS: [(u32, &str); 11] = [
    (265, "v_mul_i32_i24"),
    (266, "v_mul_hi_i32_i24"),
    (267, "v_mul_u32_u24"),
    (268, "v_mul_hi_u32_u24"),
    (273, "v_min_i32"),
    (274, "v_max_i32"),
    (275, "v_min_u32"),
    (276, "v_max_u32"),
    (286, "v_xnor_b32"),
    (294, "v_sub_nc_u32"),
    (295, "v_subrev_nc_u32"),
];
const VALUES: [u32; 12] = [
    0,
    1,
    2,
    0x007f_ffff,
    0x0080_0000,
    0x0080_0001,
    0x00ff_ffff,
    0xff80_0000,
    0xff00_0003,
    0x7fff_ffff,
    0x8000_0000,
    u32::MAX,
];
const MASKS: [u32; 4] = [u32::MAX, 0x5555_5555, 0x8000_0001, 0];

fn expected(op: u32, a: u32, b: u32) -> u32 {
    // S.i24 is bits 23:0 sign-extended from bit 23; S.u24 is zero-extended.
    let i24 = |x: u32| (i64::from(x & 0x7f_ffff)) - i64::from(x & 0x80_0000);
    let u24 = |x: u32| i64::from(x & 0xff_ffff);
    match op {
        // D.i32 = S0.i24 * S1.i24.
        265 => (i24(a) * i24(b)) as u32,
        // D.i32 = (S0.i24 * S1.i24)>>32.
        266 => (i24(a) * i24(b)).div_euclid(1 << 32) as u32,
        // D.u32 = S0.u24 * S1.u24.
        267 => (u24(a) * u24(b)) as u32,
        // D.u32 = (S0.u24 * S1.u24)>>32.
        268 => (u24(a) * u24(b)).div_euclid(1 << 32) as u32,
        // D.i32 = (S0.i32 < S1.i32 ? S0.i32 : S1.i32).
        273 => (a as i32).min(b as i32) as u32,
        // D.i32 = (S0.i32 >= S1.i32 ? S0.i32 : S1.i32).
        274 => (a as i32).max(b as i32) as u32,
        // D.u32 = (S0.u32 < S1.u32 ? S0.u32 : S1.u32).
        275 => a.min(b),
        // D.u32 = (S0.u32 >= S1.u32 ? S0.u32 : S1.u32).
        276 => a.max(b),
        // D.u32 = ~(S0.u32 ^ S1.u32).
        286 => !(a ^ b),
        // D.u32 = S0.u32 - S1.u32.
        294 => (i64::from(a) - i64::from(b)) as u32,
        // D.u32 = S1.u32 - S0.u32.
        295 => (i64::from(b) - i64::from(a)) as u32,
        _ => unreachable!("test only uses the listed forms"),
    }
}

/// VOP3A: OP 25:16, VDST 7:0, three 9-bit sources, inline zero at SRC2.
fn vop3(op: u32, dst: u32, a: u32, b: u32) -> [u32; 2] {
    [
        0xd400_0000 | op << 16 | dst,
        (256 + a) | (256 + b) << 9 | 128 << 18,
    ]
}

fn vmov(dst: u32, value: u32) -> [u32; 2] {
    [0x7e00_02ff | dst << 17, value]
}

fn smov(dst: u32, value: u32) -> [u32; 2] {
    [0xbe80_03ff | dst << 16, value]
}

fn setup(b: u32, exec: u32, scc: bool) -> Vec<u32> {
    let mut code = vec![0xe030_2000, 0x8000_0100, 0xbf8c_3f70]; // load v1
    code.extend(vmov(255, b));
    code.extend(vmov(2, 0xabad_1dea));
    code.extend(smov(106, 0x1357_9bdf));
    code.push(if scc { 0xbf06_8080 } else { 0xbf06_8180 });
    code.extend(smov(126, exec));
    code
}

fn finish(mut code: Vec<u32>, dst: u32) -> Vec<u8> {
    // Restore EXEC, read D and VCC_LO, and XOR in v1 when SCC is set.
    code.extend([
        0xbefe_03c1,
        0x7e04_0300 | dst,
        0x7e0c_026a,
        0x3a04_0506,
        0xbf84_0001,
        0x3a04_0501,
        0xe070_2000,
        0x8001_0200,
        0xbbfd_0000,
        0xbf81_0000,
    ]);
    words(&code)
}

fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for (op, name) in FORMS {
        for b in VALUES {
            for (dst, src0, src1) in [
                (2, 1, 255),
                (2, 255, 1),
                (1, 1, 255),
                (255, 1, 255),
                (255, 255, 255),
            ] {
                for exec in MASKS {
                    for lanes in [7, 32, 45] {
                        let scc = dst == 255;
                        let mut code = setup(b, exec, scc);
                        code.extend(vop3(op, dst, src0, src1));
                        let input: Vec<_> = VALUES.into_iter().cycle().take(lanes).collect();
                        let expected = input
                            .iter()
                            .enumerate()
                            .map(|(lane, &x)| {
                                let read = |reg| if reg == 1 { x } else { b };
                                // Guide §3.3: a disabled lane retains its previous VGPR.
                                let d = if exec & (1 << (lane % 32)) != 0 {
                                    expected(op, read(src0), read(src1))
                                } else {
                                    match dst {
                                        1 => x,
                                        255 => b,
                                        _ => 0xabad_1dea,
                                    }
                                };
                                d ^ 0x1357_9bdf ^ if scc { x } else { 0 }
                            })
                            .collect();
                        cases.push(Case {
                            form: name,
                            family: "vop3 integer",
                            label: format!("{name} v{dst}, v{src0}, v{src1} ({b:#x}), EXEC={exec:#x}, {lanes} lanes"),
                            code: finish(code, dst),
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
fn vop3_integer_matches_the_guide() {
    let cases = cases();
    assert_eq!(cases.len(), 11 * 12 * 5 * 4 * 3);
    for case in &cases {
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
    eprintln!("vop3_integer: {} cases per CPU interpreter", cases.len());
}

#[test]
fn vop3_integer_rejects_excluded_operands_and_modifiers() {
    let mut count = 0;
    for (op, name) in FORMS {
        let setup = setup(7, u32::MAX, false);
        let offset = setup.len() * 4;
        let [head, tail] = vop3(op, 2, 1, 255);
        let mut check = |alu: &[u32]| {
            let mut code = setup.clone();
            code.extend(alu);
            let code = if alu.len() == 1 {
                words(&code)
            } else {
                finish(code, 2)
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
        check(&[head]);
    }
    assert_eq!(count, 11 * (13 + 512 + 511 + 1));
    eprintln!("vop3_integer: {count} rejections per CPU interpreter");
}

#[test]
#[ignore = "requires a native Metal device with API and shader validation"]
fn vop3_integer_matches_the_guide_on_native_metal() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let cases = cases();
    let (failures, programs) = harness::check_metal(&cases);
    failures.assert_none("native Metal disagrees with the guide");
    eprintln!(
        "vop3_integer: {} Metal cases in {programs} programs",
        cases.len()
    );
}
