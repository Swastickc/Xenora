// SPDX-License-Identifier: GPL-3.0-or-later

//! CFG-aware probe regression tests for Xenora X16.
//!
//! These tests verify that the control-flow-aware disassembly only reports
//! instructions that are actually reachable from the entry point, rather
//! than linearly sweeping through unreachable padding bytes.

use xenora_loader::probe::{ProbeConfig, probe_pe_bytes};

/// Minimal PE32+ builder for CFG tests.
/// Returns a 4096-byte file with one executable .text section at RVA 0x1000,
/// raw offset 0x200, raw size 0x200.
struct PeBuilder {
    bytes: Vec<u8>,
    text_start: usize, // raw offset of .text
}

impl PeBuilder {
    fn new() -> Self {
        let mut pe = vec![0u8; 4096];
        // MZ header
        pe[0] = b'M';
        pe[1] = b'Z';
        pe[0x3c] = 0x80; // e_lfanew = 0x80

        let pe_sig = 0x80;
        pe[pe_sig..pe_sig + 4].copy_from_slice(b"PE\0\0");

        let coff = pe_sig + 4;
        pe[coff..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes()); // AMD64
        pe[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes()); // 1 section
        pe[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes()); // opt header size

        let opt = coff + 20;
        pe[opt..opt + 2].copy_from_slice(&0x020bu16.to_le_bytes()); // PE32+
        pe[opt + 16..opt + 20].copy_from_slice(&0x1000u32.to_le_bytes()); // entry RVA
        pe[opt + 24..opt + 32].copy_from_slice(&0x140000000u64.to_le_bytes()); // image base
        pe[opt + 32..opt + 36].copy_from_slice(&0x1000u32.to_le_bytes()); // section align
        pe[opt + 36..opt + 40].copy_from_slice(&0x200u32.to_le_bytes()); // file align
        pe[opt + 56..opt + 60].copy_from_slice(&0x2000u32.to_le_bytes()); // size of image
        pe[opt + 60..opt + 64].copy_from_slice(&0x400u32.to_le_bytes()); // size of headers
        pe[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes()); // num data dirs

        // Section header: .text — RVA 0x1000, raw offset 0x200, raw size 0x200
        let sec = opt + 240;
        pe[sec..sec + 5].copy_from_slice(b".text");
        pe[sec + 8..sec + 12].copy_from_slice(&0x200u32.to_le_bytes()); // virtual size
        pe[sec + 12..sec + 16].copy_from_slice(&0x1000u32.to_le_bytes()); // virtual addr
        pe[sec + 16..sec + 20].copy_from_slice(&0x200u32.to_le_bytes()); // raw size
        pe[sec + 20..sec + 24].copy_from_slice(&0x200u32.to_le_bytes()); // raw ptr
        // Characteristics: CNT_CODE | MEM_EXECUTE | MEM_READ
        pe[sec + 36..sec + 40].copy_from_slice(&0x60000020u32.to_le_bytes());

        PeBuilder {
            bytes: pe,
            text_start: 0x200,
        }
    }

    /// Write instruction bytes at an offset within the .text section.
    /// `text_off` is relative to the start of the .text section (raw offset 0x200).
    fn write_text(&mut self, text_off: usize, insns: &[u8]) {
        let start = self.text_start + text_off;
        self.bytes[start..start + insns.len()].copy_from_slice(insns);
    }

    fn entry(&mut self, rva: u32) {
        self.bytes[0xA8..0xAC].copy_from_slice(&rva.to_le_bytes());
    }

    fn text_sizes(&mut self, virtual_size: u32, raw_size: u32) {
        self.bytes[0x190..0x194].copy_from_slice(&virtual_size.to_le_bytes());
        self.bytes[0x198..0x19C].copy_from_slice(&raw_size.to_le_bytes());
    }

    fn second_section(&mut self, raw: u32, executable: bool) {
        self.bytes[0x86..0x88].copy_from_slice(&2u16.to_le_bytes());
        self.bytes[0xD0..0xD4].copy_from_slice(&0x3000u32.to_le_bytes());
        let section = 0x1B0;
        self.bytes[section..section + 5].copy_from_slice(b".more");
        self.bytes[section + 8..section + 12].copy_from_slice(&0x200u32.to_le_bytes());
        self.bytes[section + 12..section + 16].copy_from_slice(&0x2000u32.to_le_bytes());
        self.bytes[section + 16..section + 20].copy_from_slice(&0x200u32.to_le_bytes());
        self.bytes[section + 20..section + 24].copy_from_slice(&raw.to_le_bytes());
        let flags: u32 = if executable { 0x60000020 } else { 0x40000040 };
        self.bytes[section + 36..section + 40].copy_from_slice(&flags.to_le_bytes());
    }

    fn build(self) -> Vec<u8> {
        self.bytes
    }
}

/// Test: an unconditional JMP over an INT3 means the INT3 is unreachable.
/// The CFG-aware probe must NOT report the INT3 as the first DBT blocker.
/// The reachable destination (a MOV then HLT) has no unsupported DBT instructions.
#[test]
fn cfg_unreachable_int3_after_jmp_not_reported_as_blocker() {
    // Layout at RVA 0x1000 (text_off 0):
    //   0x1000: EB 05        JMP +5  (jump to 0x1007)
    //   0x1002: CC           INT3    (unreachable padding)
    //   0x1003: CC           INT3    (unreachable padding)
    //   0x1004: CC           INT3    (unreachable padding)
    //   0x1005: CC           INT3    (unreachable padding)
    //   0x1006: CC           INT3    (unreachable padding)
    //   0x1007: 48 89 C8     MOV RAX, RCX   (REX.W + MOV r/m, reg — reg-to-reg, DBT supported)
    //   0x100A: F4           HLT
    let mut b = PeBuilder::new();
    b.write_text(
        0,
        &[
            0xEB, 0x05, // JMP to 0x1007 (EB 05 = short JMP, skip 5 bytes)
            0xCC, 0xCC, 0xCC, 0xCC, 0xCC, // 5x INT3 (unreachable)
            0x48, 0x89, 0xC8, // MOV RAX, RCX (reg-reg, DBT ok)
            0xF4, // HLT
        ],
    );
    let pe = b.build();
    let config = ProbeConfig::default();
    let report = probe_pe_bytes(&pe, &config).expect("probe must succeed");

    // The CFG-aware probe must not report the INT3 as a blocker because it is
    // only reachable via linear sweep, not via control flow from the entry.
    assert!(
        report.first_unsupported_dbt_insn.is_none(),
        "no DBT blocker expected; got {:?}",
        report.first_unsupported_dbt_insn
    );

    // Exactly the reachable instructions should appear: JMP, MOV, HLT.
    let rvas: Vec<u32> = report.instruction_records.iter().map(|r| r.rva).collect();
    assert!(rvas.contains(&0x1000), "JMP at 0x1000 must be recorded");
    assert!(rvas.contains(&0x1007), "MOV at 0x1007 must be recorded");
    assert!(rvas.contains(&0x100A), "HLT at 0x100A must be recorded");
    // None of the INT3 RVAs should appear
    for int3_rva in [0x1002u32, 0x1003, 0x1004, 0x1005, 0x1006] {
        assert!(
            !rvas.contains(&int3_rva),
            "INT3 at 0x{int3_rva:04x} must not appear in reachable records"
        );
    }
}

/// Test: a reachable unsupported instruction IS reported as the first DBT blocker.
/// Here the entry directly contains an unsupported opcode (0x0F 0x05 = SYSCALL).
#[test]
fn cfg_reachable_unsupported_insn_is_reported() {
    // Layout:
    //   0x1000: 0F 05   SYSCALL (unsupported two-byte opcode)
    //   0x1002: F4      HLT
    let mut b = PeBuilder::new();
    b.write_text(0, &[0x0F, 0x05, 0xF4]);
    let pe = b.build();
    let config = ProbeConfig::default();
    let report = probe_pe_bytes(&pe, &config).expect("probe must succeed");

    let blocker = report
        .first_unsupported_dbt_insn
        .expect("SYSCALL must be reported as unsupported DBT blocker");
    assert_eq!(blocker.rva, 0x1000, "blocker must be at the entry RVA");
}

/// Test: a direct CALL follows the call target AND continues with the fall-through.
/// The call target has an unsupported instruction; the fall-through (after CALL) has HLT.
/// Both should appear in the records because CALL adds both paths to the worklist.
#[test]
fn cfg_direct_call_visits_target_and_continuation() {
    // Layout:
    //   0x1000: E8 05 00 00 00   CALL 0x100A   (5-byte CALL rel32, target = 0x1000+5+5 = 0x100A)
    //   0x1005: F4               HLT            (fall-through / return site)
    //   0x1006: CC               INT3           (padding after HLT, unreachable)
    //   0x1007: CC               INT3           (unreachable)
    //   0x1008: CC               INT3           (unreachable)
    //   0x1009: CC               INT3           (unreachable)
    //   0x100A: 0F 05            SYSCALL        (call target — unsupported)
    //   0x100C: C3               RET
    let mut b = PeBuilder::new();
    b.write_text(
        0,
        &[
            0xE8, 0x05, 0x00, 0x00, 0x00, // CALL 0x100A (rel32 = 5)
            0xF4, // HLT at 0x1005
            0xCC, 0xCC, 0xCC, 0xCC, // INT3 padding, unreachable
            0x0F, 0x05, // SYSCALL at 0x100A (unsupported)
            0xC3, // RET
        ],
    );
    let pe = b.build();
    let config = ProbeConfig::default();
    let report = probe_pe_bytes(&pe, &config).expect("probe must succeed");

    let rvas: Vec<u32> = report.instruction_records.iter().map(|r| r.rva).collect();

    // CALL itself must appear
    assert!(rvas.contains(&0x1000), "CALL at 0x1000 must be recorded");
    // Fall-through (HLT at 0x1005) must appear
    assert!(
        rvas.contains(&0x1005),
        "HLT at 0x1005 (fall-through) must be recorded"
    );
    // Call target (SYSCALL at 0x100A) must appear
    assert!(
        rvas.contains(&0x100A),
        "SYSCALL at 0x100A (call target) must be recorded; rvas={rvas:?}"
    );

    // The unsupported instruction must be reported as a blocker
    assert!(
        report.first_unsupported_dbt_insn.is_some(),
        "SYSCALL must be the first DBT blocker"
    );
}

/// Test: a conditional branch visits both the taken target and the not-taken fall-through.
#[test]
fn cfg_conditional_branch_visits_both_targets() {
    // Layout:
    //   0x1000: 74 05            JZ  +5   (taken → 0x1007, not-taken → 0x1002)
    //   0x1002: 48 89 C8         MOV RAX, RCX  (not-taken path, DBT ok)
    //   0x1005: F4               HLT
    //   0x1006: CC               INT3 (padding after fall-through HLT, unreachable)
    //   0x1007: 48 31 C0         XOR RAX, RAX  (taken path)
    //   0x100A: F4               HLT
    let mut b = PeBuilder::new();
    b.write_text(
        0,
        &[
            0x74, 0x05, // JZ to 0x1007 (rel8 = 5, insn is 2 bytes)
            0x48, 0x89, 0xC8, // MOV RAX, RCX at 0x1002
            0xF4, // HLT at 0x1005
            0xCC, // INT3 at 0x1006 (unreachable — after HLT)
            0x48, 0x31, 0xC0, // XOR RAX, RAX at 0x1007 (taken path)
            0xF4, // HLT at 0x100A
        ],
    );
    let pe = b.build();
    let config = ProbeConfig::default();
    let report = probe_pe_bytes(&pe, &config).expect("probe must succeed");

    let rvas: Vec<u32> = report.instruction_records.iter().map(|r| r.rva).collect();

    // Both the taken (0x1007) and not-taken (0x1002) paths must be visited
    assert!(rvas.contains(&0x1000), "JZ at 0x1000 must be recorded");
    assert!(
        rvas.contains(&0x1002),
        "not-taken path at 0x1002 must be recorded"
    );
    assert!(
        rvas.contains(&0x1007),
        "taken path at 0x1007 must be recorded"
    );

    // No DBT blockers in this fixture
    assert!(
        report.first_unsupported_dbt_insn.is_none(),
        "no DBT blocker expected; got {:?}",
        report.first_unsupported_dbt_insn
    );

    // INT3 at 0x1006 is unreachable (after HLT on not-taken path)
    assert!(
        !rvas.contains(&0x1006),
        "INT3 at 0x1006 is unreachable and must not appear"
    );
}

/// Test: an indirect JMP stops that CFG path (target uncertain) so bytes
/// immediately following it are not reported as reachable.
#[test]
fn cfg_indirect_jmp_stops_path_target_uncertain() {
    // Layout:
    //   0x1000: FF 25 00 00 00 00   JMP QWORD PTR [RIP+0]  (indirect JMP via IAT)
    //   0x1006: CC                  INT3  (after the indirect JMP — unreachable from entry)
    //   0x1007: F4                  HLT   (also unreachable)
    let mut b = PeBuilder::new();
    b.write_text(
        0,
        &[
            0xFF, 0x25, 0x00, 0x00, 0x00, 0x00, // JMP *[RIP+0] — 6 bytes
            0xCC, // INT3 (unreachable after indirect JMP)
            0xF4, // HLT  (unreachable)
        ],
    );
    let pe = b.build();
    let config = ProbeConfig::default();
    let report = probe_pe_bytes(&pe, &config).expect("probe must succeed");

    let rvas: Vec<u32> = report.instruction_records.iter().map(|r| r.rva).collect();

    // The indirect JMP itself is recorded (it's reachable from the entry)
    assert!(
        rvas.contains(&0x1000),
        "indirect JMP at 0x1000 must be recorded"
    );

    // The INT3 and HLT after it must NOT appear (the indirect JMP stops the path)
    assert!(
        !rvas.contains(&0x1006),
        "INT3 at 0x1006 must not appear (after indirect JMP)"
    );
    assert!(
        !rvas.contains(&0x1007),
        "HLT at 0x1007 must not appear (after indirect JMP)"
    );
}

#[test]
fn cfg_does_not_complete_a_call_using_overlay_bytes() {
    let mut builder = PeBuilder::new();
    builder.entry(0x11FF);
    builder.write_text(0, &[0xF4]);
    builder.write_text(0x1FF, &[0xE8]);
    // The final backed byte is CALL's opcode. Pretending the file overlay
    // contains its rel32 would make the isolated HLT reachable.
    builder.bytes[0x400..0x404].copy_from_slice(&(-516i32).to_le_bytes());

    let report = probe_pe_bytes(&builder.build(), &ProbeConfig::default()).unwrap();
    assert!(report.instruction_records.is_empty());
    assert!(report.first_unsupported_dbt_insn.is_none());
}

#[test]
fn cfg_does_not_disassemble_a_zero_fill_tail_or_nonexecutable_target() {
    let mut zero_fill = PeBuilder::new();
    zero_fill.text_sizes(0x240, 0x200);
    zero_fill.entry(0x1200);
    zero_fill.bytes[0x400] = 0xF4;
    let report = probe_pe_bytes(&zero_fill.build(), &ProbeConfig::default()).unwrap();
    assert!(report.instruction_records.is_empty());

    let mut nonexec = PeBuilder::new();
    nonexec.second_section(0x600, false);
    // JMP 0x2000, followed by no reachable executable instruction.
    nonexec.write_text(0, &[0xE9, 0xFB, 0x0F, 0, 0]);
    nonexec.bytes[0x600] = 0xF4;
    let report = probe_pe_bytes(&nonexec.build(), &ProbeConfig::default()).unwrap();
    let rvas: Vec<_> = report.instruction_records.iter().map(|r| r.rva).collect();
    assert_eq!(rvas, [0x1000]);
}

#[test]
fn cfg_joins_adjacent_executable_rvas_with_separate_raw_offsets() {
    let mut builder = PeBuilder::new();
    builder.bytes.resize(0x2000, 0);
    builder.entry(0x1FFF);
    builder.text_sizes(0x1000, 0x1000);
    builder.second_section(0x1600, true);
    builder.write_text(0xFFF, &[0xE8]);
    // Leave different bytes immediately after the first section in the file.
    // The 4-byte displacement is at the start of the next executable RVA.
    builder.bytes[0x1200..0x1204].fill(0xA5);
    builder.bytes[0x1600..0x1605].copy_from_slice(&[0, 0, 0, 0, 0xF4]);

    let report = probe_pe_bytes(&builder.build(), &ProbeConfig::default()).unwrap();
    assert_eq!(report.instruction_records.len(), 2);
    assert_eq!(report.instruction_records[0].rva, 0x1FFF);
    assert_eq!(report.instruction_records[0].bytes, [0xE8, 0, 0, 0, 0]);
    assert_eq!(report.instruction_records[1].rva, 0x2004);
    assert_eq!(report.instruction_records[1].bytes, [0xF4]);
}

#[test]
fn cfg_byte_budget_never_admits_part_of_an_instruction() {
    let mut builder = PeBuilder::new();
    builder.write_text(0, &[0xE8, 0, 0, 0, 0, 0xF4]);
    let image = builder.build();
    for (max_disasm_bytes, expected) in [(0, 0), (4, 0), (5, 5), (6, 6)] {
        let config = ProbeConfig {
            max_disasm_bytes,
            ..ProbeConfig::default()
        };
        let report = probe_pe_bytes(&image, &config).unwrap();
        let total: usize = report
            .instruction_records
            .iter()
            .map(|r| r.bytes.len())
            .sum();
        assert_eq!(total, expected, "limit={max_disasm_bytes}");
    }
}

#[test]
fn cfg_malformed_truncated_at_section_boundary() {
    let mut builder = PeBuilder::new();
    builder.entry(0x11FF);
    builder.write_text(0x1FF, &[0x48]); // REX without a backed opcode.
    let report = probe_pe_bytes(&builder.build(), &ProbeConfig::default()).unwrap();
    assert!(report.instruction_records.is_empty());
}
