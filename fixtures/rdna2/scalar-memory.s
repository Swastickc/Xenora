// SPDX-License-Identifier: GPL-3.0-or-later
// Wave32 scalar buffer loads (AMD RDNA 2 ISA guide 7.2 to 7.5). Seven input
// words reach SGPRs and are mixed into each lane's own word. A dispatch with
// fewer than eight words leaves the missing SGPRs at zero.
.text
s_mov_b32 s8, 0
s_mov_b32 s10, 0
s_mov_b32 s11, 0
s_mov_b32 s12, 0
s_mov_b32 s13, 0
s_mov_b32 s14, 0
s_mov_b32 s15, 0
s_mov_b32 s20, 8
s_buffer_load_dword s8, s[0:3], 0x4
s_buffer_load_dwordx2 s[10:11], s[0:3], s20
s_buffer_load_dwordx4 s[12:15], s[0:3], s20 offset:0x8
s_waitcnt lgkmcnt(0)
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_add_nc_u32_e32 v2, s8, v1
v_xnor_b32_e32 v2, s10, v2
v_add_nc_u32_e32 v2, s11, v2
v_xnor_b32_e32 v2, s12, v2
v_add_nc_u32_e32 v2, s13, v2
v_xnor_b32_e32 v2, s14, v2
v_add_nc_u32_e32 v2, s15, v2
buffer_store_dword v2, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
