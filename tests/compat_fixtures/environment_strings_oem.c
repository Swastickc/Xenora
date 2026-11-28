// SPDX-License-Identifier: GPL-3.0-or-later

#include <stddef.h>

// Original freestanding guest. Signatures follow the Win64 process-environment
// and heap APIs. Keep the legacy undecorated import distinct from the A export.
#define IMPORT __declspec(dllimport)
IMPORT char *__stdcall GetEnvironmentStrings(void);
#if !API_SET
IMPORT char *__stdcall GetEnvironmentStringsA(void);
#endif
IMPORT wchar_t *__stdcall GetEnvironmentStringsW(void);
IMPORT int __stdcall FreeEnvironmentStringsA(char *);
IMPORT int __stdcall FreeEnvironmentStringsW(wchar_t *);
IMPORT int __stdcall SetEnvironmentVariableW(const wchar_t *, const wchar_t *);
IMPORT void *__stdcall GetProcessHeap(void);
IMPORT void *__stdcall HeapAlloc(void *, unsigned int, size_t);
IMPORT int __stdcall HeapFree(void *, unsigned int, void *);
IMPORT unsigned int __stdcall GetLastError(void);
IMPORT void __stdcall SetLastError(unsigned int);
IMPORT __declspec(noreturn) void __stdcall ExitProcess(unsigned int);

_Static_assert(sizeof(wchar_t) == 2, "Win64 WCHAR width");

#define CHECK(condition, code) do { if (!(condition)) ExitProcess(code); } while (0)
#define CHECK_REJECTED(call, code) do { \
    SetLastError(0); \
    CHECK(!(call) && GetLastError() == 87, code); \
} while (0)

static const char original[] = "alpha=\202\0Beta=\0Zulu=?\0";
static const char mutated[] = "alpha=new\0Beta=\0Zulu=?\0";
static const wchar_t wide_original[] = L"alpha=\u00e9\0Beta=\0Zulu=\u4e2d\0";
static const wchar_t wide_mutated[] = L"alpha=new\0Beta=\0Zulu=\u4e2d\0";

static char *get_second_narrow_copy(void)
{
#if API_SET
    return GetEnvironmentStrings();
#else
    return GetEnvironmentStringsA();
#endif
}

static int same_bytes(const void *ptr, const void *expected, size_t size)
{
    const unsigned char *left = ptr;
    const unsigned char *right = expected;
    for (size_t index = 0; index < size; ++index) {
        if (left[index] != right[index])
            return 0;
    }
    return 1;
}

void entry(void)
{
    const char empty[] = {0, 0};
    const wchar_t wide_empty[] = {0, 0};
    char *first = GetEnvironmentStrings();
    char *second = get_second_narrow_copy();
    wchar_t *wide = GetEnvironmentStringsW();
    CHECK(first && second && wide && first != second, 1);
    CHECK(same_bytes(first, empty, sizeof(empty)) && same_bytes(second, empty, sizeof(empty)), 2);
    CHECK(same_bytes(wide, wide_empty, sizeof(wide_empty)), 3);
    CHECK(FreeEnvironmentStringsA(first) && FreeEnvironmentStringsA(second) && FreeEnvironmentStringsW(wide), 4);

    // Insert in non-sorted order with mixed casing and an empty value.
    CHECK(SetEnvironmentVariableW(L"Zulu", L"\u4e2d"), 5);
    CHECK(SetEnvironmentVariableW(L"Beta", L""), 6);
    CHECK(SetEnvironmentVariableW(L"alpha", L"\u00e9"), 7);
    first = GetEnvironmentStrings();
    second = get_second_narrow_copy();
    wide = GetEnvironmentStringsW();
    CHECK(first && second && wide && first != second && (void *)first != wide, 8);
    CHECK(same_bytes(first, original, sizeof(original)) && same_bytes(second, original, sizeof(original)), 9);
    CHECK(same_bytes(wide, wide_original, sizeof(wide_original)), 10);

    // Updates affect new snapshots only and retain the original variable name.
    CHECK(SetEnvironmentVariableW(L"ALPHA", L"new"), 11);
    char *updated = get_second_narrow_copy();
    wchar_t *wide_updated = GetEnvironmentStringsW();
    CHECK(updated && updated != first && updated != second && wide_updated && wide_updated != wide, 12);
    CHECK(same_bytes(updated, mutated, sizeof(mutated)), 13);
    CHECK(same_bytes(wide_updated, wide_mutated, sizeof(wide_mutated)), 14);
    CHECK(same_bytes(first, original, sizeof(original)) && same_bytes(second, original, sizeof(original)), 15);
    CHECK(same_bytes(wide, wide_original, sizeof(wide_original)), 16);

    CHECK_REJECTED(FreeEnvironmentStringsA((char *)wide), 17);
    CHECK_REJECTED(FreeEnvironmentStringsW((wchar_t *)first), 18);
    CHECK_REJECTED(FreeEnvironmentStringsA(first + 1), 19);
    CHECK_REJECTED(FreeEnvironmentStringsA(NULL), 20);

    void *heap = GetProcessHeap();
    unsigned char *unrelated = HeapAlloc(heap, 0, 16);
    CHECK(heap && unrelated, 21);
    unrelated[0] = 0x5a;
    CHECK_REJECTED(FreeEnvironmentStringsA((char *)unrelated), 22);
    CHECK(unrelated[0] == 0x5a && HeapFree(heap, 0, unrelated), 23);

    CHECK(same_bytes(first, original, sizeof(original)) && same_bytes(wide, wide_original, sizeof(wide_original)), 24);
    CHECK(FreeEnvironmentStringsA(first), 25);
    CHECK_REJECTED(FreeEnvironmentStringsA(first), 26);
    CHECK(FreeEnvironmentStringsA(second) && FreeEnvironmentStringsA(updated), 27);
    CHECK(FreeEnvironmentStringsW(wide) && FreeEnvironmentStringsW(wide_updated), 28);
    ExitProcess(0);
}
