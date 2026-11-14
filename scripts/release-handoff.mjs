// Local receipts assert checks; hashes prove byte continuity, not compiler provenance.
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

export function validateBlobMap(text) {
  const map = JSON.parse(text);
  need(map && !Array.isArray(map) && typeof map === "object" && Object.hasOwn(map, "release-receipt.json"), "receipt blob missing");
  need(Object.keys(map).length <= 6, "unexpected blob count");
  for (const [name, hash] of Object.entries(map)) need(safeName(name) && /^[0-9a-f]{40}$/.test(hash), "unsafe blob mapping");
  return map;
}

export function releaseIdentity(release) {
  if (!release) return null;
  const { id, tag_name, name, body, draft, prerelease } = release;
  const assets = release.assets.map(({ id, name, size, digest }) => ({ id, name, size, digest }));
  return { id, tag_name, name, body, draft, prerelease, assets };
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
  const stable = "gladius-windows-x86_64.zip";
  return `gladius ${version}\n\nGladius provides headless browser tools over MCP.\n\nDownload: https://github.com/nathwn12/gladius/releases/download/gladius/${stable}\nSHA-256: ${hash} (${stable})\n`;
}

export function verifyArtifacts(dir, r) {
  const artifact = (role) => r.artifacts.find((a) => a.role === role);
  const runtime = artifact("runtime");
  need(runtime.name === `gladius-v${r.version}-windows-x86_64.zip`, "immutable archive name required");
  need(artifact("npm").name === `gladius-mcp-${r.version}.tgz`, "npm artifact name mismatch");
  need(artifact("manifest").name === "engine-manifest.json" && artifact("notes").name === "release-notes.md", "metadata artifact name mismatch");
  need(artifact("checksum").name === `${runtime.name}.sha256`, "checksum artifact name mismatch");
  const tar = (file, args) => execFileSync("tar", [...args, join(dir, file)], { windowsHide: true, maxBuffer: 128 * 1024 * 1024 });
  const packed = (member) => execFileSync("tar", ["-xzf", join(dir, artifact("npm").name), "-O", member], { windowsHide: true, maxBuffer: 16 * 1024 * 1024 });
  const pkg = JSON.parse(packed("package/package.json"));
  const manifest = JSON.parse(readFileSync(join(dir, "engine-manifest.json")));
  need(JSON.stringify(JSON.parse(packed("package/engine-manifest.json"))) === JSON.stringify(manifest), "packaged manifest mismatch");
  need(pkg.name === "gladius-mcp" && pkg.version === r.version && manifest.serverVersion === r.version && manifest.commit === r.commit, "package identity mismatch");
  need(manifest.runtime.source === "https://github.com/nathwn12/gladius/releases/download/gladius", "runtime product URL mismatch");
  need(manifest.runtime.archive.name === runtime.name && manifest.runtime.archive.size === runtime.size && manifest.runtime.archive.sha256 === runtime.sha256, "manifest archive mismatch");
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
