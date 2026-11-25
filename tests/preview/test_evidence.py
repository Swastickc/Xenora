# SPDX-License-Identifier: GPL-3.0-or-later
import json
import tempfile
import unittest
from pathlib import Path

from preview.evidence import APP_ARCHIVE, ARTIFACTS, MANIFEST, read_verified_manifest, write_manifest
from preview.source import Source

SOURCE = Source("4" * 40, "0.1.0", "17")
APP_EVIDENCE = {"info_plist": SOURCE.bundle_fields()}
BUILD_EVIDENCE = {"locked_app_build_and_smoke": True, "environment": {}, "fixtures": {}}


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.directory = Path(temp.name)
        for name in ARTIFACTS:
            (self.directory / name).write_text(name, encoding="utf-8")
        self.write_manifest()

    def write_manifest(self, mode="ad-hoc", submission_id=None):
        write_manifest(
            self.directory, SOURCE, mode=mode, submission_id=submission_id,
            app=APP_EVIDENCE, build=BUILD_EVIDENCE,
        )

    def test_accepts_matching_preview(self):
        self.assertEqual(read_verified_manifest(self.directory), (SOURCE, "ad-hoc", APP_EVIDENCE))

    def test_rejects_tampered_or_missing_payload(self):
        archive = self.directory / APP_ARCHIVE
        archive.write_bytes(b"modified")
        with self.assertRaisesRegex(ValueError, "differs from the manifest"):
            read_verified_manifest(self.directory)
        archive.unlink()
        with self.assertRaisesRegex(ValueError, "missing regular preview artifact"):
            read_verified_manifest(self.directory)

    def test_rejects_artifact_path_outside_the_preview(self):
        path = self.directory / MANIFEST
        manifest = json.loads(path.read_text())
        manifest["artifacts"]["../anything"] = manifest["artifacts"].pop(APP_ARCHIVE)
        path.write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, "artifact list"):
            read_verified_manifest(self.directory)

    def test_rejects_symlinked_payload_even_when_bytes_match(self):
        archive = self.directory / APP_ARCHIVE
        target = self.directory / "same-contents"
        archive.rename(target)
        archive.symlink_to(target)
        with self.assertRaisesRegex(ValueError, "missing regular preview artifact"):
            read_verified_manifest(self.directory)

    def test_distinguishes_local_and_developer_id_manifests(self):
        self.write_manifest("developer-id")
        with self.assertRaisesRegex(ValueError, "missing or invalid Apple submission ID"):
            read_verified_manifest(self.directory)

        submission = "2efe2717-52ef-43a5-96dc-0797e4ca1041"
        self.write_manifest(submission_id=submission)
        with self.assertRaisesRegex(ValueError, "ad-hoc preview"):
            read_verified_manifest(self.directory)

        self.write_manifest("developer-id", submission)
        self.assertEqual(read_verified_manifest(self.directory), (SOURCE, "developer-id", APP_EVIDENCE))
