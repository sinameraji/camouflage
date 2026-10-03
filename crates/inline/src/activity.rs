//! Background activity: long-running jobs and agents that the host owns.
//!
//! The host reports items (`ActivityUpdate`, `ActivityLog`, `ActivityRemoved`,
//! `ActivitySnapshot`); this module keeps them, draws a footer badge and an
//! activity browser, and turns keys into requests (`ActivityStopRequested`,
//! `ActivityViewChanged`). It never starts or stops anything itself, and
//! output is shown as a followed log, not an interactive terminal.
//!
//! Nothing here animates: the badge is static, so a long-running job costs
//! no wakeups. While the browser is open the session ticks once a second to
//! keep elapsed times current.

use crate::blocks::format_elapsed;
use crate::session::Key;
use crate::style::{Line, Span, Style};
use crate::theme::Theme;
use crate::width::{ellipsize_middle, str_width, truncate};
use serde_json::Value;
use std::collections::VecDeque;

/// Output lines kept per item.
pub const LOG_LINES: usize = 2000;
/// Steps shown in the detail view before "+N more".
const MAX_STEPS: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Job,
    Agent,
}

impl Kind {
    fn parse(s: &str) -> Kind {
        if s.eq_ignore_ascii_case("agent") { Kind::Agent } else { Kind::Job }
    }
    pub fn noun(self) -> &'static str {
        match self {
            Kind::Job => "job",
            Kind::Agent => "agent",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Running,
    Waiting,
    NeedsAttention,
    Done,
    Failed,
    Stopped,
}

impl Status {
    fn parse(s: &str) -> Status {
        match s {
            "waiting" | "queued" | "pending" => Status::Waiting,
            "needs_attention" | "attention" | "blocked" => Status::NeedsAttention,
            "done" | "completed" | "succeeded" | "success" => Status::Done,
            "failed" | "error" => Status::Failed,
            "stopped" | "cancelled" | "canceled" | "killed" => Status::Stopped,
            _ => Status::Running,
        }
    }
    /// Still going (or waiting on someone): counted in the badge.
    pub fn active(self) -> bool {
        matches!(self, Status::Running | Status::Waiting | Status::NeedsAttention)
    }
    fn label(self) -> &'static str {
        match self {
            Status::Running => "running",
            Status::Waiting => "waiting",
            Status::NeedsAttention => "needs attention",
            Status::Done => "done",
            Status::Failed => "failed",
            Status::Stopped => "stopped",
        }
    }
    fn glyph(self, theme: &Theme) -> Span {
        match self {
            Status::Running => Span::styled("●", theme.accent()),
            Status::Waiting => Span::styled("○", theme.dim()),
            Status::NeedsAttention => Span::styled("!", theme.warn()),
            Status::Done => Span::styled("✓", theme.ok()),
            Status::Failed => Span::styled("✗", theme.err()),
            Status::Stopped => Span::styled("■", theme.dim()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub title: String,
    /// `pending`, `running`, `done` or `failed`.
    pub status: String,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    pub status: Status,
    pub stoppable: bool,
    pub summary: Option<String>,
    pub progress: Option<f32>,
    pub started_at_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
    pub steps: Vec<Step>,
    pub log: VecDeque<String>,
    /// The user asked to stop it; cleared when the host reports a new status.
    pub stop_requested: bool,
}

impl Item {
    fn elapsed(&self, now: i64) -> Option<u64> {
        let start = self.started_at_ms?;
        let end = if self.status.active() { now } else { self.updated_at_ms.unwrap_or(now) };
        Some((end - start).max(0) as u64)
    }
}

/// A lifecycle change worth a line in the transcript.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Started,
    Finished,
    Failed,
    NeedsAttention,
    Stopped,
}

#[derive(Default, Debug)]
pub struct Activity {
    items: Vec<Item>,
}

fn s(p: &Value, k: &str) -> Option<String> {
    p.get(k).and_then(Value::as_str).map(str::to_string)
}

impl Activity {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    /// Insert or replace an item. Output already received is kept; fields
    /// the payload leaves out keep their old values. Returns the lifecycle
    /// change, if any.
    pub fn upsert(&mut self, p: &Value, announce_new: bool) -> Option<(Transition, Item)> {
        let id = s(p, "id")?;
        let pos = self.items.iter().position(|i| i.id == id);
        let prev_status = pos.map(|i| self.items[i].status);
        let item = match pos {
            Some(i) => &mut self.items[i],
            None => {
                self.items.push(Item {
                    id: id.clone(),
                    kind: Kind::Job,
                    title: id.clone(),
                    status: Status::Running,
                    stoppable: false,
                    summary: None,
                    progress: None,
                    started_at_ms: None,
                    updated_at_ms: None,
                    steps: Vec::new(),
                    log: VecDeque::new(),
                    stop_requested: false,
                });
                self.items.last_mut().unwrap()
            }
        };
        if let Some(k) = s(p, "kind") {
            item.kind = Kind::parse(&k);
        }
        if let Some(t) = s(p, "title") {
            item.title = t;
        }
        if let Some(st) = s(p, "status") {
            let st = Status::parse(&st);
            if Some(st) != prev_status {
                item.stop_requested = false;
            }
            item.status = st;
        }
        if let Some(b) = p.get("stoppable").and_then(Value::as_bool) {
            item.stoppable = b;
        }
        if p.get("summary").is_some() {
            item.summary = s(p, "summary").filter(|x| !x.is_empty());
        }
        if p.get("progress").is_some() {
            item.progress = p.get("progress").and_then(Value::as_f64).map(|x| x.clamp(0.0, 1.0) as f32);
        }
        if let Some(t) = p.get("started_at_ms").and_then(Value::as_i64) {
            item.started_at_ms = Some(t);
        }
        if let Some(t) = p.get("updated_at_ms").and_then(Value::as_i64) {
            item.updated_at_ms = Some(t);
        }
        if let Some(steps) = p.get("steps").and_then(Value::as_array) {
            item.steps = steps
                .iter()
                .map(|st| Step { title: s(st, "title").unwrap_or_default(), status: s(st, "status").unwrap_or_else(|| "pending".into()) })
                .collect();
        }
        let now = item.status;
        let change = match (prev_status, now) {
            (None, st) if st.active() && announce_new => Some(Transition::Started),
            (Some(a), Status::Done) if a.active() => Some(Transition::Finished),
            (Some(a), Status::Failed) if a.active() => Some(Transition::Failed),
            (Some(a), Status::Stopped) if a.active() => Some(Transition::Stopped),
            (prev, Status::NeedsAttention) if prev != Some(Status::NeedsAttention) && (prev.is_some() || announce_new) => Some(Transition::NeedsAttention),
            _ => None,
        };
        change.map(|c| (c, item.clone()))
    }

    /// Append output. Unknown ids are ignored (the update may follow).
    pub fn log(&mut self, p: &Value) {
        let (Some(id), Some(chunk)) = (s(p, "id"), s(p, "chunk")) else { return };
        let Some(item) = self.items.iter_mut().find(|i| i.id == id) else { return };
        let chunk = chunk.replace("\r\n", "\n");
        let clean = |part: &str| crate::ansi::strip_ansi(part.rsplit('\r').next().unwrap_or(""));
        let mut parts = chunk.split('\n');
        let first = clean(parts.next().unwrap_or(""));
        match item.log.back_mut() {
            Some(last) => last.push_str(&first),
            None => item.log.push_back(first),
        }
        for part in parts {
            item.log.push_back(clean(part));
        }
        while item.log.len() > LOG_LINES {
            item.log.pop_front();
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.items.retain(|i| i.id != id);
    }

    /// Replace the whole list (the host's source of truth): items missing
    /// from it are dropped. Status changes of known items are announced;
    /// items seen for the first time are not (e.g. after a reconnect).
    pub fn snapshot(&mut self, p: &Value) -> Vec<(Transition, Item)> {
        let incoming: Vec<&Value> = p.get("items").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
        let keep: Vec<String> = incoming.iter().filter_map(|v| s(v, "id")).collect();
        self.items.retain(|i| keep.contains(&i.id));
        incoming.into_iter().filter_map(|v| self.upsert(v, false)).collect()
    }

    pub fn request_stop(&mut self, id: &str) {
        if let Some(i) = self.items.iter_mut().find(|i| i.id == id) {
            i.stop_requested = true;
        }
    }

    pub fn any_active(&self) -> bool {
        self.items.iter().any(|i| i.status.active())
    }

    /// Display order: agents, then jobs; within each, items needing
    /// attention, then other active ones, then finished ones.
    pub fn ordered(&self) -> Vec<&Item> {
        let mut v: Vec<&Item> = self.items.iter().collect();
        v.sort_by_key(|i| {
            let kind = if i.kind == Kind::Agent { 0 } else { 1 };
            let state = match i.status {
                Status::NeedsAttention => 0,
                s if s.active() => 1,
                _ => 2,
            };
            (kind, state)
        });
        v
    }

    /// The footer badge: `◆ 2 jobs · 1 agent · ctrl+b`. None without items.
    pub fn badge(&self, theme: &Theme) -> Option<Vec<Span>> {
        if self.items.is_empty() {
            return None;
        }
        let count = |k: Kind| self.items.iter().filter(|i| i.kind == k && i.status.active()).count();
        let (jobs, agents) = (count(Kind::Job), count(Kind::Agent));
        let attention = self.items.iter().filter(|i| i.status == Status::NeedsAttention).count();
        let plural = |n: usize, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });
        let mut parts = Vec::new();
        if jobs > 0 {
            parts.push(plural(jobs, "job"));
        }
        if agents > 0 {
            parts.push(plural(agents, "agent"));
        }
        let mut out = Vec::new();
        if parts.is_empty() {
            let done = self.items.len();
            out.push(Span::styled(format!("◇ {} finished", done), theme.dim()));
        } else if attention > 0 {
            out.push(Span::styled(format!("◆ {} · {attention} need{} attention", parts.join(" · "), if attention == 1 { "s" } else { "" }), theme.warn()));
        } else {
            out.push(Span::styled(format!("◆ {}", parts.join(" · ")), theme.accent()));
        }
        out.push(Span::styled(" · ctrl+b", theme.dim()));
        Some(out)
    }
}

/// What the activity browser shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum View {
    List { sel: usize },
    /// `scroll` counts lines up from the bottom of the log; `follow` keeps
    /// the newest output in view.
    Detail { id: String, back_sel: usize, scroll: usize, follow: bool },
    /// Asking before a stop request; `detail` returns to the item's details.
    Confirm { id: String, back_sel: usize, detail: bool },
}

/// What a key in the browser asks the session to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Tell the host the view changed: (`list` | `detail` | `closed`, id).
    ViewChanged(&'static str, Option<String>),
    Stop(String),
    Close,
}

impl View {
    /// Handle a key. `page` is the log viewport height for paging.
    pub fn key(&mut self, key: &Key, act: &Activity, page: usize) -> Vec<Outcome> {
        let items = act.ordered();
        match self.clone() {
            View::List { sel } => {
                let n = items.len();
                let sel = sel.min(n.saturating_sub(1));
                match key {
                    Key::Esc | Key::Ctrl('c') | Key::Ctrl('b') => return vec![Outcome::ViewChanged("closed", None), Outcome::Close],
                    Key::Up | Key::Ctrl('p') if n > 0 => *self = View::List { sel: (sel + n - 1) % n },
                    Key::Down | Key::Ctrl('n') | Key::Tab if n > 0 => *self = View::List { sel: (sel + 1) % n },
                    Key::Enter { .. } | Key::Right { .. } => {
                        if let Some(i) = items.get(sel) {
                            *self = View::Detail { id: i.id.clone(), back_sel: sel, scroll: 0, follow: true };
                            return vec![Outcome::ViewChanged("detail", Some(i.id.clone()))];
                        }
                    }
                    Key::Text(t) if t == "s" || t == "x" => {
                        if let Some(i) = items.get(sel).filter(|i| i.stoppable && i.status.active()) {
                            *self = View::Confirm { id: i.id.clone(), back_sel: sel, detail: false };
                        }
                    }
                    _ => {}
                }
            }
            View::Detail { id, back_sel, scroll, follow } => {
                let len = act.get(&id).map(|i| i.log.len()).unwrap_or(0);
                let max_scroll = len.saturating_sub(page.max(1));
                let set = |scroll: usize| View::Detail { id: id.clone(), back_sel, scroll: scroll.min(max_scroll), follow: scroll == 0 };
                let cur = if follow { 0 } else { scroll };
                match key {
                    Key::Esc | Key::Left { .. } | Key::Backspace { .. } => {
                        *self = View::List { sel: back_sel };
                        return vec![Outcome::ViewChanged("list", None)];
                    }
                    Key::Ctrl('c') | Key::Ctrl('b') => return vec![Outcome::ViewChanged("closed", None), Outcome::Close],
                    Key::Up | Key::Ctrl('p') => *self = set(cur + 1),
                    Key::Down | Key::Ctrl('n') => *self = set(cur.saturating_sub(1)),
                    Key::PageUp => *self = set(cur + page.max(1)),
                    Key::PageDown => *self = set(cur.saturating_sub(page.max(1))),
                    Key::Home => *self = set(max_scroll),
                    Key::End => *self = set(0),
                    Key::Text(t) if t == "s" || t == "x" => {
                        if act.get(&id).is_some_and(|i| i.stoppable && i.status.active()) {
                            *self = View::Confirm { id, back_sel, detail: true };
                        }
                    }
                    _ => {}
                }
            }
            View::Confirm { id, back_sel, detail } => {
                let back = if detail { View::Detail { id: id.clone(), back_sel, scroll: 0, follow: true } } else { View::List { sel: back_sel } };
                match key {
                    Key::Text(t) if t == "y" || t == "Y" => {
                        *self = back;
                        return vec![Outcome::Stop(id)];
                    }
                    Key::Enter { .. } => {
                        *self = back;
                        return vec![Outcome::Stop(id)];
                    }
                    Key::Esc | Key::Ctrl('c') => *self = back,
                    Key::Text(t) if t == "n" || t == "N" => *self = back,
                    _ => {}
                }
            }
        }
        Vec::new()
    }

    /// The item a view is about, if any.
    pub fn item_id(&self) -> Option<&str> {
        match self {
            View::List { .. } => None,
            View::Detail { id, .. } | View::Confirm { id, .. } => Some(id),
        }
    }
}

/// Rows the log viewport gets in a `height`-row terminal.
pub fn log_rows(height: usize, item: Option<&Item>) -> usize {
    let steps = item.map(|i| i.steps.len().min(MAX_STEPS + 1)).unwrap_or(0);
    let header = 4 + steps + usize::from(item.is_some_and(|i| i.summary.is_some())) + usize::from(item.is_some_and(|i| i.progress.is_some()));
    height.saturating_sub(header + 3).clamp(3, 40)
}

fn rule(title: &str, width: usize, theme: &Theme) -> Line {
    let label = format!("── {title} ");
    let rest = width.saturating_sub(str_width(&label));
    Line::styled(label, theme.dim()).with("─".repeat(rest), theme.dim())
}

fn hint(text: &str, theme: &Theme) -> Line {
    Line::styled(format!("  {text}"), theme.dim())
}

/// Draw the browser in `width` columns using at most `height` rows.
pub fn render(act: &Activity, view: &View, theme: &Theme, width: usize, height: usize, now: i64) -> Vec<Line> {
    let width = width.max(20);
    let (base, confirm) = match view {
        View::Confirm { id, back_sel, detail } => {
            let base = if *detail { View::Detail { id: id.clone(), back_sel: *back_sel, scroll: 0, follow: true } } else { View::List { sel: *back_sel } };
            (base, act.get(id))
        }
        v => (v.clone(), None),
    };
    let mut out = match &base {
        View::Detail { id, scroll, follow, .. } => match act.get(id) {
            Some(item) => render_detail(item, if *follow { 0 } else { *scroll }, theme, width, height, now),
            None => render_list(act, 0, theme, width, height, now),
        },
        View::List { sel } => render_list(act, *sel, theme, width, height, now),
        View::Confirm { .. } => unreachable!(),
    };
    if let Some(item) = confirm {
        out.pop();
        let q = format!("  Stop {} {}? ", item.kind.noun(), item.title);
        out.push(Line::styled(truncate(&q, width.saturating_sub(24)), theme.warn()).with("y to stop · n to keep it", theme.dim()));
    }
    out
}

fn render_list(act: &Activity, sel: usize, theme: &Theme, width: usize, height: usize, now: i64) -> Vec<Line> {
    let items = act.ordered();
    let mut out = vec![rule("Background activity", width, theme)];
    if items.is_empty() {
        out.push(Line::styled("  Nothing running.", theme.dim()));
        out.push(hint("esc close", theme));
        return out;
    }
    let sel = sel.min(items.len() - 1);
    // Rows: one per item plus a header per kind; keep the selection in view.
    let mut rows: Vec<(Option<usize>, Line)> = Vec::new();
    let mut last_kind = None;
    let title_w = items.iter().map(|i| str_width(&i.title)).max().unwrap_or(0).min(width * 2 / 5).max(8);
    for (n, item) in items.iter().enumerate() {
        if last_kind != Some(item.kind) {
            last_kind = Some(item.kind);
            let label = if item.kind == Kind::Agent { "Agents" } else { "Jobs" };
            rows.push((None, Line::styled(format!("  {label}"), theme.strong())));
        }
        let mark = if n == sel { Span::styled("› ", theme.accent()) } else { Span::raw("  ") };
        let mut line = Line::from_spans(vec![mark, item.status.glyph(theme), Span::raw(" ")]);
        let title = ellipsize_middle(&item.title, title_w);
        let pad = title_w.saturating_sub(str_width(&title));
        let title_style = if item.status.active() { Style::new() } else { theme.dim() };
        line.push(title, if n == sel { title_style.bold() } else { title_style });
        line.push(" ".repeat(pad + 2), Style::new());
        let mut info = vec![if item.stop_requested { "stopping…".to_string() } else { item.status.label().to_string() }];
        if let Some(e) = item.elapsed(now) {
            info.push(format_elapsed(e));
        }
        if let Some(sum) = &item.summary {
            info.push(sum.clone());
        }
        let used = 4 + title_w + 2;
        let style = if item.status == Status::NeedsAttention { theme.warn() } else if item.status == Status::Failed { theme.err() } else { theme.dim() };
        line.push(truncate(&info.join(" · "), width.saturating_sub(used)), style);
        rows.push((Some(n), line));
    }
    let room = height.saturating_sub(3).max(3);
    let sel_row = rows.iter().position(|(n, _)| *n == Some(sel)).unwrap_or(0);
    let start = if rows.len() <= room { 0 } else { sel_row.saturating_sub(room / 2).min(rows.len() - room) };
    out.extend(rows.into_iter().skip(start).take(room).map(|(_, l)| l));
    let stop = items.get(sel).is_some_and(|i| i.stoppable && i.status.active());
    out.push(hint(&format!("↑↓ select · enter details{} · esc close", if stop { " · s stop" } else { "" }), theme));
    out
}

fn render_detail(item: &Item, scroll: usize, theme: &Theme, width: usize, height: usize, now: i64) -> Vec<Line> {
    let mut out = vec![rule(&format!("{} · {}", item.kind.noun(), ellipsize_middle(&item.title, width.saturating_sub(16))), width, theme)];
    let mut status = Line::from_spans(vec![Span::raw("  "), item.status.glyph(theme), Span::raw(" ")]);
    let label = if item.stop_requested { "stop requested" } else { item.status.label() };
    let style = match item.status {
        Status::NeedsAttention => theme.warn(),
        Status::Failed => theme.err(),
        _ => Style::new(),
    };
    status.push(label, style);
    if let Some(e) = item.elapsed(now) {
        status.push(format!(" · {}", format_elapsed(e)), theme.dim());
    }
    out.push(status);
    if let Some(sum) = &item.summary {
        out.push(Line::styled(format!("  {}", truncate(sum, width.saturating_sub(2))), theme.dim()));
    }
    if let Some(p) = item.progress {
        let bar_w = width.saturating_sub(10).min(30);
        let filled = ((p * bar_w as f32).round() as usize).min(bar_w);
        out.push(Line::raw("  ").with("█".repeat(filled), theme.accent()).with("░".repeat(bar_w - filled), theme.dim()).with(format!(" {:.0}%", p * 100.0), theme.dim()));
    }
    for st in item.steps.iter().take(MAX_STEPS) {
        let (g, gs, ts) = match st.status.as_str() {
            "done" | "completed" => ("✓", theme.ok(), theme.dim()),
            "running" | "in_progress" => ("›", theme.accent(), Style::new().bold()),
            "failed" | "error" => ("✗", theme.err(), theme.err()),
            _ => ("·", theme.dim(), theme.dim()),
        };
        out.push(Line::styled(format!("  {g} "), gs).with(truncate(&st.title, width.saturating_sub(4)), ts));
    }
    if item.steps.len() > MAX_STEPS {
        out.push(Line::styled(format!("    +{} more", item.steps.len() - MAX_STEPS), theme.dim()));
    }

    let rows = log_rows(height, Some(item));
    let len = item.log.len();
    let scroll = scroll.min(len.saturating_sub(rows));
    let what = if item.kind == Kind::Agent { "recent output" } else { "output" };
    let state = if scroll == 0 { "following".to_string() } else { format!("↑ {scroll} lines · end to follow") };
    out.push(rule(&format!("{what} · {state}"), width, theme));
    if len == 0 {
        out.push(Line::styled("  No output yet.", theme.dim()));
    } else {
        let end = len - scroll;
        let start = end.saturating_sub(rows);
        for l in item.log.range(start..end) {
            out.push(Line::raw("  ").with(truncate(l, width.saturating_sub(2)), Style::new()));
        }
    }
    let stop = item.stoppable && item.status.active() && !item.stop_requested;
    out.push(hint(&format!("esc back · ↑↓ pgup pgdn scroll · end follow{}", if stop { " · s stop" } else { "" }), theme));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn act() -> Activity {
        let mut a = Activity::default();
        a.upsert(&json!({"id": "j1", "kind": "job", "title": "npm run dev", "status": "running", "stoppable": true, "started_at_ms": 0}), true);
        a.upsert(&json!({"id": "a1", "kind": "agent", "title": "research auth", "status": "running", "summary": "reading src/auth", "steps": [{"title": "Read code", "status": "done"}, {"title": "Write notes", "status": "running"}]}), true);
        a
    }

    fn text(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|l| l.plain_text()).collect()
    }

    #[test]
    fn badge_counts_active_items_and_flags_attention() {
        let mut a = act();
        let badge = |a: &Activity| a.badge(&Theme::default()).unwrap().iter().map(|s| s.text.clone()).collect::<String>();
        assert_eq!(badge(&a), "◆ 1 job · 1 agent · ctrl+b");
        a.upsert(&json!({"id": "a1", "status": "needs_attention"}), true);
        assert_eq!(badge(&a), "◆ 1 job · 1 agent · 1 needs attention · ctrl+b");
        a.upsert(&json!({"id": "a1", "status": "done"}), true);
        a.upsert(&json!({"id": "j1", "status": "failed"}), true);
        assert_eq!(badge(&a), "◇ 2 finished · ctrl+b");
        assert!(Activity::default().badge(&Theme::default()).is_none());
    }

    #[test]
    fn transitions_are_reported_once() {
        let mut a = Activity::default();
        assert_eq!(a.upsert(&json!({"id": "j", "title": "t", "status": "running"}), true).map(|t| t.0), Some(Transition::Started));
        assert_eq!(a.upsert(&json!({"id": "j", "summary": "half"}), true).map(|t| t.0), None);
        assert_eq!(a.upsert(&json!({"id": "j", "status": "needs_attention"}), true).map(|t| t.0), Some(Transition::NeedsAttention));
        assert_eq!(a.upsert(&json!({"id": "j", "status": "needs_attention"}), true).map(|t| t.0), None);
        assert_eq!(a.upsert(&json!({"id": "j", "status": "failed"}), true).map(|t| t.0), Some(Transition::Failed));
        assert_eq!(a.upsert(&json!({"id": "j", "status": "failed"}), true).map(|t| t.0), None);
    }

    #[test]
    fn snapshot_drops_stale_items_without_announcing_new_ones() {
        let mut a = act();
        let changes = a.snapshot(&json!({"items": [{"id": "j1", "status": "done"}, {"id": "n", "title": "new", "status": "running"}]}));
        assert_eq!(changes.iter().map(|c| c.0).collect::<Vec<_>>(), vec![Transition::Finished]);
        assert!(a.get("a1").is_none(), "missing from the snapshot means gone");
        assert!(a.get("n").is_some());
    }

    #[test]
    fn logs_split_lines_strip_ansi_and_stay_bounded() {
        let mut a = act();
        a.log(&json!({"id": "j1", "chunk": "\x1b[32mready\x1b[0m on :3000\nGET /"}));
        a.log(&json!({"id": "j1", "chunk": " 200\n"}));
        assert_eq!(a.get("j1").unwrap().log.iter().cloned().collect::<Vec<_>>(), vec!["ready on :3000", "GET / 200", ""]);
        a.log(&json!({"id": "j1", "chunk": "x\n".repeat(LOG_LINES + 50)}));
        assert_eq!(a.get("j1").unwrap().log.len(), LOG_LINES);
        a.log(&json!({"id": "nope", "chunk": "ignored"}));
    }

    #[test]
    fn list_groups_agents_then_jobs_and_moves_selection() {
        let a = act();
        let lines = text(&render(&a, &View::List { sel: 0 }, &Theme::default(), 70, 20, 5_000));
        assert_eq!(lines[1].trim(), "Agents");
        assert!(lines[2].starts_with("› ● research auth"), "{lines:?}");
        assert_eq!(lines[3].trim(), "Jobs");
        assert!(lines[4].contains("npm run dev") && lines[4].contains("running · 5s"), "{lines:?}");
        let mut v = View::List { sel: 0 };
        v.key(&Key::Down, &a, 10);
        assert_eq!(v, View::List { sel: 1 });
        assert!(text(&render(&a, &v, &Theme::default(), 70, 20, 5_000)).last().unwrap().contains("s stop"), "the job is stoppable");
    }

    #[test]
    fn detail_follows_output_and_scrolls_back() {
        let mut a = act();
        for i in 0..100 {
            a.log(&json!({"id": "j1", "chunk": format!("line {i}\n")}));
        }
        let mut v = View::List { sel: 1 };
        assert_eq!(v.key(&Key::Enter { shift: false, alt: false }, &a, 10), vec![Outcome::ViewChanged("detail", Some("j1".into()))]);
        let shown = text(&render(&a, &v, &Theme::default(), 60, 20, 0));
        assert!(shown.iter().any(|l| l.contains("following")));
        assert!(shown.iter().any(|l| l.trim() == "line 99"));
        v.key(&Key::PageUp, &a, 10);
        let shown = text(&render(&a, &v, &Theme::default(), 60, 20, 0));
        assert!(shown.iter().any(|l| l.contains("↑ 10 lines")), "{shown:?}");
        assert!(!shown.iter().any(|l| l.trim() == "line 99"));
        v.key(&Key::End, &a, 10);
        assert!(text(&render(&a, &v, &Theme::default(), 60, 20, 0)).iter().any(|l| l.contains("following")));
        assert_eq!(v.key(&Key::Esc, &a, 10), vec![Outcome::ViewChanged("list", None)]);
        assert_eq!(v, View::List { sel: 1 });
    }

    #[test]
    fn stop_asks_first_and_only_for_stoppable_active_items() {
        let a = act();
        let mut v = View::List { sel: 0 };
        v.key(&Key::Text("s".into()), &a, 10);
        assert_eq!(v, View::List { sel: 0 }, "the agent isn't stoppable");
        let mut v = View::List { sel: 1 };
        v.key(&Key::Text("s".into()), &a, 10);
        assert!(text(&render(&a, &v, &Theme::default(), 70, 20, 0)).last().unwrap().contains("Stop job npm run dev?"));
        assert_eq!(v.key(&Key::Text("n".into()), &a, 10), vec![]);
        assert_eq!(v, View::List { sel: 1 });
        v.key(&Key::Text("s".into()), &a, 10);
        assert_eq!(v.key(&Key::Text("y".into()), &a, 10), vec![Outcome::Stop("j1".into())]);
    }

    #[test]
    fn small_terminals_keep_the_selection_visible() {
        let mut a = Activity::default();
        for i in 0..30 {
            a.upsert(&json!({"id": format!("j{i}"), "title": format!("job {i}"), "status": "running"}), false);
        }
        let lines = text(&render(&a, &View::List { sel: 25 }, &Theme::default(), 40, 10, 0));
        assert!(lines.len() <= 10, "{}", lines.len());
        assert!(lines.iter().any(|l| l.starts_with("› ● job 25")), "{lines:?}");
    }
}
