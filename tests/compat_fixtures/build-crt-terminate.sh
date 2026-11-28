#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
out=${1:-"$here/../../artifacts/crt-terminate"}
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

temp=$(mktemp -d "${TMPDIR:-/tmp}/xenora-crt-terminate.XXXXXX")
trap 'rm -rf "$temp"' EXIT
cd "$temp"
cat > kernel32.def <<'DEF'
LIBRARY kernel32.dll
EXPORTS
    CreateFileW
    WriteFile
    ExitProcess
DEF
for module in ucrtbase api-ms-win-crt-runtime-l1-1-0; do
    cat > "$module.def" <<DEF
LIBRARY $module.dll
EXPORTS
    atexit
    exit
    terminate
DEF
done
for module in kernel32 ucrtbase api-ms-win-crt-runtime-l1-1-0; do
    link /lib "/def:$module.def" /machine:x64 "/out:$module.lib"
done
"$clang" --target=x86_64-pc-windows-msvc -O1 -Wall -Wextra -Werror \
    -ffreestanding -fno-builtin -fno-stack-protector -fno-vectorize -fno-slp-vectorize \
    -c "$here/crt_terminate.c" -o guest.obj

build() {
    link "/entry:$1" /subsystem:console /nodefaultlib /timestamp:0 /machine:x64 \
        "/out:$out/$2.exe" guest.obj kernel32.lib "$3.lib"
}

build entry crt-terminate-ucrt ucrtbase
build entry crt-terminate-apiset api-ms-win-crt-runtime-l1-1-0
build control_entry crt-terminate-control ucrtbase
