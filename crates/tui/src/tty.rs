//! Raw-mode helpers that operate on `/dev/tty` rather than stdin.
//!
//! Required so that `camouflage-tui --stdin-events` works when stdin is a
//! pipe carrying NDJSON: crossterm's default `enable_raw_mode` operates on
//! fd 0, which fails with ENOTTY for pipes. We bypass it by opening
//! /dev/tty directly and applying termios there.

use std::io;
use std::os::fd::RawFd;
use std::sync::Mutex;

static SAVED: Mutex<Option<SavedState>> = Mutex::new(None);

struct SavedState {
    fd: RawFd,
    termios: libc::termios,
}

/// Open /dev/tty read/write and put it into raw mode. Saves the original
/// termios so [`restore`] can return the terminal to cooked mode.
pub fn enable_raw_mode_via_tty() -> io::Result<RawFd> {
    let path = b"/dev/tty\0";
    let fd = unsafe { libc::open(path.as_ptr() as *const _, libc::O_RDWR | libc::O_NOCTTY) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut termios: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut termios) } != 0 {
        let e = io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(e);
    }
    let original = termios;
    unsafe { libc::cfmakeraw(&mut termios) };
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &termios) } != 0 {
        let e = io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(e);
    }
    *SAVED.lock().unwrap() = Some(SavedState { fd, termios: original });
    Ok(fd)
}

/// Restore the saved termios on /dev/tty and close the fd. Idempotent.
pub fn restore() {
    if let Some(state) = SAVED.lock().unwrap().take() {
        unsafe {
            libc::tcsetattr(state.fd, libc::TCSANOW, &state.termios);
            libc::close(state.fd);
        }
    }
}
