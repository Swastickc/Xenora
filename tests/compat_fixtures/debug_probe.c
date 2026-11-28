// SPDX-License-Identifier: GPL-3.0-or-later
// Freestanding Win64 guest for the exact debug API-set import boundary.

__declspec(dllimport) int IsDebuggerPresent(void);
__declspec(dllimport) void OutputDebugStringA(const char *);
__declspec(dllimport) void OutputDebugStringW(const unsigned short *);
__declspec(dllimport) void *VirtualAlloc(void *, unsigned long long, unsigned int, unsigned int);
__declspec(dllimport) int VirtualFree(void *, unsigned long long, unsigned int);
__declspec(dllimport) __declspec(noreturn) void ExitProcess(unsigned int);

enum {
    PAGE_BYTES = 4096,
    MEM_COMMIT_RESERVE = 0x3000,
    MEM_RELEASE = 0x8000,
    PAGE_READWRITE = 4,
};

void entry(void) {
    if (IsDebuggerPresent())
        ExitProcess(1);

    char *page = VirtualAlloc(0, PAGE_BYTES, MEM_COMMIT_RESERVE, PAGE_READWRITE);
    if (!page)
        ExitProcess(2);

    char *narrow = page + PAGE_BYTES - 2;
    narrow[0] = 'A';
    narrow[1] = 0;
    OutputDebugStringA(narrow);
    OutputDebugStringA(narrow + 1);

    unsigned short *wide = (unsigned short *)(page + PAGE_BYTES - 4);
    wide[0] = 0x03a9;
    wide[1] = 0;
    OutputDebugStringW(wide);
    OutputDebugStringW(wide + 1);

    OutputDebugStringA(0);
    OutputDebugStringW(0);
    if (!VirtualFree(page, 0, MEM_RELEASE))
        ExitProcess(3);
    ExitProcess(0);
}
