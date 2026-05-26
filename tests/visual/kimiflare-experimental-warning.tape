# Verifies the experimental warning lands when a user opts into
# `--ui camouflage`: stderr notice before the alt-screen, plus a
# warn-kind toast inside the TUI that says how to switch back.

Output out/kimiflare-experimental-warning/session.gif
Set Shell bash
Set FontSize 14
Set Width 1400
Set Height 900
Set Padding 10
Set TypingSpeed 30ms

Hide
Type 'cd ~/kimi-code-clone-3 && node bin/kimiflare.mjs --ui camouflage --camouflage-bin "$CAMOUFLAGE_BIN"'
Enter
Show

# Capture the stderr warning before the alt-screen takes over.
Sleep 800ms
Screenshot out/kimiflare-experimental-warning/01-stderr-warning.png

# After the alt-screen takes over, verify the warn-toast is visible.
Sleep 2500ms
Screenshot out/kimiflare-experimental-warning/02-tui-toast.png

Ctrl+C
Sleep 300ms
Ctrl+C
Sleep 200ms
