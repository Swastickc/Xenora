#include <metal_stdlib>
using namespace metal;
kernel void xenora_compute(
    device const uint* input [[buffer(0)]],
    device uint* output [[buffer(1)]],
    uint index [[thread_position_in_grid]],
    uint grid [[threads_per_grid]]) {
    uint t0 = input[index];
    uint t1 = 0x00000000u;
    uint t2 = 0x00000001u;
    uint t3 = 0x40000001u;
    uint t4 = 0x00000001u;
    uint t5 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint pass = (false) ? 1u : 0u;
        t5 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t6 = t1 << (t2 & 31u);
    uint t7 = t6 ^ t5;
    uint t8 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (as_type<int>(l0) < as_type<int>(t3)) ? 1u : 0u;
        t8 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t9 = t7 << (t2 & 31u);
    uint t10 = t9 ^ t8;
    uint t11 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (as_type<int>(l0) == as_type<int>(t3)) ? 1u : 0u;
        t11 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t12 = t10 << (t2 & 31u);
    uint t13 = t12 ^ t11;
    uint t14 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (as_type<int>(l0) <= as_type<int>(t3)) ? 1u : 0u;
        t14 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t15 = t13 << (t2 & 31u);
    uint t16 = t15 ^ t14;
    uint t17 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (as_type<int>(l0) > as_type<int>(t3)) ? 1u : 0u;
        t17 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t18 = t16 << (t2 & 31u);
    uint t19 = t18 ^ t17;
    uint t20 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (as_type<int>(l0) != as_type<int>(t3)) ? 1u : 0u;
        t20 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t21 = t19 << (t2 & 31u);
    uint t22 = t21 ^ t20;
    uint t23 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (as_type<int>(l0) >= as_type<int>(t3)) ? 1u : 0u;
        t23 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t24 = t22 << (t2 & 31u);
    uint t25 = t24 ^ t23;
    uint t26 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint pass = (true) ? 1u : 0u;
        t26 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t27 = t25 << (t2 & 31u);
    uint t28 = t27 ^ t26;
    uint t29 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint pass = (false) ? 1u : 0u;
        t29 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t30 = t28 << (t2 & 31u);
    uint t31 = t30 ^ t29;
    uint t32 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (l0 < t3) ? 1u : 0u;
        t32 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t33 = t31 << (t2 & 31u);
    uint t34 = t33 ^ t32;
    uint t35 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (l0 == t3) ? 1u : 0u;
        t35 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t36 = t34 << (t2 & 31u);
    uint t37 = t36 ^ t35;
    uint t38 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (l0 <= t3) ? 1u : 0u;
        t38 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t39 = t37 << (t2 & 31u);
    uint t40 = t39 ^ t38;
    uint t41 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (l0 > t3) ? 1u : 0u;
        t41 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t42 = t40 << (t2 & 31u);
    uint t43 = t42 ^ t41;
    uint t44 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (l0 != t3) ? 1u : 0u;
        t44 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t45 = t43 << (t2 & 31u);
    uint t46 = t45 ^ t44;
    uint t47 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (l0 >= t3) ? 1u : 0u;
        t47 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t48 = t46 << (t2 & 31u);
    uint t49 = t48 ^ t47;
    uint t50 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint pass = (true) ? 1u : 0u;
        t50 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t51 = t49 << (t2 & 31u);
    uint t52 = t51 ^ t50;
    uint t53 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (l0 == t3) ? 1u : 0u;
        t53 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t54 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (t3 > l0) ? 1u : 0u;
        t54 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t56 = t4 ^ t52;
    uint t57 = t54 != 0u ? t56 : t52;
    output[index] = t57;
}
