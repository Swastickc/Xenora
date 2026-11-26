# SPDX-License-Identifier: GPL-3.0-or-later
import plistlib
import subprocess
import tempfile
import unittest
from pathlib import Path

from preview.source import Source, snapshot, stamp_bundle

REVISION = "1" * 40


def run(args, *, capture=False, **kwargs):
    result = subprocess.run(args, check=True, text=True, capture_output=capture, **kwargs)
    return result.stdout or ""


class SourceTests(unittest.TestCase):
    def test_rejects_malformed_metadata(self):
        for fields in (
            ("HEAD", "0.1.0", "1"),
            (REVISION, "0.1.0-beta.2", "1"),
            (REVISION, "1.2", "1"),
            (REVISION, "0.1.0", "0"),
            (REVISION, "0.1.0", "10000"),
            (REVISION, "0.1.0", "1.001"),
            (REVISION, "0.1.0", "1.2.3.4"),
            (REVISION, "0.1.0", "１"),
        ):
            with self.subTest(fields=fields), self.assertRaises(ValueError):
                Source(*fields)

    def test_stamp_preserves_the_existing_bundle_keys(self):
        with tempfile.TemporaryDirectory() as temp:
            app = Path(temp)
            info = app / "Contents" / "Info.plist"
            info.parent.mkdir()
            info.write_bytes(plistlib.dumps({"CFBundleExecutable": "Xenora"}))
            stamp_bundle(app, Source(REVISION, "1.2.3", "1.7"))
            self.assertEqual(
                plistlib.loads(info.read_bytes()),
                {
                    "CFBundleExecutable": "Xenora",
                    "CFBundleShortVersionString": "1.2.3",
                    "CFBundleVersion": "1.7",
                    "XenoraSourceRevision": REVISION,
                },
            )

    def test_snapshot_uses_only_the_pinned_commit(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            checkout = root / "checkout"
            checkout.mkdir()
            run(["git", "init", "-q"], cwd=checkout)
            cargo = checkout / "Cargo.toml"
            cargo.write_text('[workspace.package]\nversion = "2.3.4"\n')
            run(["git", "add", "Cargo.toml"], cwd=checkout)
            run(
                ["git", "-c", "user.name=Test", "-c", "user.email=test@example.org",
                 "-c", "commit.gpgsign=false", "commit", "-qm", "test fixture"],
                cwd=checkout,
            )
            revision = run(["git", "rev-parse", "HEAD"], cwd=checkout, capture=True).strip()
            (checkout / "ignored-by-snapshot").write_text("local data")
            pinned = root / "snapshot"
            self.assertEqual(snapshot(checkout, pinned, "2", run=run), Source(revision, "2.3.4", "2"))
            self.assertEqual(list(pinned.iterdir()), [pinned / "Cargo.toml"])

            cargo.write_text('[workspace.package]\nversion = "9.9.9"\n')
            with self.assertRaisesRegex(ValueError, "commit tracked changes"):
                snapshot(checkout, root / "dirty", "2", run=run)
            self.assertFalse((root / "dirty").exists())
