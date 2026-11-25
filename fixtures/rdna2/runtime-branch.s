// SPDX-License-Identifier: GPL-3.0-or-later
// Wave32 if/else on a loaded value (AMD RDNA 2 ISA guide 7.2, 12.4, 12.5): the
// first input word selects v2 = x + 100 or v2 = x ^ 0xff for every lane.
.text
s_mov_b32 s8, 0
s_buffer_load_dword s8, s[0:3], null
s_waitcnt lgkmcnt(0)
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v2, 0
v_mov_b32_e32 v3, 0xff
v_mov_b32_e32 v4, 100
s_cmp_gt_u32 s8, 9
s_cbranch_scc0 otherwise
v_add_nc_u32_e64 v2, v1, v4
s_branch join
otherwise:
v_xor_b32_e32 v2, v1, v3
join:
buffer_store_dword v2, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
