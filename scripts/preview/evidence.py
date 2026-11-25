# SPDX-License-Identifier: GPL-3.0-or-later
"""File integrity, source notices and a versioned preview manifest."""

import hashlib
import json
import shutil
from dataclasses import asdict
from pathlib import Path
from uuid import UUID

from .source import Source

APP_ARCHIVE = "Xenora.zip"
MANIFEST = "manifest.json"
NOTES = "PREVIEW.md"
NOTICE_FILES = {
    "Cargo.lock": "Cargo.lock",
    "LICENSE": "LICENSE",
    "LICENSE-AMD": "crates/isa/LICENSE-AMD",
    "THIRD_PARTY_NOTICES.md": "THIRD_PARTY_NOTICES.md",
    "bitflags-LICENSE-MIT": "licenses/bitflags-LICENSE-MIT",
    "objc2-LICENSE-MIT.txt": "licenses/objc2-LICENSE-MIT.txt",
    "objc2-LICENSE.md": "licenses/objc2-LICENSE.md",
    "dependency-licenses.md": "docs/licenses.md",
}
ARTIFACTS = (APP_ARCHIVE, NOTES, *NOTICE_FILES.keys())


def checksum(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def artifact_records(directory: Path):
    records = {}
    for name in ARTIFACTS:
        path = directory / name
        if path.is_symlink() or not path.is_file():
            raise ValueError(f"missing regular preview artifact: {name}")
        records[name] = {"sha256": checksum(path), "size": path.stat().st_size}
    return records


def copy_notices(source_root: Path, directory: Path, source: Source, mode: str):
    for name, relative in NOTICE_FILES.items():
        shutil.copyfile(source_root / relative, directory / name)
    source_url = "https://github.com/ranveersoni98/Xenora"
    (directory / NOTES).write_text(
        "# Xenora contributor preview\n\n"
        f"- Version {source.version}, build {source.build_number}\n"
        f"- Source commit `{source.revision}`\n"
        f"- Upstream repository <{source_url}>\n"
        f"- Cargo.lock SHA-256 `{checksum(source_root / 'Cargo.lock')}`\n"
        f"- Signing mode `{mode}`\n\n"
        "Build inputs come from the Git commit above. The repository's app build script\n"
        "and the locked dependency graph identify the build and source inputs. A commit\n"
        "from a contribution branch may be available on its fork before upstream. Keep\n"
        "this metadata and the accompanying notices with the archive. The packaged\n"
        "license texts are [GPL-3.0-or-later](LICENSE), [AMD](LICENSE-AMD),\n"
        "[bitflags](bitflags-LICENSE-MIT), [objc2](objc2-LICENSE-MIT.txt) and its\n"
        "[upstream notice](objc2-LICENSE.md). The [dependency review](dependency-licenses.md)\n"
        "documents the remaining release attribution and corresponding-source\n"
        "work for public distribution.\n\n"
        "This Apple Silicon preview covers original synthetic fixtures. Commercial game\n"
        "execution and authentic Xbox startup are unsupported.\n",
        encoding="utf-8",
    )


def write_manifest(
    directory: Path, source: Source, *, mode: str, submission_id: str | None, app: dict, build: dict
):
    manifest = {
        "schema_version": 1,
        "source": asdict(source),
        "signing": {"mode": mode, "submission_id": submission_id},
        "app": app,
        "build": build,
        "artifacts": artifact_records(directory),
    }
    (directory / MANIFEST).write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


def read_verified_manifest(directory: Path) -> tuple[Source, str, dict]:
    path = directory / MANIFEST
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 128 * 1024:
        raise ValueError("expected a regular manifest.json of at most 128 KiB")
    manifest = json.loads(path.read_text(encoding="utf-8"))
    try:
        if type(manifest["schema_version"]) is not int or manifest["schema_version"] != 1:
            raise ValueError("unsupported preview manifest schema")
        source = Source(**manifest["source"])
        signing = manifest["signing"]
        mode = signing["mode"]
        submission = signing["submission_id"]
        records = manifest["artifacts"]
        app = manifest["app"]
        build = manifest["build"]
    except (KeyError, TypeError) as error:
        raise ValueError("invalid preview manifest") from error
    if mode == "ad-hoc":
        if submission is not None:
            raise ValueError("ad-hoc preview cannot declare an Apple submission")
    elif mode == "developer-id":
        try:
            if not isinstance(submission, str) or str(UUID(submission)) != submission.lower():
                raise ValueError
        except ValueError:
            raise ValueError("missing or invalid Apple submission ID") from None
    else:
        raise ValueError("invalid preview signing mode")

    if not isinstance(app, dict) or not app:
        raise ValueError("manifest has no bundle inspection")
    if (
        not isinstance(build, dict)
        or build.get("locked_app_build_and_smoke") is not True
        or not isinstance(build.get("environment"), dict)
        or not isinstance(build.get("fixtures"), dict)
    ):
        raise ValueError("manifest has no build or fixture evidence")
    if not isinstance(records, dict) or set(records) != set(ARTIFACTS):
        raise ValueError("manifest artifact list differs from the preview format")
    for name, current in artifact_records(directory).items():
        if records[name] != current:
            raise ValueError(f"preview artifact differs from the manifest: {name}")
    return source, mode, app
