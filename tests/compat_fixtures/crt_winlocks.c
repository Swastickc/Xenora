// SPDX-License-Identifier: GPL-3.0-or-later
//
// Hosted checks for the kernel32 services a database engine such as sqlite3
// needs besides plain file I/O: byte-range locks, the file-API code page
// mode, free disk space, heap validation, view flushing and the process id.
// Built with mingw-w64, every call is an import. A wrong check prints its
// number and the program exits with it. With CRT_WINLOCKS_BREAK one
// expectation is wrong on purpose.

#include <process.h>
#include <stdio.h>
#include <windows.h>

static unsigned checks;
static unsigned failed;
static unsigned failures;

#define CHECK(note, expr)                                                \
    do {                                                                 \
        checks++;                                                        \
        if (!(expr)) {                                                   \
            if (!failed)                                                 \
                failed = checks;                                         \
            failures++;                                                  \
            printf("check %u failed: %s\n", checks, note);              \
        }                                                                \
    } while (0)

static BOOL lock_ex(HANDLE file, DWORD flags, DWORD offset, DWORD length) {
    OVERLAPPED overlapped = {0};
    overlapped.Offset = offset;
    return LockFileEx(file, flags | LOCKFILE_FAIL_IMMEDIATELY, 0, length, 0, &overlapped);
}

static BOOL unlock_ex(HANDLE file, DWORD offset, DWORD length) {
    OVERLAPPED overlapped = {0};
    overlapped.Offset = offset;
    return UnlockFileEx(file, 0, length, 0, &overlapped);
}

int main(void) {
    const DWORD share = FILE_SHARE_READ | FILE_SHARE_WRITE;
    HANDLE first = CreateFileW(L"C:\\locks.db", GENERIC_READ | GENERIC_WRITE, share, NULL,
                               OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    HANDLE second = CreateFileW(L"C:\\locks.db", GENERIC_READ | GENERIC_WRITE, share, NULL,
                                OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    CHECK("open twice", first != INVALID_HANDLE_VALUE && second != INVALID_HANDLE_VALUE);

    // sqlite's pending, reserved and shared bytes sit at fixed offsets.
    CHECK("exclusive lock", LockFile(first, 1000, 0, 1, 0));
    CHECK("conflicting lock fails",
          !LockFile(second, 1000, 0, 1, 0) && GetLastError() == ERROR_LOCK_VIOLATION);
    CHECK("adjacent lock", LockFile(second, 1001, 0, 1, 0));
    CHECK("partial unlock fails",
          !UnlockFile(first, 1000, 0, 2, 0) && GetLastError() == ERROR_NOT_LOCKED);
    CHECK("unlock", UnlockFile(first, 1000, 0, 1, 0) && UnlockFile(second, 1001, 0, 1, 0));

    CHECK("shared lock", lock_ex(first, 0, 2048, 510));
    CHECK("second shared lock", lock_ex(second, 0, 2048, 510));
    CHECK("exclusive over shared fails",
          !lock_ex(second, LOCKFILE_EXCLUSIVE_LOCK, 2100, 1)
              && GetLastError() == ERROR_LOCK_VIOLATION);
    CHECK("exclusive beyond shared", lock_ex(second, LOCKFILE_EXCLUSIVE_LOCK, 2558, 1));
    CHECK("unlock ex", unlock_ex(first, 2048, 510) && unlock_ex(second, 2048, 510)
                           && unlock_ex(second, 2558, 1));
    CHECK("exclusive after release", lock_ex(first, LOCKFILE_EXCLUSIVE_LOCK, 2100, 1));
    CHECK("release", unlock_ex(first, 2100, 1));

    CHECK("file APIs start ANSI", AreFileApisANSI());
    SetFileApisToOEM();
    CHECK("file APIs switch to OEM", !AreFileApisANSI());
    SetFileApisToANSI();
    CHECK("file APIs switch back", AreFileApisANSI());

    DWORD sectors = 0, bytes = 0, free_clusters = 0, clusters = 0;
    CHECK("disk free space of the current root",
          GetDiskFreeSpaceA(NULL, &sectors, &bytes, &free_clusters, &clusters)
              && sectors * bytes > 0 && clusters > 0 && free_clusters <= clusters);

    HANDLE heap = GetProcessHeap();
    void *block = HeapAlloc(heap, 0, 64);
    CHECK("heap validates", block && HeapValidate(heap, 0, NULL));
    HeapFree(heap, 0, block);

    HANDLE mapping = CreateFileMappingW(INVALID_HANDLE_VALUE, NULL, PAGE_READWRITE, 0, 4096, NULL);
    char *view = mapping ? MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, 0) : NULL;
    CHECK("flush a mapped view", view && FlushViewOfFile(view, 0));
    CHECK("flush an unknown base fails", !FlushViewOfFile((void *)0x10, 0));
    if (view)
        UnmapViewOfFile(view);
    if (mapping)
        CloseHandle(mapping);

    CHECK("process id", (DWORD)_getpid() == GetCurrentProcessId());
    CHECK("close", CloseHandle(first) && CloseHandle(second));

#ifdef CRT_WINLOCKS_BREAK
    CHECK("the control fails", sectors == 999);
#endif

    if (failures) {
        printf("crt-winlocks: %u/%u checks failed, first %u\n", failures, checks, failed);
        return failed;
    }
    printf("crt-winlocks: %u/%u checks passed\n", checks, checks);
    return 0;
}
