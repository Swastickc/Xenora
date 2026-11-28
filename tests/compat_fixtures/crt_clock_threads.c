// SPDX-License-Identifier: GPL-3.0-or-later

// Two live guest threads retain distinct struct tm results across conversions.
typedef unsigned int u32;
typedef unsigned long long u64;
typedef long long i64;

struct tm {
    int sec;
    int min;
    int hour;
    int mday;
    int mon;
    int year;
    int wday;
    int yday;
    int isdst;
};

#define IMPORT __declspec(dllimport)
IMPORT struct tm *_gmtime64(const i64 *time);
IMPORT struct tm *_localtime64(const i64 *time);
IMPORT char *_ctime64(const i64 *time);
IMPORT u64 CreateEventW(void *security, int manual_reset, int initial, const unsigned short *name);
IMPORT int SetEvent(u64 event);
IMPORT u32 WaitForSingleObject(u64 handle, u32 milliseconds);
IMPORT u64 CreateThread(void *security, u64 stack_size, u32 (*worker)(void *),
                        void *argument, u32 flags, u32 *thread_id);
IMPORT int GetExitCodeThread(u64 thread, u32 *code);
IMPORT int CloseHandle(u64 handle);
IMPORT __declspec(noreturn) void ExitProcess(u32 status);

static const i64 epoch = 0;
static const i64 jan_2_1970 = 90061;
static const i64 jan_3_1970 = 172800;
static const i64 jan_1_2000 = 946684800;
static const i64 jan_2_2000 = 946771201;
static const struct tm epoch_time = {0, 0, 0, 1, 0, 70, 4, 0, 0};
static const struct tm main_local = {1, 1, 1, 2, 0, 70, 5, 1, 0};
static const struct tm main_ctime = {0, 0, 0, 3, 0, 70, 6, 2, 0};
static const struct tm worker_local = {0, 0, 0, 1, 0, 100, 6, 0, 0};
static const struct tm worker_utc = {1, 0, 0, 2, 0, 100, 0, 1, 0};
static u64 worker_ready, release_worker;
static const struct tm *volatile worker_result;

static int matches(const struct tm *result, const struct tm *expected) {
    return result && result->sec == expected->sec && result->min == expected->min &&
           result->hour == expected->hour && result->mday == expected->mday &&
           result->mon == expected->mon && result->year == expected->year &&
           result->wday == expected->wday && result->yday == expected->yday &&
           result->isdst == expected->isdst;
}

static int same_text(const char *result, const char *expected) {
    if (!result)
        return 0;
    while (*expected && *result == *expected) {
        ++result;
        ++expected;
    }
    return *result == *expected;
}

static u32 worker(void *argument) {
    (void)argument;
    struct tm *local = _localtime64(&jan_1_2000);
    if (!matches(local, &worker_local))
        return 11;
    worker_result = local;
    if (!SetEvent(worker_ready) || WaitForSingleObject(release_worker, 1000) != 0)
        return 12;
    if (!matches(local, &worker_local))
        return 13;
    if (_gmtime64(&jan_2_2000) != local || !matches(local, &worker_utc))
        return 14;
    return 0;
}

void entry(void) {
    struct tm *main_result = _gmtime64(&epoch);
    if (!matches(main_result, &epoch_time))
        ExitProcess(1);
    worker_ready = CreateEventW(0, 0, 0, 0);
    release_worker = CreateEventW(0, 0, 0, 0);
    if (!worker_ready || !release_worker)
        ExitProcess(2);
    u64 thread = CreateThread(0, 0, worker, 0, 0, 0);
    if (!thread || WaitForSingleObject(worker_ready, 1000) != 0)
        ExitProcess(3);
    const struct tm *other = worker_result;
    if (!matches(main_result, &epoch_time) || other == main_result ||
        !matches(other, &worker_local))
        ExitProcess(4);
    if (_localtime64(&jan_2_1970) != main_result || !matches(main_result, &main_local))
        ExitProcess(5);
    if (!same_text(_ctime64(&jan_3_1970), "Sat Jan  3 00:00:00 1970\n") ||
        !matches(main_result, &main_ctime) || !matches(other, &worker_local))
        ExitProcess(6);
    u32 code;
    if (!SetEvent(release_worker) || WaitForSingleObject(thread, 1000) != 0 ||
        !GetExitCodeThread(thread, &code))
        ExitProcess(7);
    if (code != 0)
        ExitProcess(code);
    if (!matches(main_result, &main_ctime) || !CloseHandle(thread) ||
        !CloseHandle(worker_ready) || !CloseHandle(release_worker))
        ExitProcess(8);
    ExitProcess(0);
}
