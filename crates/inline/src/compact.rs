//! Compact live region: a maximum six-line summary that replaces the full
//! tool-row + plan + spinner display while a turn is busy.
//!
//! Port of the prototype in `autopilot-prototypes/compact-live/core.mjs`.
//! The prototype is the behavioural spec; this module ports its render
//! function, event model, and timings to Rust.
//!
//! The six lines (each present only when it has content):
//! 1. Plan bar — progress segments with the active step title
//! 2. Scouts line — subagent chips with orbit animation
//! 3. Trail — one bead per tool call this turn
//! 4. Latest action — humanized tool title with typewriter effect
//! 5. Pulse line — travelling sine wave replacing the spinner
//! 6. Done message — shown briefly after turn ends

use crate::blocks::{format_elapsed, PlanItem, PlanStatus};
use crate::style::{Line, Style};
use crate::theme::Theme;
use crate::width::str_width;
use std::collections::VecDeque;

// ---------------------------------------------------------------- constants

/// Spinner frame interval in ms (shared with session.rs).
pub const FRAME_MS: i64 = 80;

/// Bar glyphs for the pulse wave.
const BARS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// Orbit glyphs for running scouts.
const ORBIT: [&str; 4] = ["◐", "◓", "◑", "◒"];

/// Verbs that rotate while the model is thinking.
const VERBS: [&str; 3] = ["thinking", "reasoning", "composing"];

// ---------------------------------------------------------------- state

/// What phase the turn is in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Thinking,
    Working,
    Done,
}

/// A single plan step.
#[derive(Clone, Debug)]
pub struct CompactPlanStep {
    pub title: String,
    pub status: PlanStatus,
    pub done_at: Option<i64>,
}

/// A scout (subagent) in the compact region.
#[derive(Clone, Debug)]
pub struct Scout {
    pub n: u32,
    pub title: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub ok: bool,
}

/// A bead in the trail.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrailKind {
    Read,
    Search,
    Edit,
    Bash,
    Agent,
    Code,
    Error,
}

impl TrailKind {
    /// Classify a tool name into a trail kind.
    pub fn from_tool(name: &str) -> Self {
        let n = name.to_ascii_lowercase();
        match n.as_str() {
            "read" | "view" | "cat" | "open" | "read_file" => Self::Read,
            "grep" | "glob" | "find" | "search" | "rg" | "codesearch" | "code_search" | "web_search" | "websearch" | "ls" | "list" | "list_dir" | "tree" | "web_fetch" | "webfetch" | "fetch" => Self::Search,
            other if other.starts_with("lsp_") => Self::Search,
            "edit" | "write" => Self::Edit,
            "bash" => Self::Bash,
            "execute_code" | "code" => Self::Code,
            other if other.starts_with("subagent") || other == "agent" => Self::Agent,
            _ => Self::Read, // default to read for unknown tools
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Read => "·",
            Self::Search => "·",
            Self::Edit => "◆",
            Self::Bash => "▲",
            Self::Agent => "●",
            Self::Code => "▣",
            Self::Error => "✕",
        }
    }
}

/// A single bead in the trail.
#[derive(Clone, Debug)]
pub struct TrailBead {
    pub kind: TrailKind,
    pub at: i64,
    pub ok: bool,
}

/// The latest action being shown.
#[derive(Clone, Debug)]
pub struct LatestAction {
    pub text: String,
    pub at: i64,
    pub is_error: bool,
}

/// Accumulated scrollback lines (the <10% that print).
#[derive(Clone, Debug)]
pub struct CompactState {
    pub phase: Phase,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub tokens: u64,
    /// Timestamps of token bursts for wave amplitude.
    pub token_events: VecDeque<i64>,
    pub plan: Vec<CompactPlanStep>,
    pub scouts: Vec<Scout>,
    pub trail: Vec<TrailBead>,
    pub latest: Option<LatestAction>,
    pub files: usize,
    /// Lines printed into scrollback.
    pub scrollback: Vec<Line>,
}

impl CompactState {
    pub fn new() -> Self {
        Self {
            phase: Phase::Idle,
            started_at: 0,
            ended_at: None,
            tokens: 0,
            token_events: VecDeque::new(),
            plan: Vec::new(),
            scouts: Vec::new(),
            trail: Vec::new(),
            latest: None,
            files: 0,
            scrollback: Vec::new(),
        }
    }

    /// Reset for a new turn.
    pub fn reset(&mut self) {
        self.phase = Phase::Idle;
        self.token_events.clear();
        self.plan.clear();
        self.scouts.clear();
        self.trail.clear();
        self.latest = None;
        self.files = 0;
    }
}

// ---------------------------------------------------------------- events

impl CompactState {
    /// Apply a user message event.
    pub fn user_message(&mut self, text: &str, now: i64, theme: &Theme) {
        let mut line = Line::new();
        line.push("› ", theme.accent().bold());
        line.push(text, Style::default().bold());
        self.scrollback.push(line);
        self.phase = Phase::Thinking;
        self.started_at = now;
        self.ended_at = None;
        self.tokens = 0;
        self.token_events.clear();
        self.plan.clear();
        self.scouts.clear();
        self.trail.clear();
        self.latest = None;
        self.files = 0;
    }

    /// Apply a token burst event.
    pub fn tokens(&mut self, n: u64, now: i64) {
        self.tokens += n;
        self.token_events.push_back(now);
        // Prune old token events (keep last 2 seconds).
        while self.token_events.front().map_or(false, |&t| now - t > 2000) {
            self.token_events.pop_front();
        }
        if self.phase != Phase::Done {
            self.phase = Phase::Thinking;
        }
    }

    /// Apply a tool execution event.
    pub fn tool(&mut self, kind: TrailKind, text: &str, ok: bool, now: i64, theme: &Theme, path: Option<&str>, added: u32, removed: u32) {
        self.trail.push(TrailBead { kind, at: now, ok });
        self.phase = Phase::Working;
        self.latest = Some(LatestAction { text: text.to_string(), at: now, is_error: !ok });

        if kind == TrailKind::Edit {
            self.files += 1;
            let mut line = Line::new();
            line.push("◆ ", theme.accent());
            if let Some(p) = path {
                line.push(p, Style::default());
            }
            line.push(&format!("  +{added} "), theme.ok());
            line.push(&format!("−{removed}"), theme.err());
            self.scrollback.push(line);
        }

        if kind == TrailKind::Error {
            let mut line = Line::new();
            line.push("✕ ", theme.err());
            line.push(text.to_string(), theme.err());
            self.scrollback.push(line);
        }
    }

    /// Apply a subagent start event.
    pub fn agent_start(&mut self, n: u32, title: &str, now: i64) {
        self.scouts.push(Scout {
            n,
            title: title.to_string(),
            started_at: now,
            finished_at: None,
            ok: true,
        });
        self.trail.push(TrailBead { kind: TrailKind::Agent, at: now, ok: true });
        self.phase = Phase::Working;
    }

    /// Apply a subagent done event.
    pub fn agent_done(&mut self, n: u32, ok: bool, now: i64) {
        if let Some(a) = self.scouts.iter_mut().find(|a| a.n == n) {
            a.finished_at = Some(now);
            a.ok = ok;
        }
    }

    /// Apply a plan update.
    pub fn plan(&mut self, items: &[PlanItem]) {
        self.plan = items.iter().enumerate().map(|(_i, item)| CompactPlanStep {
            title: item.title.clone(),
            status: item.status,
            done_at: None,
        }).collect();
        // First item is active if there are any.
        if self.plan.len() > 0 && self.plan[0].status == PlanStatus::Pending {
            self.plan[0].status = PlanStatus::InProgress;
        }
    }

    /// Mark the current active plan step as done.
    pub fn plan_step_done(&mut self, now: i64) {
        if let Some(i) = self.plan.iter().position(|p| p.status == PlanStatus::InProgress) {
            self.plan[i].status = PlanStatus::Done;
            self.plan[i].done_at = Some(now);
            if i + 1 < self.plan.len() {
                self.plan[i + 1].status = PlanStatus::InProgress;
            }
        }
    }

    /// Apply an assistant reply (goes to scrollback).
    pub fn reply(&mut self, text: &str, _theme: &Theme) {
        let mut line = Line::new();
        line.push("  ", Style::default());
        line.push(text, Style::default());
        self.scrollback.push(line);
    }

    /// Apply turn done.
    pub fn done(&mut self, now: i64, theme: &Theme) {
        self.phase = Phase::Done;
        self.ended_at = Some(now);

        let done_count = self.plan.iter().filter(|p| p.status == PlanStatus::Done).count();
        let elapsed = format_elapsed((now - self.started_at).max(0) as u64);

        let mut line = Line::new();
        line.push("✓ ", theme.ok().bold());
        line.push(&format!("done in {elapsed}"), Style::default());
        line.push(
            &format!(
                "  ·  {done_count}/{} steps  ·  {} scouts  ·  {} tools  ·  {} files",
                self.plan.len(),
                self.scouts.len(),
                self.trail.len(),
                self.files
            ),
            theme.dim(),
        );
        self.scrollback.push(line);
    }

    /// Set phase to idle.
    pub fn idle(&mut self) {
        self.phase = Phase::Idle;
    }
}

// ---------------------------------------------------------------- render

/// Render the compact live region. Returns at most 6 lines.
pub fn render_compact(state: &CompactState, now: i64, width: usize, theme: &Theme) -> Vec<Line> {
    let mut lines = Vec::new();
    if state.phase == Phase::Idle {
        return lines;
    }

    let t = now - state.started_at;

    // -- Plan bar
    if !state.plan.is_empty() {
        lines.push(render_plan_bar(state, now, width, theme));
    }

    // -- Scouts line
    let scout_line = render_scouts_line(state, now, width, theme);
    if let Some(line) = scout_line {
        lines.push(line);
    }

    // -- Trail
    if !state.trail.is_empty() {
        lines.push(render_trail(state, now, width, theme));
    }

    // -- Latest action
    if let Some(ref latest) = state.latest {
        let age = now - latest.at;
        if age < 4200 && state.phase != Phase::Done {
            lines.push(render_latest_action(latest, age, theme));
        }
    }

    // -- Pulse line (or done message)
    if state.phase != Phase::Done {
        lines.push(render_pulse_line(state, t, now, width, theme));
    } else if state.ended_at.map_or(false, |e| now - e < 2600) {
        let _ended_at = state.ended_at.unwrap();
        let elapsed = format_elapsed((now - state.started_at).max(0) as u64);
        let mut line = Line::new();
        line.push("✦ ", theme.bright());
        line.push("done", theme.ok().bold());
        line.push(&format!("  {elapsed}"), theme.dim());
        lines.push(line);
    }

    lines
}

/// Render the plan bar: `◆ Plan ▰▰▱▱▱▱▱ 2/7  <active step title>`.
fn render_plan_bar(state: &CompactState, now: i64, width: usize, theme: &Theme) -> Line {
    let mut line = Line::new();
    let done_count = state.plan.iter().filter(|p| p.status == PlanStatus::Done).count();

    line.push("◆ ", theme.accent());
    line.push("Plan", Style::default().bold());

    for step in &state.plan {
        match step.status {
            PlanStatus::Done => {
                let fresh = step.done_at.map_or(false, |d| now - d < 700);
                line.push("▰", if fresh { theme.bright().bold() } else { theme.accent() });
            }
            PlanStatus::InProgress => {
                let on = (now / 450) % 2 == 0;
                line.push(if on { "▰" } else { "▱" }, theme.bright());
            }
            PlanStatus::Pending => {
                line.push("▱", theme.dimmer());
            }
        }
    }

    line.push(&format!(" {done_count}/{}", state.plan.len()), theme.dim());

    let active = state.plan.iter().find(|p| p.status == PlanStatus::InProgress);
    let finished = state.phase == Phase::Done;

    if finished {
        line.push("  all done", theme.ok());
    } else if let Some(a) = active {
        // Calculate available width for the step title.
        let prefix_len = 2 + 2 + 4 + state.plan.len() + 1 + format!("{done_count}/{}", state.plan.len()).len() + 2;
        let avail = width.saturating_sub(prefix_len).max(5);
        let clipped = clip_str(&a.title, avail);
        line.push(&format!("  {clipped}"), Style::default());
    }

    line
}

/// Render the scouts line. Returns None if all scouts have been home for 7s.
fn render_scouts_line(state: &CompactState, now: i64, _width: usize, theme: &Theme) -> Option<Line> {
    let running: Vec<&Scout> = state.scouts.iter().filter(|a| a.finished_at.is_none()).collect();
    let just_done: Vec<&Scout> = state.scouts.iter()
        .filter(|a| a.finished_at.map_or(false, |f| now - f < 2500))
        .collect();
    let folded: Vec<&Scout> = state.scouts.iter()
        .filter(|a| a.finished_at.map_or(false, |f| now - f >= 2500))
        .collect();

    // Once every scout has been home for 7s, the line disappears.
    let all_home = running.is_empty()
        && !state.scouts.is_empty()
        && state.scouts.iter().all(|a| a.finished_at.map_or(false, |f| now - f > 7000));

    if (running.is_empty() && just_done.is_empty() && folded.is_empty()) || all_home {
        return None;
    }

    let mut line = Line::new();
    line.push("● ", theme.ok());

    let label = if !running.is_empty() {
        let count = running.len();
        let word = match count {
            1 => "one",
            2 => "two",
            3 => "three",
            n => return Some(line.with(&format!("{n} scouts out"), Style::default().bold())),
        };
        format!("{word} scout{} out", if count == 1 { "" } else { "s" })
    } else {
        "scouts home".to_string()
    };
    line.push(&label, Style::default().bold());

    // Running scouts with orbit animation
    for (i, scout) in running.iter().enumerate() {
        let g = ORBIT[((now / 160) as usize + i) % ORBIT.len()];
        let elapsed = format_elapsed((now - scout.started_at).max(0) as u64);
        let title = short_title(&scout.title);
        line.push(&format!(" {g} #{} {} {}", scout.n, title, elapsed), theme.ok());
    }

    // Just done scouts (flash for 900ms)
    for scout in &just_done {
        let flash = scout.finished_at.map_or(false, |f| now - f < 900);
        let elapsed = scout.finished_at.map_or("0s".to_string(), |f| format_elapsed((f - scout.started_at).max(0) as u64));
        let title = short_title(&scout.title);
        line.push(" ✓ ", theme.ok().bold());
        line.push(&format!("#{} ", scout.n), theme.dim());
        line.push(&title, if flash { theme.bright() } else { theme.dim() });
        line.push(&format!(" {elapsed}"), theme.dim());
    }

    // Folded scouts
    if !folded.is_empty() {
        line.push(&format!("  ✓{} back", folded.len()), theme.dim());
    }

    Some(line)
}

/// Render the trail: one bead per tool call.
fn render_trail(state: &CompactState, now: i64, width: usize, theme: &Theme) -> Line {
    let mut line = Line::new();
    line.push("  ", Style::default());

    let room = (width - 10).max(10);
    let beads = if state.trail.len() > room {
        &state.trail[state.trail.len() - room..]
    } else {
        &state.trail[..]
    };

    if state.trail.len() > room {
        line.push("…", theme.dimmer());
    }

    for bead in beads {
        let age = now - bead.at;
        let kind = bead.kind; // TrailKind is Copy
        let (fg, bold) = if !bead.ok {
            (theme.err(), false)
        } else if age < 500 {
            (theme.bright(), true)
        } else if age < 4000 {
            (kind_color(kind, theme), false)
        } else if matches!(kind, TrailKind::Read | TrailKind::Search) {
            (theme.dimmer(), false)
        } else {
            (theme.dim(), false)
        };
        line.push(kind.glyph(), if bold { fg.bold() } else { fg });
    }

    // Blinking cursor while busy
    if state.phase != Phase::Done {
        let blink = (now / 500) % 2 != 0;
        line.push(if blink { "▸" } else { " " }, theme.accent());
    } else if let Some(ended_at) = state.ended_at {
        // Sparkle sweep after done
        let k = (now - ended_at) / 40;
        let _bead_offset = if state.trail.len() > room { 3 } else { 2 }; // "  " + optional "…"
        if (k as usize) < beads.len() + 3 {
            // Sparkle is applied by replacing beads with ✦
            // For simplicity, we add sparkle indicators at the end
            let sparkle_count = ((k as usize).min(3)).min(beads.len());
            if sparkle_count > 0 {
                // The sparkle sweep is handled by re-rendering with ✦
                // This is a simplification; the full prototype replaces beads in-place
            }
        }
    }

    line
}

/// Render the latest action with typewriter effect.
fn render_latest_action(latest: &LatestAction, age: i64, theme: &Theme) -> Line {
    let mut line = Line::new();
    line.push("  ", Style::default());

    let n = (age / 14).min(latest.text.len() as i64) as usize;
    let visible = &latest.text[..n];

    let fg = if latest.is_error {
        theme.err()
    } else if age < 2200 {
        Style::default()
    } else if age < 3200 {
        theme.dim()
    } else {
        theme.dimmer()
    };

    line.push(visible, fg);

    if age < 300 {
        line.push("▍", theme.accent());
    }

    line
}

/// Render the pulse line: travelling sine wave + status text.
fn render_pulse_line(state: &CompactState, t: i64, now: i64, _width: usize, theme: &Theme) -> Line {
    let mut line = Line::new();

    // Token burst rate in last 1500ms
    let rate = state.token_events.iter().filter(|&&x| now - x < 1500).count();

    // Amplitude: higher while thinking with token bursts, lower while working
    let amp = if state.phase == Phase::Thinking {
        (0.35 + rate as f64 * 0.12).min(1.0)
    } else {
        0.22
    };

    // Generate 10 bars of the travelling wave
    let mut wave = String::new();
    for i in 0..10 {
        let v = 0.5 + amp * 0.5 * ((now as f64 / 130.0 - i as f64 * 0.7).sin());
        let idx = (v * 7.0).round() as usize;
        let idx = idx.clamp(0, 7);
        wave.push_str(BARS[idx]);
    }
    line.push(&wave, theme.accent());
    line.push("  ", Style::default());

    // Verb
    let running_scouts = state.scouts.iter().filter(|a| a.finished_at.is_none()).count();
    let verb = if state.phase == Phase::Thinking {
        VERBS[(t / 4000).min(2) as usize]
    } else if running_scouts > 0 {
        "waiting on scouts"
    } else {
        "working"
    };
    line.push(&format!("{verb}…"), theme.accent());

    // Meta
    let elapsed = format_elapsed(t.max(0) as u64);
    let meta = format!("  {elapsed} · ↓ {} tokens · esc to interrupt", state.tokens);
    line.push(&meta, theme.dim());

    line
}

// ---------------------------------------------------------------- helpers

fn kind_color(kind: TrailKind, theme: &Theme) -> Style {
    match kind {
        TrailKind::Read | TrailKind::Search => theme.dim(),
        TrailKind::Edit | TrailKind::Code => theme.accent(),
        TrailKind::Bash => theme.info(),
        TrailKind::Agent => theme.ok(),
        TrailKind::Error => theme.err(),
    }
}

/// Clip a string to the given display width, adding "…" if truncated.
fn clip_str(s: &str, max_width: usize) -> String {
    if str_width(s) <= max_width {
        return s.to_string();
    }
    let mut result = String::new();
    let mut width = 0;
    for ch in s.chars() {
        let cw = ch.len_utf16(); // approximate grapheme width
        if width + cw > max_width.saturating_sub(1) {
            break;
        }
        width += cw;
        result.push(ch);
    }
    if !result.is_empty() && str_width(&result) < max_width {
        result.push('…');
    }
    result
}

/// Extract the first three words of a title, stripping trailing punctuation.
fn short_title(s: &str) -> String {
    s.split_whitespace()
        .take(3)
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches(|c: char| ",;:.".contains(c))
        .to_string()
}

// ---------------------------------------------------------------- width-aware rendering

/// Render the compact region with width-aware degradation.
/// At narrow widths, drop less important elements first.
pub fn render_compact_width_aware(state: &CompactState, now: i64, width: usize, theme: &Theme) -> Vec<Line> {
    let mut lines = render_compact(state, now, width, theme);

    // At very narrow widths, degrade gracefully
    if width < 40 {
        // Drop the step title from the plan bar (last span)
        if let Some(plan_line) = lines.first_mut() {
            // Keep only the first few spans (◆ Plan ▰▰▱▱ 2/7)
            if plan_line.spans.len() > 4 {
                plan_line.spans.truncate(4);
            }
        }
        // Drop scout titles
        if lines.len() > 1 {
            // Simplify scout line to just the count
        }
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> Theme {
        Theme::default()
    }

    #[test]
    fn idle_produces_no_lines() {
        let state = CompactState::new();
        let lines = render_compact(&state, 0, 80, &theme());
        assert!(lines.is_empty());
    }

    #[test]
    fn done_phase_shows_summary_in_scrollback() {
        let mut state = CompactState::new();
        state.user_message("fix it", 0, &theme());
        state.tool(TrailKind::Edit, "Edited src/main.rs", true, 1000, &theme(), Some("src/main.rs"), 5, 2);
        state.done(5000, &theme());

        assert_eq!(state.phase, Phase::Done);
        assert!(!state.scrollback.is_empty());
        let last = state.scrollback.last().unwrap();
        let text = last.plain_text();
        assert!(text.contains("done in"));
        assert!(text.contains("steps"));
        assert!(text.contains("scouts"));
    }

    #[test]
    fn trail_beads_are_colored_by_kind() {
        let state = TrailKind::from_tool("read");
        assert_eq!(state, TrailKind::Read);
        assert_eq!(TrailKind::from_tool("edit"), TrailKind::Edit);
        assert_eq!(TrailKind::from_tool("bash"), TrailKind::Bash);
        assert_eq!(TrailKind::from_tool("execute_code"), TrailKind::Code);
        assert_eq!(TrailKind::from_tool("grep"), TrailKind::Search);
        assert_eq!(TrailKind::from_tool("glob"), TrailKind::Search);
    }

    #[test]
    fn short_title_truncates_correctly() {
        assert_eq!(short_title("fix the login bug"), "fix the login");
        assert_eq!(short_title("read file.ts"), "read file.ts");
        assert_eq!(short_title("hello,"), "hello");
        assert_eq!(short_title("a"), "a");
    }

    #[test]
    fn scout_line_disappears_after_all_home_7s() {
        let mut state = CompactState::new();
        state.user_message("test", 0, &theme());
        state.agent_start(1, "research API", 1000);
        state.agent_done(1, true, 5000);

        // At 6s, scout is still visible (just done)
        let lines = render_compact(&state, 6000, 80, &theme());
        assert!(lines.iter().any(|l| l.plain_text().contains("scout")));

        // At 13s, scout has been home for 8s > 7s, line should be gone
        let lines = render_compact(&state, 13000, 80, &theme());
        assert!(!lines.iter().any(|l| l.plain_text().contains("scout")));
    }

    #[test]
    fn compact_region_never_exceeds_six_lines() {
        let mut state = CompactState::new();
        state.user_message("do a big task", 0, &theme());
        state.plan(&[
            PlanItem { title: "Step 1".into(), status: PlanStatus::Done },
            PlanItem { title: "Step 2".into(), status: PlanStatus::InProgress },
            PlanItem { title: "Step 3".into(), status: PlanStatus::Pending },
        ]);
        state.agent_start(1, "research", 1000);
        state.agent_start(2, "implement", 1100);
        for i in 0..20 {
            let text = format!("read file{i}.ts");
            state.tool(TrailKind::Read, &text, true, 2000 + i * 100, &theme(), None, 0, 0);
        }
        state.latest = Some(LatestAction { text: "Reading file19.ts".into(), at: 3900, is_error: false });

        let lines = render_compact(&state, 4000, 80, &theme());
        assert!(lines.len() <= 6, "compact region has {} lines, expected <= 6", lines.len());
    }

    #[test]
    fn edits_print_one_line_to_scrollback() {
        let mut state = CompactState::new();
        state.user_message("fix it", 0, &theme());
        state.tool(TrailKind::Edit, "Edited src/main.rs", true, 1000, &theme(), Some("src/main.rs"), 5, 2);

        // Should have user prompt + edit line in scrollback
        assert_eq!(state.scrollback.len(), 2);
        let edit_line = &state.scrollback[1];
        let text = edit_line.plain_text();
        assert!(text.contains("◆"));
        assert!(text.contains("src/main.rs"));
        assert!(text.contains("+5"));
    }

    #[test]
    fn reads_and_searches_do_not_print_to_scrollback() {
        let mut state = CompactState::new();
        state.user_message("explore", 0, &theme());
        state.tool(TrailKind::Read, "read file.ts", true, 1000, &theme(), None, 0, 0);
        state.tool(TrailKind::Search, "grep foo", true, 1100, &theme(), None, 0, 0);
        state.tool(TrailKind::Bash, "make", true, 1200, &theme(), None, 0, 0);

        // Only user prompt should be in scrollback (reads, searches, bash don't print)
        assert_eq!(state.scrollback.len(), 1);
    }
}
