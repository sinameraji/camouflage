//! The few things that differ between Unix and Windows.

use std::fs::File;
use std::path::PathBuf;

/// The user's terminal for drawing, independent of where stdout points
/// (in the SDK's default mode stdout carries outbound events).
pub fn open_console_writer() -> std::io::Result<File> {
    #[cfg(unix)]
    let path = "/dev/tty";
    #[cfg(windows)]
    let path = "CONOUT$";
    let file = std::fs::OpenOptions::new().write(true).open(path)?;
    #[cfg(windows)]
    enable_vt_output();
    Ok(file)
}

/// Windows consoles only interpret ANSI escapes with virtual-terminal
/// processing on (Windows Terminal has it on; conhost needs asking).
#[cfg(windows)]
pub fn enable_vt_output() {
    use crossterm_winapi::{ConsoleMode, Handle};
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
    if let Ok(handle) = Handle::current_out_handle() {
        let mode = ConsoleMode::from(handle);
        if let Ok(m) = mode.mode() {
            let _ = mode.set_mode(m | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
        }
    }
}

/// A pipe the host opened for us before spawning (`--responses-fd`).
pub fn file_from_fd(fd: i32) -> anyhow::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::io::FromRawFd;
        // SAFETY: the host opened this fd for us before spawning; the File
        // closes it on drop, which the host sees as EOF.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    #[cfg(not(unix))]
    {
        let _ = fd;
        anyhow::bail!("--responses-fd isn't supported on Windows; use --emit-responses")
    }
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}
