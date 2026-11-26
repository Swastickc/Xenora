# RDNA 2 gfx1030 semantic completeness campaign

[#526](https://github.com/ranveersoni98/Xenora/issues/526) tracks applicable
gfx1030 instructions through execution. Use the
[semantic registry and generator](../shaders/semantic-registry.md) from
[#215](https://github.com/ranveersoni98/Xenora/issues/215), the
[generated coverage report](../shaders/semantic-coverage.md) and the existing
RDNA 2 planner. The campaign covers original synthetic shaders on native
Apple Silicon and Metal. Xbox hardware and commercial games are outside the
evidence.

## Coverage at the start

At main `c76e974f14c5807ce71ed50da9808b59e4c40d2c`, the pinned AMD snapshot
contains **1,465 forms in 21 encoding families**. The coverage generator
and planner reconciliation agree on the stages they both count.

| Stage | Forms |
| --- | ---: |
| Identified by metadata | 1,465 |
| LLVM three-way encoding validation recorded | 169 |
| Registry declarations, including encoding variants | 179 |
| Semantic decoder lists | 189 |
| Intermediate representation (IR) lowering recorded | 185 |
| Metal Shading Language (MSL) lowering recorded | 185 |
| Reference fixture verification recorded | 18 |
| Native Metal fixture verification recorded | 18 |

There are 1,276 identified forms outside the decoder lists. The planner's
undeclared inventory also contains 10 decoder-listed forms handled outside the
registry. IR and MSL share a status field, and the stage totals are independent.
Fixture verification counts omit local results pending acceptance. The report
lists 83 unsupported variant names summed across families. Its variant table
covers existing operand rules, so a zero in an unimplemented family leaves that
family's operand inventory open.

A local baseline run on Apple M2 Max, macOS 27.2.1 and Rust 1.100.0 passed
256 focused CPU tests (26 ignored) and 137 tests in `scripts/test-gpu.sh`
(3 manual benchmarks ignored), with Metal API and shader validation enabled.
The native AMDGPU LLVM assembler was unavailable for this run. These results
are baseline evidence and do not promote rows under #302.

## P0 instruction backlog

The second column is the decoder baseline against the snapshot. Each row
includes the current subset's remaining operand domains and verification
debt. Use the report and planner for the opcode-level inventory. A batch starts
with the AMD guide and XML entries for its specific encodings.

From the repository root, check the baseline and generate a short list of
candidates with the existing tools. Supply a current claims JSON file to
`rank` after checking issue comments and open PRs, since plain-text claims
are not necessarily in the queue.

```sh
python3 crates/isa/generate_rdna2_meta.py --check
python3 crates/isa/generate_semantics.py --check
python3 scripts/audit/semantic_coverage.py --check
python3 scripts/audit/status.py --check
(cd scripts && python3 -m qa rdna2 reconcile)
(cd scripts && python3 -m qa rdna2 candidates --family VOP3 --limit 10)
```

| Family | Decoder / identified | Remaining execution and gating dependencies |
| --- | ---: | --- |
| SOP1 | 12 / 65 | Scalar unary and register-pair operations, mask moves and save/restore. Define special-register access and SCC/EXEC effects. |
| SOP2 | 33 / 51 | Remaining scalar arithmetic, carry/borrow and 64-bit logical, shift and select forms. Define pair alignment, SCC inputs and full-width results. Check the scope of closed, unmerged PR [#319](https://github.com/ranveersoni98/Xenora/pull/319) with elforeign. |
| SOPC | 12 / 18 | Remaining scalar comparisons and bit tests. Specify 64-bit sources, special operands and SCC consumption. |
| SOPK | 17 / 27 | Remaining immediate arithmetic, control and hardware-register forms. Keep signed and unsigned immediates and privileged state distinct. |
| SOPP | 6 / 36 | Remaining branches, waits, barriers and system controls. Need validated targets, joins, termination and a memory-ordering model. [#187](https://github.com/ranveersoni98/Xenora/issues/187), [#188](https://github.com/ranveersoni98/Xenora/issues/188). |
| VOP1 | 7 / 81 | Unary integer and floating-point math, conversions and lane operations. Need exact type, rounding, register width and inactive-lane rules. |
| VOP2 | 24 / 47 | Remaining binary arithmetic, packed and floating-point forms. Cover scalar sources, carry, borrow and special mask destinations. |
| VOPC | 44 / 190 | Remaining integer, floating-point and compare-and-update-EXEC forms. Need NaN predicates and EXEC changes in the wave model. |
| VOP3 | 28 / 416 | Wider encodings and three-source operations, float multiply-add, modifiers and remaining comparisons. Need a three-source IR contract and floating-point mode rules. [#190](https://github.com/ranveersoni98/Xenora/issues/190). |
| VOP3SDST | 0 / 10 | Scalar side results alongside vector results. Need explicit carry/mask destinations and validated register pairs. |
| VOP3P | 0 / 29 | Packed arithmetic, dot products and per-half source modifiers. Need element packing and signedness, per-element rounding and defined writes. |
| SMEM | 0 / 14 | Scalar loads, resource selection and sync behavior. Need bounded uniform runtime values, descriptor ranges, alignment and wait tracking. [#189](https://github.com/ranveersoni98/Xenora/issues/189), PR [#511](https://github.com/ranveersoni98/Xenora/pull/511). |
| MUBUF | 2 / 75 | Buffer load/store widths, address modes, atomics and coherence. Extend the bounded buffer ABI before enabling additional ranges or writes. |
| MTBUF | 0 / 16 | Typed buffer access. Need format conversion, component selection, alignment and the same buffer bounds checks. |
| FLAT | 0 / 54 | Flat loads/stores and atomics. Need checked address-space selection and guest address translation. |
| GLOBAL | 0 / 57 | Global loads/stores and atomics. Need checked global address translation and ordering for concurrent accesses. |
| SCRATCH | 0 / 22 | Scratch memory access. Need per-lane addressing, lifetime, bounds, isolation and wait accounting. |
| DS | 0 / 123 | Local data share access, atomics and lane exchange. Need bounded workgroup memory, barriers and lane topology. Resolve applicability of global data share operations separately. |
| MIMG | 1 / 130 | Image loads, stores, sampling, gathers, queries, atomics and ray-related forms. Need validated descriptors, address tuples, component masks, format and sampler rules, access ordering and per-form host feasibility. |
| VINTRP | 2 / 3 | Interpolation operands and remaining forms. Need pixel-stage inputs, interpolation mode, attributes and stage checks. |
| EXP | 1 / 1 | Cover legal stage-specific targets, component enables, compressed exports and completion. The single decoded identity does not close export semantics. |

For each batch, enumerate source and destination register classes, widths and
alignment, including scalar and vector pairs, EXEC, VCC, SCC effects, M0, null
and temporary registers where the guide allows them. Cover signed/unsigned and
floating-point inlines, literal expansion and constant-bus limits. Check
encoding-specific alternatives such as 32- and 64-bit vector encodings,
sub-dword addressing (SDWA), data parallel primitives (DPP8/DPP16), non-sequential
image addresses (NSA), source modifiers, packing, clamp and rounding. Separate
spec-invalid combinations from legal combinations Xenora still refuses.

Start with Wave32 execution-mask (EXEC), vector condition code (VCC) and scalar
condition code (SCC) state, including empty, partial and full masks and runtime
uniform inputs. Model Wave64 and cross-lane behavior explicitly before
enabling corresponding forms. Preserve valid inactive-lane values, register
initialization at joins and bounded execution. Do floating-point mode and
rounding work alongside integer batches when their file scopes are disjoint.
Address resource safety, waits and workgroup ordering before enabling memory
or image encodings that depend on them. Lower each supported subset through IR
and MSL, with independent reference coverage and native readback in the same
batch or a linked dependent PR.

Advanced graphics integration, broader texture formats, resource accounting,
GPU memory management and compilation caching are P1. Performance work,
secondary benchmarks and nonessential refactoring are P2. A minimum resource
or format contract required for a P0 instruction follows that instruction into
P0. P1 and P2 can run in parallel when they leave P0 owners and test capacity
available.

## Ownership and independent verification

| Work | Owner or assignment |
| --- | --- |
| [#215](https://github.com/ranveersoni98/Xenora/issues/215), registry and shared semantic infrastructure | elforeign. Agree family scopes and shared changes before claiming work. |
| [#187](https://github.com/ranveersoni98/Xenora/issues/187), bounded runtime-uniform branches | jelly013, PR [#536](https://github.com/ranveersoni98/Xenora/pull/536), stacked on #511. |
| [#189](https://github.com/ranveersoni98/Xenora/issues/189), scalar buffer reads | jelly013, PR #511. |
| [#188](https://github.com/ranveersoni98/Xenora/issues/188), bounded loops | Unclaimed. Jelly confirmed that #188 can build on #536. Start with one uniform back edge, no outstanding loads and an explicit iteration bound. Confirm scope with the maintainer before claiming. |
| [#190](https://github.com/ranveersoni98/Xenora/issues/190), vector multiply-add | Unclaimed at kickoff. First proposed batch is Wave32 `v_fma_f32` with three VGPR inputs and modifiers clear, pending agreement with elforeign and the maintainer. Jelly confirmed that the VOP3 and float rules are outside #511 and #536. |
| [#302](https://github.com/ranveersoni98/Xenora/issues/302), independent acceptance | vedantdadhich21 claimed the policy work. The guide landed in [#332](https://github.com/ranveersoni98/Xenora/pull/332). Confirm the independence rule and pass ownership with the maintainer before promoting rows. |

Kickoff subagents reviewed the inventory independently and researched the
proposed #190 scope. Both worked read-only at low reasoning effort. For each
claimed implementation batch, assign a low-effort production worker to the
agreed paths, a separate low-effort reference/oracle worker and independent
reviewers R1 (scope and claims) and R2 (semantics and rejection cases).
Reviewers re-derive expectations from primary sources without adopting the
implementation's conclusions. The acceptance runner follows
[the #302 procedure](../testing/shader-form-acceptance.md) once the maintainer
confirms it. Keep expected results independent of generated dispatch and
production arithmetic.

Use the [AMD RDNA 2 ISA guide](https://docs.amd.com/v/u/en-US/rdna2-shader-instruction-set-architecture)
for documented behavior and the
[AMD machine-readable ISA](https://gpuopen.com/machine-readable-isa/) for
form identity and operand layout. Use the repository's pinned native LLVM
assembler/disassembler for encoding cross-checks. Cite the guide section,
snapshot row and the LLVM version in each implementation PR. The
[source record](../research/rdna2.md) documents the pinned inputs.

## Batch and completion criteria

- Claim one linked issue per coherent batch. Record the exact base and tested
  commits, form/operand domains, blocked dependencies and path owners. Publish
  complete batches as soon as their gates run.
- Give before and after counts for semantic decoding, encoding validation,
  reference execution and verification, IR lowering, MSL lowering and native
  Metal verification. Include unchanged counts where a stage did not advance.
  List remaining operand limits and distinguish local readback from acceptance.
- Verify valid encodings and explicit rejections through the decoder, compiler
  and independent reference where applicable. Include lane masks, boundaries,
  adversarial inputs and bit-exact floating-point checks where specified.
  Run original fixtures through the CPU oracle and native Metal with API and
  shader validation, keeping time, allocation and guest resource limits.
  Record commands, host, pass/fail/skip counts and any gaps.
- Give every pinned identity and every documented extension/operand domain a
  disposition. Mark supported scopes as independently verified only with a
  recorded acceptance entry. Mark partial scopes with their exact limits.
  For unsupported scopes cite the architectural blocker and a tracking issue.
  Keep unknown applicability and pending evidence open.
- Close the campaign when all 21 families have a reviewed inventory, there
  are zero unclassified forms and zero applicable forms left partial or
  pending, and all exclusions have documented architectural reasons reviewed
  by the maintainer. Report the stage totals and remaining exclusions together.

## Synchronization ledger

While working, refresh main at least every 15 minutes and again before opening
or updating a PR. Confirm the canonical upstream URL before adding a missing
remote. Keep the active worktree's changes in place. Checkpoint and coordinate
with active workers before integrating upstream changes in a clean worktree.
Rerun affected generators and tests.

```sh
git fetch upstream main --prune
git log --oneline HEAD..upstream/main
git diff --stat HEAD...upstream/main
gh pr list --repo ranveersoni98/Xenora --state open --limit 100
```

Record the latest successful check in the campaign ledger or the current
campaign issue checkpoint while holding a separate implementation scope. Keep
raw gate output under ignored `artifacts/`.

| Checked (Europe/London) | Upstream main | Checkpoint |
| --- | --- | --- |
| 2026-11-25 17:59 GMT | `c76e974f14c5807ce71ed50da9808b59e4c40d2c` | Kickoff. Reviewed main and the open PRs, including #511. |
| 2026-11-25 18:03 GMT | `c76e974f14c5807ce71ed50da9808b59e4c40d2c` | Before the roadmap PR. Main unchanged. The focused CPU and native Metal baseline ran on this commit. |
| 2026-11-25 18:21 GMT | `c76e974f14c5807ce71ed50da9808b59e4c40d2c` | #536 is open on top of #511. Its focused suite passed on M2 Max, including 2,120 cases in 2,116 Metal programs with validation. Main coverage is unchanged. |
