#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Build the original freestanding Win64 guest using native ARM64 tools.
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

scratch=$(mktemp -d "${TMPDIR:-/tmp}/xenora-thread-messages.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
cd "$scratch"

cat > kernel32.def <<'DEF'
LIBRARY kernel32.dll
EXPORTS
    CreateEventW
    CreateThread
    SetEvent
    WaitForSingleObject
    CloseHandle
    GetCurrentThreadId
    GetLastError
    SetLastError
    Sleep
    ExitProcess
DEF
cat > user32.def <<'DEF'
LIBRARY user32.dll
EXPORTS
    PeekMessageA
    PeekMessageW
    PostThreadMessageA
    PostThreadMessageW
DEF
cat > evidence.def <<'DEF'
LIBRARY xenora_test.dll
EXPORTS
    OpenHandleCount
DEF
for module in kernel32 user32 evidence; do
    link /lib "/def:$module.def" /machine:x64 "/out:$module.lib"
done

arch -arm64 "$clang" --target=x86_64-pc-windows-msvc -std=c11 -O1 -Wall -Wextra -Werror \
    -ffreestanding -fno-builtin -fno-stack-protector -fno-vectorize -fno-slp-vectorize \
    -c "$here/user32_thread_messages.c" -o guest.obj
link /entry:entry /subsystem:console /nodefaultlib /timestamp:0 /machine:x64 \
    "/out:$out/user32-thread-messages.exe" guest.obj kernel32.lib user32.lib evidence.lib
