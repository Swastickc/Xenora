// SPDX-License-Identifier: GPL-3.0-or-later

import CXenora
import GameController
import Testing
@testable import Xenora

/// Virtual-device tests: GCController.withExtendedGamepad() and
/// withMicroGamepad() create snapshot controllers whose values the test sets.
/// No physical controller is involved.
@MainActor
struct HostInputTests {
    @Test func elementBitsMatchTheRouterAndAreDistinct() {
        let all: [UInt32] = [
            ControllerElement.south, ControllerElement.east, ControllerElement.west, ControllerElement.north,
            ControllerElement.leftShoulder, ControllerElement.rightShoulder, ControllerElement.leftStick,
            ControllerElement.rightStick, ControllerElement.menu, ControllerElement.view, ControllerElement.home,
            ControllerElement.dpadUp, ControllerElement.dpadDown, ControllerElement.dpadLeft,
            ControllerElement.dpadRight, ControllerElement.paddle1, ControllerElement.paddle2,
            ControllerElement.paddle3, ControllerElement.paddle4, ControllerElement.touchpad, ControllerElement.share,
        ]
        var seen: UInt32 = 0
        for (index, bit) in all.enumerated() {
            #expect(bit == 1 << UInt32(index))
            #expect(seen & bit == 0)
            seen |= bit
        }
        #expect(MemoryLayout<xenora_controller_raw>.size == 32)
    }

    @Test func extendedGamepadValuesBecomePositionalElements() throws {
        let controller = GCController.withExtendedGamepad()
        let pad = try #require(controller.extendedGamepad)
        pad.buttonA.setValue(1)
        pad.buttonY.setValue(1)
        pad.leftShoulder.setValue(1)
        pad.buttonMenu.setValue(1)
        pad.dpad.setValueForXAxis(1, yAxis: 0)
        pad.leftThumbstick.setValueForXAxis(-0.5, yAxis: 0.25)
        pad.rightThumbstick.setValueForXAxis(1, yAxis: -1)
        pad.leftTrigger.setValue(0.75)
        pad.rightTrigger.setValue(0)

        let raw = rawController(pad, vibrationSupported: true)
        #expect(raw.elements == ControllerElement.south | ControllerElement.north
            | ControllerElement.leftShoulder | ControllerElement.menu | ControllerElement.dpadRight)
        #expect(raw.left_x == -0.5 && raw.left_y == 0.25)
        #expect(raw.right_x == 1 && raw.right_y == -1)
        #expect(raw.left_trigger == 0.75 && raw.right_trigger == 0)
        #expect(raw.features == UInt8(XENORA_XINPUT_FEATURE_VIBRATION))
        #expect(raw.reserved == (0, 0, 0))

        pad.buttonA.setValue(0)
        pad.buttonY.setValue(0)
        pad.leftShoulder.setValue(0)
        pad.buttonMenu.setValue(0)
        pad.dpad.setValueForXAxis(0, yAxis: 0)
        let released = rawController(pad, vibrationSupported: false)
        #expect(released.elements == 0)
        #expect(released.features == 0)
    }

    @Test func microGamepadReportsItsSmallSet() throws {
        let controller = GCController.withMicroGamepad()
        let pad = try #require(controller.microGamepad)
        pad.buttonA.setValue(1)
        pad.buttonX.setValue(1)
        pad.dpad.setValueForXAxis(0, yAxis: -1)
        let raw = rawController(pad, vibrationSupported: false)
        #expect(raw.elements == ControllerElement.south | ControllerElement.west | ControllerElement.dpadDown)
        #expect(raw.left_x == 0 && raw.right_trigger == 0)
    }

    @Test func familiesComeFromTheLibraryRule() {
        let cases: [(ControllerDescription, UInt32)] = [
            (ControllerDescription(productCategory: "Xbox One", extended: true), XENORA_CONTROLLER_FAMILY_XBOX_ONE),
            (ControllerDescription(productCategory: "Xbox One", extended: true, hasShare: true),
             XENORA_CONTROLLER_FAMILY_XBOX_SERIES),
            (ControllerDescription(productCategory: "Xbox One", extended: true, hasPaddles: true, hasShare: true),
             XENORA_CONTROLLER_FAMILY_XBOX_ELITE),
            (ControllerDescription(productCategory: "DualSense", extended: true), XENORA_CONTROLLER_FAMILY_DUALSENSE),
            (ControllerDescription(productCategory: "DualShock 4", extended: true),
             XENORA_CONTROLLER_FAMILY_DUALSHOCK4),
            (ControllerDescription(productCategory: "Pro Controller", extended: true),
             XENORA_CONTROLLER_FAMILY_GENERIC_EXTENDED),
            (ControllerDescription(productCategory: "DualSense", extended: false), XENORA_CONTROLLER_FAMILY_MICRO),
        ]
        for (description, family) in cases {
            #expect(description.family() == family, "\(description)")
        }
    }

    @Test func aVirtualControllerDescribesItself() {
        let controller = GCController.withExtendedGamepad()
        let description = ControllerDescription(controller)
        #expect(description.extended)
        #expect(!description.hasPaddles)
        #expect(description.family() != nil)
    }

    @Test func routedSnapshotReflectsAVirtualController() throws {
        var session: xenora_session = 0
        #expect(xenora_session_create(XENORA_ABI_VERSION, &session) == XENORA_OK)
        defer { _ = xenora_session_destroy(session) }
        let controller = GCController.withExtendedGamepad()
        let pad = try #require(controller.extendedGamepad)
        pad.buttonB.setValue(1)
        pad.rightTrigger.setValue(1)
        var slot: UInt32 = 9
        #expect(xenora_session_input_connect_device(session, 1, XENORA_CONTROLLER_FAMILY_DUALSENSE, &slot) == XENORA_OK)
        #expect(slot == 0)
        var raw = rawController(pad, vibrationSupported: false)
        #expect(xenora_session_input_device_raw(session, 1, &raw, nil, 0) == XENORA_OK)
        var slots = [xenora_xinput_state](repeating: xenora_xinput_state(), count: 4)
        #expect(xenora_session_input_snapshot(session, 0, &slots) == XENORA_OK)
        #expect(slots[0].connected == 1)
        #expect(slots[0].buttons == UInt16(XENORA_XINPUT_GAMEPAD_B))
        #expect(slots[0].right_trigger == 255)
    }
}
