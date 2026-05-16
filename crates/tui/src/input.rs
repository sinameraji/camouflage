use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub enum Action {
    None,
    Quit,
    SubmitInput(String),
    ScrollUp(u16),
    ScrollDown(u16),
    JumpToLatest,
    CancelStream,
    Replay,
}

pub fn handle_key(k: KeyEvent, buf: &mut String) -> Action {
    match (k.code, k.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => Action::Quit,
        (KeyCode::Char('e'), KeyModifiers::CONTROL) => Action::JumpToLatest,
        (KeyCode::Char('q'), _) if buf.is_empty() => Action::Quit,
        (KeyCode::Char('r'), _) if buf.is_empty() => Action::Replay,
        (KeyCode::Char('b'), _) if buf.is_empty() => Action::None, // benchmark hook (reserved)
        (KeyCode::Enter, _) => {
            if buf.is_empty() {
                Action::None
            } else {
                let text = std::mem::take(buf);
                Action::SubmitInput(text)
            }
        }
        (KeyCode::Esc, _) => Action::CancelStream,
        (KeyCode::Up, _) => Action::ScrollUp(1),
        (KeyCode::Down, _) => Action::ScrollDown(1),
        (KeyCode::PageUp, _) => Action::ScrollUp(10),
        (KeyCode::PageDown, _) => Action::ScrollDown(10),
        (KeyCode::End, _) => Action::JumpToLatest,
        (KeyCode::Backspace, _) => {
            buf.pop();
            Action::None
        }
        (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) && !m.contains(KeyModifiers::ALT) => {
            buf.push(c);
            Action::None
        }
        _ => Action::None,
    }
}
