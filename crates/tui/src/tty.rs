//! Direct /dev/tty key reader.
//!
//! Why this exists: crossterm 0.28 uses mio/kqueue to poll for terminal
//! events, and on macOS, registering a freshly-opened /dev/tty fd with
//! kqueue's EVFILT_READ returns EINVAL. When stdin is a pipe (the entire
//! point of `cmd | camouflage-tui --stdin-events`), crossterm opens
//! /dev/tty itself, so it hits this path and fails to initialize. The
//! result: `event::poll` returns "Failed to initialize input reader".
//!
//! Workaround: read bytes from /dev/tty ourselves with plain blocking
//! read(2) on a dedicated thread, and parse the small subset of escape
//! sequences we actually use. crossterm remains responsible for raw mode
//! (which works fine — it uses tcsetattr, not kqueue) and drawing.

use std::io;
use std::os::fd::RawFd;
use std::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Esc,
    Tab,
    BackTab,
    Up,
    Down,
    Left,
    Right,
    /// Option+Left on macOS / Alt+Left elsewhere — "jump word left".
    WordLeft,
    /// Option+Right / Alt+Right — "jump word right".
    WordRight,
    /// Option+D / Alt+D — readline "kill word forward" (delete from
    /// cursor to next word boundary).
    MetaD,
    PageUp,
    PageDown,
    Home,
    End,
    /// Forward-delete (fn+Backspace on macOS, Del key elsewhere — CSI 3 ~).
    Delete,
    CtrlC,
    CtrlE,
    CtrlF,
    /// Cursor to start of line (readline convention).
    CtrlA,
    /// Delete to end of line (readline convention).
    CtrlK,
    /// Delete word before cursor (readline convention).
    CtrlW,
    /// Delete to start of line (readline convention).
    CtrlU,
    /// Mouse wheel up (when SGR mouse capture is enabled).
    ScrollUp,
    /// Mouse wheel down (when SGR mouse capture is enabled).
    ScrollDown,
    /// Bracketed-paste payload — the literal text the user pasted,
    /// delivered as a single key so the input layer can insert it
    /// atomically (and embedded newlines / control chars don't trigger
    /// Enter / Ctrl+C mid-paste).
    Paste(String),
}

/// Open /dev/tty for reading and return its raw fd.
pub fn open_tty_for_read() -> io::Result<RawFd> {
    let path = b"/dev/tty\0";
    let fd = unsafe { libc::open(path.as_ptr() as *const _, libc::O_RDONLY | libc::O_NOCTTY) };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(fd)
    }
}

/// Spawn a blocking thread that reads keys from `tty_fd` and forwards them
/// through the returned receiver. The thread terminates when the fd hits
/// EOF or an error.
pub fn spawn_key_reader(tty_fd: RawFd) -> mpsc::Receiver<Key> {
    let (tx, rx) = mpsc::channel::<Key>();
    std::thread::spawn(move || {
        let mut parser = EscParser::new();
        let mut buf = [0u8; 64];
        loop {
            let n = unsafe { libc::read(tty_fd, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n <= 0 {
                break;
            }
            for &b in &buf[..n as usize] {
                if let Some(k) = parser.feed(b) {
                    if tx.send(k).is_err() {
                        return;
                    }
                }
            }
        }
    });
    rx
}

/// Tiny ANSI escape-sequence parser covering the keys we use.
///
/// States:
/// - Ground: each byte either a printable, control, or `ESC` starting a sequence
/// - Esc: after seeing `0x1b`
/// - Csi: after seeing `ESC [`
/// - InPaste: inside a bracketed-paste run (ESC [200~ … ESC [201~) — all
///   bytes are collected literally until the terminator is recognised.
/// - PasteEsc / PasteCsi: transient states used to detect the ESC [201~
///   paste terminator without leaking partial-match bytes into the paste
///   buffer.
struct EscParser {
    state: State,
    csi_buf: Vec<u8>,
    paste_buf: String,
    /// Parameter bytes accumulated while we're trying to recognise the
    /// paste terminator. If recognition fails, these get flushed back
    /// into the paste buffer along with the literal ESC [ that started
    /// the detection attempt.
    paste_csi_buf: Vec<u8>,
}

enum State {
    Ground,
    Esc,
    Csi,
    InPaste,
    PasteEsc,
    PasteCsi,
}

impl EscParser {
    fn new() -> Self {
        Self {
            state: State::Ground,
            csi_buf: Vec::with_capacity(8),
            paste_buf: String::new(),
            paste_csi_buf: Vec::with_capacity(4),
        }
    }

    fn feed(&mut self, b: u8) -> Option<Key> {
        match self.state {
            State::Ground => match b {
                0x01 => Some(Key::CtrlA),
                0x03 => Some(Key::CtrlC),
                0x05 => Some(Key::CtrlE),
                0x06 => Some(Key::CtrlF),
                0x09 => Some(Key::Tab),
                0x0b => Some(Key::CtrlK),
                0x15 => Some(Key::CtrlU),
                0x17 => Some(Key::CtrlW),
                0x0d | 0x0a => Some(Key::Enter),
                0x7f | 0x08 => Some(Key::Backspace),
                0x1b => {
                    self.state = State::Esc;
                    None
                }
                b if b >= 0x20 && b < 0x7f => Some(Key::Char(b as char)),
                _ => None,
            },
            State::Esc => match b {
                b'[' => {
                    self.state = State::Csi;
                    self.csi_buf.clear();
                    None
                }
                // macOS Terminal.app's default Option+Left / Option+Right
                // bindings: ESC b (word backward) and ESC f (word forward).
                b'b' => { self.state = State::Ground; Some(Key::WordLeft) }
                b'f' => { self.state = State::Ground; Some(Key::WordRight) }
                // ESC d — Option+D / Alt+D, "kill word forward". Mirror of
                // Ctrl+W (which kills the word backward).
                b'd' => { self.state = State::Ground; Some(Key::MetaD) }
                0x1b => None,
                _ => {
                    // Lone ESC (or Alt-X we don't handle); treat as Esc.
                    self.state = State::Ground;
                    Some(Key::Esc)
                }
            },
            State::Csi => {
                // CSI ends on a byte in 0x40..=0x7e
                if (0x40..=0x7e).contains(&b) {
                    let s = std::str::from_utf8(&self.csi_buf).unwrap_or("");
                    // Bracketed-paste start (ESC [ 200 ~). Switch to InPaste
                    // before the normal state-reset runs, so the trailing
                    // assignment below doesn't undo us.
                    if b == b'~' && s == "200" {
                        self.csi_buf.clear();
                        self.paste_buf.clear();
                        self.state = State::InPaste;
                        return None;
                    }
                    let k = match b {
                        b'A' => Some(Key::Up),
                        b'B' => Some(Key::Down),
                        b'C' => {
                            // ESC [1;3C = Meta+Right (alt for Option+Right).
                            if s == "1;3" { Some(Key::WordRight) } else { Some(Key::Right) }
                        }
                        b'D' => {
                            if s == "1;3" { Some(Key::WordLeft) } else { Some(Key::Left) }
                        }
                        b'H' => Some(Key::Home),
                        b'F' => Some(Key::End),
                        b'Z' => Some(Key::BackTab),
                        b'M' | b'm' => {
                            // SGR mouse: ESC [ < button ; x ; y M|m
                            // We only care about wheel events (buttons 64/65).
                            if let Some(rest) = s.strip_prefix('<') {
                                let parts: Vec<&str> = rest.split(';').collect();
                                if let Some(btn) = parts.first().and_then(|b| b.parse::<u32>().ok()) {
                                    match btn {
                                        64 => Some(Key::ScrollUp),
                                        65 => Some(Key::ScrollDown),
                                        _ => None,
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                        b'~' => {
                            // Function keys with optional modifier suffix.
                            // We ignore modifiers and just look at the base
                            // number before the first ';' (e.g. "5;3" → 5).
                            let base = s.split(';').next().unwrap_or("");
                            match base {
                                "1" | "7" => Some(Key::Home),
                                "3" => Some(Key::Delete),
                                "4" | "8" => Some(Key::End),
                                "5" => Some(Key::PageUp),
                                "6" => Some(Key::PageDown),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    self.csi_buf.clear();
                    self.state = State::Ground;
                    k
                } else {
                    self.csi_buf.push(b);
                    None
                }
            }
            State::InPaste => {
                // Collect bytes literally. ESC starts a terminator-detection
                // attempt; if it doesn't match, the bytes get flushed back.
                if b == 0x1b {
                    self.state = State::PasteEsc;
                    None
                } else {
                    self.paste_buf.push(b as char);
                    None
                }
            }
            State::PasteEsc => {
                if b == b'[' {
                    self.paste_csi_buf.clear();
                    self.state = State::PasteCsi;
                    None
                } else {
                    // False alarm — flush the ESC + this byte back into
                    // the paste buffer and resume collecting.
                    self.paste_buf.push(0x1b as char);
                    self.paste_buf.push(b as char);
                    self.state = State::InPaste;
                    None
                }
            }
            State::PasteCsi => {
                if (0x40..=0x7e).contains(&b) {
                    let s = std::str::from_utf8(&self.paste_csi_buf).unwrap_or("");
                    if b == b'~' && s == "201" {
                        // Real terminator — emit the paste payload.
                        self.paste_csi_buf.clear();
                        self.state = State::Ground;
                        let payload = std::mem::take(&mut self.paste_buf);
                        Some(Key::Paste(payload))
                    } else {
                        // Some other CSI sequence appeared inside the
                        // paste — push the literal bytes back and resume
                        // collecting. (Rare in real pastes but possible if
                        // the user pastes ANSI-escaped text.)
                        self.paste_buf.push(0x1b as char);
                        self.paste_buf.push('[');
                        for &pb in &self.paste_csi_buf {
                            self.paste_buf.push(pb as char);
                        }
                        self.paste_buf.push(b as char);
                        self.paste_csi_buf.clear();
                        self.state = State::InPaste;
                        None
                    }
                } else {
                    self.paste_csi_buf.push(b);
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(p: &mut EscParser, bytes: &[u8]) -> Vec<Key> {
        let mut out = vec![];
        for &b in bytes {
            if let Some(k) = p.feed(b) {
                out.push(k);
            }
        }
        out
    }

    #[test]
    fn parse_arrow_keys() {
        let mut p = EscParser::new();
        let keys = feed_all(&mut p, b"\x1b[A\x1b[B\x1b[C\x1b[D");
        assert!(matches!(keys.as_slice(), [Key::Up, Key::Down, Key::Right, Key::Left]));
    }

    #[test]
    fn parse_pageup_pagedown_end() {
        let mut p = EscParser::new();
        let keys = feed_all(&mut p, b"\x1b[5~\x1b[6~\x1b[F");
        assert!(matches!(keys.as_slice(), [Key::PageUp, Key::PageDown, Key::End]));
    }

    #[test]
    fn parse_forward_delete() {
        let mut p = EscParser::new();
        // CSI 3 ~ — the Delete key (fn+Backspace on macOS).
        let keys = feed_all(&mut p, b"\x1b[3~");
        assert!(matches!(keys.as_slice(), [Key::Delete]));
    }

    #[test]
    fn parse_ctrl_k() {
        let mut p = EscParser::new();
        // 0x0b (VT) is Ctrl+K — readline "kill to end of line".
        let keys = feed_all(&mut p, &[0x0b]);
        assert!(matches!(keys.as_slice(), [Key::CtrlK]));
    }

    #[test]
    fn parse_meta_d() {
        let mut p = EscParser::new();
        // ESC d — Option+D on macOS, "kill word forward".
        let keys = feed_all(&mut p, b"\x1bd");
        assert!(matches!(keys.as_slice(), [Key::MetaD]));
    }

    #[test]
    fn parse_printable_and_enter() {
        let mut p = EscParser::new();
        let keys = feed_all(&mut p, b"hi\r");
        assert_eq!(keys.len(), 3);
        assert!(matches!(keys[0], Key::Char('h')));
        assert!(matches!(keys[1], Key::Char('i')));
        assert!(matches!(keys[2], Key::Enter));
    }

    #[test]
    fn parse_ctrl_c() {
        let mut p = EscParser::new();
        let keys = feed_all(&mut p, &[0x03]);
        assert!(matches!(keys.as_slice(), [Key::CtrlC]));
    }

    #[test]
    fn lone_esc_emits_esc() {
        let mut p = EscParser::new();
        // ESC then a non-[ byte
        let keys = feed_all(&mut p, b"\x1bz");
        assert!(matches!(keys.first(), Some(Key::Esc)));
    }

    #[test]
    fn parse_tab_and_backtab() {
        let mut p = EscParser::new();
        let keys = feed_all(&mut p, b"\t\x1b[Z");
        assert!(matches!(keys.as_slice(), [Key::Tab, Key::BackTab]));
    }

    #[test]
    fn parse_word_jumps_macos_option() {
        let mut p = EscParser::new();
        // ESC b = Option+Left, ESC f = Option+Right on macOS Terminal.app.
        let keys = feed_all(&mut p, b"\x1bb\x1bf");
        assert!(matches!(keys.as_slice(), [Key::WordLeft, Key::WordRight]));
    }

    #[test]
    fn parse_word_jumps_xterm_modifier() {
        let mut p = EscParser::new();
        // ESC [ 1 ; 3 D = Meta+Left (xterm).
        let keys = feed_all(&mut p, b"\x1b[1;3D\x1b[1;3C");
        assert!(matches!(keys.as_slice(), [Key::WordLeft, Key::WordRight]));
    }

    #[test]
    fn parse_sgr_mouse_scroll() {
        let mut p = EscParser::new();
        // SGR mouse: ESC [ < 64 ; 10 ; 5 M = wheel up at (10,5).
        let keys = feed_all(&mut p, b"\x1b[<64;10;5M\x1b[<65;10;5M");
        assert!(matches!(keys.as_slice(), [Key::ScrollUp, Key::ScrollDown]));
    }

    #[test]
    fn parse_bracketed_paste_basic() {
        let mut p = EscParser::new();
        // ESC[200~hello world\nfoo ESC[201~ — pastes a multi-line block.
        let keys = feed_all(&mut p, b"\x1b[200~hello world\nfoo\x1b[201~");
        match keys.as_slice() {
            [Key::Paste(s)] => assert_eq!(s, "hello world\nfoo"),
            other => panic!("expected single Paste, got {:?}", other),
        }
    }

    #[test]
    fn parse_bracketed_paste_does_not_emit_enter_or_ctrl_c() {
        // The whole point: \r and \x03 inside a paste must not become
        // Key::Enter or Key::CtrlC — they ride along as part of the
        // payload so the host doesn't submit or quit mid-paste.
        let mut p = EscParser::new();
        let keys = feed_all(&mut p, b"\x1b[200~a\rb\x03c\x1b[201~");
        match keys.as_slice() {
            [Key::Paste(s)] => assert_eq!(s, "a\rb\x03c"),
            other => panic!("expected single Paste, got {:?}", other),
        }
    }

    #[test]
    fn parse_bracketed_paste_then_normal_key() {
        let mut p = EscParser::new();
        let keys = feed_all(&mut p, b"\x1b[200~x\x1b[201~\r");
        match keys.as_slice() {
            [Key::Paste(s), Key::Enter] => assert_eq!(s, "x"),
            other => panic!("expected [Paste, Enter], got {:?}", other),
        }
    }
}
