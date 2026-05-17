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
    /// v0.4+: toggle the help overlay.
    ToggleHelp,
    /// v0.4+: toggle the live-metrics overlay (events/sec, frame time, RSS).
    ToggleMetrics,
    /// v0.4+: cycle to the next built-in theme.
    CycleTheme,
    /// v0.4.5+: toggle the tool-output overlay (shows most-recent tool's
    /// captured stdout/stderr).
    ToggleToolOutput,
}

/// Variant of `handle_key` used while a PermissionRequested is pending.
/// 1/2/3 trigger the response; printable characters (other than the
/// reserved digits) are buffered as the `feedback` field of the eventual
/// `PermissionResponse`, so a user can type "deny because the path is
/// outside the workspace" and have it ride along with the choice.
pub fn handle_key_permission(k: Key, feedback: &mut String) -> Action {
    match k {
        Key::CtrlC => Action::Quit,
        Key::Char('1') => Action::PermissionAllowOnce,
        Key::Char('2') => Action::PermissionAllowSession,
        Key::Char('3') => Action::PermissionDeny,
        Key::Esc => Action::PermissionDeny,
        Key::Backspace => {
            feedback.pop();
            Action::None
        }
        Key::Char(c) => {
            // Don't let reserved digits leak into feedback. Other
            // printable chars accumulate.
            feedback.push(c);
            Action::None
        }
        _ => Action::None,
    }
}

pub fn handle_key(k: Key, buf: &mut String, cursor: &mut usize) -> Action {
    // Normalise the cursor if it's somehow past the end (defensive).
    let total = buf.chars().count();
    if *cursor > total {
        *cursor = total;
    }
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
        Key::Char('?') if buf.is_empty() => Action::ToggleHelp,
        Key::Char('M') if buf.is_empty() => Action::ToggleMetrics,
        Key::Char('T') if buf.is_empty() => Action::CycleTheme,
        Key::Char('X') if buf.is_empty() => Action::ToggleToolOutput,
        Key::Char(c) => {
            insert_at_cursor(buf, cursor, c);
            Action::None
        }
        Key::Enter => {
            if buf.is_empty() {
                Action::None
            } else {
                let text = std::mem::take(buf);
                *cursor = 0;
                Action::SubmitInput(text)
            }
        }
        Key::Esc => Action::CancelStream,
        // Cursor navigation: when there IS text, Left/Right/Home/End move
        // the insertion point. When the input is empty, Home falls through
        // to "scroll to top" and End to "jump to latest" — keeping the
        // existing v0.2 transcript-scroll keybinds usable on an empty buf.
        Key::Left if !buf.is_empty() => {
            if *cursor > 0 { *cursor -= 1; }
            Action::None
        }
        Key::Right if !buf.is_empty() => {
            if *cursor < total { *cursor += 1; }
            Action::None
        }
        Key::WordLeft => {
            *cursor = word_boundary_left(buf, *cursor);
            Action::None
        }
        Key::WordRight => {
            *cursor = word_boundary_right(buf, *cursor);
            Action::None
        }
        Key::Home if !buf.is_empty() => { *cursor = 0; Action::None }
        Key::End if !buf.is_empty() => { *cursor = total; Action::None }
        Key::Up => Action::ScrollUp(1),
        Key::Down => Action::ScrollDown(1),
        // Mouse wheel scrolls the transcript by a few lines per notch.
        // Without SGR mouse capture the terminal would translate the wheel
        // into Up/Down keys, which the input-history walker would consume —
        // hence the explicit scroll mapping.
        Key::ScrollUp => Action::ScrollUp(3),
        Key::ScrollDown => Action::ScrollDown(3),
        Key::PageUp => Action::ScrollUp(10),
        Key::PageDown => Action::ScrollDown(10),
        Key::End => Action::JumpToLatest,
        Key::Home => Action::ScrollUp(u16::MAX),
        Key::Backspace => {
            delete_before_cursor(buf, cursor);
            Action::None
        }
        _ => Action::None,
    }
}

/// Insert character `c` at the cursor (a character index, not byte
/// offset). Advances cursor.
pub fn insert_at_cursor(buf: &mut String, cursor: &mut usize, c: char) {
    let byte_idx = byte_index_of_char(buf, *cursor);
    buf.insert(byte_idx, c);
    *cursor += 1;
}

/// Delete the character before the cursor (Backspace). Decrements cursor.
pub fn delete_before_cursor(buf: &mut String, cursor: &mut usize) {
    if *cursor == 0 { return; }
    let new_byte_idx = byte_index_of_char(buf, *cursor - 1);
    let old_byte_idx = byte_index_of_char(buf, *cursor);
    buf.replace_range(new_byte_idx..old_byte_idx, "");
    *cursor -= 1;
}

/// Byte offset of the `n`-th character (or buf.len() if n is at/past end).
fn byte_index_of_char(buf: &str, n: usize) -> usize {
    buf.char_indices().nth(n).map(|(i, _)| i).unwrap_or(buf.len())
}

/// Word-boundary navigation. "Word" here is a maximal run of alphanumeric
/// characters (incl. underscores); everything else is a separator.
pub fn word_boundary_left(buf: &str, cursor: usize) -> usize {
    if cursor == 0 { return 0; }
    let chars: Vec<char> = buf.chars().collect();
    let mut i = cursor;
    // Skip separators backward
    while i > 0 && !is_word_char(chars[i - 1]) { i -= 1; }
    // Skip word chars backward
    while i > 0 && is_word_char(chars[i - 1]) { i -= 1; }
    i
}

pub fn word_boundary_right(buf: &str, cursor: usize) -> usize {
    let chars: Vec<char> = buf.chars().collect();
    let n = chars.len();
    let mut i = cursor;
    while i < n && !is_word_char(chars[i]) { i += 1; }
    while i < n && is_word_char(chars[i]) { i += 1; }
    i
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
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
