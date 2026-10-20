//! Compact, bounded live state for a busy turn.
pub use crate::anim::FRAME_MS;
use crate::anim::{self, FadeStep};
use crate::blocks::{DiffLine, PlanItem, PlanStatus, RenderCtx};
use crate::style::{Line, Style};
use crate::theme::Theme;
use crate::width::{ellipsize_middle, str_width, truncate};
use std::collections::{HashMap, VecDeque};

const MAX_TRAIL_BEADS: usize = 32;
const MAX_CODE_PREVIEWS: usize = 4;
pub(crate) const MAX_CODE_PREVIEW_LINES: usize = 96;
pub(crate) const MAX_CODE_PREVIEW_CHARS: usize = 512;
pub(crate) const MAX_CODE_PREVIEW_PATH_CHARS: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    pub fn from_tool(name: &str) -> Self {
        let n = name.to_ascii_lowercase();
        match n.as_str() {
            "read" | "view" | "cat" | "open" | "read_file" | "read_file_contents" => Self::Read,
            "grep" | "glob" | "find" | "search" | "rg" | "codesearch" | "code_search"
            | "web_search" | "websearch" | "ls" | "list" | "list_dir" | "tree" | "web_fetch"
            | "webfetch" | "fetch" => Self::Search,
            x if x.starts_with("lsp_") => Self::Search,
            "edit" | "write" | "multiedit" | "multi_edit" | "notebook_edit" | "write_file" => {
                Self::Edit
            }
            "bash" | "shell" => Self::Bash,
            "execute_code" | "code" => Self::Code,
            "task" | "agent" => Self::Agent,
            x if x.starts_with("subagent") || x.starts_with("mcp_") => Self::Agent,
            _ => Self::Read,
        }
    }
    pub fn glyph(self, g: &crate::glyphs::Glyphs) -> &'static str {
        match self {
            Self::Read | Self::Search => g.read,
            Self::Edit => g.edit,
            Self::Bash => g.shell,
            Self::Agent => g.agent,
            Self::Code => g.code,
            Self::Error => g.error,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Bead {
    pub kind: TrailKind,
    pub at: i64,
    pub ok: bool,
}
#[derive(Clone, Debug)]
pub struct LatestAction {
    pub text: String,
    pub at: i64,
    pub is_error: bool,
}

#[derive(Clone, Debug)]
struct CodePreview {
    path: String,
    diff: Vec<DiffLine>,
    at: i64,
}
#[derive(Clone, Debug)]
pub struct CompactPlanStep {
    pub title: String,
    pub status: PlanStatus,
    pub done_at: Option<i64>,
}
#[derive(Clone, Debug)]
pub struct Agent {
    pub n: u32,
    pub title: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub ok: bool,
}
#[derive(Clone, Debug, Default)]
pub struct CompactState {
    pub since: Option<i64>,
    pub beads: Vec<Bead>,
    pub by_tool: HashMap<String, usize>,
    pub latest: Option<LatestAction>,
    pub plan: Vec<CompactPlanStep>,
    pub agents: Vec<Agent>,
    pub files: usize,
    code_previews: VecDeque<CodePreview>,
}
impl CompactState {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn begin(&mut self, now: i64) {
        self.clear();
        self.since = Some(now);
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn push_code_preview(&mut self, path: &str, diff: &[DiffLine], now: i64) {
        if diff.is_empty() {
            return;
        }
        self.code_previews.push_back(CodePreview {
            path: path.chars().take(MAX_CODE_PREVIEW_PATH_CHARS).collect(),
            diff: diff
                .iter()
                .take(MAX_CODE_PREVIEW_LINES)
                .cloned()
                .map(|mut line| {
                    line.text = line.text.chars().take(MAX_CODE_PREVIEW_CHARS).collect();
                    line
                })
                .collect(),
            at: now,
        });
        while self.code_previews.len() > MAX_CODE_PREVIEWS {
            self.code_previews.pop_front();
        }
    }
    fn push_bead(&mut self, kind: TrailKind, at: i64) -> usize {
        if self.beads.len() == MAX_TRAIL_BEADS {
            self.beads.remove(0);
            self.by_tool.retain(|_, index| *index > 0);
            for index in self.by_tool.values_mut() {
                *index -= 1;
            }
        }
        let index = self.beads.len();
        self.beads.push(Bead { kind, at, ok: true });
        index
    }
    pub fn tool_started(&mut self, id: &str, name: &str, title: &str, now: i64) {
        let kind = TrailKind::from_tool(name);
        let index = self.push_bead(kind, now);
        self.by_tool.insert(id.to_string(), index);
        let text = crate::blocks::flat(if title.trim().is_empty() { name } else { title });
        self.latest = Some(LatestAction {
            text,
            at: now,
            is_error: false,
        });
    }
    pub fn tool_finished(
        &mut self,
        id: &str,
        ok: bool,
        failure: Option<&str>,
        _path: Option<&str>,
        has_diff: bool,
        now: i64,
    ) {
        if let Some(i) = self.by_tool.remove(id) {
            if let Some(b) = self.beads.get_mut(i) {
                if !ok {
                    b.kind = TrailKind::Error;
                }
                b.ok = ok;
                if ok && b.kind == TrailKind::Edit && has_diff {
                    self.files += 1;
                }
            }
        }
        if !ok {
            let text = failure
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("Tool failed");
            self.latest = Some(LatestAction {
                text: crate::blocks::flat(text),
                at: now,
                is_error: true,
            });
        }
    }
    pub fn agent_start(&mut self, n: u32, title: &str, now: i64) {
        self.agents.push(Agent {
            n,
            title: crate::blocks::flat(title),
            started_at: now,
            finished_at: None,
            ok: true,
        });
        self.push_bead(TrailKind::Agent, now);
    }
    pub fn agent_done(&mut self, n: u32, ok: bool, now: i64) {
        if let Some(a) = self
            .agents
            .iter_mut()
            .find(|a| a.n == n && a.finished_at.is_none())
        {
            a.finished_at = Some(now);
            a.ok = ok;
        }
    }
    pub fn plan(&mut self, items: &[PlanItem]) {
        self.plan = items
            .iter()
            .map(|i| CompactPlanStep {
                title: crate::blocks::flat(&i.title),
                status: i.status,
                done_at: None,
            })
            .collect();
        if !self.plan.iter().any(|p| p.status == PlanStatus::InProgress) {
            if let Some(p) = self
                .plan
                .iter_mut()
                .find(|p| p.status == PlanStatus::Pending)
            {
                p.status = PlanStatus::InProgress;
            }
        }
    }
    pub fn plan_step_done(&mut self, now: i64) {
        if let Some(i) = self
            .plan
            .iter()
            .position(|p| p.status == PlanStatus::InProgress)
        {
            self.plan[i].status = PlanStatus::Done;
            self.plan[i].done_at = Some(now);
            if i + 1 < self.plan.len() {
                self.plan[i + 1].status = PlanStatus::InProgress;
            }
        }
    }
    /// Returns at most four compact rows. The session appends the common spinner.
    pub fn render(&self, ctx: RenderCtx, width: usize, theme: &Theme) -> Vec<Line> {
        if self.since.is_none() {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(4);
        if !self.plan.is_empty() {
            out.push(render_plan(self, ctx, width, theme));
        }
        if let Some(l) = render_agents(self, ctx, width, theme) {
            out.push(l);
        }
        if !self.beads.is_empty() {
            out.push(render_trail(self, ctx, width, theme));
        }
        if let Some(a) = &self.latest {
            let age = age_frames(ctx, self.since.unwrap_or(a.at), a.at) as i64 * FRAME_MS;
            if a.is_error || anim::fade(age) != FadeStep::Gone {
                out.push(render_latest(a, age, width, theme));
            }
        }
        out.extend(render_code_glimpse(self, ctx, width, theme));
        out
    }
}
fn render_code_glimpse(
    state: &CompactState,
    ctx: RenderCtx,
    width: usize,
    theme: &Theme,
) -> Vec<Line> {
    const CYCLE_MS: i64 = 9_000;
    const VISIBLE_MS: i64 = 6_200;
    const LINES_PER_GLIMPSE: usize = 4;

    let Some(latest) = state.code_previews.back() else {
        return Vec::new();
    };
    let age = ctx.now_ms.saturating_sub(latest.at).max(0);
    if age.rem_euclid(CYCLE_MS) >= VISIBLE_MS {
        return Vec::new();
    }

    let count = state.code_previews.len();
    let rotation = (age / CYCLE_MS) as usize;
    let preview_index = count.saturating_sub(1).saturating_sub(rotation % count);
    let preview = &state.code_previews[preview_index];
    let sections = preview.diff.len().div_ceil(LINES_PER_GLIMPSE).max(1);
    let section = ((age % CYCLE_MS) / (CYCLE_MS / 2)) as usize % sections;
    let start = section * LINES_PER_GLIMPSE;
    let end = (start + LINES_PER_GLIMPSE).min(preview.diff.len());

    let mut title = Line::raw("  ");
    title.push("Code glimpse", theme.accent().bold());
    title.push(" · ", theme.dim());
    title.push(
        crate::width::truncate(&preview.path, width.saturating_sub(18)),
        theme.dim(),
    );
    let section = &preview.diff[start..end];
    let number_width = section
        .iter()
        .filter_map(|line| line.line_no)
        .max()
        .map(|number| number.to_string().len())
        .unwrap_or(1)
        .max(3);
    let text_width = width.saturating_sub(4 + number_width + 3).max(1);
    let clipped: Vec<_> = section
        .iter()
        .cloned()
        .map(|mut line| {
            line.text = truncate(&line.text, text_width);
            line
        })
        .collect();
    let mut out = vec![title];
    out.extend(crate::blocks::render_diff(
        &preview.path,
        &clipped,
        width,
        theme,
    ));
    out
}
fn safe_text(s: &str, theme: &Theme) -> String {
    let flat = crate::blocks::flat(s).replace(" ⏎ ", " ");
    if theme.glyphs.ascii {
        flat.chars()
            .map(|c| if c.is_ascii() { c } else { '?' })
            .collect()
    } else {
        flat
    }
}
fn age_frames(ctx: RenderCtx, since: i64, at: i64) -> u64 {
    (ctx.frame as u64).saturating_sub(anim::frame_for(at, since))
}

fn clipped(s: &str, max: usize, theme: &Theme) -> String {
    let s = safe_text(s, theme);
    let s = truncate(&s, max);
    if theme.glyphs.ascii {
        s.replace('…', ".")
    } else {
        s
    }
}
fn render_plan(state: &CompactState, ctx: RenderCtx, width: usize, theme: &Theme) -> Line {
    let mut l = Line::new();
    let done = state
        .plan
        .iter()
        .filter(|p| p.status == PlanStatus::Done)
        .count();
    let count = format!(" {done}/{}", state.plan.len());
    l.push(theme.glyphs.plan, theme.accent());
    l.push(" Plan ", Style::default().bold());
    let title_reserve = if width >= 60 { 2 } else { 0 };
    let segment_room = width.saturating_sub(l.width() + str_width(&count) + title_reserve);
    let hidden = state.plan.len() > segment_room;
    let segment_limit = segment_room.saturating_sub(usize::from(hidden));
    for p in state.plan.iter().take(segment_limit) {
        match p.status {
            PlanStatus::Done => {
                let fresh = p
                    .done_at
                    .is_some_and(|at| age_frames(ctx, state.since.unwrap_or(at), at) < 8);
                l.push(
                    theme.glyphs.progress_done,
                    if fresh {
                        theme.bright().bold()
                    } else {
                        theme.accent()
                    },
                );
            }
            PlanStatus::InProgress => {
                let bright = anim::breathe(ctx.frame as u64, 6) >= 2;
                l.push(
                    theme.glyphs.progress_active,
                    if bright {
                        theme.bright().bold()
                    } else {
                        theme.accent()
                    },
                );
            }
            PlanStatus::Pending => {
                l.push(theme.glyphs.progress_pending, theme.dimmer());
            }
        }
    }
    if hidden {
        l.push(theme.glyphs.ellipsis, theme.dim());
    }
    l.push(count, theme.dim());
    if width >= 60 && !hidden {
        if let Some(p) = state
            .plan
            .iter()
            .find(|p| p.status == PlanStatus::InProgress)
        {
            l.push("  ", Style::default());
            let avail = width.saturating_sub(l.width());
            if avail > 0 {
                l.push(clipped(&p.title, avail, theme), Style::default());
            }
        }
    }
    l
}
fn render_agents(
    state: &CompactState,
    ctx: RenderCtx,
    width: usize,
    theme: &Theme,
) -> Option<Line> {
    if state.agents.is_empty() {
        return None;
    }
    let running: Vec<&Agent> = state
        .agents
        .iter()
        .filter(|a| a.finished_at.is_none())
        .collect();
    let since = state.since.unwrap_or(0);
    let visible: Vec<&Agent> = state
        .agents
        .iter()
        .filter(|a| {
            a.finished_at
                .is_some_and(|t| age_frames(ctx, since, t) < 31)
        })
        .collect();
    let folded = state
        .agents
        .iter()
        .filter(|a| {
            a.finished_at
                .is_some_and(|t| age_frames(ctx, since, t) >= 31)
        })
        .count();
    if running.is_empty()
        && visible.is_empty()
        && state.agents.iter().all(|a| {
            a.finished_at
                .is_some_and(|t| age_frames(ctx, since, t) >= 88)
        })
    {
        return None;
    }
    let mut l = Line::new();
    let n = if running.is_empty() {
        state.agents.len()
    } else {
        running.len()
    };
    l.push(theme.glyphs.agent, theme.ok());
    l.push(
        format!(" {n} agent{}", if n == 1 { "" } else { "s" }),
        Style::default().bold(),
    );
    for (i, a) in running.iter().enumerate() {
        if l.width() + 5 >= width {
            break;
        }
        let si = anim::cycle(
            ctx.frame as u64 + i as u64 * 2,
            2,
            theme.glyphs.spinner.len(),
        );
        l.push(
            format!(" {} #{}", theme.glyphs.spinner[si], a.n),
            theme.ok(),
        );
        if width >= 60 {
            l.push(" ", Style::default());
            let title = safe_text(&a.title, theme);
            let available = width.saturating_sub(l.width()).saturating_sub(6);
            if available > 0 {
                l.push(clipped(&title, available, theme), theme.ok());
            }
        }
        let e = crate::blocks::format_elapsed((ctx.now_ms - a.started_at).max(0) as u64);
        if l.width() + str_width(&e) < width {
            l.push(format!(" {e}"), theme.dim());
        }
    }
    for a in visible {
        if l.width() + 5 >= width {
            break;
        }
        let done = a.finished_at.unwrap_or(ctx.now_ms);
        let style = if age_frames(ctx, since, done) < 11 {
            theme.bright().bold()
        } else {
            theme.dim()
        };
        l.push(format!(" {} #{}", theme.glyphs.success, a.n), style);
        if width >= 60 {
            l.push(" ", Style::default());
            let available = width.saturating_sub(l.width()).saturating_sub(6);
            if available > 0 {
                l.push(clipped(&a.title, available, theme), style);
            }
        }
        let e = crate::blocks::format_elapsed((done - a.started_at).max(0) as u64);
        if l.width() + str_width(&e) < width {
            l.push(format!(" {e}"), theme.dim());
        }
    }
    if folded > 0 && l.width() + 8 < width {
        l.push(
            format!(" {}{folded} back", theme.glyphs.success),
            theme.dim(),
        );
    }
    Some(l)
}
fn render_trail(state: &CompactState, ctx: RenderCtx, width: usize, theme: &Theme) -> Line {
    let mut l = Line::new();
    l.push("  ", Style::default());
    let cap = if width < 40 {
        width.saturating_sub(4)
    } else {
        width.saturating_sub(2)
    };
    let skip = state.beads.len().saturating_sub(cap);
    if skip > 0 {
        l.push(theme.glyphs.ellipsis, theme.dim());
    }
    for b in state.beads.iter().skip(skip) {
        if l.width() >= width {
            break;
        }
        let age = age_frames(ctx, state.since.unwrap_or(b.at), b.at);
        let style = if !b.ok {
            theme.err()
        } else if age < 4 {
            theme.bright().bold()
        } else {
            kind_color(b.kind, theme)
        };
        l.push(b.kind.glyph(&theme.glyphs), style);
    }
    l
}
fn render_latest(a: &LatestAction, age_ms: i64, width: usize, theme: &Theme) -> Line {
    let mut l = Line::new();
    l.push("  ", Style::default());
    let text = safe_text(&a.text, theme);
    let max = width.saturating_sub(l.width());
    let visible = if str_width(&text) > max {
        ellipsize_middle(&text, max)
    } else {
        text
    };
    let visible = if theme.glyphs.ascii {
        visible.replace('…', ".")
    } else {
        visible
    };
    let style = if a.is_error {
        theme.err()
    } else {
        match anim::fade(age_ms) {
            FadeStep::Bright => theme.bright().bold(),
            FadeStep::Normal => Style::default(),
            FadeStep::Dim => theme.dim(),
            FadeStep::Dimmer | FadeStep::Gone => theme.dimmer(),
        }
    };
    l.push(visible, style);
    l
}
fn kind_color(k: TrailKind, t: &Theme) -> Style {
    match k {
        TrailKind::Read | TrailKind::Search => t.dim(),
        TrailKind::Edit | TrailKind::Code => t.accent(),
        TrailKind::Bash => t.info(),
        TrailKind::Agent => t.ok(),
        TrailKind::Error => t.err(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tool_finish_updates_one_bead_and_counts_diff() {
        let mut s = CompactState::new();
        s.begin(1000);
        s.tool_started("e", "Edit", "Edit src/a.rs", 1100);
        s.tool_finished("e", true, None, Some("src/a.rs"), true, 1200);
        assert_eq!(s.beads.len(), 1);
        assert_eq!(s.files, 1);
        assert_eq!(s.latest.as_ref().unwrap().text, "Edit src/a.rs");
    }
    #[test]
    fn failed_tool_and_read_are_visible() {
        let mut s = CompactState::new();
        s.begin(1000);
        s.tool_started("r", "Read", "Read file.rs", 1000);
        s.tool_finished("r", true, None, None, false, 1100);
        s.tool_started("b", "Bash", "npm test", 1200);
        s.tool_finished("b", false, Some("Exit code 1"), None, false, 1400);
        assert_eq!(s.beads.len(), 2);
        assert_eq!(s.beads[0].kind, TrailKind::Read);
        assert_eq!(s.beads[1].kind, TrailKind::Error);
        assert!(s.latest.as_ref().unwrap().is_error);
    }
    #[test]
    fn busy_region_is_bounded_and_ascii_fallback_is_ascii() {
        let theme = Theme::default().with_glyphs(crate::glyphs::Glyphs::ascii());
        let mut state = CompactState::new();
        state.begin(1_000);
        let plan: Vec<PlanItem> = (0..50)
            .map(|i| PlanItem {
                title: format!("step {i} with an intentionally long title"),
                status: if i == 0 {
                    PlanStatus::InProgress
                } else {
                    PlanStatus::Pending
                },
            })
            .collect();
        state.plan(&plan);
        state.agent_start(1, "Survey approvals", 1_000);
        for i in 0..100 {
            state.tool_started(&format!("t{i}"), "Bash", "npm test\n--watch", 1_000 + i);
        }
        let ctx = RenderCtx {
            frame: 2,
            now_ms: 2_000,
            expanded: false,
        };
        let mut lines = state.render(ctx, 38, &theme);
        lines.push(crate::chrome::spinner_line(
            &theme,
            ctx.frame,
            "Writing",
            1_000,
            Some(4),
            38,
        ));
        assert_eq!(lines.len(), 5);
        for line in &lines {
            assert!(
                line.width() <= 38,
                "{}: {:?}",
                line.width(),
                line.plain_text()
            );
            assert!(!line.plain_text().contains('\n'));
            assert!(line.plain_text().is_ascii(), "{:?}", line.plain_text());
        }
    }

    #[test]
    fn tool_trail_and_index_remain_bounded_during_long_turn() {
        let mut state = CompactState::new();
        state.begin(0);
        for i in 0..(MAX_TRAIL_BEADS + 100) {
            state.tool_started(&format!("tool-{i}"), "Read", "Read src/lib.rs", i as i64);
        }
        assert_eq!(state.beads.len(), MAX_TRAIL_BEADS);
        assert_eq!(state.by_tool.len(), MAX_TRAIL_BEADS);
        assert!(!state.by_tool.contains_key("tool-0"));
        assert!(state
            .by_tool
            .contains_key(&format!("tool-{}", MAX_TRAIL_BEADS + 99)));
        assert!(state.by_tool.values().all(|i| *i < MAX_TRAIL_BEADS));

        state.tool_finished(
            &format!("tool-{}", MAX_TRAIL_BEADS + 99),
            false,
            Some("failed"),
            None,
            false,
            200,
        );
        assert_eq!(state.beads.last().unwrap().kind, TrailKind::Error);
    }

    #[test]
    fn width_and_newlines_are_bounded() {
        let mut s = CompactState::new();
        s.begin(1000);
        s.plan(&[PlanItem {
            title: "a long implementation plan step".into(),
            status: PlanStatus::InProgress,
        }]);
        s.tool_started("x", "Bash", "npm test\n--watch", 1000);
        for width in [80, 38] {
            for l in s.render(
                RenderCtx {
                    frame: 1,
                    now_ms: 1100,
                    expanded: false,
                },
                width,
                &Theme::default(),
            ) {
                assert!(l.width() <= width, "{} {:?}", l.width(), l.plain_text());
                assert!(!l.plain_text().contains('\n'));
            }
        }
    }
}
