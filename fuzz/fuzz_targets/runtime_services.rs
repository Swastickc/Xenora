// SPDX-License-Identifier: GPL-3.0-or-later
#![no_main]

#[path = "../../crates/loader/tests/support/robustness.rs"]
mod robustness;

use libfuzzer_sys::fuzz_target;
use std::sync::OnceLock;

fuzz_target!(|data: &[u8]| {
    static DRIVER: OnceLock<robustness::Driver> = OnceLock::new();
    DRIVER.get_or_init(robustness::Driver::new).run(data);
});
