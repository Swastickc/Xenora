// SPDX-License-Identifier: GPL-3.0-or-later
// Original Win64 PE fixture for the legacy CRT's per-thread random sequence.

typedef unsigned long long uintptr;
typedef unsigned int dword;
typedef void *handle;
typedef dword (*thread_start)(void *);

__declspec(dllimport) int rand(void);
__declspec(dllimport) void srand(unsigned int);
__declspec(dllimport) handle CreateEventW(void *, int, int, const unsigned short *);
__declspec(dllimport) int SetEvent(handle);
__declspec(dllimport) handle CreateThread(void *, uintptr, thread_start, void *, dword, dword *);
__declspec(dllimport) dword WaitForSingleObject(handle, dword);
__declspec(dllimport) int GetExitCodeThread(handle, dword *);
__declspec(dllimport) int CloseHandle(handle);
__declspec(dllimport) __declspec(noreturn) void ExitProcess(dword);

enum { WAIT_MS = 2000 };

struct worker_state {
    handle ready;
    handle resume;
    volatile uintptr first_teb;
    volatile uintptr second_teb;
    volatile int initial;
    volatile int seeded;
    volatile int replacement_initial;
};

static uintptr current_teb(void) {
    uintptr teb;
    __asm__ volatile("movq %%gs:0x30, %0" : "=r"(teb));
    return teb;
}

static dword first_worker(void *context) {
    struct worker_state *state = context;
    state->first_teb = current_teb();
    state->initial = rand();
    srand(2);
    state->seeded = rand();
    if (!SetEvent(state->ready))
        return 1;
    if (WaitForSingleObject(state->resume, WAIT_MS) != 0)
        return 2;
    return rand() == 29216 ? 0 : 3;
}

static dword second_worker(void *context) {
    struct worker_state *state = context;
    state->second_teb = current_teb();
    state->replacement_initial = rand();
    return 0;
}

static int join_and_close(handle thread) {
    dword code = 0;
    if (WaitForSingleObject(thread, WAIT_MS) != 0)
        return 0;
    if (!GetExitCodeThread(thread, &code) || code != 0)
        return 0;
    return CloseHandle(thread) != 0;
}

void entry(void) {
    if (rand() != 41 || rand() != 18467)
        ExitProcess(1);
    srand(1);
    if (rand() != 41 || rand() != 18467)
        ExitProcess(2);

    struct worker_state data;
    data.ready = CreateEventW(0, 0, 0, 0);
    data.resume = CreateEventW(0, 0, 0, 0);
    if (!data.ready || !data.resume)
        ExitProcess(3);

    dword first_id = 0, second_id = 0;
    handle first = CreateThread(0, 0, first_worker, &data, 0, &first_id);
    if (!first || WaitForSingleObject(data.ready, WAIT_MS) != 0)
        ExitProcess(4);
    if (data.initial != 41)
        ExitProcess(5);
    if (data.seeded != 45 || rand() != 6334)
        ExitProcess(6);
    if (!SetEvent(data.resume) || !join_and_close(first))
        ExitProcess(7);

    handle second = CreateThread(0, 0, second_worker, &data, 0, &second_id);
    if (!second || !join_and_close(second))
        ExitProcess(8);
    if (!first_id || !second_id || first_id == second_id || !data.first_teb
        || data.first_teb == current_teb() || data.second_teb != data.first_teb)
        ExitProcess(9);
    if (data.replacement_initial != 41 || rand() != 26500)
        ExitProcess(10);

    if (!CloseHandle(data.resume) || !CloseHandle(data.ready))
        ExitProcess(11);
    ExitProcess(0);
}
