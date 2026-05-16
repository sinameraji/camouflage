use crate::{draw, input};
use anyhow::{Context, Result};
use camouflage_headless::NdjsonDecoder;
use camouflage_protocol::{Event, EventType, SCHEMA_VERSION};
use camouflage_renderer::{RenderModel, ViewportState};
use camouflage_store::{EventStore, SqliteStore};
use crossterm::event::{poll as ct_poll, read as ct_read, Event as CtEvent, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{BufRead, BufReader as StdBufReader, Stdout};
use std::os::fd::{FromRawFd, RawFd};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration, Instant};
use uuid::Uuid;

pub struct Config {
    pub store: SqliteStore,
    pub stdin_events: bool,
    pub replay: Option<Uuid>,
    pub fps: u32,
    /// File descriptor for NDJSON event input. `Some` when stdin was a pipe
    /// at startup (it has been dup'd here so fd 0 can be /dev/tty).
    pub events_fd: Option<RawFd>,
}

pub async fn run(cfg: Config) -> Result<()> {
    let store = Arc::new(cfg.store);
    let session_id = cfg.replay.unwrap_or_else(Uuid::new_v4);

    let mut model = RenderModel::new();
    let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
    let mut viewport = ViewportState::new(session_id, height.saturating_sub(4), width);
    let mut input_buf = String::new();
    let mut status: String = "idle".into();

    // Preload replay events.
    if let Some(sid) = cfg.replay {
        let events = store.load_session(sid).context("loading session")?;
        for ev in &events {
            model.apply(ev);
        }
        status = format!("replay: {} events", events.len());
    }

    // Initial seq for inputs we synthesize (e.g., user message).
    let start_seq = store.latest_seq(session_id).unwrap_or(-1).max(-1) + 1;
    let seq_counter = Arc::new(AtomicI64::new(start_seq));

    // Wire NDJSON ingestion. We read from `events_fd` (the dup'd pipe, if any)
    // on a blocking thread and forward into the async channel. Using stdin
    // (fd 0) is unsafe here because we deliberately swapped fd 0 to /dev/tty.
    let (ev_tx, mut ev_rx) = mpsc::channel::<Event>(4096);
    if cfg.stdin_events {
        let decoder = NdjsonDecoder::new(session_id);
        let tx = ev_tx.clone();
        if let Some(fd) = cfg.events_fd {
            std::thread::spawn(move || {
                // Safety: we own this fd (dup'd at startup), nothing else reads it.
                let file = unsafe { std::fs::File::from_raw_fd(fd) };
                let reader = StdBufReader::new(file);
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    if line.trim().is_empty() { continue; }
                    let parsed = decoder.parse_line(&line);
                    let ev = match parsed {
                        Ok(ev) => ev,
                        Err(e) => Event {
                            id: Uuid::new_v4(),
                            session_id,
                            seq: 0,
                            timestamp_ms: now_ms(),
                            schema_version: SCHEMA_VERSION,
                            event_type: EventType::RuntimeError,
                            payload: serde_json::json!({"message": format!("ndjson: {e}")}),
                        },
                    };
                    if tx.blocking_send(ev).is_err() { break; }
                }
            });
        }
    }

    // Persist-before-render writer task.
    let (persist_tx, mut persist_rx) = mpsc::channel::<Event>(4096);
    let (rendered_tx, mut rendered_rx) = mpsc::channel::<Event>(4096);
    {
        let store = store.clone();
        tokio::spawn(async move {
            let mut batch: Vec<Event> = Vec::with_capacity(256);
            let mut flush = interval(Duration::from_millis(16));
            flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    maybe = persist_rx.recv() => {
                        match maybe {
                            Some(ev) => batch.push(ev),
                            None => break,
                        }
                        // Drain quickly.
                        while let Ok(ev) = persist_rx.try_recv() {
                            batch.push(ev);
                            if batch.len() >= 512 { break; }
                        }
                        if batch.len() >= 256 {
                            if let Err(e) = store.append_batch(&batch) {
                                tracing::warn!(?e, "store append failed");
                            }
                            for ev in batch.drain(..) {
                                if rendered_tx.send(ev).await.is_err() { return; }
                            }
                        }
                    }
                    _ = flush.tick() => {
                        if !batch.is_empty() {
                            if let Err(e) = store.append_batch(&batch) {
                                tracing::warn!(?e, "store append failed");
                            }
                            for ev in batch.drain(..) {
                                if rendered_tx.send(ev).await.is_err() { return; }
                            }
                        }
                    }
                }
            }
            if !batch.is_empty() {
                let _ = store.append_batch(&batch);
                for ev in batch.drain(..) {
                    let _ = rendered_tx.send(ev).await;
                }
            }
        });
    }

    // Bridge: stdin → persist queue (renormalize seq into session monotone).
    {
        let seq_counter = seq_counter.clone();
        let persist_tx = persist_tx.clone();
        tokio::spawn(async move {
            while let Some(mut ev) = ev_rx.recv().await {
                ev.session_id = session_id;
                ev.seq = seq_counter.fetch_add(1, Ordering::Relaxed);
                if ev.schema_version == 0 {
                    ev.schema_version = SCHEMA_VERSION;
                }
                if persist_tx.send(ev).await.is_err() {
                    break;
                }
            }
        });
    }

    // Terminal setup.
    let mut terminal = setup_terminal().context("setup terminal")?;
    install_panic_hook();

    // Blocking thread that polls crossterm and forwards into an async channel.
    let (key_tx, mut key_rx) = mpsc::channel::<CtEvent>(256);
    std::thread::spawn(move || {
        loop {
            match ct_poll(Duration::from_millis(50)) {
                Ok(true) => match ct_read() {
                    Ok(ev) => {
                        if key_tx.blocking_send(ev).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                },
                Ok(false) => {}
                Err(_) => break,
            }
        }
    });

    let frame_period = Duration::from_secs_f64(1.0 / cfg.fps as f64);
    let mut ticker = interval(frame_period);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Emit a SessionStarted if this is a fresh session (no replay).
    if cfg.replay.is_none() {
        let ev = Event {
            id: Uuid::new_v4(),
            session_id,
            seq: seq_counter.fetch_add(1, Ordering::Relaxed),
            timestamp_ms: now_ms(),
            schema_version: SCHEMA_VERSION,
            event_type: EventType::SessionStarted,
            payload: serde_json::json!({}),
        };
        let _ = persist_tx.send(ev).await;
    }

    let result: Result<()> = loop {
        tokio::select! {
            biased;
            maybe_evt = key_rx.recv() => {
                let Some(evt) = maybe_evt else { break Ok(()); };
                match evt {
                    CtEvent::Key(k) if k.kind == KeyEventKind::Press => {
                        let action = input::handle_key(k, &mut input_buf);
                        match action {
                            input::Action::Quit => break Ok(()),
                            input::Action::SubmitInput(text) => {
                                let ev = Event {
                                    id: Uuid::new_v4(),
                                    session_id,
                                    seq: seq_counter.fetch_add(1, Ordering::Relaxed),
                                    timestamp_ms: now_ms(),
                                    schema_version: SCHEMA_VERSION,
                                    event_type: EventType::UserMessageCreated,
                                    payload: serde_json::json!({"text": text}),
                                };
                                let _ = persist_tx.send(ev).await;
                            }
                            input::Action::ScrollUp(n) => {
                                viewport.scroll_up(n as i64, model.rows().len() as i64);
                                model.mark_dirty();
                            }
                            input::Action::ScrollDown(n) => {
                                viewport.scroll_down(n as i64);
                                model.mark_dirty();
                            }
                            input::Action::JumpToLatest => {
                                viewport.jump_to_latest();
                                model.mark_dirty();
                            }
                            input::Action::CancelStream => {
                                status = "canceled".into();
                                model.mark_dirty();
                            }
                            input::Action::Replay => {
                                // Reload session events.
                                model = RenderModel::new();
                                let evs = store.load_session(session_id).unwrap_or_default();
                                for ev in &evs { model.apply(ev); }
                                status = format!("replayed {} events", evs.len());
                                model.mark_dirty();
                            }
                            input::Action::None => {
                                model.mark_dirty();
                            }
                        }
                    }
                    CtEvent::Resize(w, h) => {
                        viewport.resize(h.saturating_sub(4), w);
                        model.mark_dirty();
                    }
                    _ => {}
                }
            }
            Some(ev) = rendered_rx.recv() => {
                let was_at_bottom = viewport.at_bottom();
                model.apply(&ev);
                status = match ev.event_type {
                    EventType::AssistantTokenDelta | EventType::AssistantStreamStarted => "streaming".into(),
                    EventType::AssistantMessageCompleted => "idle".into(),
                    EventType::ToolExecutionStarted | EventType::ToolExecutionStdout | EventType::ToolExecutionStderr => "tool".into(),
                    EventType::ToolExecutionFinished => "idle".into(),
                    EventType::RuntimeError => "error".into(),
                    _ => status,
                };
                // If user is not at bottom, do not yank them. scroll_offset stays.
                let _ = was_at_bottom;
                // Drain any additional events that arrived this tick to coalesce work.
                let mut drained = 0;
                while drained < 1024 {
                    match rendered_rx.try_recv() {
                        Ok(ev2) => { model.apply(&ev2); drained += 1; }
                        Err(_) => break,
                    }
                }
            }
            _ = ticker.tick() => {
                if model.dirty() {
                    let start = Instant::now();
                    draw::render(&mut terminal, &model, &viewport, &input_buf, &status)?;
                    model.mark_clean();
                    let elapsed = start.elapsed();
                    if elapsed > Duration::from_millis(20) {
                        tracing::trace!(?elapsed, "slow frame");
                    }
                }
            }
        }
    };

    teardown_terminal(&mut terminal).ok();
    result
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    // By the time we get here, fd 0 is /dev/tty (see tty::install_fd_layout
    // called from main()), so crossterm's normal raw-mode path works.
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend)?;
    term.clear()?;
    Ok(term)
}

fn teardown_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    let _ = disable_raw_mode();
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = std::io::stdout().execute(LeaveAlternateScreen);
        prev(info);
    }));
}
