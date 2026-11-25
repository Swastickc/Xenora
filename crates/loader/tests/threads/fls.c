// SPDX-License-Identifier: GPL-3.0-or-later

// Original guest workloads for Win64 worker-thread FLS cleanup.
typedef unsigned long long u64;
typedef unsigned int u32;
typedef void *HANDLE;

#define IMPORT __declspec(dllimport)
IMPORT HANDLE CreateThread(void *, u64, u32 (*)(void *), void *, u32, u32 *);
IMPORT void ExitThread(u32);
IMPORT int TerminateThread(HANDLE, u32);
IMPORT int GetExitCodeThread(HANDLE, u32 *);
IMPORT u32 GetCurrentThreadId(void);
IMPORT u32 GetThreadId(HANDLE);
IMPORT u32 WaitForSingleObject(HANDLE, u32);
IMPORT HANDLE CreateEventA(void *, int, int, const char *);
IMPORT int SetEvent(HANDLE);
IMPORT int CloseHandle(HANDLE);
IMPORT void Sleep(u32);
IMPORT u32 FlsAlloc(void (*)(void *));
IMPORT int FlsFree(u32);
IMPORT int FlsSetValue(u32, void *);

#define INFINITE 0xFFFFFFFFu
#define THREAD(n) ((void *)(u64)(n))
struct shared {
    volatile u64 out[32];
    volatile u64 calls;
    u32 indices[3];
    HANDLE ready;
};
#define S ((struct shared *)0x50000)

static void fls_first(void *value) {
    Sleep(1);
    S->out[3] = (u64)value;
    S->out[4] = GetCurrentThreadId();
    S->calls++;
}

static void fls_second(void *value) {
    S->out[5] = (u64)value;
    S->out[6] = GetCurrentThreadId();
    S->calls++;
}

static u32 fls_worker(void *mode) {
    if (!FlsSetValue(S->indices[0], THREAD(0x12345)) ||
        !FlsSetValue(S->indices[1], THREAD(0x67890))) return 1;
    if (mode == THREAD(1)) {
        ExitThread(0xBEEF);
        return 2;
    }
    if (mode == THREAD(2)) {
        SetEvent(S->ready);
        Sleep(INFINITE);
        return 3;
    }
    return 0xCAFE;
}

static void fls_exit(u64 mode) {
    S->indices[0] = FlsAlloc(fls_first);
    S->indices[1] = FlsAlloc(fls_second);
    S->indices[2] = FlsAlloc(fls_second); // unset slot has no cleanup
    S->ready = CreateEventA(0, 0, 0, 0);
    HANDLE worker = CreateThread(0, 0, fls_worker, THREAD(mode), 0, 0);
    S->out[0] = GetThreadId(worker);
    if (mode == 2) {
        S->out[8] = WaitForSingleObject(S->ready, INFINITE);
        S->out[9] = TerminateThread(worker, 0xDEAD);
    }
    S->out[1] = WaitForSingleObject(worker, INFINITE);
    u32 code = 0;
    GetExitCodeThread(worker, &code);
    S->out[2] = code;
    S->out[7] = S->calls;
    CloseHandle(worker);
    CloseHandle(S->ready);
    for (int i = 0; i < 3; i++) S->out[10 + i] = FlsFree(S->indices[i]);
    S->out[13] = S->calls;
}

void main_fls_return(void) { fls_exit(0); }
void main_fls_explicit(void) { fls_exit(1); }
void main_fls_terminate(void) { fls_exit(2); }
