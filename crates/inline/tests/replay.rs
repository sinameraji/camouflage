//! Replay recorded host sessions through the inline session and compare the
//! final screen with a saved copy, so a renderer change that would alter
//! how a real host (autopilot) looks fails here first.
//!
//! Fixtures live in `fixtures/inline/*.ndjson`, one `{"t": ms, "in" | "out":
//! event}` line per event, as written by the SDK with `CAMOUFLAGE_RECORD`.
//! `in` events are what the host sent; `out` events are what the user did
//! (a submitted prompt is typed and entered again, an answered picker is
//! closed). Three more line shapes cover what a recording can't express:
//! `{"t": ms, "resize": [cols, rows]}` for a terminal resize, `{"t": ms,
//! "key": "ctrl+r"}` for a key that isn't a prompt answer, and `{"t": ms,
//! "snapshot": "label"}` to keep the live area as it looks mid-turn. The
//! screen (everything printed, then the live area) is compared with
//! `<name>.screen.txt`; run with `UPDATE_GOLDEN=1` to rewrite those.
//!
//! Every fixture is also held to two invariants that the golden alone can't
//! express: once the last event has landed the session must be idle and ask
//! for no further frames, and the live frame must cost the same after 100k
//! lines of transcript as after 1k.

use camouflage_inline::session::{Key, Session};
use camouflage_inline::Theme;
use camouflage_protocol::{Event, EventType};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const COLS: usize = 100;
const ROWS: usize = 32;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/inline")
}

fn fixture_paths() -> Vec<PathBuf> {
    let mut names: Vec<PathBuf> = std::fs::read_dir(fixtures())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "ndjson"))
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no fixtures in {}", fixtures().display());
    names
}

struct Replayed {
    screen: String,
    session: Session,
    /// The timestamp of the last line in the fixture.
    now: i64,
}

/// The keys a fixture may name in a `{"key": "..."}` line.
fn parse_key(name: &str) -> Option<Key> {
    Some(match name {
        "esc" => Key::Esc,
        "enter" => Key::Enter { shift: false, alt: false },
        "tab" => Key::Tab,
        "backtab" => Key::BackTab,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left { word: false },
        "right" => Key::Right { word: false },
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "backspace" => Key::Backspace { word: false },
        "delete" => Key::Delete { word: false },
        "ctrl+r" => Key::Ctrl('r'),
        "ctrl+o" => Key::Ctrl('o'),
        "ctrl+b" => Key::Ctrl('b'),
        "ctrl+c" => Key::Ctrl('c'),
        "ctrl+u" => Key::Ctrl('u'),
        "?" => Key::Text("?".into()),
        _ => return None,
    })
}

fn replay(path: &Path) -> Replayed {
    let mut s = Session::new("autopilot", Theme::default(), COLS - 1, ROWS);
    let mut printed: Vec<String> = Vec::new();
    let mut now = 0;
    for line in std::fs::read_to_string(path).unwrap().lines().filter(|l| !l.trim().is_empty()) {
        let row: Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("{}: bad line ({e}): {line}", path.display()));
        now = row["t"].as_i64().unwrap_or(now);
        if let Some(size) = row.get("resize").and_then(Value::as_array) {
            let cols = size.first().and_then(Value::as_u64).unwrap_or(COLS as u64) as usize;
            let rows = size.get(1).and_then(Value::as_u64).unwrap_or(ROWS as u64) as usize;
            s.resize(cols, rows);
        } else if let Some(name) = row.get("key").and_then(Value::as_str) {
            // A key the user pressed that isn't a prompt answer: Ctrl+R,
            // Ctrl+O, `?`, and so on.
            if let Some(key) = parse_key(name) {
                s.key(key, now);
            }
        } else if let Some(label) = row.get("snapshot").and_then(Value::as_str) {
            // Keep the live area as it looked at this moment, for states that
            // only exist mid-turn (expanded tools, the shortcuts panel).
            printed.extend(s.take_history(now).iter().map(|l| l.plain_text()));
            printed.push(format!("──── live area at {label} ────"));
            printed.extend(s.live_frame(now).lines.iter().map(|l| l.plain_text()));
            printed.push("────".into());
        } else if let Some(ev) = row.get("in") {
            let Some(event_type) = ev["event_type"].as_str().and_then(EventType::from_str) else { continue };
            let event = Event {
                id: Default::default(),
                session_id: Default::default(),
                seq: 0,
                timestamp_ms: now,
                schema_version: 1,
                event_type,
                payload: ev.get("payload").cloned().unwrap_or(Value::Null),
            };
            s.apply(&event, now);
            // The app clears the terminal on TranscriptCleared; the golden
            // holds what is on screen, so drop what was printed before it.
            if s.clear_screen {
                printed.clear();
                s.clear_screen = false;
            }
        } else if let Some(ev) = row.get("out") {
            let payload = &ev["payload"];
            // A picker answer may carry the filter the user typed first, so
            // the golden records the list narrowed rather than unfiltered.
            if let Some(filter) = payload["filter"].as_str() {
                for ch in filter.chars() {
                    s.key(Key::Text(ch.to_string()), now);
                }
            }
            match ev["event_type"].as_str() {
                Some("UserInputSubmitted") => {
                    for ch in ev["payload"]["text"].as_str().unwrap_or("").chars() {
                        s.key(Key::Text(ch.to_string()), now);
                    }
                    s.key(Key::Enter { shift: false, alt: false }, now);
                }
                Some(kind @ ("SelectListResponse" | "ConfirmResponse" | "FormResponse" | "PermissionResponse" | "WizardCancelled")) => {
                    // Keep what the dialog looked like when the user answered.
                    printed.extend(s.take_history(now).iter().map(|l| l.plain_text()));
                    printed.push(format!("──── live area when the user sent {kind} ────"));
                    printed.extend(s.live_frame(now).lines.iter().map(|l| l.plain_text()));
                    printed.push("────".into());
                    match kind {
                        // A permission prompt only takes keys once it has been
                        // on screen for PERMISSION_ARM_MS, so draw it first.
                        "PermissionResponse" => {
                            s.frame_drawn(now);
                            let at = now + camouflage_inline::session::PERMISSION_ARM_MS;
                            match payload["choice"].as_str().unwrap_or("allow_once") {
                                "allow_session" => {
                                    s.key(Key::Down, at);
                                    s.key(Key::Enter { shift: false, alt: false }, at);
                                }
                                "deny" => s.key(Key::Esc, at),
                                _ => s.key(Key::Enter { shift: false, alt: false }, at),
                            }
                        }
                        "ConfirmResponse" => {
                            if payload["value"].as_bool() == Some(false) {
                                s.key(Key::Down, now);
                            }
                            s.key(Key::Enter { shift: false, alt: false }, now);
                        }
                        // Selects answer with the highlighted option; forms and
                        // wizards are closed, which is what the golden records.
                        "SelectListResponse" => s.key(Key::Enter { shift: false, alt: false }, now),
                        _ => s.key(Key::Esc, now),
                    }
                }
                _ => {}
            }
            s.take_outbound();
        }
        printed.extend(s.take_history(now).iter().map(|l| l.plain_text()));
    }
    let mut screen: Vec<String> = printed;
    screen.push("──── live area ────".into());
    screen.extend(s.live_frame(now).lines.iter().map(|l| l.plain_text()));
    let screen = screen.iter().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n") + "\n";
    Replayed { screen, session: s, now }
}

#[test]
fn recorded_sessions_render_as_before() {
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let mut failures = Vec::new();
    for path in fixture_paths() {
        let got = replay(&path).screen;
        let golden = path.with_extension("screen.txt");
        if update || !golden.exists() {
            std::fs::write(&golden, &got).unwrap();
            continue;
        }
        // Git may check the file out with CRLF (Windows); compare text, not bytes.
        let want = std::fs::read_to_string(&golden).unwrap().replace("\r\n", "\n");
        if got != want {
            let first = got.lines().zip(want.lines()).position(|(a, b)| a != b).unwrap_or_else(|| got.lines().count().min(want.lines().count()));
            failures.push(format!(
                "{}: screen changed at line {}\n  want: {:?}\n  got:  {:?}\n(rerun with UPDATE_GOLDEN=1 if the change is intended, and review the diff)",
                path.file_name().unwrap().to_string_lossy(),
                first + 1,
                want.lines().nth(first).unwrap_or(""),
                got.lines().nth(first).unwrap_or(""),
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// The idle soak: a fixture that has finished must leave the session with
/// nothing to animate. `next_wakeup` returning `Some` while idle means the
/// renderer would spin a timer forever — the bug the soak exists to catch.
#[test]
fn recorded_sessions_settle_to_idle() {
    let mut failures = Vec::new();
    for path in fixture_paths() {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let r = replay(&path);
        if r.session.is_busy() {
            failures.push(format!("{name}: still busy after the last event"));
            continue;
        }
        // Everything that expires on its own (the footer, a toast, the
        // Ctrl+C window) must be gone once time moves past it.
        let settled = r.now + 60_000;
        for at in [settled, settled + 3_600_000] {
            if let Some(next) = r.session.next_wakeup(at) {
                failures.push(format!(
                    "{name}: idle session still wants a frame at {at} (next wakeup {next})"
                ));
                break;
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Build a session whose transcript already holds `lines` printed rows.
fn session_with_transcript(lines: usize) -> Session {
    let mut s = Session::new("autopilot", Theme::default(), COLS - 1, ROWS);
    for i in 0..lines {
        let ev = Event {
            id: Default::default(),
            session_id: Default::default(),
            seq: 0,
            timestamp_ms: i as i64,
            schema_version: 1,
            event_type: EventType::UserMessageCreated,
            payload: serde_json::json!({ "text": format!("line {i} of the transcript") }),
        };
        s.apply(&ev, i as i64);
        s.take_history(i as i64);
    }
    s
}

fn frame_cost(s: &Session, now: i64) -> Duration {
    // Warm up so the first measurement doesn't pay for allocator growth, then
    // take the best of three rounds: a scheduling hiccup on a loaded CI
    // runner must not look like a regression.
    for _ in 0..20 {
        std::hint::black_box(s.live_frame(now));
    }
    let mut best = Duration::MAX;
    for _ in 0..3 {
        let start = Instant::now();
        for _ in 0..200 {
            std::hint::black_box(s.live_frame(now));
        }
        best = best.min(start.elapsed());
    }
    best
}

/// Printed blocks are dropped, so the live frame must cost the same after
/// 100k lines as after 1k. A regression that keeps finished blocks, or
/// re-renders the transcript each frame, shows up here as a ratio far above 1.
#[test]
fn frame_time_does_not_grow_with_transcript_length() {
    let small = session_with_transcript(1_000);
    let large = session_with_transcript(100_000);
    assert!(
        large.held_blocks() <= 4,
        "100k printed lines left {} blocks held in memory",
        large.held_blocks()
    );
    let small_cost = frame_cost(&small, 1_000);
    let large_cost = frame_cost(&large, 100_000);
    // Generous slack: both measurements run on the same machine, so a real
    // regression (linear in transcript length) is orders of magnitude away.
    let budget = small_cost * 4 + Duration::from_millis(20);
    assert!(
        large_cost <= budget,
        "live frame costs {large_cost:?} after 100k lines but {small_cost:?} after 1k \
         (budget {budget:?}); the frame must not depend on transcript length"
    );
}
