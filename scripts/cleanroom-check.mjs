#!/usr/bin/env node
'use strict';
//
// Clean-room gate: install the package the way a stranger does — in a
// scrubbed profile with empty caches — then prove the engine provisions
// and the server boots. Prints a PASS/FAIL table; any FAIL exits 1.
//
//   node scripts/cleanroom-check.mjs --tarball <path.tgz> [--bundle <path.zip>]
//   node scripts/cleanroom-check.mjs --registry --version <semver> [--keep]
//
// --bundle points the launcher (GLADIUS_ENGINE_BUNDLE) at a locally built
// runtime archive instead of the Release download, so the gate can run BEFORE
// that Release exists. It applies to --tarball mode only; the launcher still
// checks the archive's size and sha256 against the installed manifest, so a
// wrong zip fails this gate exactly like a bad download would. --registry mode
// scrubs the override, so the public gate can never be bypassed.
//
// The boot itself still provisions the pinned Chromium from Google's
// Chrome-for-Testing endpoint on first run: that part of the gate stays
// network-dependent whatever the local runtime bundle says.
//
// Node stdlib only. Windows-only.
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

if (process.platform !== 'win32') {
  console.error('cleanroom-check: error: Windows-only (this gate runs on windows-latest).');
  process.exit(1);
}

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const BOOT_TIMEOUT_MS = 300 * 1000;

const USAGE = 'usage: cleanroom-check.mjs --tarball <path.tgz> [--bundle <path.zip>] | --registry --version <semver> [--keep]';
let tarball = null;
let registry = false;
let version = null;
let keep = false;
let bundle = null;
for (let i = 2; i < process.argv.length; i++) {
  const a = process.argv[i];
  if (a === '--tarball') tarball = process.argv[++i];
  else if (a === '--registry') registry = true;
  else if (a === '--version') version = process.argv[++i];
  else if (a === '--bundle') {
    bundle = process.argv[++i];
    if (!bundle || bundle.startsWith('--')) {
      console.error('cleanroom-check: error: --bundle needs a <path.zip> value.');
      console.error(USAGE);
      process.exit(1);
    }
  } else if (a === '--keep') keep = true;
  else {
    console.error(`cleanroom-check: error: unknown arg '${a}'.`);
    console.error(USAGE);
    process.exit(1);
  }
}
if ((tarball && registry) || (!tarball && !registry)) {
  console.error('cleanroom-check: error: pass exactly one of --tarball <path.tgz> or --registry --version <semver>.');
  process.exit(1);
}
if (registry && !version) {
  console.error('cleanroom-check: error: --registry requires --version <semver>.');
  process.exit(1);
}
if (tarball && version) {
  console.error('cleanroom-check: error: --version is only for --registry mode.');
  process.exit(1);
}
if (tarball && !fs.existsSync(path.resolve(tarball))) {
  console.error(`cleanroom-check: error: tarball not found: ${tarball}`);
  process.exit(1);
}
if (tarball) {
  try {
    if (!fs.statSync(path.resolve(tarball)).isFile()) {
      console.error(`cleanroom-check: error: tarball is not a file: ${tarball}`);
      process.exit(1);
    }
  } catch (err) {
    console.error(`cleanroom-check: error: cannot stat tarball ${tarball}: ${err.message}`);
    process.exit(1);
  }
}
// --bundle is a tarball-mode input only: in --registry mode the launcher must
// provision from the public Release, so an override there would fake the gate.
if (bundle && registry) {
  console.error('cleanroom-check: error: --bundle applies to --tarball mode only; --registry must download from the public Release.');
  process.exit(1);
}
if (bundle) {
  try {
    if (!fs.statSync(path.resolve(bundle)).isFile()) {
      console.error(`cleanroom-check: error: bundle is not a file: ${bundle}`);
      process.exit(1);
    }
  } catch (err) {
    console.error(`cleanroom-check: error: bundle not found: ${bundle}`);
    process.exit(1);
  }
}

const rows = [];
const pass = (check, detail) => rows.push([check, 'PASS', detail]);
const fail = (check, detail) => rows.push([check, 'FAIL', detail]);

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'cleanroom-'));
const home = path.join(tmp, 'home');
const localAppData = path.join(home, 'AppData', 'Local');
const roaming = path.join(home, 'AppData', 'Roaming');
const tempDir = path.join(tmp, 'temp');
const npmCache = path.join(tmp, 'npm-cache');
const npmPrefix = path.join(tmp, 'npm-prefix');
const appDir = path.join(tmp, 'app');
for (const d of [home, localAppData, roaming, tempDir, npmCache, npmPrefix, appDir]) {
  fs.mkdirSync(d, { recursive: true });
}
// Scrubbed child env: override ONLY the profile/cache vars below; PATH,
// SystemRoot, ComSpec, ProgramData and everything else pass through.
const scrubEnv = {
  ...process.env,
  USERPROFILE: home,
  HOME: home,
  LOCALAPPDATA: localAppData,
  APPDATA: roaming,
  TEMP: tempDir,
  TMP: tempDir,
  npm_config_cache: npmCache,
  npm_config_prefix: npmPrefix,
};
// A stale engine pin in the parent env would bypass the clean room: the
// launcher would reuse a warmed runtime and the gate would prove nothing.
delete scrubEnv.GLADIUS_ENGINE_CACHE;
// A local-bundle override is only ever supplied by this script, and only for
// the pre-publish tarball gate. Scrubbing it here means --registry can never
// inherit one and skip the real download.
delete scrubEnv.GLADIUS_ENGINE_BUNDLE;
// Tarball mode + --bundle: hand the child a copy inside <tmp> so the gate still
// touches nothing outside its scratch dir. The launcher verifies the copy's
// size and sha256 against the installed manifest, so a wrong zip is refused.
if (tarball && bundle) {
  const bundleDir = path.join(tmp, 'bundle');
  fs.mkdirSync(bundleDir, { recursive: true });
  const stagedBundle = path.join(bundleDir, path.basename(path.resolve(bundle)));
  fs.copyFileSync(path.resolve(bundle), stagedBundle);
  scrubEnv.GLADIUS_ENGINE_BUNDLE = stagedBundle;
}

const q = (s) => `"${String(s).replace(/"/g, '""')}"`;
function run(cmd, { timeout = 120 * 1000, stdin = 'pipe' } = {}) {
  const r = spawnSync(cmd, {
    shell: true,
    env: scrubEnv,
    timeout,
    encoding: 'utf8',
    stdio: [stdin, 'pipe', 'pipe'],
    windowsHide: true,
  });
  return {
    status: r.status,
    signal: r.signal,
    stdout: r.stdout ?? '',
    stderr: r.stderr ?? '',
    timedOut: r.signal === 'SIGTERM' && r.error?.code === 'ETIMEDOUT',
  };
}
function dirSize(p) {
  let total = 0;
  for (const e of fs.readdirSync(p, { withFileTypes: true })) {
    const full = path.join(p, e.name);
    if (e.isDirectory()) total += dirSize(full);
    else if (e.isFile()) total += fs.statSync(full).size;
  }
  return total;
}
function countEntries(p) {
  let n = 0;
  for (const e of fs.readdirSync(p, { withFileTypes: true })) {
    n++;
    if (e.isDirectory()) n += countEntries(path.join(p, e.name));
  }
  return n;
}
function listNames(p) {
  try {
    return fs.readdirSync(p).sort();
  } catch {
    return [];
  }
}
function printTableAndExit(code) {
  const w = (s, n) => String(s).padEnd(n);
  console.log(`${w('CHECK', 22)}${w('RESULT', 8)}DETAIL`);
  for (const [c, r, d] of rows) console.log(`${w(c, 22)}${w(r, 8)}${d}`);
  if (!keep) {
    try {
      fs.rmSync(tmp, { recursive: true, force: true });
    } catch { /* ignore */ }
  } else {
    console.log(`cleanroom-check: scratch kept at ${tmp}`);
  }
  process.exit(code);
}

// The REAL cache must never be touched: the scrubbed LOCALAPPDATA keeps
// every provision under <tmp>. Snapshot names only, before and after.
const realEngineDir = path.join(process.env.LOCALAPPDATA ?? '', 'gladius', 'engine');
const realBefore = listNames(realEngineDir);

let expectedVersion;
let describeRun;
try {
  if (tarball) {
    const abs = path.resolve(tarball);
    const inst = run(`npm install --prefix ${q(appDir)} ${q(abs)}`, { timeout: BOOT_TIMEOUT_MS });
    if (inst.status !== 0) {
      console.error(`cleanroom-check: error: npm install of tarball failed (exit ${inst.status}):`);
      console.error((inst.stderr || inst.stdout).trim().split('\n').slice(-8).join('\n'));
      fail('install-tarball', `npm install --prefix failed for ${path.basename(abs)}`);
      printTableAndExit(1);
    }
    const nm = path.join(appDir, 'node_modules');
    const pkgDir = fs.readdirSync(nm, { withFileTypes: true })
      .filter((e) => e.isDirectory() && fs.existsSync(path.join(nm, e.name, 'package.json')))
      .map((e) => path.join(nm, e.name))[0];
    if (!pkgDir) {
      console.error('cleanroom-check: error: install succeeded but no package found under node_modules.');
      fail('install-tarball', 'no package dir under node_modules after install');
      printTableAndExit(1);
    }
    const pkg = JSON.parse(fs.readFileSync(path.join(pkgDir, 'package.json'), 'utf8'));
    expectedVersion = pkg.version;
    const binName = pkg.bin ? Object.keys(pkg.bin)[0] : path.basename(pkg.name, '.js');
    const binCmd = path.join(appDir, 'node_modules', '.bin', `${binName}.cmd`);
    if (!fs.existsSync(binCmd)) {
      console.error(`cleanroom-check: error: installed bin shim missing: ${binCmd}`);
      fail('install-tarball', `missing .bin shim ${binName}.cmd`);
      printTableAndExit(1);
    }
    pass('install-tarball', `${pkg.name}@${pkg.version} installed from ${path.basename(abs)}`);
    describeRun = (args) => `${q(binCmd)}${args ? ` ${args}` : ''}`;
  } else {
    let pkgName = 'gladius-mcp';
    try {
      pkgName = JSON.parse(fs.readFileSync(path.join(ROOT, 'npm', 'package.json'), 'utf8')).name || pkgName;
    } catch { /* fall back to gladius-mcp */ }
    expectedVersion = version;
    const spec = `${pkgName}@${version}`;
    // Fail fast with a clear message when the version does not exist.
    const view = run(`npm view ${q(spec)} version`, { timeout: 120 * 1000 });
    if (view.status !== 0 || view.stdout.trim() === '') {
      console.error(`cleanroom-check: error: ${spec} is not on the registry: no such version (npm view failed).`);
      console.error((view.stderr || view.stdout).trim().split('\n').slice(-5).join('\n'));
      fail('registry-version', `${spec} not found on registry`);
      printTableAndExit(1);
    }
    pass('registry-version', `${spec} resolves on registry as ${view.stdout.trim()}`);
    describeRun = (args) => `npx -y ${q(spec)}${args ? ` ${args}` : ''}`;
  }
} catch (err) {
  console.error(`cleanroom-check: error: install failed: ${err.message}`);
  fail('install', err.message);
  printTableAndExit(1);
}

// 1. --version output contains the expected version.
{
  const r = run(describeRun('--version'));
  if (r.status === 0 && r.stdout.includes(expectedVersion)) {
    pass('version-out', `bin --version prints ${r.stdout.trim().split('\n')[0]}`);
  } else {
    fail('version-out', `exit ${r.status}, stdout '${r.stdout.trim().slice(0, 120)}' lacks ${expectedVersion}`);
  }
}

// 2. --doctor exits 0 (and reports JSON we reuse below).
let doctor = null;
{
  const r = run(describeRun('--doctor'));
  if (r.status !== 0) {
    fail('doctor-exit', `--doctor exit ${r.status}: ${(r.stderr || r.stdout).trim().slice(0, 200)}`);
  } else {
    try {
      doctor = JSON.parse(r.stdout);
      pass('doctor-exit', '--doctor exit 0, manifest JSON parsed');
    } catch {
      fail('doctor-exit', '--doctor exit 0 but stdout is not JSON');
    }
  }
}

// 3. Real boot: spawn the bin with stdin closed, wait up to 300 s.
let bootErr = '';
{
  const r = run(describeRun(''), { timeout: BOOT_TIMEOUT_MS, stdin: 'ignore' });
  bootErr = (r.stderr || '').trim();
  const bad = /error:|sha256 mismatch/i.test(r.stderr || '');
  if (r.timedOut) {
    fail('boot-clean', 'boot timed out after 300 s with stdin closed');
  } else if (r.status !== 0) {
    fail('boot-clean', `boot exit ${r.status}${r.signal ? ` signal ${r.signal}` : ''}: ${bootErr.split('\n').slice(-3).join(' | ').slice(0, 240)}`);
  } else if (bad) {
    fail('boot-clean', `boot exit 0 but stderr shows error/sha256 mismatch: ${bootErr.split('\n').slice(-3).join(' | ').slice(0, 240)}`);
  } else {
    pass('boot-clean', 'boot exit 0, stdin closed, stderr has no error:/sha256 mismatch');
  }
}

// 4. Provisioned engine under <tmp>: exe present, size == manifest server.size, .ready present.
let engineDir = doctor?.runtimeCacheDir ?? '';
let serverSize = doctor?.runtime?.server?.size;
let serverName = doctor?.runtime?.server?.name ?? 'gladius.exe';
if (!engineDir || !engineDir.startsWith(tmp)) {
  fail('engine-cache', `runtimeCacheDir not under scrub tmp: '${engineDir || '<missing doctor>'}'`);
} else {
  const exe = path.join(engineDir, serverName);
  let size = -1;
  try {
    size = fs.statSync(exe).size;
  } catch { /* missing */ }
  const ready = fs.existsSync(path.join(engineDir, '.ready'));
  if (size < 0) fail('engine-cache', `missing ${exe}`);
  else if (size !== serverSize) fail('engine-cache', `${exe} size ${size} != manifest server.size ${serverSize}`);
  else if (!ready) fail('engine-cache', `${exe} size OK but .ready missing`);
  else pass('engine-cache', `${exe} (${size} bytes == manifest) + .ready present`);
}

// Chromium is provisioned alongside on first run (~123 MB): report only,
// never assert (beyond the boot-clean stderr check above).
{
  console.log(`cleanroom-check: engine source: ${scrubEnv.GLADIUS_ENGINE_BUNDLE ? `local bundle ${scrubEnv.GLADIUS_ENGINE_BUNDLE}` : 'Release download'}`);
  if (bundle) {
    console.log('cleanroom-check: the runtime bundle is local, but the boot still provisions Chromium from Google\'s Chrome-for-Testing endpoint (network-dependent)');
  }
  const cdir = doctor?.browserCacheDir && String(doctor.browserCacheDir).startsWith(tmp)
    ? doctor.browserCacheDir
    : path.join(localAppData, 'gladius', 'chromium');
  if (fs.existsSync(cdir) && countEntries(cdir) > 0) {
    console.log(`cleanroom-check: chromium provisioned under scratch: ${cdir} (${dirSize(cdir)} bytes)`);
  } else {
    console.log('cleanroom-check: chromium not provisioned under scratch');
  }
}

// 5. Real %LOCALAPPDATA%\gladius\engine listing identical before/after (names only).
{
  const after = listNames(realEngineDir);
  const same = JSON.stringify(after) === JSON.stringify(realBefore);
  if (same) pass('real-cache-untouched', `real ${realEngineDir || '<unset>'} unchanged (${after.length} entries)`);
  else fail('real-cache-untouched', `real cache changed: before [${realBefore.join(',')}] after [${after.join(',')}]`);
}

// 6. The npm cache was written under <tmp>.
{
  const n = fs.existsSync(npmCache) ? countEntries(npmCache) : 0;
  if (n > 0) pass('npm-cache-scrubbed', `${n} entries under ${npmCache}`);
  else fail('npm-cache-scrubbed', `npm cache empty at ${npmCache} (leak to user profile?)`);
}

printTableAndExit(rows.some((r) => r[1] === 'FAIL') ? 1 : 0);
