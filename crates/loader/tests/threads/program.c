// SPDX-License-Identifier: GPL-3.0-or-later
//
// Original guest programs for the thread scheduler tests. Each `main_*`
// function is run as the main guest thread and stores what it observed in the
// shared block, which the test maps at a fixed address.

typedef unsigned long long u64;
typedef unsigned int u32;
typedef void *HANDLE;
typedef u32 (*START)(void *);

#define IMPORT __declspec(dllimport)
IMPORT HANDLE CreateThread(void *, u64, START, void *, u32, u32 *);
IMPORT void ExitThread(u32);
IMPORT int TerminateThread(HANDLE, u32);
IMPORT int GetExitCodeThread(HANDLE, u32 *);
IMPORT u32 SuspendThread(HANDLE);
IMPORT u32 ResumeThread(HANDLE);
IMPORT int SwitchToThread(void);
IMPORT u32 GetCurrentThreadId(void);
IMPORT u32 GetThreadId(HANDLE);
IMPORT HANDLE OpenThread(u32, int, u32);
IMPORT u32 WaitForSingleObject(HANDLE, u32);
IMPORT u32 WaitForMultipleObjects(u32, const HANDLE *, int, u32);
IMPORT HANDLE CreateEventA(void *, int, int, const char *);
IMPORT int SetEvent(HANDLE);
IMPORT HANDLE CreateMutexA(void *, int, const char *);
IMPORT int ReleaseMutex(HANDLE);
IMPORT HANDLE CreateSemaphoreA(void *, int, int, const char *);
IMPORT int ReleaseSemaphore(HANDLE, int, int *);
IMPORT int CloseHandle(HANDLE);
IMPORT u32 GetLastError(void);
IMPORT void SetLastError(u32);
IMPORT u32 TlsAlloc(void);
IMPORT void *TlsGetValue(u32);
IMPORT int TlsSetValue(u32, void *);
IMPORT int *_errno(void);
IMPORT void InitializeCriticalSection(void *);
IMPORT void EnterCriticalSection(void *);
IMPORT int TryEnterCriticalSection(void *);
IMPORT void LeaveCriticalSection(void *);
IMPORT void AcquireSRWLockExclusive(void *);
IMPORT void AcquireSRWLockShared(void *);
IMPORT void ReleaseSRWLockExclusive(void *);
IMPORT void ReleaseSRWLockShared(void *);
IMPORT u32 TryAcquireSRWLockExclusive(void *);
IMPORT u32 TryAcquireSRWLockShared(void *);
IMPORT int InitOnceBeginInitialize(void *, u32, int *, void **);
IMPORT int InitOnceComplete(void *, u32, void *);
IMPORT int DuplicateHandle(HANDLE, HANDLE, HANDLE, HANDLE *, u32, int, u32);
IMPORT int SetThreadPriority(HANDLE, int);
IMPORT int GetThreadPriority(HANDLE);
IMPORT void *VirtualAlloc(void *, u64, u32, u32);
IMPORT int VirtualFree(void *, u64, u32);

#define INFINITE 0xFFFFFFFFu
#define THREAD(n) ((void *)(u64)(n))
struct shared {
    volatile u64 out[32];
    volatile u64 counter;
    volatile u64 slots[16];
    HANDLE handles[16];
    HANDLE a, b, c;
    volatile u32 tls;
    volatile u32 flag;
    u64 section[5], srw, once;
};
#define S ((struct shared *)0x50000)

static HANDLE spawn(START start, u64 parameter) {
    return CreateThread(0, 0, start, THREAD(parameter), 0, 0);
}

static void join(HANDLE thread) {
    WaitForSingleObject(thread, INFINITE);
    CloseHandle(thread);
}

// --- independent functions and exit codes --------------------------------
static u32 fibonacci(void *n) {
    u64 a = 0, b = 1;
    for (u64 i = 0; i < (u64)n; i++) {
        u64 next = a + b;
        a = b;
        b = next;
    }
    S->out[0] = a;
    return 11;
}

static u32 collatz(void *n) {
    u64 value = (u64)n, steps = 0;
    while (value != 1) {
        value = value & 1 ? 3 * value + 1 : value >> 1;
        steps++;
    }
    S->out[1] = steps;
    return 22;
}

void main_independent(void) {
    u32 id = 0, code = 0;
    S->handles[0] = CreateThread(0, 0, fibonacci, THREAD(80), 0, &id);
    S->handles[1] = spawn(collatz, 97);
    S->out[2] = WaitForMultipleObjects(2, S->handles, 1, INFINITE);
    GetExitCodeThread(S->handles[0], &code);
    S->out[3] = code;
    GetExitCodeThread(S->handles[1], &code);
    S->out[4] = code;
    S->out[5] = id == GetThreadId(S->handles[0]);
    S->out[6] = id != GetCurrentThreadId();
}

// --- shared counters -------------------------------------------------------
#define ROUNDS 200
// Uses the previous value, so the compiler emits LOCK XADD.
static u32 atomic_worker(void *unused) {
    u64 seen = 0;
    for (int i = 0; i < ROUNDS; i++) {
        seen |= __atomic_fetch_add(&S->counter, 1, __ATOMIC_SEQ_CST);
        if ((i & 7) == 0) SwitchToThread();
    }
    return (u32)seen;
}

static u32 racy_worker(void *unused) {
    for (int i = 0; i < ROUNDS; i++) {
        u64 value = S->counter;
        SwitchToThread();
        S->counter = value + 1;
    }
    return 0;
}

static u32 mutex_worker(void *unused) {
    for (int i = 0; i < ROUNDS; i++) {
        WaitForSingleObject(S->a, INFINITE);
        u64 value = S->counter;
        SwitchToThread();
        S->counter = value + 1;
        ReleaseMutex(S->a);
    }
    return 0;
}

static void run_four(START start) {
    for (int i = 0; i < 4; i++) S->handles[i] = spawn(start, i);
    S->out[0] = WaitForMultipleObjects(4, S->handles, 1, INFINITE);
    S->out[1] = S->counter;
}
void main_atomic(void) { run_four(atomic_worker); }
void main_racy(void) { run_four(racy_worker); }
void main_mutex(void) {
    S->a = CreateMutexA(0, 0, 0);
    run_four(mutex_worker);
}

// --- blocking and resuming -------------------------------------------------
static u32 consumer(void *unused) {
    u64 sum = 0;
    for (int i = 0; i < 50; i++) {
        WaitForSingleObject(S->a, INFINITE);
        sum = sum * 31 + S->slots[0];
        SetEvent(S->b);
    }
    S->out[0] = sum;
    return 0;
}

void main_producer(void) {
    S->a = CreateEventA(0, 0, 0, 0);
    S->b = CreateEventA(0, 0, 0, 0);
    HANDLE thread = spawn(consumer, 0);
    for (int i = 0; i < 50; i++) {
        S->slots[0] = i + 1;
        SetEvent(S->a);
        WaitForSingleObject(S->b, INFINITE);
    }
    join(thread);
    S->out[1] = 1;
}

static u32 ordered_waiter(void *index) {
    WaitForSingleObject(S->a, INFINITE);
    u64 position = __atomic_fetch_add(&S->counter, 1, __ATOMIC_SEQ_CST);
    S->out[position] = (u64)index;
    return 0;
}

// One auto-reset signal releases exactly one waiter, and the setter cannot
// take it back from a thread that was already waiting.
void main_fifo(void) {
    S->a = CreateEventA(0, 0, 0, 0);
    for (int i = 0; i < 4; i++) S->handles[i] = spawn(ordered_waiter, 10 + i);
    for (int i = 0; i < 20; i++) SwitchToThread();
    S->out[8] = S->counter;
    SetEvent(S->a);
    S->out[9] = WaitForSingleObject(S->a, 0);
    for (int i = 0; i < 20; i++) SwitchToThread();
    S->out[10] = S->counter;
    for (int i = 0; i < 3; i++) SetEvent(S->a);
    WaitForMultipleObjects(4, S->handles, 1, INFINITE);
}

void main_timeout(void) {
    S->a = CreateEventA(0, 1, 0, 0);
    S->out[0] = WaitForSingleObject(S->a, 250);
    S->out[1] = WaitForSingleObject(S->a, 0);
    SetEvent(S->a);
    S->out[2] = WaitForSingleObject(S->a, 250);
}

static u32 late_signal(void *unused) {
    WaitForSingleObject(S->b, 40);
    SetEvent(S->a);
    return 0;
}

void main_signal_before_timeout(void) {
    S->a = CreateEventA(0, 0, 0, 0);
    S->b = CreateEventA(0, 0, 0, 0);
    HANDLE thread = spawn(late_signal, 0);
    S->out[0] = WaitForSingleObject(S->a, 100);
    S->out[1] = WaitForSingleObject(S->a, 100);
    join(thread);
}

// --- per-thread state ------------------------------------------------------
static u32 isolated(void *index) {
    u64 n = (u64)index;
    u32 clean = GetLastError() == 0 && TlsGetValue(S->tls) == 0 && *_errno() == 0;
    TlsSetValue(S->tls, THREAD(n * 1000 + 7));
    *_errno() = (int)n + 40;
    SetLastError((u32)n + 500);
    u32 id = GetCurrentThreadId();
    for (int i = 0; i < 10; i++) SwitchToThread();
    u32 error = GetLastError();
    u32 good = clean && error == n + 500 && TlsGetValue(S->tls) == THREAD(n * 1000 + 7)
        && *_errno() == (int)n + 40 && id == GetCurrentThreadId();
    S->slots[n] = id;
    return good;
}

void main_isolated(void) {
    S->tls = TlsAlloc();
    TlsSetValue(S->tls, THREAD(0xAAAA));
    SetLastError(1234);
    u32 ids = 1;
    for (int i = 0; i < 4; i++) S->handles[i] = spawn(isolated, i);
    WaitForMultipleObjects(4, S->handles, 1, INFINITE);
    for (int i = 0; i < 4; i++) {
        u32 code = 0;
        GetExitCodeThread(S->handles[i], &code);
        S->out[i] = code;
        ids &= S->slots[i] == GetThreadId(S->handles[i]);
        for (int j = 0; j < i; j++) ids &= S->slots[i] != S->slots[j];
    }
    S->out[4] = GetLastError();
    S->out[5] = (u64)TlsGetValue(S->tls);
    S->out[6] = ids;
}

// --- lifecycle -------------------------------------------------------------
static u32 exits(void *code) {
    ExitThread((u32)(u64)code);
    S->out[15] = 1; // never reached
    return 99;
}

static u32 spins(void *unused) {
    for (;;) S->counter++;
}

void main_lifecycle(void) {
    u32 code = 0;
    S->a = CreateEventA(0, 1, 0, 0);
    HANDLE ended = spawn(exits, 0xC0DE);
    S->out[0] = WaitForSingleObject(ended, INFINITE);
    GetExitCodeThread(ended, &code);
    S->out[1] = code;
    HANDLE spinning = spawn(spins, 0);
    SwitchToThread();
    GetExitCodeThread(spinning, &code);
    S->out[2] = code; // STILL_ACTIVE
    S->out[3] = WaitForSingleObject(spinning, 0);
    S->out[4] = TerminateThread(spinning, 77);
    S->out[5] = WaitForSingleObject(spinning, INFINITE);
    GetExitCodeThread(spinning, &code);
    S->out[6] = code;
    S->out[7] = TerminateThread(spinning, 78);
    S->out[8] = GetLastError();
    GetExitCodeThread(spinning, &code);
    S->out[9] = code;
    S->out[10] = CloseHandle(spinning);
    S->out[11] = WaitForSingleObject(spinning, 0);
    S->out[12] = GetLastError();
    HANDLE opened = OpenThread(0x00100040, 0, GetThreadId(ended));
    S->out[13] = (u64)opened; // an ended thread has no ID
}

static u32 marks(void *unused) {
    S->flag = 1;
    return 5;
}

void main_suspend(void) {
    HANDLE thread = CreateThread(0, 0x20000, marks, 0, 4, 0);
    for (int i = 0; i < 10; i++) SwitchToThread();
    S->out[0] = S->flag;
    S->out[1] = SuspendThread(thread);
    S->out[2] = ResumeThread(thread);
    S->out[3] = WaitForSingleObject(thread, 10);
    S->out[4] = ResumeThread(thread);
    S->out[5] = WaitForSingleObject(thread, INFINITE);
    S->out[6] = S->flag;
    S->out[7] = SuspendThread(thread);
    S->out[8] = GetLastError();
    u32 id = GetThreadId(thread);
    HANDLE other = CreateThread(0, 0, marks, 0, 4, &id);
    HANDLE opened = OpenThread(0x00100002, 0, id);
    S->out[9] = ResumeThread(opened);
    S->out[10] = WaitForSingleObject(opened, INFINITE);
    u32 code = 0;
    S->out[11] = GetExitCodeThread(opened, &code);
    S->out[12] = GetLastError(); // the handle has no query right
    S->out[13] = TerminateThread(opened, 1);
    for (int i = 0; i < 200; i++) {
        if (SuspendThread(THREAD(-2)) == 0xFFFFFFFF) break;
        S->out[14]++;
        break;
    }
}

#define CYCLES 600
static u32 short_lived(void *index) {
    u32 clean = GetLastError() == 0 && TlsGetValue(S->tls) == 0 && *_errno() == 0;
    SetLastError(9);
    TlsSetValue(S->tls, index);
    *_errno() = 33;
    S->counter += clean;
    return (u32)(u64)index;
}

void main_churn(void) {
    S->tls = TlsAlloc();
    u64 right = 0;
    for (u64 i = 1; i <= CYCLES; i++) {
        u32 code = 0;
        HANDLE thread = spawn(short_lived, i);
        WaitForSingleObject(thread, INFINITE);
        GetExitCodeThread(thread, &code);
        right += code == i;
        CloseHandle(thread);
    }
    S->out[0] = right;
    S->out[1] = S->counter;
}

void main_limit(void) {
    u64 made = 0;
    S->a = CreateEventA(0, 1, 0, 0);
    for (int i = 0; i < 100; i++) {
        HANDLE thread = CreateThread(0, 0x10000, ordered_waiter, 0, 0, 0);
        if (!thread) break;
        made++;
        CloseHandle(thread);
    }
    S->out[0] = made;
    S->out[1] = GetLastError();
    S->out[2] = (u64)CreateThread(0, 0x10000000, marks, 0, 0, 0);
    S->out[3] = (u64)CreateThread(0, 0, marks, 0, 0x80, 0);
    S->out[4] = GetLastError();
}

// --- failures --------------------------------------------------------------
static u32 locks_b_then_a(void *unused) {
    WaitForSingleObject(S->b, INFINITE);
    SetEvent(S->c);
    WaitForSingleObject(S->a, INFINITE);
    return 0;
}

void main_deadlock(void) {
    S->a = CreateMutexA(0, 1, 0);
    S->b = CreateMutexA(0, 0, 0);
    S->c = CreateEventA(0, 0, 0, 0);
    spawn(locks_b_then_a, 0);
    WaitForSingleObject(S->c, INFINITE);
    WaitForSingleObject(S->b, INFINITE);
}

static u32 abandons(void *unused) {
    WaitForSingleObject(S->a, INFINITE);
    return 0;
}

void main_abandoned(void) {
    S->a = CreateMutexA(0, 0, 0);
    S->handles[0] = CreateEventA(0, 1, 0, 0);
    S->handles[1] = S->a;
    join(spawn(abandons, 0));
    S->out[0] = WaitForMultipleObjects(2, S->handles, 0, INFINITE);
    S->out[1] = ReleaseMutex(S->a);
    S->out[2] = WaitForSingleObject(S->a, INFINITE);
}

static u64 recurse(volatile u64 *previous) {
    volatile u64 frame[64];
    frame[0] = (u64)previous;
    S->out[0]++;
    return recurse(frame) + frame[0];
}
static u32 overflows(void *unused) { return (u32)recurse(0); }
void main_overflow(void) { join(spawn(overflows, 0)); }

static u32 changes_the_wait(void *unused) {
    for (int i = 0; i < 100; i++) SwitchToThread(); // main blocks first
    CloseHandle(S->a);
    S->handles[0] = S->handles[1] = S->c;
    ReleaseSemaphore(S->b, 1, 0);
    return 0;
}

// The wait keeps the objects it was given.
void main_captured(void) {
    S->a = CreateEventA(0, 1, 0, 0);
    S->b = CreateSemaphoreA(0, 0, 5, 0);
    S->c = CreateEventA(0, 1, 1, 0);
    S->handles[0] = S->a;
    S->handles[1] = S->b;
    HANDLE thread = spawn(changes_the_wait, 0);
    S->out[0] = WaitForMultipleObjects(2, S->handles, 0, INFINITE);
    join(thread);
    S->out[1] = WaitForSingleObject(S->b, 0);
}

void main_outlived(void) {
    spawn(spins, 0);
    SwitchToThread();
}

static u32 last_one(void *unused) {
    S->out[0] = WaitForSingleObject(S->a, 30);
    S->out[1] = 1;
    return 0;
}
void main_exits_first(void) {
    S->a = CreateEventA(0, 0, 0, 0);
    spawn(last_one, 0);
    ExitThread(3);
}

// --- hand-off and rights ---------------------------------------------------
static u32 takes(void *unused) {
    WaitForSingleObject(S->a, INFINITE);
    S->counter++;
    return 0;
}

static void settle(void) {
    for (int i = 0; i < 30; i++) SwitchToThread();
}

// An object signaled while a thread waits belongs to that thread, which has
// not run yet when the signaling call returns.
void main_handoff(void) {
    S->a = CreateMutexA(0, 1, 0);
    spawn(takes, 0);
    settle();
    ReleaseMutex(S->a);
    S->out[0] = WaitForSingleObject(S->a, 0);
    S->out[1] = S->counter;
    settle();
    S->out[2] = S->counter;

    S->a = CreateSemaphoreA(0, 0, 5, 0);
    spawn(takes, 0);
    settle();
    ReleaseSemaphore(S->a, 1, 0);
    S->out[3] = WaitForSingleObject(S->a, 0);
    S->out[4] = S->counter;
    settle();
    S->out[5] = S->counter;

    S->a = CreateEventA(0, 0, 0, 0);
    spawn(takes, 0);
    settle();
    SetEvent(S->a);
    S->out[6] = WaitForSingleObject(S->a, 0);
    S->out[7] = S->counter;
    settle();
    S->out[8] = S->counter;

    // Signaling does not cost the caller its turn.
    spawn(marks, 0);
    SetEvent(S->a);
    S->out[9] = S->flag;
    settle();
    S->out[10] = S->flag;
}

// A suspended thread does not take what it waits for.
void main_suspended_waiter(void) {
    S->a = CreateSemaphoreA(0, 0, 5, 0);
    HANDLE thread = spawn(takes, 0);
    settle();
    S->out[0] = SuspendThread(thread);
    ReleaseSemaphore(S->a, 1, 0);
    settle();
    S->out[1] = WaitForSingleObject(S->a, 0);
    S->out[2] = S->counter;
    ReleaseSemaphore(S->a, 1, 0);
    ResumeThread(thread);
    join(thread);
    S->out[3] = S->counter;
}

void main_rights(void) {
    S->a = CreateEventA(0, 1, 0, 0);
    HANDLE thread = spawn(takes, 0);
    u32 id = GetThreadId(thread), code = 0;
    HANDLE query = OpenThread(0x00100040, 0, id);
    HANDLE suspend = OpenThread(0x00000002, 0, id);
    S->out[0] = SuspendThread(query);
    S->out[1] = GetLastError();
    S->out[2] = TerminateThread(query, 1);
    S->out[3] = GetLastError();
    S->out[4] = GetExitCodeThread(query, &code);
    S->out[5] = code;
    S->out[6] = SuspendThread(suspend);
    S->out[7] = ResumeThread(suspend);
    S->out[8] = GetExitCodeThread(suspend, &code);
    S->out[9] = WaitForSingleObject(suspend, 0);
    S->out[10] = GetLastError();
    S->out[11] = GetThreadId(suspend);
    SetEvent(S->a);
    join(thread);
    S->out[12] = GetThreadId(query) == id; // the object outlives the thread
}

static u32 halts(void *unused) {
    __asm__ volatile("hlt");
    return 0;
}
void main_worker_halts(void) { join(spawn(halts, 0)); }

static u32 reports_stack(void *unused) {
    volatile u64 local = 0;
    if (!S->slots[0]) S->slots[0] = (u64)&local;
    S->counter += S->slots[0] == (u64)&local;
    return 0;
}
void main_reuse(void) {
    for (int i = 0; i < 50; i++) join(spawn(reports_stack, 0));
}

// Signaling over and over must not starve a thread that is ready to run.
void main_starve(void) {
    S->a = CreateEventA(0, 0, 0, 0);
    spawn(marks, 0);
    u64 signals = 0;
    while (!S->flag) {
        SetEvent(S->a);
        signals++;
    }
    S->out[0] = S->flag;
    S->out[1] = signals;
}

// --- locks of an ended thread ----------------------------------------------
static u32 takes_locks(void *unused) {
    int pending = 0;
    EnterCriticalSection(S->section);
    S->out[0] = S->section[2] == GetCurrentThreadId(); // OwningThread
    AcquireSRWLockExclusive(&S->srw);
    S->out[1] = InitOnceBeginInitialize(&S->once, 0, &pending, 0) && pending;
    return 0;
}

static u32 inherits_locks(void *unused) {
    u32 local = 0;
    S->out[2] = TryEnterCriticalSection(S->section);
    S->out[3] = TryAcquireSRWLockExclusive(&S->srw) & 0xFF;
    S->out[4] = TryAcquireSRWLockShared(&S->srw) & 0xFF;
    S->out[5] = InitOnceComplete(&S->once, 0, 0);
    S->out[6] = (u64)&local;
    return 0;
}

void main_orphaned_locks(void) {
    InitializeCriticalSection(S->section);
    join(spawn(takes_locks, 0));
    S->out[7] = TryEnterCriticalSection(S->section);
    join(spawn(inherits_locks, 0));
}

static u32 begins_once(void *where) {
    int pending = 0;
    InitOnceBeginInitialize(where, 0, &pending, 0);
    return 0;
}
void main_pending_bound(void) {
    char *block = VirtualAlloc(0, 0x10000, 0x3000, 4);
    for (u64 i = 0; i < 5000; i++) {
        join(spawn(begins_once, (u64)block + 8 * i));
        S->counter++;
    }
}

// --- identity --------------------------------------------------------------
static u32 duplicates_itself(void *unused) {
    HANDLE copy = 0;
    u32 code = 0;
    S->out[0] = DuplicateHandle(THREAD(-1), THREAD(-2), THREAD(-1), &copy, 0, 0, 2);
    S->out[1] = GetThreadId(copy) == GetCurrentThreadId();
    S->out[2] = WaitForSingleObject(copy, 0);
    S->out[3] = GetExitCodeThread(copy, &code) && code == 259;
    S->out[4] = SetThreadPriority(THREAD(-2), 2);
    S->out[5] = GetLastError();
    S->handles[0] = copy;
    return 44;
}

void main_duplicate(void) {
    u32 code = 0;
    join(spawn(duplicates_itself, 0));
    S->out[6] = WaitForSingleObject(S->handles[0], 0);
    GetExitCodeThread(S->handles[0], &code);
    S->out[7] = code;
    S->out[8] = (u64)GetThreadPriority(THREAD(-2));
}

// --- stacks ----------------------------------------------------------------
// Every frame's address is used by its callee, so this stays recursive.
static u64 deep(u64 n, volatile u64 *caller) {
    volatile u64 frame[64];
    frame[0] = n;
    frame[1] = (u64)caller;
    return n ? deep(n - 1, frame) + frame[0] : (S->slots[1] = (u64)frame, 0);
}
static u32 recurses(void *n) {
    volatile u64 top = 0;
    S->slots[0] = (u64)&top;
    S->out[0] = deep((u64)n, &top);
    return 1;
}
// 200 frames of over 512 bytes need more than 100 KB.
void main_small_commit(void) { join(CreateThread(0, 4096, recurses, THREAD(200), 0, 0)); }
void main_small_reservation(void) { join(CreateThread(0, 4096, recurses, THREAD(200), 0x10000, 0)); }

void main_area_taken(void) {
    S->out[0] = (u64)VirtualAlloc(THREAD(0x600000000), 0x10000, 0x3000, 4);
    HANDLE thread = spawn(marks, 0);
    S->out[1] = thread != 0;
    join(thread);
    S->out[2] = S->flag;
}

static u32 frees_itself(void *unused) {
    u64 local = 0;
    S->out[0] = VirtualFree(THREAD(((u64)&local & ~0xFFFFFFull) + 0x1000), 0, 0x8000);
    S->out[1] = VirtualFree(THREAD(((u64)&local & ~0xFFFFFFull) + 0xF00000), 0, 0x8000);
    return 0;
}
void main_free_stack(void) { join(spawn(frees_itself, 0)); }

// --- ending threads in every state -------------------------------------------
static u32 waits_forever(void *unused) {
    WaitForSingleObject(S->a, INFINITE);
    S->flag = 9;
    return 0;
}

static u32 terminates_itself(void *unused) {
    TerminateThread(THREAD(-2), 61);
    S->flag = 9;
    return 0;
}

static u32 owns_both(void *unused) {
    WaitForMultipleObjects(2, S->handles, 1, INFINITE);
    return 0;
}

void main_terminations(void) {
    u32 code = 0;
    S->a = CreateEventA(0, 0, 0, 0);
    HANDLE blocked = spawn(waits_forever, 0);
    settle();
    S->out[0] = TerminateThread(blocked, 60);
    GetExitCodeThread(blocked, &code);
    S->out[1] = code;
    SetEvent(S->a); // nobody is left to consume it
    S->out[2] = WaitForSingleObject(S->a, 0);
    HANDLE suspended = CreateThread(0, 0, waits_forever, 0, 4, 0);
    S->out[3] = TerminateThread(suspended, 62);
    S->out[4] = WaitForSingleObject(suspended, INFINITE);
    HANDLE itself = spawn(terminates_itself, 0);
    WaitForSingleObject(itself, INFINITE);
    GetExitCodeThread(itself, &code);
    S->out[5] = code;
    S->out[6] = S->flag;
    S->handles[0] = CreateMutexA(0, 0, 0);
    S->handles[1] = CreateMutexA(0, 0, 0);
    join(spawn(owns_both, 0));
    S->out[7] = WaitForMultipleObjects(2, S->handles, 1, INFINITE);
    ReleaseMutex(S->handles[0]);
    ReleaseMutex(S->handles[1]);
    S->out[8] = WaitForMultipleObjects(2, S->handles, 1, INFINITE);
    HANDLE limit = CreateThread(0, 0, marks, 0, 4, 0);
    for (int i = 0; i < 126; i++) SuspendThread(limit);
    S->out[9] = SuspendThread(limit);
    S->out[10] = GetLastError();
    TerminateThread(limit, 0);
}

void main_keeps_state(void) {
    S->tls = TlsAlloc();
    TlsSetValue(S->tls, THREAD(77));
    SetLastError(88);
    for (volatile u64 i = 0; i < 3000; i++) {}
    S->out[0] = GetLastError();
    S->out[1] = (u64)TlsGetValue(S->tls);
}

void main_signals(void) {
    S->a = CreateEventA(0, 1, 0, 0);
    for (volatile u64 i = 0; i < 1000; i++) SetEvent(S->a);
}

static u32 holds_exclusive(void *unused) { AcquireSRWLockExclusive(&S->srw); return 0; }
static u32 holds_shared(void *unused) { AcquireSRWLockShared(&S->srw); return 0; }
static u32 releases_exclusive(void *unused) { ReleaseSRWLockExclusive(&S->srw); return S->flag = 1; }
static u32 releases_shared(void *unused) { ReleaseSRWLockShared(&S->srw); return S->flag = 1; }
void main_orphaned_exclusive(void) {
    join(spawn(holds_exclusive, 0));
    join(spawn(releases_exclusive, 0));
}
void main_orphaned_shared(void) {
    join(spawn(holds_shared, 0));
    S->out[0] = TryAcquireSRWLockShared(&S->srw) & 0xFF; // sharing is still possible
    S->out[1] = TryAcquireSRWLockExclusive(&S->srw) & 0xFF;
    join(spawn(releases_shared, 0));
}

// Host services the test registers at fixed addresses.
#define ADVANCE ((void (*)(u64))0x60000)
#define NESTED ((void (*)(void))0x60010)

// --- a signal after the deadline belongs to nobody -------------------------
static u32 signals_too_late(void *unused) {
    while (!S->flag) SwitchToThread();
    settle();
    ADVANCE(100);
    SetEvent(S->a);
    return 0;
}
void main_expired_wait(void) {
    S->a = CreateEventA(0, 0, 0, 0);
    HANDLE late = spawn(signals_too_late, 0);
    S->flag = 1;
    S->out[0] = WaitForSingleObject(S->a, 50);
    S->out[1] = WaitForSingleObject(late, INFINITE);
    S->out[2] = WaitForSingleObject(S->a, 0);
    S->out[3] = WaitForSingleObject(S->a, 0);
}

void main_continued_wait(void) {
    S->a = CreateEventA(0, 1, 0, 0);
    S->handles[0] = S->a;
    S->out[0] = WaitForMultipleObjects(1, S->handles, 1, 1000);
    S->out[1] = 1;
}

static u32 reopens(void *id) {
    settle();
    S->out[2] = (u64)OpenThread(0x40, 0, (u32)(u64)id);
    S->out[3] = GetLastError();
    return 0;
}
void main_reopened(void) {
    u32 id = GetCurrentThreadId();
    S->out[0] = OpenThread(0x40, 0, id) != 0;
    S->out[1] = spawn(reopens, id) != 0;
    ExitThread(3);
}

// A tail call: the wait returns straight to the end of the thread.
static u32 ends_in_a_wait(HANDLE event, u32 unused) {
    __attribute__((musttail)) return WaitForSingleObject(event, 50);
}
void main_ends_in_a_wait(void) {
    u32 code = 0;
    S->a = CreateEventA(0, 1, 0, 0);
    S->handles[0] = spawn((START)(void *)ends_in_a_wait, (u64)S->a);
    S->out[0] = WaitForSingleObject(S->handles[0], INFINITE);
    S->handles[1] = spawn((START)(void *)ends_in_a_wait, (u64)S->a);
    S->out[1] = WaitForSingleObject(S->handles[1], 1000);
    GetExitCodeThread(S->handles[1], &code);
    S->out[2] = code;
}

void main_nested(void) {
    NESTED();
    S->out[1] = WaitForSingleObject(spawn(collatz, 97), INFINITE);
}
