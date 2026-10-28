# Releasing

The published product is the `camouflage-tui` npm package plus one optional
platform package per supported OS/CPU target. Each platform package contains
a pre-built native binary built from the Rust crates in this workspace.

## How releases work

- **release-please** (`.github/workflows/release-please.yml`) watches `main`
  and opens a release PR when it sees user-facing commits (`fix:` → patch,
  `feat:` → minor, `!`/`BREAKING` → major). It updates the SDK version, all
  five platform package versions, and the SDK's exact optional-dependency
  versions together.
- **release.yml** triggers on the `camouflage-tui-v*` tag, builds the binary
  for each platform, and attaches the tarballs to the GitHub Release.
- The release-please publish job extracts those build artifacts into their
  platform package directories, publishes the five platform packages first,
  then publishes the main SDK. There is no postinstall or binary download.
  All five archives are validated before publication begins; missing assets
  stop the release job.

Configure npm trusted publishing for the main SDK and each platform package,
or provide the `NPM_TOKEN` repository secret for the release workflow.

## What counts

release-please tracks the **whole repository** (package path `.` in
`.release-please-config.json`), so a `fix:` or `feat:` anywhere, including
the Rust crates the binary is built from, opens a release PR. `ci:`,
`chore:`, `docs:`, `test:` never do. It bumps `sdk/node/package.json`, the five files under
`sdk/node/platform-packages/`, `version.txt`, and `sdk/node/CHANGELOG.md`.

Versioning is standard semver since 2.4.0: `fix:` → patch, `feat:` → minor,
a breaking change (`!` / `BREAKING CHANGE:`) → major. Releases publish to
npm's `latest` dist-tag, so ordinary caret ranges (`^2.4.0`) pick them up.

To force a specific version, add a `Release-As: <version>` footer to any
releasable commit.

## Pre-releases

To publish a beta again (to the `beta` dist-tag), set `"prerelease": true`
and `"versioning": "prerelease"` in `.release-please-config.json` and land a
commit with `Release-As: <x.y.z>-beta.1`.
