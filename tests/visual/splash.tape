# Exercises the splash banner: SGR escape parsing, pinned-above-transcript
# layout, and auto-dismiss on first user submit.

Output out/splash/session.gif
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
Screenshot out/splash/01-splash-rendered.png

Type "hello"
Sleep 500ms
Screenshot out/splash/02-splash-with-input.png

Enter
Sleep 800ms
Screenshot out/splash/03-after-submit.png

Ctrl+C
Sleep 200ms
