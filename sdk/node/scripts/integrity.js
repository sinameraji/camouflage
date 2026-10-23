#!/usr/bin/env node

/**
 * Integrity checks for the pre-built renderer binary.
 *
 * GitHub release assets can be replaced after a release is cut, so the npm
 * package carries its own record of what was built: `checksums.json`, the
 * SHA-256 of every platform archive, written by the release workflow from
 * that run's build artifacts right before `npm publish` (and so covered by
 * npm provenance). The installer refuses an archive whose digest doesn't
 * match, or for which this package version has no digest.
 *
 * CLI (used by the release workflow):
 *   node scripts/integrity.js write <archive-dir> <out-file>
 */

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));

export const CHECKSUMS_PATH = join(__dirname, "..", "checksums.json");

/** Set to "1" to install a binary without checking its digest. */
export const SKIP_VERIFY_ENV = "CAMOUFLAGE_SKIP_VERIFY";

export const PLATFORM_MAP = {
  "darwin-x64": "x86_64-apple-darwin",
  "darwin-arm64": "aarch64-apple-darwin",
  "linux-x64": "x86_64-unknown-linux-gnu",
  "linux-arm64": "aarch64-unknown-linux-gnu",
  "win32-x64": "x86_64-pc-windows-msvc",
};

/** Hosts the binary download may be served from (GitHub release redirects). */
const ALLOWED_HOSTS = new Set(["github.com", "objects.githubusercontent.com"]);
const ALLOWED_HOST_SUFFIX = ".githubusercontent.com";

export function archiveName(target) {
  return `camouflage-tui-${target}.tar.gz`;
}

/** Whether a download URL (initial or a redirect target) may be fetched. */
export function isAllowedDownloadUrl(url) {
  let parsed;
  try {
    parsed = new URL(url);
  } catch {
    return false;
  }
  if (parsed.protocol !== "https:" || parsed.username || parsed.password) return false;
  if (parsed.port && parsed.port !== "443") return false;
  const host = parsed.hostname.toLowerCase();
  return ALLOWED_HOSTS.has(host) || host.endsWith(ALLOWED_HOST_SUFFIX);
}

export function sha256File(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

/**
 * Read the package's checksums.json. Returns null when it doesn't exist (a
 * git checkout or a version published before digests were recorded).
 */
export function loadChecksums(path = CHECKSUMS_PATH) {
  if (!existsSync(path)) return null;
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (err) {
    throw new Error(`could not read ${path}: ${err instanceof Error ? err.message : String(err)}`);
  }
}

/**
 * The recorded SHA-256 (lowercase hex) of `archive` for `version`, or null
 * when the checksums don't cover that version and archive.
 */
export function expectedDigest(checksums, version, archive) {
  if (!checksums || typeof checksums !== "object") return null;
  if (checksums.version !== version) return null;
  const digest = checksums.sha256 && checksums.sha256[archive];
  if (typeof digest !== "string" || !/^[0-9a-fA-F]{64}$/.test(digest)) return null;
  return digest.toLowerCase();
}

export class IntegrityError extends Error {
  constructor(message) {
    super(message);
    this.name = "IntegrityError";
  }
}

/** Throw an IntegrityError unless the file at `path` has SHA-256 `expected`. */
export function verifyArchive(path, expected, label = path) {
  const actual = sha256File(path);
  if (actual !== expected.toLowerCase()) {
    throw new IntegrityError(
      `SHA-256 mismatch for ${label}: expected ${expected.toLowerCase()}, got ${actual}. ` +
      `Refusing to install a binary that differs from the one this package was published with.`,
    );
  }
}

/**
 * Build the checksums.json document for `version` from a directory holding
 * every platform archive. Throws if any platform's archive is missing.
 */
export function buildChecksums(archiveDir, version) {
  const sha256 = {};
  for (const target of Object.values(PLATFORM_MAP)) {
    const archive = archiveName(target);
    const path = join(archiveDir, archive);
    if (!existsSync(path)) throw new Error(`missing release archive: ${path}`);
    sha256[archive] = sha256File(path);
  }
  return { version, sha256 };
}

function main(argv) {
  const [command, archiveDir, outFile] = argv;
  if (command !== "write" || !archiveDir || !outFile) {
    process.stderr.write("usage: node scripts/integrity.js write <archive-dir> <out-file>\n");
    process.exit(2);
  }
  const pkg = JSON.parse(readFileSync(join(__dirname, "..", "package.json"), "utf8"));
  const doc = buildChecksums(archiveDir, pkg.version);
  writeFileSync(outFile, `${JSON.stringify(doc, null, 2)}\n`, "utf8");
  for (const [archive, digest] of Object.entries(doc.sha256)) {
    process.stdout.write(`${digest}  ${archive}\n`);
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  main(process.argv.slice(2));
}
