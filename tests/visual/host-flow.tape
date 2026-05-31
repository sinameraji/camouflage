# Full smoke: user message -> tool execution -> permission -> assistant reply.

Output out/host-flow/session.gif
Set Shell bash
Set FontSize 14
Set Width 1200
Set Height 800
Set Padding 10
Set TypingSpeed 30ms

Hide
Env FEED_DELAY_MS 80
Type '"$FEED_SH" "$FIXTURES_DIR/adapter-simple.ndjson" 30 | "$CAMOUFLAGE_BIN" --stdin-events --responses-fd 3 --fps 30 3>/dev/null'
Enter
Show

Sleep 500ms
Screenshot out/host-flow/01-session-started.png

Sleep 800ms
Screenshot out/host-flow/02-user-message.png

Sleep 1500ms
Screenshot out/host-flow/03-assistant-reply.png

Sleep 1000ms
Screenshot out/host-flow/04-final.png

Ctrl+C
Sleep 200ms
