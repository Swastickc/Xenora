// SPDX-License-Identifier: GPL-3.0-or-later

typedef unsigned int u32;
typedef unsigned long long u64;

#define IMPORT __declspec(dllimport)
#define CHECK(condition, status) do { if (!(condition)) ExitProcess(status); } while (0)

IMPORT int WaitOnAddress(volatile void *address, const void *compare, u64 width, u32 milliseconds);
IMPORT void WakeByAddressSingle(void *address);
IMPORT void WakeByAddressAll(void *address);
IMPORT u64 CreateEventW(void *security, int manual_reset, int initial, const void *name);
IMPORT int SetEvent(u64 event);
IMPORT u64 CreateThread(void *security, u64 stack_size, u32 (*entry)(void *),
                        void *parameter, u32 flags, u32 *thread_id);
IMPORT u32 WaitForSingleObject(u64 handle, u32 milliseconds);
IMPORT int GetExitCodeThread(u64 thread, u32 *status);
IMPORT u32 SuspendThread(u64 thread);
IMPORT u32 ResumeThread(u64 thread);
IMPORT int CloseHandle(u64 handle);
IMPORT u32 GetLastError(void);
IMPORT u64 GetTickCount64(void);
IMPORT void Sleep(u32 milliseconds);
IMPORT __declspec(noreturn) void ExitProcess(u32 status);

static volatile u64 first_address;
static volatile u64 second_address;

struct waiter {
    volatile u64 *address;
    u64 ready;
    u64 thread;
    u32 milliseconds;
    volatile u32 completed;
};

static struct waiter waiters[4];

static u32 wait_in_worker(void *parameter) {
    struct waiter *waiter = parameter;
    const u64 expected = 0;
    if (!SetEvent(waiter->ready))
        return 1;
    if (!WaitOnAddress(waiter->address, &expected, 8, waiter->milliseconds))
        return 2;
    if (*waiter->address != expected)
        return 3;
    waiter->completed = 1;
    return 0;
}

static void start_waiter(struct waiter *waiter, volatile u64 *address, u32 milliseconds) {
    waiter->address = address;
    waiter->milliseconds = milliseconds;
    waiter->completed = 0;
    waiter->ready = CreateEventW(0, 0, 0, 0);
    CHECK(waiter->ready != 0, 30);
    waiter->thread = CreateThread(0, 0, wait_in_worker, waiter, 0, 0);
    CHECK(waiter->thread != 0, 31);
    CHECK(WaitForSingleObject(waiter->ready, 100) == 0, 32);
    // The scheduler cannot advance the mock clock while the worker can run.
    // With the main thread asleep, the worker parks before the next start.
    Sleep(1);
    CHECK(waiter->completed == 0, 33);
}

static void join_waiter(const struct waiter *waiter) {
    u32 status = 0xffffffffu;
    CHECK(WaitForSingleObject(waiter->thread, 100) == 0, 50);
    CHECK(GetExitCodeThread(waiter->thread, &status) && status == 0, 51);
    CHECK(waiter->completed == 1, 52);
    CHECK(CloseHandle(waiter->thread) && CloseHandle(waiter->ready), 53);
}

void entry(void) {
    const u64 byte_difference = 0x80;
    const u64 word_difference = 0x8000;
    const u64 dword_difference = 0x80000000u;
    const u64 qword_difference = 0x8000000000000000ull;
    const u64 same = 0;
    CHECK(WaitOnAddress(&first_address, &byte_difference, 1, 0), 10);
    CHECK(WaitOnAddress(&first_address, &word_difference, 2, 0), 11);
    CHECK(WaitOnAddress(&first_address, &dword_difference, 4, 0), 12);
    CHECK(WaitOnAddress(&first_address, &qword_difference, 8, 0), 13);
    CHECK(!WaitOnAddress(&first_address, &word_difference, 1, 0) && GetLastError() == 1460, 14);
    CHECK(!WaitOnAddress(&first_address, &dword_difference, 2, 0) && GetLastError() == 1460, 15);
    CHECK(!WaitOnAddress(&first_address, &qword_difference, 4, 0) && GetLastError() == 1460, 16);
    CHECK(!WaitOnAddress(&first_address, &same, 8, 0) && GetLastError() == 1460, 17);
    CHECK(!WaitOnAddress(&first_address, &same, 3, 0) && GetLastError() == 87, 18);
    CHECK(!WaitOnAddress(&first_address, &same, 0x100000001ull, 0) && GetLastError() == 87, 19);

    const u64 before = GetTickCount64();
    CHECK(!WaitOnAddress(&first_address, &same, 8, 7) && GetLastError() == 1460, 20);
    CHECK(GetTickCount64() - before >= 7, 21);

    // Neither a pre-wait wake nor a wake on the other address can release a waiter.
    WakeByAddressSingle((void *)&first_address);
    WakeByAddressAll((void *)&second_address);
    for (u32 index = 0; index < 3; ++index)
        start_waiter(&waiters[index], &first_address, 100);
    start_waiter(&waiters[3], &second_address, 100);

    WakeByAddressSingle((void *)&first_address);
    join_waiter(&waiters[0]);
    CHECK(waiters[1].completed == 0 && waiters[2].completed == 0 && waiters[3].completed == 0, 40);
    WakeByAddressAll((void *)&first_address);
    join_waiter(&waiters[2]);
    join_waiter(&waiters[1]);
    CHECK(waiters[3].completed == 0, 41);
    WakeByAddressSingle((void *)&second_address);
    join_waiter(&waiters[3]);
    CHECK(first_address == 0 && second_address == 0, 42);

    // A suspended older waiter cannot consume the second worker's wake.
    start_waiter(&waiters[0], &first_address, 0xffffffffu);
    CHECK(SuspendThread(waiters[0].thread) == 0, 43);
    start_waiter(&waiters[1], &first_address, 0xffffffffu);
    WakeByAddressSingle((void *)&first_address);
    join_waiter(&waiters[1]);
    CHECK(waiters[0].completed == 0, 44);
    CHECK(ResumeThread(waiters[0].thread) == 1, 45);
    CHECK(WaitForSingleObject(waiters[0].thread, 0) == 258, 46);
    WakeByAddressSingle((void *)&first_address);
    join_waiter(&waiters[0]);
    ExitProcess(0);
}

void bad_compare_entry(void) {
    (void)WaitOnAddress(&first_address, 0, 8, 100);
    ExitProcess(60);
}

void unscheduled_entry(void) {
    const u64 same = 0;
    CHECK(!WaitOnAddress(&first_address, &same, 8, 10) && GetLastError() == 50, 61);
    ExitProcess(0);
}
