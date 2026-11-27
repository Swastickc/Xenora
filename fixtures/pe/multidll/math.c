// SPDX-License-Identifier: GPL-3.0-or-later
//
// Original Xenora fixture. A leaf provider DLL with no CRT dependency: one
// exported function, one exported variable with a nonzero initial value, and
// one exported initialization marker written by DllMain. Nothing here is
// derived from any commercial binary.

__declspec(dllexport) unsigned int XenoraMathInit;
__declspec(dllexport) unsigned int XenoraScale = 5;

__declspec(dllexport) unsigned int XenoraAdd(unsigned int a, unsigned int b) {
    return a + b;
}

int DllMain(void *module, unsigned int reason, void *reserved) {
    (void)module;
    (void)reserved;
    if (reason == 1) {
        XenoraMathInit = 1;
    }
    return 1;
}
