# Verifies the new /ui slash command from inside Camouflage:
# typing `/ui ink` persists the choice to ~/.config/your-agent/config.json
# and emits a red error-kind toast warning that a restart is required.

Output out/host-slash-ui/session.gif
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

# Wait for the TUI to mount with the splash and startup toasts.
Sleep 3000ms
Screenshot out/host-slash-ui/01-tui-ready.png

# Open the slash picker, filter to /ui, and submit `/ui ink`.
Type "/ui ink"
Sleep 600ms
Screenshot out/host-slash-ui/02-command-typed.png

Enter
Sleep 800ms
Screenshot out/host-slash-ui/03-restart-warning.png

Ctrl+C
Sleep 300ms
Ctrl+C
Sleep 200ms
