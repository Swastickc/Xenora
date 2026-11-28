#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
out=${1:-"$here/../../artifacts/address-wait"}
mkdir -p "$out"
out=$(CDPATH= cd -- "$out" && pwd)

clang=${CLANG:-$(xcrun --find clang)}
linker=${LLD_LINK:-$(rustc --print sysroot)/lib/rustlib/aarch64-apple-darwin/bin/rust-lld}
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

temp=$(mktemp -d "${TMPDIR:-/tmp}/xenora-address-wait.XXXXXX")
trap 'rm -rf "$temp"' EXIT
cd "$temp"
cat > kernel32.def <<'DEF'
LIBRARY kernel32.dll
EXPORTS
    CreateEventW
    SetEvent
    CreateThread
    WaitForSingleObject
    GetExitCodeThread
    CloseHandle
    GetLastError
    GetTickCount64
    SuspendThread
    ResumeThread
    Sleep
    ExitProcess
DEF
cat > address_wait.def <<'DEF'
LIBRARY api-ms-win-core-synch-l1-2-0.dll
EXPORTS
    WaitOnAddress
    WakeByAddressSingle
    WakeByAddressAll
DEF
for module in kernel32 address_wait; do
    link /lib "/def:$module.def" /machine:x64 "/out:$module.lib"
done
"$clang" --target=x86_64-pc-windows-msvc -O1 -Wall -Wextra -Werror \
    -ffreestanding -fno-builtin -fno-stack-protector -fno-vectorize -fno-slp-vectorize \
    -c "$here/address_wait.c" -o guest.obj
link /entry:entry /subsystem:console /nodefaultlib /timestamp:0 /machine:x64 \
    "/out:$out/address-wait.exe" guest.obj kernel32.lib address_wait.lib
link /entry:bad_compare_entry /subsystem:console /nodefaultlib /timestamp:0 /machine:x64 \
    "/out:$out/address-wait-bad-compare.exe" guest.obj kernel32.lib address_wait.lib
link /entry:unscheduled_entry /subsystem:console /nodefaultlib /timestamp:0 /machine:x64 \
    "/out:$out/address-wait-unscheduled.exe" guest.obj kernel32.lib address_wait.lib
