# SPDX-License-Identifier: GPL-3.0-or-later
"""Verify how the LLVM oracle handles missing and explicitly invalid records."""

from importlib.util import module_from_spec, spec_from_file_location
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = spec_from_file_location("corpus", ROOT / "crates/isa/tests/llvm/corpus.py")
assert SPEC is not None and SPEC.loader is not None
corpus = module_from_spec(SPEC)
SPEC.loader.exec_module(corpus)


class CorpusTests(unittest.TestCase):
    def run_with_dump(self, output, blobs):
        with tempfile.TemporaryDirectory() as directory:
            tools = Path(directory)
            for name, script in (
                ("llvm-mc", "exit 0\n"),
                ("llvm-objdump", "cat <<'DUMP'\n" + output + "\nDUMP\n"),
            ):
                tool = tools / name
                tool.write_text("#!/bin/sh\n" + script, encoding="utf-8")
                tool.chmod(0o755)
            return corpus.disassemble_batch(tools, blobs)

    def test_successful_objdump_with_missing_first_record_is_an_error(self):
        bad_dump = (
            "Disassembly of section .text.0:\n"
            "\t.long 0xffffffff // 000000000000: FFFFFFFF\n"
            "Disassembly of section .text.1:\n"
            "\tv_illegal // 000000000004: 00000000"
        )

        with self.assertRaisesRegex(ValueError, r"offset-zero.*\.text\.1"):
            self.run_with_dump(bad_dump, [b"\xff" * 4, bytes(8)])

    def test_explicit_invalid_record_and_empty_blob_remain_undecoded(self):
        dump = (
            "Disassembly of section .text.0:\n"
            "\t.long 0xffffffff // 000000000000: FFFFFFFF\n"
            "Disassembly of section .text.2:\n"
            "\tv_illegal // 000000000000: 00000000"
        )

        self.assertEqual(
            self.run_with_dump(dump, [b"\xff" * 4, b"", bytes(4)]),
            [None, None, ("v_illegal", 4)],
        )

    def test_batch_crash_isolates_only_the_crashing_blob(self):
        valid = b"valid"
        crash = b"crash"

        def disassemble(_, blobs):
            if crash in blobs:
                raise subprocess.CalledProcessError(-11, "llvm-objdump")
            return [("v_illegal", 4) if blob else None for blob in blobs]

        with patch.object(corpus, "disassemble_batch", side_effect=disassemble):
            self.assertEqual(
                corpus.disassemble_all(Path("unused"), [valid, b"", crash, valid]),
                [("v_illegal", 4), None, corpus.CRASHED, ("v_illegal", 4)],
            )

    @unittest.skipUnless(os.environ.get("LLVM_BIN"), "set LLVM_BIN to test native LLVM")
    def test_native_zero_padding_does_not_hide_the_first_instruction(self):
        blobs = [bytes(size) for size in (4, 8, 12, 16, 20, 24)]
        blobs += [b"\xff" * 4, b""]

        actual = corpus.disassemble_batch(Path(os.environ["LLVM_BIN"]), blobs)

        self.assertEqual(actual, [("v_illegal", 4)] * 6 + [None, None])
