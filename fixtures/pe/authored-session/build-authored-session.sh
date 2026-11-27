#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
out=${1:-"$here/../../../artifacts/authored-session"}
mkdir -p "$out"
out=$(CDPATH= cd -- "$out" && pwd)

clang=${CLANG:-$(xcrun --find clang)}
linker=${LLD_LINK:-"$(rustc --print sysroot)/lib/rustlib/aarch64-apple-darwin/bin/rust-lld"}
for tool in "$clang" "$linker"; do
    case "$(file -bL "$tool")" in
        *arm64*) ;;
        *) echo "Expected a native ARM64 tool: $tool" >&2; exit 1 ;;
    esac
    lipo "$tool" -verify_arch arm64
done

link() {
    case "${linker##*/}" in
        rust-lld) "$linker" -flavor link "$@" ;;
        *) "$linker" "$@" ;;
    esac
}

temp=$(mktemp -d "${TMPDIR:-/tmp}/xenora-authored-session.XXXXXX")
trap 'rm -rf "$temp"' EXIT
cd "$temp"
cat > kernel32.def <<'DEF'
LIBRARY kernel32.dll
EXPORTS
    CreateFileW
    WriteFile
    ReadFile
    SetFilePointerEx
    CloseHandle
    Sleep
    GetTickCount64
    GetSystemTimeAsFileTime
    ExitProcess
DEF
cat > session.def <<'DEF'
LIBRARY xenora-session-v1.dll
EXPORTS
    XenoraSessionNextInput
    XenoraSessionPresent
    XenoraSessionFinish
DEF
cat > graphics.def <<'DEF'
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
    XenoraDestroyResource
    XenoraDestroyCommandList
    XenoraDestroyCommandAllocator
    XenoraDestroyCommandQueue
    XenoraDestroyDevice
DEF
for module in kernel32 session graphics; do
    link /lib "/def:$module.def" /machine:x64 "/out:$module.lib"
done
"$clang" --target=x86_64-pc-windows-msvc -O1 -Wall -Wextra -Werror \
    -ffreestanding -fno-builtin -fno-stack-protector -fno-vectorize -fno-slp-vectorize \
    -c "$here/session.c" -o guest.obj

build() {
    link "/entry:$1" /subsystem:console /nodefaultlib /timestamp:0 /machine:x64 \
        "/out:$out/$2.exe" guest.obj kernel32.lib session.lib graphics.lib
}

build entry authored-session
build missing_present_entry authored-session-missing-present
build extra_present_entry authored-session-extra-present
build leak_entry authored-session-leak
build stall_frame_entry authored-session-stall-frame
build deterministic_clock_entry authored-session-deterministic-clock
