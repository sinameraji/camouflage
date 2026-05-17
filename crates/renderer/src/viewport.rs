use uuid::Uuid;

/// Pure viewport state. Scrolling math lives here; no terminal calls.
#[derive(Debug, Clone)]
pub struct ViewportState {
    pub session_id: Uuid,
    pub viewport_height: u16,
    pub viewport_width: u16,
    /// 0 == pinned to the latest row. Positive == lines scrolled up from bottom.
    pub scroll_offset: i64,
    pub auto_follow: bool,
    pub visible_start_seq: i64,
    pub visible_end_seq: i64,
    /// "Pin to where you scrolled to" snapshot of `total_rows` at the
    /// moment the user first scrolled away from the bottom. While set,
    /// the visible window is anchored to `frozen_total - scroll_offset`
    /// rather than `current_total - scroll_offset`, so new streaming
    /// rows don't push the user's reading position around. Cleared on
    /// scroll-to-bottom or jump-to-latest.
    pub frozen_total: Option<i64>,
}

impl ViewportState {
    pub fn new(session_id: Uuid, height: u16, width: u16) -> Self {
        Self {
            session_id,
            viewport_height: height,
            viewport_width: width,
            scroll_offset: 0,
            auto_follow: true,
            visible_start_seq: 0,
            visible_end_seq: 0,
            frozen_total: None,
        }
    }

    pub fn resize(&mut self, height: u16, width: u16) {
        self.viewport_height = height.max(1);
        self.viewport_width = width.max(1);
    }

    /// Move viewport up (toward older rows). Disables auto-follow.
    /// Caps at `total_rows` to avoid scrolling past the start, but does
    /// NOT subtract viewport_height — rows often wrap into multiple
    /// visual lines, so a row-based cap leaves a lot of real content
    /// unscrollable. The draw layer handles a "scrolled past the top"
    /// state gracefully (just shows fewer rows).
    pub fn scroll_up(&mut self, lines: i64, total_rows: i64) {
        // Freeze the current total the first time the user scrolls
        // away from the bottom. The cap then stays anchored to that
        // snapshot — new rows arriving mid-scroll don't shift what's
        // visible (fixes "I can't read the model's earlier output
        // because each new token bumps it down").
        if self.frozen_total.is_none() {
            self.frozen_total = Some(total_rows);
        }
        let max_up = self.frozen_total.unwrap_or(total_rows).max(0);
        self.scroll_offset = (self.scroll_offset + lines).min(max_up);
        if self.scroll_offset > 0 {
            self.auto_follow = false;
        }
    }

    /// Move viewport down (toward newer rows). Re-engages follow at bottom.
    pub fn scroll_down(&mut self, lines: i64) {
        self.scroll_offset = (self.scroll_offset - lines).max(0);
        if self.scroll_offset == 0 {
            self.auto_follow = true;
            self.frozen_total = None;
        }
    }

    /// Jump to latest and resume auto-follow.
    pub fn jump_to_latest(&mut self) {
        self.scroll_offset = 0;
        self.auto_follow = true;
        self.frozen_total = None;
    }

    /// True iff currently pinned to the bottom.
    pub fn at_bottom(&self) -> bool {
        self.scroll_offset == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_up_disables_follow() {
        let mut v = ViewportState::new(Uuid::nil(), 10, 80);
        assert!(v.auto_follow);
        v.scroll_up(3, 100);
        assert!(!v.auto_follow);
        assert_eq!(v.scroll_offset, 3);
    }

    #[test]
    fn scroll_down_to_bottom_reenables_follow() {
        let mut v = ViewportState::new(Uuid::nil(), 10, 80);
        v.scroll_up(5, 100);
        v.scroll_down(5);
        assert_eq!(v.scroll_offset, 0);
        assert!(v.auto_follow);
    }

    #[test]
    fn jump_to_latest_resets() {
        let mut v = ViewportState::new(Uuid::nil(), 10, 80);
        v.scroll_up(50, 200);
        assert!(!v.auto_follow);
        v.jump_to_latest();
        assert!(v.auto_follow);
        assert_eq!(v.scroll_offset, 0);
    }

    #[test]
    fn scroll_clamped_to_history() {
        let mut v = ViewportState::new(Uuid::nil(), 10, 80);
        // Cap is now `total_rows` (not `total_rows - height`): row-based
        // height-subtraction left wrapped content unscrollable (real bug
        // surfaced in --ui camouflage testing).
        v.scroll_up(1_000_000, 20);
        assert_eq!(v.scroll_offset, 20);
    }
}
