# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise the claims audit against a disposable Git worktree."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


class ClaimsAuditTests(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.root = Path(scratch.name)
        self.git("init", "-q")
        self.write(
            "scripts/audit/claims.sh",
            (ROOT / "scripts/audit/claims.sh").read_text(encoding="utf-8"),
        )
        self.write("README.md", "# Fixture\n")
        self.write("AGENTS.md", "# Fixture instructions\n")
        self.write("docs/claims.toml", "claim = []\n")
        self.write("docs/status.toml", "feature = []\n")
        self.write(".gitignore", ".local/\n")
        self.python_bin = self.root / "python-bin"
        self.python_bin.mkdir()
        (self.python_bin / "python3").symlink_to(sys.executable)

    def git(self, *args):
        subprocess.run(["git", *args], cwd=self.root, check=True, capture_output=True)

    def write(self, name, content):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

    def audit(self):
        return subprocess.run(
            ["sh", "scripts/audit/claims.sh"],
            cwd=self.root,
            env={
                **os.environ,
                "PATH": f"{self.python_bin}{os.pathsep}{os.environ['PATH']}",
                "LC_ALL": "C",
                "PYTHONCOERCECLOCALE": "0",
                "PYTHONUTF8": "0",
            },
            capture_output=True,
            encoding="utf-8",
            check=False,
        )

    def test_ignored_untracked_files_and_local_deletions_are_skipped(self):
        self.write("docs/guide/.local/notes.md", "CLAIM-IGNORED-MARKDOWN\n")
        self.write("crates/example/.local/generated.rs", "CLAIM-IGNORED-RUST\n")
        self.write("docs/deleted.md", "CLAIM-DELETED\n")
        self.git("add", "docs/deleted.md")
        (self.root / "docs/deleted.md").unlink()

        result = self.audit()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Checked 0 claims", result.stdout)

    def test_tracked_and_unignored_sources_use_current_contents(self):
        tracked = "docs/guide/.local/tracked.md"
        staged = "crates/example/.local/staged.rs"
        self.write(tracked, "CLAIM-INDEX-VERSION\n")
        self.git("add", "-f", tracked)
        self.git(
            "-c", "user.name=Audit Fixture", "-c", "user.email=fixture@example.test",
            "-c", "commit.gpgsign=false", "commit", "-qm", "Track an ignored fixture",
        )
        self.write(tracked, "CLAIM-WORKTREE-VERSION\n")
        self.write(staged, "CLAIM-STAGED\n")
        self.git("add", "-f", staged)

        files = {
            "README.md": "CLAIM-README",
            "AGENTS.md": "CLAIM-AGENTS",
            "docs/guide/caf\u00e9\nextra.toml": "CLAIM-UNICODE",
            "crates/example/new.rs": "CLAIM-NEW-RUST",
        }
        for path, claim in files.items():
            self.write(path, f"{claim}\n")
        self.write("docs/not-source.txt", "CLAIM-EXCLUDED-DOC\n")
        self.write("crates/example/notes.md", "CLAIM-EXCLUDED-CRATE\n")
        self.write("examples/outside.rs", "CLAIM-EXCLUDED-ROOT\n")

        result = self.audit()

        self.assertEqual(result.returncode, 1)
        for claim in (*files.values(), "CLAIM-WORKTREE-VERSION", "CLAIM-STAGED"):
            with self.subTest(claim=claim):
                self.assertIn(f"unregistered {claim}", result.stderr)
        self.assertNotIn("CLAIM-INDEX-VERSION", result.stderr)
        self.assertNotIn("CLAIM-EXCLUDED-", result.stderr)
