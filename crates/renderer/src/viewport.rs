use uuid::Uuid;

/// Pure viewport state. Scrolling math lives here; no terminal calls.
///
/// Scroll model: when the user first scrolls away from the bottom we
/// snapshot the *visual line index* sitting at the bottom of the
/// viewport (`frozen_visual_bottom`). All subsequent up/down keystrokes
/// move `scroll_offset` (in visual lines, post-wrap). New streaming
/// rows pile up below the frozen bottom and stay invisible until the
/// user scrolls back down — at which point we drop the freeze and
/// resume auto-follow at the current latest.
#[derive(Debug, Clone)]
pub struct ViewportState {
    pub session_id: Uuid,
    pub viewport_height: u16,
    pub viewport_width: u16,
    /// Visual lines scrolled above the frozen bottom. 0 = sitting on
    /// the snapshot (= latest at the moment of the first scroll-up).
    /// Only meaningful when `frozen_visual_bottom` is `Some`.
    pub scroll_offset: i64,
    pub auto_follow: bool,
    pub visible_start_seq: i64,
    pub visible_end_seq: i64,
    /// Visual line index that should anchor the bottom of the viewport
    /// at the moment the user first scrolled away. `None` = auto-follow
    /// the live tail. Cleared by scroll_down past zero or jump_to_latest.
    pub frozen_visual_bottom: Option<i64>,
    /// Total visual lines spanned by the current transcript, updated by
    /// the draw layer each frame. `scroll_up` reads this when it has to
    /// freeze; the input loop has no width info on its own.
    pub last_total_visual: i64,
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
            frozen_visual_bottom: None,
            last_total_visual: 0,
        }
    }

    pub fn resize(&mut self, height: u16, width: u16) {
        self.viewport_height = height.max(1);
        self.viewport_width = width.max(1);
    }

    /// User pressed scroll-up. On the first press we snapshot the
    /// current latest visual line as the frozen bottom; subsequent
    /// presses just bump `scroll_offset` up. Draw clamps against the
    /// transcript's actual top each frame, so over-scrolling is a
    /// no-op visually.
    pub fn scroll_up(&mut self, lines: i64) {
        if self.frozen_visual_bottom.is_none() {
            self.frozen_visual_bottom = Some(self.last_total_visual);
        }
        self.scroll_offset += lines;
        self.auto_follow = false;
    }

    /// User pressed scroll-down. Decrements `scroll_offset` toward 0,
    /// and *past* 0 into negative territory — a negative offset means
    /// the view sits *below* the frozen bottom (i.e., scrolling through
    /// content that streamed in during the freeze). The draw layer
    /// clamps the result against the live tail and unfreezes only when
    /// the view actually catches up — that prevents the "page-jump"
    /// you'd otherwise see when offset crosses zero and we snapped to
    /// the (possibly far-away) current latest.
    pub fn scroll_down(&mut self, lines: i64) {
        if self.frozen_visual_bottom.is_none() {
            // Already at the live tail; nothing to scroll into.
            return;
        }
        self.scroll_offset -= lines;
        // Note: the draw layer is responsible for clearing the freeze
        // once the view has caught up to the live tail (it has the
        // current total_visual). Doing it here would either snap or
        // require duplicating that math.
    }

    /// Jump to latest and resume auto-follow.
    pub fn jump_to_latest(&mut self) {
        self.scroll_offset = 0;
        self.frozen_visual_bottom = None;
        self.auto_follow = true;
    }

    /// True iff currently pinned to the bottom.
    pub fn at_bottom(&self) -> bool {
        self.frozen_visual_bottom.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_up_disables_follow() {
        let mut v = ViewportState::new(Uuid::nil(), 10, 80);
        v.last_total_visual = 100;
        assert!(v.auto_follow);
        v.scroll_up(3);
        assert!(!v.auto_follow);
        assert_eq!(v.scroll_offset, 3);
        assert_eq!(v.frozen_visual_bottom, Some(100));
    }

    #[test]
    fn scroll_down_decrements_offset() {
        // viewport itself just tracks state; the draw layer is what
        // ultimately unfreezes when the view catches up to the live
        // tail. Here we verify the bookkeeping.
        let mut v = ViewportState::new(Uuid::nil(), 10, 80);
        v.last_total_visual = 100;
        v.scroll_up(5);
        assert_eq!(v.scroll_offset, 5);
        v.scroll_down(3);
        assert_eq!(v.scroll_offset, 2);
        v.scroll_down(10); // offset can go negative while frozen
        assert_eq!(v.scroll_offset, -8);
        assert_eq!(v.frozen_visual_bottom, Some(100));
    }

    #[test]
    fn scroll_down_while_unfrozen_is_noop() {
        let mut v = ViewportState::new(Uuid::nil(), 10, 80);
        v.last_total_visual = 100;
        v.scroll_down(10);
        assert_eq!(v.scroll_offset, 0);
        assert!(v.auto_follow);
        assert_eq!(v.frozen_visual_bottom, None);
    }

    #[test]
    fn jump_to_latest_resets() {
        let mut v = ViewportState::new(Uuid::nil(), 10, 80);
        v.last_total_visual = 200;
        v.scroll_up(50);
        assert!(!v.auto_follow);
        v.jump_to_latest();
        assert!(v.auto_follow);
        assert_eq!(v.scroll_offset, 0);
        assert_eq!(v.frozen_visual_bottom, None);
    }

    #[test]
    fn freeze_stays_pinned_across_streaming() {
        // Once frozen, more content streaming in shouldn't change the
        // frozen anchor — the user's view stays at the snapshot.
        let mut v = ViewportState::new(Uuid::nil(), 10, 80);
        v.last_total_visual = 50;
        v.scroll_up(5);
        assert_eq!(v.frozen_visual_bottom, Some(50));
        // Simulate streaming: total_visual grows.
        v.last_total_visual = 90;
        // Another scroll-up shouldn't re-freeze.
        v.scroll_up(2);
        assert_eq!(v.frozen_visual_bottom, Some(50));
        assert_eq!(v.scroll_offset, 7);
    }
}
