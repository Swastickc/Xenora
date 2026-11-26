#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu
cd "$(dirname "$0")/.."

export MTL_DEBUG_LAYER=1
export MTL_SHADER_VALIDATION=1
export MTL_SHADER_VALIDATION_REPORT_TO_STDERR=1
export MTL_SHADER_VALIDATION_ABORT_ON_FAULT=1
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}

. scripts/lib/timeout.sh

cargo build --locked -p xenora
# `timeout` may be the shell function from scripts/lib/timeout.sh, which `exec`
# cannot run. The demo is the last command, so its status is still the script's.
timeout 30s cargo run --locked -p xenora -- demo
