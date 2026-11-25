#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu
cd "$(dirname "$0")/../.."

uname -m
arch
test "$(uname -m)" = arm64
test "$(arch)" = arm64
test "$(sysctl -n sysctl.proc_translated)" = 0
rustc -vV
mkdir -p artifacts

cargo build --locked --workspace --message-format=json > artifacts/native-build.jsonl
cargo test --locked --workspace --all-features --no-run --message-format=json > artifacts/native-tests.jsonl
python3 - <<'PY'
import json
from pathlib import Path
import subprocess
import sys

sys.stdout.reconfigure(encoding="utf-8")

executables = set()
for filename in ("native-build.jsonl", "native-tests.jsonl"):
    for line in (Path("artifacts") / filename).read_text(encoding="utf-8").splitlines():
        entry = json.loads(line)
        if entry.get("executable"):
            executables.add(entry["executable"])
        for output in entry.get("filenames", ()):
            if output.endswith((".dylib", ".so")):
                executables.add(output)

if not executables:
    raise SystemExit("No Cargo executable or loadable dependency artifacts found.")
for path in sorted(executables):
    subprocess.run(["file", path], check=True)
    archs = subprocess.check_output(["lipo", "-archs", path], encoding="utf-8").strip()
    print(archs, flush=True)
    if archs != "arm64":
        raise SystemExit(f"Non-ARM64 build artifact: {path}")
    linkage = subprocess.check_output(["otool", "-L", path], encoding="utf-8")
    print(linkage, end="", flush=True)
    if any(name in linkage.casefold() for name in ("vulkan", "moltenvk")):
        raise SystemExit(f"Unexpected graphics linkage: {path}")
print(f"Checked {len(executables)} native executables and loadable dependencies.")
PY
