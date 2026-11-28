// SPDX-License-Identifier: GPL-3.0-or-later
// Freestanding PE32+ guest for a linker-declared main stack reservation.

typedef unsigned long long uintptr;
__declspec(dllimport) void GetCurrentThreadStackLimits(uintptr *, uintptr *);
__declspec(dllimport) __declspec(noreturn) void ExitProcess(unsigned int);

enum { PAGE_BYTES = 4096, FRAME_BYTES = 96 * 1024 };

void entry(void) {
    __declspec(align(16)) volatile unsigned char frame[FRAME_BYTES];
    for (unsigned int i = 0; i < FRAME_BYTES; i += PAGE_BYTES)
        frame[i] = (unsigned char)(i / PAGE_BYTES + 1);
    frame[FRAME_BYTES - 1] = 0xa5;

    uintptr low = 0, high = 0;
    GetCurrentThreadStackLimits(&low, &high);
    uintptr base = (uintptr)&frame[0];
    uintptr stack_pointer;
    __asm__ volatile("movq %%rsp, %0" : "=r"(stack_pointer));
    if ((stack_pointer & 15) != 0 || base < low || base + sizeof(frame) > high)
        ExitProcess(1);
    for (unsigned int i = 0; i < FRAME_BYTES; i += PAGE_BYTES)
        if (frame[i] != (unsigned char)(i / PAGE_BYTES + 1))
            ExitProcess(2);
    ExitProcess(frame[FRAME_BYTES - 1] == 0xa5 ? 0 : 3);
}

void small_entry(void) {
    uintptr low = 0, high = 0;
    GetCurrentThreadStackLimits(&low, &high);
    uintptr local = (uintptr)&low;
    ExitProcess(low <= local && local < high ? 0 : 1);
}
