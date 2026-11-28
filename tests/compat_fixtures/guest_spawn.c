// SPDX-License-Identifier: GPL-3.0-or-later
//
// A hosted guest that re-executes itself through `_spawnve` in _P_WAIT mode
// (issue #1040). The parent writes a file on the shared drive, spawns the
// child with "child" as its first argument and reports the status the spawn
// returns. The child reads the parent's file - proving the machine-wide
// filesystem is shared - and exits 42.
//
// Every expected output line is stated next to the printf that produces it.

#include <stdio.h>
#include <string.h>
#include <process.h>

int main(int argc, char **argv) {
    if (argc >= 2 && strcmp(argv[1], "child") == 0) {
        FILE *in = fopen("spawn-shared.txt", "r");
        if (in == NULL) {
            printf("guest spawn: child found no shared file\n");
            return 1;
        }
        char buffer[64] = {0};
        const char *read = fgets(buffer, sizeof buffer, in);
        fclose(in);
        if (read == NULL || strcmp(buffer, "parent data") != 0) {
            printf("guest spawn: child read the wrong bytes\n");
            return 1;
        }
        /* expected: "guest spawn: child ran\n" */
        printf("guest spawn: child ran\n");
        return 42;
    }

    FILE *out = fopen("spawn-shared.txt", "w");
    if (out == NULL) {
        printf("guest spawn: parent could not write\n");
        return 1;
    }
    fputs("parent data", out);
    fclose(out);

    const char *child_argv[3];
    child_argv[0] = argv[0];
    child_argv[1] = "child";
    child_argv[2] = NULL;
    intptr_t status = _spawnve(_P_WAIT, argv[0], child_argv, NULL);
    /* expected: "guest spawn: child status 42\n" */
    printf("guest spawn: child status %d\n", (int)status);
    return status == 42 ? 0 : 1;
}
