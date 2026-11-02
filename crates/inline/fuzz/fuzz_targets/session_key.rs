//! Fuzz `Session::key`: no key sequence may panic the renderer.
//!
//! Run with:
//!     cargo +nightly fuzz run session_key -- -max_total_time=60
//!
//! The session is seeded with a live turn and an open prompt so keys reach
//! the prompt, form, wizard, browser and editor paths instead of landing on
//! an empty input box. The driver is shared with `tests/fuzz_smoke.rs`.

#![no_main]

#[path = "../../tests/common/fuzz_drive.rs"]
mod fuzz_drive;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    fuzz_drive::drive_key(data);
});
