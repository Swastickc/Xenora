#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu
cd "$(dirname "$0")/.."

# Timing runs must be on an idle machine with validation off.
export MTL_DEBUG_LAYER=${MTL_DEBUG_LAYER:-0}
export MTL_SHADER_VALIDATION=${MTL_SHADER_VALIDATION:-0}
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}

exec cargo run --locked --release -p xenora-benchmarks --bin xenora-bench -- "$@"
