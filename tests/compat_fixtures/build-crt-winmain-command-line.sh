#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Build a freestanding PE32+ fixture with native MinGW-w64 tools on PATH.
set -eu
cd "$(dirname "$0")/../.."
out="$1"
mkdir -p "$out"

cat > "$out/crt-runtime.def" <<'EOF'
LIBRARY api-ms-win-crt-runtime-l1-1-0.dll
EXPORTS
    _get_narrow_winmain_command_line
EOF

x86_64-w64-mingw32-dlltool -m i386:x86-64 \
    -d "$out/crt-runtime.def" -l "$out/libcrt-runtime.a"

x86_64-w64-mingw32-gcc -std=c11 -O1 -Wall -Wextra -Werror \
    -fno-builtin -ffreestanding -fno-stack-protector -nostdlib \
    -Wl,--entry,entry -Wl,--subsystem,console \
    -o "$out/crt WinMain command line.exe" \
    tests/compat_fixtures/crt_winmain_command_line.c "$out/libcrt-runtime.a" -lkernel32
