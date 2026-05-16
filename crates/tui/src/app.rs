use crate::{draw, input, tty};
use anyhow::{Context, Result};
use camouflage_headless::{run_reader, NdjsonDecoder};
use camouflage_protocol::{Event, EventType, SCHEMA_VERSION};
use camouflage_renderer::{reconstruct_rows, RenderModel, Row, ViewportState};
use camouflage_store::{EventStore, SqliteStore};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::Stdout;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::io::BufReader;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};
use uuid::Uuid;

pub struct Config {
    pub store: SqliteStore,
    pub stdin_events: bool,
    pub replay: Option<Uuid>,
    pub fps: u32,
    pub row_cap: Option<usize>,
}

struct HistoryReq {
    session: Uuid,
    from_seq: i64,
    to_seq: i64,
}

struct HistoryResp {
    from_seq: i64,
    rows: Vec<Row>,
}

pub async fn run(cfg: Config) -> Result<()> {
    let store = Arc::new(cfg.store);
    let session_id = cfg.replay.unwrap_or_else(Uuid::new_v4);

    let mut model = match cfg.row_cap {
        Some(c) => RenderModel::with_cap(c),
        None => RenderModel::new(),
    };
    let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
    let mut viewport = ViewportState::new(session_id, height.saturating_sub(4), width);
    let mut input_buf = String::new();
    let mut status: String = "idle".into();

    if let Some(sid) = cfg.replay {
        let events = store.load_session(sid).context("loading session")?;
        for ev in &events {
            model.apply(ev);
        }
        status = format!("replay: {} events", events.len());
    }

    let start_seq = store.latest_seq(session_id).unwrap_or(-1).max(-1) + 1;
    let seq_counter = Arc::new(AtomicI64::new(start_seq));

    // NDJSON ingestion from stdin (pipe).
    let (ev_tx, mut ev_rx) = mpsc::channel::<Event>(4096);
    if cfg.stdin_events {
        let decoder = NdjsonDecoder::new(session_id);
        let stdin = tokio::io::stdin();
        let reader = BufReader::new(stdin);
        let tx = ev_tx.clone();
        tokio::spawn(async move {
            let _ = run_reader(reader, decoder, tx).await;
        });
    }
    drop(ev_tx); // we won't send into ev_tx ourselves.

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

    // Bridge: ev_rx (stdin) → persist queue with renormalised seq.
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

    // Terminal setup. crossterm's raw mode uses tcsetattr — works regardless
    // of what stdin is.
    let mut terminal = setup_terminal().context("setup terminal")?;
    install_panic_hook();

    // Open /dev/tty for reading and start our custom key reader. We use this
    // instead of crossterm::event::poll because the latter requires
    // registering /dev/tty with kqueue on macOS, which returns EINVAL.
    let tty_fd = tty::open_tty_for_read().context("opening /dev/tty for keys")?;
    let key_rx = tty::spawn_key_reader(tty_fd);

    let frame_period = Duration::from_secs_f64(1.0 / cfg.fps as f64);
    let mut ticker = interval(frame_period);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // History backfill worker. When the user scrolls past the in-memory
    // window we ask this worker for a chunk of older rows. Only one request
    // is in flight at a time (gated by `history_inflight`).
    const HISTORY_CHUNK: i64 = 500;
    let (history_req_tx, mut history_req_rx) = mpsc::channel::<HistoryReq>(4);
    let (history_done_tx, mut history_done_rx) = mpsc::channel::<HistoryResp>(4);
    {
        let store = store.clone();
        tokio::spawn(async move {
            while let Some(req) = history_req_rx.recv().await {
                let store = store.clone();
                let events = tokio::task::spawn_blocking(move || {
                    store.load_range(req.session, req.from_seq, req.to_seq)
                })
                .await;
                let rows = match events {
                    Ok(Ok(evs)) => reconstruct_rows(&evs),
                    Ok(Err(e)) => {
                        tracing::warn!(?e, "history load_range failed");
                        Vec::new()
                    }
                    Err(e) => {
                        tracing::warn!(?e, "history worker join failed");
                        Vec::new()
                    }
                };
                if history_done_tx
                    .send(HistoryResp { from_seq: req.from_seq, rows })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
    }
    let mut history_inflight = false;

    // SessionStarted on fresh sessions.
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

    loop {
        // Non-blocking poll for keys before each frame.
        while let Ok(key) = key_rx.try_recv() {
            let action = input::handle_key(key, &mut input_buf);
            match action {
                input::Action::Quit => return shutdown(&mut terminal, Ok(())),
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
                    viewport.scroll_up(n as i64, model.combined_len() as i64);
                    model.mark_dirty();
                }
                input::Action::ScrollDown(n) => {
                    viewport.scroll_down(n as i64);
                    model.mark_dirty();
                }
                input::Action::JumpToLatest => {
                    viewport.jump_to_latest();
                    model.clear_history();
                    model.mark_dirty();
                }
                input::Action::CancelStream => {
                    status = "canceled".into();
                    model.mark_dirty();
                }
                input::Action::Replay => {
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

        // Maybe request more history. Trigger when the user is scrolled
        // near the top of what's in memory AND the model's earliest visible
        // seq is > 0 (i.e. older events exist in the store and we haven't
        // paged them in yet).
        if !history_inflight {
            if let Some(earliest_visible) = model.earliest_visible_seq() {
                if earliest_visible > 0 {
                    let near_top = (viewport.scroll_offset + viewport.viewport_height as i64)
                        >= model.combined_len() as i64 - 8;
                    if near_top {
                        let to = earliest_visible;
                        let from = (to - HISTORY_CHUNK).max(0);
                        if from < to {
                            tracing::info!(from, to, "history fetch requested");
                            let _ = history_req_tx
                                .send(HistoryReq { session: session_id, from_seq: from, to_seq: to })
                                .await;
                            history_inflight = true;
                        }
                    }
                }
            }
        }

        tokio::select! {
            biased;
            Some(resp) = history_done_rx.recv() => {
                let n = resp.rows.len();
                tracing::info!(from_seq = resp.from_seq, rows = n, "history fetch returned");
                if !resp.rows.is_empty() {
                    model.prepend_history(resp.rows);
                }
                history_inflight = false;
                model.mark_dirty();
            }
            Some(ev) = rendered_rx.recv() => {
                model.apply(&ev);
                status = match ev.event_type {
                    EventType::AssistantTokenDelta | EventType::AssistantStreamStarted => "streaming".into(),
                    EventType::AssistantMessageCompleted => "idle".into(),
                    EventType::ToolExecutionStarted | EventType::ToolExecutionStdout | EventType::ToolExecutionStderr => "tool".into(),
                    EventType::ToolExecutionFinished => "idle".into(),
                    EventType::RuntimeError => "error".into(),
                    _ => status,
                };
                let mut drained = 0;
                while drained < 1024 {
                    match rendered_rx.try_recv() {
                        Ok(ev2) => {
                            model.apply(&ev2);
                            drained += 1;
                        }
                        Err(_) => break,
                    }
                }
            }
            _ = ticker.tick() => {
                if model.dirty() {
                    draw::render(&mut terminal, &model, &viewport, &input_buf, &status)?;
                    model.mark_clean();
                }
                // Resize check (size() is cheap; no kqueue needed)
                if let Ok((w, h)) = crossterm::terminal::size() {
                    if w != viewport.viewport_width || h.saturating_sub(4) != viewport.viewport_height {
                        viewport.resize(h.saturating_sub(4), w);
                        model.mark_dirty();
                    }
                }
            }
        }
    }
}

fn shutdown(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    result: Result<()>,
) -> Result<()> {
    teardown_terminal(terminal).ok();
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
