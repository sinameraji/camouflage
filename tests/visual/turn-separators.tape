# Exercises v0.5+ transcript labels ("You:" / "your-agent:") and the
# pad/rule/pad turn separator inserted after each user and assistant row.

Output out/turn-separators/session.gif
Set Shell bash
Set FontSize 14
Set Width 1200
Set Height 800
Set Padding 10
Set TypingSpeed 30ms

Hide
Env FEED_DELAY_MS "200"
Type '"$FEED_SH" "$FIXTURES_DIR/turn-separators.ndjson" 30 | "$CAMOUFLAGE_BIN" --stdin-events --responses-fd 3 --fps 30 3>/dev/null'
Enter
Show

Sleep 1500ms
Screenshot out/turn-separators/01-first-turn.png

Sleep 1500ms
Screenshot out/turn-separators/02-both-turns.png

Sleep 1500ms
Screenshot out/turn-separators/03-final.png

Ctrl+C
Sleep 200ms
