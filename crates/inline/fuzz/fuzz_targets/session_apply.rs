//! Fuzz `Session::apply`: no host event may panic the renderer.
//!
//! Run with:
//!     cargo +nightly fuzz run session_apply -- -max_total_time=60
//!
//! The input is a sequence of `[type tag][payload length][payload bytes]`
//! frames, so one input drives a whole session rather than a single event.
//! The driver is shared with `tests/fuzz_smoke.rs`, which runs the same code
//! on stable in every CI job.

#![no_main]

#[path = "../../tests/common/fuzz_drive.rs"]
mod fuzz_drive;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    fuzz_drive::drive_apply(data);
});
