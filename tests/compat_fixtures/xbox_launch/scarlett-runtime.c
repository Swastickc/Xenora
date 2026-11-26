// SPDX-License-Identifier: GPL-3.0-or-later

__declspec(dllimport) long XGameRuntimeInitialize(void);
__declspec(dllimport) void XGameRuntimeUninitialize(void);

int launch_entry(void) {
    long result = XGameRuntimeInitialize();
    if (result < 0) {
        return 1;
    }
    XGameRuntimeUninitialize();
    return 0;
}
