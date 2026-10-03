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

## What counts

release-please tracks the **whole repository** (package path `.` in
`.release-please-config.json`), so a `fix:` or `feat:` anywhere, including
the Rust crates the binary is built from, opens a release PR. `ci:`,
`chore:`, `docs:`, `test:` never do. It bumps `sdk/node/package.json`
(the npm package), `version.txt`, and `sdk/node/CHANGELOG.md`.

Versioning is `prerelease`: while on a beta, both fixes and features bump
the beta number (`2.4.0-beta.7` → `2.4.0-beta.8`), which keeps us inside
downstream caret ranges like autopilot's `^2.4.0-beta.3`. A breaking change
(`!` / `BREAKING CHANGE:`) moves to the next major.

To force a specific version, add a `Release-As: <version>` footer to any
releasable commit.

## Going stable

Pre-releases publish to the `beta` dist-tag. For a stable cut (publishes to
`latest`), land a commit with `Release-As: <x.y.z>` and set `"prerelease":
false` / `"versioning": "default"` in `.release-please-config.json`.
