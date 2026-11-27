// SPDX-License-Identifier: GPL-3.0-or-later
//
// Original Xenora dynamic-load fixture consumer. This no-CRT EXE imports only
// kernel32 loader services, loads the authorized logical module by name,
// resolves the function and evidence exports through GetProcAddress, calls the
// function indirectly and exits with 70 only when every observed value is the
// independently expected one. Distinct small statuses identify failed steps.

typedef unsigned int u32;
typedef unsigned long long u64;

__declspec(dllimport) void *LoadLibraryA(const char *name);
__declspec(dllimport) void *GetProcAddress(void *module, const char *name);
__declspec(dllimport) int FreeLibrary(void *module);
__declspec(dllimport) void ExitProcess(u32 status);

typedef u32 (*XenoraTripleFn)(u32);

static XenoraTripleFn as_function(void *address) {
    union {
        void *object;
        XenoraTripleFn function;
    } convert;
    convert.object = address;
    return convert.function;
}

void entry(void) {
    void *module = LoadLibraryA("xenora_loadlib.dll");
    if (module == 0) {
        ExitProcess(21);
    }

    XenoraTripleFn triple = as_function(GetProcAddress(module, "XenoraTriple"));
    if (triple == (XenoraTripleFn)0) {
        ExitProcess(22);
    }
    u32 *init = (u32 *)GetProcAddress(module, "XenoraLibInit");
    if (init == (u32 *)0) {
        ExitProcess(23);
    }
    u32 *tls = (u32 *)GetProcAddress(module, "XenoraLibTls");
    if (tls == (u32 *)0) {
        ExitProcess(24);
    }
    u32 *detach = (u32 *)GetProcAddress(module, "XenoraLibDetach");
    if (detach == (u32 *)0) {
        ExitProcess(25);
    }

    /* 60 + 3 + 7 + 0 = 70. Detach must still be zero before FreeLibrary. */
    u32 checksum = triple(20) + *init + *tls + *detach;
    if (checksum != 70) {
        ExitProcess(26);
    }
    if (!FreeLibrary(module)) {
        ExitProcess(27);
    }
    ExitProcess(checksum);
}
