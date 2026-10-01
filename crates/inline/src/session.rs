//! The inline session: turns protocol events and key presses into printed
//! history plus one live frame.
//!
//! Finished blocks are printed and then **dropped**: the terminal's
//! scrollback holds them (and SQLite holds the events), so memory and render
//! cost stay flat for the whole session. Only unfinished blocks and the
//! chrome are kept and redrawn.
//!
//! The session never sleeps on a timer of its own. [`Session::next_wakeup`]
//! says when the next frame is needed (a spinner tick, a message expiring),
//! or `None` when idle, so an idle renderer does no work at all.

use crate::ansi::parse_ansi;
use crate::blocks::{
    gap_between, render_block, Block, NoticeKind, PlanItem, PlanStatus, RenderCtx, ToolBlock, ToolStatus,
};
use crate::chrome::{self, InputMode, PickItem, PromptView};
use crate::diff::{count_changes, diff_texts, parse_unified};
use crate::editor::{EditKey, EditOutcome, Editor};
use crate::form::{FormOutcome, FormState};
use crate::style::{Color, Line, Span, Style};
use crate::term::LiveFrame;
use crate::theme::Theme;
use camouflage_protocol::{Event, EventType};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

/// Spinner frame interval.
pub const FRAME_MS: i64 = 80;
const CTRL_C_WINDOW_MS: i64 = 1600;
const TOOL_TAIL_LINES: usize = 400;

/// A key press, independent of the terminal library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Text(String),
    Paste(String),
    Enter { shift: bool, alt: bool },
    Esc,
    Tab,
    BackTab,
    Up,
    Down,
    Left { word: bool },
    Right { word: bool },
    Home,
    End,
    Backspace { word: bool },
    Delete { word: bool },
    PageUp,
    PageDown,
    /// Ctrl + a letter, lowercase.
    Ctrl(char),
}

/// An event for the host.
#[derive(Clone, Debug, PartialEq)]
pub struct Outbound {
    pub event_type: EventType,
    pub payload: Value,
}

struct Entry {
    id: u64,
    block: Block,
    done: bool,
}

enum PromptKind {
    Permission { request_id: String },
    Select { id: String, values: Vec<String>, allow_cancel: bool, filterable: bool },
    Confirm { id: String, allow_cancel: bool },
}

struct Prompt {
    kind: PromptKind,
    title: String,
    subtitle: Option<String>,
    diff: Option<(String, Vec<crate::blocks::DiffLine>)>,
    /// `diff` rendered at `.0` width, reused across frames.
    diff_lines: Option<(usize, Vec<Line>)>,
    question: Option<String>,
    options: Vec<String>,
    hints: Vec<Option<String>>,
    /// Rich select lists: per-option group header, aligned columns, on/off
    /// state, and extra search text. Empty for other prompts.
    sections: Vec<Option<String>>,
    columns: Vec<Vec<String>>,
    states: Vec<Option<bool>>,
    keywords: Vec<String>,
    current: Option<usize>,
    selected: usize,
    filter: String,
}

pub struct Session {
    pub theme: Theme,
    app_name: String,
    width: usize,
    height: usize,
    entries: Vec<Entry>,
    next_id: u64,
    /// Whether the last printed block was a tool/notice row (for spacing).
    last_printed: Option<Block>,
    streams: HashMap<String, u64>,
    /// Text of a streaming reply already printed (split off paragraph by
    /// paragraph), by entry id. A final `text` on completion replaces only
    /// what comes after it, so those paragraphs aren't printed twice.
    printed_prefix: HashMap<u64, String>,
    tools: HashMap<String, u64>,
    plan: Option<Vec<PlanItem>>,
    segments: BTreeMap<String, String>,
    editor: Editor,
    mode: InputMode,
    commands: Vec<(String, String, Option<String>)>,
    mentions: Vec<String>,
    /// Entries for a path mention (`@../`), as (directory, tokens) from the
    /// host's answer to a MentionQuery.
    path_mentions: Option<(String, Vec<String>)>,
    /// The directory last asked for, so each one is queried once.
    mention_query_sent: Option<String>,
    /// Prompts this renderer printed on submit. A host that also sends
    /// UserMessageCreated for them is not printed twice, even after the
    /// echo itself has been printed and dropped.
    echoes: std::collections::VecDeque<String>,
    pick_sel: usize,
    pick_dismissed: Option<String>,
    prompt: Option<Prompt>,
    form: Option<FormState>,
    wizard: Option<Wizard>,
    foot: Option<(String, Style, i64)>,
    ctrl_c_at: Option<i64>,
    phase_busy: bool,
    busy_since: Option<i64>,
    streamed_chars: u64,
    streamed_at_busy: u64,
    tasks: BTreeMap<String, String>,
    expanded: bool,
    /// Ctrl+R: draw the model's reasoning above replies.
    show_reasoning: bool,
    /// Open reasoning block per stream id.
    reasoning: HashMap<String, u64>,
    outbound: Vec<Outbound>,
    /// The host asked to clear the transcript; the app clears the screen.
    pub clear_screen: bool,
    /// The user asked to exit.
    pub exit: bool,
}

impl Session {
    pub fn new(app_name: impl Into<String>, theme: Theme, width: usize, height: usize) -> Self {
        Session {
            theme,
            app_name: app_name.into(),
            width: width.max(20),
            height: height.max(6),
            entries: Vec::new(),
            next_id: 1,
            last_printed: None,
            streams: HashMap::new(),
            printed_prefix: HashMap::new(),
            tools: HashMap::new(),
            plan: None,
            segments: BTreeMap::new(),
            editor: Editor::new(),
            mode: InputMode::Default,
            commands: Vec::new(),
            mentions: Vec::new(),
            path_mentions: None,
            mention_query_sent: None,
            echoes: std::collections::VecDeque::new(),
            pick_sel: 0,
            pick_dismissed: None,
            prompt: None,
            form: None,
            wizard: None,
            foot: None,
            ctrl_c_at: None,
            phase_busy: false,
            busy_since: None,
            streamed_chars: 0,
            streamed_at_busy: 0,
            tasks: BTreeMap::new(),
            expanded: false,
            show_reasoning: false,
            reasoning: HashMap::new(),
            outbound: Vec::new(),
            clear_screen: false,
            exit: false,
        }
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width.max(20);
        self.height = height.max(6);
        if let Some(p) = self.prompt.as_mut() {
            if let Some((path, d)) = &p.diff {
                p.diff_lines = Some((self.width, chrome::diff_preview(&self.theme, path, d, self.width)));
            }
        }
    }

    pub fn set_history(&mut self, history: Vec<String>) {
        self.editor.set_history(history);
    }

    /// Events for the host produced since the last call.
    pub fn take_outbound(&mut self) -> Vec<Outbound> {
        std::mem::take(&mut self.outbound)
    }

    /// Number of blocks still held in memory (unprinted).
    pub fn held_blocks(&self) -> usize {
        self.entries.len()
    }

    // ----- events from the host -------------------------------------------

    pub fn apply(&mut self, ev: &Event, now: i64) {
        let p = &ev.payload;
        let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        match ev.event_type {
            EventType::SessionStarted => {
                if p.get("synthetic").and_then(Value::as_bool) == Some(true) {
                    return;
                }
                if let Some(name) = p.get("assistant_label").and_then(Value::as_str) {
                    self.app_name = name.to_string();
                }
                if let Some(c) = p.get("accent").and_then(Value::as_str).and_then(parse_color) {
                    self.theme.accent = c;
                }
                let title = p.get("title").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| self.app_name.clone());
                let detail: Vec<String> = p
                    .get("detail")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                if !title.is_empty() || !detail.is_empty() {
                    self.push_done(Block::Welcome { title, detail });
                }
            }
            EventType::Splash => {
                let mut lines = parse_ansi(&s("text"));
                while lines.first().map(|l| l.plain_text().trim().is_empty()).unwrap_or(false) {
                    lines.remove(0);
                }
                while lines.last().map(|l| l.plain_text().trim().is_empty()).unwrap_or(false) {
                    lines.pop();
                }
                if !lines.is_empty() {
                    self.push_done(Block::Custom { lines });
                }
            }
            EventType::UserMessageCreated => {
                let text = s("text");
                if let Some(i) = self.echoes.iter().position(|e| e.trim() == text.trim()) {
                    self.echoes.drain(..=i);
                    return;
                }
                self.push_done(Block::User { text });
            }
            EventType::AssistantStreamStarted => {
                let id = self.push(Block::Assistant { markdown: String::new() }, false);
                self.streams.insert(s("stream_id"), id);
                self.became_busy(now);
            }
            EventType::AssistantReasoningDelta => {
                let token = s("token");
                let sid = s("stream_id");
                if let Some(id) = self.reasoning.get(&sid).copied() {
                    if let Some(Block::Reasoning { text }) = self.block_mut(id) {
                        text.push_str(&token);
                    }
                    return;
                }
                // Reasoning goes above its reply. Once the reply has text
                // (and may be partly printed) it's too late to show more.
                let reply = self.streams.get(&sid).copied();
                let pos = match reply {
                    Some(rid) => match self.entries.iter().position(|e| e.id == rid) {
                        Some(i) if matches!(&self.entries[i].block, Block::Assistant { markdown } if markdown.is_empty()) => i,
                        _ => return,
                    },
                    None => self.entries.len(),
                };
                let id = self.next_id;
                self.next_id += 1;
                self.entries.insert(pos, Entry { id, block: Block::Reasoning { text: token }, done: false });
                self.reasoning.insert(sid, id);
                self.became_busy(now);
            }
            EventType::AssistantTokenDelta => {
                let token = s("token");
                if let Some(rid) = self.reasoning.remove(&s("stream_id")) {
                    self.finish(rid);
                }
                self.streamed_chars += token.chars().count() as u64;
                let sid = s("stream_id");
                let id = match self.streams.get(&sid) {
                    Some(id) => *id,
                    None => {
                        // Delta without a start: open the stream implicitly.
                        let id = self.push(Block::Assistant { markdown: String::new() }, false);
                        self.streams.insert(sid, id);
                        id
                    }
                };
                if let Some(Block::Assistant { markdown }) = self.block_mut(id) {
                    markdown.push_str(&token);
                }
            }
            EventType::AssistantMessageCompleted => {
                if let Some(rid) = self.reasoning.remove(&s("stream_id")) {
                    self.finish(rid);
                }
                if let Some(id) = self.streams.remove(&s("stream_id")) {
                    let printed = self.printed_prefix.remove(&id).unwrap_or_default();
                    if let Some(text) = p.get("text").and_then(Value::as_str) {
                        // Paragraphs already in scrollback can't change; if the
                        // final text rewrote them, keep what was streamed.
                        if let Some(rest) = text.strip_prefix(printed.as_str()) {
                            let rest = rest.trim_start_matches('\n').to_string();
                            if let Some(Block::Assistant { markdown }) = self.block_mut(id) {
                                *markdown = rest;
                            }
                        }
                    }
                    self.finish(id);
                }
            }
            EventType::ToolExecutionStarted => {
                let mut t = ToolBlock::new(s("tool"), s("command"));
                t.started_at_ms = p.get("started_at_ms").and_then(Value::as_i64).or(Some(now));
                let id = self.push(Block::Tool(t), false);
                self.tools.insert(s("tool_id"), id);
                self.became_busy(now);
            }
            EventType::ToolExecutionStdout | EventType::ToolExecutionStderr => {
                let chunk = s("chunk");
                if let Some(id) = self.tools.get(&s("tool_id")).copied() {
                    if let Some(Block::Tool(t)) = self.block_mut(id) {
                        append_output(&mut t.output, &chunk);
                    }
                }
            }
            EventType::ToolExecutionFinished => {
                let Some(id) = self.tools.remove(&s("tool_id")) else { return };
                let exit = p.get("exit_code").and_then(Value::as_i64);
                let status = match p.get("status").and_then(Value::as_str) {
                    Some("error" | "err") => ToolStatus::Error,
                    Some("cancelled" | "canceled") => ToolStatus::Stopped,
                    Some("rejected") => ToolStatus::Declined,
                    Some(_) => ToolStatus::Ok,
                    None if exit.unwrap_or(0) != 0 => ToolStatus::Error,
                    None => ToolStatus::Ok,
                };
                if let Some(Block::Tool(t)) = self.block_mut(id) {
                    t.status = status;
                    t.elapsed_ms = t.started_at_ms.map(|st| (now - st).max(0) as u64);
                    if let Some(sum) = p.get("summary").and_then(Value::as_str) {
                        t.summary = Some(sum.to_string());
                    } else if status == ToolStatus::Error {
                        t.summary = Some(match exit {
                            Some(code) => format!("Exit code {code}"),
                            None => "Failed".to_string(),
                        });
                    }
                    if let Some(out) = p.get("output").and_then(Value::as_str) {
                        t.output.clear();
                        append_output(&mut t.output, out);
                    }
                    t.preview = p.get("preview").and_then(Value::as_u64).map(|n| n as usize).unwrap_or(if status == ToolStatus::Error { 12 } else { 0 });
                    t.output_lang = p.get("output_lang").and_then(Value::as_str).map(str::to_string);
                    if let Some(d) = p.get("diff") {
                        t.diff = parse_diff_payload(d);
                        if t.summary.is_none() {
                            if let Some((path, lines)) = &t.diff {
                                let (a, r) = count_changes(lines);
                                t.summary = Some(format!("Updated {path} · +{a} −{r}"));
                            }
                        }
                    }
                }
                self.finish(id);
            }
            EventType::PatchProposed => {
                let path = s("path");
                let lines = p.get("diff").and_then(Value::as_str).map(parse_unified).unwrap_or_default();
                let (a, r) = if lines.is_empty() {
                    (p.get("added").and_then(Value::as_u64).unwrap_or(0) as usize, p.get("removed").and_then(Value::as_u64).unwrap_or(0) as usize)
                } else {
                    count_changes(&lines)
                };
                let mut t = ToolBlock::new("Edit", path.clone());
                t.status = ToolStatus::Ok;
                t.summary = Some(format!("Proposed · +{a} −{r}"));
                if !lines.is_empty() {
                    t.diff = Some((path, lines));
                }
                self.push_done(Block::Tool(t));
            }
            EventType::PermissionRequested => {
                let tool = s("tool");
                let action = s("action");
                let detail = s("detail");
                let diff = p.get("diff").and_then(parse_diff_payload);
                let title = if action.is_empty() { format!("Allow {tool}?") } else { capitalize(&action) };
                let subtitle = if !detail.is_empty() { Some(detail) } else { diff.as_ref().map(|(path, _)| path.clone()) }
                    .filter(|sub| !title.contains(sub.as_str()));
                let app = self.app_name.clone();
                self.prompt = Some(Prompt {
                    kind: PromptKind::Permission { request_id: s("request_id") },
                    title,
                    subtitle,
                    diff_lines: diff.as_ref().map(|(path, d)| (self.width, chrome::diff_preview(&self.theme, path, d, self.width))),
                    diff,
                    question: Some(format!("Do you want to allow this {}?", if tool.is_empty() { "action" } else { &tool })),
                    options: vec![
                        "Yes".into(),
                        "Yes, and don't ask again this session".into(),
                        format!("No, and tell {} what to do differently", if app.is_empty() { "the agent" } else { &app }),
                    ],
                    hints: Vec::new(),
                    sections: Vec::new(),
                    columns: Vec::new(),
                    states: Vec::new(),
                    keywords: Vec::new(),
                    current: None,
                    selected: 0,
                    filter: String::new(),
                });
            }
            EventType::ShowSelectList => {
                let opts: Vec<&Value> = p.get("options").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
                let field = |o: &Value, k: &str| o.get(k).and_then(Value::as_str).map(str::to_string);
                let values: Vec<String> = opts.iter().map(|o| field(o, "value").unwrap_or_default()).collect();
                let default = p.get("default").and_then(Value::as_str);
                let selected = default.and_then(|d| values.iter().position(|v| v == d)).unwrap_or(0);
                self.prompt = Some(Prompt {
                    kind: PromptKind::Select {
                        id: s("id"),
                        values,
                        allow_cancel: p.get("allow_cancel").and_then(Value::as_bool).unwrap_or(true),
                        filterable: p.get("allow_filter").and_then(Value::as_bool).unwrap_or(true),
                    },
                    title: s("prompt"),
                    subtitle: p.get("subtitle").and_then(Value::as_str).map(str::to_string),
                    diff: None,
                    diff_lines: None,
                    question: None,
                    options: opts.iter().map(|o| field(o, "label").unwrap_or_default()).collect(),
                    hints: opts.iter().map(|o| field(o, "description")).collect(),
                    sections: opts.iter().map(|o| field(o, "section")).collect(),
                    columns: opts
                        .iter()
                        .map(|o| {
                            o.get("columns")
                                .and_then(Value::as_array)
                                .map(|c| c.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect())
                                .unwrap_or_default()
                        })
                        .collect(),
                    states: opts.iter().map(|o| o.get("state").and_then(Value::as_str).map(|v| v == "on")).collect(),
                    keywords: opts.iter().map(|o| field(o, "keywords").unwrap_or_default()).collect(),
                    current: default.map(|_| selected),
                    selected,
                    filter: String::new(),
                });
            }
            EventType::ShowConfirm => {
                let yes = p.get("yes_label").and_then(Value::as_str).unwrap_or("Yes").to_string();
                let no = p.get("no_label").and_then(Value::as_str).unwrap_or("No").to_string();
                let selected = if p.get("default").and_then(Value::as_str) == Some("no") { 1 } else { 0 };
                self.prompt = Some(Prompt {
                    kind: PromptKind::Confirm { id: s("id"), allow_cancel: p.get("allow_cancel").and_then(Value::as_bool).unwrap_or(true) },
                    title: s("prompt"),
                    subtitle: None,
                    diff: None,
                    diff_lines: None,
                    question: None,
                    options: vec![yes, no],
                    hints: Vec::new(),
                    sections: Vec::new(),
                    columns: Vec::new(),
                    states: Vec::new(),
                    keywords: Vec::new(),
                    current: None,
                    selected,
                    filter: String::new(),
                });
            }
            EventType::ShowTable => self.push_done(Block::Assistant { markdown: table_markdown(p) }),
            EventType::ShowKeyValueView => self.push_done(Block::Assistant { markdown: kv_markdown(p) }),
            EventType::ShowForm => {
                self.form = Some(FormState::from_payload(p));
            }
            EventType::ShowWizard => {
                let steps = p.get("steps").and_then(Value::as_array).cloned().unwrap_or_default();
                if steps.is_empty() {
                    self.outbound.push(Outbound { event_type: EventType::WizardCompleted, payload: json!({ "id": s("id"), "results": {} }) });
                    return;
                }
                self.wizard = Some(Wizard {
                    id: s("id"),
                    title: p.get("title").and_then(Value::as_str).unwrap_or("").to_string(),
                    allow_cancel: p.get("allow_cancel").and_then(Value::as_bool).unwrap_or(true),
                    steps,
                    at: 0,
                    results: serde_json::Map::new(),
                });
                self.show_wizard_step(now);
            }
            EventType::RuntimeError => {
                let mut text = s("message");
                if let Some(label) = p.get("cta").and_then(|c| c.get("label")).and_then(Value::as_str) {
                    text.push_str(&format!(" · {label}"));
                }
                let kind = match p.get("severity").and_then(Value::as_str) {
                    Some("info") => NoticeKind::Info,
                    Some("warn") => NoticeKind::Warn,
                    _ => NoticeKind::Error,
                };
                self.push_done(Block::Notice { kind, text });
            }
            EventType::SessionCompacted => {
                self.push_done(Block::Notice { kind: NoticeKind::Info, text: "Conversation compacted".into() });
            }
            EventType::ShowToast => {
                let style = match p.get("kind").and_then(Value::as_str) {
                    Some("warn") => self.theme.warn(),
                    Some("error") => self.theme.err(),
                    Some("success") => self.theme.ok(),
                    _ => self.theme.accent(),
                };
                let ttl = p.get("ttl_ms").and_then(Value::as_i64).unwrap_or(3500).clamp(500, 15_000);
                self.foot = Some((s("text"), style, now + ttl));
            }
            EventType::StatusUpdate => {
                if let Some(segs) = p.get("segments").and_then(Value::as_object) {
                    for (k, v) in segs {
                        let v = v.as_str().unwrap_or("").to_string();
                        if v.is_empty() {
                            self.segments.remove(k);
                        } else {
                            self.segments.insert(k.clone(), v);
                        }
                    }
                }
                self.mode = match self.segments.get("mode").map(String::as_str) {
                    Some("plan") => InputMode::Plan,
                    Some("auto" | "accept" | "accept-edits" | "accept_edits") => InputMode::AcceptEdits,
                    _ => InputMode::Default,
                };
                let busy = matches!(
                    self.segments.get("phase").map(String::as_str),
                    Some("thinking" | "streaming" | "tool" | "running" | "working")
                );
                if busy {
                    self.phase_busy = true;
                    self.became_busy(now);
                } else if self.phase_busy {
                    self.phase_busy = false;
                    self.settle(now, false);
                }
            }
            EventType::TodoListUpdate => {
                let items: Vec<PlanItem> = p
                    .get("todos")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .map(|t| PlanItem {
                                title: t.get("title").and_then(Value::as_str).unwrap_or("").to_string(),
                                status: match t.get("status").and_then(Value::as_str) {
                                    Some("completed") => PlanStatus::Done,
                                    Some("in_progress") => PlanStatus::InProgress,
                                    _ => PlanStatus::Pending,
                                },
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.plan = if items.is_empty() { None } else { Some(items) };
            }
            EventType::BackgroundTaskUpdate => {
                let id = s("task_id");
                match p.get("state").and_then(Value::as_str) {
                    Some("running") => {
                        self.tasks.insert(id, s("label"));
                    }
                    Some("error") => {
                        self.tasks.remove(&id);
                        self.push_done(Block::Notice { kind: NoticeKind::Warn, text: format!("{} failed", s("label")) });
                    }
                    _ => {
                        self.tasks.remove(&id);
                    }
                }
            }
            EventType::SlashCommandsRegistered => {
                self.commands = p
                    .get("commands")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .map(|c| {
                                (
                                    c.get("name").and_then(Value::as_str).unwrap_or("").trim_start_matches('/').to_string(),
                                    c.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
                                    c.get("args_hint").and_then(Value::as_str).map(str::to_string),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
            }
            EventType::MentionCandidatesRegistered => {
                let tokens: Vec<String> = p
                    .get("candidates")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|c| c.get("token").and_then(Value::as_str).map(str::to_string)).collect())
                    .unwrap_or_default();
                match p.get("for_query").and_then(Value::as_str) {
                    Some(dir) => self.path_mentions = Some((dir.to_string(), tokens)),
                    None => self.mentions = tokens,
                }
                self.pick_sel = 0;
            }
            EventType::TranscriptCleared => {
                self.entries.retain(|e| !e.done);
                self.last_printed = None;
                self.plan = None;
                self.clear_screen = true;
            }
            EventType::CancelRequested => self.settle(now, true),
            _ => {}
        }
    }

    // ----- keys from the user -------------------------------------------

    pub fn key(&mut self, key: Key, now: i64) {
        self.handle_key(key, now);
        self.absorb_wizard_answers(now);
    }

    fn handle_key(&mut self, key: Key, now: i64) {
        if let Some(form) = self.form.as_mut() {
            match form.key(key) {
                FormOutcome::Pending => {}
                FormOutcome::Submitted(values) => {
                    let id = form.id.clone();
                    self.form = None;
                    self.outbound.push(Outbound { event_type: EventType::FormResponse, payload: json!({ "id": id, "values": values }) });
                }
                FormOutcome::Cancelled => {
                    let id = form.id.clone();
                    self.form = None;
                    self.outbound.push(Outbound { event_type: EventType::FormResponse, payload: json!({ "id": id, "cancelled": true }) });
                }
            }
            return;
        }
        if self.prompt.is_some() {
            self.prompt_key(key, now);
            return;
        }
        if key != Key::Ctrl('c') {
            self.ctrl_c_at = None;
        }
        let text_w = self.input_text_width();
        match key {
            Key::Ctrl('c') => {
                if !self.editor.is_empty() {
                    self.editor.clear();
                } else if self.is_busy() {
                    self.interrupt(now);
                } else if self.ctrl_c_at.map(|t| now - t < CTRL_C_WINDOW_MS).unwrap_or(false) {
                    self.exit = true;
                } else {
                    self.ctrl_c_at = Some(now);
                }
                return;
            }
            Key::Ctrl('d') if self.editor.is_empty() => {
                self.exit = true;
                return;
            }
            Key::Esc => {
                if self.is_busy() {
                    self.interrupt(now);
                } else if self.picker().is_some() {
                    self.pick_dismissed = Some(self.editor.text().to_string());
                }
                return;
            }
            Key::BackTab => {
                self.outbound.push(Outbound { event_type: EventType::ModeChangeRequested, payload: json!({ "direction": "next" }) });
                return;
            }
            Key::Ctrl('o') => {
                self.expanded = !self.expanded;
                self.flash(if self.expanded { "Showing full tool output" } else { "Tool output collapsed" }, now);
                return;
            }
            Key::Ctrl('r') => {
                self.show_reasoning = !self.show_reasoning;
                self.flash(if self.show_reasoning { "Showing reasoning" } else { "Reasoning hidden" }, now);
                return;
            }
            Key::Ctrl('l') => return,
            _ => {}
        }

        if let Some(items) = self.picker() {
            let n = items.len();
            match &key {
                Key::Up => {
                    self.pick_sel = (self.pick_sel + n - 1) % n;
                    return;
                }
                Key::Down => {
                    self.pick_sel = (self.pick_sel + 1) % n;
                    return;
                }
                Key::Tab | Key::Enter { shift: false, alt: false } => {
                    let submit = matches!(key, Key::Enter { .. });
                    self.accept_pick(submit);
                    return;
                }
                _ => {}
            }
        }

        let edit = match key {
            Key::Text(t) => EditKey::Text(t),
            Key::Paste(t) => EditKey::Paste(t),
            Key::Enter { shift, alt } => EditKey::Enter { newline: shift || alt },
            Key::Ctrl('j') => EditKey::Enter { newline: true },
            Key::Backspace { word } => EditKey::Backspace { word },
            Key::Ctrl('w') => EditKey::Backspace { word: true },
            Key::Delete { word } => EditKey::Delete { word },
            Key::Left { word } => EditKey::Left { word },
            Key::Right { word } => EditKey::Right { word },
            Key::Ctrl('b') => EditKey::Left { word: false },
            Key::Ctrl('f') => EditKey::Right { word: false },
            Key::Up | Key::Ctrl('p') => EditKey::Up,
            Key::Down | Key::Ctrl('n') => EditKey::Down,
            Key::Home | Key::Ctrl('a') => EditKey::LineStart,
            Key::End | Key::Ctrl('e') => EditKey::LineEnd,
            Key::Ctrl('k') => EditKey::KillToEnd,
            Key::Ctrl('u') => EditKey::KillToStart,
            Key::Ctrl('y') => EditKey::Yank,
            Key::Ctrl('z') | Key::Ctrl('_') => EditKey::Undo,
            _ => return,
        };
        match self.editor.handle(edit, text_w) {
            EditOutcome::Submit { text, display } => self.submit(text, display),
            EditOutcome::Changed => {
                self.pick_sel = 0;
                self.pick_dismissed = None;
                self.refresh_mention_query();
            }
            EditOutcome::None => {}
        }
    }

    fn submit(&mut self, text: String, display: String) {
        self.echoes.push_back(display.clone());
        if self.echoes.len() > 32 {
            self.echoes.pop_front();
        }
        self.push(Block::User { text: display }, true);
        self.outbound.push(Outbound { event_type: EventType::UserInputSubmitted, payload: json!({ "text": text }) });
    }

    fn prompt_key(&mut self, key: Key, now: i64) {
        let Some(p) = self.prompt.as_mut() else { return };
        let visible = visible_options(p);
        let n = visible.len().max(1);
        let pos = visible.iter().position(|&i| i == p.selected).unwrap_or(0);
        match key {
            Key::Up | Key::Ctrl('p') => p.selected = *visible.get((pos + n - 1) % n).unwrap_or(&p.selected),
            Key::Down | Key::Tab | Key::Ctrl('n') => p.selected = *visible.get((pos + 1) % n).unwrap_or(&p.selected),
            Key::Enter { .. } => {
                if visible.contains(&p.selected) {
                    let i = p.selected;
                    self.resolve_prompt(Some(i), now);
                }
            }
            Key::Esc | Key::Ctrl('c') => self.resolve_prompt(None, now),
            Key::Text(t) => {
                let filterable = matches!(p.kind, PromptKind::Select { filterable: true, .. });
                let digit = t.chars().next().and_then(|c| c.to_digit(10)).map(|d| d as usize);
                let yn = t.to_ascii_lowercase();
                if matches!(p.kind, PromptKind::Confirm { .. }) && (yn == "y" || yn == "n") {
                    self.resolve_prompt(Some(if yn == "y" { 0 } else { 1 }), now);
                } else if !filterable && digit.map(|d| d >= 1 && d <= p.options.len()).unwrap_or(false) {
                    self.resolve_prompt(Some(digit.unwrap() - 1), now);
                } else if filterable {
                    p.filter.push_str(&t);
                    if let Some(first) = visible_options(p).first() {
                        p.selected = *first;
                    }
                }
            }
            Key::Backspace { .. } => {
                p.filter.pop();
            }
            _ => {}
        }
    }

    fn resolve_prompt(&mut self, choice: Option<usize>, now: i64) {
        let Some(p) = self.prompt.take() else { return };
        match p.kind {
            PromptKind::Permission { request_id } => {
                let c = match choice {
                    Some(0) => "allow_once",
                    Some(1) => "allow_session",
                    _ => "deny",
                };
                self.outbound.push(Outbound {
                    event_type: EventType::PermissionResponse,
                    payload: json!({ "request_id": request_id, "choice": c, "feedback": "" }),
                });
                if choice.is_none() {
                    self.interrupt(now);
                }
            }
            PromptKind::Select { id, values, allow_cancel, .. } => {
                let payload = match choice {
                    Some(i) => json!({ "id": id, "value": values.get(i).cloned().unwrap_or_default() }),
                    None if allow_cancel => json!({ "id": id, "cancelled": true }),
                    None => {
                        self.prompt = Some(Prompt { kind: PromptKind::Select { id, values, allow_cancel, filterable: true }, ..p });
                        return;
                    }
                };
                self.outbound.push(Outbound { event_type: EventType::SelectListResponse, payload });
            }
            PromptKind::Confirm { id, allow_cancel } => {
                let payload = match choice {
                    Some(i) => json!({ "id": id, "value": i == 0 }),
                    None if allow_cancel => json!({ "id": id, "cancelled": true }),
                    None => {
                        self.prompt = Some(Prompt { kind: PromptKind::Confirm { id, allow_cancel }, ..p });
                        return;
                    }
                };
                self.outbound.push(Outbound { event_type: EventType::ConfirmResponse, payload });
            }
        }
    }

    /// Esc / Ctrl+C during a turn: tell the host, and settle the screen now
    /// so the user sees the stop immediately.
    fn interrupt(&mut self, now: i64) {
        self.outbound.push(Outbound { event_type: EventType::CancelRequested, payload: json!({}) });
        self.settle(now, true);
    }

    /// Stop everything in motion: running tools, streams, the plan. Nothing
    /// may keep animating after a turn ends, whatever the host sent.
    fn settle(&mut self, now: i64, interrupted: bool) {
        let was_busy = self.is_busy();
        let mut any = false;
        for e in self.entries.iter_mut().filter(|e| !e.done) {
            any = true;
            if let Block::Tool(t) = &mut e.block {
                t.status = ToolStatus::Stopped;
                t.summary.get_or_insert_with(|| "Stopped".into());
                t.elapsed_ms = t.started_at_ms.map(|st| (now - st).max(0) as u64);
            }
            e.done = true;
        }
        self.streams.clear();
        self.tools.clear();
        self.phase_busy = false;
        if let Some(plan) = self.plan.take() {
            if interrupted && plan.iter().any(|i| i.status != PlanStatus::Done) {
                self.push_done(Block::Plan { items: plan, live: false });
            }
        }
        if interrupted && (was_busy || any) {
            let app = if self.app_name.is_empty() { "the agent".to_string() } else { self.app_name.clone() };
            self.push_done(Block::Notice { kind: NoticeKind::Interrupted, text: format!("Interrupted · what should {app} do instead?") });
        }
        self.busy_since = None;
        self.prompt = self.prompt.take().filter(|p| !matches!(p.kind, PromptKind::Permission { .. }));
    }

    // ----- output -------------------------------------------------------

    /// Lines to print into history now: finished blocks at the front, plus
    /// completed paragraphs of a streaming reply. Printed blocks are dropped.
    pub fn take_history(&mut self, now: i64) -> Vec<Line> {
        self.split_streaming_paragraphs();
        let ctx = self.ctx(now);
        let mut out = Vec::new();
        while self.entries.first().map(|e| e.done).unwrap_or(false) {
            let e = self.entries.remove(0);
            // A reply that ended before any text (e.g. the API failed) has
            // nothing to show; printing it would only add a blank row.
            if matches!(&e.block, Block::Assistant { markdown } if markdown.trim().is_empty()) {
                continue;
            }
            if matches!(e.block, Block::Reasoning { .. }) && !self.show_reasoning {
                continue;
            }
            if let Some(prev) = &self.last_printed {
                if gap_between(prev, &e.block) {
                    out.push(Line::new());
                }
            }
            out.extend(render_block(&e.block, self.width, &self.theme, ctx));
            self.last_printed = Some(e.block);
        }
        out
    }

    pub fn live_frame(&self, now: i64) -> LiveFrame {
        let ctx = self.ctx(now);
        let t = &self.theme;
        let mut lines: Vec<Line> = Vec::new();
        let mut prev = self.last_printed.clone();
        for e in &self.entries {
            if matches!(e.block, Block::Reasoning { .. }) && !self.show_reasoning {
                continue;
            }
            if prev.as_ref().map(|p| gap_between(p, &e.block)).unwrap_or(false) {
                lines.push(Line::new());
            }
            let mut rendered = render_block(&e.block, self.width, t, ctx);
            if let Block::Assistant { .. } = e.block {
                if let Some(last) = rendered.last_mut() {
                    last.push("▍", t.accent());
                } else {
                    rendered.push(Line::raw("  ").with("▍", t.accent()));
                }
            }
            lines.extend(rendered);
            prev = Some(e.block.clone());
        }
        let busy = self.is_busy();
        if let (Some(plan), true) = (&self.plan, busy) {
            lines.push(Line::new());
            lines.extend(render_block(&Block::Plan { items: plan.clone(), live: true }, self.width, t, ctx));
        }
        if busy && self.prompt.is_none() {
            lines.push(Line::new());
            let elapsed = self.busy_since.map(|b| (now - b).max(0) as u64).unwrap_or(0);
            let tokens = (self.streamed_chars - self.streamed_at_busy) / 4;
            lines.push(chrome::spinner_line(t, ctx.frame, &self.verb(), elapsed, Some(tokens), self.width));
        } else if !self.tasks.is_empty() {
            lines.push(Line::new());
            let label = self.tasks.values().cloned().collect::<Vec<_>>().join(" · ");
            lines.push(Line::styled(crate::blocks::SPINNER[ctx.frame % 10], t.accent()).with(" ", Style::new()).with(label, t.dim()));
        }
        lines.push(Line::new());

        let mut cursor = None;
        if let Some(form) = &self.form {
            let (boxed, (cr, cc)) = form.render(t, self.width);
            cursor = Some((lines.len() + cr, cc));
            lines.extend(boxed);
        } else if let Some(p) = &self.prompt {
            let view = prompt_view(p, self.height);
            lines.extend(chrome::prompt_box(t, &view, self.width, self.height.saturating_sub(2)));
        } else {
            let (boxed, (cr, cc)) = chrome::input_box(t, &self.editor, self.mode, &self.placeholder(), self.width, 8);
            cursor = Some((lines.len() + cr, cc));
            lines.extend(boxed);
            if let Some(items) = self.picker() {
                lines.extend(chrome::picker(t, &items, self.pick_sel.min(items.len() - 1), self.width, 8));
            } else if self.editor.text() == "?" {
                lines.extend(chrome::shortcuts(t, self.width));
            } else {
                lines.push(chrome::footer(t, self.footer_left(now), &self.footer_right(), self.width));
            }
        }
        // The terminal keeps the bottom of an oversized frame; shift the
        // cursor with it.
        let max = self.height.saturating_sub(1);
        if lines.len() > max {
            let skip = lines.len() - max;
            lines.drain(..skip);
            cursor = cursor.and_then(|(r, c)| r.checked_sub(skip).map(|r| (r, c)));
        }
        LiveFrame { lines, cursor }
    }

    /// When the next frame is needed, or None when idle.
    pub fn next_wakeup(&self, now: i64) -> Option<i64> {
        let mut next: Option<i64> = None;
        let mut want = |t: i64| next = Some(next.map_or(t, |n: i64| n.min(t)));
        // Waiting on the user (a prompt is open) is idle: nothing animates.
        if (self.is_busy() && self.prompt.is_none()) || !self.tasks.is_empty() {
            want(now + FRAME_MS - now.rem_euclid(FRAME_MS));
        }
        if let Some((_, _, until)) = &self.foot {
            if *until > now {
                want(*until);
            }
        }
        if let Some(t) = self.ctrl_c_at {
            if t + CTRL_C_WINDOW_MS > now {
                want(t + CTRL_C_WINDOW_MS);
            }
        }
        next
    }

    pub fn is_busy(&self) -> bool {
        self.phase_busy || self.entries.iter().any(|e| !e.done)
    }

    // ----- helpers ------------------------------------------------------

    fn ctx(&self, now: i64) -> RenderCtx {
        RenderCtx { frame: (now / FRAME_MS).max(0) as usize, now_ms: now, expanded: self.expanded }
    }

    fn input_text_width(&self) -> usize {
        self.width.saturating_sub(6).max(2)
    }

    fn push(&mut self, block: Block, done: bool) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.entries.push(Entry { id, block, done });
        id
    }

    fn push_done(&mut self, block: Block) {
        self.push(block, true);
    }

    fn block_mut(&mut self, id: u64) -> Option<&mut Block> {
        self.entries.iter_mut().rev().find(|e| e.id == id).map(|e| &mut e.block)
    }

    fn finish(&mut self, id: u64) {
        if let Some(e) = self.entries.iter_mut().rev().find(|e| e.id == id) {
            e.done = true;
        }
    }

    fn became_busy(&mut self, now: i64) {
        if self.busy_since.is_none() {
            self.busy_since = Some(now);
            self.streamed_at_busy = self.streamed_chars;
        }
    }

    fn flash(&mut self, text: &str, now: i64) {
        self.foot = Some((text.to_string(), self.theme.accent(), now + 2500));
    }

    /// Move completed paragraphs of the leading streaming reply into their
    /// own finished block, so a long answer never outgrows the live region.
    fn split_streaming_paragraphs(&mut self) {
        let Some(first) = self.entries.first() else { return };
        if first.done {
            return;
        }
        let Block::Assistant { markdown } = &first.block else { return };
        let Some(at) = paragraph_split(markdown) else { return };
        let head = markdown[..at].trim_end().to_string();
        let tail = markdown[at..].trim_start_matches('\n').to_string();
        if head.is_empty() {
            return;
        }
        let consumed = markdown[..markdown.len() - tail.len()].to_string();
        let stream_id = first.id;
        self.printed_prefix.entry(stream_id).or_default().push_str(&consumed);
        if let Some(Entry { block: Block::Assistant { markdown }, .. }) = self.entries.first_mut() {
            *markdown = tail;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.entries.insert(0, Entry { id, block: Block::Assistant { markdown: head }, done: true });
    }

    fn verb(&self) -> String {
        if self.prompt.is_some() {
            return "Waiting for you".into();
        }
        if let Some(v) = self.segments.get("activity") {
            return v.clone();
        }
        if let Some(Block::Tool(t)) = self.entries.iter().rev().find(|e| !e.done && matches!(e.block, Block::Tool(_))).map(|e| &e.block) {
            return format!("Running {}", t.name);
        }
        if self.entries.iter().any(|e| !e.done && matches!(e.block, Block::Assistant { .. })) {
            return "Writing".into();
        }
        "Thinking".into()
    }

    fn placeholder(&self) -> String {
        if self.is_busy() {
            "Type ahead · esc to interrupt".into()
        } else {
            format!("Ask {} anything · / for commands", if self.app_name.is_empty() { "the agent" } else { &self.app_name })
        }
    }

    fn footer_left(&self, now: i64) -> Vec<Span> {
        if self.ctrl_c_at.map(|t| now - t < CTRL_C_WINDOW_MS).unwrap_or(false) {
            return vec![Span::styled("Press Ctrl+C again to exit", self.theme.dim())];
        }
        if let Some((text, style, until)) = &self.foot {
            if *until > now {
                return vec![Span::styled(text.clone(), *style)];
            }
        }
        if let Some(w) = self.segments.get("warn") {
            return vec![Span::styled(w.clone(), self.theme.warn())];
        }
        chrome::mode_hint(&self.theme, self.mode)
    }

    fn footer_right(&self) -> String {
        const SKIP: [&str; 5] = ["mode", "phase", "elapsed", "warn", "activity"];
        const ORDER: [&str; 4] = ["model", "tokens", "cost", "branch"];
        let mut parts: Vec<&str> = ORDER.iter().filter_map(|k| self.segments.get(*k).map(String::as_str)).collect();
        for (k, v) in &self.segments {
            if !SKIP.contains(&k.as_str()) && !ORDER.contains(&k.as_str()) {
                parts.push(v);
            }
        }
        parts.join(" · ")
    }

    fn picker(&self) -> Option<Vec<PickItem>> {
        let text = self.editor.text();
        if self.pick_dismissed.as_deref() == Some(text) {
            return None;
        }
        if let Some(q) = text.strip_prefix('/') {
            if q.contains(char::is_whitespace) || self.commands.is_empty() {
                return None;
            }
            // An exact name goes first, so `/mode` Enter runs /mode even
            // when /model was registered before it.
            let mut matches: Vec<_> = self.commands.iter().filter(|(n, _, _)| n.starts_with(q)).collect();
            matches.sort_by_key(|(n, _, _)| n != q);
            let items: Vec<PickItem> = matches
                .into_iter()
                .map(|(n, d, h)| PickItem {
                    label: format!("/{n}{}", h.as_ref().map(|h| format!(" {h}")).unwrap_or_default()),
                    hint: (!d.is_empty()).then(|| d.clone()),
                    hits: Vec::new(),
                })
                .collect();
            return (!items.is_empty()).then_some(items);
        }
        let q = self.mention_partial()?;
        if let Some(dir) = path_dir(&q) {
            let (for_dir, tokens) = self.path_mentions.as_ref()?;
            if *for_dir != dir {
                return None;
            }
            let rest = &q[dir.len().min(q.len())..];
            let items: Vec<PickItem> = tokens
                .iter()
                .filter_map(|t| {
                    let name = t.strip_prefix(dir.as_str()).unwrap_or(t);
                    fuzzy(name, rest).map(|(score, hits)| {
                        let offset = t.len() - name.len();
                        (score, PickItem { label: t.clone(), hint: None, hits: hits.into_iter().map(|h| h + offset).collect() })
                    })
                })
                .take(200)
                .map(|(_, i)| i)
                .take(8)
                .collect();
            return (!items.is_empty()).then_some(items);
        }
        if self.mentions.is_empty() {
            return None;
        }
        let mut scored: Vec<(usize, PickItem)> = self
            .mentions
            .iter()
            .filter_map(|m| fuzzy(m, &q).map(|(score, hits)| (score, PickItem { label: m.clone(), hint: None, hits })))
            .collect();
        scored.sort_by_key(|(s, i)| (*s, i.label.len()));
        let items: Vec<PickItem> = scored.into_iter().take(8).map(|(_, i)| i).collect();
        (!items.is_empty()).then_some(items)
    }

    fn accept_pick(&mut self, submit: bool) {
        let Some(items) = self.picker() else { return };
        let item = &items[self.pick_sel.min(items.len() - 1)];
        if self.editor.text().starts_with('/') {
            let name = item.label.split_whitespace().next().unwrap_or("").to_string();
            // A hint in brackets (`[list|<id>]`) means the arguments are
            // optional: Enter runs the command bare, as Ink does.
            let needs_args = self.commands.iter().any(|(n, _, h)| {
                format!("/{n}") == name && h.as_deref().is_some_and(|h| !h.trim().is_empty() && !h.trim_start().starts_with('['))
            });
            if submit && !needs_args {
                self.editor.clear();
                self.submit(name.clone(), name);
            } else {
                self.editor.set_text(format!("{name} "));
            }
        } else {
            let text = self.editor.text();
            let at = text[..self.editor.cursor()].rfind('@').unwrap_or(0);
            // Picking a folder steps into it: no trailing space, and the
            // picker stays open on that folder's entries.
            let insert = if item.label.ends_with('/') { format!("@{}", item.label) } else { format!("@{} ", item.label) };
            self.editor.replace_before_cursor(at, &insert);
            self.refresh_mention_query();
        }
        self.pick_sel = 0;
    }

    /// The text after `@` at the cursor, if the cursor is in a mention.
    fn mention_partial(&self) -> Option<String> {
        let text = self.editor.text();
        let before = &text[..self.editor.cursor()];
        let at = before.rfind('@')?;
        let q = &before[at + 1..];
        if q.contains(char::is_whitespace) || (at > 0 && !before[..at].ends_with(char::is_whitespace)) {
            return None;
        }
        Some(q.to_string())
    }

    /// Ask the host for a directory's entries when a path mention enters a
    /// directory we haven't listed yet.
    fn refresh_mention_query(&mut self) {
        let Some(q) = self.mention_partial() else { return };
        let Some(dir) = path_dir(&q) else { return };
        if self.mention_query_sent.as_deref() == Some(dir.as_str()) {
            return;
        }
        self.mention_query_sent = Some(dir);
        self.outbound.push(Outbound { event_type: EventType::MentionQuery, payload: json!({ "query": q }) });
    }
}

/// A multi-step `ShowWizard`, run as one prompt or form per step.
struct Wizard {
    id: String,
    title: String,
    allow_cancel: bool,
    steps: Vec<Value>,
    at: usize,
    results: serde_json::Map<String, Value>,
}

impl Session {
    /// Show the current wizard step through the ordinary prompt and form
    /// paths, using the step's id so its answer can be recognized.
    fn show_wizard_step(&mut self, now: i64) {
        let Some(w) = &self.wizard else { return };
        let step = w.steps[w.at].clone();
        let progress = format!("{}Step {} of {}", if w.title.is_empty() { String::new() } else { format!("{} · ", w.title) }, w.at + 1, w.steps.len());
        let allow_cancel = w.allow_cancel;
        let mut payload = step.clone();
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("allow_cancel".into(), json!(allow_cancel));
        }
        let event_type = match step.get("kind").and_then(Value::as_str) {
            Some("select") | Some("confirm") => {
                let prompt = step.get("prompt").and_then(Value::as_str).unwrap_or("");
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("prompt".into(), json!(format!("{prompt}  ({progress})")));
                }
                if step.get("kind").and_then(Value::as_str) == Some("select") {
                    EventType::ShowSelectList
                } else {
                    EventType::ShowConfirm
                }
            }
            _ => {
                let title = step.get("title").and_then(Value::as_str).unwrap_or("");
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("title".into(), json!(if title.is_empty() { progress } else { format!("{title}  ({progress})") }));
                }
                EventType::ShowForm
            }
        };
        let ev = Event {
            id: Default::default(),
            session_id: Default::default(),
            seq: 0,
            timestamp_ms: now,
            schema_version: camouflage_protocol::SCHEMA_VERSION,
            event_type,
            payload,
        };
        self.apply(&ev, now);
    }

    /// Answers to wizard steps are collected here instead of going to the
    /// host; the host gets one WizardCompleted (or WizardCancelled).
    fn absorb_wizard_answers(&mut self, now: i64) {
        if self.wizard.is_none() {
            return;
        }
        let mut i = 0;
        while i < self.outbound.len() {
            let Some(w) = self.wizard.as_mut() else { return };
            let step_id = w.steps[w.at].get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let o = &self.outbound[i];
            let is_answer = matches!(o.event_type, EventType::SelectListResponse | EventType::ConfirmResponse | EventType::FormResponse)
                && o.payload.get("id").and_then(Value::as_str) == Some(step_id.as_str());
            if !is_answer {
                i += 1;
                continue;
            }
            let o = self.outbound.remove(i);
            if o.payload.get("cancelled").and_then(Value::as_bool) == Some(true) {
                let (id, at) = (w.id.clone(), w.at);
                self.wizard = None;
                self.outbound.push(Outbound { event_type: EventType::WizardCancelled, payload: json!({ "id": id, "at_step": at }) });
                return;
            }
            let value = o.payload.get("value").or_else(|| o.payload.get("values")).cloned().unwrap_or(Value::Null);
            w.results.insert(step_id, value);
            w.at += 1;
            if w.at >= w.steps.len() {
                let (id, results) = (w.id.clone(), std::mem::take(&mut w.results));
                self.wizard = None;
                self.outbound.push(Outbound { event_type: EventType::WizardCompleted, payload: json!({ "id": id, "results": results }) });
                return;
            }
            self.show_wizard_step(now);
        }
    }
}

/// For a path mention (`./`, `../`, `~`, `/`), the directory part whose
/// entries should be listed, e.g. "../src/" for "../src/ma". Bare `~`, `.`
/// and `..` (and a trailing `/..`) mean that directory itself.
fn path_dir(q: &str) -> Option<String> {
    let is_path = q.starts_with('/') || q.starts_with('~') || q.starts_with("./") || q.starts_with("../") || q == "." || q == "..";
    if !is_path {
        return None;
    }
    if q == "~" || q == "." || q == ".." || q.ends_with("/..") {
        return Some(format!("{q}/"));
    }
    Some(q[..=q.rfind('/')?].to_string())
}

/// Options to show, in order. With a filter: fuzzy matches over the label,
/// then the description and keywords, best first.
fn visible_options(p: &Prompt) -> Vec<usize> {
    if p.filter.is_empty() {
        return (0..p.options.len()).collect();
    }
    let mut scored: Vec<(usize, usize)> = (0..p.options.len())
        .filter_map(|i| {
            let label = fuzzy(&p.options[i], &p.filter).map(|(s, _)| s);
            let extra = || {
                let hay = format!(
                    "{} {}",
                    p.hints.get(i).cloned().flatten().unwrap_or_default(),
                    p.keywords.get(i).cloned().unwrap_or_default()
                );
                fuzzy(&hay, &p.filter).map(|(s, _)| s + 1000)
            };
            label.or_else(extra).map(|score| (score, i))
        })
        .collect();
    scored.sort();
    scored.into_iter().map(|(_, i)| i).collect()
}

fn prompt_view(p: &Prompt, height: usize) -> PromptView {
    let visible = visible_options(p);
    let filtering = !p.filter.is_empty();
    // Window long lists around the selection.
    let room = height.saturating_sub(10).clamp(3, 14);
    let pos = visible.iter().position(|&i| i == p.selected).unwrap_or(0);
    let start = pos.saturating_sub(room / 2).min(visible.len().saturating_sub(room));
    let mut shown: Vec<usize> = visible.iter().copied().skip(start).take(room).collect();
    // Section headers take rows too; trim from the far end of the selection.
    let header_rows = |rows: &[usize]| -> usize {
        if filtering {
            return 0;
        }
        let mut last: Option<&String> = None;
        rows.iter()
            .filter(|&&i| {
                let sec = p.sections.get(i).and_then(|s| s.as_ref());
                let new = sec.is_some() && sec != last;
                last = sec;
                new
            })
            .count()
    };
    while shown.len() > 3 && shown.len() + header_rows(&shown) > room {
        if shown.last() != Some(&p.selected) {
            shown.pop();
        } else {
            shown.remove(0);
        }
    }
    let mut sections = Vec::with_capacity(shown.len());
    let mut last: Option<String> = None;
    for &i in &shown {
        let sec = p.sections.get(i).cloned().flatten();
        sections.push(if !filtering && sec.is_some() && sec != last { sec.clone() } else { None });
        last = sec;
    }
    let mut subtitle = p.subtitle.clone();
    let is_select = matches!(p.kind, PromptKind::Select { filterable: true, .. });
    if filtering || (is_select && p.options.len() > room) {
        let shown_count = if filtering { visible.len() } else { p.options.len() };
        let search = if filtering { format!("Search: {}", p.filter) } else { "Type to search".to_string() };
        subtitle = Some(format!("{search}  ·  {shown_count} of {}", p.options.len()));
    }
    let (esc_is_last, footer) = match p.kind {
        PromptKind::Permission { .. } => (true, "↑↓ to choose · enter to confirm · esc to interrupt".to_string()),
        PromptKind::Select { filterable: true, .. } => (false, "↑↓ to choose · type to search · enter to select · esc to cancel".to_string()),
        _ => (false, "↑↓ to choose · enter to select · esc to cancel".to_string()),
    };
    PromptView {
        title: p.title.clone(),
        subtitle,
        diff_lines: p.diff_lines.as_ref().map(|(_, l)| l.clone()),
        question: p.question.clone(),
        numbered: matches!(p.kind, PromptKind::Permission { .. } | PromptKind::Confirm { .. }) || p.options.len() <= 9,
        options: shown.iter().map(|&i| p.options[i].clone()).collect(),
        hints: shown.iter().map(|&i| p.hints.get(i).cloned().flatten()).collect(),
        sections,
        columns: shown.iter().map(|&i| p.columns.get(i).cloned().unwrap_or_default()).collect(),
        states: shown.iter().map(|&i| p.states.get(i).copied().flatten()).collect(),
        current: p.current.and_then(|c| shown.iter().position(|&i| i == c)),
        selected: shown.iter().position(|&i| i == p.selected).unwrap_or(0),
        esc_is_last,
        footer,
    }
}

/// Byte offset to split a streaming reply at: after the last blank line that
/// isn't inside a code fence.
fn paragraph_split(md: &str) -> Option<usize> {
    let mut in_fence = false;
    let mut best = None;
    let mut off = 0;
    for line in md.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
        }
        off += line.len();
        if !in_fence && line.trim().is_empty() && off < md.len() {
            best = Some(off);
        }
    }
    best
}

/// Append a chunk of tool output. A trailing empty entry means the last
/// line ended with a newline; the renderer ignores it. Carriage returns
/// (progress bars) keep only the text after the last one.
fn append_output(out: &mut Vec<String>, chunk: &str) {
    let chunk = chunk.replace("\r\n", "\n");
    let clean = |p: &str| crate::ansi::strip_ansi(p.rsplit('\r').next().unwrap_or(""));
    let mut parts = chunk.split('\n');
    let first = clean(parts.next().unwrap_or(""));
    match out.last_mut() {
        Some(last) => last.push_str(&first),
        None => out.push(first),
    }
    for p in parts {
        out.push(clean(p));
    }
    if out.len() > TOOL_TAIL_LINES {
        let drop = out.len() - TOOL_TAIL_LINES;
        out.drain(..drop);
    }
}

fn parse_diff_payload(d: &Value) -> Option<(String, Vec<crate::blocks::DiffLine>)> {
    let path = d.get("path").and_then(Value::as_str).unwrap_or("").to_string();
    if let Some(u) = d.get("unified").and_then(Value::as_str).or_else(|| d.as_str()) {
        return Some((path, parse_unified(u)));
    }
    let before = d.get("before").and_then(Value::as_str);
    let after = d.get("after").and_then(Value::as_str);
    if before.is_some() || after.is_some() {
        return Some((path, diff_texts(before.unwrap_or(""), after.unwrap_or(""), 2)));
    }
    None
}

fn table_markdown(p: &Value) -> String {
    let cols: Vec<(String, String)> = p
        .get("columns")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|c| {
                    let name = c.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                    let label = c.get("label").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| name.clone());
                    (name, label)
                })
                .collect()
        })
        .unwrap_or_default();
    let mut md = String::new();
    if let Some(t) = p.get("title").and_then(Value::as_str) {
        md.push_str(&format!("**{}**\n\n", escape_cell(t)));
    }
    md.push_str(&format!("| {} |\n", cols.iter().map(|c| escape_cell(&c.1)).collect::<Vec<_>>().join(" | ")));
    md.push_str(&format!("|{}\n", "---|".repeat(cols.len().max(1))));
    for row in p.get("rows").and_then(Value::as_array).into_iter().flatten() {
        let cells: Vec<String> = cols
            .iter()
            .map(|(name, _)| match row.get(name) {
                Some(Value::String(s)) => escape_cell(s),
                Some(Value::Null) | None => String::new(),
                Some(v) => escape_cell(&v.to_string()),
            })
            .collect();
        md.push_str(&format!("| {} |\n", cells.join(" | ")));
    }
    md
}

fn kv_markdown(p: &Value) -> String {
    let mut md = String::new();
    if let Some(t) = p.get("title").and_then(Value::as_str) {
        md.push_str(&format!("**{}**\n\n", escape_cell(t)));
    }
    for item in p.get("items").and_then(Value::as_array).into_iter().flatten() {
        let label = item.get("label").and_then(Value::as_str).unwrap_or("");
        let value = item.get("value").and_then(Value::as_str).unwrap_or("");
        md.push_str(&format!("- **{}** {}\n", escape_cell(label), escape_cell(value)));
    }
    md
}

fn escape_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Subsequence match: lower score is better (tighter span, earlier start).
fn fuzzy(candidate: &str, query: &str) -> Option<(usize, Vec<usize>)> {
    if query.is_empty() {
        return Some((0, Vec::new()));
    }
    let lower = candidate.to_lowercase();
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let mut hits = Vec::new();
    let mut qi = 0;
    for (bi, ch) in lower.char_indices() {
        if qi < q.len() && ch == q[qi] {
            hits.push(bi);
            qi += 1;
        }
    }
    if qi < q.len() {
        return None;
    }
    let span = hits.last().unwrap() - hits.first().unwrap();
    Some((span * 4 + hits[0], hits))
}

pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim().to_ascii_lowercase();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let v = u32::from_str_radix(hex, 16).ok()?;
            return Some(Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8));
        }
        return None;
    }
    Some(match s.as_str() {
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" | "purple" => Color::Magenta,
        "cyan" => Color::Cyan,
        "orange" => Color::Indexed(208),
        "white" => Color::White,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Build an event without the store's bookkeeping.
    fn ev(t: EventType, payload: serde_json::Value) -> Event {
        Event { id: Default::default(), session_id: Default::default(), seq: 0, timestamp_ms: 0, schema_version: 1, event_type: t, payload }
    }

    fn session() -> Session {
        Session::new("autopilot", Theme::default(), 60, 30)
    }

    fn history_text(s: &mut Session, now: i64) -> Vec<String> {
        s.take_history(now).iter().map(|l| l.plain_text()).collect()
    }

    fn live_text(s: &Session, now: i64) -> Vec<String> {
        s.live_frame(now).lines.iter().map(|l| l.plain_text()).collect()
    }

    fn type_str(s: &mut Session, t: &str) {
        for ch in t.chars() {
            s.key(Key::Text(ch.to_string()), 0);
        }
    }

    #[test]
    fn a_turn_prints_blocks_once_and_drops_them() {
        let mut s = session();
        s.apply(&ev(EventType::UserMessageCreated, json!({"text": "fix it"})), 0);
        s.apply(&ev(EventType::ToolExecutionStarted, json!({"tool_id": "t1", "tool": "Read", "command": "src/a.ts"})), 0);
        assert_eq!(history_text(&mut s, 0), vec!["› fix it"]);
        assert!(live_text(&s, 0).iter().any(|l| l.contains("Read src/a.ts")));
        s.apply(&ev(EventType::ToolExecutionFinished, json!({"tool_id": "t1", "exit_code": 0, "summary": "142 lines"})), 500);
        assert_eq!(history_text(&mut s, 500), vec!["", "✓ Read src/a.ts", "  └ 142 lines"]);
        assert_eq!(s.held_blocks(), 0);
        assert!(history_text(&mut s, 600).is_empty());
    }

    fn reasoning_turn(s: &mut Session) {
        s.apply(&ev(EventType::AssistantStreamStarted, json!({"stream_id": "s"})), 0);
        s.apply(&ev(EventType::AssistantReasoningDelta, json!({"stream_id": "s", "token": "Check the "})), 0);
        s.apply(&ev(EventType::AssistantReasoningDelta, json!({"stream_id": "s", "token": "config first."})), 0);
    }

    #[test]
    fn reasoning_is_hidden_by_default() {
        let mut s = session();
        reasoning_turn(&mut s);
        assert!(!live_text(&s, 0).iter().any(|l| l.contains("thinking…")));
        s.apply(&ev(EventType::AssistantTokenDelta, json!({"stream_id": "s", "token": "Done."})), 0);
        s.apply(&ev(EventType::AssistantMessageCompleted, json!({"stream_id": "s"})), 0);
        assert_eq!(history_text(&mut s, 0), vec!["  Done."]);
        assert_eq!(s.held_blocks(), 0);
    }

    #[test]
    fn ctrl_r_shows_reasoning_above_the_reply() {
        let mut s = session();
        s.key(Key::Ctrl('r'), 0);
        reasoning_turn(&mut s);
        assert!(live_text(&s, 0).iter().any(|l| l == "  thinking… Check the config first."));
        s.apply(&ev(EventType::AssistantTokenDelta, json!({"stream_id": "s", "token": "Done."})), 0);
        // Reasoning after the reply started is dropped, not shown mid-reply.
        s.apply(&ev(EventType::AssistantReasoningDelta, json!({"stream_id": "s", "token": "late"})), 0);
        s.apply(&ev(EventType::AssistantMessageCompleted, json!({"stream_id": "s"})), 0);
        let mut out = history_text(&mut s, 0);
        out.extend(history_text(&mut s, 0));
        assert_eq!(out, vec!["  thinking… Check the config first.", "", "  Done."]);
        assert_eq!(s.held_blocks(), 0);
    }

    #[test]
    fn long_reasoning_is_cut_like_ink() {
        let mut s = session();
        s.key(Key::Ctrl('r'), 0);
        s.apply(&ev(EventType::AssistantReasoningDelta, json!({"stream_id": "s", "token": "x".repeat(1000)})), 0);
        s.apply(&ev(EventType::AssistantMessageCompleted, json!({"stream_id": "s"})), 0);
        let shown: String = history_text(&mut s, 0).concat();
        assert_eq!(shown.matches('x').count(), 400);
        assert!(shown.ends_with('…'));
    }

    #[test]
    fn final_text_does_not_reprint_streamed_paragraphs() {
        let mut s = session();
        s.apply(&ev(EventType::AssistantStreamStarted, json!({"stream_id": "s"})), 0);
        s.apply(&ev(EventType::AssistantTokenDelta, json!({"stream_id": "s", "token": "One.\n\nTwo.\n\nThr"})), 0);
        let mut out = history_text(&mut s, 0);
        s.apply(&ev(EventType::AssistantTokenDelta, json!({"stream_id": "s", "token": "ee."})), 0);
        s.apply(&ev(EventType::AssistantMessageCompleted, json!({"stream_id": "s", "text": "One.\n\nTwo.\n\nThree."})), 0);
        out.extend(history_text(&mut s, 0));
        let text: Vec<_> = out.into_iter().filter(|l| !l.is_empty()).collect();
        assert_eq!(text, vec!["  One.", "  Two.", "  Three."]);
        assert_eq!(s.held_blocks(), 0);
    }

    #[test]
    fn idle_needs_no_wakeups_and_busy_ticks() {
        let mut s = session();
        assert_eq!(s.next_wakeup(1000), None);
        s.apply(&ev(EventType::AssistantStreamStarted, json!({"stream_id": "s"})), 1000);
        assert_eq!(s.next_wakeup(1000), Some(1040));
        s.apply(&ev(EventType::AssistantMessageCompleted, json!({"stream_id": "s"})), 1100);
        s.take_history(1100);
        assert_eq!(s.next_wakeup(1100), None);
    }

    #[test]
    fn streaming_replies_print_finished_paragraphs() {
        let mut s = session();
        s.apply(&ev(EventType::AssistantStreamStarted, json!({"stream_id": "s"})), 0);
        s.apply(&ev(EventType::AssistantTokenDelta, json!({"stream_id": "s", "token": "First paragraph.\n\nSecond"})), 0);
        assert_eq!(history_text(&mut s, 0), vec!["  First paragraph."]);
        let live = live_text(&s, 0);
        assert!(live.iter().any(|l| l == "  Second▍"), "{live:?}");
        s.apply(&ev(EventType::AssistantTokenDelta, json!({"stream_id": "s", "token": " half.\n```\ncode\n\nmore\n"})), 0);
        assert!(history_text(&mut s, 0).is_empty(), "never split inside a code fence");
    }

    #[test]
    fn submitting_echoes_once_and_tells_the_host() {
        let mut s = session();
        type_str(&mut s, "The build fails");
        s.key(Key::Enter { shift: false, alt: false }, 0);
        let out = s.take_outbound();
        assert_eq!(out, vec![Outbound { event_type: EventType::UserInputSubmitted, payload: json!({"text": "The build fails"}) }]);
        s.apply(&ev(EventType::UserMessageCreated, json!({"text": "The build fails"})), 0);
        assert_eq!(history_text(&mut s, 0), vec!["› The build fails"]);
    }

    #[test]
    fn esc_interrupts_and_settles_everything() {
        let mut s = session();
        s.apply(&ev(EventType::StatusUpdate, json!({"segments": {"phase": "thinking"}})), 0);
        s.apply(&ev(EventType::ToolExecutionStarted, json!({"tool_id": "t", "tool": "Bash", "command": "npm test"})), 0);
        s.key(Key::Esc, 100);
        assert_eq!(s.take_outbound()[0].event_type, EventType::CancelRequested);
        assert!(!s.is_busy());
        let h = history_text(&mut s, 100);
        assert!(h.iter().any(|l| l.starts_with("■ Bash")), "{h:?}");
        assert!(h.iter().any(|l| l.contains("Interrupted")), "{h:?}");
        assert_eq!(s.next_wakeup(100), None);
    }

    #[test]
    fn permission_prompt_round_trip() {
        let mut s = session();
        s.apply(
            &ev(EventType::PermissionRequested, json!({"request_id": "r1", "tool": "edit", "action": "edit src/a.ts", "diff": {"path": "src/a.ts", "before": "a\nb", "after": "a\nc"}})),
            0,
        );
        let live = live_text(&s, 0);
        assert!(live.iter().any(|l| l.contains("› 1. Yes")), "{live:?}");
        assert!(live.iter().any(|l| l.contains("+ c")), "{live:?}");
        s.key(Key::Down, 0);
        s.key(Key::Enter { shift: false, alt: false }, 0);
        assert_eq!(s.take_outbound()[0].payload, json!({"request_id": "r1", "choice": "allow_session", "feedback": ""}));
    }

    #[test]
    fn waiting_on_a_prompt_needs_no_wakeups() {
        let mut s = session();
        s.apply(&ev(EventType::StatusUpdate, json!({"segments": {"phase": "tool"}})), 0);
        assert!(s.next_wakeup(0).is_some());
        s.apply(&ev(EventType::PermissionRequested, json!({"request_id": "r", "tool": "bash", "action": "run npm test"})), 0);
        assert_eq!(s.next_wakeup(0), None);
    }

    #[test]
    fn select_lists_filter_as_you_type() {
        let mut s = session();
        s.apply(
            &ev(EventType::ShowSelectList, json!({"id": "m", "prompt": "Select model", "options": [
                {"value": "kimi", "label": "kimi-k2.6"}, {"value": "ds", "label": "deepseek-v3.2"}, {"value": "q", "label": "qwen3-coder"}
            ]})),
            0,
        );
        type_str(&mut s, "qw");
        s.key(Key::Enter { shift: false, alt: false }, 0);
        assert_eq!(s.take_outbound()[0].payload, json!({"id": "m", "value": "q"}));
    }

    #[test]
    fn rich_select_lists_search_labels_and_descriptions() {
        let mut s = session();
        s.apply(
            &ev(EventType::ShowSelectList, json!({"id": "m", "prompt": "Select model", "default": "kimi", "options": [
                {"value": "kimi", "label": "kimi-k2.6", "section": "Best & latest", "columns": ["256k", "$0.60"]},
                {"value": "ds", "label": "deepseek-v3.2", "description": "DeepSeek V3.2", "section": "Best & latest"},
                {"value": "q", "label": "qwen3-coder", "description": "Qwen3 Coder 480B", "section": "Other"}
            ]})),
            0,
        );
        let live = live_text(&s, 0);
        assert!(live.iter().any(|l| l.contains("Best & latest")), "{live:?}");
        assert!(live.iter().any(|l| l.contains("✓ current")), "{live:?}");
        // Fuzzy, and the description counts: "480" only appears there.
        type_str(&mut s, "480");
        let live = live_text(&s, 0);
        assert!(!live.iter().any(|l| l.contains("Best & latest")), "headers hide while searching");
        s.key(Key::Enter { shift: false, alt: false }, 0);
        assert_eq!(s.take_outbound()[0].payload, json!({"id": "m", "value": "q"}));
    }

    #[test]
    fn forms_collect_fields_and_check_required_ones() {
        let mut s = session();
        s.apply(
            &ev(EventType::ShowForm, json!({"id": "f", "title": "New skill", "fields": [
                {"name": "name", "label": "Name", "required": true},
                {"name": "key", "label": "API key", "kind": "password"}
            ]})),
            0,
        );
        // Enter on the first field moves on; submitting with Name empty refuses.
        s.key(Key::Enter { shift: false, alt: false }, 0);
        type_str(&mut s, "sk-123");
        let live = live_text(&s, 0).join("\n");
        assert!(live.contains("••••••") && !live.contains("sk-123"), "password is masked");
        s.key(Key::Enter { shift: false, alt: false }, 0);
        assert!(s.take_outbound().is_empty());
        assert!(live_text(&s, 0).iter().any(|l| l.contains("Name is required")));
        type_str(&mut s, "deploy-checks");
        s.key(Key::Tab, 0);
        s.key(Key::Enter { shift: false, alt: false }, 0);
        assert_eq!(
            s.take_outbound()[0].payload,
            json!({"id": "f", "values": {"name": "deploy-checks", "key": "sk-123"}})
        );
        assert!(live_text(&s, 0).iter().any(|l| l.contains("›") && l.contains("Ask")), "the input box is back");
    }

    #[test]
    fn wizards_run_each_step_and_report_once() {
        let mut s = session();
        s.apply(
            &ev(EventType::ShowWizard, json!({"id": "w", "title": "New command", "steps": [
                {"kind": "form", "id": "basics", "fields": [{"name": "name", "label": "Name"}]},
                {"kind": "select", "id": "scope", "prompt": "Where?", "options": [{"value": "project", "label": "Project"}, {"value": "global", "label": "Global"}]},
                {"kind": "confirm", "id": "ok", "prompt": "Create it?"}
            ]})),
            0,
        );
        assert!(live_text(&s, 0).iter().any(|l| l.contains("Step 1 of 3")));
        type_str(&mut s, "deploy");
        s.key(Key::Enter { shift: false, alt: false }, 0);
        assert!(s.take_outbound().is_empty(), "step answers stay inside the wizard");
        assert!(live_text(&s, 0).iter().any(|l| l.contains("Step 2 of 3")));
        s.key(Key::Down, 0);
        s.key(Key::Enter { shift: false, alt: false }, 0);
        s.key(Key::Text("y".into()), 0);
        assert_eq!(
            s.take_outbound(),
            vec![Outbound { event_type: EventType::WizardCompleted, payload: json!({"id": "w", "results": {"basics": {"name": "deploy"}, "scope": "global", "ok": true}}) }]
        );
    }

    #[test]
    fn cancelling_a_wizard_reports_the_step() {
        let mut s = session();
        s.apply(&ev(EventType::ShowWizard, json!({"id": "w", "steps": [
            {"kind": "confirm", "id": "a", "prompt": "First?"},
            {"kind": "confirm", "id": "b", "prompt": "Second?"}
        ]})), 0);
        s.key(Key::Text("y".into()), 0);
        s.key(Key::Esc, 0);
        assert_eq!(s.take_outbound(), vec![Outbound { event_type: EventType::WizardCancelled, payload: json!({"id": "w", "at_step": 1}) }]);
    }

    #[test]
    fn multiline_fields_take_newlines_and_tab_submits() {
        let mut s = session();
        s.apply(&ev(EventType::ShowForm, json!({"id": "f", "fields": [
            {"name": "name", "label": "Name"},
            {"name": "template", "label": "Template", "kind": "multiline"}
        ]})), 0);
        type_str(&mut s, "review");
        s.key(Key::Enter { shift: false, alt: false }, 0);
        type_str(&mut s, "Review $ARGUMENTS");
        s.key(Key::Enter { shift: false, alt: false }, 0);
        type_str(&mut s, "Be strict.");
        assert!(s.take_outbound().is_empty(), "Enter adds a line in a multi-line field");
        let live = live_text(&s, 0).join("\n");
        assert!(live.contains("Review $ARGUMENTS") && live.contains("Be strict."), "{live}");
        s.key(Key::Tab, 0);
        assert_eq!(
            s.take_outbound()[0].payload,
            json!({"id": "f", "values": {"name": "review", "template": "Review $ARGUMENTS\nBe strict."}})
        );
    }

    #[test]
    fn esc_cancels_a_form() {
        let mut s = session();
        s.apply(&ev(EventType::ShowForm, json!({"id": "f", "fields": [{"name": "q", "label": "Search"}]})), 0);
        s.key(Key::Esc, 0);
        assert_eq!(s.take_outbound()[0].payload, json!({"id": "f", "cancelled": true}));
    }

    #[test]
    fn slash_picker_submits_commands_without_args() {
        let mut s = session();
        s.apply(&ev(EventType::SlashCommandsRegistered, json!({"commands": [{"name": "compact", "description": "Summarize"}, {"name": "model", "args_hint": "<id>"}]})), 0);
        type_str(&mut s, "/co");
        assert!(live_text(&s, 0).iter().any(|l| l.contains("/compact")));
        s.key(Key::Enter { shift: false, alt: false }, 0);
        assert_eq!(s.take_outbound()[0].payload, json!({"text": "/compact"}));
    }

    #[test]
    fn slash_picker_runs_optional_args_and_fills_required_ones() {
        let mut s = session();
        s.apply(&ev(EventType::SlashCommandsRegistered, json!({"commands": [
            {"name": "hooks", "args_hint": "[list|enable <id>]"},
            {"name": "remote", "args_hint": "<prompt>"},
            {"name": "model", "args_hint": "[<id>]"},
            {"name": "mode"},
        ]})), 0);
        type_str(&mut s, "/mode");
        s.key(Key::Enter { shift: false, alt: false }, 0);
        assert_eq!(s.take_outbound()[0].payload, json!({"text": "/mode"}));
        type_str(&mut s, "/hoo");
        s.key(Key::Enter { shift: false, alt: false }, 0);
        assert_eq!(s.take_outbound()[0].payload, json!({"text": "/hooks"}));
        type_str(&mut s, "/rem");
        s.key(Key::Enter { shift: false, alt: false }, 0);
        assert!(s.take_outbound().is_empty());
        assert_eq!(s.editor.text(), "/remote ");
    }

    #[test]
    fn ctrl_c_twice_exits_when_idle() {
        let mut s = session();
        s.key(Key::Ctrl('c'), 0);
        assert!(!s.exit);
        assert!(live_text(&s, 10).iter().any(|l| l.contains("Press Ctrl+C again")));
        s.key(Key::Ctrl('c'), 500);
        assert!(s.exit);
    }

    #[test]
    fn capital_letters_never_trigger_anything() {
        let mut s = session();
        type_str(&mut s, "TMXS?");
        assert!(s.take_outbound().is_empty());
        assert!(live_text(&s, 0).iter().any(|l| l.contains("TMXS?")));
    }

    #[test]
    fn status_segments_fill_the_footer_and_mode() {
        let mut s = session();
        s.apply(&ev(EventType::StatusUpdate, json!({"segments": {"mode": "plan", "model": "kimi-k2.6", "cost": "$0.04"}})), 0);
        let live = live_text(&s, 0);
        let footer = live.last().unwrap();
        assert!(footer.contains("plan mode on") && footer.ends_with("kimi-k2.6 · $0.04"), "{footer}");
    }

    #[test]
    fn tool_output_chunks_split_into_lines() {
        let mut out = Vec::new();
        append_output(&mut out, "one\n\x1b[32mtwo\x1b[0m\nthr");
        append_output(&mut out, "ee\n");
        append_output(&mut out, "10%\r50%\r100%\n");
        assert_eq!(out, vec!["one", "two", "three", "100%", ""]);
    }

    #[test]
    fn empty_replies_print_nothing() {
        let mut s = session();
        s.apply(&ev(EventType::UserMessageCreated, json!({"text": "hi"})), 0);
        s.apply(&ev(EventType::AssistantStreamStarted, json!({"stream_id": "s"})), 0);
        s.apply(&ev(EventType::AssistantMessageCompleted, json!({"stream_id": "s"})), 0);
        s.apply(&ev(EventType::RuntimeError, json!({"message": "network error"})), 0);
        assert_eq!(history_text(&mut s, 0), vec!["› hi", "", "✗ network error"]);
    }

    #[test]
    fn echoed_prompts_print_once_even_after_they_scroll_out() {
        let mut s = session();
        type_str(&mut s, "hello there");
        s.key(Key::Enter { shift: false, alt: false }, 0);
        assert_eq!(history_text(&mut s, 0), vec!["› hello there"]);
        s.apply(&ev(EventType::UserMessageCreated, json!({"text": "hello there"})), 0);
        assert!(history_text(&mut s, 0).is_empty(), "the host's copy is not printed again");
        s.apply(&ev(EventType::UserMessageCreated, json!({"text": "a resumed message"})), 0);
        assert_eq!(history_text(&mut s, 0), vec!["", "› a resumed message"]);
    }

    #[test]
    fn path_mentions_ask_the_host_and_step_into_folders() {
        let mut s = session();
        s.apply(&ev(EventType::MentionCandidatesRegistered, json!({"candidates": [{"token": "src/a.ts"}]})), 0);
        type_str(&mut s, "look at @../");
        // Typed one key at a time: "." lists ./, ".." lists ../, and "../"
        // reuses that listing instead of asking again.
        let dirs: Vec<String> = s.take_outbound().iter().map(|o| path_dir(o.payload["query"].as_str().unwrap()).unwrap()).collect();
        assert_eq!(dirs, vec!["./", "../"]);
        s.apply(&ev(EventType::MentionCandidatesRegistered, json!({"for_query": "../", "candidates": [
            {"token": "../camouflage/", "kind": "dir"}, {"token": "../notes.md"}
        ]})), 0);
        type_str(&mut s, "cam");
        assert!(live_text(&s, 0).iter().any(|l| l.contains("../camouflage/")));
        s.key(Key::Tab, 0);
        // Stepped into the folder: no trailing space, and the host is asked again.
        assert!(live_text(&s, 0).iter().any(|l| l.contains("@../camouflage/")));
        assert_eq!(s.take_outbound(), vec![Outbound { event_type: EventType::MentionQuery, payload: json!({"query": "../camouflage/"}) }]);
    }

    #[test]
    fn path_dir_parses_mentions() {
        assert_eq!(path_dir("../sr").as_deref(), Some("../"));
        assert_eq!(path_dir("~").as_deref(), Some("~/"));
        assert_eq!(path_dir("..").as_deref(), Some("../"));
        assert_eq!(path_dir("/usr/lo").as_deref(), Some("/usr/"));
        assert_eq!(path_dir("src/a"), None);
        assert_eq!(path_dir(".gitignore"), None);
    }

    #[test]
    fn frame_never_exceeds_the_screen() {
        let mut s = Session::new("a", Theme::default(), 40, 12);
        s.apply(&ev(EventType::AssistantStreamStarted, json!({"stream_id": "s"})), 0);
        let long = "word ".repeat(400);
        s.apply(&ev(EventType::AssistantTokenDelta, json!({"stream_id": "s", "token": long})), 0);
        let frame = s.live_frame(0);
        assert!(frame.lines.len() <= 11);
        let (r, _) = frame.cursor.unwrap();
        assert!(frame.lines[r].plain_text().contains('›'));
    }
}
