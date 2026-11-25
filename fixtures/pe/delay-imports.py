# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate the original PE32+ delay-load import fixture without an x86 toolchain.

The image has one ordinary import DLL and two delay-loaded DLLs, each with a
named import and one of them with an import by ordinal. It follows Microsoft's
PE format specification ("The .idata Section" and "Delay-Load Import Tables
(Image Only)"). It is for static inspection: the entry point returns at once and
nothing calls an import.
"""

import argparse
from pathlib import Path
from struct import pack_into

BASE = 0x140000000
TEXT, RDATA, DATA = 0x1000, 0x2000, 0x3000
# File offset of each section's data, after the 0x400-byte headers.
RAW = {TEXT: 0x400, RDATA: 0x600, DATA: 0xA00}
ORDINAL = 1 << 63

# Ordinary imports, in .rdata.
IMPORT_DIRECTORY = RDATA
LOOKUP, IAT = RDATA + 0x30, RDATA + 0x50
ORDINARY_DLL, ORDINARY_NAME = RDATA + 0x70, RDATA + 0x90

# Delay-load names and name tables, in .rdata before their directory so that a
# file cut inside the directory still holds everything the first entry names.
NAMES_A, NAMES_B = RDATA + 0x100, RDATA + 0x118
DELAY_DLL_A, DELAY_DLL_B = RDATA + 0x130, RDATA + 0x150
DELAY_NAME_A, DELAY_NAME_B = RDATA + 0x170, RDATA + 0x190
DELAY_DIRECTORY = RDATA + 0x200
RELOC_DIRECTORY = RDATA + 0x300

# Delay-load address tables and module handles, in writable .data.
DELAY_IAT_A, DELAY_IAT_B = DATA, DATA + 0x20
HANDLE_A, HANDLE_B = DATA + 0x40, DATA + 0x48

# Each delay-load address slot starts out pointing at its own stub.
STUBS = (TEXT + 0x10, TEXT + 0x18, TEXT + 0x20)


def fixture():
    data = bytearray(0xC00)

    def offset(rva):
        section = rva & ~0xFFF
        return RAW[section] + rva - section

    def put(rva, layout, *values):
        pack_into(layout, data, offset(rva), *values)

    def text(rva, value):
        data[offset(rva):offset(rva) + len(value) + 1] = value + b"\0"

    data[:2] = b"MZ"
    pack_into("<I", data, 0x3C, 0x80)
    data[0x80:0x84] = b"PE\0\0"
    # AMD64, three sections, 240-byte optional header; executable and
    # large-address-aware.
    pack_into("<HHIIIHH", data, 0x84, 0x8664, 3, 0, 0, 0, 240, 0x22)
    opt = 0x98
    pack_into("<H", data, opt, 0x20B)
    pack_into("<III", data, opt + 4, 0x200, 0x600, 0)
    pack_into("<IIQII", data, opt + 16, TEXT, TEXT, BASE, 0x1000, 0x200)
    pack_into("<HHHHHH", data, opt + 40, 6, 0, 0, 0, 6, 0)
    pack_into("<II", data, opt + 56, 0x4000, 0x400)
    # Console subsystem; NX compatible.
    pack_into("<HH", data, opt + 68, 3, 0x100)
    pack_into("<QQQQII", data, opt + 72, 0x10000, 0x1000, 0x10000, 0x1000, 0, 16)
    # Import table, base relocations, import address table and delay imports.
    for index, rva, size in (
        (1, IMPORT_DIRECTORY, 40),
        (5, RELOC_DIRECTORY, 16),
        (12, IAT, 24),
        (13, DELAY_DIRECTORY, 96),
    ):
        pack_into("<II", data, opt + 112 + index * 8, rva, size)

    sections = (
        (b".text", TEXT, 0x200, 0x60000020),
        (b".rdata", RDATA, 0x400, 0x40000040),
        (b".data", DATA, 0x200, 0xC0000040),
    )
    for index, (name, rva, size, characteristics) in enumerate(sections):
        pack_into(
            "<8sIIIIIIHHI", data, opt + 240 + 40 * index, name, size, rva,
            size, offset(rva), 0, 0, 0, 0, characteristics,
        )

    # Entry returns. The stubs are INT3 because the fixture is never run.
    data[offset(TEXT)] = 0xC3
    for stub in STUBS:
        data[offset(stub)] = 0xCC

    # One ordinary DLL: a named import and ordinal 3.
    put(IMPORT_DIRECTORY, "<IIIII", LOOKUP, 0, 0, ORDINARY_DLL, IAT)
    for table in (LOOKUP, IAT):
        put(table, "<QQQ", ORDINARY_NAME, ORDINAL | 3, 0)
    text(ORDINARY_DLL, b"fixture-ordinary.dll")
    text(ORDINARY_NAME + 2, b"OrdinaryByName")

    # Two delay-loaded DLLs. Attributes bit 0 marks the fields as RVAs.
    put(DELAY_DIRECTORY, "<IIIIIIII",
        1, DELAY_DLL_A, HANDLE_A, DELAY_IAT_A, NAMES_A, 0, 0, 0)
    put(DELAY_DIRECTORY + 32, "<IIIIIIII",
        1, DELAY_DLL_B, HANDLE_B, DELAY_IAT_B, NAMES_B, 0, 0, 0)
    put(NAMES_A, "<QQQ", DELAY_NAME_A, ORDINAL | 7, 0)
    put(NAMES_B, "<QQ", DELAY_NAME_B, 0)
    text(DELAY_DLL_A, b"fixture-delay-a.dll")
    text(DELAY_DLL_B, b"fixture-delay-b.dll")
    text(DELAY_NAME_A + 2, b"DelayedAlpha")
    text(DELAY_NAME_B + 2, b"DelayedBeta")
    put(DELAY_IAT_A, "<QQQ", BASE + STUBS[0], BASE + STUBS[1], 0)
    put(DELAY_IAT_B, "<QQ", BASE + STUBS[2], 0)

    # One page block. The three DIR64 entries relocate the absolute IAT
    # pointers, and the ABSOLUTE entry pads the block to a 4-byte boundary.
    # The descriptors and name tables already use RVAs.
    put(RELOC_DIRECTORY, "<II4H", DATA, 16, 0xA000, 0xA008, 0xA020, 0)
    return data


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    path = Path(__file__).with_suffix(".exe")
    if args.check:
        if path.read_bytes() != fixture():
            parser.exit(1, f"fixture bytes differ from generator: {path}\n")
    else:
        path.write_bytes(fixture())
