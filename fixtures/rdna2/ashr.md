# gfx1030 Wave32 arithmetic right shift

The arithmetic shift was selected as a candidate. Xenora uses the public
[AMD RDNA 2 ISA](https://docs.amd.com/v/u/en-US/rdna2-shader-instruction-set-architecture)
and its existing pinned AMD XML generator for the VOP2 encoding. The lowering
uses signed integer shift from the
[Metal Shading Language Specification](https://developer.apple.com/metal/Metal-Shading-Language-Specification).
The implementation and fixture were authored in Xenora. No code, tables, dependencies
or game material from other emulator projects were imported.

The decoder accepts the VOP2 VGPR/VGPR form. The shader lifter retains the synthetic
buffer ABI and emits a signed shift with an explicit five-bit count mask. The
production CPU reference extracts the operands directly from bytes. Neither
register reads nor writes add access to the reserved index register.

## Reproduce the encoding

From the repository root with native ARM64 LLVM 23.1.6:

```sh
llvm_bin="$(brew --prefix llvm)/bin"
evidence=artifacts/phase02-ashr
mkdir -p "$evidence"
"$llvm_bin/llvm-mc" -triple=amdgcn-amd-amdhsa -mcpu=gfx1030 \
  -mattr=+wavefrontsize32 -filetype=obj fixtures/rdna2/ashr.s -o "$evidence/ashr.o"
"$llvm_bin/llvm-objcopy" -O binary --only-section=.text "$evidence/ashr.o" "$evidence/rebuilt.bin"
cmp "$evidence/rebuilt.bin" fixtures/rdna2/ashr.bin
"$llvm_bin/llvm-objdump" -d --mcpu=gfx1030 --mattr=+wavefrontsize32 "$evidence/ashr.o"
shasum -a 256 fixtures/rdna2/ashr.s fixtures/rdna2/ashr.bin
```

LLVM emitted word `0x31ffff01` at offset `0x14` and disassembled it as
`v_ashrrev_i32_e32 v255, v1, v255` (class E encoding evidence).
The 40-byte binary has SHA-256
`8194ea7b3a2233823968f78472532df9d3809a5770c1be3ee8b68a9cfeedaf56`.
The source has SHA-256
`b44535bc7c38e17fa8ba1f73cf1b510efc00a21cb914f5ecaef633564346f10b`.

## Review plan

Supply counts 0 through 63 followed by `0x80000000` and `0xffffffff`.
Independently derive each output as the floor of −2147483647 divided by
2 raised to (count modulo 32), reduced modulo 2³². The checkpoint outputs
for counts 0, 1, 31, 32 and 33 are `0x80000001`, `0xc0000000`,
`0xffffffff`, `0x80000001` and `0xc0000000`. For a positive-value
control, replace only the literal word at offset `0x10` with `0x7fffffff`
and use +2147483647 in the derivation. The destination aliases the signed
source to exercise the old register value.

To reproduce the local result, run the CPU test and the native readback in a
bounded process:

```sh
CARGO_BUILD_JOBS=2 cargo test --locked -p xenora-semantic-review arithmetic_right_shift_preserves_sign_and_wraps_counts
MTL_DEBUG_LAYER=1 MTL_SHADER_VALIDATION=1 MTL_SHADER_VALIDATION_REPORT_TO_STDERR=1 \
  MTL_SHADER_VALIDATION_ABORT_ON_FAULT=1 CARGO_BUILD_JOBS=2 timeout 30s \
  cargo test --locked -p xenora-semantic-review arithmetic_right_shift_native_metal_known_answers \
  -- --ignored --nocapture --test-threads=1
```

The repository overhaul checkout based on `6ccb23d` passed the CPU
known-answer test and the opt-in native readback test on Apple M5 with
Metal API and shader validation enabled (class A). Local logs are in
`artifacts/repo-overhaul/`. Independent acceptance remains open in
[issue #19](https://github.com/ranveersoni98/Xenora/issues/19).

## Local acceptance update (2026-11-23)

On the current checkout, the ASHR fixture passed:

- the semantic-review CPU suite (21 passed and 3 ignored opt-in GPU tests);
- the fixture reassembly check;
- the gfx1030 three-way LLVM comparison (332 comparisons across 19 forms); and
- the opt-in `arithmetic_right_shift_native_metal_known_answers` test on Apple M5
  with Metal API and GPU validation enabled.

The CPU check now compares explicit checkpoint words for counts 0, 1, 2, 3, 4,
7, 8, 15, 16, 23, 24, 30, 31, 32, 33, 63, `0x80000000` and `0xffffffff`, a
positive-value control that replaces only the literal at offset `0x10`, the
aliased-destination fixture and a renamed-destination variant. The
semantic-review independent interpreter decodes the VOP2 form and derives the
arithmetic shift with floor division rather than the production signed-shift
operator. Malformed, inline-source and truncated streams are rejected by the
compiler and by both interpreters.

These are local results. Independent semantic and claims review remains open
in [issue #19](https://github.com/ranveersoni98/Xenora/issues/19), and no
commercial game execution is implied.
