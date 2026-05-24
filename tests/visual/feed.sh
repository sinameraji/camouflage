#!/usr/bin/env bash
# Paced NDJSON feeder. Reads a fixture and emits one line every
# ${FEED_DELAY_MS:-100} milliseconds so the receiving TUI has time
# to render intermediate states between events.
#
# Usage: feed.sh <fixture-path> [trailing-hold-seconds]
#   FEED_DELAY_MS  — per-line delay in milliseconds (default 100)
#
# After the last line, sleeps for ${TRAIL_HOLD:-30} seconds (or the
# positional arg) so the TUI stays alive for vhs to drive keystrokes
# and take screenshots. The TUI exits when this script's stdout closes.

set -u

fixture="${1:?fixture path required}"
trail_hold="${2:-${TRAIL_HOLD:-30}}"
delay_ms="${FEED_DELAY_MS:-100}"

# Convert ms to fractional seconds for sleep (zsh/bash both accept).
delay_s=$(awk -v ms="$delay_ms" 'BEGIN{printf "%.3f", ms/1000}')

while IFS= read -r line; do
  [ -z "$line" ] && continue
  printf '%s\n' "$line"
  sleep "$delay_s"
done < "$fixture"

sleep "$trail_hold"
