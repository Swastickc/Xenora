#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Build the original freestanding Win64 guest using native ARM64 tools.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
out=${1:-"$here/../../artifacts/crt-thread-rand"}
mkdir -p "$out"
out=$(CDPATH= cd -- "$out" && pwd)

clang=${CLANG:-$(xcrun --find clang)}
linker=${LLD_LINK:-"$(rustc --print sysroot)/lib/rustlib/aarch64-apple-darwin/bin/rust-lld"}
for tool in "$clang" "$linker"; do
    lipo "$tool" -verify_arch arm64
done

link() {
    case "${linker##*/}" in
        rust-lld) arch -arm64 "$linker" -flavor link "$@" ;;
        *) arch -arm64 "$linker" "$@" ;;
    esac
}

scratch=$(mktemp -d "${TMPDIR:-/tmp}/xenora-crt-thread-rand.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
cd "$scratch"

cat > kernel32.def <<'DEF'
LIBRARY kernel32.dll
EXPORTS
    CloseHandle
    CreateEventW
    CreateThread
    ExitProcess
    GetExitCodeThread
    SetEvent
    WaitForSingleObject
DEF
cat > msvcrt.def <<'DEF'
LIBRARY msvcrt.dll
EXPORTS
    rand
    srand
DEF
for module in kernel32 msvcrt; do
    link /lib "/def:$module.def" /machine:x64 "/out:$module.lib"
done

arch -arm64 "$clang" --target=x86_64-pc-windows-msvc -std=c11 -O1 -Wall -Wextra -Werror \
    -ffreestanding -fno-builtin -fno-stack-protector -fno-vectorize -fno-slp-vectorize \
    -c "$here/crt_thread_rand.c" -o guest.obj
link /entry:entry /subsystem:console /nodefaultlib /timestamp:0 /machine:x64 \
    "/out:$out/crt-thread-rand.exe" guest.obj kernel32.lib msvcrt.lib
