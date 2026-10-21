#!/usr/bin/env node
// Local release preflight: pack the npm package and assert it is publishable.
// Usage: node scripts/release-check.mjs [--allow-placeholders] [--compare <manifest.json>]
//   RELEASE_CHECK_TARBALL=<path> overrides npm pack (test hook: validate a fixture tarball).
// No dependencies beyond Node stdlib plus the system `tar` binary.
import { execFileSync, execSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const npmDir = join(root, "npm");
const allowPlaceholders = process.argv.includes("--allow-placeholders");
const compareIdx = process.argv.indexOf("--compare");
const comparePath = compareIdx >= 0 ? process.argv[compareIdx + 1] : null;

const rows = [];
const fail = (check, detail) => rows.push([check, "FAIL", detail]);
const pass = (check, detail) => rows.push([check, "PASS", detail]);

function printAndExit(code) {
  const w = (s, n) => s.padEnd(n);
  console.log(`${w("CHECK", 18)}${w("RESULT", 8)}DETAIL`);
  for (const [c, r, d] of rows) console.log(`${w(c, 18)}${w(r, 8)}${d}`);
  try { rmSync(tmp, { recursive: true, force: true }); } catch { /* ignore */ }
  process.exit(code);
}

// Fail closed: tar missing or non-zero exit is a hard FAIL, never "nothing found".
function tarList(tarball) {
  try {
    const out = execFileSync("tar", ["-tzf", tarball], { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
    return out.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
  } catch (err) {
    const msg = String(err?.message ?? err).split("\n")[0];
    fail("tarball-extract", `tar -tzf failed: ${msg}`);
    printAndExit(1);
  }
}

function tarExtract(tarball, member) {
  try {
    return execFileSync("tar", ["-xzf", tarball, "-O", member], { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  } catch (err) {
    return null;
  }
}

if (compareIdx >= 0 && !comparePath) {
  const t = mkdtempSync(join(tmpdir(), "release-check-"));
  try { rmSync(t, { recursive: true, force: true }); } catch { /* ignore */ }
  console.log(`${"CHECK".padEnd(18)}${"RESULT".padEnd(8)}DETAIL`);
  console.log(`${"manifest-compare".padEnd(18)}${"FAIL".padEnd(8)}--compare requires a <manifest.json> path`);
  process.exit(1);
}

let tarballPath = process.env.RELEASE_CHECK_TARBALL || null;
const tmp = mkdtempSync(join(tmpdir(), "release-check-"));
if (!tarballPath) {
  try {
    const out = execSync(`npm pack --json --pack-destination "${tmp}"`, {
      cwd: npmDir, encoding: "utf8",
    });
    const parsed = JSON.parse(out);
    const entry = Array.isArray(parsed) ? parsed[0] : Object.values(parsed)[0];
    const filename = entry?.filename;
    if (!filename) throw new Error("npm pack --json returned no filename");
    tarballPath = join(tmp, filename);
  } catch (err) {
    fail("npm-pack", String(err?.message ?? err).split("\n")[0]);
    printAndExit(1);
  }
}

let names;
let manifestText = null;
try {
  const rawNames = tarList(tarballPath);
  names = rawNames.map((n) => n.replace(/^package\//, "")).filter(Boolean);
  if (rawNames.includes("package/engine-manifest.json")) {
    manifestText = tarExtract(tarballPath, "package/engine-manifest.json");
    if (manifestText === null) {
      fail("tarball-extract", "tar -xzf package/engine-manifest.json failed");
      printAndExit(1);
    }
  }
} catch (err) {
  fail("tarball-extract", String(err?.message ?? err).split("\n")[0]);
  printAndExit(1);
}

const pkg = JSON.parse(readFileSync(join(npmDir, "package.json"), "utf8"));

if (manifestText === null) {
  fail("manifest-present", "engine-manifest.json missing from tarball");
} else {
  pass("manifest-present", "engine-manifest.json in tarball");
  let man;
  try {
    man = JSON.parse(manifestText);
  } catch {
    fail("manifest-json", "engine-manifest.json is not valid JSON");
  }
  if (man) {
    if (man.serverVersion === pkg.version) pass("serverVersion", `manifest == package == ${pkg.version}`);
    else fail("serverVersion", `manifest ${man.serverVersion} != package ${pkg.version}`);
    const count = manifestText.split("RELEASE_WORKFLOW_FILLS_THIS").length - 1;
    if (count === 0) pass("placeholders", "0 placeholders");
    else if (allowPlaceholders) pass("placeholders", `${count} placeholder(s) allowed by flag`);
    else fail("placeholders", `${count} RELEASE_WORKFLOW_FILLS_THIS (workflow fills at release)`);
    if (comparePath) {
      try {
        const ref = JSON.parse(readFileSync(resolve(comparePath), "utf8"));
        const a = JSON.stringify(man);
        const b = JSON.stringify(ref);
        if (a === b) pass("manifest-compare", `tarball manifest deep-equals ${comparePath}`);
        else fail("manifest-compare", `tarball manifest differs from ${comparePath}`);
      } catch (err) {
        fail("manifest-compare", String(err?.message ?? err).split("\n")[0]);
      }
    }
  }
}

const hit = (pred) => names.filter(pred);
const exe = hit((n) => n.toLowerCase().endsWith(".exe"));
const env = hit((n) => n.split("/").some((s) => s === ".env" || s.toLowerCase().endsWith(".env")));
const secret = hit((n) => n.toLowerCase().includes("secret"));
const modules = hit((n) => n === "node_modules" || n.startsWith("node_modules/"));

if (exe.length === 0) pass("no-exe", "no .exe in tarball");
else fail("no-exe", exe.slice(0, 5).join(", "));
if (env.length === 0) pass("no-env", "no .env in tarball");
else fail("no-env", env.slice(0, 5).join(", "));
if (secret.length === 0) pass("no-secret", "no *secret* in tarball");
else fail("no-secret", secret.slice(0, 5).join(", "));
if (modules.length === 0) pass("no-node_modules", "no node_modules in tarball");
else fail("no-node_modules", modules.slice(0, 5).join(", "));

printAndExit(rows.some((r) => r[1] === "FAIL") ? 1 : 0);
