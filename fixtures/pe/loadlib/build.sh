#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Build the original Xenora dynamic-load fixture: one provider DLL, two
# negative-control DLLs with the same embedded logical module name, and a
# no-CRT consumer EXE. All four are compiled by the native ARM64 LLVM
# toolchain into x86-64 PE32+ images.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
outdir=${1:-"$here"}
case "$outdir" in
    /*) ;;
    *) outdir="$PWD/$outdir" ;;
esac
mkdir -p "$outdir"
clang=${CLANG:-/opt/homebrew/opt/llvm/bin/clang}
linker=${LLD_LINK:-/opt/homebrew/opt/lld/bin/lld-link}
if [ ! -x "$linker" ]; then
    linker="$(rustc --print sysroot)/lib/rustlib/aarch64-apple-darwin/bin/rust-lld"
fi
for tool in "$clang" "$linker"; do
    case "$(file -bL "$tool")" in
        *arm64*) ;;
        *) echo "Expected a native ARM64 tool: $tool" >&2; exit 1 ;;
    esac
    if ! lipo "$tool" -verify_arch arm64; then
        echo "Expected a native ARM64 tool: $tool" >&2; exit 1
    fi
done
link() {
    case "${linker##*/}" in
        rust-lld) "$linker" -flavor link "$@" ;;
        *) "$linker" "$@" ;;
    esac
}
temp=$(mktemp -d "${TMPDIR:-/tmp}/xenora-loadlib.XXXXXX")
trap 'rm -rf "$temp"' EXIT
cd "$temp"

cat > xenora_loadlib.def <<'DEF'
LIBRARY xenora_loadlib.dll
EXPORTS
    XenoraTriple
    XenoraLibInit DATA
    XenoraLibTls DATA
    XenoraLibDetach DATA
DEF
cat > xenora_loadlib_missing.def <<'DEF'
LIBRARY xenora_loadlib.dll
EXPORTS
    XenoraLibInit DATA
    XenoraLibTls DATA
    XenoraLibDetach DATA
DEF
cat > kernel32.def <<'DEF'
LIBRARY kernel32.dll
EXPORTS
    LoadLibraryA
    GetProcAddress
    FreeLibrary
    ExitProcess
DEF
link /lib /machine:x64 "/def:kernel32.def" "/out:kernel32.lib"

flags="--target=x86_64-pc-windows-msvc -O1 -Wall -Wextra -Werror -ffreestanding -fno-builtin -fno-stack-protector -fno-vectorize -fno-slp-vectorize"
"$clang" $flags -c "$here/library.c" -o library.obj
"$clang" $flags -DXENORA_NO_TRIPLE_EXPORT -c "$here/library.c" -o library-missing.obj
"$clang" $flags -DXENORA_ATTACH_FAIL -c "$here/library.c" -o library-fail.obj
"$clang" $flags -c "$here/consumer.c" -o consumer.obj

link /dll /entry:DllMain /nodefaultlib /dynamicbase /timestamp:0 /machine:x64 \
    "/def:xenora_loadlib.def" "/implib:primary.lib" "/out:$outdir/xenora_loadlib.dll" library.obj
# Link controls under the same output leaf name as the provider, then copy them
# to their fixture names: the embedded export-module name remains the logical
# xenora_loadlib.dll rather than a host file name.
mkdir missing fail
# Same embedded logical module name, but the function export is absent.
link /dll /entry:DllMain /nodefaultlib /dynamicbase /timestamp:0 /machine:x64 \
    "/def:xenora_loadlib_missing.def" "/implib:missing.lib" "/out:missing/xenora_loadlib.dll" library-missing.obj
cp missing/xenora_loadlib.dll "$outdir/xenora_loadlib_missing.dll"
# Same embedded logical module name and export shape, but DllMain refuses attach.
link /dll /entry:DllMain /nodefaultlib /dynamicbase /timestamp:0 /machine:x64 \
    "/def:xenora_loadlib.def" "/implib:fail.lib" "/out:fail/xenora_loadlib.dll" library-fail.obj
cp fail/xenora_loadlib.dll "$outdir/xenora_loadlib_fail.dll"
link /entry:entry /subsystem:console /nodefaultlib /dynamicbase /timestamp:0 /machine:x64 \
    "/out:$outdir/loadlib-consumer.exe" consumer.obj kernel32.lib

chmod 644 "$outdir/xenora_loadlib.dll" "$outdir/xenora_loadlib_missing.dll" \
    "$outdir/xenora_loadlib_fail.dll" "$outdir/loadlib-consumer.exe"
