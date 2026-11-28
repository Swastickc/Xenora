// SPDX-License-Identifier: GPL-3.0-or-later

#include <stdlib.h>
#include <windows.h>

#ifndef NO_RETURN
#define NO_RETURN 0
#endif

static volatile unsigned int calls;

static void __cdecl record_handler(const wchar_t *expression,
                                   const wchar_t *function,
                                   const wchar_t *file,
                                   unsigned int line,
                                   uintptr_t reserved)
{
    if (expression != NULL || function != NULL || file != NULL ||
        line != 0 || reserved != 0) {
        ExitProcess(11);
    }
    if (++calls != 1) {
        ExitProcess(12);
    }
}

void entry(void)
{
    if (_set_invalid_parameter_handler(record_handler) != NULL) {
        ExitProcess(13);
    }

    _invalid_parameter_noinfo();
    if (calls != 1) {
        ExitProcess(14);
    }

#if NO_RETURN
    calls = 0;
    _invalid_parameter_noinfo_noreturn();
#else
    ExitProcess(0);
#endif
}
