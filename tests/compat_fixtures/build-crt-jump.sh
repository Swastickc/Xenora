#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Build tests/compat_fixtures/crt_jump.c (#632) as x86-64 PE32+ executables
# linked with the mingw-w64 C runtime (UCRT flavour). With -fno-builtin the
# jump services are imports of the private CRT api set, exactly as busybox
# links them from msvcrt.dll.
#
#   build-crt-jump.sh [output directory]      default: artifacts/crt-jump
#
# Tool, native to the host:
#   MINGW_GCC  an x86_64-w64-mingw32-gcc. Default: Homebrew mingw-w64 if
#              installed, otherwise the one on PATH.
#
# Nothing from mingw-w64 is stored in the repository; the executables are test
# output and are not committed.
set -eu
cd "$(dirname "$0")/../.."
root=$PWD
src="$root/tests/compat_fixtures/crt_jump.c"
out=${1:-"$root/artifacts/crt-jump"}

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
cd "$out"
"$gcc" --version | head -1 > toolchain.txt

build() {
    "$gcc" "-$2" -Wall -Wextra -Werror -fno-builtin -o "$1.exe" "$src"
}
for level in O0 O2; do
    build "crt-jump-$level" "$level"
done
