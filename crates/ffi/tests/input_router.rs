// SPDX-License-Identifier: GPL-3.0-or-later

//! Acceptance tests for the session input router C interface (Campaign F,
//! #942). These need no GPU: they drive the router a session owns and read
//! its snapshot back through `xenora_session_input_snapshot`.

use std::ptr::{null, null_mut};
use std::sync::Mutex;
use xenora_ffi::*;

/// The session table is process-wide; run these one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn session() -> u64 {
    let mut handle = 0;
    assert_eq!(
        unsafe { xenora_session_create(XENORA_ABI_VERSION, &mut handle) },
        XENORA_OK
    );
    handle
}

fn snapshot(session: u64, time_ms: u64) -> [xenora_xinput_state; 4] {
    let mut out = [xenora_xinput_state {
        reserved: [9, 9],
        ..Default::default()
    }; 4];
    assert_eq!(
        unsafe { xenora_session_input_snapshot(session, time_ms, out.as_mut_ptr()) },
        XENORA_OK
    );
    out
}

fn key(session: u64, code: u16, down: bool, time_ms: u64) -> bool {
    let mut consumed = 7u8;
    assert_eq!(
        unsafe { xenora_session_input_key(session, code, u8::from(down), time_ms, &mut consumed) },
        XENORA_OK
    );
    assert!(consumed <= 1);
    consumed == 1
}

fn raw(elements: u32) -> xenora_controller_raw {
    xenora_controller_raw {
        elements,
        ..Default::default()
    }
}

const KEY_W: u16 = 0x0D;
const KEY_SPACE: u16 = 0x31;
const KEY_K: u16 = 0x28;

#[test]
fn constants_match_the_header() {
    let header = include_str!("../include/xenora.h");
    for (name, value) in [
        ("XENORA_INPUT_MODE_AUTOMATIC", XENORA_INPUT_MODE_AUTOMATIC),
        (
            "XENORA_INPUT_MODE_KEYBOARD_MOUSE_ONLY",
            XENORA_INPUT_MODE_KEYBOARD_MOUSE_ONLY,
        ),
        (
            "XENORA_INPUT_MODE_PHYSICAL_ONLY",
            XENORA_INPUT_MODE_PHYSICAL_ONLY,
        ),
        ("XENORA_INPUT_MODE_COMBINED", XENORA_INPUT_MODE_COMBINED),
        (
            "XENORA_INPUT_MODE_LAST_ACTIVE",
            XENORA_INPUT_MODE_LAST_ACTIVE,
        ),
        ("XENORA_INPUT_MODE_EXPLICIT", XENORA_INPUT_MODE_EXPLICIT),
        (
            "XENORA_CONTROLLER_FAMILY_XBOX_SERIES",
            XENORA_CONTROLLER_FAMILY_XBOX_SERIES,
        ),
        (
            "XENORA_CONTROLLER_FAMILY_XBOX_ONE",
            XENORA_CONTROLLER_FAMILY_XBOX_ONE,
        ),
        (
            "XENORA_CONTROLLER_FAMILY_XBOX_ELITE",
            XENORA_CONTROLLER_FAMILY_XBOX_ELITE,
        ),
        (
            "XENORA_CONTROLLER_FAMILY_DUALSHOCK4",
            XENORA_CONTROLLER_FAMILY_DUALSHOCK4,
        ),
        (
            "XENORA_CONTROLLER_FAMILY_DUALSENSE",
            XENORA_CONTROLLER_FAMILY_DUALSENSE,
        ),
        (
            "XENORA_CONTROLLER_FAMILY_SWITCH_PRO",
            XENORA_CONTROLLER_FAMILY_SWITCH_PRO,
        ),
        (
            "XENORA_CONTROLLER_FAMILY_GENERIC_EXTENDED",
            XENORA_CONTROLLER_FAMILY_GENERIC_EXTENDED,
        ),
        (
            "XENORA_CONTROLLER_FAMILY_MICRO",
            XENORA_CONTROLLER_FAMILY_MICRO,
        ),
    ] {
        assert!(
            header.contains(&format!("#define {name} {value}u")),
            "{name}"
        );
    }
    assert_eq!(
        (XENORA_INPUT_MODE_AUTOMATIC, XENORA_INPUT_MODE_EXPLICIT),
        (0, 5),
        "modes follow the router's order"
    );
    assert_eq!(
        (
            XENORA_CONTROLLER_FAMILY_XBOX_SERIES,
            XENORA_CONTROLLER_FAMILY_MICRO
        ),
        (0, 7)
    );
    // The raw controller wire structure: elements, six floats, a feature byte, three reserved.
    assert_eq!(std::mem::size_of::<xenora_controller_raw>(), 32);
    assert!(header.contains("} xenora_controller_raw;"));
}

#[test]
fn every_entry_point_validates_its_arguments() {
    let _serial = serial();
    let mut slot = 0u32;
    let mut out = [xenora_xinput_state::default(); 4];
    let input = raw(0);
    unsafe {
        assert_eq!(xenora_session_input_set_mode(0, 0), XENORA_INVALID_HANDLE);
        assert_eq!(
            xenora_session_input_key(0, KEY_W, 1, 0, null_mut()),
            XENORA_INVALID_HANDLE
        );
        assert_eq!(
            xenora_session_input_mouse_motion(0, 1.0, 0.0, 0),
            XENORA_INVALID_HANDLE
        );
        assert_eq!(
            xenora_session_input_mouse_button(0, 0, 1, 0, null_mut()),
            XENORA_INVALID_HANDLE
        );
        assert_eq!(xenora_session_input_focus(0, 1), XENORA_INVALID_HANDLE);
        assert_eq!(
            xenora_session_input_connect_device(0, 1, 0, &mut slot),
            XENORA_INVALID_HANDLE
        );
        assert_eq!(
            xenora_session_input_disconnect_device(0, 1),
            XENORA_INVALID_HANDLE
        );
        assert_eq!(
            xenora_session_input_device_raw(0, 1, &input, null(), 0),
            XENORA_INVALID_HANDLE
        );
        assert_eq!(
            xenora_session_input_snapshot(0, 0, out.as_mut_ptr()),
            XENORA_INVALID_HANDLE
        );
        assert_eq!(xenora_session_input_publish(0, 0), XENORA_INVALID_HANDLE);
    }
    let session = session();
    unsafe {
        assert_eq!(
            xenora_session_input_set_mode(session, 6),
            XENORA_INVALID_ARGUMENT
        );
        assert_eq!(
            xenora_session_input_key(session, KEY_W, 2, 0, null_mut()),
            XENORA_INVALID_ARGUMENT
        );
        assert_eq!(
            xenora_session_input_key(session, 0x80, 1, 0, null_mut()),
            XENORA_INVALID_ARGUMENT
        );
        assert_eq!(
            xenora_session_input_mouse_motion(session, f64::NAN, 0.0, 0),
            XENORA_INVALID_ARGUMENT
        );
        assert_eq!(
            xenora_session_input_mouse_motion(session, 0.0, f64::INFINITY, 0),
            XENORA_INVALID_ARGUMENT
        );
        assert_eq!(
            xenora_session_input_mouse_button(session, 32, 1, 0, null_mut()),
            XENORA_INVALID_ARGUMENT
        );
        assert_eq!(
            xenora_session_input_mouse_button(session, 0, 3, 0, null_mut()),
            XENORA_INVALID_ARGUMENT
        );
        assert_eq!(
            xenora_session_input_focus(session, 2),
            XENORA_INVALID_ARGUMENT
        );
        assert_eq!(
            xenora_session_input_connect_device(session, 1, 8, &mut slot),
            XENORA_INVALID_ARGUMENT
        );
        assert_eq!(
            xenora_session_input_connect_device(session, 1, 0, null_mut()),
            XENORA_INVALID_ARGUMENT
        );
        assert_eq!(
            xenora_session_input_device_raw(session, 1, null(), null(), 0),
            XENORA_INVALID_ARGUMENT
        );
        // A device that was never connected.
        assert_eq!(
            xenora_session_input_device_raw(session, 1, &input, null(), 0),
            XENORA_INVALID_ARGUMENT
        );
        assert_eq!(
            xenora_session_input_disconnect_device(session, 1),
            XENORA_INVALID_ARGUMENT
        );
        let bad_reserved = xenora_controller_raw {
            reserved: [0, 1, 0],
            ..raw(0)
        };
        assert_eq!(
            xenora_session_input_connect_device(session, 1, 0, &mut slot),
            XENORA_OK
        );
        assert_eq!(
            xenora_session_input_device_raw(session, 1, &bad_reserved, null(), 0),
            XENORA_INVALID_ARGUMENT
        );
        let bad_features = xenora_controller_raw {
            features: 0x02,
            ..raw(0)
        };
        assert_eq!(
            xenora_session_input_device_raw(session, 1, &bad_features, null(), 0),
            XENORA_INVALID_ARGUMENT
        );
        let bad_battery = xenora_xinput_battery {
            present: 0,
            state: 1,
            level: 0,
            reserved: 0,
        };
        assert_eq!(
            xenora_session_input_device_raw(session, 1, &input, &bad_battery, 0),
            XENORA_INVALID_ARGUMENT
        );
        assert_eq!(
            xenora_session_input_snapshot(session, 0, null_mut()),
            XENORA_INVALID_ARGUMENT
        );
        // Publishing needs a running frame loop.
        assert_eq!(
            xenora_session_input_publish(session, 0),
            XENORA_INVALID_STATE
        );
    }
    assert_eq!(xenora_session_destroy(session), XENORA_OK);
}

#[test]
fn keyboard_reaches_slot_zero_without_hardware() {
    let _serial = serial();
    let session = session();
    let start = snapshot(session, 0);
    assert_eq!(
        start[0].connected, 1,
        "the keyboard pad is connected from the start"
    );
    assert_eq!(start[0].reserved, [0, 0]);
    assert!(
        start[1..]
            .iter()
            .all(|slot| slot.connected == 0 && slot.buttons == 0)
    );
    assert!(key(session, KEY_W, true, 0));
    assert!(key(session, KEY_SPACE, true, 0));
    assert!(
        !key(session, KEY_K, true, 0),
        "an unbound key is not consumed"
    );
    let state = snapshot(session, 1)[0];
    assert_eq!(
        (state.buttons, state.left_x, state.left_y),
        (0x1000, 0, 32767)
    );
    assert_eq!(
        state.features, 0,
        "the keyboard pad has no vibration output"
    );
    assert!(key(session, KEY_SPACE, false, 2));
    assert_eq!(snapshot(session, 2)[0].buttons, 0);
    assert_eq!(xenora_session_destroy(session), XENORA_OK);
}

#[test]
fn mouse_motion_and_buttons_reach_the_keyboard_pad() {
    let _serial = serial();
    let session = session();
    assert_eq!(
        unsafe { xenora_session_input_mouse_motion(session, 50.0, 0.0, 100) },
        XENORA_OK
    );
    assert_eq!(snapshot(session, 100)[0].right_x, 16384);
    assert_eq!(snapshot(session, 150)[0].right_x, 0, "default 50 ms decay");
    let mut consumed = 0;
    assert_eq!(
        unsafe { xenora_session_input_mouse_button(session, 0, 1, 160, &mut consumed) },
        XENORA_OK
    );
    assert_eq!(consumed, 1);
    assert_eq!(snapshot(session, 160)[0].right_trigger, 255);
    assert_eq!(
        unsafe { xenora_session_input_mouse_button(session, 5, 1, 160, &mut consumed) },
        XENORA_OK
    );
    assert_eq!(consumed, 0, "button 5 has no binding");
    assert_eq!(xenora_session_destroy(session), XENORA_OK);
}

#[test]
fn controllers_are_translated_through_their_family_profile() {
    let _serial = serial();
    let session = session();
    let mut slot = 9u32;
    unsafe {
        assert_eq!(
            xenora_session_input_connect_device(
                session,
                0xA,
                XENORA_CONTROLLER_FAMILY_DUALSENSE,
                &mut slot
            ),
            XENORA_OK
        );
        assert_eq!(slot, 0);
        assert_eq!(
            xenora_session_input_connect_device(
                session,
                0xB,
                XENORA_CONTROLLER_FAMILY_SWITCH_PRO,
                &mut slot
            ),
            XENORA_OK
        );
        assert_eq!(slot, 1);
        let first = xenora_controller_raw {
            elements: 1 << 0, // south
            left_x: 0.5,
            right_trigger: 1.0,
            features: XENORA_XINPUT_FEATURE_VIBRATION,
            ..Default::default()
        };
        let battery = xenora_xinput_battery {
            present: 1,
            state: 2,
            level: 80,
            reserved: 0,
        };
        assert_eq!(
            xenora_session_input_device_raw(session, 0xA, &first, &battery, 5),
            XENORA_OK
        );
        let second = raw(1 << 1); // east
        assert_eq!(
            xenora_session_input_device_raw(session, 0xB, &second, null(), 5),
            XENORA_OK
        );
    }
    let out = snapshot(session, 5);
    // Slot 0 merges the keyboard pad (neutral) and the DualSense.
    assert_eq!(
        (out[0].buttons, out[0].left_x, out[0].right_trigger),
        (0x1000, 16384, 255)
    );
    assert_eq!(
        out[0].features, XENORA_XINPUT_FEATURE_VIBRATION,
        "slot 0's own device reports vibration"
    );
    // The Switch Pro default profile is positional: east is B.
    assert_eq!(
        (out[1].connected, out[1].buttons, out[1].features),
        (1, 0x2000, 0)
    );
    assert_eq!(out[2].connected, 0);
    assert_eq!(
        unsafe { xenora_session_input_disconnect_device(session, 0xB) },
        XENORA_OK
    );
    assert_eq!(snapshot(session, 6)[1].connected, 0);
    assert_eq!(xenora_session_destroy(session), XENORA_OK);
}

#[test]
fn a_fifth_controller_is_refused() {
    let _serial = serial();
    let session = session();
    let mut slot = 0u32;
    for id in 1..=4u64 {
        assert_eq!(
            unsafe { xenora_session_input_connect_device(session, id, 0, &mut slot) },
            XENORA_OK
        );
    }
    slot = 77;
    assert_eq!(
        unsafe { xenora_session_input_connect_device(session, 5, 0, &mut slot) },
        XENORA_LIMIT_EXCEEDED
    );
    assert_eq!(slot, 77, "the slot is not written on failure");
    assert_eq!(xenora_session_destroy(session), XENORA_OK);
}

#[test]
fn modes_and_focus_are_applied() {
    let _serial = serial();
    let session = session();
    let mut slot = 0u32;
    unsafe {
        assert_eq!(
            xenora_session_input_connect_device(session, 3, 0, &mut slot),
            XENORA_OK
        );
        assert_eq!(
            xenora_session_input_device_raw(session, 3, &raw(1 << 2), null(), 0),
            XENORA_OK
        ); // west: X
    }
    key(session, KEY_SPACE, true, 0);
    assert_eq!(snapshot(session, 0)[0].buttons, 0x1000 | 0x4000);
    assert_eq!(
        unsafe { xenora_session_input_set_mode(session, XENORA_INPUT_MODE_PHYSICAL_ONLY) },
        XENORA_OK
    );
    assert_eq!(snapshot(session, 0)[0].buttons, 0x4000);
    assert_eq!(
        unsafe { xenora_session_input_set_mode(session, XENORA_INPUT_MODE_KEYBOARD_MOUSE_ONLY) },
        XENORA_OK
    );
    assert_eq!(snapshot(session, 0)[0].buttons, 0x1000);
    assert_eq!(unsafe { xenora_session_input_focus(session, 0) }, XENORA_OK);
    assert_eq!(
        snapshot(session, 1)[0].buttons,
        0,
        "focus loss releases every key"
    );
    let mut consumed = 1;
    assert_eq!(
        unsafe { xenora_session_input_key(session, KEY_SPACE, 1, 2, &mut consumed) },
        XENORA_OK
    );
    assert_eq!(consumed, 0, "keys are not consumed while unfocused");
    assert_eq!(unsafe { xenora_session_input_focus(session, 1) }, XENORA_OK);
    assert!(key(session, KEY_SPACE, true, 3));
    assert_eq!(xenora_session_destroy(session), XENORA_OK);
}

#[test]
fn a_custom_profile_text_replaces_the_family_default() {
    let _serial = serial();
    let session = session();
    let mut slot = 0u32;
    let text = b"xenora-input-profile 1\nname = Swap\nfamily = dualsense\nbind south = button b\n";
    let bad = b"xenora-input-profile 9\nname = x\nfamily = dualsense\n";
    unsafe {
        assert_eq!(
            xenora_session_input_connect_device(session, 4, 4, &mut slot),
            XENORA_OK
        );
        assert_eq!(
            xenora_session_input_set_device_profile(session, 4, bad.as_ptr(), bad.len() as u64),
            XENORA_INVALID_ARGUMENT
        );
        assert_eq!(
            xenora_session_input_set_device_profile(session, 4, null(), 0),
            XENORA_INVALID_ARGUMENT
        );
        assert_eq!(
            xenora_session_input_set_device_profile(session, 9, text.as_ptr(), text.len() as u64),
            XENORA_INVALID_ARGUMENT,
            "unknown device"
        );
        assert_eq!(
            xenora_session_input_set_device_profile(session, 4, text.as_ptr(), text.len() as u64),
            XENORA_OK
        );
        assert_eq!(
            xenora_session_input_device_raw(session, 4, &raw(1 << 0), null(), 0),
            XENORA_OK
        );
    }
    assert_eq!(snapshot(session, 0)[0].buttons, 0x2000);
    assert_eq!(xenora_session_destroy(session), XENORA_OK);
}

#[test]
fn a_destroyed_and_recreated_session_starts_fresh() {
    let _serial = serial();
    let first = session();
    key(first, KEY_W, true, 0);
    let mut slot = 0;
    assert_eq!(
        unsafe { xenora_session_input_connect_device(first, 1, 0, &mut slot) },
        XENORA_OK
    );
    assert_eq!(xenora_session_destroy(first), XENORA_OK);
    let second = session();
    let out = snapshot(second, 0);
    assert_eq!((out[0].connected, out[0].left_y), (1, 0));
    assert_eq!(out[1].connected, 0);
    assert_eq!(xenora_session_destroy(second), XENORA_OK);
}
