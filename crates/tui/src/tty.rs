//! Terminal-fd plumbing.
//!
//! Problem: when invoked as `cmd | camouflage-tui --stdin-events`, fd 0 is
//! the NDJSON pipe, not a TTY. crossterm's raw-mode + event reader both
//! ultimately depend on fd 0 being a terminal, so they fail/hang and the
//! TUI exits before drawing.
//!
//! Solution: at startup, dup fd 0 (the pipe) to a new fd we keep for
//! NDJSON ingestion, then dup /dev/tty over fd 0. After this swap:
//!
//!   fd 0 = /dev/tty   (crossterm is happy)
//!   fd N = original pipe   (we read NDJSON from here)
//!
//! If fd 0 is already a TTY (no pipe), we leave everything alone and
//! return `None` for the events fd.

use std::io;
use std::os::fd::RawFd;

/// Result of the fd swap.
pub struct FdLayout {
    /// If `Some`, fd 0 was a pipe; this is the dup'd fd for NDJSON.
    /// If `None`, fd 0 was already a TTY — there is no separate events fd
    /// and `--stdin-events` was effectively a no-op.
    pub events_fd: Option<RawFd>,
}

/// Returns true if `fd` refers to a terminal.
fn is_tty(fd: RawFd) -> bool {
    unsafe { libc::isatty(fd) == 1 }
}

/// Perform the fd swap described in the module docs. Idempotent: safe to
/// call once at startup.
pub fn install_fd_layout() -> io::Result<FdLayout> {
    if is_tty(0) {
        return Ok(FdLayout { events_fd: None });
    }
    // Save the original stdin (the pipe) to a fresh fd.
    let saved = unsafe { libc::dup(0) };
    if saved < 0 {
        return Err(io::Error::last_os_error());
    }
    // Open /dev/tty for both read and write.
    let tty_path = b"/dev/tty\0";
    let tty_fd = unsafe { libc::open(tty_path.as_ptr() as *const _, libc::O_RDWR) };
    if tty_fd < 0 {
        let e = io::Error::last_os_error();
        unsafe { libc::close(saved) };
        return Err(e);
    }
    // Put /dev/tty at fd 0.
    let rc = unsafe { libc::dup2(tty_fd, 0) };
    if rc < 0 {
        let e = io::Error::last_os_error();
        unsafe {
            libc::close(saved);
            libc::close(tty_fd);
        }
        return Err(e);
    }
    unsafe { libc::close(tty_fd) };
    Ok(FdLayout { events_fd: Some(saved) })
}
