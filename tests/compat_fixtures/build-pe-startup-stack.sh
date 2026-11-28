#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Build original freestanding Win64 guests using native ARM64 tools.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
out=${1:-"$here/../../artifacts/pe-startup-stack"}
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

scratch=$(mktemp -d "${TMPDIR:-/tmp}/xenora-pe-stack.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
cd "$scratch"

cat > kernel32.def <<'DEF'
LIBRARY kernel32.dll
EXPORTS
    ExitProcess
    GetCurrentThreadStackLimits
DEF
link /lib /def:kernel32.def /machine:x64 /out:kernel32.lib

# Xenora maps the supported reservation eagerly. The C loop touches each page
# explicitly, so no compiler-provided Windows stack probing routine is needed.
arch -arm64 "$clang" --target=x86_64-pc-windows-msvc -std=c11 -O1 -Wall -Wextra -Werror \
    -ffreestanding -fno-builtin -fno-stack-protector -mno-stack-arg-probe \
    -fno-vectorize -fno-slp-vectorize -fno-unroll-loops \
    -c "$here/pe_startup_stack.c" -o guest.obj

for entry in entry small_entry; do
    link "/entry:$entry" /subsystem:console /nodefaultlib /timestamp:0 /machine:x64 \
        /stack:262144,131072 "/out:$out/$entry.exe" guest.obj kernel32.lib
done
