#!/usr/bin/env node

/**
 * Install the pre-built camouflage-tui binary for the current platform.
 *
 * Failed downloads are non-fatal during npm postinstall, but are retried and
 * persisted so mount() can report the cause and retry later at runtime.
 */

import { createWriteStream, mkdirSync, chmodSync, existsSync, unlinkSync, writeFileSync, readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { homedir } from "node:os";
import { get as httpsGet } from "node:https";

const __dirname = dirname(fileURLToPath(import.meta.url));
const BIN_DIR = join(__dirname, "..", "bin");
const EXE = process.platform === "win32" ? ".exe" : "";
const BIN_PATH = join(BIN_DIR, `camouflage-tui${EXE}`);
export const INSTALL_ERROR_PATH = join(homedir(), ".camouflage", "install-error.txt");

const PLATFORM_MAP = {
  "darwin-x64": "x86_64-apple-darwin",
  "darwin-arm64": "aarch64-apple-darwin",
  "linux-x64": "x86_64-unknown-linux-gnu",
  "linux-arm64": "aarch64-unknown-linux-gnu",
  "win32-x64": "x86_64-pc-windows-msvc",
};

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

    const archive = `camouflage-tui-${target}.tar.gz`;
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
          await extract(tmpPath, binDir);
          unlinkIfExists(tmpPath);
          chmodSync(binPath, 0o755);
          clearFailure(failurePath);
          log(`camouflage-tui: installed to ${binPath}`);
          return true;
        } catch (err) {
          errors.push(`${tag} attempt ${attempt}: ${errorMessage(err)}`);
          unlinkIfExists(tmpPath);
          unlinkIfExists(binPath);
          if (attempt < retries) await pause(backoffMs * (2 ** (attempt - 1)));
        }
      }
    }

    throw new Error(errors.join("; ") || "Unknown download error");
  } catch (err) {
    const message = errorMessage(err);
    persistFailure(failurePath, message);
    reportWarning(`Download failed: ${message}`);
    return false;
  }
}

function extractArchive(archivePath, destination) {
  execFileSync("tar", ["xzf", archivePath, "-C", destination], { stdio: "pipe" });
}

function download(url, dest, redirects = 0) {
  if (redirects > 5) return Promise.reject(new Error("Too many redirects"));

  return new Promise((resolve, reject) => {
    const req = httpsGet(url, { headers: { "User-Agent": "camouflage-tui-postinstall" } }, (res) => {
      if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
        res.resume();
        return resolve(download(res.headers.location, dest, redirects + 1));
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
    `To install manually, build from source:\n` +
    `  cargo install --path crates/tui --root ~/.local\n` +
    `  # or: cargo install --git https://github.com/${REPO} camouflage-tui\n\n`,
  );
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  await installBinary();
}
