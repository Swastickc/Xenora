#include <metal_stdlib>
using namespace metal;
kernel void xenora_compute(
    device const uint* input [[buffer(0)]],
    device uint* output [[buffer(1)]],
    uint index [[thread_position_in_grid]]) {
    uint t0 = input[index];
    uint t1 = (t0 & 0x7fffffffu) >= 0x4b000000u ? t0 : (t0 & 0x7fffffffu) < 0x3f800000u ? (t0 & 0x80000000u) : (t0 & ~(0x007fffffu >> ((((t0 >> 23u) & 0xffu) - 127u) & 31u)));
    uint t2 = (t0 & 0x7fffffffu) >= 0x4b000000u ? t0 : (t0 & 0x7fffffffu) < 0x3f800000u ? ((t0 != 0u && t0 < 0x80000000u) ? 0x3f800000u : (t0 & 0x80000000u)) : ((t0 & ~(0x007fffffu >> ((((t0 >> 23u) & 0xffu) - 127u) & 31u))) + (((t0 & (0x007fffffu >> ((((t0 >> 23u) & 0xffu) - 127u) & 31u))) != 0u && (t0 & 0x80000000u) == 0u) ? ((0x007fffffu >> ((((t0 >> 23u) & 0xffu) - 127u) & 31u)) + 1u) : 0u));
    uint t3 = (t0 & 0x7fffffffu) >= 0x4b000000u ? t0 : (t0 & 0x7fffffffu) < 0x3f800000u ? ((t0 & 0x7fffffffu) > 0x3f000000u ? ((t0 & 0x80000000u) | 0x3f800000u) : (t0 & 0x80000000u)) : ((t0 & ~(0x007fffffu >> ((((t0 >> 23u) & 0xffu) - 127u) & 31u))) + (((t0 & (0x007fffffu >> ((((t0 >> 23u) & 0xffu) - 127u) & 31u))) > (((0x007fffffu >> ((((t0 >> 23u) & 0xffu) - 127u) & 31u)) + 1u) >> 1u) || ((t0 & (0x007fffffu >> ((((t0 >> 23u) & 0xffu) - 127u) & 31u))) == (((0x007fffffu >> ((((t0 >> 23u) & 0xffu) - 127u) & 31u)) + 1u) >> 1u) && (t0 & ((0x007fffffu >> ((((t0 >> 23u) & 0xffu) - 127u) & 31u)) + 1u)) != 0u)) ? ((0x007fffffu >> ((((t0 >> 23u) & 0xffu) - 127u) & 31u)) + 1u) : 0u));
    uint t4 = (t0 & 0x7fffffffu) >= 0x4b000000u ? t0 : (t0 & 0x7fffffffu) < 0x3f800000u ? (t0 > 0x80000000u ? 0xbf800000u : (t0 & 0x80000000u)) : ((t0 & ~(0x007fffffu >> ((((t0 >> 23u) & 0xffu) - 127u) & 31u))) + (((t0 & (0x007fffffu >> ((((t0 >> 23u) & 0xffu) - 127u) & 31u))) != 0u && (t0 & 0x80000000u) != 0u) ? ((0x007fffffu >> ((((t0 >> 23u) & 0xffu) - 127u) & 31u)) + 1u) : 0u));
    uint t5 = ((0x55555555u >> (index & 31u)) & 1u) != 0u ? t4 : t3;
    uint t6 = 0x40200000u;
    uint t7 = (t6 & 0x7fffffffu) >= 0x4b000000u ? t6 : (t6 & 0x7fffffffu) < 0x3f800000u ? ((t6 & 0x7fffffffu) > 0x3f000000u ? ((t6 & 0x80000000u) | 0x3f800000u) : (t6 & 0x80000000u)) : ((t6 & ~(0x007fffffu >> ((((t6 >> 23u) & 0xffu) - 127u) & 31u))) + (((t6 & (0x007fffffu >> ((((t6 >> 23u) & 0xffu) - 127u) & 31u))) > (((0x007fffffu >> ((((t6 >> 23u) & 0xffu) - 127u) & 31u)) + 1u) >> 1u) || ((t6 & (0x007fffffu >> ((((t6 >> 23u) & 0xffu) - 127u) & 31u))) == (((0x007fffffu >> ((((t6 >> 23u) & 0xffu) - 127u) & 31u)) + 1u) >> 1u) && (t6 & ((0x007fffffu >> ((((t6 >> 23u) & 0xffu) - 127u) & 31u)) + 1u)) != 0u)) ? ((0x007fffffu >> ((((t6 >> 23u) & 0xffu) - 127u) & 31u)) + 1u) : 0u));
    uint t8 = t1 ^ t2;
    uint t9 = t5 ^ t8;
    uint t10 = t7 ^ t9;
    output[index] = t10;
}
