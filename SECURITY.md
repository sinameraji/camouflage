# Security policy

## Supported versions

Security fixes go into the latest `camouflage-tui` release on npm (the `beta`
tag) and the matching GitHub release binaries.

## Reporting a vulnerability

Please report vulnerabilities privately through
[GitHub security advisories](https://github.com/sinameraji/camouflage/security/advisories/new)
rather than in a public issue. Include the version, your platform, and steps
to reproduce. You should get a reply within a few days.

## Scope

Camouflage renders events that a host program sends it. It does not execute
commands from those events, open network connections, or read files beyond
its own session store (`~/.camouflage`). The npm package's install script
downloads the prebuilt binary for your platform from this repository's
GitHub releases.

The optional `camouflage-broadcast` tool (built from source, not part of the
npm package) serves the event stream over a WebSocket. It only accepts
clients that present the random access token it prints on stderr at
startup; keep that token private, since it grants read access to the whole
session. Connections that have not completed the WebSocket handshake within
a few seconds are closed, and only a bounded number may be pending at once.
