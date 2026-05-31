#!/usr/bin/env bash
# Run a vhs tape scenario against camouflage-tui.
#
# Usage: run.sh <scenario>
#   scenario — tape file basename (e.g. "splash" -> tests/visual/splash.tape)
#
# Outputs land in tests/visual/out/<scenario>/.

set -euo pipefail

scenario="${1:?scenario name required (e.g. splash, esc-idle, host-flow)}"

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
tape_dir="$repo_root/tests/visual"
tape_file="$tape_dir/${scenario}.tape"
out_dir="$tape_dir/out/${scenario}"

if [ ! -f "$tape_file" ]; then
  echo "no such tape: $tape_file" >&2
  echo "available tapes:" >&2
  ls "$tape_dir"/*.tape 2>/dev/null | sed 's|.*/||;s|\.tape$||' | sed 's/^/  /' >&2
  exit 2
fi

if ! command -v vhs >/dev/null 2>&1; then
  echo "vhs not found. install with: brew install vhs" >&2
  exit 127
fi

# Build the binary (incremental — fast after the first time).
( cd "$repo_root" && cargo build -p camouflage-tui --release 1>&2 )

mkdir -p "$out_dir"
rm -f "$out_dir"/*.png "$out_dir"/*.gif 2>/dev/null || true

# Export paths the tape file references.
export CAMOUFLAGE_BIN="$repo_root/target/release/camouflage-tui"
export FIXTURES_DIR="$repo_root/fixtures"
export FEED_SH="$tape_dir/feed.sh"
export OUT_DIR="$out_dir"

cd "$tape_dir"
vhs "$tape_file"

echo
echo "frames written to: $out_dir"
ls "$out_dir" | sed 's/^/  /'
