//! `--ui inline`: the inline renderer (docs/inline-redesign.md §3–4).
//!
//! Finished output goes into the terminal's normal scrollback; only a small
//! live region is redrawn. The loop is event-driven: it wakes for host
//! events, key presses, and the session's own next deadline (a spinner
//! frame, a message expiring), and sleeps with no timers at all when idle.
//! Every event is still persisted to SQLite before it is shown.

use anyhow::{Context, Result};
use camouflage_headless::{NdjsonDecoder, NdjsonError};
use camouflage_inline::session::{Key, Session, UNFOCUSED_FRAME_MS};
use camouflage_inline::{InlineTerminal, Theme};
use camouflage_protocol::{Event, EventType, SCHEMA_VERSION};
use camouflage_store::{EventStore, SqliteStore};
use crossterm::event::{Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::collections::HashSet;
use std::io::Write;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;
use uuid::Uuid;

pub struct InlineConfig {
    pub store: SqliteStore,
    pub stdin_events: bool,
    pub emit_responses: bool,
    pub responses_fd: Option<i32>,
    pub app_title: String,
}

enum Input {
    Host(Event),
    HostClosed,
    Term(TermEvent),
    /// The session's deadline passed (spinner frame, message expiry).
    Tick,
}

pub async fn run(cfg: InlineConfig) -> Result<()> {
    let InlineConfig { store, stdin_events, emit_responses, responses_fd, app_title } = cfg;
    let store = Arc::new(store);
    let session_id = Uuid::new_v4();
    let seq = Arc::new(AtomicI64::new(store.latest_seq(session_id).unwrap_or(-1).max(-1) + 1));

    // Render to stdout when it's a terminal; otherwise (the SDK's default
    // piped mode) draw on /dev/tty and leave stdout for outbound events.
    let stdout_is_tty = crossterm::tty::IsTty::is_tty(&std::io::stdout());
    let screen: Box<dyn Write + Send> = if stdout_is_tty {
        Box::new(std::io::stdout())
    } else {
        Box::new(crate::platform::open_console_writer().context("opening the terminal for drawing")?)
    };

    std::thread::spawn(camouflage_inline::highlight::warm);
    let outbound = spawn_outbound(responses_fd, emit_responses, stdout_is_tty)?;
    let (tx, mut rx) = mpsc::channel::<Input>(4096);

    let writer = spawn_store_writer(store.clone());
    if stdin_events {
        spawn_host_reader(session_id, tx.clone(), writer.0.clone(), seq.clone());
    }
    spawn_key_reader(tx.clone());
    spawn_signal_handler(tx.clone());
    drop(tx);

    let (w, h) = crossterm::terminal::size().unwrap_or((80, 24));
    let theme = detect_theme();
    let mut term = InlineTerminal::new(screen, w, h);
    let mut session = Session::new(app_title, theme, term.content_width(), h as usize);

    enter_terminal_modes()?;
    install_panic_hook();

    // A synthetic SessionStarted for the store; the session ignores it and
    // waits for the host's own (which may carry a title and welcome lines).
    persist(&store, &seq, session_id, EventType::SessionStarted, serde_json::json!({ "synthetic": true }));

    let mut last_frame = None;
    // While the terminal is unfocused, draws are batched to one per
    // UNFOCUSED_FRAME_MS; `deferred` means there's an undrawn change.
    let mut last_draw: i64 = 0;
    let mut deferred = false;
    // Loop statistics, written on exit when CAMOUFLAGE_STATS_PATH is set
    // (the idle soak test uses these to prove idle means no wakeups).
    let mut wakeups: u64 = 0;
    let mut frames: u64 = 0;
    let mut by_source: std::collections::BTreeMap<String, u64> = Default::default();
    let result: Result<()> = async {
        loop {
            let now = now_ms();
            let mut deadline = session.next_wakeup(now);
            if deferred {
                let at = last_draw + UNFOCUSED_FRAME_MS;
                deadline = Some(deadline.map_or(at, |d| d.min(at)));
            }
            let input = match deadline {
                Some(at) => {
                    let wait = std::time::Duration::from_millis((at - now).max(1) as u64);
                    match tokio::time::timeout(wait, rx.recv()).await {
                        Ok(v) => v,
                        Err(_) => Some(Input::Tick),
                    }
                }
                None => rx.recv().await,
            };
            let Some(first) = input else { break };
            wakeups += 1;
            let source = match &first {
                Input::Host(_) => "host".to_string(),
                Input::HostClosed => "host_closed".to_string(),
                Input::Tick => "tick".to_string(),
                Input::Term(TermEvent::Key(k)) => format!("key:{:?}", k.code),
                Input::Term(other) => format!("term:{}", format!("{other:?}").split('(').next().unwrap_or("?")),
            };
            *by_source.entry(source).or_default() += 1;

            // Handle this input and anything else already queued, then draw
            // once: a burst of tokens costs one frame, not one per token.
            let mut pending = Some(first);
            let mut handled = 0;
            let mut host_closed = false;
            // Anything the user did is drawn at once, focused or not.
            let mut urgent = false;
            while let Some(inp) = pending.take() {
                let now = now_ms();
                if matches!(inp, Input::Term(_)) {
                    urgent = true;
                }
                match inp {
                    Input::Host(ev) => session.apply(&ev, now),
                    Input::HostClosed => host_closed = true,
                    Input::Term(TermEvent::FocusLost) => session.focused = false,
                    Input::Term(TermEvent::FocusGained) => session.focused = true,
                    Input::Term(TermEvent::Resize(w, h)) => {
                        session.resize((w as usize).saturating_sub(1), h as usize);
                        let frame = session.live_frame(now);
                        term.resize(w, h, &frame)?;
                        last_frame = Some(frame);
                    }
                    Input::Term(TermEvent::Paste(text)) => session.key(Key::Paste(text), now),
                    Input::Term(TermEvent::Key(k)) => {
                        if let Some(key) = map_key(k) {
                            session.key(key, now);
                        }
                    }
                    Input::Term(_) | Input::Tick => {}
                }
                handled += 1;
                if handled < 1000 {
                    pending = rx.try_recv().ok();
                }
            }

            for out in session.take_outbound() {
                let ev = Event {
                    id: Uuid::new_v4(),
                    session_id,
                    seq: seq.fetch_add(1, Ordering::Relaxed),
                    timestamp_ms: now_ms(),
                    schema_version: SCHEMA_VERSION,
                    event_type: out.event_type,
                    payload: out.payload,
                };
                if let Some(tx) = &outbound {
                    let _ = tx.send(ev);
                }
            }

            let now = now_ms();
            if !session.focused && !urgent && !session.clear_screen && !session.exit && !host_closed && now - last_draw < UNFOCUSED_FRAME_MS {
                deferred = true;
                continue;
            }
            deferred = false;
            last_draw = now;
            if session.clear_screen {
                session.clear_screen = false;
                term.get_mut().write_all(b"\x1b[2J\x1b[H")?;
                term.clear_live()?;
                last_frame = None;
            }
            let history = session.take_history(now);
            let frame = session.live_frame(now);
            if !history.is_empty() {
                term.print(&history, &frame)?;
                last_frame = Some(frame);
                frames += 1;
            } else if last_frame.as_ref() != Some(&frame) {
                term.draw(&frame)?;
                last_frame = Some(frame);
                frames += 1;
            }

            if session.exit || host_closed {
                break;
            }
        }
        Ok(())
    }
    .await;

    // Leave the transcript on screen, drop the input box, restore the tty.
    let now = now_ms();
    session.resize(term.content_width(), term.size().1 as usize);
    let history = session.take_history(now);
    let _ = term.clear_live();
    if !history.is_empty() {
        let _ = term.print(&history, &Default::default());
    }
    let _ = term.finish();
    leave_terminal_modes();
    // Flush what the writer still holds before the final event.
    let (writer_tx, writer_thread) = writer;
    let _ = writer_tx.send(None);
    if let Some(t) = writer_thread {
        let _ = t.join();
    }
    persist(&store, &seq, session_id, EventType::SessionEnded, serde_json::json!({}));
    if let Some(path) = std::env::var_os("CAMOUFLAGE_STATS_PATH") {
        let _ = std::fs::write(path, serde_json::json!({ "wakeups": wakeups, "frames": frames, "by_source": by_source }).to_string());
    }
    result
}

/// Map a crossterm key to a session key. Releases are ignored.
fn map_key(k: KeyEvent) -> Option<Key> {
    if k.kind == KeyEventKind::Release {
        return None;
    }
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    let shift = k.modifiers.contains(KeyModifiers::SHIFT);
    Some(match k.code {
        KeyCode::Char(c) if ctrl => Key::Ctrl(c.to_ascii_lowercase()),
        KeyCode::Char('b') if alt => Key::Left { word: true },
        KeyCode::Char('f') if alt => Key::Right { word: true },
        KeyCode::Char('d') if alt => Key::Delete { word: true },
        KeyCode::Char(c) => Key::Text(c.to_string()),
        KeyCode::Enter => Key::Enter { shift, alt },
        KeyCode::Esc => Key::Esc,
        KeyCode::Tab if shift => Key::BackTab,
        KeyCode::Tab => Key::Tab,
        KeyCode::BackTab => Key::BackTab,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left { word: alt || ctrl },
        KeyCode::Right => Key::Right { word: alt || ctrl },
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::Backspace => Key::Backspace { word: alt || ctrl },
        KeyCode::Delete => Key::Delete { word: alt || ctrl },
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        _ => return None,
    })
}

/// How long the store writer gathers events into one transaction. One
/// transaction per streamed token cost more CPU than drawing them.
const STORE_BATCH_MS: u64 = 250;

/// Persist events on a background thread, grouped into one transaction per
/// STORE_BATCH_MS. Sleeps (no timers) when no events arrive. A crash loses
/// at most the last batch from the store; the screen is unaffected.
/// `None` asks the writer to flush and stop; the host reader may still hold
/// a sender while it waits on stdin.
type StoreMsg = Option<Event>;

fn spawn_store_writer(store: Arc<SqliteStore>) -> (std::sync::mpsc::Sender<StoreMsg>, Option<std::thread::JoinHandle<()>>) {
    let (tx, rx) = std::sync::mpsc::channel::<StoreMsg>();
    let handle = std::thread::Builder::new()
        .name("camouflage-store".into())
        .spawn(move || {
            while let Ok(Some(first)) = rx.recv() {
                let mut batch = vec![first];
                let mut close = false;
                let until = std::time::Instant::now() + std::time::Duration::from_millis(STORE_BATCH_MS);
                while let Some(left) = until.checked_duration_since(std::time::Instant::now()) {
                    match rx.recv_timeout(left) {
                        Ok(Some(ev)) => batch.push(ev),
                        Ok(None) => {
                            close = true;
                            break;
                        }
                        Err(_) => break,
                    }
                }
                let _ = store.append_batch(&batch);
                if close {
                    break;
                }
            }
        })
        .ok();
    (tx, handle)
}

/// Read NDJSON from stdin, queue each event for the store, then hand it to the loop.
/// Unknown event types produce one visible warning each instead of being
/// dropped silently.
fn spawn_host_reader(session_id: Uuid, tx: mpsc::Sender<Input>, writer: std::sync::mpsc::Sender<StoreMsg>, seq: Arc<AtomicI64>) {
    tokio::spawn(async move {
        let decoder = NdjsonDecoder::new(session_id);
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        let mut warned: HashSet<String> = HashSet::new();
        loop {
            let line = match lines.next_line().await {
                Ok(Some(l)) => l,
                _ => break,
            };
            if line.trim().is_empty() {
                continue;
            }
            let mut ev = match decoder.parse_line(&line) {
                Ok(ev) => ev,
                Err(NdjsonError::UnknownEventType(name)) => {
                    if !warned.insert(name.clone()) {
                        continue;
                    }
                    warning(session_id, format!("Ignored unknown event \"{name}\". The host may need a newer Camouflage."))
                }
                Err(e) => warning(session_id, format!("Ignored an invalid event from the host: {e}")),
            };
            ev.session_id = session_id;
            ev.seq = seq.fetch_add(1, Ordering::Relaxed);
            if ev.schema_version == 0 {
                ev.schema_version = SCHEMA_VERSION;
            }
            let _ = writer.send(Some(ev.clone()));
            if tx.send(Input::Host(ev)).await.is_err() {
                return;
            }
        }
        let _ = tx.send(Input::HostClosed).await;
    });
}

fn warning(session_id: Uuid, message: String) -> Event {
    Event {
        id: Uuid::new_v4(),
        session_id,
        seq: 0,
        timestamp_ms: now_ms(),
        schema_version: SCHEMA_VERSION,
        event_type: EventType::RuntimeError,
        payload: serde_json::json!({ "message": message, "severity": "warn" }),
    }
}

/// Blocking key reader on its own thread. With crossterm's `use-dev-tty`
/// it reads /dev/tty even when stdin is the host's pipe, and it blocks in
/// poll() with no timeout, so an idle renderer never wakes.
fn spawn_key_reader(tx: mpsc::Sender<Input>) {
    std::thread::spawn(move || {
        while let Ok(ev) = crossterm::event::read() {
            if tx.blocking_send(Input::Term(ev)).is_err() {
                break;
            }
        }
    });
}

/// SIGTERM/SIGHUP (e.g. the SDK's kill()) exit cleanly so the terminal is
/// restored instead of being left in raw mode.
fn spawn_signal_handler(tx: mpsc::Sender<Input>) {
    #[cfg(unix)]
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let (Ok(mut term), Ok(mut hup)) = (signal(SignalKind::terminate()), signal(SignalKind::hangup())) else {
            return;
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = hup.recv() => {}
        }
        let _ = tx.send(Input::HostClosed).await;
    });
    // Windows: closing the console window ends the process; the host
    // closing stdin is handled by the reader.
    #[cfg(not(unix))]
    drop(tx);
}

/// Outbound NDJSON on a blocking thread: write and flush each event as it
/// arrives, no flush timer.
fn spawn_outbound(responses_fd: Option<i32>, emit_responses: bool, stdout_is_tty: bool) -> Result<Option<std::sync::mpsc::Sender<Event>>> {
    let sink: Box<dyn Write + Send> = if let Some(fd) = responses_fd {
        Box::new(crate::platform::file_from_fd(fd)?)
    } else if emit_responses {
        if stdout_is_tty {
            anyhow::bail!("--ui inline draws on stdout when it's a terminal; pass --responses-fd for outbound events");
        }
        Box::new(std::io::stdout())
    } else {
        return Ok(None);
    };
    let (tx, rx) = std::sync::mpsc::channel::<Event>();
    std::thread::spawn(move || {
        let mut sink = std::io::BufWriter::new(sink);
        while let Ok(ev) = rx.recv() {
            if let Ok(s) = serde_json::to_string(&ev) {
                if writeln!(sink, "{s}").and_then(|_| sink.flush()).is_err() {
                    break;
                }
            }
        }
    });
    Ok(Some(tx))
}

fn persist(store: &SqliteStore, seq: &AtomicI64, session_id: Uuid, event_type: EventType, payload: serde_json::Value) {
    let ev = Event {
        id: Uuid::new_v4(),
        session_id,
        seq: seq.fetch_add(1, Ordering::Relaxed),
        timestamp_ms: now_ms(),
        schema_version: SCHEMA_VERSION,
        event_type,
        payload,
    };
    let _ = store.append_batch(&[ev]);
}

/// Dark unless COLORFGBG says the background is light.
fn detect_theme() -> Theme {
    let light = std::env::var("COLORFGBG")
        .ok()
        .and_then(|v| v.rsplit(';').next().and_then(|bg| bg.parse::<u8>().ok()))
        .map(|bg| bg == 7 || bg == 15)
        .unwrap_or(false);
    if light {
        Theme::light()
    } else {
        Theme::dark()
    }
}

/// Bracketed paste and focus reporting (so the loop can slow down while
/// the terminal is in the background).
const ENTER_MODES: &[u8] = b"\x1b[?2004h\x1b[?1004h";
const LEAVE_MODES: &[u8] = b"\x1b[?2004l\x1b[?1004l\x1b[<u\x1b[0m\x1b[?25h";

fn enter_terminal_modes() -> Result<()> {
    crossterm::terminal::enable_raw_mode()?;
    let mut tty = tty_writer();
    tty.write_all(ENTER_MODES)?;
    // Ask for disambiguated keys (kitty protocol) so Shift+Enter arrives as
    // its own key. Terminals without it ignore the sequence. Don't probe
    // with crossterm's supports_keyboard_enhancement(): it writes its query
    // to stdout, which in piped mode is the host's event stream (it
    // corrupted the first outbound event), and then waits for an answer
    // that never comes.
    tty.write_all(b"\x1b[>1u")?;
    tty.flush()?;
    Ok(())
}

fn leave_terminal_modes() {
    let mut tty = tty_writer();
    let _ = tty.write_all(LEAVE_MODES);
    let _ = tty.flush();
    let _ = crossterm::terminal::disable_raw_mode();
}

fn tty_writer() -> Box<dyn Write> {
    match crate::platform::open_console_writer() {
        Ok(f) => Box::new(f),
        Err(_) => Box::new(std::io::stdout()),
    }
}

/// Restore the terminal on panic and write a crash dump under
/// ~/.camouflage (never into the user's working directory).
fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        leave_terminal_modes();
        if let Some(home) = crate::platform::home_dir() {
            let dir = home.join(".camouflage");
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join(format!("crash-{}.txt", now_ms()));
            if std::fs::write(&path, format!("{info}\n")).is_ok() {
                eprintln!("camouflage crashed; details in {}", path.display());
            }
        }
        prev(info);
    }));
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventState;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    #[test]
    fn maps_keys() {
        assert_eq!(map_key(key(KeyCode::Char('T'), KeyModifiers::SHIFT)), Some(Key::Text("T".into())));
        assert_eq!(map_key(key(KeyCode::Char('é'), KeyModifiers::NONE)), Some(Key::Text("é".into())));
        assert_eq!(map_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL)), Some(Key::Ctrl('c')));
        assert_eq!(map_key(key(KeyCode::Enter, KeyModifiers::SHIFT)), Some(Key::Enter { shift: true, alt: false }));
        assert_eq!(map_key(key(KeyCode::Backspace, KeyModifiers::ALT)), Some(Key::Backspace { word: true }));
        assert_eq!(map_key(key(KeyCode::Tab, KeyModifiers::SHIFT)), Some(Key::BackTab));
        let mut release = key(KeyCode::Char('a'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert_eq!(map_key(release), None);
    }
}
