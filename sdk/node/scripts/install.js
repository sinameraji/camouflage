#!/usr/bin/env node

/**
 * postinstall script — downloads the pre-built camouflage-tui binary
 * for the current platform from GitHub Releases.
 *
 * Falls back gracefully: if the download fails (e.g. no internet, unsupported
 * platform, CI without GitHub access), prints instructions to build from source.
 */

import { createWriteStream, mkdirSync, chmodSync, existsSync, unlinkSync } from "node:fs";
import { execSync } from "node:child_process";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { get as httpsGet } from "node:https";

const __dirname = dirname(fileURLToPath(import.meta.url));
const BIN_DIR = join(__dirname, "..", "bin");
const BIN_PATH = join(BIN_DIR, "camouflage-tui");

const PLATFORM_MAP = {
  "darwin-x64": "x86_64-apple-darwin",
  "darwin-arm64": "aarch64-apple-darwin",
  "linux-x64": "x86_64-unknown-linux-gnu",
  "linux-arm64": "aarch64-unknown-linux-gnu",
};

const REPO = "sinameraji/camouflage";

async function main() {
  // Skip if binary already exists (e.g. user built from source)
  if (existsSync(BIN_PATH)) {
    return;
  }

  const key = `${process.platform}-${process.arch}`;
  const target = PLATFORM_MAP[key];
  if (!target) {
    warn(`Unsupported platform: ${key}`);
    return;
  }

  // Read version from package.json
  const { default: pkg } = await import("../package.json", { with: { type: "json" } });
  const version = pkg.version;
  const archive = `camouflage-tui-${target}.tar.gz`;
  // release-please tags this package's releases with the component prefix
  // (camouflage-tui-v<version>). Older releases used a bare v<version> tag.
  // Try the prefixed tag first, then fall back to the legacy one so both
  // old and new published versions resolve to a real release asset.
  const tags = [`camouflage-tui-v${version}`, `v${version}`];

  mkdirSync(BIN_DIR, { recursive: true });

  const tmpPath = `${BIN_PATH}.tmp`;

  let lastErr;
  for (const tag of tags) {
    const url = `https://github.com/${REPO}/releases/download/${tag}/${archive}`;
    try {
      process.stdout.write(`camouflage-tui: downloading ${target} binary (${tag})...\n`);
      await download(url, tmpPath);

      // Extract the binary from the tarball
      execSync(`tar xzf "${tmpPath}" -C "${BIN_DIR}"`, { stdio: "pipe" });
      unlinkSync(tmpPath);

      chmodSync(BIN_PATH, 0o755);
      process.stdout.write(`camouflage-tui: installed to ${BIN_PATH}\n`);
      return;
    } catch (err) {
      // Clean up partial downloads and try the next candidate tag.
      lastErr = err;
      try { unlinkSync(tmpPath); } catch {}
      try { unlinkSync(BIN_PATH); } catch {}
    }
  }

  warn(`Download failed: ${lastErr ? lastErr.message : "unknown error"}`);
}

function download(url, dest, redirects = 0) {
  if (redirects > 5) return Promise.reject(new Error("Too many redirects"));

  return new Promise((resolve, reject) => {
    httpsGet(url, { headers: { "User-Agent": "camouflage-tui-postinstall" } }, (res) => {
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
      file.on("finish", () => { file.close(); resolve(); });
      file.on("error", reject);
      res.on("error", reject);
    }).on("error", reject);
  });
}

function warn(msg) {
  process.stdout.write(
    `\ncamouflage-tui: ${msg}\n` +
    `\nTo install manually, build from source:\n` +
    `  cargo install --path crates/tui --root ~/.local\n` +
    `  # or: cargo install --git https://github.com/${REPO} camouflage-tui\n\n`,
  );
}

main().catch((err) => {
  warn(err.message);
});
