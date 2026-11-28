#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Build two freestanding PE32+ fixtures with native MinGW-w64 tools on PATH.
set -eu
cd "$(dirname "$0")/../.."
out="$1"
mkdir -p "$out"

cat > "$out/crt-runtime.def" <<'EOF'
LIBRARY api-ms-win-crt-runtime-l1-1-0.dll
EXPORTS
    _invalid_parameter_noinfo
    _invalid_parameter_noinfo_noreturn
    _set_invalid_parameter_handler
EOF

x86_64-w64-mingw32-dlltool -m i386:x86-64 \
    -d "$out/crt-runtime.def" -l "$out/libcrt-runtime.a"

build() {
    x86_64-w64-mingw32-gcc -std=c11 -O1 -Wall -Wextra -Werror \
        -fno-builtin -ffreestanding -fno-stack-protector -nostdlib \
        -Wl,--entry,entry -Wl,--subsystem,console -DNO_RETURN="$1" \
        -o "$out/crt-invalid-parameter-$2.exe" \
        tests/compat_fixtures/crt_invalid_parameter.c "$out/libcrt-runtime.a" -lkernel32
}

build 0 return
build 1 noreturn
