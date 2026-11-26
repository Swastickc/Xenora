// SPDX-License-Identifier: GPL-3.0-or-later
// Wave32 VOP1 binary32 rounding forms (AMD RDNA 2 ISA guide 12.8).
// Round the input toward zero, up and to nearest even, then round it down in
// even lanes only, and combine the results with a rounded literal (2.5 -> 2.0).
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_trunc_f32_e32 v2, v1
v_ceil_f32_e32 v3, v1
v_rndne_f32_e32 v4, v1
s_mov_b32 exec_lo, 0x55555555
v_floor_f32_e32 v4, v1
s_mov_b32 exec_lo, -1
v_rndne_f32_e32 v6, 0x40200000
v_xor_b32_e32 v5, v2, v3
v_xor_b32_e32 v5, v4, v5
v_xor_b32_e32 v5, v6, v5
buffer_store_dword v5, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
