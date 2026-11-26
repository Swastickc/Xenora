# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise the graphics dependency audit with small Cargo resolve graphs."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


class DependencyAuditTests(unittest.TestCase):
    def audit(self, members, edges, *, external_names=None):
        names = {package: package for package in members}
        names.update(external_names or {})
        graph = {
            "workspace_members": members,
            "packages": [
                {"id": package, "name": name} for package, name in names.items()
            ],
            "resolve": {
                "nodes": [
                    {"id": package, "dependencies": edges.get(package, [])}
                    for package in names
                ],
            },
        }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cargo = root / "cargo"
            cargo.write_text(
                '#!/bin/sh\n[ "$*" = "metadata --format-version 1 --locked" ] || exit 2\n'
                'cat "$TEST_METADATA"\n',
                encoding="utf-8",
            )
            cargo.chmod(0o755)
            metadata = root / "metadata.json"
            metadata.write_text(json.dumps(graph), encoding="utf-8")
            return subprocess.run(
                ["sh", str(ROOT / "scripts/audit/dependencies.sh")],
                cwd=root,
                env={
                    **os.environ,
                    "PATH": f"{root}{os.pathsep}{os.environ['PATH']}",
                    "TEST_METADATA": str(metadata),
                },
                capture_output=True,
                encoding="utf-8",
                check=False,
            )

    def test_texture_cannot_depend_on_an_x86_package(self):
        result = self.audit(
            ["xenora-decoder", "xenora-texture"],
            {"xenora-texture": ["iced"]},
            external_names={"iced": "iced-x86"},
        )

        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("xenora-texture depends on iced-x86", result.stderr)

    def test_graphics_cannot_reach_workspace_loader_transitively(self):
        result = self.audit(
            ["xenora-decoder", "xenora-loader", "xenora-texture"],
            {
                "xenora-decoder": ["xenora-texture"],
                "xenora-texture": ["xenora-loader"],
            },
        )

        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("xenora-decoder depends on xenora-loader", result.stderr)
        self.assertIn("xenora-texture depends on xenora-loader", result.stderr)

    def test_ordinary_edges_and_external_loader_names_remain_allowed(self):
        result = self.audit(
            ["xenora-decoder", "xenora-ref", "xenora-texture", "xenora-loader"],
            {
                "xenora-decoder": ["xenora-ref", "xenora-texture"],
                "xenora-ref": ["external-texture"],
                "xenora-texture": ["external-loader"],
                "xenora-loader": ["xenora-decoder"],
            },
            external_names={"external-loader": "xenora-loader", "external-texture": "texture"},
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("3 graphics crates", result.stdout)

    def test_reference_remains_independent(self):
        for target in ("xenora-decoder", "xenora-texture"):
            with self.subTest(target=target):
                result = self.audit(
                    ["xenora-ref", target],
                    {"xenora-ref": [target]},
                )

                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertIn(f"xenora-ref depends on {target}", result.stderr)
