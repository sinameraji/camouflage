import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { installBinary } from "../scripts/install.js";

function tempDir(t) {
  const dir = mkdtempSync(join(tmpdir(), "camouflage-install-test-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  return dir;
}

test("installBinary retries transient download failures and clears old diagnostics", async (t) => {
  const dir = tempDir(t);
  const binPath = join(dir, "bin", "camouflage-tui");
  const failurePath = join(dir, "install-error.txt");
  writeFileSync(failurePath, "old failure");
  let attempts = 0;

  const installed = await installBinary({
    platform: "darwin",
    arch: "arm64",
    version: "2.8.0",
    binPath,
    failurePath,
    retries: 3,
    backoffMs: 0,
    request: async () => {
      attempts++;
      if (attempts < 3) throw new Error("temporary network error");
    },
    extract: async (_archive, destination) => {
      writeFileSync(join(destination, "camouflage-tui"), "fake renderer");
    },
    pause: async () => {},
    log: () => {},
    reportWarning: () => {},
  });

  assert.equal(installed, true);
  assert.equal(attempts, 3);
  assert.equal(existsSync(binPath), true);
  assert.equal(existsSync(failurePath), false);
});

test("installBinary persists failure details after exhausting retries and tags", async (t) => {
  const dir = tempDir(t);
  const binPath = join(dir, "bin", "camouflage-tui");
  const failurePath = join(dir, "install-error.txt");
  let attempts = 0;

  const installed = await installBinary({
    platform: "darwin",
    arch: "arm64",
    version: "2.8.0",
    binPath,
    failurePath,
    retries: 2,
    backoffMs: 0,
    request: async () => {
      attempts++;
      throw new Error("offline");
    },
    pause: async () => {},
    log: () => {},
    reportWarning: () => {},
  });

  assert.equal(installed, false);
  assert.equal(attempts, 4); // two attempts for each of the current and legacy tags
  assert.match(readFileSync(failurePath, "utf8"), /offline/);
});
