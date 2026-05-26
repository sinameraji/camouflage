# Verifies the no-arg `/ui` form opens a Camouflage SelectList picker
# matching the /theme pattern: arrow keys navigate, Enter selects.

Output out/kimiflare-ui-picker/session.gif
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

Sleep 3000ms

# Invoke `/ui` with no arg — should open the picker.
Type "/ui"
Sleep 400ms
Enter
Sleep 800ms
Screenshot out/kimiflare-ui-picker/01-picker-open.png

# Down to camouflage, then back up — verify navigation works.
Down
Sleep 400ms
Screenshot out/kimiflare-ui-picker/02-camouflage-highlighted.png

Up
Sleep 400ms
Screenshot out/kimiflare-ui-picker/03-ink-highlighted.png

# Select ink — should close picker and show the loud restart toast.
Enter
Sleep 800ms
Screenshot out/kimiflare-ui-picker/04-restart-warning.png

Ctrl+C
Sleep 300ms
Ctrl+C
Sleep 200ms
