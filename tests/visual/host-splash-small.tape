# Same as host-splash but at a small terminal size so we can check
# the splash adapts (trimmed padding rows + responsive height cap).

Output out/host-splash-small/session.gif
Set Shell bash
Set FontSize 14
Set Width 900
Set Height 520
Set Padding 10
Set TypingSpeed 30ms

Hide
Type 'your-agent --ui camouflage --camouflage-bin "$CAMOUFLAGE_BIN"'
Enter
Show

Sleep 3500ms
Screenshot out/host-splash-small/01-splash.png

Type "hi"
Sleep 400ms
Screenshot out/host-splash-small/02-with-input.png

Ctrl+C
Sleep 300ms
Ctrl+C
Sleep 200ms
