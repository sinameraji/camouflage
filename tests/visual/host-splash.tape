# Launches the host with the --ui camouflage adapter so we can
# see the real end-user splash logo through the same renderer the user runs.
#
# Requires:
#   - the host app (built and on PATH as `your-agent`)
#   - $CAMOUFLAGE_BIN points at target/release/camouflage-tui

Output out/host-splash/session.gif
Set Shell bash
Set FontSize 14
Set Width 1400
Set Height 900
Set Padding 10
Set TypingSpeed 30ms

Hide
Type 'your-agent --ui camouflage --camouflage-bin "$CAMOUFLAGE_BIN"'
Enter
Show

# Give the host time to spawn camouflage-tui and emit the splash.
Sleep 3s
Screenshot out/host-splash/01-splash.png

Sleep 1500ms
Screenshot out/host-splash/02-splash-settled.png

# Type something to see if splash dismisses, and how the input area looks.
Type "hi"
Sleep 400ms
Screenshot out/host-splash/03-with-input.png

# Quit cleanly.
Ctrl+C
Sleep 300ms
Ctrl+C
Sleep 200ms
