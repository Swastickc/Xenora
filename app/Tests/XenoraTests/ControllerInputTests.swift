// SPDX-License-Identifier: GPL-3.0-or-later

import CXenora
import GameController
import Testing
@testable import Xenora

@MainActor
struct ControllerInputTests {
    @Test func microDirectionAndPrimaryActionReachTheSnapshot() throws {
        let controller = GCController.withMicroGamepad()
        let pad = try #require(controller.microGamepad)
        pad.dpad.setValueForXAxis(-1, yAxis: 0)
        pad.buttonA.setValue(1)

        let detected = try #require(pollGameController(current: controller, controllers: []))
        #expect(detected.identity == ObjectIdentifier(controller))
        #expect(detected.snapshot == ControllerSnapshot(
            buttons: UInt32(XENORA_INPUT_LEFT) | UInt32(XENORA_INPUT_ACTION_A),
            x: 0, y: 0, leftTrigger: 0, rightTrigger: 0
        ))

        pad.dpad.setValueForXAxis(0, yAxis: -1)
        pad.buttonA.setValue(0)
        pad.buttonX.setValue(1)
        let down = try #require(pollGameController(current: controller, controllers: []))
        #expect(down.snapshot.buttons == UInt32(XENORA_INPUT_DOWN))

        pad.dpad.setValueForXAxis(0, yAxis: 0)
        let released = try #require(pollGameController(current: controller, controllers: []))
        #expect(released.snapshot.buttons == 0)
    }

    @Test func pollPrefersExtendedControllersThenTheCurrentMicroController() {
        let micro = GCController.withMicroGamepad()
        let otherMicro = GCController.withMicroGamepad()
        let extended = GCController.withExtendedGamepad()
        let otherExtended = GCController.withExtendedGamepad()

        #expect(pollGameController(current: nil, controllers: []) == nil)
        #expect(pollGameController(current: nil, controllers: [micro])?.identity ==
                ObjectIdentifier(micro))
        #expect(pollGameController(current: otherMicro, controllers: [micro, otherMicro])?.identity ==
                ObjectIdentifier(otherMicro))
        #expect(pollGameController(current: micro, controllers: [micro, extended])?.identity ==
                ObjectIdentifier(extended))
        #expect(pollGameController(current: otherExtended, controllers: [extended, otherExtended])?.identity ==
                ObjectIdentifier(otherExtended))
    }

    @Test func extendedControllerStillForwardsButtonsThumbstickAndTriggers() throws {
        let controller = GCController.withExtendedGamepad()
        let pad = try #require(controller.extendedGamepad)
        pad.dpad.setValueForXAxis(1, yAxis: 0)
        pad.buttonB.setValue(1)
        pad.leftThumbstick.setValueForXAxis(-1, yAxis: 1)
        pad.leftTrigger.setValue(0.25)
        pad.rightTrigger.setValue(0.75)

        let detected = try #require(pollGameController(current: controller, controllers: []))
        #expect(detected.snapshot.buttons == UInt32(XENORA_INPUT_RIGHT) | UInt32(XENORA_INPUT_ACTION_B))
        #expect(detected.snapshot.x == -32767)
        #expect(detected.snapshot.y == 32767)
        #expect(detected.snapshot.leftTrigger == 0.25)
        #expect(detected.snapshot.rightTrigger == 0.75)
    }
}
