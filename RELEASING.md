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
  tag matching the installed package version, and refuses it unless its
  SHA-256 matches the package's `checksums.json`.

## Binary checksums

The `publish` job in `release-please.yml` writes `sdk/node/checksums.json`
(SHA-256 of every platform archive) from the same run's build artifacts,
right before `npm publish`, so the digests are part of the provenance-signed
package. The file is gitignored and never committed. If an archive is
missing, the job fails and nothing is published.

Don't rebuild binaries for a version that is already on npm (the Release
workflow's manual dispatch): builds aren't reproducible, so the new archives
won't match the published digests and installs of that version will fail.
Cut a new patch release instead. The manual npm-publish recovery path
(`workflow_dispatch` on `release-please.yml`) takes the digests from the
archives attached to the existing release, so check those are the ones you
built.

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
