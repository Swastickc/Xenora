#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Regenerate the gfx1030 Wave32 instruction-identification oracle corpus."""

import argparse
from collections import defaultdict
from itertools import islice, zip_longest
from pathlib import Path
import random
import re
import runpy
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
ISA = HERE.parents[1]
TARGET = ("-triple=amdgcn-amd-amdhsa", "-mcpu=gfx1030", "-mattr=+wavefrontsize32")
BASE_32 = {"SOP1", "SOP2", "SOPC", "SOPK", "SOPP", "VINTRP", "VOP1", "VOP2", "VOPC"}
ENCODING = re.compile(r"^\s*(.*?)\s*; encoding: \[([^\]]+)\]", re.MULTILINE)
SECTION = re.compile(r"^Disassembly of section \.text\.(\d+):")
# Instruction text, offset, consumed words, then optional trailing text: a
# branch-target annotation such as `<.text+0x841c>` or an LLVM warning such as
# `; Warning: ... scalar reg isn't aligned`, neither of which stops decoding.
INSTRUCTION = re.compile(r"^\t(.*?)\s*// ([0-9A-F]{12}): ((?:[0-9A-F]{8}(?: |$))+)(.*)$")


def llvm(executable, *args, source=None):
    return subprocess.run(
        [executable, *TARGET, "-show-encoding", *args],
        input=source, text=True, capture_output=True, check=True,
    )


def encodings(output):
    for text, bytes_text in ENCODING.findall(output):
        yield " ".join(text.split()), bytes(int(token, 16) for token in bytes_text.split(","))


def disassemble_batch(llvm_bin, blobs):
    """LLVM's decode of the first instruction of each blob.

    Each blob goes into its own section of one object built from `.byte`
    directives; llvm-objdump disassembles every section from offset zero and
    prints the words it consumed. That length is what the decoder read, not the
    length of a re-encoding (llvm-mc --disassemble re-encodes, which shortens
    inconsistent MIMG NSA probes). Returns (text, consumed bytes) or None when
    the blob is empty or LLVM explicitly cannot decode offset zero (it prints
    `.long`/`.short`/`.byte`). A missing record for a nonempty blob is an error.
    """
    source = []
    for index, blob in enumerate(blobs):
        source.append(f'.section .text.{index},"ax"')
        if blob:
            source.append(".byte " + ",".join(f"0x{byte:02x}" for byte in blob))
    with tempfile.TemporaryDirectory() as directory:
        assembly_path = Path(directory) / "probes.s"
        object_path = Path(directory) / "probes.o"
        assembly_path.write_text("\n".join(source) + "\n")
        subprocess.run(
            [llvm_bin / "llvm-mc", *TARGET, "-filetype=obj", assembly_path, "-o", object_path],
            check=True, capture_output=True,
        )
        dump = subprocess.run(
            [
                llvm_bin / "llvm-objdump", "-d", "--disassemble-zeroes",
                "--mcpu=gfx1030", "--mattr=+wavefrontsize32", object_path,
            ],
            check=True, capture_output=True, text=True,
        ).stdout
    first = {}
    section = None
    for line in dump.splitlines():
        if line.startswith("Disassembly of section "):
            header = SECTION.fullmatch(line)
            section = int(header.group(1)) if header else None
            continue
        instruction = INSTRUCTION.match(line)
        if section is None or section in first or not instruction:
            continue
        text, offset, words, warning = instruction.groups()
        if int(offset, 16) != 0:
            continue
        if text.startswith((".long", ".short", ".byte")):
            first[section] = None
        else:
            text = " ".join(text.split())
            warning = warning.strip()
            if warning.startswith(";"):
                text += f" [LLVM: {warning.lstrip('; ')}]"
            first[section] = (text, 4 * len(words.split()))
    for index, blob in enumerate(blobs):
        if blob and index not in first:
            raise ValueError(f"llvm-objdump omitted an offset-zero record for .text.{index}")
    return [first.get(index) for index in range(len(blobs))]


CRASHED = "crashed"


def disassemble_all(llvm_bin, blobs):
    """Batches of 128; a batch that crashes llvm-objdump is split until the
    crashing blobs are isolated, and each of those is reported as CRASHED."""
    results = []
    for start in range(0, len(blobs), 128):
        results.extend(_isolate(llvm_bin, blobs[start:start + 128]))
    return results


def _isolate(llvm_bin, blobs):
    try:
        return disassemble_batch(llvm_bin, blobs)
    except subprocess.CalledProcessError as error:
        if error.returncode != -11:
            raise
    if len(blobs) == 1:
        return [CRASHED]
    half = len(blobs) // 2
    return _isolate(llvm_bin, blobs[:half]) + _isolate(llvm_bin, blobs[half:])


def extension(text, length, family):
    mnemonic = text.split()[0]
    if "dpp8:" in text:
        return "dpp8fi" if "fi:1" in text else "dpp8"
    if mnemonic.endswith("_dpp"):
        return "dpp16"
    if mnemonic.endswith("_sdwa"):
        return "sdwa"
    base_bytes = 4 if family in BASE_32 else 8
    if mnemonic.endswith("_e64"):
        base_bytes = 8
    extra = length - base_bytes
    if mnemonic.startswith("image_") and extra > 0:
        return f"nsa{extra // 4}"
    # After excluding vector control and image addresses, an extra dword
    # carrying a numeric operand is the literal (including mandatory literals).
    if extra == 4 and re.search(r"(?:,|\s)\s*-?(?:0x[\da-f]+|\d+)(?=,|\s|$|/\*)", text):
        return "literal"
    if extra > 0:
        raise ValueError(f"unclassified LLVM extension: {length} bytes {text}")
    return "-"


def assembly(executable):
    source = HERE / "corpus.s"
    families = [
        line.split("//", 1)[1].strip()
        for line in source.read_text().splitlines()
        if line.split("//", 1)[0].strip()
    ]
    result = llvm(executable, source)
    if result.stderr:
        raise ValueError(result.stderr)
    decoded = list(encodings(result.stdout))
    if len(decoded) != len(families):
        raise ValueError("LLVM must emit exactly one encoding per assembly line")
    return [(family, text, data) for family, (text, data) in zip(families, decoded)]


def metadata_forms():
    snapshot = ISA / "data/rdna2.jsonl"
    project = runpy.run_path(str(ISA / "generate_rdna2_meta.py"))["forms_from_snapshot"]
    return project(snapshot.read_text())[2]


def probes():
    # The normalized snapshot stores instruction variants and encoding records
    # separately. Reuse its projection to obtain identifiers, widths and labels
    # for coverage. The projected mnemonic and extensions are never consulted.
    forms = metadata_forms()
    rng = random.Random(209)
    for form in forms:
        width = form["bytes"]
        # One probe with every non-identifying bit zero (operands s0/v0, no
        # modifiers), then two with seeded random operand bits.
        yield "zero", form["encoding"].upper(), form["bits"].to_bytes(width, "little") + bytes(16)
        for _ in range(2):
            value = form["bits"] | (rng.getrandbits(width * 8) & ~form["mask"])
            yield "random", form["encoding"].upper(), value.to_bytes(width, "little") + rng.randbytes(16)


# Extension selectors from the RDNA 2 ISA reference: the SRC0 (VOP1, VOP2,
# VOPC, VOP3*) or SSRC0 (SOP1, SOP2, SOPC) value that makes one extra dword
# follow the base encoding.
SELECTORS = {"literal": 0xFF, "sdwa": 0xF9, "dpp16": 0xFA, "dpp8": 0xE9, "dpp8fi": 0xEA}
# Instructions whose last dword is a legal control word for each extension;
# LLVM assembles them, so the control words are LLVM's own.
CONTROL_SOURCES = {
    "sdwa": "v_add_f32_sdwa v0, v0, v0 dst_sel:DWORD dst_unused:UNUSED_PAD src0_sel:DWORD src1_sel:DWORD",
    "sdwa-vop1": "v_mov_b32_sdwa v0, v0 dst_sel:DWORD dst_unused:UNUSED_PAD src0_sel:DWORD",
    "sdwa-vopc": "v_cmp_eq_f32_sdwa vcc_lo, v0, v0 src0_sel:DWORD src1_sel:DWORD",
    "dpp16": "v_add_f32_dpp v0, v0, v0 quad_perm:[0,1,2,3] row_mask:0xf bank_mask:0xf",
    "dpp8": "v_add_f32_dpp v0, v0, v0 dpp8:[0,1,2,3,4,5,6,7]",
    "dpp8fi": "v_add_f32_dpp v0, v0, v0 dpp8:[0,1,2,3,4,5,6,7] fi:1",
}
LITERAL = (0x12345678).to_bytes(4, "little")


def controls(executable):
    source = "\n".join(CONTROL_SOURCES.values()) + "\n"
    decoded = list(encodings(llvm(executable, source=source).stdout))
    return {name: data[-4:] for name, (_, data) in zip(CONTROL_SOURCES, decoded)}


def extension_probes(executable, forms):
    """Every form's zero-operand base with each extension selector its family
    can carry, followed by a legal control dword or literal. LLVM decides which
    combinations are instructions; the metadata's extension list is not read."""
    control = controls(executable)
    for form in forms:
        family = form["encoding"].upper()
        base = form["bits"]
        if family in ("VOP1", "VOP2", "VOPC"):
            kinds = ["literal", "sdwa", "dpp16", "dpp8", "dpp8fi"]
            shift = 0
        elif family in ("VOP3", "VOP3SDST", "VOP3P"):
            kinds, shift = ["literal"], 32
        elif family in ("SOP1", "SOP2", "SOPC"):
            kinds, shift = ["literal"], 0
        else:
            continue
        field = 0x1FF if shift == 32 or family.startswith("VOP") else 0xFF
        if base & form["mask"] & (field << shift):
            continue  # SRC0/SSRC0 is part of this form's identifier
        for kind in kinds:
            value = (base & ~(field << shift)) | (SELECTORS[kind] << shift)
            if kind == "literal":
                trailer = LITERAL
            elif kind == "sdwa" and family in ("VOPC", "VOP1"):
                trailer = control[f"sdwa-{family.lower()}"]
            else:
                trailer = control[kind]
            yield kind, family, value.to_bytes(form["bytes"], "little") + trailer + bytes(12)


def truncations(entries):
    by_family = defaultdict(list)
    for entry in entries:
        by_family[entry[0]].append(entry)
    groups = [
        sorted(group, key=lambda entry: -len(entry[2]))
        for _, group in sorted(by_family.items())
    ]
    interleaved = (entry for batch in zip_longest(*groups) for entry in batch if entry)
    yield from islice(interleaved, 40)


def decoded_columns(text, length, family):
    return text.split()[0], str(length), extension(text, length, family)


def generate(llvm_bin):
    rows = []
    entries = assembly(llvm_bin / "llvm-mc")
    for family, text, data in entries:
        rows.append(("asm", data.hex(), *decoded_columns(text, len(data), family), f"{family} {text}"))
    probe_list = list(probes()) + [
        (f"ext-{kind}", family, data)
        for kind, family, data in extension_probes(llvm_bin / "llvm-mc", metadata_forms())
    ]
    cut_list = [
        (family, text, data, data[:size])
        for family, text, data in truncations(entries)
        for size in range(0, len(data), 4)
    ]
    decoded_all = disassemble_all(llvm_bin, [data for _, _, data in probe_list] + [cut for *_, cut in cut_list])
    for (kind, family, data), decoded in zip(probe_list, decoded_all):
        if decoded is None:
            columns = ("invalid", "0", "-")
            note = "LLVM cannot decode offset zero"
        elif decoded == CRASHED:
            columns = ("crashed", "0", "-")
            note = "llvm-objdump crashed (SIGSEGV)"
        elif "/*Invalid" not in decoded[0]:
            text, length = decoded
            columns = decoded_columns(text, length, family)
            note = text
        if decoded not in (None, CRASHED) and "/*Invalid" in decoded[0]:
            # LLVM decodes the words (so it knows the opcode and the length)
            # but flags an operand the instruction cannot take, for example an
            # SGPR or literal for a VGPR-only source.
            text, length = decoded
            mnemonic, *rest = decoded_columns(text, length, family)
            columns = (f"invalid-operand:{mnemonic}", *rest)
            note = text
        source = f"probe-{kind}" if kind in ("zero", "random") else "probe-ext"
        if kind.startswith("ext-"):
            note = f"{kind[4:]} {note}"
        rows.append((source, data.hex(), *columns, f"{family} {note}"))
    for (family, text, data, prefix), decoded in zip(cut_list, decoded_all[len(probe_list):]):
        if decoded is None:
            observed = "cannot decode" if prefix else "no instruction"
        elif decoded == CRASHED:
            observed = "crashed (SIGSEGV)"
        else:
            observed = f"decoded {decoded[1]} bytes: {decoded[0]}"
        note = f"{family} LLVM {observed} | cut {data.hex()} {text}"
        rows.append(("truncation", prefix.hex(), "truncated", "0", "-", note))
    header = "source\thex\tllvm_mnemonic\tllvm_len\tllvm_ext\tnote\n"
    return header + "".join("\t".join(row) + "\n" for row in sorted(rows))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--llvm-bin", required=True, type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    data = generate(args.llvm_bin.resolve())
    output = HERE / "corpus.tsv"
    if args.check:
        if output.read_bytes() != data.encode():
            parser.exit(1, f"{output} differs from the LLVM output\n")
    else:
        output.write_bytes(data.encode())


if __name__ == "__main__":
    main()
