// SPDX-License-Identifier: GPL-3.0-or-later
//
// Original Xenora dynamic-load fixture provider. This no-CRT DLL exports one
// function and three initialization evidence variables. A TLS callback and
// DllMain write distinct markers so a future LoadLibrary integration can
// prove callback/order behavior without inspecting host-side implementation
// details. Nothing here is derived from any commercial binary.

typedef unsigned int u32;
typedef unsigned long long u64;

__declspec(dllexport) u32 XenoraLibInit;
__declspec(dllexport) u32 XenoraLibTls;
__declspec(dllexport) u32 XenoraLibDetach;

#ifdef XENORA_NO_TRIPLE_EXPORT
#define XENORA_TRIPLE_EXPORT
#else
#define XENORA_TRIPLE_EXPORT __declspec(dllexport)
#endif

XENORA_TRIPLE_EXPORT u32 XenoraTriple(u32 value) {
    return value * 3;
}

/* One thread's TLS template. The loader copies it into thread-owned storage. */
__declspec(allocate(".data")) static unsigned char xenora_tls_template[8] = {
    1, 2, 3, 4, 5, 6, 7, 8,
};
static u32 xenora_tls_index;

static void xenora_tls_callback(void *module, u32 reason, void *reserved) {
    (void)module;
    (void)reserved;
    if (reason == 1) {
        XenoraLibTls = 7;
    }
}

__declspec(allocate(".data")) static void *xenora_tls_callbacks[2] = {
    (void *)xenora_tls_callback,
    0,
};

typedef struct {
    u64 start;
    u64 end;
    u64 index;
    u64 callbacks;
    u32 zero_fill;
    u32 characteristics;
} TlsDirectory;

/* The linker recognises `_tls_used` and emits data directory 9 from it. */
__declspec(allocate(".rdata$T")) const TlsDirectory _tls_used = {
    (u64)xenora_tls_template,
    (u64)xenora_tls_template + sizeof(xenora_tls_template),
    (u64)&xenora_tls_index,
    (u64)xenora_tls_callbacks,
    0,
    0,
};

int DllMain(void *module, u32 reason, void *reserved) {
    (void)module;
    (void)reserved;
    if (reason == 1) {
        /* TLS process-attach callbacks must run before DllMain. */
        XenoraLibInit = XenoraLibTls == 7 ? 3 : 0xBAD;
#ifdef XENORA_ATTACH_FAIL
        return 0;
#endif
    } else if (reason == 0) {
        XenoraLibDetach = 1;
    }
    return 1;
}
