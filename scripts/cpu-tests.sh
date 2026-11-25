#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Deterministic CPU-only selection of the workspace test targets (#159).
#
#   scripts/cpu-tests.sh --list         print every default-feature test target
#   scripts/cpu-tests.sh --shard K/N    run shard K of N (1 <= K <= N)
#   scripts/cpu-tests.sh --check N      prove N shards cover every target once
#
# Targets are the ones `cargo test --locked --workspace` builds with default
# features, sorted by package, kind and name. Shard K takes every target whose
# index i in that order has i mod N == K - 1; shard 1 also runs the doctests.
# New targets join a shard automatically. The selection never replaces the full
# gate, `cargo test --locked --workspace`; `gpu-tests` targets stay in
# scripts/test-gpu.sh.
set -eu
cd "$(dirname "$0")/.."
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}

usage() {
    echo "usage: $0 --list | --shard K/N | --check N" >&2
    exit 2
}

# Prints "package selector..." for each test target, one per line, sorted.
# Fails when Cargo cannot build every target, so a partial list never runs.
targets() {
    json=$(cargo test --locked --workspace --no-run --message-format=json) || {
        echo "cpu-tests: cargo could not build the workspace test targets" >&2
        exit 1
    }
    printf '%s\n' "$json" | python3 -c '
import json, sys
rows = set()
for line in sys.stdin:
    m = json.loads(line)
    if m.get("reason") != "compiler-artifact" or not m.get("executable"):
        continue
    if not m["profile"]["test"]:
        continue
    pkg = m["package_id"].split("#")[-1].split("@")[0]
    kind, name = m["target"]["kind"][0], m["target"]["name"]
    sel = {"test": "--test " + name, "bin": "--bin " + name}.get(kind, "--lib")
    rows.add(f"{pkg} {sel}")
if not rows:
    sys.exit("cpu-tests: no test targets found; does the workspace build?")
print("\n".join(sorted(rows)))
'
}

# Keeps the lines of stdin that belong to shard $1 of $2.
select_shard() {
    awk -v k="$1" -v n="$2" '(NR - 1) % n == k - 1'
}

parse_count() {
    case $1 in '' | *[!0-9]*) usage ;; esac
    [ "$1" -ge 1 ] || usage
}

[ $# -ge 1 ] || usage
case $1 in
--list)
    [ $# -eq 1 ] || usage
    targets
    ;;
--check)
    [ $# -eq 2 ] || usage
    parse_count "$2"
    all=$(targets)
    total=$(printf '%s\n' "$all" | wc -l | tr -d ' ')
    union=$(k=1; while [ "$k" -le "$2" ]; do
        printf '%s\n' "$all" | select_shard "$k" "$2"
        k=$((k + 1))
    done)
    if [ "$(printf '%s\n' "$union" | sort)" != "$(printf '%s\n' "$all" | sort)" ]; then
        echo "cpu-tests: $2 shards do not cover the $total targets exactly once" >&2
        exit 1
    fi
    echo "cpu-tests: $2 shards cover all $total targets exactly once"
    ;;
--shard)
    [ $# -eq 2 ] || usage
    k=${2%/*}
    n=${2#*/}
    [ "$k/$n" = "$2" ] || usage
    parse_count "$k"
    parse_count "$n"
    [ "$k" -le "$n" ] || usage
    all=$(targets)
    selected=$(printf '%s\n' "$all" | select_shard "$k" "$n")
    failed=0
    # Word splitting of $line is intended: it holds "package --test name".
    while IFS= read -r line; do
        [ -n "$line" ] || continue
        echo "cpu-tests[$k/$n]: cargo test --locked -p $line"
        # shellcheck disable=SC2086
        cargo test --locked -p $line || failed=$((failed + 1))
    done <<EOF
$selected
EOF
    if [ "$k" -eq 1 ]; then
        echo "cpu-tests[$k/$n]: cargo test --locked --workspace --doc"
        cargo test --locked --workspace --doc || failed=$((failed + 1))
    fi
    if [ "$failed" -ne 0 ]; then
        echo "cpu-tests[$k/$n]: $failed target(s) failed" >&2
        exit 1
    fi
    echo "cpu-tests[$k/$n]: all selected targets passed"
    ;;
*) usage ;;
esac
