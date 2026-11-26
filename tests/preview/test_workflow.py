# SPDX-License-Identifier: GPL-3.0-or-later
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from preview import __main__ as preview
from preview.source import Source

SOURCE = Source("a" * 40, "0.1.0", "9")


class WorkflowTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.destination = self.root / "preview"

    def test_incomplete_signing_configuration_never_starts_the_build(self):
        with self.assertRaisesRegex(ValueError, "provide both"):
            preview.build(self.root, self.destination, "9", identity="Developer ID Application: A")
        self.assertFalse(self.destination.exists())

    def test_build_failure_leaves_no_preview(self):
        with patch.object(preview, "snapshot", side_effect=RuntimeError("Git failed")):
            with self.assertRaisesRegex(RuntimeError, "Git failed"):
                preview.build(self.root, self.destination, "9")
        self.assertEqual(list(self.root.iterdir()), [])

    def test_build_cannot_replace_an_existing_output(self):
        self.destination.mkdir()
        existing = self.destination / "artifact"
        existing.write_bytes(b"preserve this")
        with self.assertRaises(FileExistsError):
            preview.build(self.root, self.destination, "9")
        self.assertEqual(existing.read_bytes(), b"preserve this")

    def test_architecture_evidence_must_match_current_inspection(self):
        provided = {
            "info_plist": SOURCE.bundle_fields(),
            "executables": {"Xenora": {"architectures": ["x86_64"]}},
        }
        observed = {
            "info_plist": SOURCE.bundle_fields(),
            "executables": {"Xenora": {"architectures": ["arm64"]}},
        }
        with (
            patch.object(preview, "read_verified_manifest", return_value=(SOURCE, "ad-hoc", provided)),
            patch.object(preview, "check_archive", return_value=(None, observed)),
            self.assertRaisesRegex(ValueError, "inspection differs"),
        ):
            preview.verify(self.destination)

    def test_public_verification_rejects_local_mode(self):
        with (
            patch.object(preview, "read_verified_manifest", return_value=(SOURCE, "ad-hoc", {})),
            patch.object(preview, "check_archive") as check_archive,
            self.assertRaisesRegex(ValueError, "does not identify a Developer ID build"),
        ):
            preview.verify(self.destination, require_developer_id=True)
        check_archive.assert_not_called()
