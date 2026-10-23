#!/usr/bin/env node

/**
 * Install the pre-built camouflage-tui binary for the current platform.
 *
 * Failed downloads are non-fatal during npm postinstall, but are retried and
 * persisted so mount() can report the cause and retry later at runtime.
 *
 * The archive's SHA-256 must match the digest this package version ships in
 * checksums.json (see integrity.js) before anything is extracted; downloads
 * only follow redirects to GitHub hosts. Set CAMOUFLAGE_SKIP_VERIFY=1 to
 * install without a digest check (e.g. from a git checkout).
 */

import { createWriteStream, mkdirSync, chmodSync, existsSync, unlinkSync, writeFileSync, readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join, dirname, resolve, win32 } from "node:path";
import { fileURLToPath } from "node:url";
import { homedir } from "node:os";
import { get as httpsGet } from "node:https";
import {
  PLATFORM_MAP,
  SKIP_VERIFY_ENV,
  IntegrityError,
  archiveName,
  expectedDigest,
  isAllowedDownloadUrl,
  loadChecksums,
  verifyArchive,
} from "./integrity.js";

const __dirname = dirname(fileURLToPath(import.meta.url));
const BIN_DIR = join(__dirname, "..", "bin");
const EXE = process.platform === "win32" ? ".exe" : "";
const BIN_PATH = join(BIN_DIR, `camouflage-tui${EXE}`);
export const INSTALL_ERROR_PATH = join(homedir(), ".camouflage", "install-error.txt");

const REPO = "sinameraji/camouflage";
const DEFAULT_RETRIES = 3;
const DEFAULT_BACKOFF_MS = 500;

/**
 * Ensure the package-local renderer is installed.
 * Options are injectable to keep retry and failure-reporting behavior testable.
 */
export async function installBinary({
  platform = process.platform,
  arch = process.arch,
  version,
  binPath = BIN_PATH,
  failurePath = INSTALL_ERROR_PATH,
  retries = DEFAULT_RETRIES,
  backoffMs = DEFAULT_BACKOFF_MS,
  request = download,
  extract = extractArchive,
  checksums = loadChecksums,
  env = process.env,
  pause = delay,
  log = (message) => process.stdout.write(`${message}\n`),
  reportWarning = warn,
} = {}) {
  try {
    if (existsSync(binPath)) {
      clearFailure(failurePath);
      return true;
    }

    const key = `${platform}-${arch}`;
    const target = PLATFORM_MAP[key];
    if (!target) throw new Error(`Unsupported platform: ${key}`);

    if (!version) {
      const pkg = JSON.parse(readFileSync(join(__dirname, "..", "package.json"), "utf8"));
      version = pkg.version;
    }

    const archive = archiveName(target);
    const expected = expectedDigest(
      typeof checksums === "function" ? checksums() : checksums,
      version,
      archive,
    );
    const skipVerify = env[SKIP_VERIFY_ENV] === "1";
    if (!expected && !skipVerify) {
      throw new IntegrityError(
        `no SHA-256 digest for ${archive} in this package's checksums.json (version ${version}); ` +
        `refusing to install an unverified binary. ` +
        `Set ${SKIP_VERIFY_ENV}=1 to install it without verification, or pass { bin } to mount().`,
      );
    }
    if (!expected) {
      log(`camouflage-tui: WARNING: ${SKIP_VERIFY_ENV}=1 is set; installing the renderer without verifying its SHA-256.`);
    }
    // release-please uses the component-prefixed tag; keep the legacy tag as
    // a fallback for versions published before component-prefixed releases.
    const tags = [`camouflage-tui-v${version}`, `v${version}`];
    const binDir = dirname(binPath);
    mkdirSync(binDir, { recursive: true });
    const tmpPath = `${binPath}.tmp`;
    const errors = [];

    for (const tag of tags) {
      const url = `https://github.com/${REPO}/releases/download/${tag}/${archive}`;
      for (let attempt = 1; attempt <= retries; attempt++) {
        try {
          log(`camouflage-tui: downloading ${target} binary (${tag}, attempt ${attempt}/${retries})...`);
          await request(url, tmpPath);
          if (expected) verifyArchive(tmpPath, expected, `${tag}/${archive}`);
          await extract(tmpPath, binDir);
          unlinkIfExists(tmpPath);
          chmodSync(binPath, 0o755);
          clearFailure(failurePath);
          log(`camouflage-tui: installed to ${binPath}`);
          return true;
        } catch (err) {
          unlinkIfExists(tmpPath);
          unlinkIfExists(binPath);
          // A digest mismatch is not a transient failure: stop here.
          if (err instanceof IntegrityError) throw err;
          errors.push(`${tag} attempt ${attempt}: ${errorMessage(err)}`);
          if (attempt < retries) await pause(backoffMs * (2 ** (attempt - 1)));
        }
      }
    }

    throw new Error(errors.join("; ") || "Unknown download error");
  } catch (err) {
    const message = errorMessage(err);
    persistFailure(failurePath, message);
    reportWarning(`Renderer install failed: ${message}`);
    return false;
  }
}

/**
 * Absolute path of the system tar. Never a bare "tar": on Windows, libuv
 * looks for tar.com / tar.exe in the working directory before PATH, and the
 * runtime repair in mount() runs in the host's cwd (often an untrusted
 * project), so a planted tar.exe there would run.
 */
export function systemTar({ platform = process.platform, env = process.env, exists = existsSync } = {}) {
  if (platform === "win32") {
    // bsdtar has shipped in System32 since Windows 10 1803.
    const roots = [env.SystemRoot, env.windir, "C:\\Windows"];
    for (const root of roots) {
      if (!root || !win32.isAbsolute(root)) continue;
      const candidate = win32.join(root, "System32", "tar.exe");
      if (exists(candidate)) return candidate;
    }
    throw new Error("no tar.exe in %SystemRoot%\\System32 (Windows 10 1803 or later is required to install the renderer)");
  }
  for (const candidate of ["/usr/bin/tar", "/bin/tar", "/usr/local/bin/tar", "/opt/homebrew/bin/tar"]) {
    if (exists(candidate)) return candidate;
  }
  // Unusual layout: fall back to a PATH lookup. POSIX exec does not search
  // the working directory, and cwd is set to the destination anyway.
  return "tar";
}

export function extractArchive(archivePath, destination, { tar = systemTar(), run = execFileSync } = {}) {
  run(tar, ["xzf", archivePath, "-C", destination], { stdio: "pipe", cwd: destination });
}

export function download(url, dest, redirects = 0, get = httpsGet) {
  if (redirects > 5) return Promise.reject(new Error("Too many redirects"));
  if (!isAllowedDownloadUrl(url)) {
    return Promise.reject(new Error(`Refusing to download from ${url}: not a GitHub release host`));
  }

  return new Promise((resolve, reject) => {
    const req = get(url, { headers: { "User-Agent": "camouflage-tui-postinstall" } }, (res) => {
      if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
        res.resume();
        let next;
        try {
          next = new URL(res.headers.location, url).toString();
        } catch {
          return reject(new Error(`Bad redirect from ${url}: ${res.headers.location}`));
        }
        return resolve(download(next, dest, redirects + 1, get));
      }
      if (res.statusCode !== 200) {
        res.resume();
        return reject(new Error(`HTTP ${res.statusCode} from ${url}`));
      }
      const file = createWriteStream(dest);
      res.pipe(file);
      file.on("finish", () => file.close((err) => err ? reject(err) : resolve()));
      file.on("error", reject);
      res.on("error", reject);
    });
    req.setTimeout(30_000, () => req.destroy(new Error(`Timed out downloading ${url}`)));
    req.on("error", reject);
  });
}

function persistFailure(path, message) {
  try {
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, `${new Date().toISOString()}\n${message}\n`, "utf8");
  } catch (err) {
    process.stderr.write(`camouflage-tui: could not persist install failure: ${errorMessage(err)}\n`);
  }
}

function clearFailure(path) {
  try { unlinkSync(path); } catch {}
}

function unlinkIfExists(path) {
  try { unlinkSync(path); } catch {}
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function errorMessage(err) {
  return err instanceof Error ? err.message : String(err);
}

function warn(message) {
  process.stderr.write(
    `\ncamouflage-tui: ${message}\n` +
    `Install diagnostics: ${INSTALL_ERROR_PATH}\n` +
    `To use your own renderer, pass { bin: "/path/to/camouflage-tui" } to mount(),\n` +
    `or build from source:\n` +
    `  cargo install --path crates/tui --root ~/.local\n` +
    `  # or: cargo install --git https://github.com/${REPO} camouflage-tui\n\n`,
  );
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  await installBinary();
}
