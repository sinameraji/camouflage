use crate::tty::Key;

pub enum Action {
    None,
    Quit,
    SubmitInput(String),
    ScrollUp(u16),
    ScrollDown(u16),
    JumpToLatest,
    CancelStream,
    Replay,
    /// v0.1.5+: user resolved a pending PermissionRequested.
    PermissionAllowOnce,
    PermissionAllowSession,
    PermissionDeny,
}

/// Variant of `handle_key` used while a PermissionRequested is pending.
/// Only quit + permission-choice keys are honoured; everything else is ignored.
pub fn handle_key_permission(k: Key) -> Action {
    match k {
        Key::CtrlC => Action::Quit,
        Key::Char('1') => Action::PermissionAllowOnce,
        Key::Char('2') => Action::PermissionAllowSession,
        Key::Char('3') | Key::Char('?') => Action::PermissionDeny,
        Key::Esc => Action::PermissionDeny,
        _ => Action::None,
    }
}

pub fn handle_key(k: Key, buf: &mut String) -> Action {
    match k {
        Key::CtrlC => Action::Quit,
        Key::CtrlE => Action::JumpToLatest,
        Key::Char('q') if buf.is_empty() => Action::Quit,
        Key::Char('r') if buf.is_empty() => Action::Replay,
        Key::Char(c) => {
            buf.push(c);
            Action::None
        }
        Key::Enter => {
            if buf.is_empty() {
                Action::None
            } else {
                let text = std::mem::take(buf);
                Action::SubmitInput(text)
            }
        }
        Key::Esc => Action::CancelStream,
        Key::Up => Action::ScrollUp(1),
        Key::Down => Action::ScrollDown(1),
        Key::PageUp => Action::ScrollUp(10),
        Key::PageDown => Action::ScrollDown(10),
        Key::End => Action::JumpToLatest,
        Key::Home => Action::ScrollUp(u16::MAX),
        Key::Backspace => {
            buf.pop();
            Action::None
        }
        _ => Action::None,
    }
}
