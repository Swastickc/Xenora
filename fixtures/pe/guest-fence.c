// SPDX-License-Identifier: GPL-3.0-or-later

// Original PE32+ guest that orders two graphics command queues through a fence.
typedef unsigned int u32;
typedef unsigned long long u64;

#define IMPORT __declspec(dllimport)
#define S_OK 0u
#define E_INVALIDARG 0x80070057u
#define E_ACCESSDENIED 0x80070005u
#define E_TIMEOUT 0x800705b4u
#define WIDTH 4u
#define HEIGHT 3u
#define FRAME_BYTES (WIDTH * HEIGHT * 4u)
#define CLEAR 0xfb5d2b13u
#define INITIAL 0x100000003ULL
#define READY 0x100000007ULL
#define DONE 0x10000000dULL

IMPORT void __declspec(noreturn) ExitProcess(u32 status);
IMPORT u32 XenoraCreateDevice(u64 *device);
IMPORT u32 XenoraCreateCommandQueue(u64 device, u32 list_type, u64 *queue);
IMPORT u32 XenoraCreateCommandAllocator(u64 device, u32 list_type, u64 *allocator);
IMPORT u32 XenoraCreateCommandList(u64 device, u64 allocator, u32 list_type, u64 *list);
IMPORT u32 XenoraCreateBuffer(u64 device, u64 guest_addr, u64 size, u64 *buffer);
IMPORT u32 XenoraCreateRenderTarget(u64 device, u32 width, u32 height, u64 *target);
IMPORT u32 XenoraSetVertexBuffer(u64 list, u32 slot, u64 buffer, u64 offset, u64 stride);
IMPORT u32 XenoraSetRenderTarget(u64 list, u64 target, u32 clear);
IMPORT u32 XenoraSetViewport(u64 list, u32 width, u32 height);
IMPORT u32 XenoraDraw(u64 list, u32 vertices, u32 instances, u32 start);
IMPORT u32 XenoraCloseCommandList(u64 list);
IMPORT u32 XenoraExecuteCommandLists(u64 queue, u64 list);
IMPORT u32 XenoraReadbackResource(u64 resource, u64 guest_addr, u64 size);
IMPORT u32 XenoraGetLastFrameInfo(u32 *info);
IMPORT u32 XenoraCreateFence(u64 device, u64 initial, u64 *fence);
IMPORT u32 XenoraGetFenceCompletedValue(u64 fence, u64 *value);
IMPORT u32 XenoraQueueWait(u64 queue, u64 fence, u64 value);
IMPORT u32 XenoraQueueSignal(u64 queue, u64 fence, u64 value);
IMPORT u32 XenoraDestroyFence(u64 fence);
IMPORT u32 XenoraDestroyResource(u64 resource);
IMPORT u32 XenoraDestroyCommandList(u64 list);
IMPORT u32 XenoraDestroyCommandAllocator(u64 allocator);
IMPORT u32 XenoraDestroyCommandQueue(u64 queue);
IMPORT u32 XenoraDestroyDevice(u64 device);

#define REQUIRE(condition) do { if (!(condition)) ExitProcess(__LINE__); } while (0)

// These two triangles cover only the leftmost and rightmost pixel columns.
static const float vertices[2][3][4] = {
    {{-1.0f, -10.0f, 0.5f, 1.0f}, {-0.3f, 0.0f, 0.5f, 1.0f}, {-1.0f, 10.0f, 0.5f, 1.0f}},
    {{ 0.5f, -10.0f, 0.5f, 1.0f}, { 1.2f, 0.0f, 0.5f, 1.0f}, { 0.5f, 10.0f, 0.5f, 1.0f}},
};

struct FenceReport {
    u32 magic, producer_backend, consumer_backend, reserved;
    u64 initial, ready, done;
    unsigned char producer[FRAME_BYTES], consumer[FRAME_BYTES];
};

_Static_assert(__builtin_offsetof(struct FenceReport, initial) == 16, "fence values offset");
_Static_assert(__builtin_offsetof(struct FenceReport, producer) == 40, "producer frame offset");
_Static_assert(__builtin_offsetof(struct FenceReport, consumer) == 88, "consumer frame offset");
_Static_assert(sizeof(struct FenceReport) == 136, "fence report size");

#pragma section(".fence", read, write)
__declspec(allocate(".fence")) static struct FenceReport report;

static u64 record_draw(u64 device, u64 allocator, u64 buffer, u64 target, u64 offset) {
    u64 list;
    REQUIRE(XenoraCreateCommandList(device, allocator, 0, &list) == S_OK);
    REQUIRE(XenoraSetVertexBuffer(list, 0, buffer, offset, sizeof(vertices[0][0])) == S_OK);
    REQUIRE(XenoraSetRenderTarget(list, target, CLEAR) == S_OK);
    REQUIRE(XenoraSetViewport(list, WIDTH, HEIGHT) == S_OK);
    REQUIRE(XenoraDraw(list, 3, 1, 0) == S_OK);
    REQUIRE(XenoraCloseCommandList(list) == S_OK);
    return list;
}

static u32 frame_backend(void) {
    u32 info[4];
    REQUIRE(XenoraGetLastFrameInfo(info) == S_OK);
    REQUIRE(info[0] <= 1 && info[1] == WIDTH && info[2] == HEIGHT && info[3] == 1);
    return info[0];
}

void entry(void) {
    u64 device, producer, consumer, allocator, buffer, target, fence, other, other_queue;
    REQUIRE(XenoraCreateDevice(&device) == S_OK);
    REQUIRE(XenoraCreateCommandQueue(device, 0, &producer) == S_OK);
    REQUIRE(XenoraCreateCommandQueue(device, 0, &consumer) == S_OK);
    REQUIRE(XenoraCreateCommandAllocator(device, 0, &allocator) == S_OK);
    REQUIRE(XenoraCreateBuffer(device, (u64)vertices, sizeof(vertices), &buffer) == S_OK);
    REQUIRE(XenoraCreateRenderTarget(device, WIDTH, HEIGHT, &target) == S_OK);

    const u64 left = record_draw(device, allocator, buffer, target, 0);
    const u64 right = record_draw(device, allocator, buffer, target, sizeof(vertices[0]));
    REQUIRE(XenoraCreateFence(device, INITIAL, 0) == E_INVALIDARG);
    REQUIRE(XenoraCreateFence(device, ~0ULL, &fence) == E_INVALIDARG);
    REQUIRE(XenoraCreateFence(device, INITIAL, &fence) == S_OK);
    REQUIRE(XenoraGetFenceCompletedValue(producer, &report.initial) == E_INVALIDARG);
    REQUIRE(XenoraGetFenceCompletedValue(fence, 0) == E_INVALIDARG);
    REQUIRE(XenoraGetFenceCompletedValue(fence, &report.initial) == S_OK);
    REQUIRE(report.initial == INITIAL);
    REQUIRE(XenoraQueueSignal(producer, fence, INITIAL - 1) == E_INVALIDARG);
    REQUIRE(XenoraQueueWait(consumer, fence, ~0ULL) == E_INVALIDARG);
    REQUIRE(XenoraCreateDevice(&other) == S_OK);
    REQUIRE(XenoraCreateCommandQueue(other, 0, &other_queue) == S_OK);
    REQUIRE(XenoraQueueWait(other_queue, fence, READY) == E_INVALIDARG);
    REQUIRE(XenoraQueueSignal(other_queue, fence, READY) == E_INVALIDARG);
    REQUIRE(XenoraDestroyCommandQueue(other_queue) == S_OK);
    REQUIRE(XenoraDestroyDevice(other) == S_OK);

    REQUIRE(XenoraQueueWait(consumer, fence, READY) == S_OK);
    REQUIRE(XenoraExecuteCommandLists(producer, left) == S_OK);
    report.producer_backend = frame_backend();
    REQUIRE(XenoraReadbackResource(target, (u64)report.producer, FRAME_BYTES) == S_OK);

    REQUIRE(XenoraQueueSignal(consumer, fence, READY) == E_TIMEOUT);
    REQUIRE(XenoraExecuteCommandLists(consumer, right) == E_TIMEOUT);
    REQUIRE(XenoraDestroyFence(fence) == E_ACCESSDENIED);
    REQUIRE(XenoraReadbackResource(target, (u64)report.consumer, FRAME_BYTES) == S_OK);
    for (u32 i = 0; i < FRAME_BYTES; i++)
        REQUIRE(report.producer[i] == report.consumer[i]);

    REQUIRE(XenoraQueueSignal(producer, fence, READY) == S_OK);
    REQUIRE(XenoraExecuteCommandLists(consumer, right) == S_OK);
    report.consumer_backend = frame_backend();
    REQUIRE(XenoraReadbackResource(target, (u64)report.consumer, FRAME_BYTES) == S_OK);
    REQUIRE(XenoraGetFenceCompletedValue(fence, &report.ready) == S_OK);
    REQUIRE(report.ready == READY);

    REQUIRE(XenoraQueueSignal(consumer, fence, DONE) == S_OK);
    REQUIRE(XenoraQueueWait(producer, fence, DONE) == S_OK);
    REQUIRE(XenoraQueueSignal(producer, fence, DONE) == S_OK);
    REQUIRE(XenoraGetFenceCompletedValue(fence, &report.done) == S_OK);
    REQUIRE(report.done == DONE);

    REQUIRE(XenoraDestroyFence(fence) == S_OK);
    REQUIRE(XenoraGetFenceCompletedValue(fence, &report.done) == E_INVALIDARG);
    REQUIRE(report.done == DONE);
    REQUIRE(XenoraDestroyCommandList(left) == S_OK);
    REQUIRE(XenoraDestroyCommandList(right) == S_OK);
    REQUIRE(XenoraDestroyResource(target) == S_OK);
    REQUIRE(XenoraDestroyResource(buffer) == S_OK);
    REQUIRE(XenoraDestroyCommandAllocator(allocator) == S_OK);
    REQUIRE(XenoraDestroyCommandQueue(producer) == S_OK);
    REQUIRE(XenoraDestroyCommandQueue(consumer) == S_OK);
    REQUIRE(XenoraDestroyDevice(device) == S_OK);
    report.magic = 0x46454e43u;
    ExitProcess(0);
}
