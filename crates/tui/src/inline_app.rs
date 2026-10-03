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

/// Shortest gap between draws caused by the host (tokens, tool output):
/// 20 fps is smooth for streaming text, and a burst of tokens then costs
/// one frame instead of one per token. User input always draws at once.
const MIN_FRAME_GAP_MS: i64 = 50;
use camouflage_inline::{InlineTerminal, Theme};
use camouflage_protocol::{Event, EventType, SCHEMA_VERSION};
use camouflage_store::{EventStore, SqliteStore};
use crossterm::event::{Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;
use uuid::Uuid;

pub struct InlineConfig {
    /// None: save nothing (`--no-store`, for hosts with their own history).
    pub store: Option<SqliteStore>,
    /// Delete stored sessions idle for longer than this at startup.
    /// None keeps everything.
    pub retention_days: Option<u32>,
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
    let InlineConfig { store, retention_days, stdin_events, emit_responses, responses_fd, app_title } = cfg;
    let store = store.map(Arc::new);
    let session_id = Uuid::new_v4();
    let seq = Arc::new(AtomicI64::new(store.as_ref().and_then(|s| s.latest_seq(session_id).ok()).unwrap_or(-1).max(-1) + 1));
    if let Some(store) = &store {
        spawn_housekeeping(store.clone(), retention_days);
    }

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
    let keys = spawn_key_reader(tx.clone());
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
    // Through the writer, like everything else: housekeeping may hold the
    // database for a while on the first run after an upgrade.
    let _ = writer.0.send(Some(make_event(&seq, session_id, EventType::SessionStarted, serde_json::json!({ "synthetic": true }))));

    let mut last_frame = None;
    // Host-driven draws are batched to one per MIN_FRAME_GAP_MS (one per
    // UNFOCUSED_FRAME_MS in the background); `deferred` means there's an
    // undrawn change waiting for its slot.
    let mut last_draw: i64 = 0;
    let mut deferred = false;
    // A child process owns the terminal (TerminalSuspend): no drawing.
    let mut suspended = false;
    // Loop statistics, written on exit when CAMOUFLAGE_STATS_PATH is set
    // (the idle soak test uses these to prove idle means no wakeups).
    let mut wakeups: u64 = 0;
    let mut frames: u64 = 0;
    let mut by_source: std::collections::BTreeMap<String, u64> = Default::default();
    let result: Result<()> = async {
        loop {
            let now = now_ms();
            let mut deadline = if suspended { None } else { session.next_wakeup(now) };
            let gap = if session.focused { MIN_FRAME_GAP_MS } else { UNFOCUSED_FRAME_MS };
            if deferred {
                let at = last_draw + gap;
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
                    Input::Host(ev) if ev.event_type == EventType::TerminalSuspend => {
                        let id = ev.payload.get("id").cloned().unwrap_or(serde_json::Value::Null);
                        let supported = KeyGate::supported();
                        if supported && !suspended {
                            keys.set_paused(true);
                            term.suspend()?;
                            leave_terminal_modes();
                            suspended = true;
                            last_frame = None;
                        }
                        session.push_outbound(EventType::TerminalSuspended, serde_json::json!({ "id": id, "supported": supported }));
                    }
                    Input::Host(ev) if ev.event_type == EventType::TerminalResume => {
                        if suspended {
                            enter_terminal_modes()?;
                            keys.set_paused(false);
                            suspended = false;
                            // The window may have changed while the child ran.
                            if let Ok((w, h)) = crossterm::terminal::size() {
                                session.resize((w as usize).saturating_sub(1), h as usize);
                                term.resize(w, h, &Default::default())?;
                            }
                            last_frame = None;
                            urgent = true;
                        }
                    }
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
            if suspended {
                continue;
            }
            let gap = if session.focused { MIN_FRAME_GAP_MS } else { UNFOCUSED_FRAME_MS };
            if !urgent && !session.clear_screen && !session.exit && !host_closed && now - last_draw < gap {
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
    // Save what's buffered plus the final event. Don't make the user wait
    // behind a long housekeeping pass (first run after an upgrade): after
    // 1.5 s exit anyway; SQLite rolls an unfinished VACUUM back safely.
    let (writer_tx, writer_thread) = writer;
    let _ = writer_tx.send(Some(make_event(&seq, session_id, EventType::SessionEnded, serde_json::json!({}))));
    let _ = writer_tx.send(None);
    if let Some(t) = writer_thread {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1500);
        while !t.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if t.is_finished() {
            let _ = t.join();
        }
    }
    drop(store);
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

fn spawn_store_writer(store: Option<Arc<SqliteStore>>) -> (std::sync::mpsc::Sender<StoreMsg>, Option<std::thread::JoinHandle<()>>) {
    let (tx, rx) = std::sync::mpsc::channel::<StoreMsg>();
    // No store: nothing receives, so sends just fail quietly.
    let Some(store) = store else { return (tx, None) };
    let handle = std::thread::Builder::new()
        .name("camouflage-store".into())
        .spawn(move || {
            // Saves conversations, not keystrokes; see persist.rs.
            let mut coalescer = crate::persist::Coalescer::default();
            loop {
                let first = match rx.recv() {
                    Ok(Some(ev)) => ev,
                    // Closing (or every sender gone): save what's buffered.
                    _ => {
                        let rest = coalescer.flush();
                        if !rest.is_empty() {
                            let _ = store.append_batch(&rest);
                        }
                        break;
                    }
                };
                let mut batch = coalescer.push(first);
                let mut close = false;
                let until = std::time::Instant::now() + std::time::Duration::from_millis(STORE_BATCH_MS);
                while let Some(left) = until.checked_duration_since(std::time::Instant::now()) {
                    match rx.recv_timeout(left) {
                        Ok(Some(ev)) => batch.extend(coalescer.push(ev)),
                        Ok(None) => {
                            close = true;
                            break;
                        }
                        Err(_) => break,
                    }
                }
                if close {
                    batch.extend(coalescer.flush());
                }
                if !batch.is_empty() {
                    let _ = store.append_batch(&batch);
                }
                if close {
                    break;
                }
            }
        })
        .ok();
    (tx, handle)
}

/// Retention and compaction, once per start on a low-priority thread: drop
/// sessions idle for longer than `retention_days`, collapse repeated
/// registrations, and give space back when much of the file is free. The
/// writer simply waits if it needs the database meanwhile.
fn spawn_housekeeping(store: Arc<SqliteStore>, retention_days: Option<u32>) {
    let _ = std::thread::Builder::new().name("camouflage-housekeeping".into()).spawn(move || {
        let cutoff = retention_days.filter(|d| *d > 0).map(|d| now_ms() - i64::from(d) * 86_400_000);
        if store.prune(cutoff).is_ok() {
            let _ = store.compact_if_sparse();
        }
    });
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

/// Lets the loop stop the key reader while a child process owns the
/// terminal (TerminalSuspend), without the reader consuming its keys.
#[derive(Clone)]
struct KeyGate {
    paused: Arc<std::sync::atomic::AtomicBool>,
    #[cfg(unix)]
    wake: Arc<std::fs::File>,
}

impl KeyGate {
    fn supported() -> bool {
        cfg!(unix)
    }

    fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
        #[cfg(unix)]
        {
            let _ = (&*self.wake).write_all(b"x");
        }
    }
}

/// Key reader on its own thread. With crossterm's `use-dev-tty` it reads
/// /dev/tty even when stdin is the host's pipe. On Unix it waits in poll()
/// on the tty and a wake pipe with no timeout, so an idle renderer never
/// wakes, and while paused it doesn't touch the tty at all.
fn spawn_key_reader(tx: mpsc::Sender<Input>) -> KeyGate {
    let paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
    #[cfg(unix)]
    {
        use std::os::fd::{AsRawFd, FromRawFd};
        let mut fds = [0i32; 2];
        // SAFETY: plain pipe(2); both ends are owned by Files below.
        let ok = unsafe { libc::pipe(fds.as_mut_ptr()) } == 0;
        let (wake_r, wake_w) = if ok {
            unsafe { (std::fs::File::from_raw_fd(fds[0]), std::fs::File::from_raw_fd(fds[1])) }
        } else {
            (std::fs::File::open("/dev/null").expect("/dev/null"), std::fs::OpenOptions::new().write(true).open("/dev/null").expect("/dev/null"))
        };
        let gate = KeyGate { paused: paused.clone(), wake: Arc::new(wake_w) };
        std::thread::spawn(move || {
            let tty = std::fs::File::open("/dev/tty").ok();
            let mut drain = [0u8; 64];
            loop {
                if !paused.load(Ordering::SeqCst) {
                    // Events crossterm already buffered never show up as
                    // tty readiness, so hand those over first. poll(ZERO)
                    // skips crossterm's parser buffer (the rest of a burst
                    // of keys); a 1 ms poll checks it. Only runs after
                    // input, never while idle.
                    while !paused.load(Ordering::SeqCst) && crossterm::event::poll(std::time::Duration::from_millis(1)).unwrap_or(false) {
                        match crossterm::event::read() {
                            Ok(ev) => {
                                if tx.blocking_send(Input::Term(ev)).is_err() {
                                    return;
                                }
                            }
                            Err(_) => return,
                        }
                    }
                }
                // select(), not poll(): macOS poll() reports POLLNVAL for
                // /dev/tty (crossterm uses select for the same reason).
                let watch_tty = !paused.load(Ordering::SeqCst);
                let wake_fd = wake_r.as_raw_fd();
                let tty_fd = tty.as_ref().filter(|_| watch_tty).map(|t| t.as_raw_fd());
                // SAFETY: fd_set is plain data; both fds are open and below
                // FD_SETSIZE (they're among the first few we open).
                let n = unsafe {
                    let mut set: libc::fd_set = std::mem::zeroed();
                    libc::FD_ZERO(&mut set);
                    libc::FD_SET(wake_fd, &mut set);
                    let mut max = wake_fd;
                    if let Some(t) = tty_fd {
                        libc::FD_SET(t, &mut set);
                        max = max.max(t);
                    }
                    let n = libc::select(max + 1, &mut set, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
                    if n > 0 && libc::FD_ISSET(wake_fd, &set) {
                        let _ = (&wake_r).read(&mut drain);
                    }
                    (n, tty_fd.is_some_and(|t| n > 0 && libc::FD_ISSET(t, &set)))
                };
                let (n, keys_ready) = n;
                if n < 0 {
                    if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return;
                }
                // Keys are waiting (or there's no /dev/tty to watch): read
                // them. crossterm's poll(ZERO) only returns events it has
                // already buffered, so the read has to happen here.
                if !paused.load(Ordering::SeqCst) && (tty_fd.is_none() || keys_ready) {
                    let r = crossterm::event::read();
                    match r {
                        Ok(ev) => {
                            if tx.blocking_send(Input::Term(ev)).is_err() {
                                return;
                            }
                        }
                        Err(_) => return,
                    }
                }
            }
        });
        gate
    }
    #[cfg(not(unix))]
    {
        std::thread::spawn(move || {
            while let Ok(ev) = crossterm::event::read() {
                if tx.blocking_send(Input::Term(ev)).is_err() {
                    break;
                }
            }
        });
        KeyGate { paused }
    }
}

/// SIGTERM/SIGHUP (e.g. the SDK's kill()) exit cleanly so the terminal is
/// restored instead of being left in raw mode.
fn spawn_signal_handler(tx: mpsc::Sender<Input>) {
    // Window resizes: the key reader waits in its own select(), not inside
    // crossterm, so crossterm's SIGWINCH handling doesn't run while idle.
    #[cfg(unix)]
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            use tokio::signal::unix::{signal, SignalKind};
            let Ok(mut winch) = signal(SignalKind::window_change()) else { return };
            while winch.recv().await.is_some() {
                if let Ok((w, h)) = crossterm::terminal::size() {
                    if tx.send(Input::Term(TermEvent::Resize(w, h))).await.is_err() {
                        return;
                    }
                }
            }
        });
    }
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

fn make_event(seq: &AtomicI64, session_id: Uuid, event_type: EventType, payload: serde_json::Value) -> Event {
    Event {
        id: Uuid::new_v4(),
        session_id,
        seq: seq.fetch_add(1, Ordering::Relaxed),
        timestamp_ms: now_ms(),
        schema_version: SCHEMA_VERSION,
        event_type,
        payload,
    }
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
        let path = crate::platform::crash_dir().join(format!("crash-{}.txt", now_ms()));
        if std::fs::write(&path, format!("{info}\n")).is_ok() {
            eprintln!("camouflage crashed; details in {}", path.display());
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
