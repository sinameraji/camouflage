//! `camouflage-broadcast` — websocket transport for live event streams.
//!
//! Reads Camouflage NDJSON from stdin and broadcasts every line as a
//! websocket text frame to all connected clients. Newly-connected clients
//! receive the in-memory replay buffer (every event seen since the server
//! started) so a browser that connects mid-session still sees the full
//! transcript.
//!
//! Pipeline:
//!     your-agent --emit-events -p "..." | camouflage-broadcast --port 8080
//!     # stderr prints the URL to connect with, including the access token:
//!     websocat "ws://127.0.0.1:8080/?token=<token>"
//!
//! Access control: the replay buffer holds the whole session, and any web
//! page the user visits can open a WebSocket to localhost, so every client
//! must present an access token (`?token=` query parameter or
//! `Authorization: Bearer` header). The token is random per run unless
//! `--token` / `CAMOUFLAGE_BROADCAST_TOKEN` sets one, and is only printed to
//! stderr. See `camouflage_headless::broadcast_auth`.
//!
//! The token is only checked once a peer has sent a complete handshake, so
//! the unauthenticated phase is bounded: a peer gets `--handshake-timeout-ms`
//! (default 5 s) to finish it, and at most `MAX_PENDING_HANDSHAKES` peers
//! can be mid-handshake at once (further connections wait in the listen
//! backlog). Silent connections therefore can't exhaust file descriptors and
//! lock out viewers that hold the token.
//!
//! Wire format: one event per websocket text frame, identical NDJSON to what
//! `camouflage-tui --stdin-events` consumes on stdin. Clients can either
//! parse them directly or wrap a `RenderModel` in JS/WASM (Slice F).
//!
//! This is the "broadcast everything to everyone" minimum. Snapshot-mode
//! transport (where each client gets `Snapshot` documents instead of raw
//! events, suitable for thin clients without protocol knowledge) is left
//! for a follow-up if the browser viewer demands it.

use anyhow::{bail, Context, Result};
use camouflage_headless::broadcast_auth;
use clap::Parser;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, OwnedSemaphorePermit, RwLock, Semaphore};
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
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
    /// Access token clients must present (`?token=<token>` or an
    /// `Authorization: Bearer` header). Default: a random token generated at
    /// startup and printed to stderr.
    #[arg(long, env = broadcast_auth::TOKEN_ENV, hide_env_values = true)]
    token: Option<String>,
    /// How long an unauthenticated peer may take to complete the websocket
    /// handshake before it is disconnected.
    #[arg(long, default_value_t = 5_000, hide = true)]
    handshake_timeout_ms: u64,
}

/// Cap on connections that have been accepted but not yet completed (and
/// authenticated) the websocket handshake.
const MAX_PENDING_HANDSHAKES: usize = 64;

/// Pause after a failed accept() (e.g. EMFILE) so the loop doesn't spin.
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(100);

type Replay = Arc<RwLock<Vec<String>>>;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let token: Arc<str> = match args.token.as_deref() {
        Some("") => bail!("--token / {} must not be empty", broadcast_auth::TOKEN_ENV),
        Some(t) => {
            if t.len() < 16 {
                eprintln!(
                    "camouflage-broadcast: warning: the access token is short ({} chars); \
                     anyone who can guess it can read the session",
                    t.len()
                );
            }
            t.into()
        }
        None => broadcast_auth::generate_token().into(),
    };
    let addr = format!("{}:{}", args.bind, args.port);
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    let local = listener.local_addr().context("reading bound address")?;
    eprintln!("camouflage-broadcast: listening on ws://{local}");
    // The URL clients need. Never written to the event stream / stdout.
    let mut shown = local;
    if shown.ip().is_unspecified() {
        shown.set_ip(std::net::Ipv4Addr::LOCALHOST.into());
    }
    eprintln!(
        "camouflage-broadcast: connect with ws://{shown}/?{}={}",
        broadcast_auth::TOKEN_PARAM,
        url_encode(&token)
    );
    eprintln!(
        "camouflage-broadcast: viewer: open viewer/index.html#ws=ws://{shown}&{}={}",
        broadcast_auth::TOKEN_PARAM,
        url_encode(&token)
    );

    // Live broadcast channel — every stdin line gets cloned to every
    // connected client's subscriber. 256-message buffer absorbs short
    // back-pressure spikes; a slow client that overflows is dropped.
    let (tx, _) = broadcast::channel::<String>(256);
    let replay: Replay = Arc::new(RwLock::new(Vec::with_capacity(args.replay_cap.min(1024))));

    // Accept loop
    let accept_tx = tx.clone();
    let accept_replay = replay.clone();
    let handshake_timeout = Duration::from_millis(args.handshake_timeout_ms);
    let pending = Arc::new(Semaphore::new(MAX_PENDING_HANDSHAKES));
    tokio::spawn(async move {
        loop {
            // Take a handshake slot before accepting, so that once the cap is
            // reached new peers wait in the kernel backlog rather than each
            // holding a file descriptor.
            let permit = pending
                .clone()
                .acquire_owned()
                .await
                .expect("semaphore is never closed");
            let (stream, peer) = match listener.accept().await {
                Ok(x) => x,
                Err(e) => {
                    eprintln!("camouflage-broadcast: accept error: {e}");
                    tokio::time::sleep(ACCEPT_ERROR_BACKOFF).await;
                    continue;
                }
            };
            let rx = accept_tx.subscribe();
            let rep = accept_replay.clone();
            let token = token.clone();
            tokio::spawn(async move {
                let client =
                    handle_client(stream, peer, rx, rep, &token, handshake_timeout, permit);
                if let Err(e) = client.await {
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
    token: &str,
    handshake_timeout: Duration,
    handshake_permit: OwnedSemaphorePermit,
) -> Result<()> {
    // The large `Err` type is fixed by tungstenite's `Callback` signature.
    #[allow(clippy::result_large_err)]
    let check = |req: &Request, resp: Response| {
        let result = broadcast_auth::check_handshake(req, resp, token);
        if result.is_err() {
            let origin = req
                .headers()
                .get("origin")
                .and_then(|o| o.to_str().ok())
                .unwrap_or("-");
            eprintln!(
                "camouflage-broadcast: rejected {peer} (origin {origin}): missing or invalid token"
            );
        }
        result
    };
    let ws = tokio::time::timeout(
        handshake_timeout,
        tokio_tungstenite::accept_hdr_async(stream, check),
    )
    .await
    .map_err(|_| anyhow::anyhow!("ws handshake with {peer} timed out"))?
    .with_context(|| format!("ws handshake with {peer}"))?;
    drop(handshake_permit);
    eprintln!("camouflage-broadcast: client connected: {peer}");
    let (mut sink, mut src) = futures_split(ws);

    // Replay buffer first, then live stream.
    {
        let snap: Vec<String> = replay.read().await.clone();
        for line in snap {
            if sink_send(&mut sink, Message::Text(line.into())).await.is_err() {
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
                    if sink_send(&mut sink, Message::Text(line.into())).await.is_err() {
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

/// Percent-encode a token for use in a URL query (user-supplied tokens may
/// contain any characters; generated ones are plain hex).
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
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
