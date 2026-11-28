// SPDX-License-Identifier: GPL-3.0-or-later
//
// A hosted C program for package 6 of #459: the wide conversions and
// classification. The expectations are the C locale. Prints how many
// checks passed and returns the number of the first failure, or 0.
#include <wchar.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>

static unsigned passed, total, first;

static void check(int ok, const char *what) {
    total++;
    if (ok) {
        passed++;
    } else {
        if (!first) first = total;
        printf("check %u failed: %s\n", total, what);
    }
}

static int wide_equal(const wchar_t *a, const wchar_t *b) {
    while (*a && *a == *b) a++, b++;
    return *a == *b;
}

int main(void) {
    wchar_t wide[16];
    char text[16];

    // Every byte is the wide character of the same value.
    check(mbstowcs(wide, "abc", 8) == 3, "mbstowcs count");
    check(wide[0] == L'a' && wide[2] == L'c' && wide[3] == 0, "zero extended");
    check(mbstowcs(NULL, "abc", 0) == 3, "length query");
    check(mbstowcs(wide, "abcdef", 3) == 3, "limited");

    // The inverse fails above 255.
    wide[0] = L'a'; wide[1] = L'b'; wide[2] = L'c'; wide[3] = 0;
    check(wcstombs(text, wide, 8) == 3 && strcmp(text, "abc") == 0, "wcstombs");
    wide[0] = 0x100;
    check(wcstombs(text, wide, 8) == (size_t)-1, "EILSEQ above 255");

    // The single characters.
    check(mbtowc(wide, "z", 1) == 1 && wide[0] == L'z', "mbtowc");
    check(wctomb(text, L'q') == 1 && text[0] == 'q', "wctomb");
    check(wctomb(text, 0x100) == -1, "wctomb fails above 255");

    // The restartable forms advance their pointer.
    const char *scan = "xy";
    check(mbsrtowcs(wide, &scan, 1, NULL) == 1 && scan[0] == 'y', "mbsrtowcs advances");
    check(mbsrtowcs(wide, &scan, 4, NULL) == 1 && scan == NULL, "mbsrtowcs consumes");

    // Exact capacity leaves the terminator unconverted; a separate call
    // copies it and clears the source. Sizing queries make no progress.
    scan = "xy";
    check(mbsrtowcs(wide, &scan, 2, NULL) == 2 && scan != NULL && scan[0] == 0,
          "mbsrtowcs exact capacity");
    check(mbsrtowcs(wide, &scan, 1, NULL) == 0 && scan == NULL && wide[0] == 0,
          "mbsrtowcs separate terminator");
    scan = "abc";
    check(mbsrtowcs(NULL, &scan, 0, NULL) == 3 && scan != NULL && scan[0] == 'a',
          "mbsrtowcs sizing query");

    const wchar_t *wscan = L"xy";
    check(wcsrtombs(text, &wscan, 2, NULL) == 2 && wscan != NULL && wscan[0] == 0,
          "wcsrtombs exact capacity");
    check(wcsrtombs(text, &wscan, 1, NULL) == 0 && wscan == NULL && text[0] == 0,
          "wcsrtombs separate terminator");
    wscan = L"abc";
    check(wcsrtombs(NULL, &wscan, 0, NULL) == 3 && wscan != NULL && wscan[0] == L'a',
          "wcsrtombs sizing query");

    // An unrepresentable wide unit is left as the next input unit.
    wchar_t bad[] = { L'a', 0x100, L'b', 0 };
    wscan = bad;
    check(wcsrtombs(text, &wscan, 8, NULL) == (size_t)-1 && wscan == bad + 1 && text[0] == 'a',
          "wcsrtombs error progress");

    // Converting a NUL byte stores a zero wide character in a real destination.
    wide[0] = 0xffff;
    check(mbtowc(wide, "", 1) == 0 && wide[0] == 0, "mbtowc stores NUL");

    // The classification is ASCII's.
    check(iswalpha(L'a') && !iswalpha(L'5'), "iswalpha");
    check(!iswalpha(0xE9), "no class above ASCII");
    check(iswspace(L' ') && iswdigit(L'5') && iswupper(L'A'), "classes");
    check(towupper(L'a') == L'A' && towlower(L'A') == L'a', "case conversion");

    // The string forms.
    wchar_t joined[16] = L"a";
    wcsncat(joined, L"bcd", 2);
    check(wide_equal(joined, L"abc"), "wcsncat counts");
    wchar_t *copy = _wcsdup(L"copy");
    check(copy != NULL && wide_equal(copy, L"copy"), "_wcsdup");
    free(copy);

#ifdef CRT_WIDE_BREAK
    check(1 == 2, "control");
#endif
    printf("crt_wide: %u of %u checks passed\n", passed, total);
    return (int)first;
}
