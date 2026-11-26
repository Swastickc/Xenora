#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Provide `timeout` on hosts without GNU coreutils.
#
# macOS does not ship GNU timeout. Python 3 is already a project requirement, so
# this defines a bounded wrapper with the same statuses as the GNU tool: 124 when
# the command outlives its duration, 127 when it cannot be started. Sourced by
# scripts/test-gpu.sh and scripts/demo.sh so the fallback has one definition.
#
# Do not call the wrapper through `exec`: exec cannot run a shell function.
if ! command -v timeout >/dev/null 2>&1; then
    timeout() {
        duration=$1
        shift
        python3 - "$duration" "$@" <<'PY'
import os
import re
import signal
import subprocess
import sys

text = sys.argv[1]
match = re.fullmatch(r"(\d+(?:\.\d+)?)([smh]?)", text)
if match is None:
    print(f"timeout: invalid duration {text!r}", file=sys.stderr)
    raise SystemExit(2)
seconds = float(match.group(1)) * {"": 1, "s": 1, "m": 60, "h": 3600}[match.group(2)]
try:
    process = subprocess.Popen(sys.argv[2:], start_new_session=True)
except FileNotFoundError as error:
    print(f"timeout: {error}", file=sys.stderr)
    raise SystemExit(127)
try:
    code = process.wait(timeout=seconds)
except subprocess.TimeoutExpired:
    os.killpg(process.pid, signal.SIGKILL)
    process.wait()
    print(f"timeout: command exceeded {seconds:g} seconds", file=sys.stderr)
    raise SystemExit(124)
raise SystemExit(code)
PY
    }
fi
