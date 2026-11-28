#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Build tests/compat_fixtures/guest_spawn.c as an x86-64 PE32+ executable
# linked with the mingw-w64 C runtime (UCRT flavour), for the `_spawnve`
# _P_WAIT test of issue #1040. With -fno-builtin every CRT function it calls
# is an import of the Universal CRT.
#
#   build-guest-spawn.sh [output directory]      default: artifacts/guest-spawn
#
# Tool, native to the host:
#   MINGW_GCC  an x86_64-w64-mingw32-gcc. Default: Homebrew mingw-w64 if
#              installed, otherwise the one on PATH.
#
# Nothing from mingw-w64 is stored in the repository; the executable is test
# output and is not committed.
set -eu
cd "$(dirname "$0")/../.."
root=$PWD
src="$root/tests/compat_fixtures/guest_spawn.c"
out=${1:-"$root/artifacts/guest-spawn"}

gcc=${MINGW_GCC:-}
if [ -z "$gcc" ]; then
    gcc=/opt/homebrew/opt/mingw-w64/bin/x86_64-w64-mingw32-gcc
    [ -x "$gcc" ] || gcc=$(command -v x86_64-w64-mingw32-gcc || true)
fi
if [ -z "$gcc" ] || [ ! -x "$gcc" ]; then
    echo "missing tool: x86_64-w64-mingw32-gcc (set MINGW_GCC, or install mingw-w64)" >&2
    exit 1
fi
# AGENTS.md: host tools must be ARM64.
if [ "$(uname -s)" = Darwin ]; then
    for tool in "$gcc" "$("$gcc" -print-prog-name=cc1)" \
        "$("$gcc" -print-prog-name=as)" "$("$gcc" -print-prog-name=ld)"; do
        case "$(lipo -archs "$tool")" in
            *arm64*) ;;
            *) echo "ARM64 tool required: $tool" >&2; exit 1 ;;
        esac
    done
fi

mkdir -p "$out"
"$gcc" -Wall -Wextra -Werror -fno-builtin -O1 -o "$out/guest-spawn.exe" "$src"
echo "built guest-spawn.exe in $out"
