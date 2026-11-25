#include <metal_stdlib>
using namespace metal;
kernel void xenora_compute(
    device const uint* input [[buffer(0)]],
    device uint* output [[buffer(1)]],
    uint index [[thread_position_in_grid]]) {
    uint t0 = input[index];
    uint t1 = as_type<uint>((as_type<int>(t0 << 8u) >> 8)) * as_type<uint>((as_type<int>(t0 << 8u) >> 8));
    uint t2 = as_type<uint>(mulhi((as_type<int>(t1 << 8u) >> 8), (as_type<int>(t0 << 8u) >> 8)));
    uint t3 = (t1 & 0x00ffffffu) * (t0 & 0x00ffffffu);
    uint t4 = mulhi((t3 & 0x00ffffffu), (t0 & 0x00ffffffu));
    uint t5 = as_type<uint>(min(as_type<int>(t0), as_type<int>(t1)));
    uint t6 = as_type<uint>(max(as_type<int>(t0), as_type<int>(t3)));
    uint t7 = min(t2, t0);
    uint t8 = max(t4, t1);
    uint t9 = ~(t0 ^ t4);
    uint t10 = t5 - t9;
    uint t11 = t8 - t0;
    uint t12 = t1 ^ t2;
    uint t13 = t3 ^ t12;
    uint t14 = t4 ^ t13;
    uint t15 = t5 ^ t14;
    uint t16 = t6 ^ t15;
    uint t17 = t7 ^ t16;
    uint t18 = t8 ^ t17;
    uint t19 = t9 ^ t18;
    uint t20 = t10 ^ t19;
    uint t21 = t11 ^ t20;
    output[index] = t21;
}
