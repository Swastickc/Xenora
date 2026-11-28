// SPDX-License-Identifier: GPL-3.0-or-later
//
// Hosted checks for the legacy Windows memory services (package 9, first
// part, of #459). Built with mingw-w64, every call is an import of kernel32.
// A wrong check prints its number and the program exits with it. With
// CRT_WINMEMORY_BREAK one expectation is wrong on purpose.

#include <stdio.h>
#include <string.h>
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

static BOOL private_heap_without_serialization(void) {
    HANDLE heap = HeapCreate(HEAP_NO_SERIALIZE, 0, 0);
    if (!heap)
        return FALSE;
    unsigned char *block = HeapAlloc(heap, HEAP_ZERO_MEMORY, 8);
    BOOL valid = FALSE;
    if (block) {
        block[0] = 0x5a;
        unsigned char *grown = HeapReAlloc(heap, HEAP_NO_SERIALIZE | HEAP_ZERO_MEMORY, block, 16);
        if (grown) {
            valid = grown[0] == 0x5a && grown[15] == 0
                    && HeapSize(heap, HEAP_NO_SERIALIZE, grown) == 16;
            valid = HeapFree(heap, HEAP_NO_SERIALIZE, grown) && valid;
        }
    }
    return HeapDestroy(heap) && valid;
}

int main(void) {
    // 1. A fixed allocation is a writable pointer into zeroed memory.
    HGLOBAL fixed = GlobalAlloc(GMEM_FIXED, 64);
    CHECK("fixed nonzero", fixed != NULL);
    CHECK("fixed zeroed", *(unsigned *)fixed == 0);
    memset(fixed, 0x5a, 64);
    CHECK("fixed writable", ((unsigned char *)fixed)[63] == 0x5a);
    CHECK("fixed freed", GlobalFree(fixed) == NULL);

    // 2. The zero-initialization form, the moveable bit and the
    // discardable hint are accepted; the handle is still the pointer.
    HGLOBAL moveable = GlobalAlloc(GHND | GMEM_DISCARDABLE, 32);
    CHECK("moveable nonzero", moveable != NULL);
    CHECK("moveable zeroed", ((unsigned char *)moveable)[31] == 0);
    CHECK("moveable freed", GlobalFree(moveable) == NULL);

    // 3. The local forms behave identically.
    HLOCAL local = LocalAlloc(LPTR, 16);
    CHECK("local nonzero", local != NULL);
    CHECK("local zeroed", *(unsigned long long *)local == 0);
    CHECK("local freed", LocalFree(local) == NULL);

    // 4. Freeing the null handle is a documented no-op success.
    CHECK("null free", GlobalFree(NULL) == NULL && LocalFree(NULL) == NULL);

    // 5. The failures are the documented ones: freeing a non-allocation
    // answers the handle itself with the invalid-handle error.
    SetLastError(0);
    char bogus[8];
    HGLOBAL answer = GlobalFree((HGLOBAL)(void *)bogus);
    CHECK("bogus free answers handle",
          answer == (HGLOBAL)(void *)bogus && GetLastError() == ERROR_INVALID_HANDLE);

    // 6. A double free fails the same way.
    HGLOBAL once = GlobalAlloc(GPTR, 16);
    CHECK("once allocated", once != NULL);
    CHECK("once freed", GlobalFree(once) == NULL);
    SetLastError(0);
    CHECK("double free fails",
          GlobalFree(once) == once && GetLastError() == ERROR_INVALID_HANDLE);

    // 7. The legacy services share the process heap with HeapAlloc.
    HANDLE heap = GetProcessHeap();
    void *direct = HeapAlloc(heap, 0, 24);
    HGLOBAL legacy = GlobalAlloc(GPTR, 24);
    CHECK("legacy memory shares the process heap and a private heap supports HEAP_NO_SERIALIZE",
          direct != NULL && legacy != NULL && direct != (void *)legacy
              && private_heap_without_serialization());
    HeapFree(heap, 0, direct);
    GlobalFree(legacy);

#ifdef CRT_WINMEMORY_BREAK
    CHECK("the control fails", fixed == NULL);
#endif

    if (failures) {
        printf("crt-winmemory: %u/%u checks failed, first %u\n", failures, checks, failed);
        return failed;
    }
    printf("crt-winmemory: %u/%u checks passed\n", checks, checks);
    return 0;
}
