#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
out=$1
mkdir -p "$out"
out=$(CDPATH= cd -- "$out" && pwd)
clang=${CLANG:-$(xcrun --find clang)}
linker=${LLD_LINK:-$(rustc --print sysroot)/lib/rustlib/aarch64-apple-darwin/bin/rust-lld}
for tool in "$clang" "$linker"; do
    lipo "$tool" -verify_arch arm64
done
link() {
    case "${linker##*/}" in
        rust-lld) arch -arm64 "$linker" -flavor link "$@" ;;
        *) arch -arm64 "$linker" "$@" ;;
    esac
}

tmp=$(mktemp -d "${TMPDIR:-/tmp}/xenora-crt-wformat-budget.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"
cat > kernel32.def <<'DEF'
LIBRARY kernel32.dll
EXPORTS
    ExitProcess
    VirtualAlloc
    VirtualFree
DEF
cat > ucrtbase.def <<'DEF'
LIBRARY ucrtbase.dll
EXPORTS
    __stdio_common_vswprintf
    memset
DEF
for module in kernel32 ucrtbase; do
    link /lib "/def:$module.def" /machine:x64 "/out:$module.lib"
done

build() {
    name=$1
    shift
    arch -arm64 "$clang" --target=x86_64-pc-windows-msvc -std=c11 -O1 -Wall -Wextra -Werror \
        -ffreestanding -fno-builtin -fno-stack-protector -fno-vectorize -fno-slp-vectorize \
        "$@" -c "$here/crt_wformat_budget.c" -o guest.obj
    link /entry:entry /subsystem:console /nodefaultlib /timestamp:0 /machine:x64 \
        "/out:$out/crt-wformat-budget-$name.exe" guest.obj kernel32.lib ucrtbase.lib
}
build controls
build wide -DOVER_WIDE
build narrow -DOVER_NARROW
