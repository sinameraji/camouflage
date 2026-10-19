# Busy-turn animation smoke: plan, scout, tool trail, spinner and streamed text.

Output out/busy-turn/session.gif
Set Shell bash
Set FontSize 14
Set Width 1200
Set Height 800
Set Padding 10
Set TypingSpeed 30ms

Hide
Env FEED_DELAY_MS "220"
Type '"$FEED_SH" "$FIXTURES_DIR/busy-turn.ndjson" 30 | "$CAMOUFLAGE_BIN" --stdin-events --responses-fd 3 --fps 30 3>/dev/null'
Enter
Show

Sleep 1600ms
Screenshot out/busy-turn/01-tools-and-plan.png

Sleep 1600ms
Screenshot out/busy-turn/02-streaming.png

Sleep 1200ms
Screenshot out/busy-turn/03-steady-busy.png

Ctrl+C
Sleep 200ms
