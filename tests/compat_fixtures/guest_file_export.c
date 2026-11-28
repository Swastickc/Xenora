// SPDX-License-Identifier: GPL-3.0-or-later

#include <stdint.h>
#include <stdio.h>
#include <string.h>

static const unsigned char payload[] = {0, 0x41, 0x0a, 0x0d, 0, 0xff, 0x80};

int main(int argc, char **argv) {
    FILE *binary = fopen("C:\\guest-export.bin", "wb");
    if (!binary)
        return 50;
    size_t written = fwrite(payload, 1, sizeof payload, binary);
    if (fclose(binary) != 0 || written != sizeof payload)
        return 51;

    FILE *empty = fopen("C:\\empty-export.bin", "wb");
    if (!empty || fclose(empty) != 0)
        return 52;
    if (fputs("guest files ready\n", stdout) < 0)
        return 53;

    if (argc == 2 && strcmp(argv[1], "fail") == 0)
        return 0x100;
    if (argc == 2 && strcmp(argv[1], "spin") == 0) {
        volatile uint32_t counter = 0;
        for (;;) {
            ++counter;
            (void)counter;
        }
    }
    return argc == 1 ? 0 : 54;
}
