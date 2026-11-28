// SPDX-License-Identifier: GPL-3.0-or-later

// Original no-CRT program that draws and exports two verified package frames.
typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned int u32;
typedef unsigned long long u64;

#define IMPORT __declspec(dllimport)
#define WIDTH 4u
#define HEIGHT 3u
#define RGBA_SIZE (WIDTH * HEIGHT * 4u)
#define CLEAR_RGBA 0xff1e140au
#define GENERIC_WRITE 0x40000000u
#define CREATE_ALWAYS 2u
#define WAIT_OBJECT_0 0u
#define WAIT_TIMEOUT 258u

IMPORT const float *SceneVertices(u32 frame);
IMPORT u64 CreateFileW(const u16 *path, u32 access, u32 share, void *security,
                       u32 disposition, u32 flags, u64 template);
IMPORT int WriteFile(u64 file, const void *bytes, u32 size, u32 *written, void *overlapped);
IMPORT int SetFilePointerEx(u64 file, long long distance, u64 *position, u32 origin);
IMPORT u64 CreateEventW(void *security, int manual_reset, int initial, const u16 *name);
IMPORT int SetEvent(u64 event);
IMPORT u32 WaitForSingleObject(u64 handle, u32 milliseconds);
IMPORT int CloseHandle(u64 handle);
IMPORT __declspec(noreturn) void ExitProcess(u32 status);

IMPORT int XenoraCreateDevice(u64 *out);
IMPORT int XenoraCreateCommandQueue(u64 device, u32 type, u64 *out);
IMPORT int XenoraCreateCommandAllocator(u64 device, u32 type, u64 *out);
IMPORT int XenoraCreateCommandList(u64 device, u64 allocator, u32 type, u64 *out);
IMPORT int XenoraCreateBuffer(u64 device, u64 address, u64 size, u64 *out);
IMPORT int XenoraCreateRenderTarget(u64 device, u32 width, u32 height, u64 *out);
IMPORT int XenoraSetVertexBuffer(u64 list, u32 slot, u64 buffer, u64 offset, u64 stride);
IMPORT int XenoraSetRenderTarget(u64 list, u64 resource, u32 clear);
IMPORT int XenoraSetViewport(u64 list, u32 width, u32 height);
IMPORT int XenoraDraw(u64 list, u32 vertices, u32 instances, u32 first_vertex);
IMPORT int XenoraCloseCommandList(u64 list);
IMPORT int XenoraExecuteCommandLists(u64 queue, u64 list);
IMPORT int XenoraReadbackResource(u64 resource, u64 address, u64 size);
IMPORT int XenoraDestroyResource(u64 resource);
IMPORT int XenoraDestroyCommandList(u64 list);
IMPORT int XenoraDestroyCommandAllocator(u64 allocator);
IMPORT int XenoraDestroyCommandQueue(u64 queue);
IMPORT int XenoraDestroyDevice(u64 device);

static const u16 output_path[] = {
    'C', ':', '\\', 'p', 'a', 'c', 'k', 'a', 'g', 'e', '-', 'f', 'r', 'a', 'm', 'e', 's',
    '.', 'b', 'i', 'n', 0,
};

static const u8 expected_pixels[2][4] = {
    {255, 0, 0, 255},
    {10, 20, 30, 255},
};

static void require(int condition, u32 status) {
    if (!condition)
        ExitProcess(status);
}

void entry(void) {
    u64 file = CreateFileW(output_path, GENERIC_WRITE, 0, 0, CREATE_ALWAYS, 0, 0);
    require(file != 0 && file != ~0ull, 1);
    u64 ready = CreateEventW(0, 0, 0, 0);
    require(ready != 0 && WaitForSingleObject(ready, 0) == WAIT_TIMEOUT, 2);

    u64 device, queue, allocator, target;
    require(XenoraCreateDevice(&device) == 0, 3);
    require(XenoraCreateCommandQueue(device, 0, &queue) == 0, 3);
    require(XenoraCreateCommandAllocator(device, 0, &allocator) == 0, 3);
    require(XenoraCreateRenderTarget(device, WIDTH, HEIGHT, &target) == 0, 3);

    for (u32 frame = 0; frame < 2; frame++) {
        const float *vertices = SceneVertices(frame);
        require(vertices != 0, 4);

        u64 buffer, list;
        require(XenoraCreateBuffer(device, (u64)vertices, 12 * sizeof(float), &buffer) == 0, 5);
        require(XenoraCreateCommandList(device, allocator, 0, &list) == 0, 5);
        require(XenoraSetVertexBuffer(list, 0, buffer, 0, 4 * sizeof(float)) == 0, 5);
        require(XenoraSetRenderTarget(list, target, CLEAR_RGBA) == 0, 5);
        require(XenoraSetViewport(list, WIDTH, HEIGHT) == 0, 5);
        require(XenoraDraw(list, 3, 1, 0) == 0, 5);
        require(XenoraCloseCommandList(list) == 0, 5);
        require(XenoraExecuteCommandLists(queue, list) == 0, 5);

        require(SetEvent(ready) && WaitForSingleObject(ready, 0) == WAIT_OBJECT_0 &&
                    WaitForSingleObject(ready, 0) == WAIT_TIMEOUT,
                6);

        u8 rgba[RGBA_SIZE];
        require(XenoraReadbackResource(target, (u64)rgba, sizeof(rgba)) == 0, 7);
        for (u32 byte = 0; byte < sizeof(rgba); byte++)
            require(rgba[byte] == expected_pixels[frame][byte % 4], 7);
        u32 written;
        require(WriteFile(file, rgba, sizeof(rgba), &written, 0) && written == sizeof(rgba), 8);

        require(XenoraDestroyCommandList(list) == 0, 9);
        require(XenoraDestroyResource(buffer) == 0, 9);
    }

    u64 end;
    require(SetFilePointerEx(file, 0, &end, 2) && end == 2 * RGBA_SIZE, 8);
    require(CloseHandle(file) && CloseHandle(ready), 9);
    require(XenoraDestroyResource(target) == 0, 9);
    require(XenoraDestroyCommandAllocator(allocator) == 0, 9);
    require(XenoraDestroyCommandQueue(queue) == 0, 9);
    require(XenoraDestroyDevice(device) == 0, 9);
    ExitProcess(0);
}
