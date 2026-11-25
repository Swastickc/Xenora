# Independent review

## gfx1030 form acceptance

Use the [AMD RDNA2 desktop GPU (gfx1030) form acceptance rule](shader-form-acceptance.md) for entries that
promote instruction forms to `fixture verified`. Record only completed runs with
a named commit, independent runner, host, exact commands and real counts. Raw
logs stay under ignored `artifacts/acceptance/<commit>/`.

## R1. First synthetic compute checkpoint, 2026-11-22

Reviewed the integrated working tree before a candidate commit. The findings below cover the first checkpoint.

| Check | Observed |
|---|---|
| `cargo fmt --all -- --check` | Exit 0. |
| `CARGO_BUILD_JOBS=2 cargo clippy --workspace --all-targets -- -D warnings` | Exit 0. |
| `CARGO_BUILD_JOBS=2 cargo test --workspace` | Exit 0. 11 passes, one opt-in LLVM oracle ignored. Raw output in local `artifacts/r1-tests.log`. |
| `cargo test --workspace --no-run` plus native compute test build | Nine test executables discovered. |
| `file`, `lipo -archs`, and `otool -L` on the CLI and nine test executables | All Mach-O arm64. Metal, Foundation, Objective-C, and libSystem linkage where applicable. No Vulkan or MoltenVK linkage. Local output in `artifacts/r1-native.log`. |
| `scripts/audit/dependencies.sh` | Exit 0. Checked five graphics crates. |
| `scripts/assemble-fixture.sh --check` | Exit 0. |
| `scripts/audit/claims.sh` | Exit 0. Ten claim IDs reviewed structurally at this checkpoint, with B=1, D=6, E=2 and U=1. |
| `git diff --check` | Exit 0. |

Inspected Git history filenames and searched revision diffs for common private key, token, and home-directory path patterns. No matches in the inspected diffs. Local build output, local lab evidence, and installed skill files were ignored. The history check must be repeated after the candidate commit includes the implementation.

The AMD XML source has the MIT notice preserved in the ISA crate. The Cargo compatibility inventory lists the known dependencies and their selected license expressions. The independent release check requires the top-level third-party notices and the preserved applicable upstream license notices before artifact distribution.

The local execution transcript records Apple M5, Metal API and GPU validation banners, and the four matching words for the one translated fixture. This review did not independently consume the GPU slot. At review start, `docs/status.toml` still described synthetic execution as pending, and the claim registry did not include native smoke execution or RDNA semantics. Update status after reconciling the test evidence, and register the hardware claims with exact scope. The structural lint cannot detect new prose claims that lack claim IDs.

The published D3D12 specification applies to its recorded API, and the LLVM processor table describes its supported tool target. Neither supplies the console ABI. [D3D12 Resource Binding Functional Spec](https://microsoft.github.io/DirectX-Specs/d3d/ResourceBinding.html), [LLVM AMDGPU processor table](https://llvm.org/docs/AMDGPUUsage.html#processors), [AMD machine-readable ISA](https://gpuopen.com/machine-readable-isa/)

The full G3 suite, three-shader G4 demo, mutation, fuzz and benchmarks remain outside this checkpoint evidence. A final R1 pass must run against the release candidate after which the gates can be evaluated.

## R1. Three-fixture checkpoint, 2026-11-22

The implementation was still in the working tree at review start. Results below are checkpoint evidence. Gate evaluation requires a candidate commit and matching logs.

| Gate | Checkpoint evidence and remaining work |
|---|---|
| G1 | The first checkpoint's ten inspected executables were arm64. Repeat the inventory for the current CLI, workspace tests, GPU tests, validation tests, and fuzz tooling using the exact candidate outputs. |
| G2 | The first checkpoint passed formatting, Clippy, and workspace tests. Repeat on the candidate after the compiler and validation suite changes. |
| G3 | The lead's local `artifacts/native-gpu-current.log` shows Apple M5, API and GPU validation, and 12 passes with zero skips, including offscreen vertex/fragment output. R1 did not independently rerun GPU workloads. |
| G4 | The lead's `artifacts/three-shader-demo.log` shows three matching outputs on Apple M5. Run the documented command from a clean candidate checkout and align the README and status scope with all three fixtures. |
| G5 | R1 reran `cargo test --locked --manifest-path tests/validation/Cargo.toml`, with four passes and zero failures. R2 owns the semantic decision and details. |
| G6 | The LLVM oracle and independent interpreter checks provide evidence. Require a single three-way comparison and report that maps every supported instruction and operand encoding to the current fixtures and explains coverage exclusions. |
| G7 | No completed candidate mutation score or survivor triage verified by R1. |
| G8 | `scripts/audit/claims.sh` exited 0 with 13 IDs (A=2, B=2, D=6, E=2, U=1). Claims/status in the reviewed snapshot still describe the smoke test and integer-add slice. Record the expanded host observations, floating-point domain, and control-flow semantics with scoped IDs. |
| G9 | Fuzzing was running for an earlier snapshot. Require logged final campaigns covering each parser at the candidate. |
| G10 | No candidate benchmark report or raw measurements verified by R1. |
| G11 | Runtime dependency notices, AMD MIT text, and top-level notices now present. The fuzz dependency graph and bundled libFuzzer notices were absent from the reviewed license inventory. Upstream objc2's SDK redistribution note remains relevant before distributing a linked binary. |
| G12 | Existing history scan from the first checkpoint predates the implementation. The documented standalone validation command creates a Git-visible `tests/validation/target/` build tree unless explicitly ignored. Repeat a hygiene scan on the candidate after the ignore fix. |
| G13 | At review start the README described only integer addition, status described the initial smoke and slice, and progress described IR and readback as pending. Reconcile the checkout docs, interface contract, and coverage table with the candidate. |
| G14 | CPU boundary ADR present. `scripts/audit/dependencies.sh` exited 0, checking five graphics crates. Rerun on the candidate. |
| G15 | This is a checkpoint review. Final R1 and R2 sign-off remains contingent on candidate evidence and finding closure. |

The runtime lockfile resolves five third-party packages and the standalone validation lockfile has none. The separate fuzz lockfile resolves arbitrary 1.5.0, cc 1.11.3, cfg-if 1.0.5, find-msvc-tools 0.1.15, getrandom 0.4.3, jobserver 0.1.35, libc 0.2.191, libfuzzer-sys 0.4.13, r-efi 6.0.0, and shlex 2.0.1. The root advisory report showed no known advisories. R1 independently ran `cargo audit --no-fetch --json --file fuzz/Cargo.lock`, which exited 0. Its database revision and scope appear in local `artifacts/r1-fuzz-audit.json`.

Compared the preserved runtime notices against pinned upstream license files. The selected license expressions are compatible with GPL-3.0-or-later; this finding does not resolve the upstream SDK-derived-code distribution question. [objc2 0.6.5 notice](https://raw.githubusercontent.com/madsmtm/objc2/d7d2fa23ceaa5e6096c923b081040e5d81b3b9df/LICENSE.md), [objc2-encode 4.1.0 notice](https://raw.githubusercontent.com/madsmtm/objc2/8d214f5477365ffcbcbb7de058c86ed9a518efb7/LICENSE.md), [Foundation and Metal bindings notice](https://raw.githubusercontent.com/madsmtm/objc2/7b1abfd750a2cacaea71d6a56ecfb83cb7de560b/LICENSE.md), [bitflags MIT license](https://raw.githubusercontent.com/bitflags/bitflags/80ce9b545acb0bd42150695fc351889cac1d8eb4/LICENSE-MIT)
## Phase 01 assurance review, 2026-11-22

R2 independently reviewed all 399 source mutations at `aff46b0`.
The audited numerator is 359 named test failures. All 13 raw misses have
individual equivalence proofs. The 25 invalid mutations all failed Rust
compilation, and the two non-progress outcomes remain in the denominator.
The conservative score is 359/361 (99.45%). The full ledger and exact
commands are in the [mutation report](mutation.md).

R1 independently reviewed the 12 native semantic mutations. Every
unmodified GPU workload matched its known answers. The mutated
submissions completed with both validation layers active and differed
from those answers, as required by the negative test. No invalid
dispatch, compiler error, crash or timeout was counted as a kill.

R2 rederived arithmetic, register and memory behaviors for the bounded
gfx1030 scope. R1 independently rederived the 20 FP32 boundary answers
from rational arithmetic and reviewed the independent integer oracle.
The report maps all 15 forms to their evidence. See the
[reference validation report](../../tests/validation/README.md).

R1 ran the claims and generated status checks at `5c47900`. Both passed,
with 20 registered claims and all 15 instruction forms represented.
The signed, next-PC-relative dword branch displacement description needs
an explicit claim ID. Scope the interpreter prose to documented desktop
RDNA2 and unknown Xbox applicability. The generated status and supported
feature descriptions passed review.

R1's strict Git integrity and pattern scans across 20 reachable commits
found no matching credentials, private home paths or tracked build
artifacts. Missing post-policy DCO attestations remain unresolved.
Preserve the historical commit objects and use authentic attestations.

R1 confirmed that the preserved AC report files match the acquisition
copies byte for byte. The logged CPU load leaves replay idleness
unestablished. G10 and G11 remain open. More protocol and corpus work is
required for the former, and linked SDK-derived rights and attribution
remain unresolved for the latter.

## Phase 01 evidence recheck at 4b9a4ce

R1 signed off G8 for the Phase 01 documentation after checking the
desktop RDNA 2 branch and wait evidence, interpreter scope and
Microsoft's CPU disclosure. The claims audit passed with 22 entries
and generated status matched. Phase 02 claims require separate review.

G12 remains blocked. Of 13 contributions after the policy commit
`9ab5bf9f9e812685883f3239319c10aa2a19f7d5` through `4b9a4ce`, seven
have matching author trailers and six lack sign-off:

- `79df1194ebd5c076370d2814adecd7869e6d48a2`
- `095ec65aee69998b48d67c1f84c60d6b3224d9fe`
- `cc0daa79e63c468e748f0333bd1314224752893a`
- `aff46b0904a0182a6f0543125e4491ab67270f64`
- `5c479005b0e0631a608a369b9e41327da61bb61e`
- `4b9a4cecf35c8d4d90d8ecdb9c1510e7bd260102`

The recorded contributor must personally review and attest to each
change. The full recheck is in `artifacts/r1-phase01-g8-g12.md`.
