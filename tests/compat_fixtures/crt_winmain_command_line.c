// SPDX-License-Identifier: GPL-3.0-or-later

#include <corecrt_startup.h>
#include <windows.h>

static const char program[] = "\"crt WinMain command line.exe\"";
static const char expected_tail[] = "\"two  words\" \"say\\\"hello\" \"\" \"a\tb\" --trace";

static int same_string(const char *left, const char *right)
{
    while (*left == *right) {
        if (*left == '\0') {
            return 1;
        }
        ++left;
        ++right;
    }
    return 0;
}

void entry(void)
{
    const char *tail = _get_narrow_winmain_command_line();
    const char *full = GetCommandLineA();
    if (tail == NULL || full == NULL || GetCommandLineA() != full ||
        _get_narrow_winmain_command_line() != tail) {
        ExitProcess(11);
    }

    for (unsigned int index = 0; index < sizeof(program) - 1; ++index) {
        if (full[index] != program[index]) {
            ExitProcess(12);
        }
    }

    const char *after_program = full + sizeof(program) - 1;
    if (*after_program == '\0') {
        if (tail != after_program || *tail != '\0') {
            ExitProcess(13);
        }
        ExitProcess(41);
    }

    if (*after_program != ' ' || tail != after_program + 1 ||
        !same_string(tail, expected_tail)) {
        ExitProcess(14);
    }
    ExitProcess(42);
}
