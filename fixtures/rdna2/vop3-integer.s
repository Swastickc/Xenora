// SPDX-License-Identifier: GPL-3.0-or-later
// Original VOP3A vectors for the eleven per-lane integer encoding variants.
// All arithmetic operands are vector general-purpose registers (VGPRs) and every result
// depends on the lane's own input word. XOR folds the results into one word.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen // Load the lane's input word.
s_waitcnt vmcnt(0)                        // Wait for the load.
v_mul_i32_i24_e64 v2, v1, v1              // Multiply signed 24-bit inputs and keep the low word.
v_mul_hi_i32_i24_e64 v3, v2, v1           // Multiply signed 24-bit inputs and keep the high word.
v_mul_u32_u24_e64 v4, v2, v1              // Multiply unsigned 24-bit inputs and keep the low word.
v_mul_hi_u32_u24_e64 v255, v4, v1         // Multiply unsigned 24-bit inputs and keep the high word.
v_min_i32_e64 v6, v1, v2                  // Select the signed minimum.
v_max_i32_e64 v7, v1, v4                  // Select the signed maximum.
v_min_u32_e64 v8, v3, v1                  // Select the unsigned minimum.
v_max_u32_e64 v9, v255, v2                // Select the unsigned maximum.
v_xnor_b32_e64 v10, v1, v255              // Complement the XOR.
v_sub_nc_u32_e64 v11, v6, v10              // Subtract the second source from the first.
v_subrev_nc_u32_e64 v1, v1, v9            // Subtract the first source from the second, overwriting the first.
v_xor_b32_e32 v13, v2, v3                 // Start the XOR fold.
v_xor_b32_e32 v13, v4, v13                // Fold the unsigned product low word.
v_xor_b32_e32 v13, v255, v13              // Fold the unsigned product high word.
v_xor_b32_e32 v13, v6, v13                // Fold the signed minimum.
v_xor_b32_e32 v13, v7, v13                // Fold the signed maximum.
v_xor_b32_e32 v13, v8, v13                // Fold the unsigned minimum.
v_xor_b32_e32 v13, v9, v13                // Fold the unsigned maximum.
v_xor_b32_e32 v13, v10, v13               // Fold the XNOR result.
v_xor_b32_e32 v13, v11, v13               // Fold the subtraction result.
v_xor_b32_e32 v13, v1, v13                // Fold the reverse subtraction result.
buffer_store_dword v13, v0, s[4:7], 0 idxen // Store the folded word.
s_waitcnt_vscnt null, 0                     // Wait for the store.
s_endpgm                                  // End the program.
