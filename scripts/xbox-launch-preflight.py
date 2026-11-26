#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Inspect a loose Xbox launch fixture with Xenora's static PE probe."""

from __future__ import annotations

import argparse
import csv
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import tempfile
from xml.parsers import expat


MAX_CONFIG_BYTES = 256 * 1024
MAX_PE_BYTES = 64 * 1024 * 1024
MAX_XML_DEPTH = 32
TARGETS = {"Scarlett", "XboxOne", "PC"}


class PreflightError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


def read_regular(path: Path, limit: int) -> bytes:
    # Avoid waiting for a producer if a path names a pipe.
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NONBLOCK), "rb") as source:
        info = os.fstat(source.fileno())
        if not stat.S_ISREG(info.st_mode):
            raise PreflightError("not_regular_file", "Expected a regular file")
        if info.st_size > limit:
            raise PreflightError("file_too_large", f"File exceeds the {limit}-byte limit")
        contents = source.read(limit + 1)
    if len(contents) > limit:
        raise PreflightError("file_too_large", f"File exceeds the {limit}-byte limit")
    return contents


def executable_name(name: str) -> str:
    parts = name.replace("\\", "/").split("/")
    if (
        not name.lower().endswith(".exe")
        or any(
            not part
            or part.endswith((".", " "))
            or re.search(r'[\x00-\x1f<>:"|?*]', part)
            for part in parts
        )
    ):
        raise PreflightError("invalid_executable_name", "Expected a relative .exe path")
    return "/".join(parts)


def parse_config(contents: bytes) -> dict:
    elements: list[str] = []
    executables: dict[str, dict] = {}
    declarations = {"config_version": None, "identity": None, "title_id": None}
    seen: set[tuple[str, ...]] = set()
    title_id_text: list[str] = []
    parser = expat.ParserCreate(namespace_separator=" ")

    def reject_doctype(*_):
        raise PreflightError("unsupported_xml", "Document type declarations are unsupported")

    def start(name, attributes):
        elements.append(name)
        if len(elements) > MAX_XML_DEPTH:
            raise PreflightError("xml_too_deep", "XML nesting exceeds the preflight limit")
        path = tuple(elements)
        if len(path) == 1 and path != ("Game",):
            raise PreflightError("invalid_root", "Expected the Game element")
        if len(path) == 2 and name in {"Identity", "TitleId", "ExecutableList"}:
            if path in seen:
                raise PreflightError("duplicate_element", f"Multiple {name} elements")
            seen.add(path)
        if path == ("Game",):
            version = attributes.get("configVersion")
            if version not in {"0", "1"}:
                raise PreflightError(
                    "unsupported_config_version", "Expected configVersion 0 or 1"
                )
            declarations["config_version"] = int(version)
        elif path == ("Game", "Identity"):
            declarations["identity"] = {
                field: attributes[field]
                for field in ("Name", "Publisher", "Version", "ResourceId")
                if field in attributes
            }
        elif path == ("Game", "ExecutableList", "Executable"):
            file_name = executable_name(attributes.get("Name", ""))
            key = file_name.casefold()
            if key in executables:
                raise PreflightError("duplicate_executable", f"Ambiguous path {file_name!r}")
            target = attributes.get("TargetDeviceFamily")
            if target is not None and target not in TARGETS:
                raise PreflightError("unknown_target", f"Unsupported TargetDeviceFamily {target!r}")
            dev_only = attributes.get("IsDevOnly")
            if dev_only not in {None, "true", "false", "1", "0"}:
                raise PreflightError("invalid_dev_only", "Expected an XML boolean for IsDevOnly")
            executables[key] = {
                "name": file_name,
                "target_device_family": target,
                "id": attributes.get("Id"),
                "is_dev_only": None if dev_only is None else dev_only in {"true", "1"},
                "architecture": attributes.get("Architecture"),
            }
        elif len(path) > 2 and path[:2] == ("Game", "TitleId"):
            raise PreflightError("invalid_title_id", "Expected text in TitleId")

    def characters(data):
        if elements == ["Game", "TitleId"]:
            title_id_text.append(data)

    def end(_name):
        if elements == ["Game", "TitleId"]:
            title_id = "".join(title_id_text).strip()
            if not re.fullmatch(r"[0-9a-fA-F]{8}", title_id):
                raise PreflightError("invalid_title_id", "Expected eight hexadecimal digits")
            declarations["title_id"] = title_id
        elements.pop()

    parser.StartDoctypeDeclHandler = reject_doctype
    parser.StartElementHandler = start
    parser.EndElementHandler = end
    parser.CharacterDataHandler = characters
    try:
        parser.Parse(contents, True)
    except expat.ExpatError as error:
        raise PreflightError("invalid_xml", str(error)) from error
    declarations["executables"] = list(executables.values())
    return declarations


def select_executable(declarations: dict, selection: str | None) -> dict:
    executables = declarations["executables"]
    if selection is not None:
        key = executable_name(selection).casefold()
        executables = [item for item in executables if item["name"].casefold() == key]
    if not executables:
        raise PreflightError("executable_not_declared", "No matching executable declaration")
    if len(executables) > 1:
        raise PreflightError("ambiguous_executable", "Select a declared executable with --exe")

    selected = executables[0]
    if selected["target_device_family"] != "Scarlett":
        raise PreflightError("not_scarlett", "Expected an explicit Scarlett target declaration")
    if selected["architecture"] not in {None, "x64"}:
        raise PreflightError("not_x64", "The declared architecture is outside the x64 profile")
    return selected


def probe(binary: Path, contents: bytes) -> dict:
    binary = binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="xenora-launch-") as directory:
        root = Path(directory)
        snapshot = root / "fixture.exe"
        snapshot.write_bytes(contents)
        result = subprocess.run(
            [binary, "probe", "--pe", snapshot.name, "--report", "report"],
            cwd=root,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            errors="replace",
            timeout=30,
            check=False,
        )
        if result.returncode:
            detail = next(
                (line for line in result.stderr.splitlines() if line.startswith("probe failed:")),
                "Xenora probe terminated without a report",
            )
            raise PreflightError("pe_probe_failed", detail[:512])
        try:
            report = json.loads((root / "report/candidate_static_report.json").read_text())
            with (root / "report/missing_imports.csv").open(newline="", encoding="utf-8") as rows:
                first_missing = next(csv.DictReader(rows), None)
        except (OSError, ValueError, csv.Error) as error:
            raise PreflightError("invalid_probe_report", "Cannot read the static PE report") from error
        report["first_listed_unresolved_ordinary_import"] = first_missing
        return report


def preflight(config: Path, binary: Path, selection: str | None) -> tuple[dict, int]:
    report = {
        "schema_version": 1,
        "profile": "experimental_xbox_series_preflight",
        "events": [],
    }
    stage = "manifest"
    try:
        config = config.resolve(strict=True)
        declarations = parse_config(read_regular(config, MAX_CONFIG_BYTES))
        report["manifest"] = declarations
        report["events"].append(stage)

        stage = "executable_selection"
        executable = select_executable(declarations, selection)
        report["selected_executable"] = executable
        report["events"].append(stage)

        stage = "executable_read"
        image = (config.parent / executable["name"]).resolve(strict=True)
        if not image.is_relative_to(config.parent):
            raise PreflightError("outside_fixture_root", "Executable resolves outside the fixture directory")
        contents = read_regular(image, MAX_PE_BYTES)
        report["events"].append(stage)

        stage = "static_pe_probe"
        report["static_pe"] = probe(binary, contents)
        report["events"].append(stage)
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        if isinstance(error, PreflightError):
            code, message = error.code, str(error)
        elif isinstance(error, subprocess.TimeoutExpired):
            code, message = "probe_timeout", "Static PE probe exceeded 30 seconds"
        elif isinstance(error, FileNotFoundError):
            code, message = "file_not_found", "Required file was not found"
        else:
            code, message = "input_error", "Could not read input or invoke the static PE probe"
        report["outcome"] = {"kind": "failed", "stage": stage, "code": code, "message": message}
        return report, 1

    report["outcome"] = {
        "kind": "blocked",
        "stage": "console_bootstrap",
        "code": "unverified_console_contract",
        "message": "Console bootstrap requires authorized evidence. Preflight completed.",
    }
    return report, 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True, type=Path, help="path to MicrosoftGame.config")
    parser.add_argument("--xenora", required=True, type=Path, help="path to the built Xenora CLI")
    parser.add_argument("--exe", help="declared executable path when the manifest lists several")
    args = parser.parse_args()
    report, status = preflight(args.config, args.xenora, args.exe)
    print(json.dumps(report, indent=2))
    return status


if __name__ == "__main__":
    raise SystemExit(main())
