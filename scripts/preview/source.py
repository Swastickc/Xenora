# SPDX-License-Identifier: GPL-3.0-or-later
"""Source identity and bundle versioning for contributor previews."""

import plistlib
import re
import tarfile
import tomllib
from dataclasses import dataclass
from pathlib import Path

GIT_REVISION = re.compile(r"(?:[0-9a-f]{40}|[0-9a-f]{64})")
MARKETING_VERSION = re.compile(r"(?:0|[1-9][0-9]*)(?:\.(?:0|[1-9][0-9]*)){2}")
BUILD_NUMBER = re.compile(r"[1-9][0-9]{0,3}(?:\.(?:0|[1-9][0-9]?)){0,2}")


@dataclass(frozen=True)
class Source:
    revision: str
    version: str
    build_number: str

    def __post_init__(self):
        if not isinstance(self.revision, str) or not GIT_REVISION.fullmatch(self.revision):
            raise ValueError("expected a full Git commit ID")
        if not isinstance(self.version, str) or not MARKETING_VERSION.fullmatch(self.version):
            raise ValueError("workspace version must have three numeric components")
        validate_build_number(self.build_number)

    def bundle_fields(self):
        return {
            "CFBundleShortVersionString": self.version,
            "CFBundleVersion": self.build_number,
            "XenoraSourceRevision": self.revision,
        }


def validate_build_number(value: str):
    if not isinstance(value, str) or not BUILD_NUMBER.fullmatch(value):
        raise ValueError("build number must look like 123 or 1.2.3 (no leading zeroes)")


def read_version(source: Path) -> str:
    with (source / "Cargo.toml").open("rb") as cargo_toml:
        document = tomllib.load(cargo_toml)
    try:
        return document["workspace"]["package"]["version"]
    except (KeyError, TypeError) as error:
        raise ValueError("Cargo.toml is missing workspace.package.version") from error


def snapshot(checkout: Path, destination: Path, build_number: str, *, run) -> Source:
    validate_build_number(build_number)
    revision = run(["git", "rev-parse", "--verify", "HEAD^{commit}"], cwd=checkout, capture=True).strip()
    if not GIT_REVISION.fullmatch(revision):
        raise ValueError("git did not return a full commit ID")
    changes = run(
        ["git", "status", "--porcelain=v1", "--untracked-files=no"],
        cwd=checkout,
        capture=True,
    )
    if changes:
        raise ValueError("commit tracked changes before building a preview")

    archive = destination.with_suffix(".tar")
    run(["git", "archive", "--format=tar", f"--output={archive}", revision], cwd=checkout)
    destination.mkdir()
    with tarfile.open(archive) as contents:
        contents.extractall(destination, filter="data")
    archive.unlink()
    return Source(revision, read_version(destination), build_number)


def stamp_bundle(app: Path, source: Source):
    info_path = app / "Contents" / "Info.plist"
    with info_path.open("rb") as stream:
        info = plistlib.load(stream)
    info.update(source.bundle_fields())
    with info_path.open("wb") as stream:
        plistlib.dump(info, stream)
