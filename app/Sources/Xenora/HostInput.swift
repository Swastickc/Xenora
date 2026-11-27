// SPDX-License-Identifier: GPL-3.0-or-later
//
// Host input for the session router (Campaign F, #942).
//
// GameController values become positional raw elements that the session's
// router translates through a per-family profile. Keyboard and mouse events
// become physical key codes and deltas. Nothing here maps input to XInput
// itself: the router in the Rust library does that, for guest execution and
// for any settings screen alike.

import CXenora
import GameController

/// Positional element bits, the same values as `xenora_input::controller::element`.
enum ControllerElement {
    static let south: UInt32 = 1 << 0
    static let east: UInt32 = 1 << 1
    static let west: UInt32 = 1 << 2
    static let north: UInt32 = 1 << 3
    static let leftShoulder: UInt32 = 1 << 4
    static let rightShoulder: UInt32 = 1 << 5
    static let leftStick: UInt32 = 1 << 6
    static let rightStick: UInt32 = 1 << 7
    static let menu: UInt32 = 1 << 8
    static let view: UInt32 = 1 << 9
    static let home: UInt32 = 1 << 10
    static let dpadUp: UInt32 = 1 << 11
    static let dpadDown: UInt32 = 1 << 12
    static let dpadLeft: UInt32 = 1 << 13
    static let dpadRight: UInt32 = 1 << 14
    static let paddle1: UInt32 = 1 << 15
    static let paddle2: UInt32 = 1 << 16
    static let paddle3: UInt32 = 1 << 17
    static let paddle4: UInt32 = 1 << 18
    static let touchpad: UInt32 = 1 << 19
    static let share: UInt32 = 1 << 20
}

/// What the router needs to recognize a controller family.
struct ControllerDescription: Equatable {
    var productCategory: String
    var extended: Bool
    var hasPaddles: Bool
    var hasShare: Bool

    @MainActor
    init(_ controller: GCController) {
        productCategory = controller.productCategory
        extended = controller.extendedGamepad != nil
        let xbox = controller.extendedGamepad as? GCXboxGamepad
        hasPaddles = xbox?.paddleButton1 != nil
        hasShare = xbox?.buttonShare != nil
    }

    init(productCategory: String, extended: Bool, hasPaddles: Bool = false, hasShare: Bool = false) {
        self.productCategory = productCategory
        self.extended = extended
        self.hasPaddles = hasPaddles
        self.hasShare = hasShare
    }

    /// The family the library recognizes for this description, or nil if the
    /// library refused the call.
    func family() -> UInt32? {
        var family = UInt32.max
        let status = productCategory.utf8CString.withUnsafeBufferPointer { text in
            text.withMemoryRebound(to: UInt8.self) { bytes in
                xenora_controller_recognize(
                    bytes.baseAddress, UInt64(bytes.count - 1),
                    extended ? 1 : 0, hasPaddles ? 1 : 0, hasShare ? 1 : 0, &family
                )
            }
        }
        return status == XENORA_OK ? family : nil
    }
}

/// Raw positional values of an extended gamepad.
@MainActor
func rawController(_ pad: GCExtendedGamepad, vibrationSupported: Bool) -> xenora_controller_raw {
    var elements: UInt32 = 0
    func add(_ button: GCControllerButtonInput?, _ bit: UInt32) {
        if button?.isPressed == true { elements |= bit }
    }
    add(pad.buttonA, ControllerElement.south)
    add(pad.buttonB, ControllerElement.east)
    add(pad.buttonX, ControllerElement.west)
    add(pad.buttonY, ControllerElement.north)
    add(pad.leftShoulder, ControllerElement.leftShoulder)
    add(pad.rightShoulder, ControllerElement.rightShoulder)
    add(pad.leftThumbstickButton, ControllerElement.leftStick)
    add(pad.rightThumbstickButton, ControllerElement.rightStick)
    add(pad.buttonMenu, ControllerElement.menu)
    add(pad.buttonOptions, ControllerElement.view)
    add(pad.buttonHome, ControllerElement.home)
    add(pad.dpad.up, ControllerElement.dpadUp)
    add(pad.dpad.down, ControllerElement.dpadDown)
    add(pad.dpad.left, ControllerElement.dpadLeft)
    add(pad.dpad.right, ControllerElement.dpadRight)
    if let xbox = pad as? GCXboxGamepad {
        add(xbox.paddleButton1, ControllerElement.paddle1)
        add(xbox.paddleButton2, ControllerElement.paddle2)
        add(xbox.paddleButton3, ControllerElement.paddle3)
        add(xbox.paddleButton4, ControllerElement.paddle4)
        add(xbox.buttonShare, ControllerElement.share)
    }
    if let dualSense = pad as? GCDualSenseGamepad { add(dualSense.touchpadButton, ControllerElement.touchpad) }
    if let dualShock = pad as? GCDualShockGamepad { add(dualShock.touchpadButton, ControllerElement.touchpad) }
    return xenora_controller_raw(
        elements: elements,
        left_x: pad.leftThumbstick.xAxis.value,
        left_y: pad.leftThumbstick.yAxis.value,
        right_x: pad.rightThumbstick.xAxis.value,
        right_y: pad.rightThumbstick.yAxis.value,
        left_trigger: pad.leftTrigger.value,
        right_trigger: pad.rightTrigger.value,
        features: vibrationSupported ? UInt8(XENORA_XINPUT_FEATURE_VIBRATION) : 0,
        reserved: (0, 0, 0)
    )
}

/// Raw positional values of a micro gamepad: a directional pad, A, X and menu.
@MainActor
func rawController(_ pad: GCMicroGamepad, vibrationSupported: Bool) -> xenora_controller_raw {
    var elements: UInt32 = 0
    if pad.buttonA.isPressed { elements |= ControllerElement.south }
    if pad.buttonX.isPressed { elements |= ControllerElement.west }
    if pad.buttonMenu.isPressed { elements |= ControllerElement.menu }
    if pad.dpad.up.isPressed { elements |= ControllerElement.dpadUp }
    if pad.dpad.down.isPressed { elements |= ControllerElement.dpadDown }
    if pad.dpad.left.isPressed { elements |= ControllerElement.dpadLeft }
    if pad.dpad.right.isPressed { elements |= ControllerElement.dpadRight }
    return xenora_controller_raw(
        elements: elements, left_x: 0, left_y: 0, right_x: 0, right_y: 0,
        left_trigger: 0, right_trigger: 0,
        features: vibrationSupported ? UInt8(XENORA_XINPUT_FEATURE_VIBRATION) : 0,
        reserved: (0, 0, 0)
    )
}

/// Milliseconds on the host's monotonic clock, the time base the router uses.
func hostInputMilliseconds() -> UInt64 {
    clock_gettime_nsec_np(CLOCK_UPTIME_RAW) / 1_000_000
}
