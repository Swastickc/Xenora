#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

case $# in
    0) check=false ;;
    1) [ "$1" = --check ] || { echo "usage: $0 [--check]" >&2; exit 2; }; check=true ;;
    *) echo "usage: $0 [--check]" >&2; exit 2 ;;
esac

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
clang=${CLANG:-/opt/homebrew/opt/llvm/bin/clang}
linker=${LLD_LINK:-/opt/homebrew/opt/lld/bin/lld-link}
if [ -z "${CLANG:-}" ] && [ ! -x "$clang" ]; then
    clang=$(command -v clang)
fi
if [ -z "${LLD_LINK:-}" ] && [ ! -x "$linker" ]; then
    linker="$(rustc --print sysroot)/lib/rustlib/aarch64-apple-darwin/bin/rust-lld"
fi
for tool in "$clang" "$linker"; do
    if [ ! -x "$tool" ] || ! lipo -verify_arch arm64 "$tool"; then
        printf 'Native ARM64 tool required: %s\n' "$tool" >&2
        exit 1
    fi
done

compile() {
    arch -arm64 "$clang" --target=x86_64-pc-windows-msvc -O1 -Wall -Wextra -Werror \
        -ffreestanding -fno-builtin -fno-stack-protector -fno-vectorize -fno-slp-vectorize "$@"
}

link() {
    case "${linker##*/}" in
        rust-lld) arch -arm64 "$linker" -flavor link "$@" ;;
        *) arch -arm64 "$linker" "$@" ;;
    esac
}

scratch=$(mktemp -d "${TMPDIR:-/tmp}/xenora-package-scene.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
cd "$scratch"
mkdir reject

cat > kernel32.def <<'DEF'
LIBRARY kernel32.dll
EXPORTS
    CloseHandle
    CreateEventW
    CreateFileW
    ExitProcess
    SetEvent
    SetFilePointerEx
    WaitForSingleObject
    WriteFile
DEF
cat > xenora_graphics.def <<'DEF'
LIBRARY xenora_graphics.dll
EXPORTS
    XenoraCloseCommandList
    XenoraCreateBuffer
    XenoraCreateCommandAllocator
    XenoraCreateCommandList
    XenoraCreateCommandQueue
    XenoraCreateDevice
    XenoraCreateRenderTarget
    XenoraDestroyCommandAllocator
    XenoraDestroyCommandList
    XenoraDestroyCommandQueue
    XenoraDestroyDevice
    XenoraDestroyResource
    XenoraDraw
    XenoraExecuteCommandLists
    XenoraReadbackResource
    XenoraSetRenderTarget
    XenoraSetVertexBuffer
    XenoraSetViewport
DEF
for module in kernel32 xenora_graphics; do
    link /lib "/def:$module.def" /machine:x64 "/out:$module.lib"
done

compile -DREJECT_ATTACH=0 -c "$here/scene-provider.c" -o provider.obj
link /dll /entry:DllMain /nodefaultlib /dynamicbase /base:0x140000000 /timestamp:0 /machine:x64 \
    /out:xenora_scene.dll /implib:xenora_scene.lib provider.obj
compile -DREJECT_ATTACH=1 -c "$here/scene-provider.c" -o rejected.obj
link /dll /entry:DllMain /nodefaultlib /dynamicbase /base:0x140000000 /timestamp:0 /machine:x64 \
    /out:reject/xenora_scene.dll /implib:rejected.lib rejected.obj
compile -c "$here/scene-app.c" -o app.obj
link /entry:entry /subsystem:console /nodefaultlib /dynamicbase /base:0x150000000 /timestamp:0 /machine:x64 \
    /out:xenora-package-scene.exe app.obj xenora_scene.lib kernel32.lib xenora_graphics.lib

for name in xenora-package-scene.exe xenora_scene.dll reject/xenora_scene.dll; do
    if "$check"; then
        cmp "$name" "$here/$name"
    else
        mkdir -p "$(dirname "$here/$name")"
        install -m 0644 "$name" "$here/$name"
    fi
done
