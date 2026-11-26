#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import { need, verifyReceipt, verifyArtifacts, versionTagName, assertNetworkAllowed } from "./release-handoff.mjs";
import { publishAndVerify, verifyRegistry } from "./registry-verification.mjs";

// Zero GitHub Release pages: the version tag and the npm registry are the
// only remote writes this publisher makes. Nothing creates, patches or
// uploads a release page or a release asset, and no existing page is read.
const repo = "nathwn12/gladius";
const env = process.env;
const gh = (args, input) => {
  assertNetworkAllowed(`gh ${args.join(" ")}`);
  return execFileSync("gh", args, { input, encoding: "utf8", windowsHide: true, maxBuffer: 64 * 1024 * 1024 });
};
const api = (suffix, data, method) => JSON.parse(gh(["api", `repos/${repo}/${suffix}`, ...(method ? ["--method", method] : []), ...(data ? ["--input", "-"] : [])], data ? JSON.stringify(data) : undefined));
const maybe = (suffix) => {
  assertNetworkAllowed(`gh api repos/${repo}/${suffix}`);
  const r = spawnSync("gh", ["api", `repos/${repo}/${suffix}`], { encoding: "utf8", windowsHide: true });
  if (r.status === 0) return JSON.parse(r.stdout);
  need(/HTTP 404/.test(r.stderr), `GitHub lookup failed: ${suffix}: ${r.stderr}`);
  return null;
};
const netFetch = (url, init) => {
  assertNetworkAllowed(`fetch ${url}`);
  return fetch(url, init);
};
const bytes = (file) => fs.readFileSync(file);
async function download(url) {
  const response = await netFetch(url);
  need(response.ok, `download HTTP ${response.status}: ${url}`);
  return Buffer.from(await response.arrayBuffer());
}
const dir = "staged";
function tagCommit(ref) {
  if (!ref) return null;
  let object = ref.object;
  for (let i = 0; object.type === "tag" && i < 5; i++) object = api(`git/tags/${object.sha}`).object;
  need(object.type === "commit", "product tag is not a commit");
  return object.sha;
}
try {
  need(env.GITHUB_REPOSITORY === repo && env.GITHUB_REF === "refs/heads/main" && env.GITHUB_EVENT_NAME === "workflow_dispatch", "publisher requires exact main workflow dispatch");
  need(env.PRODUCT_TAG === "gladius" && /^[0-9a-f]{40}$/.test(env.COMMIT) && /^\d+\.\d+\.\d+$/.test(env.VERSION), "invalid product, commit or version");
  need(env.GITHUB_SHA === env.COMMIT && gh(["api", `repos/${repo}/git/ref/heads/main`, "--jq", ".object.sha"]).trim() === env.COMMIT, "main moved since the local gate");
  need(["true", "false"].includes(env.DRY_RUN) && /^\d+$/.test(env.STAGING_RUN_ID), "invalid dry-run or staging run");
  need(["--prepare", "--publish"].includes(process.argv[2]), "expected --prepare or --publish");
  const head = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8", windowsHide: true }).trim();
  need(head === env.COMMIT, "checkout commit mismatch");
  const derived = JSON.parse(execFileSync(process.execPath, ["scripts/derive-release.mjs", "--json"], { encoding: "utf8", windowsHide: true }));
  need(derived.version === env.VERSION, "derived version mismatch");
  const run = api(`actions/runs/${env.STAGING_RUN_ID}`);
  need(run.head_sha === env.COMMIT && run.head_branch === "main" && run.event === "workflow_dispatch" && run.conclusion === "success" && run.path === ".github/workflows/stage-release.yml", "staging run identity mismatch");
  const artifacts = api(`actions/runs/${env.STAGING_RUN_ID}/artifacts`).artifacts;
  need(artifacts.length === 1 && artifacts[0].name === "release-handoff" && !artifacts[0].expired, "missing, expired or unexpected staging artifact");
  const prepare = process.argv[2] === "--prepare";
  const npmVersion = execFileSync("npm --version", { shell: true, encoding: "utf8", windowsHide: true }).trim().split(".").map(Number);
  need(npmVersion[0] > 11 || (npmVersion[0] === 11 && (npmVersion[1] > 5 || (npmVersion[1] === 5 && npmVersion[2] >= 1))), "trusted publishing requires npm >= 11.5.1");
  if (prepare) {
    need(!fs.existsSync(dir), "handoff directory already exists");
    gh(["run", "download", env.STAGING_RUN_ID, "--repo", repo, "--name", "release-handoff", "--dir", dir]);
  }
  const receipt = verifyReceipt(dir, { commit: env.COMMIT, version: env.VERSION, receiptSha256: env.RECEIPT_SHA256 });
  verifyArtifacts(dir, receipt);
  const npmArtifact = receipt.artifacts.find((a) => a.role === "npm");
  const tarball = path.resolve(dir, npmArtifact.name);
  const integrity = `sha512-${createHash("sha512").update(bytes(tarball)).digest("base64")}`;
  const registryUrl = `https://registry.npmjs.org/gladius-mcp/${env.VERSION}`;
  const existing = await netFetch(registryUrl);
  need(existing.ok || existing.status === 404, `registry lookup HTTP ${existing.status}`);
  const published = existing.ok ? await existing.json() : null;
  if (published) need(published.dist?.integrity === integrity, "immutable npm version integrity conflict");
  const versionTag = versionTagName(env.VERSION);
  const ref = maybe(`git/ref/tags/${versionTag}`);
  need(!ref || tagCommit(ref) === env.COMMIT, `conflicting version tag ${versionTag}: it binds another commit; refusing to publish a different identity`);
  if (prepare) {
    console.log(`PREPARED ${env.VERSION}: exact tarball integrity ${integrity}; dry_run=${env.DRY_RUN}`);
  } else {
    need(env.DRY_RUN === "false", "publish forbidden during dry run");
    if (!ref) api("git/refs", { ref: `refs/tags/${versionTag}`, sha: env.COMMIT });
    const verify = () => verifyRegistry({
      fetch: netFetch, version: env.VERSION, integrity, download,
      hash: (data) => `sha512-${createHash("sha512").update(data).digest("base64")}`,
    });
    if (published) await verify();
    else await publishAndVerify({
      publish: () => {
        assertNetworkAllowed("npm publish");
        const command = `npm publish "${npmArtifact.name}" --ignore-scripts`;
        const result = spawnSync(command, { shell: true, cwd: path.resolve(dir), stdio: "inherit", windowsHide: true });
        need(result.status === 0, "npm publish did not confirm success; query registry integrity before any further attempt");
      },
      verify,
    });
    console.log(`VERIFIED gladius-mcp@${env.VERSION} ${integrity}; no GitHub Release page was created or modified.`);
  }
} catch (err) {
  console.error(`publish-release: ${err.message}`);
  console.error("No automatic retry. After any write, inspect remote state and registry integrity.");
  process.exitCode = 1;
}
