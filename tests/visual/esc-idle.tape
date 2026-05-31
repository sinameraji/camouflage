# Exercises Esc-at-idle: dismisses the pinned splash without spurious renders.
# Uses the splash-sgr fixture which seeds a Splash banner via
# SessionStarted + Splash + StatusUpdate.

Output out/esc-idle/session.gif
Set Shell bash
Set FontSize 14
Set Width 1200
Set Height 700
Set Padding 10
Set TypingSpeed 30ms

Hide
Type '"$FEED_SH" "$FIXTURES_DIR/splash-sgr.ndjson" 30 | "$CAMOUFLAGE_BIN" --stdin-events --responses-fd 3 --fps 30 3>/dev/null'
Enter
Show

Sleep 1500ms
Screenshot out/esc-idle/01-before-esc.png

Escape
Sleep 400ms
Screenshot out/esc-idle/02-after-esc.png

Escape
Sleep 400ms
Screenshot out/esc-idle/03-second-esc-noop.png

Ctrl+C
Sleep 200ms
