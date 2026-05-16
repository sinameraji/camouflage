//! `camouflage-broadcast` — websocket transport for live event streams.
//!
//! Reads Camouflage NDJSON from stdin and broadcasts every line as a
//! websocket text frame to all connected clients. Newly-connected clients
//! receive the in-memory replay buffer (every event seen since the server
//! started) so a browser that connects mid-session still sees the full
//! transcript.
//!
//! Pipeline:
//!     kimiflare --emit-events -p "..." | camouflage-broadcast --port 8080
//!     # then in a browser / websocat:
//!     websocat ws://localhost:8080
//!
//! Wire format: one event per websocket text frame, identical NDJSON to what
//! `camouflage-tui --stdin-events` consumes on stdin. Clients can either
//! parse them directly or wrap a `RenderModel` in JS/WASM (Slice F).
//!
//! This is the "broadcast everything to everyone" minimum. Snapshot-mode
//! transport (where each client gets `Snapshot` documents instead of raw
//! events, suitable for thin clients without protocol knowledge) is left
//! for a follow-up if the browser viewer demands it.

use anyhow::{Context, Result};
use clap::Parser;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, RwLock};
use tokio_tungstenite::tungstenite::Message;

#[derive(Parser, Debug)]
#[command(name = "camouflage-broadcast", about = "Broadcast Camouflage NDJSON over websocket.")]
struct Args {
    /// TCP port to bind. Default 8080.
    #[arg(long, default_value_t = 8080)]
    port: u16,
    /// Bind address. Default 127.0.0.1; pass 0.0.0.0 to expose on the LAN.
    #[arg(long, default_value = "127.0.0.1")]
    bind: String,
    /// Cap on the in-memory replay buffer (lines). Older lines are dropped
    /// from replay (live broadcast is unaffected). Default 10_000.
    #[arg(long, default_value_t = 10_000)]
    replay_cap: usize,
}

type Replay = Arc<RwLock<Vec<String>>>;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let addr = format!("{}:{}", args.bind, args.port);
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    eprintln!("camouflage-broadcast: listening on ws://{}", addr);

    // Live broadcast channel — every stdin line gets cloned to every
    // connected client's subscriber. 256-message buffer absorbs short
    // back-pressure spikes; a slow client that overflows is dropped.
    let (tx, _) = broadcast::channel::<String>(256);
    let replay: Replay = Arc::new(RwLock::new(Vec::with_capacity(args.replay_cap.min(1024))));

    // Accept loop
    let accept_tx = tx.clone();
    let accept_replay = replay.clone();
    tokio::spawn(async move {
        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(x) => x,
                Err(e) => {
                    eprintln!("camouflage-broadcast: accept error: {e}");
                    continue;
                }
            };
            let rx = accept_tx.subscribe();
            let rep = accept_replay.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_client(stream, peer, rx, rep).await {
                    eprintln!("camouflage-broadcast: client {peer} ended: {e}");
                }
            });
        }
    });

    // Stdin → replay buffer + live broadcast
    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        {
            let mut buf = replay.write().await;
            if buf.len() >= args.replay_cap {
                buf.remove(0);
            }
            buf.push(line.clone());
        }
        // Best-effort: if no subscribers, drop silently.
        let _ = tx.send(line);
    }

    eprintln!("camouflage-broadcast: stdin closed, exiting");
    Ok(())
}

async fn handle_client(
    stream: tokio::net::TcpStream,
    peer: std::net::SocketAddr,
    mut rx: broadcast::Receiver<String>,
    replay: Replay,
) -> Result<()> {
    let ws = tokio_tungstenite::accept_async(stream)
        .await
        .with_context(|| format!("ws handshake with {peer}"))?;
    eprintln!("camouflage-broadcast: client connected: {peer}");
    let (mut sink, mut src) = futures_split(ws);

    // Replay buffer first, then live stream.
    {
        let snap: Vec<String> = replay.read().await.clone();
        for line in snap {
            if sink_send(&mut sink, Message::Text(line)).await.is_err() {
                return Ok(());
            }
        }
    }

    loop {
        tokio::select! {
            biased;
            // Drain client messages so the read half of the socket stays open
            // (clients typically don't send anything; this catches pings /
            // close frames). Discard payloads.
            incoming = src_next(&mut src) => match incoming {
                Some(Ok(Message::Close(_))) | None => break,
                Some(Err(e)) => {
                    return Err(e.into());
                }
                _ => {}
            },
            recv = rx.recv() => match recv {
                Ok(line) => {
                    if sink_send(&mut sink, Message::Text(line)).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_n)) => {
                    // Slow client — drop it rather than buffer unbounded.
                    eprintln!("camouflage-broadcast: client {peer} lagged, disconnecting");
                    break;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
    Ok(())
}

// Minimal Sink/Stream helpers — avoid pulling `futures` just for `.split()`.

type WsStream = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

fn futures_split(ws: WsStream) -> (Sink, Src) {
    let shared = Arc::new(tokio::sync::Mutex::new(ws));
    (Sink(shared.clone()), Src(shared))
}

struct Sink(Arc<tokio::sync::Mutex<WsStream>>);
struct Src(Arc<tokio::sync::Mutex<WsStream>>);

async fn sink_send(sink: &mut Sink, msg: Message) -> tokio_tungstenite::tungstenite::Result<()> {
    use futures_util::SinkExt;
    let mut guard = sink.0.lock().await;
    guard.send(msg).await
}

async fn src_next(src: &mut Src) -> Option<tokio_tungstenite::tungstenite::Result<Message>> {
    use futures_util::StreamExt;
    let mut guard = src.0.lock().await;
    guard.next().await
}
