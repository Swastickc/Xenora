// SPDX-License-Identifier: GPL-3.0-or-later

// Original Win64 program using the public winuser.h MSG and function layouts.
typedef unsigned int u32;
typedef unsigned long long u64;
typedef long long lparam;
typedef void *handle;
typedef u32 (__stdcall *thread_proc)(void *);

struct message {
    handle hwnd;
    u32 id;
    u64 wparam;
    lparam lparam;
    u32 time;
    struct { int x, y; } point;
    u32 private_value;
};

_Static_assert(sizeof(struct message) == 48, "Win64 MSG size");
_Static_assert(__builtin_offsetof(struct message, wparam) == 16, "WPARAM offset");
_Static_assert(__builtin_offsetof(struct message, lparam) == 24, "LPARAM offset");
_Static_assert(__builtin_offsetof(struct message, time) == 32, "time offset");

#define IMPORT __declspec(dllimport)
IMPORT int __stdcall PeekMessageA(struct message *, handle, u32, u32, u32);
IMPORT int __stdcall PeekMessageW(struct message *, handle, u32, u32, u32);
IMPORT int __stdcall PostThreadMessageA(u32, u32, u64, lparam);
IMPORT int __stdcall PostThreadMessageW(u32, u32, u64, lparam);
IMPORT handle __stdcall CreateEventW(void *, int, int, const unsigned short *);
IMPORT handle __stdcall CreateThread(void *, u64, thread_proc, void *, u32, u32 *);
IMPORT int __stdcall SetEvent(handle);
IMPORT u32 __stdcall WaitForSingleObject(handle, u32);
IMPORT int __stdcall CloseHandle(handle);
IMPORT u32 __stdcall GetCurrentThreadId(void);
IMPORT u32 __stdcall GetLastError(void);
IMPORT void __stdcall SetLastError(u32);
IMPORT void __stdcall Sleep(u32);
IMPORT __declspec(noreturn) void __stdcall ExitProcess(u32);
IMPORT u64 OpenHandleCount(void);

#define CHECK(condition, code) do { if (!(condition)) ExitProcess(code); } while (0)
#define THREAD_ONLY ((handle)~0ull)
#define INVALID_THREAD_ID 1444u
#define NOT_ENOUGH_QUOTA 1816u
#define NOACCESS 998u
#define QUIT 0x12u
#define NOREMOVE 0u
#define REMOVE 1u
#define FIRST 0x411u
#define SELECTED 0x415u
#define MAIN_ONLY 0x419u
#define WPARAM_VALUE 0xdeadbeef01234567ull
#define LPARAM_VALUE 0xfedcba9876543210ull
#define BEFORE 0x0123456789abcdefull
#define AFTER 0xf0e1d2c3b4a59687ull

struct guarded_message {
    u64 before;
    struct message value;
    u64 after;
};

static handle start_worker;
static handle worker_ready;
static handle posted;

static void prepare(struct guarded_message *out)
{
    out->before = BEFORE;
    out->value.id = 0xa5a5a5a5u;
    out->after = AFTER;
}

static void check_guards(const struct guarded_message *out)
{
    CHECK(out->before == BEFORE && out->after == AFTER, 2);
}

static void check_empty(const struct guarded_message *out)
{
    CHECK(out->value.id == 0xa5a5a5a5u, 3);
    check_guards(out);
}

static void check_message(const struct guarded_message *out, u32 id, u64 wparam, u64 param)
{
    const struct message *msg = &out->value;
    CHECK(msg->hwnd == 0 && msg->id == id, 4);
    CHECK(msg->wparam == wparam && (u64)msg->lparam == param, 5);
    CHECK(msg->time == 37 && msg->point.x == 0 && msg->point.y == 0 && msg->private_value == 0, 6);
    check_guards(out);
}

static u32 __stdcall pull_messages(void *parameter)
{
    struct guarded_message out;
    CHECK(parameter == 0 && WaitForSingleObject(start_worker, 0xffffffffu) == 0, 10);
    prepare(&out);
    CHECK(!PeekMessageW(&out.value, THREAD_ONLY, 0, 0, NOREMOVE), 11);
    check_empty(&out);
    CHECK(SetEvent(worker_ready) && WaitForSingleObject(posted, 0xffffffffu) == 0, 12);

    // Search beyond the first queued record without disturbing either record.
    CHECK(PeekMessageW(&out.value, THREAD_ONLY, SELECTED - 1, SELECTED, NOREMOVE), 13);
    check_message(&out, SELECTED, WPARAM_VALUE, LPARAM_VALUE);
    CHECK(!PeekMessageA((struct message *)0x10, 0, SELECTED, SELECTED, REMOVE) &&
          GetLastError() == NOACCESS, 41);
    CHECK(PeekMessageA(&out.value, 0, SELECTED, SELECTED, REMOVE), 14);
    check_message(&out, SELECTED, WPARAM_VALUE, LPARAM_VALUE);

    // The two messages with identical IDs retain their original FIFO order.
    CHECK(PeekMessageA(&out.value, 0, 0, 0, REMOVE), 15);
    check_message(&out, FIRST, 1, 2);
    CHECK(PeekMessageW(&out.value, THREAD_ONLY, 0, 0, REMOVE), 16);
    check_message(&out, FIRST, 3, 4);
    CHECK(PeekMessageW(&out.value, THREAD_ONLY, SELECTED, SELECTED, REMOVE), 42);
    check_message(&out, QUIT, 11, 12);
    prepare(&out);
    CHECK(!PeekMessageA(&out.value, 0, 0, 0, REMOVE), 17);
    check_empty(&out);
    CHECK(PostThreadMessageW(GetCurrentThreadId(), FIRST, 9, 10), 18);
    return 0;
}

void entry(void)
{
    struct guarded_message out;
    const u32 main_id = GetCurrentThreadId();
    CHECK(main_id != 0, 20);
    CHECK(!PostThreadMessageW(main_id, FIRST, 0, 0) && GetLastError() == INVALID_THREAD_ID, 21);
    prepare(&out);
    CHECK(!PeekMessageA(&out.value, (handle)13, 0, 0, NOREMOVE), 22);
    check_empty(&out);
    SetLastError(0);
    CHECK(!PostThreadMessageA(main_id, FIRST, 0, 0) && GetLastError() == INVALID_THREAD_ID, 23);
    CHECK(!PeekMessageA(&out.value, 0, FIRST, FIRST, NOREMOVE), 24);
    check_empty(&out);

    start_worker = CreateEventW(0, 1, 0, 0);
    worker_ready = CreateEventW(0, 1, 0, 0);
    posted = CreateEventW(0, 1, 0, 0);
    CHECK(start_worker && worker_ready && posted, 25);
    u32 worker_id = 0;
    handle worker = CreateThread(0, 0, pull_messages, 0, 0, &worker_id);
    CHECK(worker && worker_id != 0 && worker_id != main_id, 26);
    SetLastError(0);
    CHECK(!PostThreadMessageA(worker_id, FIRST, 0, 0) && GetLastError() == INVALID_THREAD_ID, 27);
    CHECK(SetEvent(start_worker) && WaitForSingleObject(worker_ready, 0xffffffffu) == 0, 28);

    Sleep(37);
    CHECK(PostThreadMessageA(worker_id, FIRST, 1, 2), 30);
    CHECK(PostThreadMessageW(worker_id, SELECTED, WPARAM_VALUE, (lparam)LPARAM_VALUE), 31);
    CHECK(PostThreadMessageA(worker_id, FIRST, 3, 4), 32);
    CHECK(PostThreadMessageW(worker_id, QUIT, 11, 12), 43);
    CHECK(!PeekMessageW(&out.value, 0, 0, 0, NOREMOVE), 33);
    check_empty(&out);
    CHECK(PostThreadMessageW(main_id, MAIN_ONLY, 5, 6), 34);
    // Delivery occurs at a later time than posting.
    Sleep(9);
    CHECK(SetEvent(posted) && WaitForSingleObject(worker, 0xffffffffu) == 0, 35);
    SetLastError(0);
    CHECK(!PostThreadMessageW(worker_id, FIRST, 0, 0) && GetLastError() == INVALID_THREAD_ID, 36);
    CHECK(PeekMessageA(&out.value, THREAD_ONLY, 0, 0, REMOVE), 37);
    check_message(&out, MAIN_ONLY, 5, 6);
    prepare(&out);
    CHECK(!PeekMessageW(&out.value, 0, 0, 0, REMOVE), 38);
    check_empty(&out);

    for (u32 index = 0; index < 10000; ++index)
        CHECK(PostThreadMessageA(main_id, MAIN_ONLY, index, 0), 44);
    CHECK(!PostThreadMessageW(main_id, MAIN_ONLY, 0, 0) &&
          GetLastError() == NOT_ENOUGH_QUOTA, 45);
    CHECK(PeekMessageA(&out.value, 0, 0, 0, REMOVE) && out.value.wparam == 0, 46);
    CHECK(PostThreadMessageW(main_id, MAIN_ONLY, 10000, 0), 47);

    CHECK(CloseHandle(worker) && CloseHandle(posted) && CloseHandle(worker_ready) && CloseHandle(start_worker), 39);
    CHECK(OpenHandleCount() == 0, 40);
    ExitProcess(0);
}
