// SPDX-License-Identifier: GPL-3.0-or-later
//
// Original no-CRT C++ fixture for the array allocation operators (issue #936).
// The runtime-size new[] expressions make the compiler emit the public Itanium
// ABI imports _Znay (operator new[]) and _ZdaPv (operator delete[]). Every
// outside function is imported by name from libstdc++-6.dll or kernel32.dll.
//
// Built by tests/compat_fixtures/build-cxx-array.sh. The program reports
// through a .gate section. Normal exit status 0 means every check passed;
// statuses 1..3 identify the failed check.

typedef unsigned int u32;
typedef unsigned long long u64;

extern "C" __declspec(dllimport) void ExitProcess(u32 status);

void* operator new[](u64 size);
void operator delete[](void* address) noexcept;

#define MAGIC 0x6378616eu // "cxan"

struct ResultBlock {
    volatile u32 magic;
    volatile u32 first_value;
    volatile u32 second_value;
    volatile u32 distinct;
    volatile u32 zero_nonzero;
    volatile u32 zero_distinct;
    volatile u32 reserved;
    volatile u64 first;
    volatile u64 second;
    volatile u64 zero_first;
    volatile u64 zero_second;
};

#pragma section(".gate", read, write)
__declspec(allocate(".gate")) ResultBlock result = {
    MAGIC, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0};

extern "C" void guest_cxx_array_entry() {
    // A runtime count keeps the allocation calls in the object at every
    // optimization level. No destructor is needed, so no array cookie is used.
    const u32 count = 4;
    int* first = new int[count];
    int* second = new int[count];
    result.first = reinterpret_cast<u64>(first);
    result.second = reinterpret_cast<u64>(second);
    result.distinct = static_cast<u32>(first != second);
    if (first == 0 || second == 0 || first == second)
        ExitProcess(1);

    for (u32 index = 0; index < count; ++index) {
        first[index] = 0x11u + index;
        second[index] = 0x21u + index;
    }
    result.first_value = first[3];
    result.second_value = second[3];
    if (result.first_value != 0x14u || result.second_value != 0x24u)
        ExitProcess(2);

    delete[] first;
    delete[] second;

    // A zero-size array still yields a distinct, freeable block in Xenora's
    // shared guest heap, matching the existing scalar operator choice.
    int* zero_first = new int[0];
    int* zero_second = new int[0];
    result.zero_nonzero = static_cast<u32>(zero_first != 0 && zero_second != 0);
    result.zero_distinct = static_cast<u32>(zero_first != zero_second);
    if (!result.zero_nonzero || !result.zero_distinct)
        ExitProcess(3);
    result.zero_first = reinterpret_cast<u64>(zero_first);
    result.zero_second = reinterpret_cast<u64>(zero_second);

    delete[] zero_first;
    delete[] zero_second;
    ExitProcess(0);
}
