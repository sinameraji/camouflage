import { test } from "node:test";
import assert from "node:assert/strict";
import { cpSync, existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { PLATFORM_TARGETS, preparePlatformPackages } from "../scripts/prepare-platform-packages.js";

const SDK_DIR = join(dirname(fileURLToPath(import.meta.url)), "..");
const packageJson = (path) => JSON.parse(readFileSync(path, "utf8"));

test("optional renderer packages match the host package version and npm platform filters", () => {
  const root = packageJson(join(SDK_DIR, "package.json"));
  const expected = {
    "darwin-arm64": ["darwin", "arm64", "camouflage-tui"],
    "darwin-x64": ["darwin", "x64", "camouflage-tui"],
    "linux-arm64": ["linux", "arm64", "camouflage-tui"],
    "linux-x64": ["linux", "x64", "camouflage-tui"],
    "win32-x64": ["win32", "x64", "camouflage-tui.exe"],
  };

  for (const { id } of PLATFORM_TARGETS) {
    const pkg = packageJson(join(SDK_DIR, "platform-packages", id, "package.json"));
    const [os, cpu, executable] = expected[id];
    assert.equal(root.optionalDependencies[pkg.name], root.version);
    assert.equal(pkg.version, root.version);
    assert.deepEqual(pkg.os, [os]);
    assert.deepEqual(pkg.cpu, [cpu]);
    assert.deepEqual(pkg.files, [executable]);
  }
  assert.equal(Object.keys(root.optionalDependencies).length, PLATFORM_TARGETS.length);
});

test("release staging extracts each archive into its version-matched package", (t) => {
  const temp = mkdtempSync(join(tmpdir(), "camouflage-platform-packages-"));
  t.after(() => rmSync(temp, { recursive: true, force: true }));
  const sdkDir = join(temp, "sdk");
  const archiveDir = join(temp, "archives");
  mkdirSync(archiveDir, { recursive: true });
  mkdirSync(sdkDir, { recursive: true });
  cpSync(join(SDK_DIR, "package.json"), join(sdkDir, "package.json"));

  for (const { target, id } of PLATFORM_TARGETS) {
    const packageDir = join(sdkDir, "platform-packages", id);
    mkdirSync(packageDir, { recursive: true });
    cpSync(join(SDK_DIR, "platform-packages", id, "package.json"), join(packageDir, "package.json"));
    writeFileSync(join(archiveDir, `camouflage-tui-${target}.tar.gz`), "fixture");
  }

  const extracted = [];
  preparePlatformPackages(archiveDir, {
    sdkDir,
    extract: (archive, destination, executable) => {
      extracted.push({ archive, destination, executable });
      writeFileSync(join(destination, executable), "test renderer");
    },
  });

  assert.equal(extracted.length, PLATFORM_TARGETS.length);
  for (const { id, executable } of PLATFORM_TARGETS) {
    assert.equal(existsSync(join(sdkDir, "platform-packages", id, executable)), true);
  }
});
