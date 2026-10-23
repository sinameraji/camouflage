//! End-to-end check of `camouflage-broadcast`'s access token over a real
//! loopback socket: clients without the token are refused with 401 and get
//! none of the replay buffer; clients with it get the replayed events.
//! Peers that never complete the handshake are disconnected and can't lock
//! out clients that have the token.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest, http::StatusCode, Message};

const TOKEN: &str = "integration-test-token-0123456789";
const EVENT: &str = r#"{"event_type":"UserMessageCreated","payload":{"text":"secret prompt"}}"#;

struct Server {
    child: Child,
    addr: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Start the binary on an ephemeral port, feed it one event, and return the
/// bound address parsed from its startup banner.
fn start_server() -> Server {
    start_server_with(Command::new(env!("CARGO_BIN_EXE_camouflage-broadcast")))
}

fn start_server_with(mut command: Command) -> Server {
    let mut child = command
        .args(["--port", "0", "--token", TOKEN])
        .env_remove("CAMOUFLAGE_BROADCAST_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn camouflage-broadcast");

    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut addr = None;
    let mut line = String::new();
    while addr.is_none() {
        line.clear();
        assert!(
            stderr.read_line(&mut line).unwrap() > 0,
            "server exited before listening"
        );
        if let Some(rest) = line
            .trim()
            .strip_prefix("camouflage-broadcast: listening on ws://")
        {
            addr = Some(rest.to_string());
        }
    }
    // Keep draining stderr so the child never blocks on a full pipe.
    std::thread::spawn(move || for _ in stderr.lines() {});

    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, "{EVENT}").unwrap();
    stdin.flush().unwrap();
    // Leak stdin into a thread so it stays open (closing it exits the server).
    std::thread::spawn(move || {
        let _stdin = stdin;
        std::thread::sleep(Duration::from_secs(60));
    });

    Server {
        child,
        addr: addr.unwrap(),
    }
}

async fn connect_status(req: tungstenite::handshake::client::Request) -> Result<(), StatusCode> {
    match tokio_tungstenite::connect_async(req).await {
        Ok(_) => Ok(()),
        Err(tungstenite::Error::Http(resp)) => Err(resp.status()),
        Err(e) => panic!("unexpected connect error: {e}"),
    }
}

#[tokio::test]
async fn rejects_clients_without_the_token() {
    let server = start_server();
    let base = format!("ws://{}", server.addr);

    for url in [
        format!("{base}/"),
        format!("{base}/?token="),
        format!("{base}/?token=wrong"),
        format!("{base}/?token={TOKEN}x"),
    ] {
        let req = url.as_str().into_client_request().unwrap();
        assert_eq!(
            connect_status(req).await,
            Err(StatusCode::UNAUTHORIZED),
            "{url}"
        );
    }

    // A cross-origin browser page without the token is refused too.
    let mut req = format!("{base}/").into_client_request().unwrap();
    req.headers_mut()
        .insert("origin", "https://evil.example".parse().unwrap());
    assert_eq!(connect_status(req).await, Err(StatusCode::UNAUTHORIZED));
}

#[tokio::test]
async fn replays_to_clients_with_the_token() {
    let server = start_server();

    // Query parameter (what the browser viewer uses), from another origin.
    let mut req = format!("ws://{}/?token={TOKEN}", server.addr)
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("origin", "https://viewer.example".parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .expect("authorized connect");
    let msg = tokio::time::timeout(Duration::from_secs(10), ws.next())
        .await
        .expect("replayed event")
        .unwrap()
        .unwrap();
    assert_eq!(msg, Message::Text(EVENT.into()));

    // Authorization header (non-browser clients).
    let mut req = format!("ws://{}/", server.addr)
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .expect("authorized connect");
    let msg = tokio::time::timeout(Duration::from_secs(10), ws.next())
        .await
        .expect("replayed event")
        .unwrap()
        .unwrap();
    assert_eq!(msg, Message::Text(EVENT.into()));
}

#[test]
fn disconnects_peers_that_never_finish_the_handshake() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_camouflage-broadcast"));
    command.args(["--handshake-timeout-ms", "300"]);
    let server = start_server_with(command);
    let mut silent = std::net::TcpStream::connect(&server.addr).unwrap();
    silent
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut buf = [0u8; 64];
    match silent.read(&mut buf) {
        Ok(0) => {}
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
        other => panic!("silent peer was not disconnected: {other:?}"),
    }
}

/// With a low file-descriptor limit, silent connections used to hold every
/// descriptor forever (and spin the accept loop on EMFILE), so a client with
/// the token could never connect.
#[cfg(unix)]
#[tokio::test]
async fn silent_peers_do_not_lock_out_clients_with_the_token() {
    let mut command = Command::new("/bin/sh");
    command.args([
        "-c",
        r#"ulimit -n 40 && exec "$0" "$@""#,
        env!("CARGO_BIN_EXE_camouflage-broadcast"),
        "--handshake-timeout-ms",
        "300",
    ]);
    let server = start_server_with(command);
    // More silent connections than the server has descriptors for.
    let silent: Vec<_> = (0..100)
        .map(|_| std::net::TcpStream::connect(&server.addr).unwrap())
        .collect();

    let req = format!("ws://{}/?token={TOKEN}", server.addr)
        .into_client_request()
        .unwrap();
    let (mut ws, _) = tokio::time::timeout(
        Duration::from_secs(10),
        tokio_tungstenite::connect_async(req),
    )
    .await
    .expect("authorized connect timed out")
    .expect("authorized connect");
    let msg = tokio::time::timeout(Duration::from_secs(10), ws.next())
        .await
        .expect("replayed event")
        .unwrap()
        .unwrap();
    assert_eq!(msg, Message::Text(EVENT.into()));
    drop(silent);
}
