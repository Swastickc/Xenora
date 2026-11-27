#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Build the original freestanding Win64 guest using native ARM64 tools.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
out=${1:-"$here/../../artifacts/debug-probe"}
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
        rust-lld) arch -arm64 "$linker" -flavor link "$@" ;;
        *) arch -arm64 "$linker" "$@" ;;
    esac
}

scratch=$(mktemp -d "${TMPDIR:-/tmp}/xenora-debug-probe.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
cd "$scratch"

cat > kernel32.def <<'DEF'
LIBRARY kernel32.dll
EXPORTS
    VirtualAlloc
    VirtualFree
    ExitProcess
DEF
cat > debug.def <<'DEF'
LIBRARY api-ms-win-core-debug-l1-1-0.dll
EXPORTS
    IsDebuggerPresent
    OutputDebugStringA
    OutputDebugStringW
DEF
for module in kernel32 debug; do
    link /lib "/def:$module.def" /machine:x64 "/out:$module.lib"
done

arch -arm64 "$clang" --target=x86_64-pc-windows-msvc -std=c11 -O1 -Wall -Wextra -Werror \
    -ffreestanding -fno-builtin -fno-stack-protector -fno-vectorize -fno-slp-vectorize \
    -c "$here/debug_probe.c" -o guest.obj
link /entry:entry /subsystem:console /nodefaultlib /timestamp:0 /machine:x64 \
    "/out:$out/debug-probe.exe" guest.obj kernel32.lib debug.lib
