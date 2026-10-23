import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync, existsSync } from "node:fs";
import { createHash } from "node:crypto";
import { EventEmitter } from "node:events";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { installBinary, download } from "../scripts/install.js";
import {
  PLATFORM_MAP,
  archiveName,
  buildChecksums,
  expectedDigest,
  isAllowedDownloadUrl,
  loadChecksums,
  verifyArchive,
} from "../scripts/integrity.js";

const ARCHIVE = "camouflage-tui-aarch64-apple-darwin.tar.gz";
const FAKE_ARCHIVE = "fake archive bytes";
const FAKE_DIGEST = createHash("sha256").update(FAKE_ARCHIVE).digest("hex");
const CHECKSUMS = { version: "2.8.0", sha256: { [ARCHIVE]: FAKE_DIGEST } };

// A request() stand-in that "downloads" `body` to the destination path.
function serve(body = FAKE_ARCHIVE) {
  return async (_url, dest) => writeFileSync(dest, body);
}

function fakeExtract(_archive, destination) {
  writeFileSync(join(destination, "camouflage-tui"), "fake renderer");
}

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
    request: async (url, dest) => {
      attempts++;
      if (attempts < 3) throw new Error("temporary network error");
      await serve()(url, dest);
    },
    extract: fakeExtract,
    checksums: CHECKSUMS,
    env: {},
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
    checksums: CHECKSUMS,
    env: {},
    pause: async () => {},
    log: () => {},
    reportWarning: () => {},
  });

  assert.equal(installed, false);
  assert.equal(attempts, 4); // two attempts for each of the current and legacy tags
  assert.match(readFileSync(failurePath, "utf8"), /offline/);
});

function installOpts(dir, overrides = {}) {
  return {
    platform: "darwin",
    arch: "arm64",
    version: "2.8.0",
    binPath: join(dir, "bin", "camouflage-tui"),
    failurePath: join(dir, "install-error.txt"),
    retries: 2,
    backoffMs: 0,
    request: serve(),
    extract: fakeExtract,
    checksums: CHECKSUMS,
    env: {},
    pause: async () => {},
    log: () => {},
    reportWarning: () => {},
    ...overrides,
  };
}

test("installBinary installs an archive whose SHA-256 matches checksums.json", async (t) => {
  const dir = tempDir(t);
  const opts = installOpts(dir);
  assert.equal(await installBinary(opts), true);
  assert.equal(existsSync(opts.binPath), true);
});

test("installBinary refuses an archive whose SHA-256 does not match, without retrying", async (t) => {
  const dir = tempDir(t);
  let downloads = 0;
  let extracted = false;
  const opts = installOpts(dir, {
    request: async (url, dest) => { downloads++; writeFileSync(dest, "tampered archive"); },
    extract: (...args) => { extracted = true; fakeExtract(...args); },
  });
  assert.equal(await installBinary(opts), false);
  assert.equal(downloads, 1);
  assert.equal(extracted, false);
  assert.equal(existsSync(opts.binPath), false);
  assert.equal(existsSync(`${opts.binPath}.tmp`), false);
  assert.match(readFileSync(opts.failurePath, "utf8"), /SHA-256 mismatch/);
});

test("installBinary refuses when checksums.json has no digest for this version or platform", async (t) => {
  for (const checksums of [
    null, // no checksums.json (git checkout / older package)
    { ...CHECKSUMS, version: "2.7.0" }, // digests for a different version
    { version: "2.8.0", sha256: {} }, // no entry for this platform
    { version: "2.8.0", sha256: { [ARCHIVE]: "not-a-digest" } },
  ]) {
    const dir = tempDir(t);
    let downloads = 0;
    const opts = installOpts(dir, { checksums, request: async () => { downloads++; } });
    assert.equal(await installBinary(opts), false, JSON.stringify(checksums));
    assert.equal(downloads, 0);
    assert.match(readFileSync(opts.failurePath, "utf8"), /no SHA-256 digest.*CAMOUFLAGE_SKIP_VERIFY=1/);
  }
});

test("CAMOUFLAGE_SKIP_VERIFY=1 installs without a digest, and only when set to 1", async (t) => {
  const logs = [];
  const dir = tempDir(t);
  const opts = installOpts(dir, {
    checksums: null,
    env: { CAMOUFLAGE_SKIP_VERIFY: "1" },
    log: (m) => logs.push(m),
  });
  assert.equal(await installBinary(opts), true);
  assert.ok(logs.some((m) => /WARNING: CAMOUFLAGE_SKIP_VERIFY=1/.test(m)));

  const other = tempDir(t);
  assert.equal(await installBinary(installOpts(other, {
    checksums: null,
    env: { CAMOUFLAGE_SKIP_VERIFY: "true" },
  })), false);
});

test("expectedDigest and verifyArchive", (t) => {
  assert.equal(expectedDigest(CHECKSUMS, "2.8.0", ARCHIVE), FAKE_DIGEST);
  assert.equal(expectedDigest({ version: "2.8.0", sha256: { [ARCHIVE]: FAKE_DIGEST.toUpperCase() } }, "2.8.0", ARCHIVE), FAKE_DIGEST);
  assert.equal(expectedDigest(CHECKSUMS, "2.8.1", ARCHIVE), null);
  assert.equal(expectedDigest(CHECKSUMS, "2.8.0", "camouflage-tui-x86_64-pc-windows-msvc.tar.gz"), null);
  assert.equal(expectedDigest(null, "2.8.0", ARCHIVE), null);

  const dir = tempDir(t);
  const file = join(dir, ARCHIVE);
  writeFileSync(file, FAKE_ARCHIVE);
  verifyArchive(file, FAKE_DIGEST);
  writeFileSync(file, `${FAKE_ARCHIVE}!`);
  assert.throws(() => verifyArchive(file, FAKE_DIGEST), /SHA-256 mismatch/);
});

test("buildChecksums records every platform archive and loadChecksums reads it back", (t) => {
  const dir = tempDir(t);
  const archives = join(dir, "archives");
  mkdirSync(archives);
  for (const target of Object.values(PLATFORM_MAP)) {
    writeFileSync(join(archives, archiveName(target)), `archive for ${target}`);
  }
  const doc = buildChecksums(archives, "9.9.9");
  assert.equal(doc.version, "9.9.9");
  assert.equal(Object.keys(doc.sha256).length, Object.keys(PLATFORM_MAP).length);
  const linux = archiveName("x86_64-unknown-linux-gnu");
  assert.equal(doc.sha256[linux], createHash("sha256").update("archive for x86_64-unknown-linux-gnu").digest("hex"));

  const out = join(dir, "checksums.json");
  writeFileSync(out, JSON.stringify(doc));
  assert.equal(expectedDigest(loadChecksums(out), "9.9.9", linux), doc.sha256[linux]);
  assert.equal(loadChecksums(join(dir, "missing.json")), null);

  rmSync(join(archives, linux));
  assert.throws(() => buildChecksums(archives, "9.9.9"), /missing release archive/);
});

test("download URLs and redirects are limited to GitHub release hosts", () => {
  for (const ok of [
    "https://github.com/sinameraji/camouflage/releases/download/v1/a.tar.gz",
    "https://objects.githubusercontent.com/github-production-release-asset/1",
    "https://release-assets.githubusercontent.com/github-production-release-asset/1",
    "https://GitHub.com:443/x",
  ]) assert.equal(isAllowedDownloadUrl(ok), true, ok);
  for (const bad of [
    "http://github.com/x",
    "https://evil.example/x",
    "https://github.com.evil.example/x",
    "https://evilgithubusercontent.com/x",
    "https://githubusercontent.com.evil.example/x",
    "https://user:pass@github.com/x",
    "https://github.com:8443/x",
    "file:///etc/passwd",
    "not a url",
  ]) assert.equal(isAllowedDownloadUrl(bad), false, bad);
});

// Minimal https.get stand-in: responds per URL from `routes`.
function fakeGet(routes, seen) {
  return (url, _opts, onResponse) => {
    seen.push(url);
    const req = new EventEmitter();
    req.setTimeout = () => req;
    const route = routes[url] || { status: 404 };
    queueMicrotask(() => {
      const res = new EventEmitter();
      res.statusCode = route.status;
      res.headers = route.location ? { location: route.location } : {};
      res.resume = () => {};
      res.pipe = (stream) => { stream.end(route.body || ""); };
      onResponse(res);
    });
    return req;
  };
}

test("download follows redirects to GitHub hosts and refuses others", async (t) => {
  const dir = tempDir(t);
  const dest = join(dir, "out.tar.gz");
  const start = "https://github.com/sinameraji/camouflage/releases/download/v1/a.tar.gz";

  const seen = [];
  await download(start, dest, 0, fakeGet({
    [start]: { status: 302, location: "https://release-assets.githubusercontent.com/asset?sig=1" },
    "https://release-assets.githubusercontent.com/asset?sig=1": { status: 200, body: "payload" },
  }, seen));
  assert.equal(readFileSync(dest, "utf8"), "payload");
  assert.equal(seen.length, 2);

  const seenBad = [];
  await assert.rejects(
    download(start, dest, 0, fakeGet({
      [start]: { status: 302, location: "https://evil.example/a.tar.gz" },
    }, seenBad)),
    /Refusing to download from https:\/\/evil\.example/,
  );
  assert.deepEqual(seenBad, [start]);

  await assert.rejects(download("https://evil.example/a.tar.gz", dest, 0, fakeGet({}, [])), /Refusing/);
});
