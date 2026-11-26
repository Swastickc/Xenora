# GPU and guest execution evidence

The resumed checkout started clean on main at
`4b9a4cecf35c8d4d90d8ecdb9c1510e7bd260102`, tracking origin/main.
The diff since `5c479005b0e0631a608a369b9e41327da61bb61e` contains three
documentation files. The baseline reuses the implementation evidence from
that commit and retains the original attribution for every run. The
[Phase 01 assurance checkpoint](phase01-assurance.md) records the original release-gate definitions and results.
The [independent claims audit (G8)](review-log.md#phase-01-evidence-recheck-at-4b9a4ce) and parser fuzzing (G9) passed for the Phase 01 candidate. Controlled benchmarks (G10), redistribution rights (G11), repository hygiene and contributor attestations (G12), and final independent acceptance (G15) remain open. Phase 01 is not release-ready.

The resumed host reported arm64, 16 GiB RAM, 276 GiB available disk,
macOS 27.3 (26C5047e), Xcode 27.2 (27B52) and Rust 1.100.0.
The implementation evidence is from Apple M5. The archived command and
exit logs at `artifacts/assurance-5c47900/` report passing format, Clippy,
workspace, LLVM, fixtures, GPU, demo, semantic, native, claims, status and
dependency checks. The workspace ran 54 tests. The native suite ran 12
tests with validation enabled. Three original shader fixtures passed and
12 GPU semantic mutants were killed. Those are separate suites and their
counts do not estimate GPU dispatch totals. The oracle-only campaign
killed 44 of 44 mutants. The [integrated mutation and oracle review](mutation.md#integrated-source-campaign-at-4b9a4ce)
closes G5 and G7 for the Phase 01 candidate.

## Track A — GPU engineering

At the Phase 01 baseline, 15 recognized and decoded gfx1030 forms
contributed to the verified fixture path. See the [current inventory](../shaders/instruction-coverage.md) for later additions.
Forward scalar control is resolved when lifting to IR. The generated MSL
embodies that resolution without emitting a dynamic GPU branch. The
translated stage is compute, with lane-indexed 32-bit reads and writes to
two disjoint buffers. The original native offscreen render test provides
separate backend evidence. The [host evidence](../research/host-lab.md) records why translation is limited to the documented subset of 32-bit floating-point operations (FP32).

## Track B — guest execution

| Milestone | Baseline status |
| --- | --- |
| Native host bootstrap (X0) | Phase 01 recorded native host and Metal capability evidence. |
| Native guest instruction execution (X1) | Demonstrated on ARM64 macOS with both known-answer traces and six fault tests. |
| Executable loading and synthetic service dispatch (X2) | Independently demonstrated on ARM64 with the original fixed-base PE32+ fixture and synthetic add-one service. |
| Guest-to-GPU command submission (X3) | Independently verified from the original PE through the shared loader, interpreter and native Metal. |
| Guest-driven Metal rendering (X4) | Independently verified on Apple M5 from original guest PE through RGBA8 texture readback. |
| Authentic Xbox interfaces (X5) | Blocked on public or authorized interfaces and test inputs. |
| Commercial title validation (X6) | Blocked on authentic Xbox interfaces and suitable project test inputs. User import follows the [game file policy](../legal.md#user-provided-game-files). |
| Historical stretch goal for a commercial title (X7) | Blocked. The runtime path and suitable project test inputs are unavailable. |

## Phase 02 acceptance

| Gate | Baseline disposition |
| --- | --- |
| P2-01 | Baseline captured. Phase 01 evidence and blockers are linked above. |
| P2-02 | KyTx and nine additional projects screened. Original fixtures and public specifications are the implementation source. No emulator code, tables or metadata are in scope for adoption. |
| P2-03 | VGPR-only OR verified on Apple M5. |
| P2-04 | Workspace, LLVM and native GPU regression runs passed after OR integration. |
| P2-05 | Independent OR expected values matched CPU execution and Metal readback. |
| P2-06 | Integrated Phase 01 source campaign passed. Reassess changed Phase 02 inputs. |
| P2-07 | Matched performance comparisons pending a validated slice. |
| P2-08 | Native audit and Metal suite passed on the OR working tree. |
| P2-09 | OR status inventory updated. Remaining Phase 02 claims require review. |
| P2-10 | Independent Phase 02 R1 and R2 reports pending. |

## Implementation checkpoint

The working tree based on `4b9a4ce` adds VGPR-only
`v_or_b32_e32`. The verification snapshot and fixture hashes are in
`artifacts/phase02-or/`. The workspace ran 56 tests successfully.
The three opt-in tests were run separately. LLVM reported 312 three-way
comparisons across 16 forms and 15 malformed or out-of-subset probes.
The 12 native regression tests passed. With Metal API and shader
validation enabled, the original OR fixture matched all 131 expected
output words in one native dispatch. Clippy, fixture regeneration and
the 15-executable native audit passed. This verifies the bounded
feature on Apple M5. Changed-code fuzz and mutation evidence and
matched performance still remain open for Phase 02.

The [isolated X1 experiment](../../experiments/x1/README.md) passed
Clippy and eight tests, including six fault cases. Its ARM64 driver
retired six and five original guest instructions in the two paths
and produced the expected output words 42 and 0xffffffff. The
architecture audit found only libSystem as a dynamic dependency.
Logs and traces are in `artifacts/x1/`.

The next GPU slice adds VGPR-only logical shifts
`v_lshlrev_b32_e32` and `v_lshrrev_b32_e32`. Both use the low five
bits of the first source as the count for the unsigned second source.
The original combined fixture exercises count wrapping, operand aliases
and high-bit values. The workspace ran 58 tests, the separate LLVM
suite checked 324 three-way comparisons across 18 forms and 24 malformed
or out-of-subset probes, and all 12 native regression tests passed.
The OR and shift fixtures each matched 131 output words on Apple M5
with both validation layers active. Clippy and the native audit passed.
Exit codes, fixture hashes and the working patch are under
`artifacts/phase02-shifts/`.

The isolated [X2 experiment](../../experiments/x1/README.md) loaded
the original fixed-base PE32+ image on ARM64 and retired three guest
instructions. Independent review confirmed that service 1 changed the
argument 41 to 42, resumed at `0x140001007`, and stored the eight-byte
little-endian result. The independent run passed 16 tests, including
alternate arguments, wraparound and rejection before retirement.
Formatting and Clippy passed. The profile rejects relocation and
zero-fill inputs and owns a private map per load. Logs are under
`artifacts/x2-review/`.

X3 passed independent acceptance after replacing the flat fixture with
an original PE32+ image using the existing loader and interpreter.
The independent suite passed 21 tests. Invalid entry and trap edits
rejected the command before dispatch. Five fresh Apple M5 runs with
both Metal validation layers matched the exact three-word output and
zero guest status. Evidence is under `artifacts/x3-pe.pQUnzE/`.

The isolated X4 driver loaded two original PE fixtures and invoked an
8×8 RGBA8 offscreen Metal draw. With API and shader validation enabled
on Apple M5, readback matched all 256 expected bytes per target for
guest parameters 17 and 93. The 24 Metal-feature tests passed.
Implementation evidence is under `artifacts/x4/`. Independent review
confirmed 384 exact pixels over three fresh runs of both fixtures and
all 24 tests passed. Review evidence is under `artifacts/x4-review/`.
