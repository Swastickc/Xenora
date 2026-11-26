# SPDX-License-Identifier: GPL-3.0-or-later
"""Check RDNA2 fixture publication with isolated LLVM command stubs."""

import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/assemble-fixture.sh"


class AssembleFixtureTests(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        root = Path(scratch.name)
        self.script = root / "scripts" / SCRIPT.name
        self.script.parent.mkdir()
        self.script.write_bytes(SCRIPT.read_bytes())
        self.fixtures = root / "fixtures/rdna2"
        self.fixtures.mkdir(parents=True)
        self.expected = {}
        for source in (ROOT / "fixtures/rdna2").glob("*.s"):
            name = source.stem
            payload = f"rebuilt {name}\n".encode()
            (self.fixtures / source.name).write_bytes(payload)
            (self.fixtures / f"{name}.bin").write_bytes(f"original {name}\n".encode())
            self.expected[f"{name}.bin"] = hashlib.sha256(payload).hexdigest()

        self.llvm = root / "llvm"
        self.llvm.mkdir()
        self.tool(
            "llvm-mc",
            """
for arg do
    case "$arg" in *.s) source=$arg ;; esac
done
while [ "$1" != -o ]; do shift; done
[ "${source##*/}" != "$ASSEMBLER_FAIL" ] || exit 42
cp "$source" "$2"
""",
        )
        self.tool(
            "llvm-objcopy",
            """
[ "${4##*/}" != "$EXTRACTOR_FAIL" ] || exit 43
cp "$4" "$5"
""",
        )

    def tool(self, name, body):
        tool = self.llvm / name
        tool.write_text("#!/bin/sh\nset -eu\n" + body, encoding="utf-8")
        tool.chmod(0o755)

    def digests(self):
        return {
            path.name: hashlib.sha256(path.read_bytes()).hexdigest()
            for path in self.fixtures.glob("*.bin")
        }

    def assemble(self, *args, assembler_fail="", extractor_fail=""):
        return subprocess.run(
            ["sh", str(self.script), *args],
            env={
                **os.environ,
                "LLVM_BIN": str(self.llvm),
                "ASSEMBLER_FAIL": assembler_fail,
                "EXTRACTOR_FAIL": extractor_fail,
            },
            capture_output=True,
            encoding="utf-8",
            check=False,
        )

    def test_late_assembler_failure_preserves_every_binary(self):
        before = self.digests()

        result = self.assemble(assembler_fail="runtime-branch.s")

        self.assertEqual(result.returncode, 42, result.stderr)
        self.assertEqual(self.digests(), before)

    def test_late_extractor_failure_preserves_every_binary(self):
        before = self.digests()

        result = self.assemble(extractor_fail="runtime-branch.o")

        self.assertEqual(result.returncode, 43, result.stderr)
        self.assertEqual(self.digests(), before)

    def test_regeneration_and_read_only_check(self):
        before = self.digests()
        mismatch = self.assemble("--check")
        self.assertEqual(mismatch.returncode, 1, mismatch.stderr)
        self.assertEqual(self.digests(), before)

        rebuilt = self.assemble()
        self.assertEqual(rebuilt.returncode, 0, rebuilt.stderr)
        self.assertEqual(self.digests(), self.expected)

        checked = self.assemble("--check")
        self.assertEqual(checked.returncode, 0, checked.stderr)
        self.assertEqual(self.digests(), self.expected)
