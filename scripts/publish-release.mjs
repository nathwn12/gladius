#!/usr/bin/env node
// No compiler, packer or test runner here. Publish only verified local bytes.
import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import { need, sha256, safeName, verifyReceipt, verifyArtifacts, releaseIdentity } from "./release-handoff.mjs";
import { publishAndVerify, verifyRegistry } from "./registry-verification.mjs";

const repo = "nathwn12/gladius";
const env = process.env;
const gh = (args, input) => execFileSync("gh", args, { input, encoding: "utf8", windowsHide: true, maxBuffer: 64 * 1024 * 1024 });
const api = (suffix, data, method) => JSON.parse(gh(["api", `repos/${repo}/${suffix}`, ...(method ? ["--method", method] : []), ...(data ? ["--input", "-"] : [])], data ? JSON.stringify(data) : undefined));
const maybe = (suffix) => {
  const r = spawnSync("gh", ["api", `repos/${repo}/${suffix}`], { encoding: "utf8", windowsHide: true });
  if (r.status === 0) return JSON.parse(r.stdout);
  need(/HTTP 404/.test(r.stderr), `GitHub lookup failed: ${suffix}: ${r.stderr}`);
  return null;
};
const bytes = (file) => fs.readFileSync(file);
const dir = "staged";
const backupDir = "release-backup";
async function download(url) {
  const response = await fetch(url);
  need(response.ok, `download HTTP ${response.status}: ${url}`);
  return Buffer.from(await response.arrayBuffer());
}
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
    need(!fs.existsSync(dir) && !fs.existsSync(backupDir), "handoff directories already exist");
    gh(["run", "download", env.STAGING_RUN_ID, "--repo", repo, "--name", "release-handoff", "--dir", dir]);
  }
  const receipt = verifyReceipt(dir, { commit: env.COMMIT, version: env.VERSION, receiptSha256: env.RECEIPT_SHA256 });
  verifyArtifacts(dir, receipt);
  const npmArtifact = receipt.artifacts.find((a) => a.role === "npm");
  const tarball = path.resolve(dir, npmArtifact.name);
  const integrity = `sha512-${createHash("sha512").update(bytes(tarball)).digest("base64")}`;
  const registryUrl = `https://registry.npmjs.org/gladius-mcp/${env.VERSION}`;
  const existing = await fetch(registryUrl);
  need(existing.ok || existing.status === 404, `registry lookup HTTP ${existing.status}`);
  const published = existing.ok ? await existing.json() : null;
  if (published) need(published.dist?.integrity === integrity, "immutable npm version integrity conflict");
  const ref = maybe("git/ref/tags/gladius");
  need(!ref || tagCommit(ref) === env.COMMIT, "product tag movement requires a specific safety approval; publisher never force-moves refs");
  let release = maybe("releases/tags/gladius");
  if (release) need(!release.draft && !release.prerelease && /^gladius \d+\.\d+\.\d+$/.test(release.name), "unrelated product release collision");
  const immutable = receipt.artifacts.filter((a) => ["runtime", "checksum"].includes(a.role));
  for (const a of immutable) {
    const found = release?.assets.find((asset) => asset.name === a.name);
    if (found) need(found.size === a.size && found.digest === `sha256:${a.sha256}`, `immutable asset conflict: ${a.name}`);
  }
  if (prepare) {
    fs.mkdirSync(backupDir);
    fs.mkdirSync(path.join(backupDir, "assets"));
    fs.writeFileSync(path.join(backupDir, "state.json"), JSON.stringify({ ref, release }, null, 2));
    for (const a of release?.assets || []) {
      need(safeName(a.name), "unsafe existing asset name");
      const data = await download(a.browser_download_url);
      need(data.length === a.size && a.digest === `sha256:${sha256(data)}`, `backup mismatch: ${a.name}`);
      fs.writeFileSync(path.join(backupDir, "assets", a.name), data);
    }
    console.log(`PREPARED ${env.VERSION}: exact tarball integrity ${integrity}; dry_run=${env.DRY_RUN}`);
  } else {
    need(env.DRY_RUN === "false", "publish forbidden during dry run");
    const snapshot = JSON.parse(bytes(path.join(backupDir, "state.json")));
    need(JSON.stringify(snapshot.ref) === JSON.stringify(ref) && JSON.stringify(releaseIdentity(snapshot.release)) === JSON.stringify(releaseIdentity(release)), "remote state changed after backup");
    for (const a of snapshot.release?.assets || []) {
      const data = bytes(path.join(backupDir, "assets", a.name));
      need(data.length === a.size && a.digest === `sha256:${sha256(data)}`, `backup changed: ${a.name}`);
    }
    const backups = api(`actions/runs/${env.GITHUB_RUN_ID}/artifacts`).artifacts;
    need(backups.some((a) => a.name === "product-backup" && !a.expired), "verified backup was not uploaded");
    if (!ref) api("git/refs", { ref: "refs/tags/gladius", sha: env.COMMIT });
    const notes = bytes(path.join(dir, "release-notes.md")).toString();
    if (!release) release = api("releases", { tag_name: "gladius", target_commitish: env.COMMIT, name: `gladius ${env.VERSION}`, body: notes, draft: false, prerelease: false });
    // Immutable npm runtime assets are additive. Never clobber or remove them.
    for (const a of immutable) {
      if (!release.assets.some((asset) => asset.name === a.name)) gh(["release", "upload", "gladius", path.join(dir, a.name), "--repo", repo]);
    }
    const runtime = receipt.artifacts.find((a) => a.role === "runtime");
    const stable = "gladius-windows-x86_64.zip";
    const currentDir = "current-assets";
    fs.mkdirSync(currentDir);
    fs.copyFileSync(path.join(dir, runtime.name), path.join(currentDir, stable));
    fs.copyFileSync(path.join(dir, "engine-manifest.json"), path.join(currentDir, "engine-manifest.json"));
    fs.writeFileSync(path.join(currentDir, `${stable}.sha256`), `${runtime.sha256}  ${stable}\n`);
    for (const name of [stable, `${stable}.sha256`, "engine-manifest.json"]) {
      gh(["release", "upload", "gladius", path.join(currentDir, name), "--repo", repo, "--clobber"]);
      need(sha256(await download(`https://github.com/${repo}/releases/download/gladius/${name}`)) === sha256(bytes(path.join(currentDir, name))), `published asset fetch mismatch: ${name}`);
    }
    for (const a of immutable) need(sha256(await download(`https://github.com/${repo}/releases/download/gladius/${a.name}`)) === a.sha256, `immutable asset fetch mismatch: ${a.name}`);
    const verify = () => verifyRegistry({
      fetch, version: env.VERSION, integrity, download,
      hash: (data) => `sha512-${createHash("sha512").update(data).digest("base64")}`,
    });
    if (published) await verify();
    else await publishAndVerify({
      // OIDC uses the existing release.yml publisher identity. Never repack.
      publish: () => {
        const command = `npm publish "${npmArtifact.name}" --ignore-scripts`;
        const result = spawnSync(command, { shell: true, cwd: path.resolve(dir), stdio: "inherit", windowsHide: true });
        need(result.status === 0, "npm publish did not confirm success; query registry integrity before any further attempt");
      },
      verify,
    });
    const current = api(`releases/${release.id}`, { name: `gladius ${env.VERSION}`, body: notes, make_latest: "true" }, "PATCH");
    need(current.tag_name === "gladius" && current.name === `gladius ${env.VERSION}` && current.body === notes, "published release metadata mismatch");
    console.log(`VERIFIED gladius-mcp@${env.VERSION} ${integrity}; historical releases and tags untouched.`);
  }
} catch (err) {
  console.error(`publish-release: ${err.message}`);
  console.error("No automatic retry. After any write, inspect remote state and registry integrity.");
  process.exitCode = 1;
}
