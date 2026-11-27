// SPDX-License-Identifier: GPL-3.0-or-later

//! Publishing the session input router into a running frame loop's XInput
//! slots (Campaign F, #942). Needs `--features gpu-tests`
//! (scripts/test-gpu.sh), because a frame loop renders on native Metal.
#![cfg(feature = "gpu-tests")]

use std::ptr::null;
use std::sync::Mutex;
use xenora_ffi::*;

static SERIAL: Mutex<()> = Mutex::new(());

fn connected(session: u64, slot: u32) -> u8 {
    let mut vibration = xenora_xinput_vibration::default();
    assert_eq!(
        unsafe { xenora_session_poll_xinput_vibration(session, slot, &mut vibration) },
        XENORA_OK
    );
    vibration.connected
}

#[test]
fn publish_installs_router_output_into_the_running_loop() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut session = 0;
    assert_eq!(
        unsafe { xenora_session_create(XENORA_ABI_VERSION, &mut session) },
        XENORA_OK
    );
    assert_eq!(
        xenora_session_start_frame_loop(session, XENORA_FRAME_LOOP_MAX_FRAMES),
        XENORA_OK
    );
    // Before any publish the runtime slots start disconnected.
    assert_eq!(connected(session, 0), 0);
    assert_eq!(
        unsafe { xenora_session_input_publish(session, 0) },
        XENORA_OK
    );
    assert_eq!(
        connected(session, 0),
        1,
        "the keyboard pad is published on slot 0"
    );
    assert_eq!(connected(session, 1), 0);
    let mut slot = 0;
    let raw = xenora_controller_raw {
        elements: 1,
        ..Default::default()
    };
    unsafe {
        assert_eq!(
            xenora_session_input_connect_device(session, 1, 0, &mut slot),
            XENORA_OK
        );
        assert_eq!(
            xenora_session_input_connect_device(session, 2, 0, &mut slot),
            XENORA_OK
        );
        assert_eq!(slot, 1);
        assert_eq!(
            xenora_session_input_device_raw(session, 2, &raw, null(), 1),
            XENORA_OK
        );
        assert_eq!(xenora_session_input_publish(session, 1), XENORA_OK);
    }
    assert_eq!(connected(session, 1), 1);
    unsafe {
        assert_eq!(
            xenora_session_input_disconnect_device(session, 2),
            XENORA_OK
        );
        assert_eq!(xenora_session_input_publish(session, 2), XENORA_OK);
    }
    assert_eq!(connected(session, 1), 0, "a disconnect is published");
    assert_eq!(
        unsafe { xenora_session_input_set_mode(session, XENORA_INPUT_MODE_PHYSICAL_ONLY) },
        XENORA_OK
    );
    // Device 1 still holds slot 0 in physical-only mode.
    assert_eq!(
        unsafe { xenora_session_input_publish(session, 3) },
        XENORA_OK
    );
    assert_eq!(connected(session, 0), 1);
    assert_eq!(
        unsafe { xenora_session_input_disconnect_device(session, 1) },
        XENORA_OK
    );
    assert_eq!(
        unsafe { xenora_session_input_publish(session, 4) },
        XENORA_OK
    );
    assert_eq!(
        connected(session, 0),
        0,
        "physical-only mode with no device leaves slot 0 empty"
    );
    assert_eq!(xenora_session_stop_frame_loop(session), XENORA_OK);
    assert_eq!(xenora_session_destroy(session), XENORA_OK);
}
