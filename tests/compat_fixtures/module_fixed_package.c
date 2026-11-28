// SPDX-License-Identifier: GPL-3.0-or-later
// The root and provider deliberately have the same linker image base.

#ifdef FIXED_PROVIDER
static unsigned int attached_value;

int DllMain(void *module, unsigned int reason, void *reserved) {
    (void)module;
    (void)reserved;
    if (reason == 1)
        attached_value = 17;
    return 1;
}

__declspec(dllexport) unsigned int FixedProviderValue(void) {
    return attached_value;
}
#else
__declspec(dllimport) unsigned int FixedProviderValue(void);
__declspec(dllimport) __declspec(noreturn) void ExitProcess(unsigned int);

static unsigned int local_value(void) {
    return 25;
}

// The root needs a DIR64 relocation before the indirect call reaches its code.
static unsigned int (*volatile local_target)(void) = local_value;

void entry(void) {
    ExitProcess(local_target() + FixedProviderValue());
}
#endif
