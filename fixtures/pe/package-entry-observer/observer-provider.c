// SPDX-License-Identifier: GPL-3.0-or-later

typedef unsigned int u32;
__declspec(dllimport) void PhaseProbe(u32 marker);

static u32 attached;

int DllMain(void *instance, u32 reason, void *reserved) {
    (void)instance;
    (void)reserved;
    if (reason == 1) {
        PhaseProbe(1);
#ifdef REJECT_ATTACH
        return 0;
#else
        attached = 7;
#endif
    } else if (reason == 0) {
        PhaseProbe(3);
        attached = 0;
    }
    return 1;
}

__declspec(dllexport) u32 ProviderValue(void) {
    return attached;
}
