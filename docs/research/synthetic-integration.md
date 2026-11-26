# Synthetic guest integration and upstream evidence

## Source snapshots

The repository audit supplied for this session inspected these public
snapshots on 2026-11-23. The observations describe source or test assertions.
External executables and retail games were not run. Licenses are recorded at
repository level and grant no file-level reuse clearance. The implementation
in this checkout uses original fixtures and public specifications.

| Project and audited snapshot | Type and observed evidence | Claimed-only behavior or provenance risk | Xenora work and original acceptance fixture | Priority and owner |
| --- | --- | --- | --- | --- |
| [Xenora `3133d8e`](https://github.com/ranveersoni98/Xenora/tree/3133d8e) | Native Rust guest and shader research. X1–X4 fixtures and gfx1030 compiler in tree. | GPL-3.0-or-later. Console execution unsupported. | Re-run the guest and translated compute fixtures on ARM64. | Immediate, integration |
| [Xbox2Windows `6de5738`](https://github.com/R0Y00/Xbox2Windows/tree/6de5738) | Static recompilation tests for imports, CPU startup, TLS and GPU output. See `tests/cpu/startup_exe.cpp`. | MIT at repository level. Full title startup unverified. | Build a source-authored PE guest and test exact service results. | Immediate, CPU |
| [americas-recomp `9bf220e`](https://github.com/realdody/americas-recomp/tree/9bf220e) | Audited main root had a license file. | GPL-2.0. Other branches uninspected. | No fixture justified by audited tree. | Deferred, research |
| [6stars `480d9a4`](https://github.com/meetox80/6stars/tree/480d9a4) | Swift/Metal renderer and sampler mapping in `Sources/SixStars`. | No verified root license. Guest CPU execution unverified. | Original translated shader, validated resource range and known readback. | Immediate, Metal |
| [PercyStar `6a64e53`](https://github.com/rpatty-dev/PercyStar/tree/6a64e53) | Rust pipeline cache and texture exploration with RDNA2 disassembly. | Custom non-commercial terms. Archive and shader provenance require review. | Use synthetic shader streams and unrelated original texture bytes. | Later, shader and textures |
| [Percy6Stars `be8adf8`](https://github.com/Ahmet-Dedeler/Percy6Stars/tree/be8adf8) | Derivative viewer and media GUI. | Custom non-commercial terms, overlapping PercyStar lineage. | No separate execution fixture justified. | Deferred, research |
| [XCL `dd713e3`](https://github.com/ivall/xcl/tree/dd713e3) | Windows mapper and resolver in `crates/xcl/src/loader.rs`, test titles in `crates/xcl/tests/e2e.rs`. | GPLv2 root, compatibility unreviewed. README playability claims unverified. | Named import resolution and pre-entry rejection using original PE files. | Immediate, CPU |
| [TomBox `4f66ea9`](https://github.com/drycheetah84/TomBox/tree/4f66ea9) | Rust PE parser and strict shader failure tests. | No verified root license. Lobby startup reported in README. | Explicit diagnostics for unsupported imports and shader operands. | Later, CPU and shader |
| [xbox-emulator `6e81ffc`](https://github.com/omonkeyz/xbox-emulator/tree/6e81ffc) | Windows compatibility modules and service handlers. | GPL-3, TomBox-derived. Retail claims unverified. | Isolated service ABI tests when public contracts exist. | Later, CPU |
| [ScarlettX `01c642c`](https://github.com/Freminet/ScarlettX/tree/01c642c) | Windows loader, auto-stubs and synthetic `hello_scarlett` triangle. | No verified root license. Retail execution reported in README. | Translate an original shader and verify pixels. Unsupported services fail explicitly. | Later, Metal |
| [OpenScarlett `c7a7e46`](https://github.com/californiaAlmonds/OpenScarlett/tree/c7a7e46) | C++ PE inspection and RDNA2 lifter in `src/gpu/rdna2_lifter.cpp`. | MIT at repository level. Commercial execution unverified. | Validate Wave32 control flow against AMD and independent reference. | Immediate, shader |
| [AnyXboxX `8816a7c`](https://github.com/LooperSalty/AnyXboxX/tree/8816a7c) | Unicorn model, HLE and Rust BC7 module. | GPL-3. Dependencies and attributed code need file-level review. | Generate original malformed memory and texture inputs. | Later, CPU and textures |
| [SixNG `28c0129`](https://github.com/holdmysocks/SixNG/tree/28c0129) | Archive and asset tools, including `crates/sixng-core/src/gfx10.rs`. | GPL-3. Format-fixture provenance unreviewed. | Test original synthetic mip and tile addressing with independent offsets. | Immediate, textures |
| [bcpack-rs `c51ec52`](https://github.com/Ahmet-Dedeler/bcpack-rs/tree/c51ec52) | rANS and BC1 code with `tests/roundtrip.rs`. | MIT OR Apache-2.0 at repository level. Table provenance unreviewed. | Public BC1 palette and malformed-block tests using original bytes. | Immediate, textures |
| [ttpf-viewer `bafe9af`](https://github.com/paryzius/ttpf-viewer/tree/bafe9af) | Rust texture and panorama tooling. | MIT at repository level. No guest execution evidence. | Test strict original texture metadata bounds before considering public container contracts. | Later, textures |
| [PercyScarlett `5903e0b`](https://github.com/rpatty-dev/PercyScarlett/tree/5903e0b) | Rust shims and synthetic PE test in `tests/e2e_bringup.rs`. | Custom non-commercial license despite the audited project description. Synthetic execution reported in README. | Compile and inspect a distinct original import fixture. | Immediate, CPU |
| [FEX `727948a`](https://github.com/FEX-Emu/FEX/tree/727948a) | Linux x86-64-to-ARM64 translation, intermediate representation and JIT. | MIT at repository level. Different host ABI and memory model. | Preserve interpreter known answers before evaluating any native translation. | Later, CPU |
| [silicon-rans](https://github.com/microsoft/silicon-rans) | Hardware rANS example and documentation. Audit did not pin a SHA. | MIT at repository level. Relevance to console textures unverified. | No codec fixture required for BC1 block decoding. | Deferred, textures |
| [LLVM](https://github.com/llvm/llvm-project) | Compiler and assembler used as a separate PE and AMD GPU instruction oracle. Audit did not pin a repository SHA. | Apache-2.0 with LLVM exceptions. File terms still apply. | Compare fixture bytes and disassembly using the installed LLVM 23.1.6. | Immediate, CPU and shader |
| [wgpu / Naga](https://github.com/gfx-rs/wgpu) | Shader IR and reflection reference. Audit did not pin a SHA or inspect individual files. | Apache-2.0 at repository level. Xenora's Metal backend remains direct. | Original reflection and binding tests once the fixture ABI expands. | Later, shader |

## Commercial startup gap

- The [guest experiments](../../experiments/x1/README.md) cover bounded
  synthetic CPU execution, fixed-base executable loading, service dispatch,
  native compute and an offscreen render. See the [baseline acceptance
  record](../testing/phase-02-baseline.md#track-b--guest-execution) for X1–X4.
- The X6 fixture reads and writes mapped guest regions, exercises function
  calls and returns, and checks output and the number of retired
  instructions. The [X7 fixtures](../../tests/compat_fixtures/README.md)
  exercise named imports in an independent compiler output. The positive
  guest invoked both services and returned 36. The unresolved-import guest
  failed before entry.
- Apple M5 readback provides class A evidence for the synthetic guest compute
  import, partial Wave32 arithmetic, and native BC1 texture reads. The
  source-compiled PE guest submits a writable range and one-shot descriptor,
  reports the three translated XOR results, and returns their sum. The
  Wave32 check covers inactive-lane preservation across 67 invocations.
  The BC1 test checks 61 texels using authored Metal Shading Language
  and the synthetic tiled layout. The layout has no validated console
  equivalent. See the [guest compute gate](../../tests/compat_fixtures/verify-compute.sh)
  and [compute and texture interfaces](../interfaces.md).
- AMD's published RDNA 2 ISA and the public PE format provide class B
  evidence for their documented contracts. Console loader behavior, guest
  services, hardware queues, formats and memory initialization need their
  own sources and tests. [Xbox GPU evidence](../research/xbox-gpu.md) ·
  [PE format](https://learn.microsoft.com/en-us/windows/win32/debug/pe-format)
- Commercial game startup and execution remain unsupported. No authorized retail
  executable was loaded. Actual title imports, startup ordering, resource
  bindings and pipeline behavior remain unverified. Relocation, thread-local
  storage, process and thread services, dynamic shader execution and
  GPU synchronization need independent fixtures before any title-specific
  readiness claim.

## Local verification

The checks ran on native ARM64 macOS. GPU results apply to the Apple M5
test host. The local checks do not change release acceptance.

| Check | Result |
| --- | --- |
| `cargo run --locked --manifest-path experiments/x1/Cargo.toml --target aarch64-apple-darwin -- x6` | Exit 0. Stored 0, 2, 4, 6 and retired 12, 16, 20, 24 instructions. LLVM disassembly and a separate control-flow trace agreed. |
| `tests/compat_fixtures/verify.sh` | The positive guest passed `[4, 10, 22]` through both services and returned 36. The unresolved-import case returned host exit 1 and produced no guest output. |
| `scripts/test-gpu.sh` | Exit 0. Fourteen tests passed, including the translated XOR shader and nine invalid views checked in a single test. |
| `MTL_DEBUG_LAYER=1 MTL_SHADER_VALIDATION=1 cargo test --locked -p xenora-semantic-review wave32_masked_ashr_native_metal_known_answers -- --ignored --nocapture` | Exit 0. The translated Wave32 control-flow fixture matched native readback. |
| `cargo test --locked --workspace` | Exit 0. Includes five CPU-only tiled texture and BC1 tests. |
| `scripts/demo.sh` and `scripts/audit/native.sh` | Exit 0. All three demos matched known answers, and 16 binaries and loadable dependencies passed native inspection. |
