// SPDX-License-Identifier: GPL-3.0-or-later
// Original Wave32 VOP3A vectors for the sixteen integer compare encoding variants.
// Each compare writes vcc_lo through the explicit 64-bit destination field
// (one writes s8), and each wave folds the masks into v2 as
// v2 = (v2 << 1) ^ mask. Sources are VGPRs. This fixture is checked through
// its IR and MSL goldens; the per-lane expected values for these forms are in
// tests/validation/tests/vop3_compare.rs.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen // Load the lane's input word.
s_waitcnt vmcnt(0)                        // Wait for the load.
v_mov_b32_e32 v2, 0                       // Clear the accumulator.
v_mov_b32_e32 v5, 1                       // Shift count for the fold.
v_mov_b32_e32 v3, 0x40000001              // Second source: inputs fall below, on and above it.
v_mov_b32_e32 v6, 1                       // Low bit for the branch fold.
v_cmp_f_i32_e64 vcc_lo, v1, v3            // Compare the input with 0x40000001.
v_mov_b32_e32 v4, vcc_lo                  // Move the mask into a VGPR.
v_lshlrev_b32_e32 v2, v5, v2              // Shift the accumulator.
v_xor_b32_e32 v2, v2, v4                  // Fold the mask.
v_cmp_lt_i32_e64 vcc_lo, v1, v3           // Compare the input with 0x40000001.
v_mov_b32_e32 v4, vcc_lo                  // Move the mask into a VGPR.
v_lshlrev_b32_e32 v2, v5, v2              // Shift the accumulator.
v_xor_b32_e32 v2, v2, v4                  // Fold the mask.
v_cmp_eq_i32_e64 vcc_lo, v1, v3           // Compare the input with 0x40000001.
v_mov_b32_e32 v4, vcc_lo                  // Move the mask into a VGPR.
v_lshlrev_b32_e32 v2, v5, v2              // Shift the accumulator.
v_xor_b32_e32 v2, v2, v4                  // Fold the mask.
v_cmp_le_i32_e64 vcc_lo, v1, v3           // Compare the input with 0x40000001.
v_mov_b32_e32 v4, vcc_lo                  // Move the mask into a VGPR.
v_lshlrev_b32_e32 v2, v5, v2              // Shift the accumulator.
v_xor_b32_e32 v2, v2, v4                  // Fold the mask.
v_cmp_gt_i32_e64 vcc_lo, v1, v3           // Compare the input with 0x40000001.
v_mov_b32_e32 v4, vcc_lo                  // Move the mask into a VGPR.
v_lshlrev_b32_e32 v2, v5, v2              // Shift the accumulator.
v_xor_b32_e32 v2, v2, v4                  // Fold the mask.
v_cmp_ne_i32_e64 vcc_lo, v1, v3           // Compare the input with 0x40000001.
v_mov_b32_e32 v4, vcc_lo                  // Move the mask into a VGPR.
v_lshlrev_b32_e32 v2, v5, v2              // Shift the accumulator.
v_xor_b32_e32 v2, v2, v4                  // Fold the mask.
v_cmp_ge_i32_e64 vcc_lo, v1, v3           // Compare the input with 0x40000001.
v_mov_b32_e32 v4, vcc_lo                  // Move the mask into a VGPR.
v_lshlrev_b32_e32 v2, v5, v2              // Shift the accumulator.
v_xor_b32_e32 v2, v2, v4                  // Fold the mask.
v_cmp_t_i32_e64 vcc_lo, v1, v3            // Compare the input with 0x40000001.
v_mov_b32_e32 v4, vcc_lo                  // Move the mask into a VGPR.
v_lshlrev_b32_e32 v2, v5, v2              // Shift the accumulator.
v_xor_b32_e32 v2, v2, v4                  // Fold the mask.
v_cmp_f_u32_e64 vcc_lo, v1, v3            // Compare the input with 0x40000001.
v_mov_b32_e32 v4, vcc_lo                  // Move the mask into a VGPR.
v_lshlrev_b32_e32 v2, v5, v2              // Shift the accumulator.
v_xor_b32_e32 v2, v2, v4                  // Fold the mask.
v_cmp_lt_u32_e64 vcc_lo, v1, v3           // Compare the input with 0x40000001.
v_mov_b32_e32 v4, vcc_lo                  // Move the mask into a VGPR.
v_lshlrev_b32_e32 v2, v5, v2              // Shift the accumulator.
v_xor_b32_e32 v2, v2, v4                  // Fold the mask.
v_cmp_eq_u32_e64 vcc_lo, v1, v3           // Compare the input with 0x40000001.
v_mov_b32_e32 v4, vcc_lo                  // Move the mask into a VGPR.
v_lshlrev_b32_e32 v2, v5, v2              // Shift the accumulator.
v_xor_b32_e32 v2, v2, v4                  // Fold the mask.
v_cmp_le_u32_e64 vcc_lo, v1, v3           // Compare the input with 0x40000001.
v_mov_b32_e32 v4, vcc_lo                  // Move the mask into a VGPR.
v_lshlrev_b32_e32 v2, v5, v2              // Shift the accumulator.
v_xor_b32_e32 v2, v2, v4                  // Fold the mask.
v_cmp_gt_u32_e64 vcc_lo, v1, v3           // Compare the input with 0x40000001.
v_mov_b32_e32 v4, vcc_lo                  // Move the mask into a VGPR.
v_lshlrev_b32_e32 v2, v5, v2              // Shift the accumulator.
v_xor_b32_e32 v2, v2, v4                  // Fold the mask.
v_cmp_ne_u32_e64 vcc_lo, v1, v3           // Compare the input with 0x40000001.
v_mov_b32_e32 v4, vcc_lo                  // Move the mask into a VGPR.
v_lshlrev_b32_e32 v2, v5, v2              // Shift the accumulator.
v_xor_b32_e32 v2, v2, v4                  // Fold the mask.
v_cmp_ge_u32_e64 vcc_lo, v1, v3           // Compare the input with 0x40000001.
v_mov_b32_e32 v4, vcc_lo                  // Move the mask into a VGPR.
v_lshlrev_b32_e32 v2, v5, v2              // Shift the accumulator.
v_xor_b32_e32 v2, v2, v4                  // Fold the mask.
v_cmp_t_u32_e64 vcc_lo, v1, v3            // Compare the input with 0x40000001.
v_mov_b32_e32 v4, vcc_lo                  // Move the mask into a VGPR.
v_lshlrev_b32_e32 v2, v5, v2              // Shift the accumulator.
v_xor_b32_e32 v2, v2, v4                  // Fold the mask.
v_cmp_eq_u32_e64 s8, v1, v3               // A mask into a plain SGPR, never read.
v_cmp_gt_u32_e64 vcc_lo, v3, v1           // Unsigned 0x40000001 > input.
s_cbranch_vccz 1                          // Skip the next fold when no lane passed.
v_xor_b32_e32 v2, v6, v2                  // Flip the low bit when some lane passed.
buffer_store_dword v2, v0, s[4:7], 0 idxen // Store the folded word.
s_waitcnt_vscnt null, 0                   // Wait for the store.
s_endpgm                                  // End the program.
