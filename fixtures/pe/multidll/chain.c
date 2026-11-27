// SPDX-License-Identifier: GPL-3.0-or-later
//
// Original Xenora fixture. A middle provider DLL that imports a function from
// the leaf provider, so startup must bind a transitive package dependency and
// initialize the leaf first. DllMain refuses to mark itself initialized unless
// the leaf already has.

__declspec(dllimport) unsigned int XenoraAdd(unsigned int a, unsigned int b);
__declspec(dllimport) unsigned int XenoraMathInit;

__declspec(dllexport) unsigned int XenoraChainInit;
__declspec(dllexport) unsigned int XenoraChainDetach;

__declspec(dllexport) unsigned int XenoraQuad(unsigned int value) {
    return XenoraAdd(XenoraAdd(value, value), XenoraAdd(value, value));
}

int DllMain(void *module, unsigned int reason, void *reserved) {
    (void)module;
    (void)reserved;
    if (reason == 1) {
        XenoraChainInit = XenoraMathInit == 1 ? 2 : 0xBAD;
    } else if (reason == 0) {
        XenoraChainDetach = 1;
    }
    return 1;
}
