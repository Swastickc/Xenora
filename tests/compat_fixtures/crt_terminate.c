// SPDX-License-Identifier: GPL-3.0-or-later

typedef unsigned int u32;
typedef unsigned short u16;
typedef unsigned long long u64;

#define IMPORT __declspec(dllimport)

IMPORT u64 CreateFileW(const u16 *path, u32 access, u32 share, void *security,
                       u32 disposition, u32 flags, u64 template);
IMPORT int WriteFile(u64 file, const void *bytes, u32 size, u32 *written, void *overlapped);
IMPORT __declspec(noreturn) void ExitProcess(u32 status);
IMPORT int atexit(void (*callback)(void));
IMPORT __declspec(noreturn) void exit(int status);
// Keep the continuation visible in the executable to catch an erroneous return.
IMPORT void terminate(void);

static const u16 journal_path[] = {
    'C', ':', '\\', 't', 'e', 'r', 'm', 'i', 'n', 'a', 't', 'e', '.', 'l', 'o', 'g', 0,
};
static u64 journal;

static void append(char marker) {
    u32 written;
    if (!WriteFile(journal, &marker, 1, &written, 0) || written != 1)
        ExitProcess(80);
}

static void callback(void) {
    append('B');
}

static void prepare(void) {
    // Leave the file open to exercise the process teardown path.
    journal = CreateFileW(journal_path, 0x40000000u, 0, 0, 2, 0, 0);
    if (journal == 0 || journal == ~0ull)
        ExitProcess(81);
    if (atexit(callback) != 0)
        ExitProcess(82);
    append('A');
}

void entry(void) {
    prepare();
    terminate();
    append('C');
    ExitProcess(83);
}

void control_entry(void) {
    prepare();
    exit(0);
}
