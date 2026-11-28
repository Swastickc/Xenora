#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Build freestanding PE32+ guests with native ARM64 tools.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
mkdir -p "$1"
out=$(CDPATH= cd -- "$1" && pwd)

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

scratch=$(mktemp -d "${TMPDIR:-/tmp}/xenora-environment-strings.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
cd "$scratch"

cat > kernel32-base.def <<'DEF'
LIBRARY kernel32.dll
EXPORTS
    GetEnvironmentStringsW
    FreeEnvironmentStringsW
    SetEnvironmentVariableW
    GetProcessHeap
    HeapAlloc
    HeapFree
    GetLastError
    SetLastError
    ExitProcess
DEF
{
    cat kernel32-base.def
    printf '    %s\n' GetEnvironmentStrings GetEnvironmentStringsA FreeEnvironmentStringsA
} > kernel32-direct.def
cat > processenvironment.def <<'DEF'
LIBRARY api-ms-win-core-processenvironment-l1-1-0.dll
EXPORTS
    GetEnvironmentStrings
    FreeEnvironmentStringsA
DEF
for module in kernel32-base kernel32-direct processenvironment; do
    link /lib "/def:$module.def" /machine:x64 "/out:$module.lib"
done

build() {
    api_set=$1
    name=$2
    shift 2
    arch -arm64 "$clang" --target=x86_64-pc-windows-msvc -std=c11 -O1 -Wall -Wextra -Werror \
        -ffreestanding -fno-builtin -fno-stack-protector -fno-vectorize -fno-slp-vectorize \
        -DAPI_SET="$api_set" -c "$here/environment_strings_oem.c" -o guest.obj
    link /entry:entry /subsystem:console /nodefaultlib /timestamp:0 /machine:x64 \
        "/out:$out/environment-strings-$name.exe" guest.obj "$@"
}

build 0 kernel32 kernel32-direct.lib
build 1 apiset kernel32-base.lib processenvironment.lib
