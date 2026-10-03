//! End-to-end tests for `--ui inline`: run the real binary on a pseudo-
//! terminal, act as the host on stdin and as the user on the tty, and check
//! what a terminal emulator would show. Also the idle soak from
//! docs/inline-redesign.md §4.2: idling must cost no CPU and write nothing.

#![cfg(unix)]

use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const COLS: u16 = 90;
const ROWS: u16 = 30;

struct Pty {
    child: Child,
    out: Arc<Mutex<Vec<u8>>>,
    master: std::fs::File,
    _db: tempdir::TempDir,
}

mod tempdir {
    pub struct TempDir(pub std::path::PathBuf);
    impl TempDir {
        pub fn new() -> Self {
            let p = std::env::temp_dir().join(format!("camo-pty-{}-{}", std::process::id(), rand()));
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn rand() -> u128 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    }
}

impl Pty {
    fn spawn() -> Pty {
        let (mut master, mut slave): (RawFd, RawFd) = (0, 0);
        let mut ws = libc::winsize { ws_row: ROWS, ws_col: COLS, ws_xpixel: 0, ws_ypixel: 0 };
        // SAFETY: plain openpty with valid out-pointers.
        let rc = unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &mut ws) };
        assert_eq!(rc, 0, "openpty failed");
        let db = tempdir::TempDir::new();
        let slave_fd = slave;
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_camouflage-tui"));
        cmd.env("CAMOUFLAGE_STATS_PATH", db.0.join("stats.json"));
        cmd.args(["--ui", "inline", "--stdin-events", "--emit-responses=false", "--app-title", "autopilot", "--db"])
            .arg(db.0.join("s.db"))
            .env("TERM", "xterm-256color")
            .stdin(Stdio::piped())
            // SAFETY: each Stdio takes ownership of its own dup of the slave.
            .stdout(unsafe { Stdio::from(OwnedFd::from_raw_fd(libc::dup(slave_fd))) })
            .stderr(Stdio::null());
        // Make the pty the child's controlling terminal so /dev/tty works.
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(move || {
                libc::setsid();
                libc::ioctl(slave_fd, libc::TIOCSCTTY as _, 0);
                Ok(())
            });
        }
        let child = cmd.spawn().expect("spawn camouflage-tui");
        unsafe { libc::close(slave) };
        let master = unsafe { std::fs::File::from_raw_fd(master) };
        let out = Arc::new(Mutex::new(Vec::new()));
        {
            let out = out.clone();
            let mut reader = master.try_clone().unwrap();
            let mut writer = master.try_clone().unwrap();
            std::thread::spawn(move || {
                let mut buf = [0u8; 65536];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let chunk = &buf[..n];
                    // Answer crossterm's keyboard-enhancement probe (and
                    // the device-attributes query it sends with it).
                    if chunk.windows(4).any(|w| w == b"\x1b[?u") {
                        let _ = writer.write_all(b"\x1b[?0u\x1b[?1;2c");
                    } else if chunk.windows(3).any(|w| w == b"\x1b[c") {
                        let _ = writer.write_all(b"\x1b[?1;2c");
                    }
                    out.lock().unwrap().extend_from_slice(chunk);
                }
            });
        }
        let p = Pty { child, out, master, _db: db };
        std::thread::sleep(Duration::from_millis(400));
        p
    }

    fn host(&mut self, event_type: &str, payload: serde_json::Value) {
        let line = serde_json::json!({ "event_type": event_type, "payload": payload }).to_string();
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();
    }

    fn keys(&mut self, bytes: &[u8]) {
        self.master.write_all(bytes).unwrap();
        self.master.flush().unwrap();
    }

    fn len(&self) -> usize {
        self.out.lock().unwrap().len()
    }

    /// Wait until the emulated screen (or scrollback) shows `needle`.
    fn wait_for(&self, needle: &str) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let all = self.all_lines();
            if all.iter().any(|l| l.contains(needle)) {
                return all;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {needle:?}; screen:\n{}", all.join("\n"));
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn all_lines(&self) -> Vec<String> {
        let bytes = self.out.lock().unwrap().clone();
        let mut vt = vt100::Parser::new(ROWS, COLS, 5000);
        vt.process(&bytes);
        let mut back = 0;
        loop {
            vt.screen_mut().set_scrollback(back + 1);
            if vt.screen().scrollback() == back {
                break;
            }
            back += 1;
        }
        let mut lines = Vec::new();
        for offset in (1..=back).rev() {
            vt.screen_mut().set_scrollback(offset);
            lines.push(vt.screen().rows(0, COLS).next().unwrap_or_default().trim_end().to_string());
        }
        vt.screen_mut().set_scrollback(0);
        lines.extend(vt.screen().rows(0, COLS).map(|r| r.trim_end().to_string()));
        lines
    }

    /// Close stdin (the host going away) and return the child's CPU time
    /// and how many times its event loop woke up on a timer.
    fn finish(mut self) -> (Duration, u64) {
        drop(self.child.stdin.take());
        let pid = self.child.id() as libc::pid_t;
        let mut status = 0;
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        // SAFETY: waiting on our own child with valid out-pointers.
        let rc = unsafe { libc::wait4(pid, &mut status, 0, &mut usage) };
        assert_eq!(rc, pid);
        let tv = |t: libc::timeval| Duration::from_secs(t.tv_sec as u64) + Duration::from_micros(t.tv_usec as u64);
        let stats: serde_json::Value = std::fs::read_to_string(self._db.0.join("stats.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        // Timer wakeups only: host events and keys legitimately wake the loop
        // (and a burst arrives in a varying number of batches).
        let ticks = stats["by_source"]["tick"].as_u64().unwrap_or(0);
        if stats.is_null() {
            return (tv(usage.ru_utime) + tv(usage.ru_stime), u64::MAX);
        }
        (tv(usage.ru_utime) + tv(usage.ru_stime), ticks)
    }
}

fn nonempty(lines: &[String]) -> Vec<&str> {
    lines.iter().map(|s| s.as_str()).filter(|s| !s.is_empty()).collect()
}

#[test]
fn a_turn_renders_inline_and_stays_after_exit() {
    let mut t = Pty::spawn();
    t.host("SessionStarted", serde_json::json!({ "title": "autopilot 1.3.0", "detail": ["kimi-k2.6 · ~/autopilot"] }));
    t.wait_for("autopilot 1.3.0");

    // The user types (capital first letter, non-ASCII) and submits.
    t.keys("The café build fails 👍".as_bytes());
    t.wait_for("The café build fails 👍");
    t.keys(b"\r");
    t.wait_for("› The café build fails 👍");

    t.host("StatusUpdate", serde_json::json!({ "segments": { "phase": "thinking" } }));
    t.host("ToolExecutionStarted", serde_json::json!({ "tool_id": "t1", "tool": "Bash", "command": "npm run build" }));
    t.host("ToolExecutionStdout", serde_json::json!({ "tool_id": "t1", "chunk": "src/a.ts:3:1 - error TS2345\n" }));
    t.host("ToolExecutionFinished", serde_json::json!({ "tool_id": "t1", "exit_code": 2 }));
    t.host("AssistantStreamStarted", serde_json::json!({ "stream_id": "s" }));
    t.host("AssistantTokenDelta", serde_json::json!({ "stream_id": "s", "token": "Fixed **it**.\n\n- one\n- two" }));
    t.host("AssistantMessageCompleted", serde_json::json!({ "stream_id": "s" }));
    t.host("StatusUpdate", serde_json::json!({ "segments": { "phase": "idle" } }));
    let screen = t.wait_for("• two");
    let text = nonempty(&screen);
    let pos = |needle: &str| text.iter().position(|l| l.contains(needle)).unwrap_or_else(|| panic!("{needle:?} missing:\n{}", text.join("\n")));
    assert!(pos("› The café build fails") < pos("✗ Bash npm run build · Exit code 2"));
    assert!(pos("✗ Bash npm run build") < pos("error TS2345"));
    assert!(pos("Fixed it.") < pos("• one"));
    assert!(text.iter().any(|l| l.contains("error TS2345")), "failed output is shown");

    // Each printed line appears exactly once (nothing re-printed).
    assert_eq!(text.iter().filter(|l| l.contains("✗ Bash npm run build")).count(), 1);

    // Ctrl+C twice exits; the transcript remains, the input box does not.
    t.keys(b"\x03");
    std::thread::sleep(Duration::from_millis(150));
    t.keys(b"\x03");
    std::thread::sleep(Duration::from_millis(300));
    let after = t.all_lines();
    t.finish();
    assert!(after.iter().any(|l| l.contains("Fixed it.")));
    assert!(!after.iter().any(|l| l.contains("╭")), "input box removed on exit");
}

#[test]
fn idle_costs_nothing() {
    let run = |idle: Duration| -> (Duration, u64, usize) {
        let mut t = Pty::spawn();
        t.host("SessionStarted", serde_json::json!({ "title": "soak" }));
        for i in 0..300 {
            t.host("UserMessageCreated", serde_json::json!({ "text": format!("message {i} with some words in it") }));
        }
        t.wait_for("message 299");
        std::thread::sleep(Duration::from_millis(200));
        let before = t.len();
        std::thread::sleep(idle);
        let written = t.len() - before;
        let (cpu, wakeups) = t.finish();
        (cpu, wakeups, written)
    };
    let (short_cpu, short_wakeups, _) = run(Duration::from_millis(300));
    let (long_cpu, long_wakeups, written) = run(Duration::from_millis(2500));
    assert_eq!(written, 0, "an idle renderer must not write to the terminal");
    // At most one timer wakeup: the batched draw after the burst of host
    // events (frames are capped at 20 fps). Idling longer must add none.
    assert!(short_wakeups <= 1, "nothing animates here ({short_wakeups} timer wakeups)");
    assert_eq!(long_wakeups, short_wakeups, "an idle renderer must not wake on a timer ({long_wakeups} vs {short_wakeups} timer wakeups after 2.5s vs 0.3s idle)");
    let extra = long_cpu.saturating_sub(short_cpu);
    assert!(extra < Duration::from_millis(60), "2.2s more idle cost {extra:?} of CPU (short {short_cpu:?}, long {long_cpu:?})");
}

/// The SDK's default mode: stdout is a pipe carrying outbound events and the
/// renderer draws on /dev/tty. Nothing but NDJSON may appear on that pipe,
/// and the very first event must arrive intact.
#[test]
fn piped_stdout_carries_only_clean_events() {
    let (mut master, mut slave): (RawFd, RawFd) = (0, 0);
    let mut ws = libc::winsize { ws_row: ROWS, ws_col: COLS, ws_xpixel: 0, ws_ypixel: 0 };
    let rc = unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &mut ws) };
    assert_eq!(rc, 0);
    let db = tempdir::TempDir::new();
    let slave_fd = slave;
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_camouflage-tui"));
    cmd.args(["--ui", "inline", "--stdin-events", "--emit-responses", "--db"])
        .arg(db.0.join("s.db"))
        .env("TERM", "xterm-256color")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(move || {
            libc::setsid();
            libc::ioctl(slave_fd, libc::TIOCSCTTY as _, 0);
            Ok(())
        });
    }
    let mut child = cmd.spawn().unwrap();
    unsafe { libc::close(slave) };
    let mut tty = unsafe { std::fs::File::from_raw_fd(master) };
    // Drain the tty so the renderer never blocks drawing, keeping what it
    // drew so we know when it's ready for keys.
    let drawn = Arc::new(Mutex::new(Vec::new()));
    let mut drain = tty.try_clone().unwrap();
    {
        let drawn = drawn.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 65536];
            while let Ok(n) = drain.read(&mut buf) {
                if n == 0 {
                    break;
                }
                drawn.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });
    }
    let mut events = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut all = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = events.read(&mut buf) {
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buf[..n]);
            let _ = tx.send(all.clone());
        }
    });
    // Type only once the input box is on screen: keys sent before the
    // renderer switches the tty to raw mode can be discarded.
    let ready = Instant::now() + Duration::from_secs(5);
    while Instant::now() < ready && !String::from_utf8_lossy(&drawn.lock().unwrap()).contains('╭') {
        std::thread::sleep(Duration::from_millis(20));
    }
    tty.write_all(b"/help\r").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut got = Vec::new();
    while Instant::now() < deadline && !got.ends_with(b"\n") {
        if let Ok(b) = rx.recv_timeout(Duration::from_millis(100)) {
            got = b;
        }
    }
    let _ = child.kill();
    let text = String::from_utf8_lossy(&got).to_string();
    let first = text.lines().next().unwrap_or("");
    let ev: serde_json::Value = serde_json::from_str(first).unwrap_or_else(|e| panic!("first outbound line isn't JSON ({e}): {first:?}"));
    assert_eq!(ev["event_type"], "UserInputSubmitted");
    assert_eq!(ev["payload"]["text"], "/help");
}

/// `!` commands: the host suspends the renderer, a child owns the terminal
/// in cooked mode, then the renderer takes it back and redraws.
#[test]
fn terminal_suspend_hands_the_tty_back_and_resume_redraws() {
    let (mut master, mut slave): (RawFd, RawFd) = (0, 0);
    let mut ws = libc::winsize { ws_row: ROWS, ws_col: COLS, ws_xpixel: 0, ws_ypixel: 0 };
    let rc = unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &mut ws) };
    assert_eq!(rc, 0);
    let db = tempdir::TempDir::new();
    let slave_fd = slave;
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_camouflage-tui"));
    cmd.args(["--ui", "inline", "--stdin-events", "--emit-responses", "--db"])
        .arg(db.0.join("s.db"))
        .env("TERM", "xterm-256color")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(move || {
            libc::setsid();
            libc::ioctl(slave_fd, libc::TIOCSCTTY as _, 0);
            Ok(())
        });
    }
    let mut child = cmd.spawn().unwrap();
    let drawn = Arc::new(Mutex::new(Vec::new()));
    let mut drain = unsafe { std::fs::File::from_raw_fd(master) };
    {
        let drawn = drawn.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 65536];
            while let Ok(n) = drain.read(&mut buf) {
                if n == 0 {
                    break;
                }
                drawn.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });
    }
    let mut events = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut all = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = events.read(&mut buf) {
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buf[..n]);
            let _ = tx.send(String::from_utf8_lossy(&all).to_string());
        }
    });
    let wait_drawn = |count: usize| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && String::from_utf8_lossy(&drawn.lock().unwrap()).matches('╭').count() < count {
            std::thread::sleep(Duration::from_millis(20));
        }
        String::from_utf8_lossy(&drawn.lock().unwrap()).matches('╭').count()
    };
    // The first frame comes with the host's first event.
    child.stdin.as_mut().unwrap().write_all(b"{\"event_type\":\"SessionStarted\",\"payload\":{}}\n").unwrap();
    assert!(wait_drawn(1) >= 1, "input box never drawn");
    let raw_mode = || {
        let mut t: libc::termios = unsafe { std::mem::zeroed() };
        unsafe { libc::tcgetattr(slave_fd, &mut t) };
        t.c_lflag & libc::ICANON == 0
    };
    assert!(raw_mode(), "renderer runs the tty in raw mode");

    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"{\"event_type\":\"TerminalSuspend\",\"payload\":{\"id\":\"s1\"}}\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut got = String::new();
    while Instant::now() < deadline && !got.contains("TerminalSuspended") {
        if let Ok(s) = rx.recv_timeout(Duration::from_millis(100)) {
            got = s;
        }
    }
    let line = got.lines().find(|l| l.contains("TerminalSuspended")).unwrap_or_else(|| panic!("no TerminalSuspended: {got:?}"));
    let ev: serde_json::Value = serde_json::from_str(line).unwrap();
    assert_eq!(ev["payload"]["id"], "s1");
    assert_eq!(ev["payload"]["supported"], true);
    assert!(!raw_mode(), "the child gets a cooked tty");

    let before = wait_drawn(0);
    stdin.write_all(b"{\"event_type\":\"TerminalResume\",\"payload\":{\"id\":\"s1\"}}\n").unwrap();
    assert!(wait_drawn(before + 1) > before, "input box redrawn after resume");
    assert!(raw_mode(), "raw mode again after resume");
    let _ = child.kill();
    unsafe { libc::close(slave) };
}
