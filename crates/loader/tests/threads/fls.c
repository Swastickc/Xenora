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
enum exit_mode {
    RETURN,
    EXPLICIT,
    TERMINATE_WHILE_RUNNING,
    EXIT_IN_CALLBACK,
    TERMINATE_IN_CALLBACK,
    TEARDOWN_IN_CALLBACK,
};

struct shared {
    volatile u64 out[32];
    volatile u64 calls;
    u32 indices[3];
    enum exit_mode mode;
    HANDLE ready;
};
#define S ((struct shared *)0x50000)

static void fls_first(void *value) {
    if (S->mode == TERMINATE_IN_CALLBACK || S->mode == TEARDOWN_IN_CALLBACK) {
        S->out[14] = (u64)value;
        SetEvent(S->ready);
        Sleep(INFINITE);
    }
    Sleep(1);
    S->out[3] = (u64)value;
    S->out[4] = GetCurrentThreadId();
    S->calls++;
    if (S->mode == EXIT_IN_CALLBACK) {
        S->out[14] = FlsSetValue(S->indices[2], THREAD(0xABCDE));
        ExitThread(0xF15);
        S->out[15] = 1;
    }
}

static void fls_second(void *value) {
    S->out[5] = (u64)value;
    S->out[6] = GetCurrentThreadId();
    S->calls++;
}

static u32 fls_worker(void *unused) {
    u32 stack_word = 0;
    if (S->mode == TERMINATE_IN_CALLBACK) S->out[20] = (u64)&stack_word;
    if (S->mode == RETURN) S->out[21] = (u64)&stack_word;
    if (!FlsSetValue(S->indices[0], THREAD(0x12345)) ||
        !FlsSetValue(S->indices[1], THREAD(0x67890))) return 1;
    if (S->mode == EXPLICIT) {
        ExitThread(0xBEEF);
        return 2;
    }
    if (S->mode == TERMINATE_WHILE_RUNNING) {
        SetEvent(S->ready);
        Sleep(INFINITE);
        return 3;
    }
    return 0xCAFE;
}

static void fls_exit(enum exit_mode mode) {
    S->indices[0] = FlsAlloc(fls_first);
    S->indices[1] = FlsAlloc(fls_second);
    S->indices[2] = FlsAlloc(fls_second); // unset slot has no cleanup
    S->mode = mode;
    S->ready = CreateEventA(0, 0, 0, 0);
    HANDLE worker = CreateThread(0, 0, fls_worker, 0, 0, 0);
    S->out[0] = GetThreadId(worker);
    if (mode == TERMINATE_WHILE_RUNNING || mode == TERMINATE_IN_CALLBACK ||
        mode == TEARDOWN_IN_CALLBACK) {
        S->out[8] = WaitForSingleObject(S->ready, INFINITE);
        if (mode != TERMINATE_WHILE_RUNNING) {
            u32 active = 0;
            GetExitCodeThread(worker, &active);
            S->out[15] = active;
            S->out[16] = WaitForSingleObject(worker, 0);
        }
        if (mode == TEARDOWN_IN_CALLBACK) {
            CloseHandle(worker);
            CloseHandle(S->ready);
            return;
        }
        S->out[9] = TerminateThread(worker, 0xDEAD);
    }
    S->out[1] = WaitForSingleObject(worker, INFINITE);
    u32 code = 0;
    GetExitCodeThread(worker, &code);
    S->out[2] = code;
    S->out[7] = S->calls;
    CloseHandle(worker);
    if (mode == TERMINATE_IN_CALLBACK) {
        S->mode = RETURN;
        HANDLE next = CreateThread(0, 0, fls_worker, 0, 0, 0);
        S->out[17] = GetThreadId(next);
        S->out[18] = WaitForSingleObject(next, INFINITE);
        GetExitCodeThread(next, &code);
        S->out[19] = code;
        CloseHandle(next);
    }
    CloseHandle(S->ready);
    for (int i = 0; i < 3; i++) S->out[10 + i] = FlsFree(S->indices[i]);
    S->out[13] = S->calls;
}

void main_fls_return(void) { fls_exit(RETURN); }
void main_fls_explicit(void) { fls_exit(EXPLICIT); }
void main_fls_terminate(void) { fls_exit(TERMINATE_WHILE_RUNNING); }
void main_fls_reenter(void) { fls_exit(EXIT_IN_CALLBACK); }
void main_fls_cancel(void) { fls_exit(TERMINATE_IN_CALLBACK); }
void main_fls_teardown(void) { fls_exit(TEARDOWN_IN_CALLBACK); }
