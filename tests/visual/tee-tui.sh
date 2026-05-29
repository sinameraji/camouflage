#!/usr/bin/env bash
# Wrapper that tees the host's outgoing NDJSON to a log file before piping
# it into the real camouflage-tui. Lets us see what events the adapter
# actually emits without changing the TUI.
#
# Use by passing --camouflage-bin /path/to/tee-tui.sh to the host.

set -u
log="${CAMOUFLAGE_TEE_LOG:-/tmp/cam-events.ndjson}"
real_bin="${CAMOUFLAGE_REAL_BIN:?CAMOUFLAGE_REAL_BIN must point at camouflage-tui}"
: > "$log"
exec tee -a "$log" | "$real_bin" "$@"
