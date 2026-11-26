// SPDX-License-Identifier: GPL-3.0-or-later

// Original PE32+ guest for the macOS fixture viewer's frame loop.
// The renderer is host-backed, but the guest owns every graphics command.

typedef unsigned int u32;
typedef unsigned long long u64;

#define IMPORT __declspec(dllimport)
#define S_OK 0
#define WIDTH 4u
#define HEIGHT 3u
#define CLEAR 0xfb5d2b13u

IMPORT u32 frame_begin(u32 sequence);
IMPORT u32 input_poll(void);
IMPORT u32 frame_present(u32 sequence, u32 column, u64 guest_readback_address);

IMPORT int XenoraCreateDevice(u64 *device);
IMPORT int XenoraCreateCommandQueue(u64 device, u32 list_type, u64 *queue);
IMPORT int XenoraCreateCommandAllocator(u64 device, u32 list_type, u64 *allocator);
IMPORT int XenoraCreateCommandList(u64 device, u64 allocator, u32 list_type, u64 *list);
IMPORT int XenoraCreateBuffer(u64 device, u64 guest_addr, u64 size, u64 *buffer);
IMPORT int XenoraCreateRenderTarget(u64 device, u32 width, u32 height, u64 *target);
IMPORT int XenoraSetVertexBuffer(u64 list, u32 slot, u64 buffer, u64 offset, u64 stride);
IMPORT int XenoraSetRenderTarget(u64 list, u64 target, u32 clear);
IMPORT int XenoraSetViewport(u64 list, u32 width, u32 height);
IMPORT int XenoraDraw(u64 list, u32 vertices, u32 instances, u32 start);
IMPORT int XenoraCloseCommandList(u64 list);
IMPORT int XenoraExecuteCommandLists(u64 queue, u64 list);
IMPORT int XenoraReadbackResource(u64 resource, u64 guest_addr, u64 size);
IMPORT int XenoraDestroyResource(u64 resource);
IMPORT int XenoraDestroyCommandList(u64 list);
IMPORT int XenoraDestroyCommandAllocator(u64 allocator);
IMPORT int XenoraDestroyCommandQueue(u64 queue);
IMPORT int XenoraDestroyDevice(u64 device);
IMPORT __declspec(noreturn) void ExitProcess(u32 status);

// The triangle for column x starts at NDC x = -1 + x/2 and covers only
// that column's pixel centres in the three framebuffer rows.
static const float vertices[WIDTH][3][4] = {
    {{-1.0f, -10.0f, 0.5f, 1.0f}, {-0.3f, 0.0f, 0.5f, 1.0f}, {-1.0f, 10.0f, 0.5f, 1.0f}},
    {{-0.5f, -10.0f, 0.5f, 1.0f}, { 0.2f, 0.0f, 0.5f, 1.0f}, {-0.5f, 10.0f, 0.5f, 1.0f}},
    {{ 0.0f, -10.0f, 0.5f, 1.0f}, { 0.7f, 0.0f, 0.5f, 1.0f}, { 0.0f, 10.0f, 0.5f, 1.0f}},
    {{ 0.5f, -10.0f, 0.5f, 1.0f}, { 1.2f, 0.0f, 0.5f, 1.0f}, { 0.5f, 10.0f, 0.5f, 1.0f}},
};

static unsigned char frame[WIDTH * HEIGHT * 4];

static void require(int condition, u32 failure) {
    if (!condition)
        ExitProcess(failure);
}

void entry(void) {
    u64 device, queue, allocator, buffer, target, lists[WIDTH];
    require(XenoraCreateDevice(&device) == S_OK, 1);
    require(XenoraCreateCommandQueue(device, 0, &queue) == S_OK, 1);
    require(XenoraCreateCommandAllocator(device, 0, &allocator) == S_OK, 1);
    require(XenoraCreateBuffer(device, (u64)vertices, sizeof(vertices), &buffer) == S_OK, 1);
    require(XenoraCreateRenderTarget(device, WIDTH, HEIGHT, &target) == S_OK, 1);

    for (u32 column = 0; column < WIDTH; column++) {
        u64 list;
        require(XenoraCreateCommandList(device, allocator, 0, &list) == S_OK, 2);
        require(XenoraSetVertexBuffer(list, 0, buffer, column * sizeof(vertices[0]),
                                      sizeof(vertices[0][0])) == S_OK, 2);
        require(XenoraSetRenderTarget(list, target, CLEAR) == S_OK, 2);
        require(XenoraSetViewport(list, WIDTH, HEIGHT) == S_OK, 2);
        require(XenoraDraw(list, 3, 1, 0) == S_OK, 2);
        require(XenoraCloseCommandList(list) == S_OK, 2);
        lists[column] = list;
    }

    u32 interactive_column = 0;
    for (u32 sequence = 0; ; sequence++) {
        u32 mode = frame_begin(sequence);
        if (mode == 0)
            break;
        require(mode == 1 || mode == 2, 3);
        u32 column = sequence % WIDTH;
        if (mode == 2) {
            u32 input = input_poll();
            require(input <= 2, 3);
            if (input == 1)
                interactive_column = (interactive_column + 1) % WIDTH;
            else if (input == 2)
                interactive_column = (interactive_column + WIDTH - 1) % WIDTH;
            column = interactive_column;
        }

        require(XenoraExecuteCommandLists(queue, lists[column]) == S_OK, 4);
        require(XenoraReadbackResource(target, (u64)frame, sizeof(frame)) == S_OK, 4);
        require(frame_present(sequence, column, (u64)frame) == 1, 5);
    }

    for (u32 column = 0; column < WIDTH; column++)
        require(XenoraDestroyCommandList(lists[column]) == S_OK, 6);
    require(XenoraDestroyResource(target) == S_OK, 6);
    require(XenoraDestroyResource(buffer) == S_OK, 6);
    require(XenoraDestroyCommandAllocator(allocator) == S_OK, 6);
    require(XenoraDestroyCommandQueue(queue) == S_OK, 6);
    require(XenoraDestroyDevice(device) == S_OK, 6);
    ExitProcess(0);
}
