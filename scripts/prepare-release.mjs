#!/usr/bin/env node
// Build once, test the final package, write a receipt only after every gate.
import fs from "node:fs";
import path from "node:path";
import { execFileSync, execSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { need, sha256, requiredChecks, releaseNotes, verifyArtifacts, releaseEnvironment, assembleRuntime } from "./release-handoff.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
process.chdir(root);
const args = process.argv.slice(2);
const arg = (name) => args[args.indexOf(name) + 1];
const read = (file) => fs.readFileSync(file);
const json = (file) => JSON.parse(read(file));
let childEnv = releaseEnvironment(null);
const run = (exe, argv, env = childEnv) => execFileSync(exe, argv, { cwd: root, env, windowsHide: true, stdio: "inherit", timeout: 1800000 });
const capture = (exe, argv) => execFileSync(exe, argv, { cwd: root, windowsHide: true, encoding: "utf8" }).trim();
let work;
let createdWork = false;
try {
  need(args.length === 4 && args.includes("--commit") && args.includes("--out"), "usage: prepare-release.mjs --commit <40-hex> --out <new-directory>");
  const commit = arg("--commit");
  need(/^[0-9a-f]{40}$/.test(commit) && commit === capture("git", ["rev-parse", "HEAD"]), "commit must equal exact HEAD");
  need(capture("git", ["status", "--porcelain"]) === "", "release requires a clean working tree");
  need(process.platform === "win32", "local release requires Windows");
  need(capture("rustc", ["--version"]).startsWith("rustc 1.88.0 "), "release requires pinned Rust 1.88.0");
  const out = path.resolve(arg("--out"));
  work = `${out}.work`;
  need(!fs.existsSync(out) && !fs.existsSync(work), "output and work directories must not exist");
  fs.mkdirSync(out, { recursive: true });
  fs.mkdirSync(work);
  createdWork = true;
  childEnv = releaseEnvironment(path.join(work, "chromium"));
  const checks = [];
  const gate = (name, fn) => { fn(); checks.push({ name, result: "pass" }); console.log(`GATE ${name}: PASS`); };
  let version;
  gate("derive", () => {
    run(process.execPath, ["scripts/derive-release.mjs", "--check"]);
    version = JSON.parse(capture(process.execPath, ["scripts/derive-release.mjs", "--json"])).version;
    need(json("npm/package.json").version === version && json("npm/engine-manifest.json").serverVersion === version, "derived version drift");
  });
  gate("fmt", () => run("cargo", ["fmt", "--check"]));
  gate("clippy", () => run("cargo", ["clippy", "--all-targets", "--locked", "--", "-D", "warnings"]));
  gate("browser", () => run("pwsh", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "scripts/fetch-chromium.ps1"]));
  gate("build", () => run("cargo", ["build", "--release", "--locked"]));
  gate("test", () => {
    run(process.execPath, ["--test", "scripts/release-handoff.test.mjs"]);
    run(process.execPath, ["scripts/check-release-workflow.mjs"]);
    run("cargo", ["test", "--release", "--locked"]);
  });
  gate("selftest", () => run(path.join(root, "target/release/gladius.exe"), ["selftest"]));

  const manifest = json("npm/engine-manifest.json");
  const browser = json("scripts/chromium-manifest.json").browser;
  need(JSON.stringify(manifest.browser) === JSON.stringify(browser), "browser manifest drift");
  const provisioner = read("scripts/fetch-chromium.ps1").toString();
  for (const value of [browser.version, browser.asset.url, browser.asset.sha256, browser.asset.size]) need(provisioner.includes(String(value)), "provisioner browser pin drift");
  const cache = path.join(childEnv.GLADIUS_CHROMIUM_CACHE, `${browser.version}-${browser.asset.sha256.slice(0, 8)}`);
  for (const entry of browser.files) {
    const bytes = read(path.join(cache, entry.name));
    need(bytes.length === entry.size && sha256(bytes) === entry.sha256, `browser file drift: ${entry.name}`);
  }
  const bundle = `gladius-v${version}-windows-x86_64.zip`;
  const stage = path.join(work, "runtime");
  fs.mkdirSync(path.join(stage, "THIRD-PARTY-LICENSES"), { recursive: true });
  for (const file of ["LICENSE", "THIRD-PARTY-NOTICES.md", "README.md"]) fs.copyFileSync(file, path.join(stage, file));
  fs.copyFileSync("target/release/gladius.exe", path.join(stage, "gladius.exe"));
  fs.copyFileSync(path.join(cache, browser.root, "LICENSE.headless_shell"), path.join(stage, "THIRD-PARTY-LICENSES/LICENSE.headless_shell"));
  assembleRuntime(stage, path.join(out, bundle));
  const archive = read(path.join(out, bundle));
  manifest.commit = commit;
  manifest.runtime.source = "https://github.com/nathwn12/gladius/releases/download/gladius";
  manifest.runtime.archive = { name: bundle, size: archive.length, sha256: sha256(archive) };
  const server = read("target/release/gladius.exe");
  manifest.runtime.server.sha256 = sha256(server);
  manifest.runtime.server.size = server.length;
  fs.writeFileSync(path.join(out, "engine-manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
  fs.writeFileSync(path.join(out, `${bundle}.sha256`), `${sha256(archive)}  ${bundle}\n`);
  const pkg = path.join(work, "npm");
  fs.cpSync("npm", pkg, { recursive: true });
  fs.copyFileSync(path.join(out, "engine-manifest.json"), path.join(pkg, "engine-manifest.json"));
  // Constant command, generated working directory: no caller-controlled shell arguments.
  const packed = JSON.parse(execSync("npm pack --json ./npm", { cwd: work, windowsHide: true, encoding: "utf8" }));
  const entry = Array.isArray(packed) ? packed[0] : Object.values(packed)[0];
  need(entry.filename === `gladius-mcp-${version}.tgz`, "unexpected npm tarball name");
  const tarball = path.join(out, entry.filename);
  fs.copyFileSync(path.join(work, entry.filename), tarball);
  gate("release-check", () => {
    execFileSync(process.execPath, ["scripts/release-check.mjs", "--compare", path.join(out, "engine-manifest.json")], {
      cwd: root, windowsHide: true, stdio: "inherit", env: { ...process.env, RELEASE_CHECK_TARBALL: tarball },
    });
  });
  gate("cleanroom", () => run(process.execPath, ["scripts/cleanroom-check.mjs", "--tarball", tarball, "--bundle", path.join(out, bundle)], releaseEnvironment(null)));
  fs.writeFileSync(path.join(out, "release-notes.md"), releaseNotes(version, sha256(archive)));
  need(capture("git", ["rev-parse", "HEAD"]) === commit && capture("git", ["status", "--porcelain"]) === "", "source changed during release gate");
  need(requiredChecks.every((name) => checks.some((c) => c.name === name)), "incomplete local gate");
  const artifacts = [
    ["runtime", bundle], ["npm", entry.filename], ["manifest", "engine-manifest.json"],
    ["checksum", `${bundle}.sha256`], ["notes", "release-notes.md"],
  ].map(([role, name]) => { const bytes = read(path.join(out, name)); return { role, name, size: bytes.length, sha256: sha256(bytes) }; });
  const receipt = { schemaVersion: 1, repository: "nathwn12/gladius", productTag: "gladius", version, commit, checks, artifacts };
  verifyArtifacts(out, receipt);
  fs.writeFileSync(path.join(out, "release-receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.log(`Receipt SHA-256: ${sha256(read(path.join(out, "release-receipt.json")))}`);
} catch (err) {
  console.error(`prepare-release: ${err.message}`);
  process.exitCode = 1;
} finally {
  if (createdWork) fs.rmSync(work, { recursive: true, force: true });
}
