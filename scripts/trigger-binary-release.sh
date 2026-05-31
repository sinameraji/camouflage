#!/usr/bin/env bash
#
# Force a release of the `camouflage-tui` npm package so a fresh native
# binary is built and published from the current `main`.
#
# Why this exists
# ---------------
# release-please is configured to track only the `sdk/node` package path
# (see .release-please-config.json). But the actual TUI binary is built from
# `crates/**` by .github/workflows/release.yml on the release tag. So a
# user-facing change that lives ONLY in `crates/**` (e.g. a TUI bug fix) does
# not produce a release-please release on its own — `sdk/node` saw no
# user-facing commit — and the new binary never ships.
#
# This script lands a small, releasable, in-path commit (a provenance stamp
# under `sdk/node/`) with a `Release-As:` footer, so release-please opens a
# release PR at the version you specify. Merging that PR tags the release,
# which builds + attaches the binary from current `main` and publishes to npm.
#
# Usage
# -----
#   scripts/trigger-binary-release.sh <next-version>
#   e.g. scripts/trigger-binary-release.sh 1.1.1-beta.1
#
# Then:
#   git push -u origin <printed branch>
#   gh pr create --base main --fill
#   # merge it → merge the release-please PR it produces → publish
set -euo pipefail

version="${1:-}"
if [ -z "$version" ]; then
  echo "usage: $0 <next-version>   e.g. 1.1.1-beta.1" >&2
  exit 2
fi

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

current_branch="$(git rev-parse --abbrev-ref HEAD)"
if [ "$current_branch" != "main" ]; then
  echo "warning: you are on '$current_branch', not 'main'. The release builds from" >&2
  echo "         the merged commit, so make sure this branch is up to date with main." >&2
fi

branch="chore/trigger-release-${version}"
git checkout -b "$branch"

sha="$(git rev-parse HEAD)"
stamp="$repo_root/sdk/node/BINARY_SOURCE.txt"
printf 'The camouflage-tui native binary is built from the camouflage workspace.\nReleased from commit: %s\n' "$sha" > "$stamp"

git add "$stamp"
git commit -m "fix(sdk): release camouflage-tui ${version} (rebuild binary from ${sha:0:12})

Forces a release so the prebuilt native binary is rebuilt and published from
current main. release-please only tracks the sdk/node package path, so
crate-only changes don't trigger a release on their own.

Release-As: ${version}"

cat <<EOF

✓ Created branch '${branch}' with a Release-As: ${version} commit.

Next:
  git push -u origin ${branch}
  gh pr create --base main --fill

Merge that PR → release-please opens the ${version} release PR → merge it to
build + publish the binary.
EOF
