# Phase 02 collaboration research

The implementation policy is [independent authorship](../../CONTRIBUTING.md).
Historical reuse discussion records the earlier survey. Xenora uses
public specifications and original fixtures, with other emulator
projects serving only as references for test selection.

Reviewed 2026-11-23 against Xenora `4b9a4cecf35c8d4d90d8ecdb9c1510e7bd260102`. Evidence consists of public source inspection and upstream reports. No builds, fixtures, game inputs or hardware tests were run.

## KyTx

TxCsOffc offered help with Wave64/EXEC, descriptors, PM4 and large MSL programs. Xenora already accepted the offer in principle and invited a small contribution. The FC 27 menu and 30–37 FPS match on M5 Max are class D developer reports, with rendering flaws and stability issues. The issue and README come from the same project and provide no independent reproduction. This PS5 report does not establish Xbox behavior. [Offer and maintainer reply](https://github.com/ranveersoni98/Xenora/issues/1), [KyTx README at inspected revision](https://github.com/TxCsOffc/KyTx/blob/10f50d3eff0c0df36ee5d7007848a7b0b15072eb/README.md)

At KyTx `10f50d3eff0c0df36ee5d7007848a7b0b15072eb`, `Wave64Pair`, the emitter's per-half instruction path and its two-word ballot support the architectural description. The selector also has stage and diagnostic controls, so the source does not establish uniform coverage of all stages, opcodes or cross-half cases. Use this as a design lead. KyTx documents a native Metal product path and an external FEX-darwin build dependency. [MSL emitter](https://github.com/TxCsOffc/KyTx/blob/10f50d3eff0c0df36ee5d7007848a7b0b15072eb/src/graphics/shader/recompiler/backend/msl/MslEmitter.cpp#L3152), [Build notes](https://github.com/TxCsOffc/KyTx/blob/10f50d3eff0c0df36ee5d7007848a7b0b15072eb/README.md#building)

### First contribution

Request original gfx1030 Wave64 assembly, a mask/lane design note and per-lane expected outputs. Specify an isolated 64-lane synthetic ABI with full initial EXEC, lane-indexed output and a sentinel for inactive writes. Cover full, empty, alternating and asymmetric masks, lanes 0/31/32/63, and nested save, modify and restore of EXEC. Use observable scalar results to catch accidental duplicate scalar effects. Identify encodings with the pinned LLVM oracle (class E) and have R2 derive expected results independently from AMD's public ISA (class B for documented desktop RDNA 2, with Xbox applicability unknown). Require explicit rejection outside the supported subset. Keep the first PR fixture-only. Any later native lowering needs its own exact readback and validation evidence, including the measured pipeline execution width. [AMD RDNA 2 ISA](https://www.amd.com/content/dam/amd/en/documents/radeon-tech-docs/instruction-set-architectures/rdna2-shader-instruction-set-architecture.pdf), [Xenora interface](../interfaces.md)

KyTx declares GPL-2.0. Tx's conditional GPL-2.0-or-later offer covers only material they own and can license. Require file-level authorship, explicit compatible terms (GPL-3.0-or-later preferred for new Xenora fixtures), preserved notices and DCO. Inherited GPL-2.0-only code cannot enter the GPL-3.0-or-later combined work under those terms. The separate Kyty MIT notice does not settle a derived file's full provenance. No KyTx submodule is justified for the fixture. [KyTx license and notices](https://github.com/TxCsOffc/KyTx/tree/10f50d3eff0c0df36ee5d7007848a7b0b15072eb/LICENSES), [KyTx license](https://github.com/TxCsOffc/KyTx/blob/10f50d3eff0c0df36ee5d7007848a7b0b15072eb/LICENSE)
