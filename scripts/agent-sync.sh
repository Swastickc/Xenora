#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Keeps a checkout in step with the real upstream repository (not a fork's origin).
#   agent-sync.sh [--once] [--interval MINUTES] [--merge] [--setup]
# --setup  only make sure the `upstream` remote exists, then exit
# --once   one fetch and report, then exit (default: repeat every 12 minutes)
# --merge  merge upstream/main into the current task branch if the tree is clean
# After each fetch it lists new upstream commits that touch files of tasks you hold, so
# interface changes landing during your work are noticed. Nothing is merged unless --merge.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
PYTHONPATH="$root/scripts"
export PYTHONPATH
aq() { python3 -m agent_queue --config "$root/.github/agent-queue.json" "$@"; }

once=no
setup=no
merge=no
interval=12
while [ $# -gt 0 ]; do
    case "$1" in
        --once) once=yes ;;
        --setup) setup=yes ;;
        --merge) merge=yes ;;
        --interval) shift; interval=${1:?minutes} ;;
        *) echo "usage: agent-sync.sh [--once] [--interval 10-15] [--merge] [--setup]" >&2; exit 2 ;;
    esac
    shift
done
case "$interval" in ''|*[!0-9]*) echo "interval must be whole minutes" >&2; exit 2 ;; esac
if [ "$interval" -lt 10 ] || [ "$interval" -gt 15 ]; then
    echo "interval must be 10-15 minutes" >&2
    exit 2
fi

upstream_repo=$(aq config-get upstream)
url="https://github.com/$upstream_repo.git"
git -C "$root" rev-parse --git-dir >/dev/null

if ! git -C "$root" remote get-url upstream >/dev/null 2>&1; then
    git -C "$root" remote add upstream "$url"
    echo "added remote upstream -> $url"
elif [ "$(git -C "$root" remote get-url upstream)" != "$url" ]; then
    echo "remote upstream is $(git -C "$root" remote get-url upstream), expected $url; fix it by hand (origin is never changed)" >&2
    exit 1
fi
[ "$setup" = no ] || exit 0

pass() {
    old=$(git -C "$root" rev-parse -q --verify refs/remotes/upstream/main || true)
    git -C "$root" fetch --quiet upstream
    new=$(git -C "$root" rev-parse refs/remotes/upstream/main)
    if [ -n "$old" ] && [ "$old" != "$new" ]; then
        echo "upstream/main moved: $(git -C "$root" rev-list --count "$old..$new") new commit(s)"
        tmp=$(mktemp -d "${TMPDIR:-/tmp}/agent-sync.XXXXXX")
        login=$(gh api user -q .login)
        aq gather --repo "$upstream_repo" --no-prs > "$tmp/state.json"
        git -C "$root" log --no-renames --name-only --format='@%h' "$old..$new" |
            awk '/^@/ {sha=substr($0,2); next} NF {print sha "\t" $0}' > "$tmp/files.tsv"
        aq touch --state "$tmp/state.json" --login "$login" < "$tmp/files.tsv" > "$tmp/hits.json"
        if [ "$(tr -d ' \n' < "$tmp/hits.json")" != "[]" ]; then
            echo "upstream changed files of tasks you hold (check for interface changes):"
            cat "$tmp/hits.json"
        fi
        rm -rf "$tmp"
    elif [ -z "$old" ]; then
        echo "fetched upstream/main at $(printf '%s' "$new" | cut -c1-12)"
    else
        echo "upstream/main unchanged"
    fi
    if [ "$merge" = yes ]; then
        if [ -n "$(git -C "$root" status --porcelain)" ]; then
            echo "not merging: working tree not clean" >&2
        else
            git -C "$root" merge --no-edit upstream/main
        fi
    fi
}

pass
[ "$once" = yes ] && exit 0
while true; do
    sleep $((interval * 60))
    pass
done
