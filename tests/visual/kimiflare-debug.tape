# Same as kimiflare-splash but routes Kimiflare's NDJSON through a tee so
# we can inspect what events the adapter emits.

Output out/kimiflare-debug/session.gif
Set Shell bash
Set FontSize 14
Set Width 1400
Set Height 900
Set Padding 10
Set TypingSpeed 30ms

Env CAMOUFLAGE_REAL_BIN "/Users/sinameraji/camouflage/target/release/camouflage-tui"
Env CAMOUFLAGE_TEE_LOG "/tmp/kimiflare-events.ndjson"

Hide
Type 'cd ~/kimi-code-clone-3 && node bin/kimiflare.mjs --ui camouflage --camouflage-bin /Users/sinameraji/camouflage/tests/visual/tee-tui.sh'
Enter
Show

Sleep 3500ms
Screenshot out/kimiflare-debug/01-splash.png

Ctrl+C
Sleep 300ms
Ctrl+C
Sleep 200ms
