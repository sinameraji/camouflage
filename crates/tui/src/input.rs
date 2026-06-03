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
    /// v0.4.8+: move the highlighted permission option up/down + commit.
    PermissionSelectPrev,
    PermissionSelectNext,
    PermissionConfirmSelected,
    /// v0.4.8+: toggle the in-modal `?` help overlay (Ink parity).
    PermissionToggleHelp,
    /// v0.2+: replay controls (only meaningful when `--replay` is active).
    ReplayTogglePlay,
    ReplayStepForward,
    ReplayStepBackward,
    ReplayFaster,
    ReplaySlower,
    ReplayRestart,
    /// Jump to N% of the way through replay. Value is 0..=100.
    ReplayJumpPct(u8),
    /// Jump to the end of replay (last event applied).
    ReplayJumpEnd,
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
    /// v0.4.9+: toggle terminal mouse capture. While ON the wheel scrolls
    /// the transcript (default). While OFF the user can drag-select text
    /// with the cursor — at the cost of losing wheel scroll until they
    /// turn it back on. Workaround for terminals where Option/Shift-click
    /// doesn't bypass mouse reporting.
    ToggleMouseCapture,
}

/// Variant of `handle_key` used while a PermissionRequested is pending.
/// 1/2/3 trigger the response; printable characters (other than the
/// reserved digits) are buffered as the `feedback` field of the eventual
/// `PermissionResponse`, so a user can type "deny because the path is
/// outside the workspace" and have it ride along with the choice.
pub fn handle_key_permission(k: Key, feedback: &mut String) -> Action {
    match k {
        Key::CtrlC => Action::Quit,
        // Direct digit shortcuts (mirrors Ink's Alt+1/2/3 — but bare
        // digits work too since this handler only fires when a
        // permission is pending).
        Key::Char('1') => Action::PermissionAllowOnce,
        Key::Char('2') => Action::PermissionAllowSession,
        Key::Char('3') => Action::PermissionDeny,
        // Arrow / vim navigation + Enter confirms the highlighted row.
        Key::Up => Action::PermissionSelectPrev,
        Key::Down => Action::PermissionSelectNext,
        Key::Char('k') => Action::PermissionSelectPrev,
        Key::Char('j') => Action::PermissionSelectNext,
        Key::Enter => Action::PermissionConfirmSelected,
        // ? overlays a help panel. Any subsequent key dismisses (handled
        // in app.rs by clearing help_open on the next iteration).
        Key::Char('?') => Action::PermissionToggleHelp,
        // Esc here used to *only* deny the current permission, which
        // made it feel like Esc "did nothing" when the user actually
        // wanted to abort the whole running turn. Map it to the same
        // path as Ctrl+C's single press: the CancelStream handler in
        // app.rs denies the pending permission AND emits CancelRequested.
        Key::Esc => Action::CancelStream,
        Key::Backspace => {
            feedback.pop();
            Action::None
        }
        Key::Char(c) => {
            feedback.push(c);
            Action::None
        }
        Key::Paste(text) => {
            // Same rationale as handle_key: a paste into the permission
            // feedback box should land as a single block of literal text.
            feedback.push_str(&text);
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
        // Ctrl+A and Ctrl+E pull double duty: when input has content,
        // they're readline-style "cursor to line start/end". When input
        // is empty, they fall through to their v0.2 transcript-scroll
        // bindings (CtrlA was unused → now usable as line-start).
        Key::CtrlA if !buf.is_empty() => { *cursor = 0; Action::None }
        Key::CtrlE if !buf.is_empty() => { *cursor = total; Action::None }
        Key::CtrlE => Action::JumpToLatest,
        Key::CtrlF => Action::SearchOpen,
        // CtrlA when buf empty: no useful default; no-op so it doesn't
        // accidentally fire something else.
        Key::CtrlA => Action::None,
        // Removed vim-style lowercase single-char shortcuts (q/r/i/f/n/N/m/').
        // They surprised users by quitting / toggling state mid-typing
        // session — the most common report being "I typed 'q' and the
        // whole thing exited". Power-users now use slash commands
        // (/quit, /help) and the uppercase keybinds below which are
        // unlikely to collide with normal prose.
        Key::Char('?') if buf.is_empty() => Action::ToggleHelp,
        Key::Char('M') if buf.is_empty() => Action::ToggleMetrics,
        Key::Char('T') if buf.is_empty() => Action::CycleTheme,
        Key::Char('X') if buf.is_empty() => Action::ToggleToolOutput,
        Key::Char('S') if buf.is_empty() => Action::ToggleMouseCapture,
        // Readline-style deletions:
        //   Ctrl+W (0x17) → delete word before cursor
        //   Ctrl+U (0x15) → delete to start of line
        // These mirror what most macOS users hit when Option+Delete /
        // Cmd+Delete don't get through (Terminal.app's defaults vary).
        Key::CtrlW => {
            let new_cursor = word_boundary_left(buf, *cursor);
            let from = byte_index_of_char(buf, new_cursor);
            let to = byte_index_of_char(buf, *cursor);
            buf.replace_range(from..to, "");
            *cursor = new_cursor;
            Action::None
        }
        Key::CtrlU => {
            let to = byte_index_of_char(buf, *cursor);
            buf.replace_range(0..to, "");
            *cursor = 0;
            Action::None
        }
        Key::CtrlK => {
            // Readline "kill to end of line": drop everything from the
            // cursor to the end of the buffer. Cursor stays put — it's
            // now at the (new) end of the line.
            let from = byte_index_of_char(buf, *cursor);
            buf.truncate(from);
            Action::None
        }
        Key::MetaD => {
            // Readline "kill word forward": drop from cursor up to (but
            // not including) the start of the next word. Mirror of
            // Ctrl+W. Cursor stays put.
            let new_end = word_boundary_right(buf, *cursor);
            let from = byte_index_of_char(buf, *cursor);
            let to = byte_index_of_char(buf, new_end);
            buf.replace_range(from..to, "");
            Action::None
        }
        Key::Char(c) => {
            insert_at_cursor(buf, cursor, c);
            Action::None
        }
        Key::Paste(text) => {
            // Bracketed paste payload — insert atomically at the cursor.
            // Newlines / control chars are preserved as literal characters
            // (the parser already filtered out the wrapping ESC sequences),
            // so a multi-line paste stays multi-line and never accidentally
            // submits the input mid-stream.
            for c in text.chars() {
                insert_at_cursor(buf, cursor, c);
            }
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
        Key::ShiftEnter => {
            insert_at_cursor(buf, cursor, '\n');
            Action::None
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
        Key::Delete => {
            delete_after_cursor(buf, cursor);
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

/// Delete the character at the cursor (forward-delete / Delete key). The
/// cursor stays in place — the next character slides left to fill the gap.
pub fn delete_after_cursor(buf: &mut String, cursor: &mut usize) {
    let total = buf.chars().count();
    if *cursor >= total { return; }
    let from = byte_index_of_char(buf, *cursor);
    let to = byte_index_of_char(buf, *cursor + 1);
    buf.replace_range(from..to, "");
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
        // 1..=9 jump to 10%..90% of the timeline. Cheap muscle memory:
        // tap "5" and you're halfway through the session.
        Key::Char(c @ '1'..='9') => {
            let pct = (c as u8 - b'0') * 10;
            Action::ReplayJumpPct(pct)
        }
        // Shift+G / "G" jumps to the very end (matches vim).
        Key::Char('G') => Action::ReplayJumpEnd,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_allow_once(a: &Action) -> bool { matches!(a, Action::PermissionAllowOnce) }
    fn is_allow_session(a: &Action) -> bool { matches!(a, Action::PermissionAllowSession) }
    fn is_deny(a: &Action) -> bool { matches!(a, Action::PermissionDeny) }
    fn is_prev(a: &Action) -> bool { matches!(a, Action::PermissionSelectPrev) }
    fn is_next(a: &Action) -> bool { matches!(a, Action::PermissionSelectNext) }
    fn is_confirm(a: &Action) -> bool { matches!(a, Action::PermissionConfirmSelected) }

    #[test]
    fn permission_digit_shortcuts_map_to_choices() {
        let mut fb = String::new();
        assert!(is_allow_once(&handle_key_permission(Key::Char('1'), &mut fb)));
        assert!(is_allow_session(&handle_key_permission(Key::Char('2'), &mut fb)));
        assert!(is_deny(&handle_key_permission(Key::Char('3'), &mut fb)));
        // Digits MUST NOT also leak into the feedback buffer.
        assert!(fb.is_empty());
    }

    #[test]
    fn permission_arrow_keys_navigate_and_enter_confirms() {
        let mut fb = String::new();
        assert!(is_next(&handle_key_permission(Key::Down, &mut fb)));
        assert!(is_prev(&handle_key_permission(Key::Up, &mut fb)));
        assert!(is_next(&handle_key_permission(Key::Char('j'), &mut fb)));
        assert!(is_prev(&handle_key_permission(Key::Char('k'), &mut fb)));
        assert!(is_confirm(&handle_key_permission(Key::Enter, &mut fb)));
    }

    #[test]
    fn permission_non_digit_chars_go_into_feedback() {
        let mut fb = String::new();
        for c in "deny because path is unsafe".chars() {
            let _ = handle_key_permission(Key::Char(c), &mut fb);
        }
        assert_eq!(fb, "deny because path is unsafe");
        // Backspace removes the last char.
        let _ = handle_key_permission(Key::Backspace, &mut fb);
        assert_eq!(fb, "deny because path is unsaf");
    }
}
