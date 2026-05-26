# Exercises toast rendering: wrap-to-content sizing, multi-toast stacking,
# kind-based styling, and TTL expiry.

Output out/toasts/session.gif
Set Shell bash
Set FontSize 14
Set Width 1200
Set Height 700
Set Padding 10
Set TypingSpeed 30ms

Hide
Env FEED_DELAY_MS 250
Type '"$FEED_SH" "$FIXTURES_DIR/toast-variants.ndjson" 30 | "$CAMOUFLAGE_BIN" --stdin-events --responses-fd 3 --fps 30 3>/dev/null'
Enter
Show

Sleep 400ms
Screenshot out/toasts/01-first-toast.png

Sleep 600ms
Screenshot out/toasts/02-three-toasts-stacked.png

Sleep 1200ms
Screenshot out/toasts/03-error-toast-added.png

Sleep 2000ms
Screenshot out/toasts/04-error-expired.png

Ctrl+C
Sleep 200ms
