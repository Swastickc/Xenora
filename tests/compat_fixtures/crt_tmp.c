// SPDX-License-Identifier: GPL-3.0-or-later
//
// A hosted C program for the last part of #493: tmpnam and tmpfile. The
// expectations are the Microsoft pages for the two functions. It prints
// how many checks passed and returns the number of the first that failed,
// or 0.
#include <stdio.h>

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

static int differs(const char *a, const char *b) {
    while (*a && *a == *b) a++, b++;
    return *a != *b;
}

int main(void) {
    char one[L_tmpnam], two[L_tmpnam], line[32];

    // tmpnam names differ from each other and hold no file.
    check(tmpnam(one) == one && one[0], "tmpnam writes the buffer");
    check(tmpnam(two) == two && two[0], "second name");
    check(differs(one, two), "names differ");
    check(fopen(one, "rb") == NULL, "first name holds no file");
    check(fopen(two, "rb") == NULL, "second name holds no file");
    check(tmpnam(NULL) != NULL, "null uses the static buffer");

    // tmpfile reads and writes in binary, and closing deletes the file.
    FILE *f = tmpfile();
    check(f != NULL, "tmpfile");
    check(fwrite("temp\n", 1, 5, f) == 5, "write");
    check(fgets(line, sizeof line, f) == NULL, "at the end after writing");
    fclose(f);

#ifdef CRT_TMP_BREAK
    // Control: a wrong expectation must be detected.
    check(1 == 2, "control");
#endif
    printf("crt_tmp: %u of %u checks passed\n", passed, total);
    return (int)first;
}
