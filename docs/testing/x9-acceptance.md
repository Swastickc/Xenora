# Persistent resources and translated rendering acceptance

The upstream observations below describe pinned source inspected for this
milestone. Xenora tests use original fixtures. The upstream tests were not
run here. PASS records a native ARM64 run with assertions, FAIL a reproduced
mismatch, UNIMPLEMENTED absent behavior, and BLOCKED a dependency or host
limit. A CPU rejection pass does not certify GPU execution.

| Upstream reference | Tested behavior | Xenora equivalent | Current result | Blocker |
| --- | --- | --- | --- | --- |
| [Xbox2Windows startup](https://github.com/R0Y00/Xbox2Windows/blob/cea1d927f891007262af75e0d907fddcc511b424/tests/cpu/startup_exe.cpp) | CPU startup exercises loading two DLLs, thread notifications and wait failures. | Original named-import PE32+ startup and missing import control. DLL and thread-local storage ordering are separate gates. | UNIMPLEMENTED for DLL and thread ordering | Bounded fixed-base guest profile. |
| [Xbox2Windows wave shuffle](https://github.com/R0Y00/Xbox2Windows/blob/cea1d927f891007262af75e0d907fddcc511b424/tests/wave_shuffle.cpp) | D3D12 dispatch maps output by reported wave and lane, checks affine and rotation shuffles plus mirror and uniform controls, and rejects duplicate lanes or wrong widths. Forced variants require device capability. | ARM64 CPU compiler rejects an original gfx1030 cross-lane permute with a supported compile control. Native Wave64 shuffle needs a two-wave exact oracle. | PASS for rejection. UNIMPLEMENTED for Wave64 Metal dispatch. | No cross-lane lowering or Wave64 execution contract. |
| [Xbox2Windows compute services](https://github.com/R0Y00/Xbox2Windows/blob/cea1d927f891007262af75e0d907fddcc511b424/tests/cpu/shim_compute_check.py) | Checks allocation, dispatch and free observations. Independently calculates four readback words. Provider rejection must produce zero output bytes. Native hardware is selected for the explicit hardware case. | Exact guest XOR readback, invalid import commands and observable destroy/reuse. | PASS for X8 XOR and malformed commands | Persistent resources have a separate X9 gate. |
| [Xbox2Windows pixel attributes](https://github.com/R0Y00/Xbox2Windows/blob/cea1d927f891007262af75e0d907fddcc511b424/tests/pixel_attribute_check.py) | Compiles and executes an interpolated pixel function. Tests invalid attribute and register operands. A deliberately wrong interior channel must fail the image check. | Original gfx1030 paired interpolation and vertex export; independent interior pixel expectations and a wrong-image comparison. | PASS on Apple M5 | One linked four-component varying. |
| [Xbox2Windows texture LOD](https://github.com/R0Y00/Xbox2Windows/blob/cea1d927f891007262af75e0d907fddcc511b424/tests/pixel_sample_lod_check.py) | Checks explicit level-of-detail and bias against fixed pixel answers and rejects wrong mip, invalid operands and missing waits. | Original translated sampling fixture with distinct mip colors and invalid-pitch controls. | UNIMPLEMENTED | No translated image sampling. |
| [XCL synthetic startup](https://github.com/ivall/xcl/blob/60599a948f0ec49dfce4367082928eaef9d8a7ce/crates/xcl/tests/e2e.rs) | Test executable completes with zero failures and leaves the input tree unchanged while changes appear in an overlay. The graphics test checks backend and frame reports. | Original synthetic PE entry and guest status. A filesystem overlay is a separate feature. | UNIMPLEMENTED for overlay | Guest services have no virtual filesystem. |
| [XCL Wave64 compute](https://github.com/ivall/xcl/blob/60599a948f0ec49dfce4367082928eaef9d8a7ce/crates/xcl/tests/gpu_compute_e2e.rs) | Requires DXC and D3D12 device, asserts translated pipeline selection and checks 768 coordinate and cross-thread local data share outputs. | Require actual generated pipeline and exact Metal readback. | UNIMPLEMENTED for Wave64 local data share | No local data share, barriers or Wave64 lowering. |
| [OpenScarlett parity](https://github.com/californiaAlmonds/OpenScarlett/blob/1ffd9396ab7bc5c8d6ec51621fde3ccf7cffd1b1/tests/test_rdna2_corpus_parity.cpp), [live lifter](https://github.com/californiaAlmonds/OpenScarlett/blob/1ffd9396ab7bc5c8d6ec51621fde3ccf7cffd1b1/tests/test_rdna2_lifter_live.cpp) | Compares CPU and D3D12 hardware or WARP outputs for Wave32 isolation. Live tests exercise Wave64 across narrower host waves and gate backend-dependent operations. | Original 67-word partial-execution fixture with inactive-lane preservation and independent CPU expected values. | PASS for Wave32 on Apple M5 (X8) | Dynamic divergence and Wave64 remain unsupported. |
| [TomBox synthetic shaders](https://github.com/drycheetah84/TomBox/blob/4f66ea98423e02e1cbb1d6d068e55e3a98d408fd/crates/tombox-xshader/tests/synthetic.rs) | CPU tests reject unresolved inputs and unsupported mesh semantics. An opt-in WARP test checks four output words. | Reject unsupported instructions and verify actual translated output using Metal. | PASS for X8 compute subset | Mesh stages remain unsupported. |
| [bcpack-rs round trips](https://github.com/Ahmet-Dedeler/bcpack-rs/blob/c51ec521cbe6869136f29f5d157342374ef5cf33/tests/roundtrip.rs) | CPU tests check exact decoded data, malformed spans and invalid mip-chain dimensions. | Original Block Compression 1 (BC1) mip/layer fixture with exact RGBA8 readback and invalid layer, mip, extent and truncated data controls. | PASS for X8 tiled and X9 pitched Metal uploads. UNIMPLEMENTED for translated guest texture sampling. | Requires image instruction lowering and linked guest texture bindings. |
| [6stars renderer](https://github.com/meetox80/6stars/blob/480d9a4d00ed11560f7b4d989643723a0f5c451c/Sources/SixStars/Renderer.swift) | Native Metal renderer builds pipeline states and binds buffers and a preview texture. The inspected renderer contains no independent image assertion. | Submit translated vertex and pixel stages through an offscreen Metal pipeline and compare every pixel against an independent oracle. | PASS on Apple M5 | One offscreen RGBA8 target and one triangle per call. |

The compute ABI explicitly selects Wave32 semantics. In that mode the upper
execution-mask and vector-condition halves do not select lanes. Raw gfx1030
instruction bytes do not declare a wave width. Wave64 needs an explicit mode,
two 32-bit halves and an independent oracle covering both cross-half traffic
and isolation at wave boundaries.

## Native Metal readback

`CARGO_BUILD_TARGET=aarch64-apple-darwin scripts/test-gpu.sh` exited 0 on
Apple M5 with 20 passes, zero failures and zero ignored tests. Both Metal
validation layers were enabled. The shader stages came from the original
[assembled gfx1030 fixtures](../../fixtures/manifest.toml).

- Constant color and clipping checked 12 opaque red pixels and 12 exact
  clear pixels in separate draws.
- Translated vertex arithmetic checked 3 red and 13 clear pixels. The same
  input with a pass-through vertex stage checked 7 red and 9 clear pixels.
  Neither comparison places an edge on a pixel center.
- Perspective interpolation checked 16 pixels with differing clip-space
  W values and 16 more after swapping the vertex red and green channels.
  A rational CPU calculation determines the expected bytes and avoids
  half-byte rounding ties. Every channel was checked exactly.
- Pitched BC1 checked 33 texels across mip levels and layers, including
  partial edge blocks, transparency and a bad layer control. The upload
  uses a fixed authored Metal texel reader. A translated sampling
  instruction remains unimplemented.

## Guest resource and draw gates

The guest service uses an eight-slot generation-checked table. A compute
or draw request captures a checked read-only view. Completion writes
the result to a writable guest span. The guest PE uses separate code and
data sections, and no native host pointer crosses that boundary.

| Upstream reference | Tested behavior | Xenora equivalent | Current result | Blocker |
| --- | --- | --- | --- | --- |
| Xbox2Windows compute services | Repeat dispatch, update and release a resource with observable results | Original X9 PE repeats XOR with a nonzero view, updates words and destroys and reuses a slot. | PASS on Apple M5 | Device buffers are uploaded per completion. |
| — | Submission bindings and lifetime are independent of later changes | X9 queues a view, changes or destroys its owner and still gets the captured output. A second queued submission is rejected. | PASS on Apple M5 for queued snapshots | Actual GPU work already in flight during release is UNIMPLEMENTED. |
| — | Isolate guest runs and release resources at exit | X9 drops the table and any queued view at guest return. A queued-work exit returns guest status 73 and produces no GPU commands for that submission. | PASS for queued-work exit | Native Metal object residency across guest calls is UNIMPLEMENTED. |
| — | Reject malformed resource ranges | X9 rejects invalid or stale handles, out-of-range views, offset overflow, writeback into read-only memory and invalid resource types. | PASS for guest fault controls | The guest profile exposes one bounded submission at a time. |
| Xbox2Windows pixel attributes | GPU pixels agree with a CPU expectation | X9 renders a constant or perspective-interpolated triangle through both translated stages. The PE checks every returned pixel. A separate host oracle checks exact values and the guest report. Changing the expected red byte of one interior pixel makes the PE return 93 after the same GPU readback. | PASS on Apple M5 | Shader selection uses original fixed fixture IDs. |
| bcpack-rs round trips and Xbox2Windows texture LOD | Edge data and malformed input distinguish valid subresources | X9 validates a pitched descriptor, rejects wrong usage, layer, size and update requests, and reads selected BC1 mips and layers back through Metal. | PASS on Apple M5 for selected subresource readback | Translated texture sampling and GPU texture lifetime across guest calls are UNIMPLEMENTED. |
