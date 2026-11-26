// SPDX-License-Identifier: GPL-3.0-or-later

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <windows.h>

#define GRAPHICS __declspec(dllimport)
GRAPHICS int XenoraCreateDevice(uint64_t *device);
GRAPHICS int XenoraCreateCommandQueue(uint64_t device, uint32_t type, uint64_t *queue);
GRAPHICS int XenoraCreateCommandAllocator(uint64_t device, uint32_t type, uint64_t *allocator);
GRAPHICS int XenoraCreateCommandList(uint64_t device, uint64_t allocator, uint32_t type, uint64_t *list);
GRAPHICS int XenoraCreateBuffer(uint64_t device, uint64_t address, uint64_t bytes, uint64_t *buffer);
GRAPHICS int XenoraCreateRenderTarget(uint64_t device, uint32_t width, uint32_t height, uint64_t *target);
GRAPHICS int XenoraSetVertexBuffer(uint64_t list, uint32_t slot, uint64_t buffer, uint64_t offset, uint64_t stride);
GRAPHICS int XenoraSetRenderTarget(uint64_t list, uint64_t target, uint32_t clear);
GRAPHICS int XenoraSetViewport(uint64_t list, uint32_t width, uint32_t height);
GRAPHICS int XenoraDraw(uint64_t list, uint32_t vertices, uint32_t instances, uint32_t first);
GRAPHICS int XenoraCloseCommandList(uint64_t list);
GRAPHICS int XenoraExecuteCommandLists(uint64_t queue, uint64_t list);
GRAPHICS int XenoraReadbackResource(uint64_t resource, uint64_t address, uint64_t bytes);
GRAPHICS int XenoraGetLastFrameInfo(uint32_t *info);
GRAPHICS int XenoraDestroyResource(uint64_t resource);
GRAPHICS int XenoraDestroyCommandList(uint64_t list);
GRAPHICS int XenoraDestroyCommandAllocator(uint64_t allocator);
GRAPHICS int XenoraDestroyCommandQueue(uint64_t queue);
GRAPHICS int XenoraDestroyDevice(uint64_t device);

enum {
    WIDTH = 8,
    HEIGHT = 8,
    FRAME_BYTES = WIDTH * HEIGHT * 4,
    DIRECT = 0,
    FRAME_COUNT = 2,
    MISSING_INPUT = 61,
    SCHEDULER_REQUIRED = 62,
    UNEXPECTED_THREAD_ERROR = 63,
};
#define WORKER_OK UINT32_C(0x41c0ffee)
#define CLEAR_COLOR UINT32_C(0xff1e140a)

struct HostedReport {
    uint32_t magic;
    uint32_t main_id;
    uint32_t assigned_worker_id;
    uint32_t worker_id;
    uint32_t join_result;
    uint32_t worker_status;
    uint32_t info[FRAME_COUNT][4];
    unsigned char rgba[FRAME_COUNT][FRAME_BYTES];
};

_Static_assert(offsetof(struct HostedReport, info) == 24, "frame info offset");
_Static_assert(offsetof(struct HostedReport, rgba) == 56, "frame bytes offset");
_Static_assert(sizeof(struct HostedReport) == 568, "report size");

__attribute__((section(".hostgfx"), used))
static struct HostedReport report;

// The first scene covers the viewport. The second lies entirely beyond the
// right clip plane, so the render target retains its clear color.
static const float geometry[FRAME_COUNT][3][4] = {
    {{-1.0f, -1.0f, 0.0f, 1.0f}, {4.0f, -1.0f, 0.0f, 1.0f}, {-1.0f, 4.0f, 0.0f, 1.0f}},
    {{2.0f, -0.5f, 0.0f, 1.0f}, {4.0f, -0.5f, 0.0f, 1.0f}, {2.0f, 0.5f, 0.0f, 1.0f}},
};
static const unsigned char scene_pixels[FRAME_COUNT][4] = {
    {255, 0, 0, 255},
    {10, 20, 30, 255},
};

#define CHECK(condition) do { if (!(condition)) return __LINE__; } while (0)

static int read_scene(FILE *scene) {
    char record[16];
    if (!fgets(record, sizeof record, scene))
        return -1;
    if (strcmp(record, "inside\n") == 0)
        return 0;
    if (strcmp(record, "outside\n") == 0)
        return 1;
    return -1;
}

static DWORD WINAPI render_scenes(void *argument) {
    report.worker_id = GetCurrentThreadId();
    FILE *scene = fopen(argument, "r");
    if (scene == NULL)
        return MISSING_INPUT;

    int scenes[FRAME_COUNT];
    for (unsigned i = 0; i < FRAME_COUNT; ++i)
        scenes[i] = read_scene(scene);
    char extra[16];
    int exhausted = fgets(extra, sizeof extra, scene) == NULL;
    CHECK(fclose(scene) == 0);
    CHECK(exhausted && scenes[0] >= 0 && scenes[1] >= 0);

    uint64_t device, queue, allocator, buffer, target;
    uint64_t lists[FRAME_COUNT];
    CHECK(XenoraCreateDevice(&device) == S_OK);
    CHECK(XenoraCreateCommandQueue(device, DIRECT, &queue) == S_OK);
    CHECK(XenoraCreateCommandAllocator(device, DIRECT, &allocator) == S_OK);
    CHECK(XenoraCreateBuffer(device, (uintptr_t)geometry, sizeof geometry, &buffer) == S_OK);
    CHECK(XenoraCreateRenderTarget(device, WIDTH, HEIGHT, &target) == S_OK);

    for (unsigned i = 0; i < FRAME_COUNT; ++i) {
        const uint64_t offset = (unsigned)scenes[i] * sizeof geometry[0];
        CHECK(XenoraCreateCommandList(device, allocator, DIRECT, &lists[i]) == S_OK);
        CHECK(XenoraSetRenderTarget(lists[i], target, CLEAR_COLOR) == S_OK);
        CHECK(XenoraSetViewport(lists[i], WIDTH, HEIGHT) == S_OK);
        CHECK(XenoraSetVertexBuffer(lists[i], 0, buffer, offset, sizeof geometry[0][0]) == S_OK);
        CHECK(XenoraDraw(lists[i], 3, 1, 0) == S_OK);
        CHECK(XenoraCloseCommandList(lists[i]) == S_OK);
        CHECK(XenoraExecuteCommandLists(queue, lists[i]) == S_OK);
        CHECK(XenoraReadbackResource(target, (uintptr_t)report.rgba[i], FRAME_BYTES) == S_OK);
        CHECK(XenoraGetLastFrameInfo(report.info[i]) == S_OK);
        CHECK(report.info[i][1] == WIDTH && report.info[i][2] == HEIGHT && report.info[i][3] == 1);
        for (unsigned pixel = 0; pixel < WIDTH * HEIGHT; ++pixel) {
            CHECK(memcmp(&report.rgba[i][4 * pixel], scene_pixels[scenes[i]], 4) == 0);
        }
    }

    for (unsigned i = 0; i < FRAME_COUNT; ++i)
        CHECK(XenoraDestroyCommandList(lists[i]) == S_OK);
    CHECK(XenoraDestroyResource(target) == S_OK);
    CHECK(XenoraDestroyResource(buffer) == S_OK);
    CHECK(XenoraDestroyCommandAllocator(allocator) == S_OK);
    CHECK(XenoraDestroyCommandQueue(queue) == S_OK);
    CHECK(XenoraDestroyDevice(device) == S_OK);
    return WORKER_OK;
}

int main(int argc, char **argv) {
    report.magic = UINT32_C(0x48474658);
    report.main_id = GetCurrentThreadId();
    CHECK(argc == 2 || argc == 3);
    const char *records = "inside\noutside\n";
    if (argc == 3) {
        if (strcmp(argv[2], "missing") == 0)
            records = NULL;
        else {
            CHECK(strcmp(argv[2], "reverse") == 0);
            records = "outside\ninside\n";
        }
    }
    if (records != NULL) {
        FILE *scene = fopen(argv[1], "w");
        CHECK(scene != NULL);
        int written = fputs(records, scene);
        CHECK(fclose(scene) == 0 && written >= 0);
    }

    DWORD assigned = 0;
    HANDLE worker = CreateThread(NULL, 0, render_scenes, argv[1], 0, &assigned);
    if (worker == NULL)
        return GetLastError() == ERROR_NOT_SUPPORTED ? SCHEDULER_REQUIRED : UNEXPECTED_THREAD_ERROR;
    report.assigned_worker_id = assigned;
    report.join_result = WaitForSingleObject(worker, 1000);
    DWORD status = 0;
    int read_status = report.join_result == WAIT_OBJECT_0 && GetExitCodeThread(worker, &status);
    int closed = CloseHandle(worker);
    CHECK(read_status && closed);
    report.worker_status = status;
    CHECK(report.main_id != 0 && assigned != 0 && assigned != report.main_id && assigned == report.worker_id);
    if (status != WORKER_OK)
        return (int)status;
    CHECK(fputs("hosted graphics: two frames\n", stdout) >= 0);
    return 0;
}
