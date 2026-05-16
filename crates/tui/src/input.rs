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
    /// v0.2+: replay controls (only meaningful when `--replay` is active).
    ReplayTogglePlay,
    ReplayStepForward,
    ReplayStepBackward,
    ReplayFaster,
    ReplaySlower,
    ReplayRestart,
    /// v0.2+: toggle the event inspector side panel.
    ToggleInspector,
    /// v0.2+: move inspector cursor within the visible rows.
    InspectorCursorUp,
    InspectorCursorDown,
    /// v0.2+: cycle the row-kind filter (all → errors → tools → patches → permissions → all).
    CycleFilter,
    /// v0.2+: open the inline search prompt (Ctrl+F).
    SearchOpen,
    /// v0.2+: a printable character typed while the search prompt is open.
    SearchChar(char),
    SearchBackspace,
    SearchSubmit,
    SearchClose,
    SearchNext,
    SearchPrev,
    /// v0.2+: bookmark the current focus seq.
    BookmarkAdd,
    /// v0.2+: cycle to the next bookmark.
    BookmarkNext,
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
        Key::CtrlF => Action::SearchOpen,
        Key::Char('q') if buf.is_empty() => Action::Quit,
        Key::Char('r') if buf.is_empty() => Action::Replay,
        Key::Char('i') if buf.is_empty() => Action::ToggleInspector,
        Key::Char('f') if buf.is_empty() => Action::CycleFilter,
        Key::Char('n') if buf.is_empty() => Action::SearchNext,
        Key::Char('N') if buf.is_empty() => Action::SearchPrev,
        Key::Char('m') if buf.is_empty() => Action::BookmarkAdd,
        Key::Char('\'') if buf.is_empty() => Action::BookmarkNext,
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

/// Key handler used while the search prompt is open. Captures printable
/// chars + edit keys; Enter submits, Esc closes.
pub fn handle_key_search(k: Key) -> Action {
    match k {
        Key::CtrlC | Key::Esc => Action::SearchClose,
        Key::Enter => Action::SearchSubmit,
        Key::Backspace => Action::SearchBackspace,
        Key::Char(c) => Action::SearchChar(c),
        _ => Action::None,
    }
}

/// Replay-mode key handler. When `--replay` is active and the input buffer
/// is empty, replay controls take priority over normal text entry. Returns
/// `None` (no action) for unrecognised keys so the caller can fall through
/// to the default handler.
pub fn handle_key_replay(k: Key, buf: &str) -> Option<Action> {
    if !buf.is_empty() {
        return None;
    }
    Some(match k {
        Key::Char(' ') => Action::ReplayTogglePlay,
        Key::Right => Action::ReplayStepForward,
        Key::Left => Action::ReplayStepBackward,
        Key::Char('+') | Key::Char('=') => Action::ReplayFaster,
        Key::Char('-') | Key::Char('_') => Action::ReplaySlower,
        Key::Char('0') => Action::ReplayRestart,
        _ => return None,
    })
}
