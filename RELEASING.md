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

Versioning is standard semver since 2.4.0: `fix:` → patch, `feat:` → minor,
a breaking change (`!` / `BREAKING CHANGE:`) → major. Releases publish to
npm's `latest` dist-tag, so ordinary caret ranges (`^2.4.0`) pick them up.

To force a specific version, add a `Release-As: <version>` footer to any
releasable commit.

## Pre-releases

To publish a beta again (to the `beta` dist-tag), set `"prerelease": true`
and `"versioning": "prerelease"` in `.release-please-config.json` and land a
commit with `Release-As: <x.y.z>-beta.1`.
