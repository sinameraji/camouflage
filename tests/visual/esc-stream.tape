# Exercises Esc-during-stream: cancels an active AssistantTokenDelta stream
# without corrupting the transcript.

Output out/esc-stream/session.gif
Set Shell bash
Set FontSize 14
Set Width 1200
Set Height 800
Set Padding 10
Set TypingSpeed 30ms

Hide
Env FEED_DELAY_MS 150
Type '"$FEED_SH" "$FIXTURES_DIR/kimiflare-mock.ndjson" 30 | "$CAMOUFLAGE_BIN" --stdin-events --responses-fd 3 --fps 30 3>/dev/null'
Enter
Show

Sleep 1800ms
Screenshot out/esc-stream/01-streaming.png

Escape
Sleep 500ms
Screenshot out/esc-stream/02-after-esc.png

Sleep 1500ms
Screenshot out/esc-stream/03-transcript-stable.png

Ctrl+C
Sleep 200ms
