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
