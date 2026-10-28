#!/usr/bin/env node

/** Stage the release-built renderer binaries in their npm platform packages. */
import { chmodSync, existsSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const SDK_DIR = resolve(SCRIPT_DIR, "..");

export const PLATFORM_TARGETS = [
  { target: "aarch64-apple-darwin", id: "darwin-arm64", executable: "camouflage-tui" },
  { target: "x86_64-apple-darwin", id: "darwin-x64", executable: "camouflage-tui" },
  { target: "aarch64-unknown-linux-gnu", id: "linux-arm64", executable: "camouflage-tui" },
  { target: "x86_64-unknown-linux-gnu", id: "linux-x64", executable: "camouflage-tui" },
  { target: "x86_64-pc-windows-msvc", id: "win32-x64", executable: "camouflage-tui.exe" },
];

/**
 * Extract the five release archives into their npm package directories.
 * The manifests must already have been versioned by release-please.
 */
export function preparePlatformPackages(archiveDir, {
  sdkDir = SDK_DIR,
  extract = (archive, destination, executable) => {
    execFileSync("tar", ["-xzf", archive, "-C", destination, executable], { stdio: "inherit" });
  },
} = {}) {
  if (!archiveDir) throw new Error("usage: prepare-platform-packages.js <archive-directory>");
  const rootPackage = JSON.parse(readFileSync(join(sdkDir, "package.json"), "utf8"));

  for (const { target, id, executable } of PLATFORM_TARGETS) {
    const packageDir = join(sdkDir, "platform-packages", id);
    const packagePath = join(packageDir, "package.json");
    const manifest = JSON.parse(readFileSync(packagePath, "utf8"));
    const expectedName = `camouflage-tui-${id}`;
    const archive = join(archiveDir, `camouflage-tui-${target}.tar.gz`);
    const binary = join(packageDir, executable);

    if (manifest.name !== expectedName) {
      throw new Error(`${packagePath}: expected package name ${expectedName}, got ${manifest.name}`);
    }
    if (manifest.version !== rootPackage.version || rootPackage.optionalDependencies?.[expectedName] !== rootPackage.version) {
      throw new Error(`${expectedName} version must match camouflage-tui ${rootPackage.version}`);
    }
    if (!existsSync(archive)) throw new Error(`missing release archive: ${archive}`);

    mkdirSync(packageDir, { recursive: true });
    rmSync(binary, { force: true });
    extract(archive, packageDir, executable);
    if (!existsSync(binary)) throw new Error(`${archive} did not contain ${executable}`);
    if (executable !== "camouflage-tui.exe") chmodSync(binary, 0o755);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    preparePlatformPackages(process.argv[2]);
  } catch (err) {
    console.error(`prepare-platform-packages: ${err instanceof Error ? err.message : String(err)}`);
    process.exitCode = 1;
  }
}
