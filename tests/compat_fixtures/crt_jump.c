// SPDX-License-Identifier: GPL-3.0-or-later
//
// `_setjmp` and `longjmp` through the guest import table (#632). The values
// printed next to each check are the expected ones, stated here so the test
// reads the program and not a run of it.
//
// At -O2 the locals that survive a jump live in the callee-saved registers
// the jump buffer carries, so a correct run is itself the register
// preservation check.

#include <stdio.h>
#include <setjmp.h>

static jmp_buf buf;
static int failures = 0;

static void check(const char *name, long got, long want) {
    printf("crt jump: %s %ld\n", name, got);
    if (got != want) {
        failures++;
    }
}

/* Jumps back with `value` when `fire` is nonzero, answers 0 otherwise. */
static int jumper(int fire, int value) {
    if (fire) {
        longjmp(buf, value);
    }
    return 0;
}

int main(void) {
    volatile int stage = 0;
    int r = setjmp(buf);
    if (r == 0) {
        stage = 1;
        jumper(1, 42);
        check("unreached", 1, 0);
    }
    /* first jump: longjmp(buf, 42) resumes here with 42 */
    check("value", r, 42);
    check("stage", stage, 1);

    /* a zero value becomes one, as documented */
    r = setjmp(buf);
    if (r == 0) {
        longjmp(buf, 0);
    }
    check("zero", r, 1);

    /* no jump: the callee answers, the first return stays the only one */
    r = setjmp(buf);
    if (r == 0) {
        check("calm", jumper(0, 7), 0);
    } else {
        check("calm-unexpected", 1, 0);
    }

    /* counted loop through a jump: five passes, then done */
    {
        volatile int passes = 0;
        r = setjmp(buf);
        passes++;
        if (passes < 5) {
            longjmp(buf, r + 1);
        }
        check("passes", passes, 5);
    }

    printf("crt jump: failures %d\n", failures);
    return failures != 0;
}
