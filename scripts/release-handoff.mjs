import { createHash } from "node:crypto";
import { readFileSync, readdirSync, lstatSync } from "node:fs";
import { join } from "node:path";
import { execFileSync } from "node:child_process";

export const requiredChecks = [
  "derive", "fmt", "clippy", "browser", "build", "test", "selftest", "release-check", "cleanroom",
];
export const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
export const need = (condition, message) => { if (!condition) throw new Error(message); };
export const safeName = (name) => typeof name === "string" && /^[a-zA-Z0-9][a-zA-Z0-9._-]*$/.test(name) && name !== "." && name !== "..";

export class OfflineRefusal extends Error {
  constructor(route) {
    super(`OC_NO_NETWORK guard: refused network call to ${route}`);
    this.name = "OfflineRefusal";
    this.code = "OC_NO_NETWORK";
  }
}

export const assertNetworkAllowed = (route) => {
  if (process.env.OC_NO_NETWORK === "1") throw new OfflineRefusal(route);
};

export function releaseEnvironment(cache, parent = process.env) {
  const env = Object.fromEntries(Object.entries(parent).filter(([name]) => !/^GLADIUS_/i.test(name)));
  if (cache) env.GLADIUS_CHROMIUM_CACHE = cache;
  return env;
}

export function assembleRuntime(stage, destination) {
  execFileSync("pwsh", [
    "-NoProfile", "-Command",
    "$ErrorActionPreference = 'Stop'; Compress-Archive -Path (Join-Path $env:RELEASE_RUNTIME_STAGE '*') -DestinationPath $env:RELEASE_RUNTIME_ZIP",
  ], { windowsHide: true, stdio: "inherit", env: { ...process.env, RELEASE_RUNTIME_STAGE: stage, RELEASE_RUNTIME_ZIP: destination } });
}

export function validateBlobMap(text) {
  const map = JSON.parse(text);
  need(map && !Array.isArray(map) && typeof map === "object" && Object.hasOwn(map, "release-receipt.json"), "receipt blob missing");
  need(Object.keys(map).length <= 6, "unexpected blob count");
  for (const [name, hash] of Object.entries(map)) need(safeName(name) && /^[0-9a-f]{40}$/.test(hash), "unsafe blob mapping");
  return map;
}

// Historical only: the release-page helpers below are retained for reading old
// fixtures. No lane here creates, patches or uploads a GitHub Release page.
export function releaseIdentity(release) {
  if (!release) return null;
  const { id, tag_name, name, body, draft, prerelease } = release;
  const assets = release.assets.map(({ id, name, size, digest }) => ({ id, name, size, digest }));
  return { id, tag_name, name, body, draft, prerelease, assets };
}

export function resolveTagCommit(ref, readTagObject) {
  if (!ref) return null;
  need(ref.object && typeof ref.object === "object", "malformed product tag ref");
  let object = ref.object;
  for (let i = 0; object.type === "tag" && i < 5; i++) object = readTagObject(object.sha);
  need(object.type === "commit" && /^[0-9a-f]{40}$/.test(object.sha), "product tag is not a commit");
  return object.sha;
}

export const versionTagName = (version) => `v${version}`;

/// Prove a release-page mutation only appended the versioned asset. Every
/// pre-existing asset must survive byte-identical, and each expected new
/// asset must land with the exact size and digest from the receipt. A missing
/// asset or a changed byte fails closed instead of reading as a green publish.
export function verifyReleaseAppend(before, after, appended) {
  need(before && after, "release page missing before or after append");
  need(before.id === after.id && before.tag_name === after.tag_name, "release page identity changed");
  need(!after.draft && !after.prerelease, "release page is not a published page");
  const now = new Map((after.assets ?? []).map((a) => [a.name, a]));
  for (const old of before.assets ?? []) {
    const kept = now.get(old.name);
    need(kept && kept.size === old.size && kept.digest === old.digest, `existing asset changed or vanished: ${old.name}`);
  }
  for (const a of appended) {
    const landed = now.get(a.name);
    need(landed && landed.size === a.size && landed.digest === a.digest, `versioned asset not appended: ${a.name}`);
  }
  return after;
}

export function releasePreflight({ commit, main, tag, readTagObject }) {
  need(/^[0-9a-f]{40}$/.test(commit), "invalid planned commit");
  need(/^[0-9a-f]{40}$/.test(main) && main === commit, "landed mismatch: live main is not the planned commit; land the exact tested commit on main before staging");
  const bound = resolveTagCommit(tag, readTagObject);
  need(!bound || bound === commit, "conflicting version tag: it binds another commit; refusing to publish a different identity");
  return { commit, main, tag: bound };
}

export function verifyReceipt(dir, expected) {
  need(/^[0-9a-f]{64}$/.test(expected.receiptSha256), "invalid receipt digest");
  const bytes = readFileSync(join(dir, "release-receipt.json"));
  need(sha256(bytes) === expected.receiptSha256, "receipt digest mismatch");
  const r = JSON.parse(bytes);
  need(r.schemaVersion === 1 && r.repository === "nathwn12/gladius" && r.productTag === "gladius", "receipt identity mismatch");
  need(/^[0-9a-f]{40}$/.test(r.commit) && r.commit === expected.commit, "receipt commit mismatch");
  need(/^\d+\.\d+\.\d+$/.test(r.version) && r.version === expected.version, "receipt version mismatch");
  need(Array.isArray(r.checks) && r.checks.length === requiredChecks.length, "required checks missing");
  need(new Set(r.checks.map((c) => c.name)).size === requiredChecks.length, "duplicate checks");
  for (const name of requiredChecks) need(r.checks.some((c) => c.name === name && c.result === "pass"), `required check ${name} missing`);
  const roles = ["runtime", "npm", "manifest", "checksum", "notes"];
  need(Array.isArray(r.artifacts) && r.artifacts.length === roles.length, "required artifacts missing");
  const names = new Set(["release-receipt.json"]);
  for (const role of roles) need(r.artifacts.filter((a) => a.role === role).length === 1, `artifact role ${role} missing or duplicate`);
  for (const a of r.artifacts) {
    need(safeName(a.name) && !names.has(a.name), "unsafe or duplicate artifact name");
    names.add(a.name);
    const file = join(dir, a.name);
    need(lstatSync(file).isFile(), "artifact is not a regular file");
    const data = readFileSync(file);
    need(Number.isSafeInteger(a.size) && a.size > 0 && a.size === data.length, `artifact size mismatch: ${a.name}`);
    need(/^[0-9a-f]{64}$/.test(a.sha256) && sha256(data) === a.sha256, `artifact hash mismatch: ${a.name}`);
  }
  need(readdirSync(dir).every((name) => names.has(name)), "unexpected staged file");
  return r;
}

export function releaseNotes(version, hash) {
  const asset = `gladius-v${version}-windows-x86_64.zip`;
  return `gladius ${version}\n\nGladius provides headless browser tools over MCP.\n\nInstall: npx gladius-mcp@${version} (npm is the release; no GitHub Release page). This version's tarball bundles ${asset}.\nSHA-256: ${hash} (${asset}, bundled under bin/)\n`;
}

export function verifyArtifacts(dir, r) {
  const artifact = (role) => r.artifacts.find((a) => a.role === role);
  const runtime = artifact("runtime");
  need(runtime.name === `gladius-v${r.version}-windows-x86_64.zip`, "immutable archive name required");
  need(artifact("npm").name === `gladius-mcp-${r.version}.tgz`, "npm artifact name mismatch");
  need(artifact("manifest").name === "engine-manifest.json" && artifact("notes").name === "release-notes.md", "metadata artifact name mismatch");
  need(artifact("checksum").name === `${runtime.name}.sha256`, "checksum artifact name mismatch");
  const tar = (file, args) => execFileSync("tar", [...args, join(dir, file)], { windowsHide: true, maxBuffer: 128 * 1024 * 1024 });
  const packed = (member) => execFileSync("tar", ["-xzf", join(dir, artifact("npm").name), "-O", member], { windowsHide: true, maxBuffer: 256 * 1024 * 1024 });
  const pkg = JSON.parse(packed("package/package.json"));
  const manifest = JSON.parse(readFileSync(join(dir, "engine-manifest.json")));
  need(JSON.stringify(JSON.parse(packed("package/engine-manifest.json"))) === JSON.stringify(manifest), "packaged manifest mismatch");
  need(pkg.name === "gladius-mcp" && pkg.version === r.version && manifest.serverVersion === r.version && manifest.commit === r.commit, "package identity mismatch");
  need(manifest.runtime.source === `https://github.com/nathwn12/gladius/releases/download/v${r.version}`, "runtime legacy URL marker mismatch");
  need(manifest.runtime.archive.name === runtime.name && manifest.runtime.archive.size === runtime.size && manifest.runtime.archive.sha256 === runtime.sha256, "manifest archive mismatch");
  // npm is the release: the runtime zip must ride inside the tarball under
  // bin/, byte-identical to the staged runtime artifact.
  const bundled = packed(`package/bin/${runtime.name}`);
  need(Buffer.isBuffer(bundled) && bundled.length === runtime.size && sha256(bundled) === runtime.sha256, "bundled runtime zip missing from npm tarball or byte mismatch");
  const browser = JSON.parse(readFileSync(new URL("./chromium-manifest.json", import.meta.url))).browser;
  need(JSON.stringify(manifest.browser) === JSON.stringify(browser), "packaged browser pin drift");
  const members = tar(runtime.name, ["-tf"]).toString().trim().split(/\r?\n/);
  need(members.every((name) => !name.startsWith("/") && !name.includes("\\") && !name.split("/").includes("..")), "unsafe archive member");
  const serverMember = members.find((name) => name === "gladius.exe" || name === "./gladius.exe");
  need(serverMember, "runtime server missing");
  const server = execFileSync("tar", ["-xf", join(dir, runtime.name), "-O", serverMember], { windowsHide: true, maxBuffer: 64 * 1024 * 1024 });
  need(manifest.runtime.server.name === "gladius.exe" && manifest.runtime.server.size === server.length && manifest.runtime.server.sha256 === sha256(server), "runtime server mismatch");
  need(readFileSync(join(dir, artifact("checksum").name), "utf8").trim() === `${runtime.sha256}  ${runtime.name}`, "checksum mismatch");
  need(readFileSync(join(dir, "release-notes.md"), "utf8") === releaseNotes(r.version, runtime.sha256), "release notes mismatch");
  return manifest;
}
