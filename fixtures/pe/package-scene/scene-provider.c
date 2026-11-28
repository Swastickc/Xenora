// SPDX-License-Identifier: GPL-3.0-or-later

// Original no-CRT provider for the packaged graphics fixture.
typedef unsigned int u32;
typedef unsigned long long u64;

struct Scene {
    float position[2][3][4];
};

static const struct Scene scene = {
    {
        {{-1.0f, -1.0f, 0.5f, 1.0f}, {3.0f, -1.0f, 0.5f, 1.0f}, {-1.0f, 3.0f, 0.5f, 1.0f}},
        {{2.0f, -0.5f, 0.5f, 1.0f}, {4.0f, -0.5f, 0.5f, 1.0f}, {2.0f, 0.5f, 0.5f, 1.0f}},
    },
};

// The PE contains a DIR64 relocation for this initialized address. The
// volatile read prevents folding it into a RIP-relative reference to scene.
static const struct Scene *volatile scene_address = &scene;
static u32 attached;

#ifndef REJECT_ATTACH
#define REJECT_ATTACH 0
#endif

int DllMain(void *module, u32 reason, void *reserved) {
    (void)reserved;
    if (reason == 0) {
        attached = 0;
        return 1;
    }
    if (reason != 1)
        return 1;
    if (scene_address != &scene || (u64)module == 0x140000000ull)
        return 0;
    if (REJECT_ATTACH)
        return 0;
    attached = 1;
    return 1;
}

__declspec(dllexport) const float *SceneVertices(u32 frame) {
    if (!attached || frame >= 2)
        return 0;
    return scene_address->position[frame][0];
}
