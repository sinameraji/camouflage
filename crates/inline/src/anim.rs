//! Shared frame-indexed animation primitives.
pub const FRAME_MS: i64 = 80;
pub const FADE_BRIGHT_MS: i64 = 4 * FRAME_MS;
pub const FADE_NORMAL_MS: i64 = 28 * FRAME_MS;
pub const FADE_DIM_MS: i64 = 40 * FRAME_MS;
pub const FADE_GONE_MS: i64 = 52 * FRAME_MS;
pub fn frame_for(now: i64, since: i64) -> u64 {
    (now.saturating_sub(since).max(0) / FRAME_MS) as u64
}
pub fn cycle(frame: u64, period_frames: u64, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        (frame / period_frames.max(1) % len as u64) as usize
    }
}
pub fn breathe(frame: u64, period_frames: u64) -> u64 {
    let p = period_frames.max(2);
    let at = frame % p;
    at.min(p - at)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FadeStep {
    Bright,
    Normal,
    Dim,
    Dimmer,
    Gone,
}
pub fn fade(age_ms: i64) -> FadeStep {
    let age = age_ms.max(0);
    if age < FADE_BRIGHT_MS {
        FadeStep::Bright
    } else if age < FADE_NORMAL_MS {
        FadeStep::Normal
    } else if age < FADE_DIM_MS {
        FadeStep::Dim
    } else if age < FADE_GONE_MS {
        FadeStep::Dimmer
    } else {
        FadeStep::Gone
    }
}
pub fn shimmer_band(frame: u64, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        (frame % (len as u64 + 6)) as usize
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn periodic_helpers_repeat() {
        for f in 0..100 {
            assert_eq!(cycle(f, 2, 10), cycle(f + 20, 2, 10));
            assert_eq!(breathe(f, 6), breathe(f + 6, 6));
            assert_eq!(shimmer_band(f, 12), shimmer_band(f + 18, 12));
        }
    }
    #[test]
    fn fades_are_frame_aligned_and_monotone() {
        let steps = [
            FadeStep::Bright,
            FadeStep::Normal,
            FadeStep::Dim,
            FadeStep::Dimmer,
            FadeStep::Gone,
        ];
        let points = [0, FADE_BRIGHT_MS, FADE_NORMAL_MS, FADE_DIM_MS, FADE_GONE_MS];
        for (t, s) in points.into_iter().zip(steps) {
            assert_eq!(fade(t), s);
        }
        for t in [FADE_BRIGHT_MS, FADE_NORMAL_MS, FADE_DIM_MS, FADE_GONE_MS] {
            assert_eq!(t % FRAME_MS, 0);
        }
    }
}
