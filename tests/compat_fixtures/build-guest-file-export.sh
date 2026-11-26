#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later

# Build the original hosted C input for the xenora guest-file export test.
# Usage: build-guest-file-export.sh [output directory]
set -eu
cd "$(dirname "$0")/../.."
root=$PWD
out=${1:-"$root/artifacts/guest-file-export"}
gcc=${MINGW_GCC:-}
if [ -z "$gcc" ]; then
    gcc=/opt/homebrew/opt/mingw-w64/bin/x86_64-w64-mingw32-gcc
    [ -x "$gcc" ] || gcc=$(command -v x86_64-w64-mingw32-gcc || true)
fi
if [ ! -x "$gcc" ]; then
    echo "missing native x86_64-w64-mingw32-gcc (set MINGW_GCC)" >&2
    exit 1
fi

mkdir -p "$out"
if [ "$(uname -s)" = Darwin ]; then
    for tool in "$gcc" "$("$gcc" -print-prog-name=cc1)" \
        "$("$gcc" -print-prog-name=as)" "$("$gcc" -print-prog-name=ld)"; do
        case "$(file -bL "$tool")" in
            *arm64*) ;;
            *) echo "ARM64 tool required: $tool" >&2; exit 1 ;;
        esac
        lipo -verify_arch arm64 "$tool"
        otool -L "$tool"
    done > "$out/toolchain-native.txt"
fi
"$gcc" --version | head -1 > "$out/toolchain.txt"
"$gcc" -O2 -Wall -Wextra -Werror -fno-builtin -Wl,--no-insert-timestamp \
    -o "$out/guest-file-export.exe" "$root/tests/compat_fixtures/guest_file_export.c"
