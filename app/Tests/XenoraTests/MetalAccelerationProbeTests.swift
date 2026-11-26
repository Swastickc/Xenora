// SPDX-License-Identifier: GPL-3.0-or-later

import Testing
@testable import Xenora

struct MetalAccelerationProbeTests {
    @Test func singleTrianglePrimitiveBuildCompletesTwice() throws {
        let first = try MetalAccelerationStructureProbe.buildSingleTriangle()
        let second = try MetalAccelerationStructureProbe.buildSingleTriangle()

        #expect(!first.deviceName.isEmpty)
        #expect(first.supportsRaytracing)
        #expect(first.accelerationStructureSize > 0)
        #expect(first.buildScratchBufferSize > 0)
        #expect(first.accelerationStructureSize == second.accelerationStructureSize)
        #expect(first.buildScratchBufferSize == second.buildScratchBufferSize)
        #expect(first.refitScratchBufferSize == second.refitScratchBufferSize)
    }
}
