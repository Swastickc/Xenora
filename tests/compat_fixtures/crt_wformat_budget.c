// SPDX-License-Identifier: GPL-3.0-or-later

// Original freestanding PE guest for bounded wide printf buffer sizing.
typedef unsigned short u16;
typedef unsigned long long u64;

#define IMPORT __declspec(dllimport)
#define OUTPUT_CAP (1024 * 1024)
#define STANDARD_SNPRINTF_BEHAVIOR 2

IMPORT int __stdio_common_vswprintf(u64 options, u16 *buffer, u64 count,
                                    const u16 *format, void *locale, __builtin_ms_va_list args);
IMPORT void *memset(void *buffer, int value, u64 bytes);
IMPORT void *VirtualAlloc(void *address, u64 size, unsigned int type, unsigned int protection);
IMPORT int VirtualFree(void *address, u64 size, unsigned int type);
IMPORT __declspec(noreturn) void ExitProcess(unsigned int status);

static u16 wide[OUTPUT_CAP + 2];
static char narrow[OUTPUT_CAP + 2];

static int format(u16 *out, u64 count, const u16 *spec, ...) {
    __builtin_ms_va_list args;
    __builtin_ms_va_start(args, spec);
    int result = __stdio_common_vswprintf(STANDARD_SNPRINTF_BEHAVIOR, out, count, spec, 0, args);
    __builtin_ms_va_end(args);
    return result;
}

void entry(void) {
    // The unrelated memset service bounds one call to one MiB of bytes.
    memset(wide, 0x41, OUTPUT_CAP);
    memset(wide + OUTPUT_CAP / 2, 0x41, OUTPUT_CAP);
    wide[OUTPUT_CAP] = 0x4141;
    memset(narrow, 'b', OUTPUT_CAP);
    narrow[OUTPUT_CAP] = 'b';

#if defined(OVER_WIDE)
    format(0, 0, L"%s", wide);
    ExitProcess(77); // The oversized unqualified conversion must stop the guest.
#elif defined(OVER_NARROW)
    format(0, 0, L"%S", narrow);
    ExitProcess(78);
#else
    wide[OUTPUT_CAP] = 0;
    narrow[OUTPUT_CAP] = 0;
    if (format(0, 0, L"%s", wide) != OUTPUT_CAP)
        ExitProcess(1);
    if (format(0, 0, L"%S", narrow) != OUTPUT_CAP)
        ExitProcess(2);

    // The terminator is now beyond the precision; neither form may insist
    // on reading it, or report the unqualified count.
    wide[OUTPUT_CAP] = 0x4141;
    narrow[OUTPUT_CAP] = 'b';
    if (format(0, 0, L"%.1048576s", wide) != OUTPUT_CAP)
        ExitProcess(3);
    if (format(0, 0, L"%.1048576S", narrow) != OUTPUT_CAP)
        ExitProcess(4);

    // Three readable source units at the end of a page, with no readable
    // terminator after them. Verify precision also limits the memory reads.
    unsigned char *page = VirtualAlloc(0, 4096, 0x3000, 4);
    if (!page)
        ExitProcess(5);
    u16 *wide_edge = (u16 *)(page + 4096 - 3 * sizeof(u16));
    wide_edge[0] = wide_edge[1] = wide_edge[2] = 0x4141;

    u16 out[5];
    out[4] = 0x1234;
    if (format(out, 4, L"%.3s", wide_edge) != 3 || out[0] != 0x4141 ||
        out[1] != 0x4141 || out[2] != 0x4141 || out[3] != 0 || out[4] != 0x1234)
        ExitProcess(6);
    char *narrow_edge = (char *)(page + 4096 - 3);
    narrow_edge[0] = narrow_edge[1] = narrow_edge[2] = 'b';
    if (format(out, 4, L"%.3S", narrow_edge) != 3 || out[0] != 'b' ||
        out[1] != 'b' || out[2] != 'b' || out[3] != 0 || out[4] != 0x1234)
        ExitProcess(7);
    if (!VirtualFree(page, 0, 0x8000))
        ExitProcess(8);
    ExitProcess(0);
#endif
}
