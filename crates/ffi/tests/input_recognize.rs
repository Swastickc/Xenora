// SPDX-License-Identifier: GPL-3.0-or-later

//! Family recognition through the C interface (Campaign F, #942, review).

use std::ptr::{null, null_mut};
use xenora_ffi::*;

fn recognize(category: &str, extended: u8, paddles: u8, share: u8) -> u32 {
    let mut family = u32::MAX;
    let status = unsafe {
        xenora_controller_recognize(
            category.as_ptr(),
            category.len() as u64,
            extended,
            paddles,
            share,
            &mut family,
        )
    };
    assert_eq!(status, XENORA_OK, "{category}");
    family
}

#[test]
fn documented_categories_map_to_their_families() {
    assert_eq!(
        recognize("Xbox One", 1, 0, 0),
        XENORA_CONTROLLER_FAMILY_XBOX_ONE
    );
    assert_eq!(
        recognize("Xbox One", 1, 0, 1),
        XENORA_CONTROLLER_FAMILY_XBOX_SERIES
    );
    assert_eq!(
        recognize("Xbox One", 1, 1, 1),
        XENORA_CONTROLLER_FAMILY_XBOX_ELITE
    );
    assert_eq!(
        recognize("DualSense", 1, 0, 0),
        XENORA_CONTROLLER_FAMILY_DUALSENSE
    );
    assert_eq!(
        recognize("DualShock 4", 1, 0, 0),
        XENORA_CONTROLLER_FAMILY_DUALSHOCK4
    );
    assert_eq!(
        recognize("Pro Controller", 1, 0, 0),
        XENORA_CONTROLLER_FAMILY_GENERIC_EXTENDED
    );
    assert_eq!(
        recognize("", 1, 0, 0),
        XENORA_CONTROLLER_FAMILY_GENERIC_EXTENDED
    );
    assert_eq!(
        recognize("DualSense", 0, 0, 0),
        XENORA_CONTROLLER_FAMILY_MICRO
    );
}

#[test]
fn malformed_arguments_are_refused_without_writing() {
    let mut family = 42u32;
    let name = "Xbox One";
    unsafe {
        assert_eq!(
            xenora_controller_recognize(name.as_ptr(), 8, 1, 0, 0, null_mut()),
            XENORA_INVALID_ARGUMENT
        );
        assert_eq!(
            xenora_controller_recognize(null(), 3, 1, 0, 0, &mut family),
            XENORA_INVALID_ARGUMENT
        );
        assert_eq!(
            xenora_controller_recognize(null(), 0, 1, 0, 0, &mut family),
            XENORA_OK
        );
        family = 42;
        assert_eq!(
            xenora_controller_recognize(name.as_ptr(), 8, 2, 0, 0, &mut family),
            XENORA_INVALID_ARGUMENT
        );
        assert_eq!(
            xenora_controller_recognize(name.as_ptr(), 8, 1, 2, 0, &mut family),
            XENORA_INVALID_ARGUMENT
        );
        assert_eq!(
            xenora_controller_recognize(name.as_ptr(), 8, 1, 0, 2, &mut family),
            XENORA_INVALID_ARGUMENT
        );
        let long = vec![b'x'; 257];
        assert_eq!(
            xenora_controller_recognize(long.as_ptr(), 257, 1, 0, 0, &mut family),
            XENORA_INVALID_ARGUMENT
        );
        let invalid = [0xFFu8, 0xFE];
        assert_eq!(
            xenora_controller_recognize(invalid.as_ptr(), 2, 1, 0, 0, &mut family),
            XENORA_INVALID_ARGUMENT
        );
    }
    assert_eq!(family, 42);
    let header = include_str!("../include/xenora.h");
    assert!(header.contains("xenora_status xenora_controller_recognize("));
}
