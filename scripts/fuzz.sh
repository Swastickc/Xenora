#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu
cd "$(dirname "$0")/.."

target=${1:-}
case "$target" in
    instruction_stream|fixture|reference|runtime_services) ;;
    *) printf '%s\n' 'usage: scripts/fuzz.sh <instruction_stream|fixture|reference|runtime_services> [seconds]' >&2; exit 2 ;;
esac
seconds=${2:-600}
case "$seconds" in
    ''|*[!0-9]*) printf '%s\n' 'seconds must be a positive integer' >&2; exit 2 ;;
esac
[ "$seconds" -gt 0 ]
corpus="artifacts/fuzz/$target"
mkdir -p "$corpus"
for fixture in fixtures/rdna2/*.bin; do
    cp "$fixture" "$corpus/seed-${fixture##*/}"
done
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}
nightly_bin=$(dirname "$(rustup which --toolchain nightly rustc)")
export PATH="$nightly_bin:$PATH"
export RUSTC="$nightly_bin/rustc"
exec rustup run nightly cargo fuzz run "$target" "$corpus" -- \
    "-max_total_time=$seconds" -max_len=4096 -timeout=5 -rss_limit_mb=2048
