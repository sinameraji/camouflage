//! The few things that differ between Unix and Windows.

use std::fs::File;
use std::path::PathBuf;

/// The user's terminal for drawing, independent of where stdout points
/// (in the SDK's default mode stdout carries outbound events).
///
/// On Windows this is a plain file handle on `CONOUT$`: writes go through
/// `WriteFile`, which hands the console raw bytes that it decodes with the
/// console *output code page* (437 on a stock US install), not as UTF-8.
/// (Rust's `std::io::stdout()` converts to UTF-16 and calls
/// `WriteConsoleW` when stdout is the console, so only this path is
/// affected.) [`enter_console`] switches the code page to UTF-8.
pub fn open_console_writer() -> std::io::Result<File> {
    #[cfg(unix)]
    let path = "/dev/tty";
    #[cfg(windows)]
    let path = "CONOUT$";
    std::fs::OpenOptions::new().write(true).open(path)
}

/// Windows console mode bits (wincon.h).
#[cfg_attr(not(windows), allow(dead_code))]
const ENABLE_PROCESSED_OUTPUT: u32 = 0x0001;
#[cfg_attr(not(windows), allow(dead_code))]
const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
#[cfg_attr(not(windows), allow(dead_code))]
const CP_UTF8: u32 = 65001;

/// The output mode the renderer needs: ANSI escapes interpreted (cursor
/// movement, erase, colours) rather than printed. VT processing only works
/// together with processed output.
#[cfg_attr(not(windows), allow(dead_code))]
fn vt_output_mode(mode: u32) -> u32 {
    mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING
}

/// Console settings that were in place before [`enter_console`], put back
/// by [`leave_console`].
#[cfg(windows)]
#[derive(Clone, Copy)]
struct SavedConsole {
    input_cp: u32,
    output_cp: u32,
    output_mode: Option<u32>,
}

#[cfg(windows)]
static SAVED_CONSOLE: std::sync::Mutex<Option<SavedConsole>> = std::sync::Mutex::new(None);

/// Prepare the console for drawing. Windows: UTF-8 input/output code
/// pages and VT processing on the active screen buffer, remembering the
/// previous settings. The code page and modes belong to the console, which
/// is shared with the parent shell, so [`leave_console`] must run on every
/// exit path. Idempotent; a no-op elsewhere.
///
/// Input VT mode is deliberately left off: crossterm reads keys as console
/// input records, and turning `ENABLE_VIRTUAL_TERMINAL_INPUT` on would hand
/// it escape sequences it doesn't parse on Windows.
pub fn enter_console() {
    #[cfg(windows)]
    {
        use crossterm_winapi::{ConsoleMode, Handle};
        use windows_sys::Win32::System::Console::{GetConsoleCP, GetConsoleOutputCP, SetConsoleCP, SetConsoleOutputCP};
        let mut saved = SAVED_CONSOLE.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: plain Win32 calls without pointer arguments; they fail
        // (return 0) when the process has no console.
        let (input_cp, output_cp) = unsafe { (GetConsoleCP(), GetConsoleOutputCP()) };
        // CONOUT$ is the console's active screen buffer even when stdout
        // is a pipe (the SDK's default mode).
        let out = Handle::current_out_handle().ok().map(ConsoleMode::from);
        let output_mode = out.as_ref().and_then(|m| m.mode().ok());
        if saved.is_none() {
            *saved = Some(SavedConsole { input_cp, output_cp, output_mode });
        }
        unsafe {
            if output_cp != 0 && output_cp != CP_UTF8 {
                SetConsoleOutputCP(CP_UTF8);
            }
            if input_cp != 0 && input_cp != CP_UTF8 {
                SetConsoleCP(CP_UTF8);
            }
        }
        if let (Some(out), Some(mode)) = (out, output_mode) {
            let _ = out.set_mode(vt_output_mode(mode));
        }
    }
}

/// Undo [`enter_console`]. Safe to call more than once (and from the panic
/// hook); a no-op when nothing was changed.
pub fn leave_console() {
    #[cfg(windows)]
    {
        use crossterm_winapi::{ConsoleMode, Handle};
        use windows_sys::Win32::System::Console::{SetConsoleCP, SetConsoleOutputCP};
        let Some(saved) = SAVED_CONSOLE.lock().unwrap_or_else(|e| e.into_inner()).take() else {
            return;
        };
        if let (Some(mode), Ok(handle)) = (saved.output_mode, Handle::current_out_handle()) {
            let _ = ConsoleMode::from(handle).set_mode(mode);
        }
        // SAFETY: as in enter_console.
        unsafe {
            if saved.output_cp != 0 {
                SetConsoleOutputCP(saved.output_cp);
            }
            if saved.input_cp != 0 {
                SetConsoleCP(saved.input_cp);
            }
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

/// Where crash dumps go: `~/.camouflage`, never the user's working
/// directory. Falls back to the temp dir when there's no home.
pub fn crash_dir() -> PathBuf {
    let dir = home_dir().map(|h| h.join(".camouflage")).unwrap_or_else(std::env::temp_dir);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vt_output_mode_adds_processed_output_and_vt() {
        assert_eq!(vt_output_mode(0), 0x0005);
        // Keeps what was already set (wrap-at-EOL, etc.).
        assert_eq!(vt_output_mode(0x0002), 0x0007);
        assert_eq!(vt_output_mode(0x0007), 0x0007);
    }

    #[test]
    fn console_enter_and_leave_are_safe_to_repeat() {
        // On Unix these are no-ops; on Windows without a console (CI) every
        // call fails quietly. Either way nothing may panic, and leave may
        // run more than once (normal exit, then the panic hook).
        enter_console();
        enter_console();
        leave_console();
        leave_console();
    }
}
