// SPDX-License-Identifier: GPL-3.0-or-later
//
// An original freestanding PE32+ guest that reads controller input only
// through its own XInputGetState calls (Campaign F, #942).
//
// Each frame it asks the host for the next frame number through Xenora's
// frame-tick service, XenoraNextEvent, the same service the persistent guest
// uses. A frame number of zero ends the run. After each tick it polls user
// slots 0 to 3 and prints one line for every slot whose packet number changed
// or that connected or disconnected:
//
//   f<frame> s<slot> p<packet> b<buttons, 4 hex digits> t<lt>,<rt> l<lx>,<ly> r<rx>,<ry>
//   f<frame> s<slot> disconnected
//
// The structure layout is the public XINPUT_STATE one: a 32-bit packet number
// and the 12-byte XINPUT_GAMEPAD.

typedef unsigned int u32;
typedef int i32;
typedef unsigned short u16;
typedef short i16;
typedef unsigned char u8;
typedef unsigned long long u64;

#define IMPORT __declspec(dllimport)

typedef struct {
    u16 buttons;
    u8 left_trigger;
    u8 right_trigger;
    i16 left_x;
    i16 left_y;
    i16 right_x;
    i16 right_y;
} XINPUT_GAMEPAD;

typedef struct {
    u32 packet;
    XINPUT_GAMEPAD pad;
} XINPUT_STATE;

IMPORT u32 XInputGetState(u32 user_index, XINPUT_STATE *state);
IMPORT u32 XenoraNextEvent(void);
IMPORT u64 GetStdHandle(u32 which);
IMPORT int WriteConsoleA(u64 console, const void *text, u32 characters, u32 *written, void *reserved);
IMPORT __declspec(noreturn) void ExitProcess(u32 status);

#define STD_OUTPUT_HANDLE ((u32)-11)
#define ERROR_SUCCESS 0u
#define ERROR_DEVICE_NOT_CONNECTED 1167u

static char line[128];
static u32 length;
static u64 out;

static void put_char(char c) {
    if (length < sizeof line) {
        line[length++] = c;
    }
}

static void put_text(const char *text) {
    while (*text) {
        put_char(*text++);
    }
}

static void put_u32(u32 value) {
    char digits[10];
    u32 count = 0;
    do {
        digits[count++] = (char)('0' + value % 10);
        value /= 10;
    } while (value != 0);
    while (count != 0) {
        put_char(digits[--count]);
    }
}

static void put_i32(i32 value) {
    if (value < 0) {
        put_char('-');
        put_u32((u32)0 - (u32)value);
    } else {
        put_u32((u32)value);
    }
}

static void put_hex16(u16 value) {
    static const char hex[] = "0123456789abcdef";
    for (int shift = 12; shift >= 0; shift -= 4) {
        put_char(hex[(value >> shift) & 0xF]);
    }
}

static void flush_line(void) {
    u32 written = 0;
    put_char('\n');
    WriteConsoleA(out, line, length, &written, 0);
    length = 0;
}

static u32 connected[4];
static u32 last_packet[4];

void entry(void) {
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    for (;;) {
        u32 frame = XenoraNextEvent();
        if (frame == 0) {
            break;
        }
        for (u32 slot = 0; slot < 4; ++slot) {
            XINPUT_STATE state;
            state.packet = 0xFFFFFFFFu;
            state.pad.buttons = 0;
            state.pad.left_trigger = 0;
            state.pad.right_trigger = 0;
            state.pad.left_x = 0;
            state.pad.left_y = 0;
            state.pad.right_x = 0;
            state.pad.right_y = 0;
            u32 result = XInputGetState(slot, &state);
            if (result == ERROR_DEVICE_NOT_CONNECTED) {
                if (connected[slot]) {
                    connected[slot] = 0;
                    put_char('f');
                    put_u32(frame);
                    put_text(" s");
                    put_u32(slot);
                    put_text(" disconnected");
                    flush_line();
                }
                continue;
            }
            if (result != ERROR_SUCCESS) {
                put_text("unexpected XInputGetState result ");
                put_u32(result);
                flush_line();
                ExitProcess(2);
            }
            if (connected[slot] && state.packet == last_packet[slot]) {
                continue;
            }
            connected[slot] = 1;
            last_packet[slot] = state.packet;
            put_char('f');
            put_u32(frame);
            put_text(" s");
            put_u32(slot);
            put_text(" p");
            put_u32(state.packet);
            put_text(" b");
            put_hex16(state.pad.buttons);
            put_text(" t");
            put_u32(state.pad.left_trigger);
            put_char(',');
            put_u32(state.pad.right_trigger);
            put_text(" l");
            put_i32(state.pad.left_x);
            put_char(',');
            put_i32(state.pad.left_y);
            put_text(" r");
            put_i32(state.pad.right_x);
            put_char(',');
            put_i32(state.pad.right_y);
            flush_line();
        }
    }
    put_text("done");
    flush_line();
    ExitProcess(0);
}
