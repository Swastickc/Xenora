#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Build the original XInput polling guest (Campaign F, #942) with native ARM64
# clang and lld. The output is reproducible: /timestamp:0 and no default libs.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
out=${1:-"$here/xinput-poll.exe"}
case "$out" in
    /*) ;;
    *) out="$PWD/$out" ;;
esac
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
    case "$(lipo -archs "$tool")" in
        *arm64*) ;;
        *) echo "Expected a native ARM64 tool: $tool" >&2; exit 1 ;;
    esac
done
link() {
    case "${linker##*/}" in
        rust-lld) "$linker" -flavor link "$@" ;;
        *) "$linker" "$@" ;;
    esac
}
temp=$(mktemp -d "${TMPDIR:-/tmp}/xenora-xinput-poll.XXXXXX")
trap 'rm -rf "$temp"' EXIT
cd "$temp"
cat > kernel32.def <<'DEF'
LIBRARY kernel32.dll
EXPORTS
    GetStdHandle
    WriteConsoleA
    ExitProcess
DEF
cat > xinput1_4.def <<'DEF'
LIBRARY xinput1_4.dll
EXPORTS
    XInputGetState
DEF
cat > xenora_session.def <<'DEF'
LIBRARY xenora_session.dll
EXPORTS
    XenoraNextEvent
DEF
for module in kernel32 xinput1_4 xenora_session; do
    link /lib "/def:$module.def" /machine:x64 "/out:$module.lib"
done
"$clang" --target=x86_64-pc-windows-msvc -O1 -Wall -Wextra -Werror \
    -ffreestanding -fno-builtin -fno-stack-protector -fno-vectorize -fno-slp-vectorize \
    -c "$here/xinput-poll.c" -o guest.obj
link /entry:entry /subsystem:console /nodefaultlib /timestamp:0 /machine:x64 \
    "/out:$out" guest.obj kernel32.lib xinput1_4.lib xenora_session.lib
