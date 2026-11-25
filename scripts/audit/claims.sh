#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu
cd "$(dirname "$0")/../.."
exec python3 - <<'PY'
from collections import Counter
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib

registry = Path("docs/claims.toml")
claims = tomllib.loads(registry.read_text(encoding="utf-8"))["claim"]
classes = {}
errors = []
claim_id = re.compile(r"\bCLAIM-[A-Z0-9]+(?:-[A-Z0-9]+)*\b")

for entry in claims:
    identifier = entry.get("id", "")
    evidence = entry.get("class")
    if not claim_id.fullmatch(identifier) or identifier in classes:
        errors.append(f"invalid or duplicate claim ID {identifier!r}")
    classes[identifier] = evidence
    if evidence not in {"A", "B", "C", "D", "E", "F", "U"}:
        errors.append(f"{identifier}: invalid evidence class")
    sources = entry.get("sources")
    if not isinstance(sources, list) or not sources or not all(
        isinstance(source, str) and source.strip() for source in sources
    ):
        errors.append(f"{identifier}: expected one or more sources")
    for field in ("claim", "architecture", "retrieved", "test", "uncertainty"):
        if not str(entry.get(field, "")).strip():
            errors.append(f"{identifier}: missing {field}")

roots = ("README.md", "AGENTS.md", "docs", "crates")
paths = subprocess.check_output(
    ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", *roots]
)
for name in sorted(set(paths.split(b"\0")) - {b""}):
    source = Path(os.fsdecode(name))
    root = source.parts[0]
    if root == "docs":
        supported = source.suffix in {".md", ".toml", ".rs"}
    elif root == "crates":
        supported = source.suffix == ".rs"
    else:
        supported = len(source.parts) == 1
    if not supported or source == registry or not source.is_file():
        continue
    for reference in set(claim_id.findall(source.read_text(encoding="utf-8"))) - classes.keys():
        errors.append(f"{source}: unregistered {reference}")

status = tomllib.loads(Path("docs/status.toml").read_text(encoding="utf-8"))
for feature in status["feature"]:
    verification = feature.get("hardware_verification", "")
    if verification.casefold() != "verified":
        continue
    references = feature.get("claims", [])
    evidence = {classes.get(reference) for reference in references}
    if not references or not evidence <= {"A", "B"} or "A" not in evidence:
        errors.append(f"{feature['id']}: verified hardware needs class A evidence")
    if not feature.get("test") or not feature.get("correctness", "").endswith("passed"):
        errors.append(f"{feature['id']}: verified hardware needs a recorded passing test")

if errors:
    sys.exit("\n".join(sorted(errors)))

counts = Counter(classes.values())
print(f"Checked {len(classes)} claims ({', '.join(f'{k}={v}' for k, v in sorted(counts.items()))}).")
PY
