import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { verifyReceipt, requiredChecks, verifyArtifacts, validateBlobMap, releaseIdentity } from "./release-handoff.mjs";

test("handoff accepts exact bytes and refuses corruption, stale identity and incomplete gates", () => {
  const dir = mkdtempSync(join(tmpdir(), "gladius-handoff-"));
  try {
    const commit = "a".repeat(40);
    const artifacts = ["runtime", "npm", "manifest", "checksum", "notes"].map((role) => {
      const name = `${role}.txt`;
      writeFileSync(join(dir, name), role);
      return { role, name, size: Buffer.byteLength(role), sha256: createHash("sha256").update(role).digest("hex") };
    });
    const receipt = {
      schemaVersion: 1, repository: "nathwn12/gladius", productTag: "gladius", commit, version: "0.8.0",
      checks: requiredChecks.map((name) => ({ name, result: "pass" })), artifacts,
    };
    const receiptPath = join(dir, "release-receipt.json");
    const save = (value) => {
      writeFileSync(receiptPath, JSON.stringify(value));
      return createHash("sha256").update(readFileSync(receiptPath)).digest("hex");
    };
    const expected = { commit, version: "0.8.0", receiptSha256: save(receipt) };
    assert.equal(verifyReceipt(dir, expected).version, "0.8.0");
    writeFileSync(join(dir, "runtime.txt"), "changed");
    assert.throws(() => verifyReceipt(dir, expected), /artifact/);
    writeFileSync(join(dir, "runtime.txt"), "runtime");
    assert.throws(() => verifyReceipt(dir, { ...expected, commit: "b".repeat(40) }), /commit/);
    assert.throws(() => verifyReceipt(dir, { ...expected, version: "0.9.0" }), /version/);
    assert.throws(() => verifyReceipt(dir, { ...expected, receiptSha256: "0".repeat(64) }), /receipt digest/);
    for (const mutate of [
      (r) => { r.checks.pop(); },
      (r) => { r.checks[0].result = "fail"; },
      (r) => { r.artifacts[0].name = "../escape"; },
      (r) => { r.artifacts.push(r.artifacts[0]); },
      (r) => { r.repository = "someone/else"; },
      (r) => { r.productTag = "other"; },
    ]) {
      const bad = structuredClone(receipt);
      mutate(bad);
      const digest = save(bad);
      assert.throws(() => verifyReceipt(dir, { ...expected, receiptSha256: digest }));
    }
    expected.receiptSha256 = save(receipt);
    writeFileSync(join(dir, "unlisted"), "extra");
    assert.throws(() => verifyReceipt(dir, expected), /unexpected/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("artifact contract rejects a mutable archive name before publication", () => {
  const r = { version: "0.8.0", artifacts: [
    { role: "runtime", name: "gladius-windows-x86_64.zip" },
    { role: "npm", name: "gladius-mcp-0.8.0.tgz" },
    { role: "manifest", name: "engine-manifest.json" },
    { role: "checksum", name: "gladius-windows-x86_64.zip.sha256" },
    { role: "notes", name: "release-notes.md" },
  ] };
  assert.throws(() => verifyArtifacts(".", r), /immutable archive name/);
});

test("local preparation refuses a different commit before producing a receipt", () => {
  const dir = mkdtempSync(join(tmpdir(), "gladius-prepare-"));
  try {
    const r = spawnSync(process.execPath, ["scripts/prepare-release.mjs", "--commit", "0".repeat(40), "--out", join(dir, "release")], { encoding: "utf8", windowsHide: true });
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /exact HEAD/);
    assert.throws(() => readFileSync(join(dir, "release", "release-receipt.json")));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("staging refuses unsafe blob inputs before network access", () => {
  assert.deepEqual(validateBlobMap('{"release-receipt.json":"' + "a".repeat(40) + '"}'), { "release-receipt.json": "a".repeat(40) });
  for (const value of ['{"../escape":"' + "a".repeat(40) + '"}', '{"release-receipt.json":"not-a-sha"}', "[]", "{}"]) {
    assert.throws(() => validateBlobMap(value));
  }
});

test("publishing workflow never rebuilds, deletes history or forces a shared tag", () => {
  const text = readFileSync(".github/workflows/release.yml", "utf8");
  assert.doesNotMatch(text, /cargo build|cargo test|release delete|git push|overwrite_files: true/);
  assert.match(text, /staging_run_id:/);
  assert.match(text, /receipt_sha256:/);
  assert.match(text, /scripts\/publish-release\.mjs/);
});

test("publisher refuses a local or wrong-ref invocation before network access", () => {
  const r = spawnSync(process.execPath, ["scripts/publish-release.mjs", "--prepare"], {
    encoding: "utf8", windowsHide: true,
    env: { ...process.env, GITHUB_REPOSITORY: "nathwn12/gladius", GITHUB_REF: "refs/heads/not-main" },
  });
  assert.notEqual(r.status, 0);
  assert.match(r.stderr, /exact main workflow dispatch/);
});

test("backup identity ignores download counters but detects asset replacement", () => {
  const release = { id: 1, tag_name: "gladius", name: "gladius 0.8.0", body: "notes", assets: [{ id: 2, name: "runtime.zip", size: 3, digest: "sha256:abc", download_count: 1 }] };
  const downloaded = structuredClone(release);
  downloaded.assets[0].download_count++;
  assert.deepEqual(releaseIdentity(downloaded), releaseIdentity(release));
  downloaded.assets[0].digest = "sha256:changed";
  assert.notDeepEqual(releaseIdentity(downloaded), releaseIdentity(release));
});
