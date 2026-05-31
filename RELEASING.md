# Releasing

The published product is the `camouflage-tui` npm package, which downloads a
pre-built native binary built from the Rust crates in this workspace.

## How releases work

- **release-please** (`.github/workflows/release-please.yml`) watches `main`
  and opens a release PR when it sees user-facing commits (`fix:` → patch,
  `feat:` → minor, `!`/`BREAKING` → major). Merging that PR bumps the version,
  tags `camouflage-tui-v<version>`, and publishes to npm (pre-releases go to
  the `beta` dist-tag; stable to `latest`).
- **release.yml** triggers on the `camouflage-tui-v*` tag, builds the binary
  for each platform, and attaches the tarballs to the GitHub Release.
- **postinstall** (`sdk/node/scripts/install.js`) downloads the binary for the
  tag matching the installed package version.

## The gotcha: crate-only changes don't auto-release

release-please is configured to track only the **`sdk/node`** package path
(`.release-please-config.json`). The binary, however, is built from
**`crates/**`**. So a user-facing change that lives only under `crates/**`
(e.g. a TUI bug fix) does **not** trigger a release on its own — `sdk/node`
saw no releasable commit — and the new binary never ships.

This is easy to miss. Two safety nets are in place:

1. The release-please workflow prints a **`::warning::` and job-summary note**
   whenever it cuts no release but there are unreleased `fix:`/`feat:` commits
   under `crates/**` since the last tag.
2. Conventional commit types matter: `ci:`, `chore:`, `docs:`, etc. never
   trigger a release, even under `sdk/node`.

## Cutting a release for a crate-only change

Run, from a branch off an up-to-date `main`:

```bash
scripts/trigger-binary-release.sh 1.1.1-beta.1
git push -u origin chore/trigger-release-1.1.1-beta.1
gh pr create --base main --fill
```

This lands a tiny `fix(sdk):` provenance commit with a `Release-As:` footer so
release-please opens the release PR. Merge the trigger PR, then merge the
release-please PR it produces — that builds and publishes the binary from
current `main`.

## Picking the version

- Bump the **patch** for fixes: `1.1.0-beta.1` → `1.1.1-beta.1`.
- Keep the `-beta.N` suffix while on the beta channel (publishes to the `beta`
  dist-tag, which downstream consumers like kimiflare track).
- Drop the suffix for a stable cut: `1.1.1` (publishes to `latest`).
