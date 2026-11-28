// SPDX-License-Identifier: GPL-3.0-or-later

//! Controller family recognition for the app (Campaign F, #942).
//!
//! The app passes what GameController reports about a controller and gets the
//! family number to give `xenora_session_input_connect_device`. The rule is
//! the one in `xenora_input::controller::recognize`: an exact match on the
//! documented product category strings, with the Elite told apart by paddle
//! buttons and the Series X|S by a Share button. Keeping it here means the app
//! does not carry a second copy of that rule.

use crate::guarded;
use xenora_input::controller::{Family, HostDescription, recognize};

/// The longest product category accepted, in bytes.
const MAX_CATEGORY_BYTES: u64 = 256;

/// # Safety
/// `category` must be null with `length` zero, or valid for `length` bytes.
/// `out_family` must be null or valid for one aligned `u32` write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xenora_controller_recognize(
    category: *const u8,
    length: u64,
    extended: u8,
    has_paddles: u8,
    has_share: u8,
    out_family: *mut u32,
) -> i32 {
    guarded(|| {
        if out_family.is_null()
            || length > MAX_CATEGORY_BYTES
            || (category.is_null() && length != 0)
            || extended > 1
            || has_paddles > 1
            || has_share > 1
        {
            return crate::XENORA_INVALID_ARGUMENT;
        }
        let bytes: &[u8] = if length == 0 {
            &[]
        } else {
            // SAFETY: non-null and bounded, checked above; the caller
            // guarantees the bytes are readable.
            unsafe { std::slice::from_raw_parts(category, length as usize) }
        };
        let Ok(product_category) = std::str::from_utf8(bytes) else {
            return crate::XENORA_INVALID_ARGUMENT;
        };
        let family = recognize(&HostDescription {
            product_category,
            extended: extended == 1,
            has_paddles: has_paddles == 1,
            has_share: has_share == 1,
        });
        let Some(index) = Family::ALL.iter().position(|known| *known == family) else {
            return crate::XENORA_INTERNAL_ERROR;
        };
        // SAFETY: checked non-null above; the caller guarantees the write.
        unsafe { out_family.write(index as u32) };
        crate::XENORA_OK
    })
}
