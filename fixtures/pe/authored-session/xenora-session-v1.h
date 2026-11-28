// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef XENORA_SESSION_V1_H
#define XENORA_SESSION_V1_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Import this interface from xenora-session-v1.dll on Win64. */
__declspec(dllimport) int XenoraSessionNextInput(uint32_t *out);
__declspec(dllimport) int XenoraSessionPresent(void);
__declspec(dllimport) int XenoraSessionFinish(void);

#ifdef __cplusplus
}
#endif

#endif
