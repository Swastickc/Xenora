# Scalar buffer loads

`s_buffer_load_dword`, `s_buffer_load_dwordx2` and `s_buffer_load_dwordx4` copy one, two or four consecutive words of a buffer into scalar registers (SGPRs). They are scalar memory (SMEM) instructions: every lane of a wave sees the same value, and EXEC does not affect them. Rules below come from the AMD "RDNA 2" Instruction Set Architecture Reference Guide (document 70648), sections 7.1 to 7.5 and 13.2.1.

## Encoding subset

| Field | Accepted |
| --- | --- |
| SBASE | An aligned quad of normal SGPRs. A fixture may use only `s[0:3]`. |
| SDATA | Normal SGPRs `s8`-`s105` in a fixture; even for two words and a multiple of four for four words |
| OFFSET | Bits 0-19. Bit 20 is the sign, which must be clear for a buffer load. |
| SOFFSET | `null` or a normal SGPR with a value known during translation |
| GLC, DLC, reserved bits | Zero |

Other scalar loads, other widths, `vcc` destinations and `m0` offsets are refused.

## Address and bounds

The byte address is `(OFFSET + SOFFSET) & ~3`, computed without wrapping at 32 bits. The guide ignores the two low bits of the address, so an unaligned offset is legal. Following words are four bytes apart.

The fixture has no hardware descriptor. Under the [synthetic memory ABI](../adr/0003-demo-memory-model.md), `s[0:3]` is the dispatch input: base 0, one word per invocation. A word is in range if the dispatch has it. The guide says that a load is not performed for a word that is out of range, so **that SGPR keeps its value**. For that value to be defined, every destination SGPR must be initialized. Whether hardware also leaves the register alone has not been observed.

The emitted MSL tests the word index against the dispatch size before it reads the buffer, so a guest offset never reaches memory outside it.

## Waits

Scalar loads can return out of order, so only `s_waitcnt lgkmcnt(0)` completes them. It completes every outstanding scalar load and no vector load; `s_waitcnt vmcnt(0)` completes no scalar load. Before the wait, reading or writing a destination is refused. So is ending the program or taking a run-time `s_cbranch_vccz`, as for an outstanding vector load.

## Uses of a loaded SGPR

A loaded value is uniform across the wave and known only at run time. The compiler accepts it as the scalar source of `v_mov_b32` and of the VOP1, VOP2 and VOPC forms whose operand rule allows an SGPR. SOPC and SOPK compares accept it too, for [run-time branches](runtime-branches.md). Other scalar instructions that read it, including EXEC writes and SGPR offsets, are refused, because scalar arithmetic is still evaluated during translation. The CPU reference computes those.

## Evidence

`tests/validation/tests/smem_buffer.rs` states the expected value of each case from the guide, without the decoder or the compiler, and runs them on the CPU reference and on native Metal. The LLVM three-way comparison covers the encodings. Nothing was compared with Xbox or AMD hardware.
