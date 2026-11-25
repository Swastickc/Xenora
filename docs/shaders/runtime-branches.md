# Run-time uniform branches

A uniform branch takes the same path for a whole wave. When its condition is known during translation, the skipped code is simply not translated. This page covers `s_cbranch_scc0` when SCC is known only at run time. The other run-time branch, `s_cbranch_vccz` on a lane-derived mask, has [its own contract](vector-compare.md#6-branches), which this does not change.

## Where a run-time SCC comes from

The twelve SOPC and twelve SOPK integer compares accept a [loaded SGPR](scalar-memory.md) as an operand and then write a run-time SCC (AMD "RDNA 2" guide, 12.2 and 12.4). Equality compares raw bits; the ordered compares are signed or unsigned by the mnemonic, and unsigned SOPK compares zero-extend SIMM16. A later compare of known values makes SCC known again.

Only `s_cbranch_scc0` may read a run-time SCC. `s_cselect_b32`, carry inputs, `s_cmovk_i32` and all scalar arithmetic on loaded values are refused.

## Skipped code

The branch skips code up to a forward target. Its last instruction may be an `s_branch` to a later target, which makes the code between the two targets an `else` part:

```
s_cmp_gt_u32 s8, 9
s_cbranch_scc0 otherwise
v_add_nc_u32_e64 v2, v1, v4   ; SCC = 1
s_branch join
otherwise:
v_xor_b32_e32 v2, v1, v3      ; SCC = 0
join:
```

Both parts follow the rules of the `s_cbranch_vccz` region, checked whichever path runs:

- only the integer vector instructions listed there; no scalar instruction, compare, wait, memory access, branch or program end;
- every VGPR that is written was initialized before the branch, and every source is ready;
- no vector or scalar load is outstanding at the branch.

Nested branches, a second `s_branch`, backward targets and targets inside an instruction are refused, so every accepted program terminates after at most its own length.

At the join, a VGPR holds its value from the path that ran. EXEC applies within each part as usual. Scalar state cannot change inside a part.

## Metal

Both parts are pure 32-bit arithmetic and are evaluated unconditionally; each VGPR that differs is selected at the join by the SCC value. All threads of a dispatch compute the same SCC from the same input words.

## Evidence

`tests/validation/tests/runtime_branch.rs` derives each expected word from the guide and runs both outcomes of every compare on the CPU reference and on native Metal. Nothing was compared with AMD or Xbox hardware. Loops and divergent control flow are not supported.
