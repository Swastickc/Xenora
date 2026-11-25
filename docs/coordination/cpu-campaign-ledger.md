# Xenora CPU/Runtime Expansion Campaign — Progress Ledger

Evidence below is local to the named host unless stated. Nothing here is independently reviewed yet.

## Runtime campaign checkpoint — 25 November 2026

Baseline: main `c76e974`. [The runtime survey][survey] provides the earlier
import census and package order. This checkpoint records merged work, active
owners and gaps that still need a focused issue or original guest fixture.

| Area | Merged on main | Open work and dependencies |
| --- | --- | --- |
| Process and thread lifecycle | [#339] and [#449] added the guest scheduler and blocking operations. [#469] added startup info and command-line services. [#480] added partial process termination, exit status, stack bounds and thread group affinity. | Full process semantics, feature reporting and additional handle lifecycle cases still need coverage. Keep process termination scoped to the guest. |
| Synchronization | Unnamed events, mutexes and semaphores, per-thread ownership, waits, and scheduler-backed critical sections, slim reader/writer locks and condition variables. | [#512] covers extended semaphore creation and requested access. Named objects, cancellation, deadlock diagnostics and a memory model for concurrent atomics remain separate work. |
| Virtual memory | Guest allocation, release, heap and limited protection and query operations. | [#203] tracks reservation, commit and decommit. Coordinate allocation accounting with [#207]. Extended queries, local allocation and guard-page semantics still need scoped work. |
| Module loading | [#490] added module identity, module-handle lookup and reference management for the executable and built-ins. | [#312] includes synthetic module filenames. Arbitrary mapped DLLs, dependency graphs, forwarded exports, DLL initialization and DLL thread-local storage still need a separately scoped loader contract. |
| C and C++ runtime | [#452] runs original statically linked C++ exception fixtures. [#460], [#486], [#489], [#492] and [#495] cover floating-point output, math, buffer formatting, bounded stdin and some file positioning. | [#312] owns overlapping CRT changes. C time [#502], wide conversions [#505] and wide streams [#508] have open PRs. General locale support and dynamically linked C++ compatibility remain open. |
| Exceptions and unwinding | [#452] and [#461] added synthetic C++ exceptions and conversion of selected guest CPU faults to structured exceptions. [#484] preserves explicit process exits from TLS in startup reports. | Swastickc owns [#476] and its [draft #479][#479]. The normal TLS callback return regression is reported there. Exit unwinds, nested exceptions and collided unwinds remain unsupported in that draft. |
| System information and localization | The guest machine profile, performance counters, system time and calendar conversions are bounded guest services. | [#312] includes processor features, topology and UTC time zone. A broader locale, encoding, and Win32 message/error translation pass still needs a scoped issue and fixtures. |
| Filesystem | Main has a confined synthetic drive and the first [#493] file batch via [#495]. | [#312] is still conflicting and includes host mounts, metadata, timestamps and moves. [#498], [#500] and [#501] cover further CRT file operations. Preserve existing drive and quota boundaries. |
| Public platform services | The [guest graphics command path](../runtime/guest-graphics.md) exercises a bounded synthetic D3D-style surface on native Metal. | Window/message dispatch, COM and registry services still need defined guest models. [#312] includes guest-only Winsock loopback. [#306] tracks the native audio path. |
| Coverage and reporting | The generated typed list and the live import probe have different scopes. Both distinguish stubs from functional handlers. | catminou4 owns [#465]. Coordinate with the different live registry report in [#312]. A wanted-list inventory, provenance, explicit status checks, and a stale-report gate are still tracked in #465. |
| Guest CPU | [#288] established the shared decoder and SSE2 basis. [#482], [#488] and [#497] added SSSE3 and SSE4.1. | jelly013 has SSE4.2 and further ARM64 lowering in [#499]. AVX, memory ordering [#199], broader floating-point control and exact decoding of unsupported forms still need their own evidence. |

### Verification and coordination

- On Apple M2 Max, macOS 27.2.1 and Rust 1.100.0, the previous main
  revision `ece8991` passed seven original C exception tests and both C++
  exception tests. The same main with [#479] applied passed eleven C
  exception tests and failed both C++ tests with guest status `0xc0000005`.
  The same eleven C passes and two C++ failures reproduced when [#479]
  was applied to `c76e974`. Three original CLI startup-report cases
  passed on the earlier combined tree.
- With LLDB, the uncaught C++ fixture reached the TLS return address
  `0xffffffffffff0000` before entry. [#479] routed that fetch through the
  exception dispatcher and latched a process exit. Stopping at the expected
  callback return before fetch is the proposed prerequisite. The DBT file
  overlaps [#499], so the fix needs coordination with both contributors.
- [#519] adds the two canonical services and four API-set aliases for
  [#512]. The original guest fixture passed at `-O0`, `-O1`, `-O2`
  and `-Os`. The workspace run for #519 passed 1,191 tests with 39
  ignored. The PR is open.
- Original cross-family fixtures exist for the
  [hosted multithreaded C++ program][#438] and the
  [guest-to-Metal workload][#433]. The campaign still needs additional
  combinations of module loading, exceptions, threading and file operations.
  Evidence applies to the named synthetic programs. It does not establish
  Xbox hardware or commercial game execution.

[survey]: https://github.com/ranveersoni98/Xenora/issues/459
[#199]: https://github.com/ranveersoni98/Xenora/issues/199
[#203]: https://github.com/ranveersoni98/Xenora/issues/203
[#207]: https://github.com/ranveersoni98/Xenora/issues/207
[#288]: https://github.com/ranveersoni98/Xenora/pull/288
[#306]: https://github.com/ranveersoni98/Xenora/issues/306
[#312]: https://github.com/ranveersoni98/Xenora/pull/312
[#339]: https://github.com/ranveersoni98/Xenora/pull/339
[#433]: https://github.com/ranveersoni98/Xenora/pull/433
[#438]: https://github.com/ranveersoni98/Xenora/pull/438
[#449]: https://github.com/ranveersoni98/Xenora/pull/449
[#452]: https://github.com/ranveersoni98/Xenora/pull/452
[#460]: https://github.com/ranveersoni98/Xenora/pull/460
[#461]: https://github.com/ranveersoni98/Xenora/pull/461
[#465]: https://github.com/ranveersoni98/Xenora/issues/465
[#469]: https://github.com/ranveersoni98/Xenora/pull/469
[#476]: https://github.com/ranveersoni98/Xenora/issues/476
[#479]: https://github.com/ranveersoni98/Xenora/pull/479
[#480]: https://github.com/ranveersoni98/Xenora/pull/480
[#482]: https://github.com/ranveersoni98/Xenora/pull/482
[#484]: https://github.com/ranveersoni98/Xenora/pull/484
[#486]: https://github.com/ranveersoni98/Xenora/pull/486
[#488]: https://github.com/ranveersoni98/Xenora/pull/488
[#489]: https://github.com/ranveersoni98/Xenora/pull/489
[#490]: https://github.com/ranveersoni98/Xenora/pull/490
[#492]: https://github.com/ranveersoni98/Xenora/pull/492
[#493]: https://github.com/ranveersoni98/Xenora/issues/493
[#495]: https://github.com/ranveersoni98/Xenora/pull/495
[#497]: https://github.com/ranveersoni98/Xenora/pull/497
[#498]: https://github.com/ranveersoni98/Xenora/pull/498
[#499]: https://github.com/ranveersoni98/Xenora/pull/499
[#500]: https://github.com/ranveersoni98/Xenora/pull/500
[#501]: https://github.com/ranveersoni98/Xenora/pull/501
[#502]: https://github.com/ranveersoni98/Xenora/pull/502
[#505]: https://github.com/ranveersoni98/Xenora/pull/505
[#508]: https://github.com/ranveersoni98/Xenora/pull/508
[#512]: https://github.com/ranveersoni98/Xenora/issues/512
[#519]: https://github.com/ranveersoni98/Xenora/pull/519

## Continuation checkpoint

- Continuation branch base: `0836936` (tip of `feat/cpu-runtime-expansion`), merged with main `d22f046` (#272, #273, #274) as `56febbb`. The merge had no conflicts.
- All recorded checkpoints (`a953ba9`, `9cd6f43`, `7e09c9c`, `bfd14e4`, `0836936`) exist upstream. The earlier session's uncommitted MOVSX patch was not available and was not used.
- Host: Apple M5 Pro, macOS, Rust 1.100.0, Apple clang 17.0.0. No Homebrew LLVM, `llvm-mc` or full Xcode.
- Baseline of the merge: `cargo test --locked --workspace --no-fail-fast` exited 101 with 392 passed, 1 failed, 21 ignored. The failure (`probe_fuzz_harness_minimized_artifact`) also occurs on `0836936`: the test read an untracked file.

## Corrections to earlier entries

- The earlier claim of 381 passed and 0 failed did not hold for a fresh checkout (see above).
- The SXTW encoding recorded as verified was wrong for most source registers.
- #65, #66 and #117 were marked complete prematurely. They stay open until reviewed against their own acceptance criteria.

## Milestones

| # | Commit subject | Scope | Evidence |
| --- | --- | --- | --- |
| 1 | cpu: correct the sign- and zero-extension family | MOVZX/MOVSX/MOVSXD, MOV-immediate regressions, SXTW, 32-bit access width, RIP-relative and LEA addressing, self-contained prefix test | `tests/movsxd.rs`: 9 tests over 93 LLVM-assembled register forms and 32 MOV immediates, each run by the interpreter and as native translated code; 16 of 16 targeted mutations caught |
| 2 | cpu: share one generated decoder and store guest flags | Registry, generator, shared decoder and semantics, integer expansion, architectural DBT flags, truthful probe support | `tests/x86_integer.rs`, `tests/arm64_encodings.rs`, `tests/x86_generator`; 37 targeted mutations: 36 caught (4 only after new tests), and one equivalent, whose redundant check was removed |
| 3 | cpu: add SSE2 state, moves and packed integer forms | XMM and MXCSR state, mandatory prefixes, SSE/SSE2 moves and packed integer operations, NEON lowering | `tests/x86_vector.rs` (9 tests, 374 LLVM-assembled instructions, 64 states each), `tests/arm64_encodings.rs`; 35 targeted mutations: 34 caught (1 only after a new test) and one equivalent |
| 4 | cpu: add exact SSE and SSE2 floating point | Software binary32/binary64 arithmetic with MXCSR, compares and conversions | `tests/x86_float.rs` (9 tests, 6,824 exact-fraction vectors, 172 LLVM-assembled instructions, over 2 million host comparisons); 53 targeted mutations: 52 caught (2 only after directed vectors), and one changes a rule deliberately left unasserted |
| 5 | runtime: one import resolver and typed service adapters | Bounded import parser shared by probe and binder, strict registry, transactional binding, Win64 preservation check, typed registry and generator, 20 services | `tests/runtime_services.rs` (19 tests; every service is called by translated guest code), generator unit tests (4); 33 valid targeted mutations: all caught, 2 only after new cases |

Encodings: `clang -target x86_64-apple-macos -c` and `clang -target arm64-apple-macos -c`, read back with `objdump -d`.

## Verification of milestone 2 (local, real exit codes)

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check`; strict Clippy, workspace, all targets and features | exit 0; exit 0 |
| `cargo test --locked --workspace --no-fail-fast` | exit 0: 409 passed, 0 failed, 21 ignored |
| `generate_x86.py --check`, `assemble.py --check`, generator tests (8) | exit 0 |
| RDNA2 metadata, semantic registry, coverage and status checks; their Python tests | exit 0 |
| `scripts/audit/{claims,dependencies,native}.sh`, release build, `shaders coverage` | exit 0 |
| `scripts/test-gpu.sh` | first run exit 124 (its own timeout, in `sop2_arith`, straight after a release build); rerun exit 0: 69 passed, 0 failed, 1 ignored |
| `scripts/demo.sh`, `scripts/build-app.sh`, `--smoke-frame`, `--inspect` | exit 0 |
| `scripts/assemble-fixture.sh --check`; ignored LLVM tests in `xenora-isa` and `xenora-decoder` | not verified: need Homebrew LLVM (`llvm-mc`) |
| Fuzz and mutation campaigns | not run |

Coverage: 56 operations, 184 declared forms, 279 opcode slots. 985 LLVM-assembled instructions execute every form except control flow and exits, which have their own tests.

`dbt_bench` on an Apple M5 Pro, 101 samples, three runs each of `5ab561b` and milestone 2, middle run: interpreter 12.69 to 11.12 MIPS, cold DBT 10.43 to 8.86 MIPS, warm DBT 468.83 to 424.86 MIPS, native code 242,608 to 290,184 bytes. The slowdown comes with the new work of storing six flags and the shared decode; the earlier figures belong to code that computed wrong flags. Not optimized yet. These are synthetic and say nothing about games.

## Verification of milestone 3 (local, real exit codes)

Main `19bea60` (#278 and QA tooling) was merged first as `70f1793`; it does not touch `crates/loader`. `CHANGELOG.md` and `docs/README.md` conflicted and both sides were kept.

| Gate | Result |
| --- | --- |
| Formatting; strict Clippy, workspace, all targets and features | exit 0; exit 0 |
| `cargo test --locked --workspace --no-fail-fast` | exit 0: 429 passed, 0 failed, 22 ignored |
| `generate_x86.py --check`, `assemble.py --check`, generator tests (10) | exit 0 |
| RDNA2 metadata, semantic registry, coverage and status checks; their Python tests; claims, dependencies and native audits; release build | exit 0 |
| `scripts/test-gpu.sh`, `demo.sh`, `build-app.sh`, `--smoke-frame`, `--inspect` | exit 0; GPU 74 passed, 0 failed, 1 ignored |
| `scripts/assemble-fixture.sh --check` and ignored LLVM AMDGPU tests | not verified: need `llvm-mc` |
| Fuzz and mutation campaigns | not run |

The registry now has 139 operations, 306 forms and 401 opcode slots. `dbt_bench`, integer only, middle of three runs: interpreter 11.18, cold DBT 8.62 and warm DBT 430.19 MIPS, with the same 290,184 native bytes as milestone 2. No SIMD throughput benchmark exists yet.

## Verification of milestone 4 (local, real exit codes)

| Gate | Result |
| --- | --- |
| Formatting; strict Clippy, workspace, all targets and features | exit 0; exit 0 |
| `cargo test --locked --workspace --no-fail-fast` | exit 0: 438 passed, 0 failed, 22 ignored |
| `generate_x86.py --check`, `assemble.py --check`, `float_reference.py --check`, generator tests (10) | exit 0 |
| RDNA2 metadata, semantic registry, coverage and status checks; their Python tests; claims, dependencies and native audits; release build | exit 0 |
| `scripts/test-gpu.sh`, `demo.sh`, `build-app.sh`, `--smoke-frame`, `--inspect` | exit 0; GPU 74 passed, 0 failed, 1 ignored |
| `scripts/assemble-fixture.sh --check` and ignored LLVM AMDGPU tests | not verified: need `llvm-mc` |
| Fuzz and mutation campaigns | not run |

The registry now has 191 operations, 358 forms and 453 opcode slots. `dbt_bench`, integer only, middle of three runs: interpreter 11.32, cold DBT 8.81 and warm DBT 430.19 MIPS (one warm run read 377.43). No floating-point or SIMD throughput benchmark exists.

Main moved to 15 commits past this branch after milestone 3; they do not touch `crates/loader` and are not merged yet.

## Verification of milestone 5 (local, real exit codes)

Main `f685ede` (QA tooling) was merged first as `5e7c21b`.

| Gate | Result |
| --- | --- |
| Formatting; strict Clippy, workspace, all targets and features | exit 0; exit 0 |
| `cargo test --locked --workspace --no-fail-fast` | exit 0: 462 passed, 0 failed, 22 ignored |
| `xenora-runtime-gen --check`; CPU generator, assembler and float reference checks; generator tests | exit 0 |
| RDNA2 metadata, semantic registry, coverage and status checks; their Python tests; claims, dependencies and native audits; release build | exit 0 |
| `scripts/test-gpu.sh`, `demo.sh`, `build-app.sh`, `--smoke-frame`, `--inspect` | exit 0; GPU 74 passed, 0 failed, 1 ignored |
| `scripts/assemble-fixture.sh --check` and ignored LLVM AMDGPU tests | not verified: need `llvm-mc` |
| Fuzz and mutation campaigns | not run |

`dbt_bench`, middle of three runs: interpreter 11.08, cold DBT 8.48 and warm DBT 409.65 MIPS. Warm runs read 392 to 417, against 430 for milestone 4; no change here is known to affect that path and the cause was not investigated.

The panic of #270 was reproduced on `5e7c21b` (`range start index 1025 out of range`) and does not occur after this milestone.

## Integration of main `c6a354b`

Merged after milestone 5. It carries #280 (#72), #281 (#66) and #283 (#65), published while this branch was not; those issues are not claimed here and the published work takes precedence in authorship.

- Kept: every upstream test, unchanged (`tests/bootstrap.rs` and the three new tests in `dbt.rs`), and the assembler's `sxtb_32` and `sxth_32`, now checked against LLVM.
- Source: `cpu.rs` and `dbt.rs` keep the shared decoder and lowering, which already cover the merged behaviour.
- Adopted from upstream: FS and GS together are undefined, and GS applies. This branch used to reject the combination. RIP-relative, negative-displacement and fault-address regressions for both segments were added (11 encodings, both engines); 3 of 3 mutations of the rule caught.
- Compared: the integer fixture run through both trees in both engines. Upstream supports 142 instruction and engine pairs. They differ only where upstream stores no guest flags in the DBT (#64, #70), reads AH, CH, DH and BH as SPL, BPL, SIL and DIL (#68, including MOVZX and MOVSX in both engines), or writes eight bytes for a 32-bit store.
- Gates: workspace 474 passed, 0 failed, 22 ignored; formatting, strict Clippy, all generator checks, repository audits, release build, GPU suite, demo and app gates exit 0. GPU: 80 passed, 0 failed, 1 ignored.

Main `4db0b9c` (#286, #287) was merged next. #286 fixed #270 first; its test and its bounds check on section offsets are kept, and imports stay on the shared parser. Gates on that merge: workspace 477 passed, 0 failed, 22 ignored; GPU 84 passed, 0 failed, 1 ignored; formatting, strict Clippy, generator checks, audits, release build, demo and app gates exit 0.

## Known limitations

- Only the forms in `x86/instructions.toml` exist. The RCP and RSQRT approximations, SSE3 and later, MMX, VEX, string instructions, RCL, RCR, SHLD, SHRD, LOCK, fences and atomics are unsupported.
- Every floating-point form runs through software arithmetic in the DBT; none is lowered to NEON. No x86 hardware comparison exists, and `docs/cpu/instruction-registry.md` lists the choices left unasserted.
- Packs, shuffles, high multiplies, PSADBW, sign-mask extraction, word insertion and extraction, byte shifts and shifts by a register count run through the shared semantics in the DBT.
- The fixture graphics services, `XMemAlloc` and the `XMemFree` stub are outside the typed registry. Several older generated conformance tests check Rust library functions, not services.
- Last error is one value per process; per-thread isolation (#202) waits for guest threads.
- 8- and 16-bit ALU operations, shifts, MUL, DIV and bit operations run through the shared semantics in the DBT, not as ARM64.
- The reference and the implementation share an author.
- New CPU checks are not in CI; `.github` was left unchanged.

## Pending acceptance

- Two independent reviews required by `AGENTS.md`: not performed.
- Issues touched here have not been claimed on GitHub and stay open.

## Next task

Agree with the maintainer and the contributor of #280, #281, #283 and #286 how this branch should be offered, before more overlapping work. Then native CRT memory and string services, which are not started.
