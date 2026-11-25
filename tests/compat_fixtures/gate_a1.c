// SPDX-License-Identifier: GPL-3.0-or-later
//
// Gate A1: an original C program with no C library linked.
//
// It allocates and frees memory, sorts with a callback, does 64-bit and
// double arithmetic, uses the C string functions, reads and writes files on
// the synthetic C: drive, checks two error paths, writes a text report and
// ends its process with a status. Every outside function is imported by name
// from kernel32.dll or ucrtbase.dll; the prototypes below follow the public
// Windows SDK and C standard declarations. Nothing else is linked.
//
// Build with tests/compat_fixtures/build-gate-a1.sh. The test that runs it,
// crates/loader/tests/gate_a1.rs, recomputes every number in the report.

typedef unsigned long long u64;
typedef long long i64;
typedef unsigned int u32;
typedef unsigned short wchar;
typedef unsigned char u8;

#define IMPORT __declspec(dllimport)

// ucrtbase.dll
IMPORT void *malloc(u64 size);
IMPORT void *realloc(void *block, u64 size);
IMPORT void free(void *block);
IMPORT int *_errno(void);
IMPORT u64 strlen(const char *text);
IMPORT int strcmp(const char *left, const char *right);
IMPORT int strncmp(const char *left, const char *right, u64 count);
IMPORT long strtol(const char *text, char **end, int base);
IMPORT void *memcpy(void *to, const void *from, u64 count);
IMPORT void *memset(void *to, int value, u64 count);
IMPORT int memcmp(const void *left, const void *right, u64 count);
IMPORT void qsort(void *base, u64 count, u64 size, int (*compare)(const void *, const void *));

// kernel32.dll
IMPORT void *CreateFileW(const wchar *name, u32 access, u32 share, void *security,
                         u32 disposition, u32 flags, void *template_file);
IMPORT int WriteFile(void *file, const void *data, u32 size, u32 *written, void *overlapped);
IMPORT int ReadFile(void *file, void *data, u32 size, u32 *read, void *overlapped);
IMPORT int SetFilePointerEx(void *file, i64 distance, i64 *position, u32 method);
IMPORT int GetFileSizeEx(void *file, i64 *size);
IMPORT int CloseHandle(void *object);
IMPORT u32 GetLastError(void);
IMPORT void ExitProcess(u32 status);
#ifdef GATE_MISSING
// Negative control: a name no service provides.
IMPORT void GateMissingService(void);
#endif

#define INVALID_HANDLE ((void *)-1)
#define GENERIC_READ 0x80000000u
#define GENERIC_WRITE 0x40000000u
#define CREATE_ALWAYS 2
#define OPEN_EXISTING 3
#define ERROR_FILE_NOT_FOUND 2
#define ENOMEM 12

// The status of a run in which every check passed. It is wider than a byte so
// the test can tell that all 32 bits arrive.
#define GATE_A1_STATUS 0x00A1A1A1u
#define RECORDS 64

// The compiler refers to this when a program uses floating point.
int _fltused = 0x9875;

struct record {
    u32 key;
    u32 weight;
    double ratio;
};

// What the host reads from guest memory afterwards, in a section of its own
// so the test can find it by name.
struct gate_result {
    u32 magic;
    u32 checks;
    u64 key_checksum;
    void *kept;
    void *freed;
};
#pragma section(".gate", read, write)
__declspec(allocate(".gate")) struct gate_result gate_result = {0x31415926u, 0, 0, 0, 0};

// Six steps reached through a table of pointers, which gives the image base
// relocations. The table is volatile so that an optimizer keeps it.
static u64 step_multiply(u64 value, u32 round) {
    (void)round;
    return value * 6364136223846793005ULL + 1442695040888963407ULL;
}
static u64 step_rotate(u64 value, u32 round) {
    (void)round;
    return (value << 13) | (value >> 51);
}
static u64 step_divide(u64 value, u32 round) {
    (void)round;
    return value / 7 + value % 1000003;
}
static u64 step_fold(u64 value, u32 round) {
    (void)round;
    return value ^ (value >> 29);
}
static u64 step_signed(u64 value, u32 round) {
    (void)round;
    return (u64)((i64)value / -3);
}
static u64 step_subtract(u64 value, u32 round) { return value - round; }

static u64 (*volatile steps[6])(u64, u32) = {step_multiply, step_rotate, step_divide,
                                             step_fold,     step_signed, step_subtract};

static u64 mix(u64 value, u32 round) { return steps[round % 6](value, round); }

static int by_key(const void *left, const void *right) {
    const struct record *a = left, *b = right;
    return a->key < b->key ? -1 : a->key > b->key;
}

static u32 fibonacci(u32 n) { return n < 2 ? n : fibonacci(n - 1) + fibonacci(n - 2); }

// FNV-1a over the keys, least significant byte first.
static u64 checksum_keys(const struct record *records, u32 count) {
    u64 sum = 0xCBF29CE484222325ULL;
    for (u32 index = 0; index < count; index++)
        for (u32 byte = 0; byte < 4; byte++)
            sum = (sum ^ ((records[index].key >> (8 * byte)) & 0xFF)) * 0x100000001B3ULL;
    return sum;
}

// A small text writer, since no formatted output function is linked.
struct text {
    char *bytes;
    u32 used;
};
static void put_text(struct text *out, const char *text) {
    u64 length = strlen(text);
    memcpy(out->bytes + out->used, text, length);
    out->used += (u32)length;
}
static void put_unsigned(struct text *out, u64 value) {
    char digits[20];
    u32 count = 0;
    do {
        digits[count++] = (char)('0' + value % 10);
        value /= 10;
    } while (value);
    while (count)
        out->bytes[out->used++] = digits[--count];
}
static void put_signed(struct text *out, i64 value) {
    if (value < 0) {
        out->bytes[out->used++] = '-';
        put_unsigned(out, (u64)0 - (u64)value);
    } else {
        put_unsigned(out, (u64)value);
    }
}

// A failed check ends the process with the number of that check.
#define CHECK(condition)                    \
    do {                                    \
        gate_result.checks++;               \
        if (!(condition))                   \
            return gate_result.checks;      \
    } while (0)

static u32 run(void) {
    // Allocation: fill a block, grow it, and keep what it held.
    struct record *records = malloc(RECORDS * sizeof *records);
    CHECK(records != 0);
    u64 state = 0x243F6A8885A308D3ULL;
    for (u32 index = 0; index < RECORDS; index++) {
        state = mix(state, index);
        // The index in the low six bits makes every key different, so the
        // sorted order does not depend on how the sort treats equal keys.
        records[index].key = ((u32)(state >> 20) << 6) | index;
        records[index].weight = index;
        records[index].ratio = (double)(state & 0xFFFF) / 65536.0;
    }
    u32 first_key = records[0].key;
    struct record *grown = realloc(records, 2 * RECORDS * sizeof *records);
    CHECK(grown != 0);
    records = grown;
    CHECK(records[0].key == first_key && records[RECORDS - 1].weight == RECORDS - 1);

    // Computation. Each product below is an exact double, and so is their sum,
    // so the scaled total is an integer the host can recompute.
    double total = 0;
    for (u32 index = 0; index < RECORDS; index++)
        total += records[index].ratio * records[index].weight;
    u64 weighted = (u64)(total * 65536.0);
    qsort(records, RECORDS, sizeof *records, by_key);
    for (u32 index = 1; index < RECORDS; index++)
        CHECK(records[index - 1].key <= records[index].key);
    gate_result.key_checksum = checksum_keys(records, RECORDS);
#ifdef GATE_BREAK
    // Negative control: one expectation is wrong, so this check fails.
    CHECK(fibonacci(12) == 145);
#else
    CHECK(fibonacci(12) == 144);
#endif
    i64 wide = (i64)(state % 1000000007ULL) * -977 / 13;

    // Strings.
    static const char line[] = "gate-a1:0x7b:-42";
    char scratch[32];
    char *end = 0;
    CHECK(strlen(line) == 16);
    CHECK(strcmp(line, "gate-a1:0x7b:-42") == 0 && strcmp("gate", "gave") < 0);
    CHECK(strncmp(line, "gate-b9", 5) == 0 && strncmp(line, "gate-b9", 6) != 0);
    long hex = strtol(line + 8, &end, 16);
    CHECK(hex == 123 && end == line + 12);
    long negative = strtol(line + 13, &end, 10);
    CHECK(negative == -42 && *end == 0);
    memset(scratch, '.', sizeof scratch);
    memcpy(scratch, line, 7);
    CHECK(memcmp(scratch, "gate-a1.", 8) == 0 && scratch[31] == '.');
    // Unequal blocks, in both orders: a memcmp that always says "equal" fails here.
    CHECK(memcmp(scratch, "gate-a2.", 8) < 0 && memcmp("gate-b", scratch, 6) > 0);

    // Errors: a file that does not exist and an allocation that cannot succeed.
    static const wchar none[] = {'C', ':', '\\', 'n', 'o', 'n', 'e', '.', 'b', 'i', 'n', 0};
    CHECK(CreateFileW(none, GENERIC_READ, 0, 0, OPEN_EXISTING, 0, 0) == INVALID_HANDLE);
    u32 missing = GetLastError();
    CHECK(missing == ERROR_FILE_NOT_FOUND);
    *_errno() = 0;
    CHECK(malloc(~0ULL) == 0);
    int exhausted = *_errno();
    CHECK(exhausted == ENOMEM);

    // Files: write the sorted records, then read them back through the same
    // handle and compare.
    static const wchar data_name[] = {'C', ':', '\\', 'g', 'a', 't', 'e', '-', 'a',
                                      '1', '.', 'b', 'i', 'n', 0};
    u32 bytes = RECORDS * (u32)sizeof *records;
    u32 moved = 0;
    i64 size = 0;
    void *file = CreateFileW(data_name, GENERIC_READ | GENERIC_WRITE, 0, 0, CREATE_ALWAYS, 0, 0);
    CHECK(file != INVALID_HANDLE);
    CHECK(WriteFile(file, records, bytes, &moved, 0) && moved == bytes);
    CHECK(GetFileSizeEx(file, &size) && size == bytes);
    CHECK(SetFilePointerEx(file, 0, 0, 0));
    u8 *copy = malloc(bytes);
    CHECK(copy != 0);
    CHECK(ReadFile(file, copy, bytes, &moved, 0) && moved == bytes);
    CHECK(memcmp(copy, records, bytes) == 0);
    // The same comparison without memcmp, and a one-bit difference in the
    // last byte that memcmp has to see.
    u32 same = 0;
    for (u32 i = 0; i < bytes; i++) same += copy[i] == ((const u8 *)records)[i];
    CHECK(same == bytes);
    copy[bytes - 1] ^= 1;
    CHECK(memcmp(copy, records, bytes) != 0);
    copy[bytes - 1] ^= 1;
    CHECK(CloseHandle(file));

    // Memory the host inspects afterwards: one block freed, one kept.
    static const char kept_text[] = "kept by gate a1";
    char *kept = malloc(sizeof kept_text);
    CHECK(kept != 0);
    memcpy(kept, kept_text, sizeof kept_text);
    gate_result.kept = kept;
    gate_result.freed = copy;
    free(copy);
    free(records);

    // The report. Its handle is left open on purpose: ending the process has
    // to close it.
    static const wchar report_name[] = {'C', ':', '\\', 'g', 'a', 't', 'e', '-', 'a',
                                        '1', '.', 't', 'x', 't', 0};
    struct text report = {malloc(256), 0};
    CHECK(report.bytes != 0);
    put_text(&report, "gate a1\nrecords ");
    put_unsigned(&report, RECORDS);
    put_text(&report, "\nkeys ");
    put_unsigned(&report, gate_result.key_checksum);
    put_text(&report, "\nweighted ");
    put_unsigned(&report, weighted);
    put_text(&report, "\nfibonacci ");
    put_unsigned(&report, fibonacci(12));
    put_text(&report, "\nwide ");
    put_signed(&report, wide);
    put_text(&report, "\nstrings ");
    put_unsigned(&report, strlen(line));
    put_text(&report, " ");
    put_signed(&report, hex);
    put_text(&report, " ");
    put_signed(&report, negative);
    put_text(&report, "\nerrors ");
    put_unsigned(&report, missing);
    put_text(&report, " ");
    put_signed(&report, exhausted);
    put_text(&report, "\nfile ");
    put_unsigned(&report, bytes);
    put_text(&report, "\n");
    void *log = CreateFileW(report_name, GENERIC_WRITE, 0, 0, CREATE_ALWAYS, 0, 0);
    CHECK(log != INVALID_HANDLE);
    CHECK(WriteFile(log, report.bytes, report.used, &moved, 0) && moved == report.used);
    return GATE_A1_STATUS;
}

#ifdef GATE_RETURN
// Variant: the entry point returns its status instead of calling ExitProcess.
u32 gate_entry(void) { return run(); }
#else
void gate_entry(void) {
#ifdef GATE_MISSING
    if (!gate_result.magic)
        GateMissingService();
#endif
    ExitProcess(run());
}
#endif
