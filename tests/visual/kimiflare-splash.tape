# Launches Kimiflare with the --ui camouflage adapter so we can
# see the real end-user splash logo through the same renderer the user runs.
#
# Requires:
#   - ~/kimi-code-clone-3 with branch `camouflage-adapter` and `dist/` built
#   - $CAMOUFLAGE_BIN points at target/release/camouflage-tui

Output out/kimiflare-splash/session.gif
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

# Give kimiflare time to spawn camouflage-tui and emit the splash.
Sleep 3s
Screenshot out/kimiflare-splash/01-splash.png

Sleep 1500ms
Screenshot out/kimiflare-splash/02-splash-settled.png

# Type something to see if splash dismisses, and how the input area looks.
Type "hi"
Sleep 400ms
Screenshot out/kimiflare-splash/03-with-input.png

# Quit cleanly.
Ctrl+C
Sleep 300ms
Ctrl+C
Sleep 200ms
