//! Transcript blocks and how each one looks. This is the visual spec from
//! `docs/inline-redesign.md` §2 in code:
//!
//! - user prompts: a tinted band with a dim `›`, no speaker label
//! - assistant text: markdown indented two columns, no label
//! - tools: one quiet row each (glyph, name, args cut in the middle, then
//!   `+a −r` / failure / elapsed); a running tool adds one faint line of its
//!   latest output; failures show the end of theirs; runs of reads, searches
//!   and read-only shell commands fold into `Explored 5 files · 3 searches ·
//!   2 commands`, and runs of one tool (code steps) into `Code · 25 steps`
//!   with the name printed once. Ctrl+O shows the full layout (summary, diff,
//!   output).
//! - plan updates (`tasks_set`) are state, not actions: the plan is pinned in
//!   the live area, so they print no row of their own.
//! - one blank row between blocks; consecutive tool rows sit together

use crate::highlight::highlight;
use crate::markdown::render_markdown;
use crate::style::{Line, Span, Style};
use crate::theme::Theme;
use crate::width::{ellipsize_middle, str_width, truncate, wrap};

pub const SPINNER: [&str; 10] = crate::glyphs::SPINNER;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Ok,
    Error,
    /// The user declined it (permission denied).
    Declined,
    /// Interrupted before it finished.
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffKind {
    Context,
    Add,
    Del,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub line_no: Option<u32>,
    pub kind: DiffKind,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolBlock {
    pub name: String,
    pub args: String,
    pub status: ToolStatus,
    pub summary: Option<String>,
    /// Captured output, one entry per line.
    pub output: Vec<String>,
    /// Highlight output as this language (e.g. a written file); plain if None.
    pub output_lang: Option<String>,
    /// Output lines shown when collapsed. Errors show more by default.
    pub preview: usize,
    pub diff: Option<(String, Vec<DiffLine>)>,
    pub started_at_ms: Option<i64>,
    pub elapsed_ms: Option<u64>,
    /// What this tool counts as in a group summary, when the host said it
    /// only reads and the command itself couldn't be classified.
    pub explore: Option<&'static str>,
}

impl ToolBlock {
    pub fn new(name: impl Into<String>, args: impl Into<String>) -> Self {
        ToolBlock {
            name: name.into(),
            args: args.into(),
            status: ToolStatus::Running,
            summary: None,
            output: Vec::new(),
            output_lang: None,
            preview: 0,
            diff: None,
            started_at_ms: None,
            elapsed_ms: None,
            explore: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanStatus {
    Pending,
    InProgress,
    Done,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanItem {
    pub title: String,
    pub status: PlanStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Info,
    Success,
    Warn,
    Error,
    /// A turn was interrupted; printed under the last row.
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Welcome {
        title: String,
        detail: Vec<String>,
    },
    User {
        text: String,
    },
    Assistant {
        markdown: String,
    },
    /// The model's reasoning before a reply. Drawn only when the user has
    /// turned reasoning on (Ctrl+R); the session skips it otherwise.
    Reasoning {
        text: String,
    },
    Tool(ToolBlock),
    Plan {
        items: Vec<PlanItem>,
        live: bool,
    },
    Notice {
        kind: NoticeKind,
        text: String,
    },
    /// Pre-styled lines from the host (the generic escape hatch).
    Custom {
        lines: Vec<Line>,
    },
    /// A run of read-only steps (reads, searches, listings) folded into one
    /// row: `✓ Explored 5 files · 3 searches`.
    Group(GroupBlock),
}

/// Consecutive tools that fold into one row. Open while more may join; done
/// once closed and every tool has finished.
///
/// `label` is `None` for a run of read-only steps (`Explored 5 files`), and
/// the tool's name for a run of one tool (`Code · 25 steps`), where the name
/// is printed once as a header instead of on every row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupBlock {
    pub tools: Vec<ToolBlock>,
    pub closed: bool,
    pub label: Option<String>,
}

/// Tools that fold into a named run (`Code · 25 steps`) rather than getting a
/// row each. Returns the run's label.
pub fn run_label(name: &str) -> Option<&'static str> {
    let n = name.to_ascii_lowercase();
    Some(match n.as_str() {
        "execute_code" | "code" => "Code",
        _ => return None,
    })
}

/// What a read-only tool counts as in a group summary, or None when the
/// tool changes something (and so gets its own row).
pub fn explore_kind(name: &str) -> Option<&'static str> {
    let n = name.to_ascii_lowercase();
    Some(match n.as_str() {
        "read" | "view" | "cat" | "open" | "read_file" => "file",
        "grep" | "glob" | "find" | "search" | "rg" | "codesearch" | "code_search"
        | "web_search" | "websearch" => "search",
        "ls" | "list" | "list_dir" | "tree" => "folder",
        "web_fetch" | "webfetch" | "fetch" | "browser_fetch" | "browserfetch" => "page",
        "github_read_code" | "github_read_issue" | "github_read_pr" | "github_read_file"
        | "github_get_file" => "page",
        "memory_recall" | "job_status" | "job_logs" | "worker_status" | "github_pr_checks"
        | "github_list_releases" | "github_list_merged_prs" | "github_list_prs"
        | "github_list_issues" | "github_list_commits" => "lookup",
        _ if n.starts_with("lsp_") => "lookup",
        _ => return None,
    })
}

/// Like [`explore_kind`], but also reads the arguments: a shell tool is
/// exploration when its command only reads (`grep -rn foo`, `ls src`,
/// `sed -n '1,20p' a.rs`). Those are how a turn actually explores a repo, so
/// they join the run of reads instead of breaking it with a row each.
pub fn explore_kind_of(name: &str, args: &str) -> Option<&'static str> {
    if let Some(kind) = explore_kind(name) {
        return Some(kind);
    }
    match name.to_ascii_lowercase().as_str() {
        "bash" | "shell" | "sh" | "run" | "exec" | "terminal" | "command" => {
            shell_explore_kind(args)
        }
        _ => None,
    }
}

/// Tools that only update the plan. Their rows are noise: the plan itself is
/// pinned in the live area and arrives as `TodoListUpdate`.
pub fn is_plan_update(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "tasks_set" | "set_tasks" | "todo_write" | "todowrite" | "update_plan" | "plan_set"
    )
}

/// What a read-only shell command counts as in a group summary, or None when
/// it may change something (and so gets its own row).
pub fn shell_explore_kind(command: &str) -> Option<&'static str> {
    let mut best: Option<&'static str> = None;
    let mut seen = false;
    for seg in shell_segments(strip_prompt(command)) {
        let kind = segment_kind(&seg)?;
        seen = true;
        if best.is_none_or(|b| rank(kind) > rank(b)) {
            best = Some(kind);
        }
    }
    seen.then_some(best.unwrap_or("command"))
}

/// A host may decorate a shell command with a prompt — autopilot's bash rows
/// send `"$ grep -n foo"` — and that is not part of the command. Only a `$`
/// followed by whitespace is a prompt; `$HOME/bin/x` is a path.
fn strip_prompt(command: &str) -> &str {
    let command = command.trim_start();
    match command.strip_prefix('$') {
        Some(rest) if rest.chars().next().is_none_or(char::is_whitespace) => rest.trim_start(),
        _ => command,
    }
}

/// Which noun wins when one command does several read-only things.
fn rank(kind: &str) -> u8 {
    match kind {
        "search" => 3,
        "folder" => 2,
        "file" => 1,
        _ => 0,
    }
}

/// Split a command into its simple commands. Separators inside quotes don't
/// count, and `>&` / `&>` are redirections, not separators.
fn shell_segments(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            cur.push(c);
            if c == '\\' && q == '"' {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                cur.push(c);
            }
            '\\' => {
                cur.push(c);
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            ';' | '\n' => out.push(std::mem::take(&mut cur)),
            '|' | '&' => {
                if cur.ends_with('>') || chars.peek() == Some(&'>') {
                    cur.push(c);
                    continue;
                }
                out.push(std::mem::take(&mut cur));
                if chars.peek() == Some(&c) {
                    chars.next();
                }
            }
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out.retain(|s| !s.trim().is_empty());
    out
}

/// Words of one simple command, with quotes removed.
fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in s.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else {
                    cur.push(c);
                }
                started = true;
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    started = true;
                }
                c if c.is_whitespace() => {
                    if started {
                        out.push(std::mem::take(&mut cur));
                        started = false;
                    }
                }
                _ => {
                    cur.push(c);
                    started = true;
                }
            },
        }
    }
    if started {
        out.push(cur);
    }
    out
}

/// Drop the harmless redirections a command uses to silence noise.
fn strip_dev_null(s: &str) -> String {
    let mut out = s.to_string();
    for pat in [
        "2>/dev/null",
        "1>/dev/null",
        "&>/dev/null",
        ">/dev/null",
        "2>&1",
        "1>&2",
    ] {
        out = out.replace(pat, " ");
    }
    out
}

fn has_unquoted(s: &str, needle: char) -> bool {
    let mut quote: Option<char> = None;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == '\\' && q == '"' {
                    chars.next();
                } else if c == q {
                    quote = None;
                }
            }
            None => {
                if c == '\'' || c == '"' {
                    quote = Some(c);
                } else if c == needle {
                    return true;
                }
            }
        }
    }
    false
}

fn is_env_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// One simple command: what it reads, or None when it may write.
fn segment_kind(seg: &str) -> Option<&'static str> {
    let seg = strip_dev_null(seg);
    // "… +3 lines": the preview hid whole commands, and one of them may write.
    if seg.contains("… +") {
        return None;
    }
    // A redirection writes a file; only /dev/null was harmless.
    if has_unquoted(&seg, '>') {
        return None;
    }
    let mut w = words(&seg);
    while let Some(first) = w.first() {
        let f = first.as_str();
        if f == "!" || f == "time" || f == "command" || f == "builtin" || is_env_assignment(f) {
            w.remove(0);
        } else {
            break;
        }
    }
    let cmd = w.first()?.rsplit('/').next().unwrap_or("");
    // A preview cut to fit the row ends in "…". The visible part is what we
    // can judge, so a bare ellipsis is neutral and a clipped word is still
    // that word ("echo…" is "echo").
    if cmd.starts_with('…') {
        return Some("command");
    }
    let cmd = cmd.trim_end_matches('…');
    let args: Vec<&str> = w.iter().skip(1).map(String::as_str).collect();
    Some(match cmd {
        "grep" | "egrep" | "fgrep" | "rg" | "ag" | "ack" | "fd" | "locate" => "search",
        "find" => {
            if args.iter().any(|a| {
                matches!(
                    *a,
                    "-delete"
                        | "-exec"
                        | "-execdir"
                        | "-ok"
                        | "-okdir"
                        | "-fprint"
                        | "-fprintf"
                        | "-fls"
                )
            }) {
                return None;
            }
            "search"
        }
        "ls" | "tree" | "du" | "df" => "folder",
        "cat" | "head" | "tail" | "wc" | "nl" | "bat" | "less" | "more" | "jq" | "diff"
        | "file" | "stat" | "md5sum" | "shasum" | "sha256sum" | "cut" | "sort" | "uniq" | "tr"
        | "column" | "basename" | "dirname" | "realpath" | "readlink" => "file",
        "sed" => {
            if args
                .iter()
                .any(|a| *a == "--in-place" || a.starts_with("--in-place=") || a.starts_with("-i"))
            {
                return None;
            }
            "file"
        }
        "awk" | "gawk" | "mawk" => "file",
        "echo" | "printf" | "pwd" | "which" | "type" | "env" | "date" | "true" | "false" | "cd"
        | "export" | "set" | "unset" | "sleep" | "test" | "[" | "seq" | "yes" | "whoami"
        | "uname" | "hostname" => "command",
        "git" => git_kind(&args)?,
        _ => return None,
    })
}

/// A git subcommand that only reads.
fn git_kind(args: &[&str]) -> Option<&'static str> {
    // Global flags before the subcommand: `git -C dir log`, `git --no-pager diff`.
    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" => i += 2,
            a if a.starts_with("--git-dir=")
                || a.starts_with("--work-tree=")
                || a.starts_with("-c") =>
            {
                i += 1
            }
            "--no-pager" | "--paginate" | "-P" | "--no-optional-locks" => i += 1,
            _ => break,
        }
    }
    let sub = *args.get(i)?;
    let rest = &args[i + 1..];
    Some(match sub {
        "grep" => "search",
        "log" | "show" | "diff" | "blame" | "status" | "ls-files" | "rev-parse" | "describe"
        | "shortlog" | "cat-file" | "show-ref" | "for-each-ref" | "whatchanged" | "reflog" => {
            "command"
        }
        // Listing forms only: `git branch new` creates one.
        "branch" | "tag" => {
            if rest.iter().any(|a| !a.starts_with('-')) {
                return None;
            }
            "command"
        }
        "remote" => {
            if rest
                .iter()
                .any(|a| !matches!(*a, "-v" | "--verbose" | "show" | "get-url"))
            {
                return None;
            }
            "command"
        }
        "stash" => {
            if !matches!(rest.first().copied(), None | Some("list") | Some("show")) {
                return None;
            }
            "command"
        }
        "config" => {
            if !rest
                .iter()
                .any(|a| matches!(*a, "-l" | "--list" | "--get" | "--get-all" | "--get-regexp"))
            {
                return None;
            }
            "command"
        }
        "worktree" => {
            if rest.first().copied() != Some("list") {
                return None;
            }
            "command"
        }
        _ => return None,
    })
}

fn plural(n: usize, noun: &str) -> String {
    match (n, noun) {
        (1, _) => format!("1 {noun}"),
        (_, "search") => format!("{n} searches"),
        _ => format!("{n} {noun}s"),
    }
}

/// Per-frame context: which spinner frame to show, the current time for
/// elapsed counters, and whether collapsed output is expanded.
#[derive(Clone, Copy, Debug, Default)]
pub struct RenderCtx {
    pub frame: usize,
    pub now_ms: i64,
    pub expanded: bool,
}

/// Whether a blank row separates `prev` and `next`.
pub fn gap_between(prev: &Block, next: &Block) -> bool {
    let tight = |b: &Block| matches!(b, Block::Tool(_) | Block::Group(_) | Block::Notice { .. });
    !(tight(prev) && tight(next))
}

pub fn render_block(block: &Block, width: usize, theme: &Theme, ctx: RenderCtx) -> Vec<Line> {
    match block {
        Block::Welcome { title, detail } => {
            let mut out = vec![Line::new().with(title.clone(), theme.accent().bold())];
            for d in detail {
                out.extend(wrap(
                    &Line::styled(d.clone(), theme.dim()),
                    width,
                    &Line::new(),
                ));
            }
            out
        }
        Block::User { text } => render_user(text, width, theme),
        Block::Assistant { markdown } => {
            indent_lines(render_markdown(markdown, width.saturating_sub(2), theme), 2)
        }
        Block::Reasoning { text } => render_reasoning(text, width, theme),
        Block::Tool(t) => render_tool(t, width, theme, ctx),
        Block::Plan { items, live } => render_plan(items, *live, width, theme),
        Block::Notice { kind, text } => render_notice(*kind, text, width, theme),
        Block::Custom { lines } => lines.clone(),
        Block::Group(g) => render_group(g, width, theme, ctx),
    }
}

fn render_group(g: &GroupBlock, width: usize, theme: &Theme, ctx: RenderCtx) -> Vec<Line> {
    if g.label.is_some() {
        // One step reads better as a plain tool row than as a header with
        // nothing under it.
        if g.tools.len() == 1 {
            return render_tool(&g.tools[0], width, theme, ctx);
        }
        return render_run(g, width, theme, ctx);
    }
    if ctx.expanded || g.tools.len() == 1 {
        return g
            .tools
            .iter()
            .flat_map(|t| render_tool(t, width, theme, ctx))
            .collect();
    }
    let dim = theme.dim();
    let live = !g.closed || g.tools.iter().any(|t| t.status == ToolStatus::Running);
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for t in &g.tools {
        let kind = t
            .explore
            .or_else(|| explore_kind_of(&t.name, &t.args))
            .unwrap_or("step");
        match counts.iter_mut().find(|(k, _)| *k == kind) {
            Some((_, n)) => *n += 1,
            None => counts.push((kind, 1)),
        }
    }
    let failed = g
        .tools
        .iter()
        .filter(|t| t.status == ToolStatus::Error)
        .count();
    let glyph = if live {
        Span::styled(
            theme.glyphs.spinner[ctx.frame % theme.glyphs.spinner.len()],
            theme.accent(),
        )
    } else if failed == g.tools.len() {
        Span::styled(theme.glyphs.error, theme.err())
    } else {
        Span::styled(theme.glyphs.success, theme.ok())
    };
    let mut head = Line::from_spans(vec![glyph, Span::raw(" ")]);
    head.push(if live { "Exploring" } else { "Explored" }, Style::new());
    let summary = counts
        .iter()
        .map(|(k, n)| plural(*n, k))
        .collect::<Vec<_>>()
        .join(" · ");
    head.push(format!(" {summary}"), dim);
    if failed > 0 {
        head.push(format!(" · {failed} failed"), theme.warn());
    }
    if !live {
        if let Some(ms) = run_elapsed(g) {
            head.push(format!(" · {}", format_elapsed(ms)), dim);
        }
    }
    let mut out = vec![one_line(head, width)];
    if live {
        if let Some(t) = g
            .tools
            .iter()
            .rev()
            .find(|t| t.status == ToolStatus::Running)
            .or(g.tools.last())
        {
            let text = format!("{} {}", t.name, flat(&t.args));
            out.push(
                Line::styled("  └ ", dim)
                    .with(ellipsize_middle(&text, width.saturating_sub(4)), dim),
            );
        }
    }
    out
}

/// Wall time of a whole run: first start to last finish, once it is worth
/// showing.
fn run_elapsed(g: &GroupBlock) -> Option<u64> {
    let start = g.tools.iter().filter_map(|t| t.started_at_ms).min()?;
    let end = g
        .tools
        .iter()
        .filter_map(|t| Some(t.started_at_ms? + t.elapsed_ms? as i64))
        .max()?;
    (end - start >= 1000).then_some((end - start) as u64)
}

/// What a step in a named run says: its arguments, or its name when it has
/// none.
fn step_text(t: &ToolBlock) -> String {
    let args = flat(&t.args);
    if args.is_empty() {
        t.name.clone()
    } else {
        args
    }
}

/// A run of one tool (`Code · 25 steps`): the name appears once as a header
/// and each step hangs under it. Collapsed it is a single row; Ctrl+O lists
/// every step.
fn render_run(g: &GroupBlock, width: usize, theme: &Theme, ctx: RenderCtx) -> Vec<Line> {
    let dim = theme.dim();
    let label = g.label.as_deref().unwrap_or("Run");
    let live = !g.closed || g.tools.iter().any(|t| t.status == ToolStatus::Running);
    let failed = g
        .tools
        .iter()
        .filter(|t| t.status == ToolStatus::Error)
        .count();
    let glyph = if live {
        Span::styled(
            theme.glyphs.spinner[ctx.frame % theme.glyphs.spinner.len()],
            theme.accent(),
        )
    } else if failed == g.tools.len() {
        Span::styled(theme.glyphs.error, theme.err())
    } else {
        Span::styled(theme.glyphs.success, theme.ok())
    };
    let mut head = Line::from_spans(vec![glyph, Span::raw(" ")]);
    head.push(label, theme.strong());
    head.push(format!(" · {}", plural(g.tools.len(), "step")), dim);
    if failed > 0 {
        head.push(format!(" · {failed} failed"), theme.warn());
    }
    if !live {
        if let Some(ms) = run_elapsed(g) {
            head.push(format!(" · {}", format_elapsed(ms)), dim);
        }
    }
    let mut out = vec![one_line(head, width)];

    if ctx.expanded {
        let n = g.tools.len();
        for (i, t) in g.tools.iter().enumerate() {
            let mut line = Line::styled(if i + 1 == n { "  └ " } else { "  ├ " }, dim);
            let style = if t.status == ToolStatus::Error {
                theme.err()
            } else {
                Style::new()
            };
            line.push(step_text(t), style);
            if let Some(ms) = t.elapsed_ms.filter(|ms| *ms >= 1000) {
                line.push(format!(" · {}", format_elapsed(ms)), dim);
            }
            out.push(one_line(line, width));
        }
    } else if live {
        if let Some(t) = g
            .tools
            .iter()
            .rev()
            .find(|t| t.status == ToolStatus::Running)
            .or(g.tools.last())
        {
            out.push(Line::styled("  └ ", dim).with(
                ellipsize_middle(&step_text(t), width.saturating_sub(4)),
                dim,
            ));
        }
    }
    out
}

/// Args on one row: newlines shown as ⏎.
pub(crate) fn flat(s: &str) -> String {
    s.trim().replace('\n', " ⏎ ")
}

/// Cut a styled line to one row.
fn one_line(line: Line, width: usize) -> Line {
    let mut out = Line::new();
    let mut left = width;
    for sp in line.spans {
        if left == 0 {
            break;
        }
        let w = str_width(&sp.text);
        if w <= left {
            left -= w;
            out.spans.push(sp);
        } else {
            out.push(truncate(&sp.text, left), sp.style);
            left = 0;
        }
    }
    out
}

fn indent_lines(lines: Vec<Line>, n: usize) -> Vec<Line> {
    lines
        .into_iter()
        .map(|mut l| {
            if !l.spans.is_empty() {
                l.spans.insert(0, Span::raw(" ".repeat(n)));
            }
            l
        })
        .collect()
}

/// Split user text so `@mentions` show in the accent color.
fn mention_spans(text: &str, theme: &Theme) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut plain = String::new();
    for (i, word) in text.split(' ').enumerate() {
        if i > 0 {
            plain.push(' ');
        }
        if word.len() > 1 && word.starts_with('@') {
            if !plain.is_empty() {
                spans.push(Span::raw(std::mem::take(&mut plain)));
            }
            spans.push(Span::styled(word, theme.accent()));
        } else {
            plain.push_str(word);
        }
    }
    if !plain.is_empty() {
        spans.push(Span::raw(plain));
    }
    spans
}

fn render_user(text: &str, width: usize, theme: &Theme) -> Vec<Line> {
    let mut out = Vec::new();
    for (i, para) in text.split('\n').enumerate() {
        let mut line = Line::new();
        line.fill = theme.user_band;
        line.push(if i == 0 { "› " } else { "  " }, theme.dim());
        line.spans.extend(mention_spans(para, theme));
        let mut indent = Line::raw("  ");
        indent.fill = theme.user_band;
        out.extend(wrap(&line, width, &indent));
    }
    out
}

fn status_glyph(t: &ToolBlock, theme: &Theme, ctx: RenderCtx) -> Span {
    match t.status {
        ToolStatus::Running => Span::styled(
            theme.glyphs.spinner[ctx.frame % theme.glyphs.spinner.len()],
            theme.accent(),
        ),
        ToolStatus::Ok => Span::styled(theme.glyphs.success, theme.ok()),
        ToolStatus::Error => Span::styled(theme.glyphs.error, theme.err()),
        ToolStatus::Declined => Span::styled(theme.glyphs.declined, theme.warn()),
        ToolStatus::Stopped => Span::styled(theme.glyphs.code, theme.dim()),
    }
}

pub fn format_elapsed(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        format!("{s}s")
    } else {
        format!("{}m {:02}s", s / 60, s % 60)
    }
}

fn lang_for_path(path: &str) -> &str {
    path.rsplit('.').next().unwrap_or("")
}

/// A tool as one quiet row: `✓ Read src/a.ts · 2s`. Running tools add one
/// faint line of their latest output; failures show the end of theirs.
/// Ctrl+O (expanded) shows the full layout instead.
fn render_tool(t: &ToolBlock, width: usize, theme: &Theme, ctx: RenderCtx) -> Vec<Line> {
    if ctx.expanded {
        return render_tool_full(t, width, theme, ctx);
    }
    let dim = theme.dim();
    let elapsed = match t.status {
        ToolStatus::Running => t.started_at_ms.map(|s| (ctx.now_ms - s).max(0) as u64),
        _ => t.elapsed_ms,
    }
    .filter(|ms| *ms >= 1000);

    let mut suffix: Vec<Span> = Vec::new();
    let mut add = |text: String, style: Style| {
        suffix.push(Span::styled(" · ", dim));
        suffix.push(Span::styled(text, style));
    };
    match t.status {
        ToolStatus::Ok | ToolStatus::Running => {
            let counts = t
                .diff
                .as_ref()
                .map(|(_, d)| crate::diff::count_changes(d))
                .or_else(|| {
                    let s = t.summary.as_deref()?;
                    let tail = s.rsplit(" · ").next()?;
                    let (a, r) = tail.strip_prefix('+')?.split_once(" −")?;
                    Some((a.parse().ok()?, r.parse().ok()?))
                });
            if let Some((a, r)) = counts {
                suffix.push(Span::styled(" · ", dim));
                suffix.push(Span::styled(format!("+{a}"), theme.ok()));
                suffix.push(Span::styled(" ", dim));
                suffix.push(Span::styled(format!("−{r}"), theme.err()));
            }
        }
        ToolStatus::Error => add(
            t.summary.clone().unwrap_or_else(|| "failed".into()),
            theme.err(),
        ),
        ToolStatus::Declined => add(
            t.summary.clone().unwrap_or_else(|| "declined".into()),
            theme.warn(),
        ),
        ToolStatus::Stopped => add("stopped".into(), dim),
    }
    if let Some(ms) = elapsed {
        suffix.push(Span::styled(format!(" · {}", format_elapsed(ms)), dim));
    }

    let name_w = str_width(&t.name);
    let suffix_w: usize = suffix.iter().map(|s| str_width(&s.text)).sum();
    let args_room = width.saturating_sub(2 + name_w + 1 + suffix_w);
    let mut head = Line::from_spans(vec![status_glyph(t, theme, ctx), Span::raw(" ")]);
    head.push(t.name.clone(), Style::new());
    let args = flat(&t.args);
    if !args.is_empty() && args_room > 3 {
        head.push(" ", Style::new())
            .push(ellipsize_middle(&args, args_room - 1), dim);
    }
    head.spans.extend(suffix);
    let mut out = vec![one_line(head, width)];

    let output = trim_trailing_empty(&t.output);
    let room = width.saturating_sub(4);
    match t.status {
        ToolStatus::Running => {
            if let Some(last) = output.iter().rev().find(|l| !l.trim().is_empty()) {
                out.push(Line::styled("  └ ", dim).with(truncate(last.trim(), room), dim));
            }
        }
        ToolStatus::Error => {
            const TAIL: usize = 6;
            let shown = &output[output.len().saturating_sub(TAIL)..];
            if output.len() > shown.len() {
                out.push(Line::styled(
                    format!("    … {} earlier lines", output.len() - shown.len()),
                    dim,
                ));
            }
            for l in shown {
                out.push(Line::raw("    ").with(truncate(l.trim_end(), room), theme.err()));
            }
        }
        _ => {}
    }
    out
}

fn render_tool_full(t: &ToolBlock, width: usize, theme: &Theme, ctx: RenderCtx) -> Vec<Line> {
    let dim = theme.dim();
    let mut out = Vec::new();

    let mut head = Line::from_spans(vec![status_glyph(t, theme, ctx), Span::raw(" ")]);
    head.push(t.name.clone(), theme.strong());
    if !t.args.is_empty() {
        head.push(" ", Style::new())
            .push(t.args.clone(), Style::new());
    }
    out.extend(wrap(&head, width, &Line::raw("  ")));

    let sub = |text: Line| -> Vec<Line> {
        let mut l = Line::styled("  └ ", dim);
        l.spans.extend(text.spans);
        wrap(&l, width, &Line::raw("    "))
    };
    let out_indent = "    ";

    if t.status == ToolStatus::Running {
        let output = trim_trailing_empty(&t.output);
        if output.is_empty() {
            if let Some(start) = t.started_at_ms {
                let ms = (ctx.now_ms - start).max(0) as u64;
                out.extend(sub(Line::styled(format_elapsed(ms), dim)));
            }
        } else {
            for l in output.iter().rev().take(4).rev() {
                out.extend(wrap(
                    &Line::styled(format!("{out_indent}{l}"), dim),
                    width,
                    &Line::raw(out_indent),
                ));
            }
        }
        return out;
    }

    if let Some(summary) = &t.summary {
        let style = match t.status {
            ToolStatus::Error => theme.err(),
            ToolStatus::Declined => theme.warn(),
            _ => dim,
        };
        let mut l = Line::styled(summary.clone(), style);
        if let Some(ms) = t.elapsed_ms.filter(|ms| *ms >= 1000) {
            l.push(format!(" · {}", format_elapsed(ms)), dim);
        }
        out.extend(sub(l));
    }

    if let Some((path, diff)) = &t.diff {
        out.extend(render_diff(path, diff, width, theme));
    }

    let output = trim_trailing_empty(&t.output);
    if !output.is_empty() {
        let show = if ctx.expanded {
            output.len()
        } else {
            t.preview.min(output.len())
        };
        let highlighted = t
            .output_lang
            .as_deref()
            .map(|lang| highlight(&output[..show].join("\n"), lang));
        for (i, l) in output[..show].iter().enumerate() {
            let mut line = Line::raw(out_indent);
            match &highlighted {
                Some(h) => line.spans.extend(h.get(i).cloned().unwrap_or_default()),
                None if t.status == ToolStatus::Error => {
                    line.push(l.clone(), theme.err());
                }
                None => {
                    line.push(l.clone(), dim);
                }
            }
            out.extend(wrap(&line, width, &Line::raw(out_indent)));
        }
        let rest = output.len() - show;
        if rest > 0 {
            let noun = if rest == 1 { "line" } else { "lines" };
            let what = if show > 0 {
                format!("… +{rest} more {noun}")
            } else {
                format!("… {rest} {noun}")
            };
            out.push(Line::styled(
                format!("{out_indent}{what} (ctrl+o to expand)"),
                dim,
            ));
        }
    }
    out
}

fn trim_trailing_empty(lines: &[String]) -> &[String] {
    let mut end = lines.len();
    while end > 0 && lines[end - 1].is_empty() {
        end -= 1;
    }
    &lines[..end]
}

pub fn render_diff(path: &str, diff: &[DiffLine], width: usize, theme: &Theme) -> Vec<Line> {
    let lang = lang_for_path(path);
    let code = diff
        .iter()
        .map(|d| d.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let hl = highlight(&code, lang);
    let num_w = diff
        .iter()
        .filter_map(|d| d.line_no)
        .max()
        .map(|n| n.to_string().len())
        .unwrap_or(1)
        .max(3);
    let mut out = Vec::new();
    for (i, d) in diff.iter().enumerate() {
        let (sign, sign_style, fill) = match d.kind {
            DiffKind::Add => ("+", theme.ok(), theme.diff_add_bg),
            DiffKind::Del => ("-", theme.err(), theme.diff_del_bg),
            DiffKind::Context => (" ", theme.dim(), None),
        };
        let num = d.line_no.map(|n| n.to_string()).unwrap_or_default();
        let mut line = Line::raw("    ");
        line.fill = fill;
        line.push(format!("{num:>num_w$} "), theme.dim());
        line.push(format!("{sign} "), sign_style);
        line.spans.extend(
            hl.get(i)
                .cloned()
                .unwrap_or_else(|| vec![Span::raw(d.text.clone())]),
        );
        let mut indent = Line::raw(" ".repeat(4 + num_w + 3));
        indent.fill = fill;
        out.extend(wrap(&line, width, &indent));
    }
    out
}

fn render_plan(items: &[PlanItem], live: bool, width: usize, theme: &Theme) -> Vec<Line> {
    let done = items
        .iter()
        .filter(|i| i.status == PlanStatus::Done)
        .count();
    let mut head = Line::new();
    head.push(
        format!(
            "{} ",
            if live {
                theme.glyphs.plan
            } else {
                theme.glyphs.radio_off
            }
        ),
        if live { theme.accent() } else { theme.dim() },
    );
    head.push("Plan", theme.strong());
    head.push(
        format!(
            " · {done} of {} done{}",
            items.len(),
            if live { "" } else { " · stopped here" }
        ),
        theme.dim(),
    );
    let mut out = vec![head];
    for it in items {
        let (glyph, gstyle, tstyle) = match it.status {
            PlanStatus::Done => (theme.glyphs.success, theme.ok(), theme.dim().strike()),
            PlanStatus::InProgress => (theme.glyphs.shell, theme.accent(), theme.strong()),
            PlanStatus::Pending => (theme.glyphs.radio_off, theme.dim(), Style::new()),
        };
        let line = Line::raw("  ")
            .with(format!("{glyph} "), gstyle)
            .with(it.title.clone(), tstyle);
        out.extend(wrap(&line, width, &Line::raw("    ")));
    }
    out
}

/// Longest reasoning shown, like the Ink UI.
const REASONING_MAX: usize = 400;

fn render_reasoning(text: &str, width: usize, theme: &Theme) -> Vec<Line> {
    let text = text.trim();
    let shown: String = if text.chars().count() > REASONING_MAX {
        text.chars().take(REASONING_MAX).collect::<String>() + "…"
    } else {
        text.to_string()
    };
    let style = theme.dim().italic();
    let mut out = Vec::new();
    for (i, line) in shown.split('\n').enumerate() {
        let head = if i == 0 {
            Line::raw("  ").with("thinking… ", theme.dim())
        } else {
            Line::raw("  ")
        };
        out.extend(wrap(&head.with(line, style), width, &Line::raw("  ")));
    }
    out
}

fn render_notice(kind: NoticeKind, text: &str, width: usize, theme: &Theme) -> Vec<Line> {
    let (glyph, gstyle, tstyle) = match kind {
        NoticeKind::Info => ("· ", theme.dim(), Style::new()),
        NoticeKind::Success => ("✓ ", theme.ok(), Style::new()),
        NoticeKind::Warn => ("! ", theme.warn(), theme.warn()),
        NoticeKind::Error => ("✗ ", theme.err(), theme.err()),
        NoticeKind::Interrupted => ("  └ ", theme.err(), theme.err()),
    };
    let indent = " ".repeat(str_width(glyph));
    // Keep the host's line breaks (reports, lists); wrap each line under
    // the glyph.
    let mut out = Vec::new();
    for (i, line) in text.split('\n').enumerate() {
        let head = if i == 0 {
            Line::styled(glyph, gstyle)
        } else {
            Line::raw(indent.clone())
        };
        out.extend(wrap(
            &head.with(line, tstyle),
            width,
            &Line::raw(indent.clone()),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|l| l.plain_text()).collect()
    }

    fn ctx() -> RenderCtx {
        RenderCtx {
            frame: 0,
            now_ms: 10_000,
            expanded: false,
        }
    }

    #[test]
    fn user_prompt_is_a_band_with_a_dim_marker() {
        let t = Theme::default();
        let lines = render_block(
            &Block::User {
                text: "fix @src/a.ts please".into(),
            },
            40,
            &t,
            ctx(),
        );
        assert_eq!(text(&lines), vec!["› fix @src/a.ts please"]);
        assert_eq!(lines[0].fill, t.user_band);
        assert!(lines[0]
            .spans
            .iter()
            .any(|s| s.text == "@src/a.ts" && s.style.fg == t.accent));
    }

    #[test]
    fn assistant_text_is_indented_without_a_label() {
        let lines = render_block(
            &Block::Assistant {
                markdown: "Found it.".into(),
            },
            40,
            &Theme::default(),
            ctx(),
        );
        assert_eq!(text(&lines), vec!["  Found it."]);
    }

    #[test]
    fn finished_tool_is_one_quiet_row() {
        let mut t = ToolBlock::new("Read", "src/auth/login.ts");
        t.status = ToolStatus::Ok;
        t.summary = Some("142 lines".into());
        t.started_at_ms = Some(0);
        t.elapsed_ms = Some(2400);
        let lines = render_block(&Block::Tool(t.clone()), 60, &Theme::default(), ctx());
        assert_eq!(text(&lines), vec!["✓ Read src/auth/login.ts · 2s"]);
        let full = render_block(
            &Block::Tool(t),
            60,
            &Theme::default(),
            RenderCtx {
                expanded: true,
                ..ctx()
            },
        );
        assert_eq!(
            text(&full),
            vec!["✓ Read src/auth/login.ts", "  └ 142 lines · 2s"]
        );
    }

    #[test]
    fn long_commands_are_cut_in_the_middle() {
        let mut t = ToolBlock::new("Bash", "npx tsx --test src/server/aster-api.test.ts src/server/aster-store.test.ts src/server/aster-tools.test.ts");
        t.status = ToolStatus::Ok;
        let row = text(&render_block(&Block::Tool(t), 60, &Theme::default(), ctx()));
        assert_eq!(row.len(), 1);
        assert!(row[0].starts_with("✓ Bash npx tsx --test"), "{row:?}");
        assert!(row[0].ends_with("aster-tools.test.ts"), "{row:?}");
        assert!(str_width(&row[0]) <= 60);
    }

    #[test]
    fn failed_tool_shows_the_end_of_its_output() {
        let mut t = ToolBlock::new("Bash", "npm test");
        t.status = ToolStatus::Error;
        t.summary = Some("Exit code 1".into());
        t.output = (1..=9).map(|i| format!("line {i}")).collect();
        let lines = text(&render_block(&Block::Tool(t), 60, &Theme::default(), ctx()));
        assert_eq!(lines[0], "✗ Bash npm test · Exit code 1");
        assert_eq!(lines[1], "    … 3 earlier lines");
        assert_eq!(lines.last().unwrap(), "    line 9");
    }

    #[test]
    fn explore_groups_fold_into_one_row() {
        let tool = |name: &str, args: &str| {
            let mut t = ToolBlock::new(name, args);
            t.status = ToolStatus::Ok;
            t.started_at_ms = Some(0);
            t.elapsed_ms = Some(1500);
            t
        };
        let g = GroupBlock {
            tools: vec![
                tool("Read", "a.ts"),
                tool("Read", "b.ts"),
                tool("Grep", "foo"),
                tool("Glob", "*.rs"),
            ],
            closed: true,
            label: None,
        };
        assert_eq!(
            text(&render_block(
                &Block::Group(g.clone()),
                60,
                &Theme::default(),
                ctx()
            )),
            vec!["✓ Explored 2 files · 2 searches · 1s"]
        );
        let mut live = g;
        live.closed = false;
        live.tools[3].status = ToolStatus::Running;
        assert_eq!(
            text(&render_block(
                &Block::Group(live),
                60,
                &Theme::default(),
                ctx()
            )),
            vec!["⠋ Exploring 2 files · 2 searches", "  └ Glob *.rs"]
        );
    }

    #[test]
    fn read_only_shell_commands_count_as_exploration() {
        assert_eq!(shell_explore_kind("grep -rn foo src"), Some("search"));
        assert_eq!(
            shell_explore_kind("ls crates/sdk crates/sdk/src 2>/dev/null"),
            Some("folder")
        );
        assert_eq!(
            shell_explore_kind("sed -n '1440,1520p' crates/inline/src/session.rs"),
            Some("file")
        );
        assert_eq!(
            shell_explore_kind("ls sdk/node; echo ===; grep -rn \"turn\" crates/"),
            Some("search")
        );
        assert_eq!(
            shell_explore_kind("git -C repo log --oneline -3"),
            Some("command")
        );
        assert_eq!(shell_explore_kind("cat a.rs | head -20"), Some("file"));
        assert_eq!(shell_explore_kind("cd crates && rg settle"), Some("search"));
        // A row preview cut to fit the terminal keeps its first word.
        assert_eq!(
            shell_explore_kind("ls crates/sdk crates/sdk/src 2>/dev/null;…"),
            Some("folder")
        );
        assert_eq!(
            shell_explore_kind("grep -n 'cam\\.send(\\|sendEvent('…"),
            Some("search")
        );
        // A preview that hid whole lines can't be judged.
        assert_eq!(shell_explore_kind("cd repo … +2 lines"), None);
    }

    #[test]
    fn a_shell_prompt_prefix_is_not_part_of_the_command() {
        // autopilot's bash rows send "$ <command>" (formatBashTitle).
        assert_eq!(shell_explore_kind("$ grep -rn foo src"), Some("search"));
        assert_eq!(
            shell_explore_kind("$ ls crates/sdk crates/sdk/src 2>/dev/null;…"),
            Some("folder")
        );
        assert_eq!(shell_explore_kind("$ sed -n '1,9p' a.rs"), Some("file"));
        assert_eq!(shell_explore_kind("$ npm test"), None);
        assert_eq!(shell_explore_kind("$ cd repo … +2 lines"), None);
        // A path that starts with $ is not a prompt.
        assert_eq!(shell_explore_kind("$HOME/bin/tool --flag"), None);
    }

    #[test]
    fn read_only_host_tools_fold_too() {
        assert_eq!(explore_kind("Github_read_issue"), Some("page"));
        assert_eq!(explore_kind("Github_read_code"), Some("page"));
        assert_eq!(explore_kind("Browser_fetch"), Some("page"));
        assert_eq!(explore_kind("Memory_recall"), Some("lookup"));
        assert_eq!(explore_kind("Job_status"), Some("lookup"));
        assert_eq!(explore_kind("Github_list_releases"), Some("lookup"));
        // Actions keep their own row.
        assert_eq!(explore_kind("Github_create_pr"), None);
        assert_eq!(explore_kind("Edit"), None);
        assert_eq!(explore_kind("Subagent"), None);
    }

    #[test]
    fn commands_that_change_things_are_not_exploration() {
        for cmd in [
            "npm test",
            "cargo build",
            "rm -rf target",
            "sed -i 's/a/b/' a.rs",
            "echo hi > out.txt",
            "git commit -m x",
            "git branch new",
            "find . -delete",
            "curl https://example.com",
            "grep foo a.rs | xargs rm",
            "",
        ] {
            assert_eq!(shell_explore_kind(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn a_shell_search_joins_the_run_of_reads() {
        assert_eq!(explore_kind_of("Bash", "grep -rn foo src"), Some("search"));
        assert_eq!(explore_kind_of("Bash", "npm test"), None);
        assert_eq!(explore_kind_of("Read", "a.ts"), Some("file"));
    }

    #[test]
    fn a_run_of_shell_searches_folds_with_reads() {
        let tool = |name: &str, args: &str| {
            let mut t = ToolBlock::new(name, args);
            t.status = ToolStatus::Ok;
            t.started_at_ms = Some(0);
            t.elapsed_ms = Some(1500);
            t
        };
        let g = GroupBlock {
            tools: vec![
                tool("Read", "a.ts"),
                tool("Bash", "grep -rn foo src"),
                tool("Bash", "sed -n '1,9p' b.rs"),
            ],
            closed: true,
            label: None,
        };
        assert_eq!(
            text(&render_block(
                &Block::Group(g),
                60,
                &Theme::default(),
                ctx()
            )),
            vec!["✓ Explored 2 files · 1 search · 1s"]
        );
    }

    #[test]
    fn a_code_run_prints_its_name_once() {
        let step = |args: &str| {
            let mut t = ToolBlock::new("Code", args);
            t.status = ToolStatus::Ok;
            t.started_at_ms = Some(0);
            t.elapsed_ms = Some(1200);
            t
        };
        let g = GroupBlock {
            tools: vec![
                step("Read the prompts file."),
                step("Inspect worktrees."),
                step("Read pickModel."),
            ],
            closed: true,
            label: Some("Code".into()),
        };
        assert_eq!(
            text(&render_block(
                &Block::Group(g.clone()),
                60,
                &Theme::default(),
                ctx()
            )),
            vec!["✓ Code · 3 steps · 1s"]
        );
        assert_eq!(
            text(&render_block(
                &Block::Group(g),
                60,
                &Theme::default(),
                RenderCtx {
                    expanded: true,
                    ..ctx()
                }
            )),
            vec![
                "✓ Code · 3 steps · 1s",
                "  ├ Read the prompts file. · 1s",
                "  ├ Inspect worktrees. · 1s",
                "  └ Read pickModel. · 1s",
            ]
        );
    }

    #[test]
    fn a_live_code_run_shows_its_current_step() {
        let mut done = ToolBlock::new("Code", "Read the prompts file.");
        done.status = ToolStatus::Ok;
        done.started_at_ms = Some(0);
        done.elapsed_ms = Some(400);
        let mut running = ToolBlock::new("Code", "Inspect worktrees.");
        running.started_at_ms = Some(400);
        let g = GroupBlock {
            tools: vec![done, running],
            closed: false,
            label: Some("Code".into()),
        };
        assert_eq!(
            text(&render_block(
                &Block::Group(g),
                60,
                &Theme::default(),
                ctx()
            )),
            vec!["⠋ Code · 2 steps", "  └ Inspect worktrees."]
        );
    }

    #[test]
    fn a_single_code_step_stays_a_plain_row() {
        let mut t = ToolBlock::new("Code", "Read the prompts file.");
        t.status = ToolStatus::Ok;
        t.started_at_ms = Some(0);
        t.elapsed_ms = Some(1200);
        let g = GroupBlock {
            tools: vec![t],
            closed: true,
            label: Some("Code".into()),
        };
        assert_eq!(
            text(&render_block(
                &Block::Group(g),
                60,
                &Theme::default(),
                ctx()
            )),
            vec!["✓ Code Read the prompts file. · 1s"]
        );
    }

    #[test]
    fn a_failed_step_is_counted_in_the_header() {
        let mut ok = ToolBlock::new("Code", "Read the prompts file.");
        ok.status = ToolStatus::Ok;
        ok.started_at_ms = Some(0);
        ok.elapsed_ms = Some(400);
        let mut bad = ToolBlock::new("Code", "Inspect worktrees.");
        bad.status = ToolStatus::Error;
        bad.started_at_ms = Some(400);
        bad.elapsed_ms = Some(400);
        let g = GroupBlock {
            tools: vec![ok, bad],
            closed: true,
            label: Some("Code".into()),
        };
        assert_eq!(
            text(&render_block(
                &Block::Group(g),
                60,
                &Theme::default(),
                ctx()
            )),
            vec!["✓ Code · 2 steps · 1 failed"]
        );
    }

    #[test]
    fn running_tool_tails_its_output() {
        let mut t = ToolBlock::new("Bash", "npm test");
        t.output = (1..=6).map(|i| format!("line {i}")).collect();
        let lines = render_block(&Block::Tool(t), 60, &Theme::default(), ctx());
        assert_eq!(text(&lines), vec!["⠋ Bash npm test", "  └ line 6"]);
    }

    #[test]
    fn collapsed_output_offers_expansion() {
        let mut t = ToolBlock::new("Search", "\"issueSession\"");
        t.status = ToolStatus::Ok;
        t.summary = Some("4 matches".into());
        t.output = vec!["a".into(), "b".into(), "c".into(), "d".into()];
        let collapsed = text(&render_block(
            &Block::Tool(t.clone()),
            60,
            &Theme::default(),
            ctx(),
        ));
        assert_eq!(collapsed, vec!["✓ Search \"issueSession\""]);
        let expanded = text(&render_block(
            &Block::Tool(t),
            60,
            &Theme::default(),
            RenderCtx {
                expanded: true,
                ..ctx()
            },
        ));
        assert_eq!(expanded.len(), 2 + 4);
    }

    #[test]
    fn diffs_show_line_numbers_signs_and_tints() {
        let t = Theme::default();
        let diff = vec![
            DiffLine {
                line_no: Some(44),
                kind: DiffKind::Del,
                text: "// TODO".into(),
            },
            DiffLine {
                line_no: Some(44),
                kind: DiffKind::Add,
                text: "rotate()".into(),
            },
        ];
        let lines = render_diff("src/a.ts", &diff, 60, &t);
        assert_eq!(
            text(&lines),
            vec!["     44 - // TODO", "     44 + rotate()"]
        );
        assert_eq!(lines[0].fill, t.diff_del_bg);
        assert_eq!(lines[1].fill, t.diff_add_bg);
    }

    #[test]
    fn plan_marks_progress() {
        let items = vec![
            PlanItem {
                title: "Trace".into(),
                status: PlanStatus::Done,
            },
            PlanItem {
                title: "Rotate".into(),
                status: PlanStatus::InProgress,
            },
            PlanItem {
                title: "Test".into(),
                status: PlanStatus::Pending,
            },
        ];
        let lines = render_block(
            &Block::Plan { items, live: true },
            60,
            &Theme::default(),
            ctx(),
        );
        assert_eq!(
            text(&lines),
            vec![
                "◆ Plan · 1 of 3 done",
                "  ✓ Trace",
                "  › Rotate",
                "  ○ Test"
            ]
        );
    }

    #[test]
    fn notices_keep_line_breaks() {
        let lines = render_block(
            &Block::Notice {
                kind: NoticeKind::Info,
                text: "Session  $0.01\nToday    $0.48".into(),
            },
            60,
            &Theme::default(),
            ctx(),
        );
        assert_eq!(text(&lines), vec!["· Session  $0.01", "  Today    $0.48"]);
    }

    #[test]
    fn tool_rows_sit_together_but_prose_gets_a_gap() {
        let tool = Block::Tool(ToolBlock::new("Read", "a"));
        let prose = Block::Assistant {
            markdown: "x".into(),
        };
        assert!(!gap_between(&tool, &tool));
        assert!(gap_between(&prose, &tool));
    }
}
