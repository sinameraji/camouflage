# Same as host-splash but routes the host's NDJSON through a tee so
# we can inspect what events the adapter emits.

Output out/host-debug/session.gif
Set Shell bash
Set FontSize 14
Set Width 1400
Set Height 900
Set Padding 10
Set TypingSpeed 30ms

Env CAMOUFLAGE_REAL_BIN "/Users/sinameraji/camouflage/target/release/camouflage-tui"
Env CAMOUFLAGE_TEE_LOG "/tmp/host-events.ndjson"

Hide
Type 'your-agent --ui camouflage --camouflage-bin /Users/sinameraji/camouflage/tests/visual/tee-tui.sh'
Enter
Show

Sleep 3500ms
Screenshot out/host-debug/01-splash.png

Ctrl+C
Sleep 300ms
Ctrl+C
Sleep 200ms
