import { test } from "node:test";
import assert from "node:assert/strict";
import { publishAndVerify, verifyRegistry } from "./registry-verification.mjs";

const version = "0.8.0";
const integrity = "sha512-fixture";
const metadata = { version, dist: { integrity, tarball: "https://registry.test/package.tgz" } };

test("registry verification waits for delayed exact-version and latest visibility", async () => {
  const seen = { exact: 0, latest: 0 };
  const fetch = async (url) => {
    if (url.endsWith("/latest")) {
      seen.latest++;
      return seen.latest < 3 ? new Response("", { status: 404 }) : Response.json(metadata);
    }
    seen.exact++;
    return seen.exact < 2 ? new Response("", { status: 404 }) : Response.json(metadata);
  };
  const result = await verifyRegistry({ fetch, version, integrity, attempts: 3, intervalMs: 0, download: async () => Buffer.from("tarball"), hash: () => integrity });
  assert.equal(result.version, version);
  assert.deepEqual(seen, { exact: 3, latest: 3 });
});

test("registry timeout fails closed after exactly one publish", async () => {
  let publishes = 0;
  const fetch = async () => new Response("", { status: 404 });
  await assert.rejects(publishAndVerify({
    publish: () => publishes++,
    verify: () => verifyRegistry({ fetch, version, integrity, attempts: 2, intervalMs: 0, download: async () => Buffer.from("unused"), hash: () => integrity }),
  }), /registry verification timed out/);
  assert.equal(publishes, 1);
});
