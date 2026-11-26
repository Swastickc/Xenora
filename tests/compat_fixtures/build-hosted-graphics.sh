#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later

# Build an original hosted C application with mingw-w64 UCRT startup.
# Usage: build-hosted-graphics.sh [output directory]
set -eu
cd "$(dirname "$0")/../.."
root=$PWD
out=${1:-"$root/artifacts/hosted-graphics"}
src="$root/tests/compat_fixtures/hosted_graphics.c"
gcc=${MINGW_GCC:-}
if [ -z "$gcc" ]; then
    gcc=/opt/homebrew/opt/mingw-w64/bin/x86_64-w64-mingw32-gcc
    [ -x "$gcc" ] || gcc=$(command -v x86_64-w64-mingw32-gcc || true)
fi
if [ ! -x "$gcc" ]; then
    echo "missing native x86_64-w64-mingw32-gcc (set MINGW_GCC)" >&2
    exit 1
fi
dlltool=${gcc%gcc}dlltool
mkdir -p "$out"
cd "$out"
if [ "$(uname -s)" = Darwin ]; then
    for tool in "$gcc" "$dlltool" "$("$gcc" -print-prog-name=cc1)" \
        "$("$gcc" -print-prog-name=as)" "$("$gcc" -print-prog-name=ld)"; do
        case "$(file -bL "$tool")" in
            *arm64*) ;;
            *) echo "ARM64 tool required: $tool" >&2; exit 1 ;;
        esac
        lipo "$tool" -verify_arch arm64
        otool -L "$tool"
    done > toolchain-native.txt
fi
"$gcc" --version | head -1 > toolchain.txt

cat > xenora_graphics.def <<'DEF'
LIBRARY xenora_graphics.dll
EXPORTS
    XenoraCreateDevice
    XenoraCreateCommandQueue
    XenoraCreateCommandAllocator
    XenoraCreateCommandList
    XenoraCreateBuffer
    XenoraCreateRenderTarget
    XenoraSetVertexBuffer
    XenoraSetRenderTarget
    XenoraSetViewport
    XenoraDraw
    XenoraCloseCommandList
    XenoraExecuteCommandLists
    XenoraReadbackResource
    XenoraGetLastFrameInfo
    XenoraDestroyResource
    XenoraDestroyCommandList
    XenoraDestroyCommandAllocator
    XenoraDestroyCommandQueue
    XenoraDestroyDevice
DEF
"$dlltool" -d xenora_graphics.def -l xenora_graphics.a
for level in O0 O2; do
    "$gcc" "-$level" -Wall -Wextra -Werror -fno-builtin -Wl,--no-insert-timestamp \
        -o "hosted-graphics-$level.exe" "$src" xenora_graphics.a
done
