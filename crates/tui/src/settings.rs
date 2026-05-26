//! Process-wide settings resolved once at startup.

use std::sync::OnceLock;

/// True if mouse-wheel scroll direction should be inverted relative to
/// the terminal's raw wheel events. Matches macOS's "Natural" scrolling
/// preference: two fingers down on the trackpad should reveal older
/// (above) content, not newer (below).
///
/// Resolution order:
///   1. `CAMOUFLAGE_NATURAL_SCROLL=1|true|on|yes` (or 0/false/off/no) — overrides.
///   2. On macOS, read `defaults -g com.apple.swipescrolldirection` (default 1 = natural).
///   3. Other platforms: default to false (no inversion).
pub fn natural_scroll() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| {
        if let Ok(v) = std::env::var("CAMOUFLAGE_NATURAL_SCROLL") {
            return matches!(
                v.to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            );
        }
        #[cfg(target_os = "macos")]
        {
            return read_macos_natural_scroll().unwrap_or(true);
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    })
}

#[cfg(target_os = "macos")]
fn read_macos_natural_scroll() -> Option<bool> {
    let out = std::process::Command::new("defaults")
        .args(["read", "-g", "com.apple.swipescrolldirection"])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let t = s.trim();
    // `defaults` prints "1" for the (default) natural-scrolling setting
    // and "0" when the user has flipped it. If the key is unset at all,
    // macOS still behaves as if it were 1 — so we treat a missing key
    // (non-zero exit) as `natural=true` via the unwrap_or above.
    Some(t == "1")
}
