#!/usr/bin/env node
'use strict';
//
// gladius-mcp launcher.
//
// Published package model (see docs/PUBLISHING.md): this tarball holds ONLY
// this launcher plus the pinned browser/runtime manifest (engine-manifest.json).
// It holds NO browser bytes, NO server bytes, NO `os` field, NO
// `scripts`/postinstall field. npm 12 blocks dependency install scripts by
// default (measured `allow-scripts=false`), and npx has no project
// package.json for `allowScripts` to consult, so a postinstall download
// would silently never run. Everything is therefore acquired on first RUN
// by the code below.
//
// Two provisions, two caches, both verified:
//   - The server runtime (manifest runtime.archive) downloads from the
//     tagged GitHub Release, verifies zip sha256+size BEFORE extraction,
//     extracts atomically (staging dir, then rename) into
//     %LOCALAPPDATA%\gladius\engine\<serverVersion>-<bundleSha8>\, verifies
//     the server binary against the manifest, writes a .ready marker, and
//     is reused on later runs (re-verified each time).
//   - The browser (manifest browser.asset) downloads from Google's
//     Chrome-for-Testing endpoint, verifies zip sha256+size BEFORE
//     extraction, extracts atomically into the owned browser cache
//     %LOCALAPPDATA%\gladius\chromium\<version>-<zipSha8>\ (the same layout
//     scripts/fetch-chromium.ps1 provisions), verifies every extracted file
//     against the manifest (exact set, no extras), writes a .ready marker,
//     and is reused on later runs (re-verified each time: marker + exe
//     sha256 + a full name/size walk). Any mismatch refuses and writes
//     nothing. The browser is NEVER redistributed: it is fetched here, at
//     provision time, from Google.
//   - The server (gladius.exe, no args = MCP stdio serve) spawns with
//     GLADIUS_CHROMIUM pointed at the provisioned browser (the server reads
//     that variable), unless the user already set it: an explicit
//     GLADIUS_CHROMIUM wins, and then no browser is provisioned at all.
//     The child env always carries GLADIUS_EXPECTED_VERSION set to this
//     launcher's own package.json version, so the engine's fail-closed gate
//     (src/main.rs) refuses instead of serving a mixed-version pair.
//     stdio is inherited. ALL launcher chatter goes to stderr; stdout belongs
//     to the MCP channel. No stdout writes exist on the run path.
//   - `--doctor` / `--print-manifest` is the no-network diagnostic: it
//     prints the manifest plus the resolved cache paths and exits 0. It
//     returns BEFORE any fetch call (all fetch use lives in
//     downloadToMemory, reachable only from the ensure* functions, which the
//     diagnostic path never calls).
//
// Builtins only: node:fs, node:path, node:os, node:crypto,
// node:child_process, node:zlib, and the global fetch (Node >= 22.14).
// No dependencies, so the published tarball stays tiny and auditable.
//

const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const zlib = require('node:zlib');
const { spawn } = require('node:child_process');

const PKG_DIR = path.dirname(path.dirname(__filename));
const MANIFEST_PATH = path.join(PKG_DIR, 'engine-manifest.json');
const PKG_JSON_PATH = path.join(PKG_DIR, 'package.json');
const PLATFORM_OVERRIDE_ENV = 'GLADIUS_MCP_ALLOW_NON_WINDOWS';
const RUNTIME_CACHE_OVERRIDE_ENV = 'GLADIUS_ENGINE_CACHE';
// Local-bundle override, diagnostics only. A normal run leaves it unset and
// downloads runtime.archive from the Release. The pre-publish cleanroom names
// the archive it just built, because the Release that would serve it does not
// exist until AFTER the build job passes. Every size/sha256 assertion still
// runs on the local bytes: an override can never smuggle in a bad artifact.
const RUNTIME_BUNDLE_OVERRIDE_ENV = 'GLADIUS_ENGINE_BUNDLE';
const BROWSER_CACHE_OVERRIDE_ENV = 'GLADIUS_CHROMIUM_CACHE';
const BROWSER_PATH_OVERRIDE_ENV = 'GLADIUS_CHROMIUM';
const DOWNLOAD_TIMEOUT_MS = 15 * 60 * 1000;
// Staging/extraction guards: the real runtime bundle is small and the real
// browser zip holds 288 entries / ~290 MB extracted. These caps reject zip
// bombs, never the real artifacts.
const MAX_ENTRIES = 4096;
const MAX_TOTAL_UNCOMPRESSED = 2 * 1024 * 1024 * 1024;
const MAX_ENTRY_SIZE = 1 * 1024 * 1024 * 1024;

function fail(message) {
  process.stderr.write(`gladius-mcp: error: ${message}\n`);
  process.exit(1);
}

function log(message) {
  process.stderr.write(`gladius-mcp: ${message}\n`);
}

// The launcher's own version (the referent the engine gate checks against).
// Read fresh from package.json on every run: never a cached constant, never
// a fixed tag.
function loadPackageVersion() {
  let pkg;
  try {
    pkg = JSON.parse(fs.readFileSync(PKG_JSON_PATH, 'utf8'));
  } catch (err) {
    fail(`cannot read package version at ${PKG_JSON_PATH}: ${err.message}`);
  }
  if (!pkg || typeof pkg.version !== 'string' || pkg.version.length === 0) {
    fail(`package.json has no version; refusing to run (reinstall with \`npx -y gladius-mcp@latest\`).`);
  }
  return pkg.version;
}

function loadManifest() {
  let raw;
  try {
    raw = fs.readFileSync(MANIFEST_PATH, 'utf8');
  } catch (err) {
    fail(`cannot read engine manifest at ${MANIFEST_PATH}: ${err.message}`);
  }
  try {
    return JSON.parse(raw);
  } catch (err) {
    fail(`engine manifest is not valid JSON: ${err.message}`);
  }
}

// A sha256 slot the release workflow has not filled yet. The committed
// manifest carries the sentinel for every hash computed at release time;
// the workflow replaces each with the real 64-hex digest. Anything else
// shaped (64 hex chars) is release-provisioned.
function isPlaceholderSha(sha) {
  return typeof sha !== 'string' || !/^[0-9a-fA-F]{64}$/.test(sha);
}

function shortHash(sha) {
  return isPlaceholderSha(sha) ? 'unpublished' : sha.toLowerCase().slice(0, 8);
}

// Resolved runtime cache directory. Never throws: the diagnostic path must
// print SOMETHING and exit 0 even where provisioning could never run.
function resolveRuntimeDir(manifest) {
  const notes = [];
  let root = process.env[RUNTIME_CACHE_OVERRIDE_ENV];
  let overridden = Boolean(root);
  if (!root) {
    root = process.env.LOCALAPPDATA;
    if (!root) notes.push('LOCALAPPDATA is unset; set it or ' + RUNTIME_CACHE_OVERRIDE_ENV);
  }
  const leaf = `${manifest.serverVersion}-${shortHash(manifest.runtime.archive.sha256)}`;
  if (!root) return { dir: path.join('<unresolvable>', 'gladius', 'engine', leaf), overridden, notes };
  return { dir: path.join(root, 'gladius', 'engine', leaf), overridden, notes };
}

// Resolved browser cache directory: the same owned layout
// scripts/fetch-chromium.ps1 provisions (<version>-<zipSha8>). Never throws,
// for the same diagnostic reason as above.
function resolveBrowserDir(manifest) {
  const notes = [];
  let root = process.env[BROWSER_CACHE_OVERRIDE_ENV];
  let overridden = Boolean(root);
  if (!root) {
    root = process.env.LOCALAPPDATA;
    if (!root) notes.push('LOCALAPPDATA is unset; set it or ' + BROWSER_CACHE_OVERRIDE_ENV);
  }
  const leaf = `${manifest.browser.version}-${shortHash(manifest.browser.asset.sha256)}`;
  if (!root) return { dir: path.join('<unresolvable>', 'gladius', 'chromium', leaf), overridden, notes };
  return { dir: path.join(root, 'gladius', 'chromium', leaf), overridden, notes };
}

function printDoctor(manifest) {
  const runtime = resolveRuntimeDir(manifest);
  const browser = resolveBrowserDir(manifest);
  const report = {
    package: manifest.package,
    serverVersion: manifest.serverVersion,
    platform: process.platform,
    node: process.version,
    runtimeCacheDir: runtime.dir,
    runtimeCacheOverridden: runtime.overridden,
    runtimeCacheNotes: runtime.notes,
    runtimeBundleOverride: {
      env: RUNTIME_BUNDLE_OVERRIDE_ENV,
      set: Boolean(process.env[RUNTIME_BUNDLE_OVERRIDE_ENV]),
      path: process.env[RUNTIME_BUNDLE_OVERRIDE_ENV] || null,
      note: 'diagnostics only: the pre-publish cleanroom points this at the archive it just built; a normal run leaves it unset and downloads runtime.archive, whose size and sha256 are checked either way',
    },
    browserCacheDir: browser.dir,
    browserCacheOverridden: browser.overridden,
    browserCacheNotes: browser.notes,
    browserPathOverride: process.env[BROWSER_PATH_OVERRIDE_ENV] || null,
    browser: {
      project: manifest.browser.project,
      version: manifest.browser.version,
      platform: manifest.browser.platform,
      asset: manifest.browser.asset,
      root: manifest.browser.root,
      fileCount: manifest.browser.fileCount,
      totalSize: manifest.browser.totalSize,
      filesPinned: Array.isArray(manifest.browser.files) ? manifest.browser.files.length : 0,
    },
    runtime: manifest.runtime,
    network: 'none: this diagnostic performs no fetch and exits before any download code',
  };
  process.stdout.write(JSON.stringify(report, null, 2) + '\n');
  process.exit(0);
}

function usage(exitCode, stream) {
  const text = [
    'usage: gladius-mcp [--doctor | --print-manifest | --version | --help]',
    '',
    '  (no args)            provision the pinned runtime + browser on first run, then serve MCP over stdio',
    '  --doctor,            print the pinned manifest and the resolved cache',
    '  --print-manifest     paths, perform no network access, exit 0',
    '  --version            print the launcher version, perform no network access, exit 0',
    '  --help, -h           print this usage',
    '',
    'runtime cache: ' + path.join('%LOCALAPPDATA%', 'gladius', 'engine', '<serverVersion>-<bundleSha8>') + ` (override: ${RUNTIME_CACHE_OVERRIDE_ENV})`,
    `local bundle: unset by default; ${RUNTIME_BUNDLE_OVERRIDE_ENV} names a zip file to verify and extract`,
    '              instead of downloading (pre-publish diagnostics only; size and sha256 are still checked)',
    'browser cache: ' + path.join('%LOCALAPPDATA%', 'gladius', 'chromium', '<version>-<zipSha8>') + ` (override: ${BROWSER_CACHE_OVERRIDE_ENV})`,
    `explicit browser binary (skips browser provisioning): ${BROWSER_PATH_OVERRIDE_ENV}`,
    `platform: windows-x86_64 only (override: ${PLATFORM_OVERRIDE_ENV}=1, diagnostics only)`,
  ].join('\n') + '\n';
  stream.write(text);
  process.exit(exitCode);
}

// --- minimal zip reader (stored + deflate, builtins only) -----------------
// Both zips (the Compress-Archive runtime bundle, the CfT browser archive)
// are ordinary deflate zips. Node has no builtin unzip, and this package is
// dependency-free by design, so the container parse is hand-rolled here
// (~120 lines) over zlib.inflateRawSync. Anything outside the artifacts'
// actual shape (encryption, data descriptors, zip64, unknown methods) is an
// explicit error, never a silent misparse.

const CRC_TABLE = (() => {
  const t = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c;
  }
  return t;
})();

function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function findEocd(buf) {
  // EOCD is >= 22 bytes at the very end, with up to 64 KiB of comment.
  const lo = Math.max(0, buf.length - (65557 + 22));
  for (let i = buf.length - 22; i >= lo; i--) {
    if (buf.readUInt32LE(i) === 0x06054b50) return i;
  }
  return -1;
}

function extractZip(buf, destDir) {
  const eocd = findEocd(buf);
  if (eocd < 0) throw new Error('not a zip archive: end-of-central-directory not found');
  const entries = buf.readUInt16LE(eocd + 10);
  const cdSize = buf.readUInt32LE(eocd + 12);
  const cdOff = buf.readUInt32LE(eocd + 16);
  if (entries === 0xffff || cdSize === 0xffffffff || cdOff === 0xffffffff) {
    throw new Error('zip64 archives are not supported');
  }
  if (entries > MAX_ENTRIES) throw new Error(`too many entries: ${entries}`);
  if (cdOff + cdSize > buf.length) throw new Error('central directory runs past end of buffer');

  let total = 0;
  const extracted = [];
  let p = cdOff;
  for (let n = 0; n < entries; n++) {
    if (buf.readUInt32LE(p) !== 0x02014b50) throw new Error(`bad central-directory entry ${n}`);
    const flags = buf.readUInt16LE(p + 8);
    const method = buf.readUInt16LE(p + 10);
    const crc = buf.readUInt32LE(p + 16);
    const compSize = buf.readUInt32LE(p + 20);
    const uncompSize = buf.readUInt32LE(p + 24);
    const nameLen = buf.readUInt16LE(p + 28);
    const extraLen = buf.readUInt16LE(p + 30);
    const commentLen = buf.readUInt16LE(p + 32);
    const localOff = buf.readUInt32LE(p + 42);
    const rawName = buf.toString('utf8', p + 46, p + 46 + nameLen);
    p += 46 + nameLen + extraLen + commentLen;

    if (flags & 0x0001) throw new Error(`encrypted entry unsupported: ${rawName}`);
    if (flags & 0x0008) throw new Error(`data-descriptor entry unsupported: ${rawName}`);
    if (method !== 0 && method !== 8) throw new Error(`compression method ${method} unsupported: ${rawName}`);
    if (compSize === 0xffffffff || uncompSize === 0xffffffff) throw new Error(`zip64 entry unsupported: ${rawName}`);
    if (uncompSize > MAX_ENTRY_SIZE) throw new Error(`entry too large: ${rawName}`);
    total += uncompSize;
    if (total > MAX_TOTAL_UNCOMPRESSED) throw new Error('archive uncompressed total exceeds cap');

    // Traversal guard: zip names normally use forward slashes, but
    // PowerShell's Compress-Archive can write nested entry names
    // with backslashes (e.g. `THIRD-PARTY-LICENSES\Apache-2.0.txt`). Split
    // on `/` alone would treat those as one segment, so normalize every
    // backslash to `/` FIRST, then run the unchanged checks below on the
    // normalized name: absolute paths, drive letters, and `..` segments
    // stay rejected, and a `..\..\evil.txt` entry still trips the `..`
    // guard. The resolved target must still sit under destDir.
    // (Spelled via char code so the source never holds a literal
    // double-backslash.)
    const BACKSLASH = String.fromCharCode(92);
    const name = rawName.split(BACKSLASH).join('/');
    if (name.startsWith('/') || /^[A-Za-z]:/.test(name)) {
      throw new Error(`unsafe entry name: ${rawName}`);
    }
    const parts = name.split('/').filter((s) => s !== '' && s !== '.');
    if (parts.length === 0 || parts.includes('..')) throw new Error(`unsafe entry name: ${rawName}`);
    const target = path.join(destDir, ...parts);
    const rel = path.relative(destDir, target);
    if (rel.startsWith('..') || path.isAbsolute(rel)) throw new Error(`entry escapes destination: ${rawName}`);
    const isDir = name.endsWith('/');
    if (isDir) {
      fs.mkdirSync(target, { recursive: true });
      continue;
    }

    if (buf.readUInt32LE(localOff) !== 0x04034b50) throw new Error(`bad local header: ${rawName}`);
    const lhNameLen = buf.readUInt16LE(localOff + 26);
    const lhExtraLen = buf.readUInt16LE(localOff + 28);
    const dataOff = localOff + 30 + lhNameLen + lhExtraLen;
    const comp = buf.subarray(dataOff, dataOff + compSize);
    if (comp.length !== compSize) throw new Error(`truncated entry data: ${rawName}`);
    const data = method === 0 ? Buffer.from(comp) : zlib.inflateRawSync(comp);
    if (data.length !== uncompSize) throw new Error(`size mismatch after inflate: ${rawName}`);
    if (crc32(data) !== crc) throw new Error(`crc mismatch: ${rawName}`);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, data);
    extracted.push(parts.join('/'));
  }
  return extracted;
}

// --- provisioning ---------------------------------------------------------

function sha256File(filePath) {
  const h = crypto.createHash('sha256');
  h.update(fs.readFileSync(filePath));
  return h.digest('hex');
}

function checkFile(filePath, want) {
  // want: {name, sha256, size}. Returns null when good, else a reason.
  let st;
  try {
    st = fs.statSync(filePath);
  } catch {
    return 'missing';
  }
  if (!st.isFile()) return 'not a file';
  if (st.size !== want.size) return `size ${st.size} != manifest ${want.size}`;
  const got = sha256File(filePath);
  if (got !== want.sha256.toLowerCase()) return 'sha256 mismatch';
  return null;
}

// Walk a cache dir into zip-relative names (forward slashes) with sizes.
// The browser cache keeps the zip's own tree (root dir included), so the
// walked names compare directly against the manifest file list.
function walkFiles(dir) {
  const out = [];
  const visit = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const full = path.join(d, e.name);
      if (e.isDirectory()) visit(full);
      else if (e.isFile()) {
        const rel = path.relative(dir, full).split(path.sep).join('/');
        out.push({ rel, full, size: fs.statSync(full).size });
      }
    }
  };
  visit(dir);
  return out;
}

// --- concurrent provisioning ---------------------------------------------
//
// Two MCP clients (two project directories in the same editor, two agents in
// the same session) start this launcher at the same instant and resolve the
// SAME cache leaf. Both then staged and renamed onto the same final path, and
// each deleted that path on entry and on failure, so on Windows one process
// got `EPERM ... rename` from the move and its cleanup deleted the tree the
// other had just published; the winner then reported "server binary missing
// after provisioning" and the whole tool namespace vanished (observed
// 2026-10-23 against v0.4.5, engine 0.4.5-997b287f, two pids seconds apart).
//
// The shape below makes a shared cache safe without weakening any check:
//   - ONE writer at a time, via an atomically-created `<cache>.lock` directory
//     (`mkdirSync` is the only exclusive-create primitive Node has on Windows).
//     Waiters poll; a lock whose owner pid is gone, or that outlives the hard
//     ceiling without a heartbeat, is broken by renaming it aside so exactly
//     one waiter can win the break.
//   - A bounded retry on the transient Windows errors only (EPERM/EACCES/
//     EBUSY: an indexer, an AV scanner, or a just-closed handle), never on a
//     real failure. Every retry re-runs the same size/sha256 assertions.
//   - "The final directory now verifies" is SUCCESS, not failure: that is the
//     other process having just produced the exact bytes. A rename that loses
//     the race therefore reuses the winner's cache instead of erroring.
//   - Staging leftovers from a crashed run are reaped so they cannot pile up.
//   - The final path is never deleted outside the lock, and never deleted in
//     a failure path at all: a loser can no longer destroy a winner's cache.
const LOCK_WAIT_MS = 25 * 60 * 1000;
// A live owner heartbeats the lock directory at every phase, so this ceiling
// only fires for a lock whose process is wedged, not for a slow download.
const LOCK_STALE_MS = 30 * 60 * 1000;
const STAGING_STALE_MS = 60 * 60 * 1000;
const TRANSIENT_FS_CODES = new Set(['EPERM', 'EACCES', 'EBUSY']);
const RENAME_BACKOFF_MS = [0, 50, 150, 400, 1000, 2000, 4000];
const LOCK_POLL_MS = [25, 50, 100, 200, 400, 800];

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function isTransientFsError(err) {
  return Boolean(err) && TRANSIENT_FS_CODES.has(err.code);
}

function pidAlive(pid) {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    // EPERM means the pid exists but belongs to someone we may not signal.
    return err.code === 'EPERM';
  }
}

function readLockOwner(lockDir) {
  try {
    return JSON.parse(fs.readFileSync(path.join(lockDir, 'owner.json'), 'utf8'));
  } catch {
    return null;
  }
}

function heartbeatLock(lockDir) {
  const now = new Date();
  try {
    fs.utimesSync(lockDir, now, now);
  } catch {
    // The lock may have been broken and taken over; the release owner check
    // below is what keeps that safe.
  }
}

function releaseProvisionLock(lockDir) {
  if (!lockDir) return;
  const owner = readLockOwner(lockDir);
  if (owner && Number.isInteger(owner.pid) && owner.pid !== process.pid) return; // someone else's
  try {
    fs.rmSync(lockDir, { recursive: true, force: true });
  } catch {
    // Best effort: a leftover lock is broken by the stale rule on the next run.
  }
}

// Returns true when this caller moved a dead/slow lock out of the way. The
// rename is the race-breaker: only one waiter can perform it.
function breakStaleLock(lockDir, label) {
  let st;
  try {
    st = fs.statSync(lockDir);
  } catch {
    return false; // released between our mkdir attempt and now
  }
  const owner = readLockOwner(lockDir);
  const age = Date.now() - st.mtimeMs;
  const ownerDead = owner && Number.isInteger(owner.pid) ? !pidAlive(owner.pid) : false;
  // A lock is a directory. Anything else sitting on that path is corruption,
  // not a live holder, so it is cleared without waiting for the ceiling.
  if (st.isDirectory() && !ownerDead && age < LOCK_STALE_MS) return false;
  const park = `${lockDir}.broken-${process.pid}-${Date.now()}`;
  try {
    fs.renameSync(lockDir, park);
  } catch {
    return false; // another waiter broke it first
  }
  const who = owner && owner.pid ? `pid ${owner.pid}${ownerDead ? ', gone' : ', still running'}` : 'no owner file';
  log(`clearing a stale ${label} provision lock at ${lockDir} (${who}, ${Math.round(age / 1000)}s old)`);
  fs.rmSync(park, { recursive: true, force: true });
  return true;
}

async function acquireProvisionLock(cacheDir, label) {
  const lockDir = `${cacheDir}.lock`;
  fs.mkdirSync(path.dirname(cacheDir), { recursive: true });
  const deadline = Date.now() + LOCK_WAIT_MS;
  let announced = false;
  let poll = 0;
  for (;;) {
    try {
      fs.mkdirSync(lockDir); // exclusive create: EEXIST means held
      try {
        fs.writeFileSync(
          path.join(lockDir, 'owner.json'),
          JSON.stringify({ pid: process.pid, host: os.hostname(), since: new Date().toISOString(), label }, null, 2)
        );
      } catch {
        // The directory itself is the lock; the owner file only names it.
      }
      return lockDir;
    } catch (err) {
      if (err.code !== 'EEXIST') {
        throw new Error(`cannot create the ${label} provision lock ${lockDir}: ${err.message}`);
      }
    }
    if (breakStaleLock(lockDir, label)) continue;
    if (!announced) {
      const owner = readLockOwner(lockDir);
      log(
        `another gladius-mcp process is provisioning the ${label} cache` +
        (owner && owner.pid ? ` (pid ${owner.pid} since ${owner.since})` : '') +
        `; waiting for it to finish: ${cacheDir}`
      );
      announced = true;
    }
    if (Date.now() > deadline) {
      const owner = readLockOwner(lockDir);
      throw new Error(
        `timed out after ${Math.round(LOCK_WAIT_MS / 60000)} min waiting for the ${label} provision lock ${lockDir}` +
        (owner && owner.pid
          ? ` held by pid ${owner.pid} on ${owner.host || 'this host'} since ${owner.since}`
          : ' (holder unknown: the lock has no readable owner.json)')
      );
    }
    await sleep(LOCK_POLL_MS[Math.min(poll, LOCK_POLL_MS.length - 1)] + Math.floor(Math.random() * 25));
    poll++;
  }
}

// Runs fn under the single-writer lock and always releases it, including when
// fn throws. fn must throw on failure and must NOT call fail(): process.exit()
// would skip the release.
async function withProvisionLock(cacheDir, label, fn) {
  const lockDir = await acquireProvisionLock(cacheDir, label);
  try {
    return await fn(lockDir);
  } finally {
    releaseProvisionLock(lockDir);
  }
}

// A directory rename that loses to a concurrent mover, or to a handle an
// indexer/AV has open, fails with one of the transient codes. Retry those
// with short backoff; any other error is real and propagates at once.
async function renameWithRetry(from, to, label) {
  let last = null;
  for (let i = 0; i < RENAME_BACKOFF_MS.length; i++) {
    if (RENAME_BACKOFF_MS[i] > 0) await sleep(RENAME_BACKOFF_MS[i]);
    try {
      fs.renameSync(from, to);
      return { ok: true, error: null };
    } catch (err) {
      last = err;
      if (!isTransientFsError(err)) throw err;
      const next = RENAME_BACKOFF_MS[i + 1];
      log(`publishing the ${label} cache was blocked transiently (${err.code}); ` +
        (next === undefined ? 'no retries left' : `retrying in ${next}ms`));
    }
  }
  return { ok: false, error: last };
}

// Remove a cache tree that cannot be used. Moved out of the final path first:
// a half-deleted tree must never sit where the next run reads it as a cache.
// Only called while holding the lock.
async function removeCacheTree(target) {
  const park = `${target}.stale-${process.pid}-${Date.now()}`;
  const moved = await (async () => {
    for (const delay of [0, 50, 250]) {
      if (delay > 0) await sleep(delay);
      try {
        fs.renameSync(target, park);
        return true;
      } catch (err) {
        if (!isTransientFsError(err)) return false;
      }
    }
    return false;
  })();
  if (!moved) {
    fs.rmSync(target, { recursive: true, force: true });
    return;
  }
  fs.rmSync(park, { recursive: true, force: true });
}

// Reap leftovers of an earlier crashed provision next to this cache: staging
// and parked trees whose pid is gone or that are older than any live run can
// be. Best effort: a live or unreadable sibling is left alone.
function reapStaleSiblings(cacheDir) {
  const parent = path.dirname(cacheDir);
  const base = path.basename(cacheDir) + '.';
  const reusable = ['staging-', 'stale-', 'lock.broken-'];
  let entries;
  try {
    entries = fs.readdirSync(parent);
  } catch {
    return;
  }
  for (const name of entries) {
    if (!name.startsWith(base)) continue;
    const suffix = name.slice(base.length);
    const kind = reusable.find((prefix) => suffix.startsWith(prefix));
    if (!kind) continue;
    const full = path.join(parent, name);
    let st;
    try {
      st = fs.statSync(full);
    } catch {
      continue;
    }
    const tail = suffix.slice(kind.length);
    const pid = Number.parseInt(kind === 'stale-' ? tail.split('-')[0] : tail, 10);
    const abandoned = !Number.isInteger(pid) || !pidAlive(pid) || Date.now() - st.mtimeMs > STAGING_STALE_MS;
    if (!abandoned) continue;
    try {
      fs.rmSync(full, { recursive: true, force: true });
      log(`removed an abandoned provisioning leftover: ${full}`);
    } catch {
      // In use after all; the next run tries again.
    }
  }
}

function describeCacheStatus(status) {
  if (status === 'ok') return 'present and verified';
  if (status === 'absent') return 'absent';
  return `present but did not verify (${status})`;
}

// The ONLY fetch call site in this package. The diagnostic path returns
// before the ensure* functions, so --doctor/--print-manifest provably reach
// no network: grep `fetch(` — it appears exactly once, here.
async function downloadToMemory(url) {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), DOWNLOAD_TIMEOUT_MS);
  let res;
  try {
    res = await fetch(url, { signal: ctrl.signal });
  } catch (err) {
    throw new Error(`download failed: ${err.message}`);
  } finally {
    clearTimeout(timer);
  }
  if (!res.ok) throw new Error(`download failed: HTTP ${res.status} ${res.statusText}`);
  return Buffer.from(await res.arrayBuffer());
}

// Cache state without trusting the marker: 'ok' means the server binary at
// the final path matches the manifest name/size/sha256 exactly.
function runtimeCacheStatus(cacheDir, rt) {
  let st;
  try {
    st = fs.statSync(cacheDir);
  } catch {
    return 'absent';
  }
  if (!st.isDirectory()) return 'invalid';
  const bad = checkFile(path.join(cacheDir, rt.server.name), rt.server);
  if (bad === 'missing') return 'incomplete';
  if (bad) return `invalid: ${bad}`;
  return 'ok';
}

// The marker records what this launcher verified for the tree it published. It
// is written INSIDE the staged tree, so the rename publishes a cache that is
// already marked and a reader can never observe an unmarked final directory.
function writeRuntimeMarker(dir, manifest, rt) {
  fs.writeFileSync(
    path.join(dir, '.ready'),
    JSON.stringify({ serverVersion: manifest.serverVersion, bundleSha256: rt.archive.sha256.toLowerCase() }, null, 2)
  );
}

async function ensureRuntime(manifest, cacheDir) {
  const rt = manifest.runtime;
  if (isPlaceholderSha(rt.archive.sha256) || !(rt.archive.size > 0)) {
    fail(
      'this package was not published by the release workflow: runtime hashes are placeholders. ' +
      'See docs/PUBLISHING.md: publish only the workflow-built npm-package artifact.'
    );
  }
  if (isPlaceholderSha(rt.server.sha256) || !(rt.server.size > 0)) {
    fail('engine manifest has no release-provisioned server hash; refusing to run.');
  }

  const markerPath = path.join(cacheDir, '.ready');
  const reusable = (announce) => {
    try {
      const marker = JSON.parse(fs.readFileSync(markerPath, 'utf8'));
      if (marker.serverVersion !== manifest.serverVersion) return false;
      if (String(marker.bundleSha256).toLowerCase() !== rt.archive.sha256.toLowerCase()) return false;
    } catch {
      return false;
    }
    const bad = checkFile(path.join(cacheDir, rt.server.name), rt.server);
    if (bad) {
      if (announce) log(`cached ${rt.server.name} invalid (${bad}); re-provisioning.`);
      return false;
    }
    return true;
  };
  if (reusable(true)) {
    log(`runtime cache ready (reused): ${cacheDir}`);
    return cacheDir;
  }

  try {
    await withProvisionLock(cacheDir, 'runtime', async (lockDir) => {
      // The process we may have just waited behind can have finished: re-check
      // under the lock before spending a download on the same bytes.
      if (reusable(false)) {
        log(`runtime cache ready (reused): ${cacheDir}`);
        return;
      }
      const status = runtimeCacheStatus(cacheDir, rt);
      if (status === 'ok') {
        // A previous run published these exact bytes but never got to write
        // the marker (killed between rename and marker). Adopt, don't refetch.
        log(`runtime cache bytes verify but are unmarked; adopting ${cacheDir}`);
        writeRuntimeMarker(cacheDir, manifest, rt);
        return;
      }
      if (status !== 'absent') {
        log(`runtime cache at ${cacheDir} is unusable (${status}); replacing it`);
        await removeCacheTree(cacheDir);
      }
      await provisionRuntime(manifest, cacheDir, rt, lockDir);
    });
  } catch (err) {
    const status = runtimeCacheStatus(cacheDir, rt);
    const detail = String(err.message).replace(/\.\s*$/, '');
    fail(
      `provisioning runtime cache ${cacheDir} failed: ${detail}. ` +
      `Cache dir: ${describeCacheStatus(status)}. ` +
      `Retry \`npx -y gladius-mcp@latest\`; if it keeps failing, delete ${cacheDir} ` +
      `and any sibling ${path.basename(cacheDir)}.lock / .staging-* directories, then retry.`
    );
  }
  return cacheDir;
}

async function provisionRuntime(manifest, cacheDir, rt, lockDir) {
  // Local bundle override: the pre-publish cleanroom builds the archive in
  // this same job, before any GitHub Release exists, so it hands the zip over
  // instead of fetching a URL that cannot exist yet. The size and sha256
  // checks below stay unconditional: a local zip that disagrees with the
  // manifest is refused exactly like a corrupt download would be.
  const localBundle = process.env[RUNTIME_BUNDLE_OVERRIDE_ENV];
  const origin = localBundle ? 'local bundle' : 'download';
  let data;
  if (localBundle) {
    log(`provisioning runtime cache: ${cacheDir}`);
    log(`reading ${localBundle} (${RUNTIME_BUNDLE_OVERRIDE_ENV} set; pre-publish diagnostics only)`);
    try {
      data = fs.readFileSync(localBundle);
    } catch (err) {
      throw new Error(`cannot read local bundle ${localBundle}: ${err.message}`);
    }
  } else {
    const url = `${rt.source}/${rt.archive.name}`;
    log(`provisioning runtime cache: ${cacheDir}`);
    log(`downloading ${url} (${rt.archive.size} bytes)`);
    try {
      data = await downloadToMemory(url);
    } catch (err) {
      throw new Error(err.message);
    }
  }
  heartbeatLock(lockDir);

  // Integrity: mandatory on every path, first attempt or retry. A retry can
  // never skip or weaken these two checks.
  if (data.length !== rt.archive.size) {
    throw new Error(`${origin} size ${data.length} != manifest ${rt.archive.size}; wrote nothing.`);
  }
  const digest = crypto.createHash('sha256').update(data).digest('hex');
  if (digest !== rt.archive.sha256.toLowerCase()) {
    throw new Error(`${origin} sha256 mismatch; wrote nothing.`);
  }

  reapStaleSiblings(cacheDir);

  // Extract into a staging dir, verify, then rename: the target dir is never
  // observed half-written.
  const staging = `${cacheDir}.staging-${process.pid}`;
  fs.rmSync(staging, { recursive: true, force: true });
  try {
    fs.mkdirSync(staging, { recursive: true });
    const names = extractZip(data, staging);
    log(`extracted ${names.length} entries`);
    const bad = checkFile(path.join(staging, rt.server.name), rt.server);
    if (bad) throw new Error(`extracted ${rt.server.name} invalid (${bad})`);
    writeRuntimeMarker(staging, manifest, rt);
    heartbeatLock(lockDir);
    const moved = await renameWithRetry(staging, cacheDir, 'runtime');
    if (!moved.ok) {
      // Windows refuses to move a directory onto one a concurrent process has
      // just published. If that directory holds exactly the bytes this process
      // verified, the race was won elsewhere and provisioning has SUCCEEDED.
      const status = runtimeCacheStatus(cacheDir, rt);
      if (status !== 'ok') {
        throw new Error(`${moved.error.message} (final dir ${describeCacheStatus(status)})`);
      }
      log(`another gladius-mcp process published the runtime cache first; using it`);
      writeRuntimeMarker(cacheDir, manifest, rt);
      fs.rmSync(staging, { recursive: true, force: true });
      return;
    }
  } catch (err) {
    // Staging only. The final path is NEVER deleted here: it may hold the other
    // process's good cache, and deleting it is what produced the original
    // "server binary missing after provisioning" crash.
    fs.rmSync(staging, { recursive: true, force: true });
    throw err;
  }
  log(`runtime cache ready (provisioned): ${cacheDir}`);
}

function checkBrowserPin(browser) {
  if (!browser || typeof browser.version !== 'string' || browser.version.length === 0) {
    fail('engine manifest has no pinned browser version; refusing to run.');
  }
  const asset = browser.asset || {};
  if (isPlaceholderSha(asset.sha256) || !(asset.size > 0)) {
    fail('engine manifest has no pinned browser archive hash; refusing to run.');
  }
  if (typeof asset.url !== 'string' || !asset.url.startsWith('https://')) {
    fail('engine manifest has no https browser asset URL; refusing to run.');
  }
  if (!Array.isArray(browser.files) || browser.files.length === 0) {
    fail('engine manifest has no browser file list; refusing to run.');
  }
  if (browser.files.length !== browser.fileCount) {
    fail(`engine manifest browser file list (${browser.files.length}) != fileCount (${browser.fileCount}); refusing to run.`);
  }
}

// Re-verification on reuse: the marker must bind this exact pin, the exe
// (the security-critical file) must match name/size/sha256, and a full
// name/size walk must equal the manifest set (exact files, no extras).
// Returns null when good, else a reason.
function reverifyBrowserCache(dir, browser) {
  const exeRel = `${browser.root}/chrome-headless-shell.exe`;
  const exeWant = browser.files.find((f) => f.name === exeRel);
  if (!exeWant) return `manifest has no entry for ${exeRel}`;
  const bad = checkFile(path.join(dir, ...exeRel.split('/')), exeWant);
  if (bad) return `cached ${exeRel} invalid (${bad})`;
  const want = new Map(browser.files.map((f) => [f.name, f.size]));
  let total = 0;
  let count = 0;
  for (const a of walkFiles(dir)) {
    if (a.rel === '.ready') continue;
    const size = want.get(a.rel);
    if (size === undefined) return `cached file not in manifest: ${a.rel}`;
    if (a.size !== size) return `cached ${a.rel} size ${a.size} != manifest ${size}`;
    total += a.size;
    count++;
  }
  if (count !== browser.fileCount) return `cached file count ${count} != manifest ${browser.fileCount}`;
  if (total !== browser.totalSize) return `cached total size ${total} != manifest ${browser.totalSize}`;
  return null;
}

async function ensureBrowser(manifest, cacheDir) {
  const browser = manifest.browser;
  checkBrowserPin(browser);
  const asset = browser.asset;

  const markerPath = path.join(cacheDir, '.ready');
  const browserExeIn = (dir) => path.join(dir, ...`${browser.root}/chrome-headless-shell.exe`.split('/'));
  const reusable = (announce) => {
    let marker;
    try {
      marker = JSON.parse(fs.readFileSync(markerPath, 'utf8'));
    } catch {
      return false;
    }
    if (marker.version !== browser.version) return false;
    if (String(marker.zipSha256).toLowerCase() !== asset.sha256.toLowerCase()) return false;
    const reason = reverifyBrowserCache(cacheDir, browser);
    if (reason) {
      if (announce) log(`cached browser invalid (${reason}); re-provisioning.`);
      return false;
    }
    return true;
  };
  if (reusable(true)) {
    const ready = browserExeIn(cacheDir);
    log(`browser cache ready (reused): ${cacheDir}`);
    return ready;
  }

  try {
    await withProvisionLock(cacheDir, 'browser', async (lockDir) => {
      // The process we may have waited behind can have finished: re-check
      // before downloading ~120 MB of the same bytes.
      if (reusable(false)) {
        log(`browser cache ready (reused): ${cacheDir}`);
        return;
      }
      const reason = reverifyBrowserCache(cacheDir, browser);
      if (reason === null) {
        // Exact manifest set present, only the marker is missing (a run killed
        // between rename and marker). Adopt the bytes, don't refetch them.
        log(`browser cache verifies but is unmarked; adopting ${cacheDir}`);
        writeBrowserMarker(cacheDir, browser, asset);
        return;
      }
      let present = false;
      try {
        present = fs.statSync(cacheDir).isDirectory();
      } catch {
        present = false;
      }
      if (present) {
        log(`browser cache at ${cacheDir} is unusable (${reason}); replacing it`);
        await removeCacheTree(cacheDir);
      }
      await provisionBrowser(manifest, cacheDir, browser, asset, lockDir);
    });
  } catch (err) {
    const reason = reverifyBrowserCache(cacheDir, browser);
    const state = reason === null ? 'present and verified' : (() => {
      try {
        fs.statSync(cacheDir);
        return `present but did not verify (${reason})`;
      } catch {
        return 'absent';
      }
    })();
    fail(
      `browser provisioning failed for ${cacheDir}: ${String(err.message).replace(/\.\s*$/, '')}. Cache dir: ${state}. ` +
      `The browser is fetched from Google's Chrome-for-Testing endpoint at first run and is never bundled; ` +
      `check network access to ${asset.url}. Retry \`npx -y gladius-mcp@latest\`; if it keeps failing, delete ${cacheDir} ` +
      `and any sibling ${path.basename(cacheDir)}.lock / .staging-* directories, then retry.`
    );
  }
  const ready = browserExeIn(cacheDir);
  return ready;
}

function writeBrowserMarker(dir, browser, asset) {
  fs.writeFileSync(
    path.join(dir, '.ready'),
    JSON.stringify({ version: browser.version, zipSha256: asset.sha256.toLowerCase() }, null, 2)
  );
}

async function provisionBrowser(manifest, cacheDir, browser, asset, lockDir) {
  log(`provisioning browser cache: ${cacheDir}`);
  log(`downloading ${asset.url} (${asset.size} bytes)`);
  let data;
  try {
    data = await downloadToMemory(asset.url);
  } catch (err) {
    throw new Error(`cannot provision the pinned browser (${err.message})`);
  }
  heartbeatLock(lockDir);

  // Integrity: mandatory on every path, first attempt or retry.
  if (data.length !== asset.size) {
    throw new Error(`browser download size ${data.length} != manifest ${asset.size}; wrote nothing.`);
  }
  const digest = crypto.createHash('sha256').update(data).digest('hex');
  if (digest !== asset.sha256.toLowerCase()) {
    throw new Error('browser download sha256 mismatch; wrote nothing.');
  }

  reapStaleSiblings(cacheDir);

  // Extract into a staging dir, verify EVERY file against the manifest
  // (exact set, no extras), then rename: the target dir is never observed
  // half-written.
  const staging = `${cacheDir}.staging-${process.pid}`;
  fs.rmSync(staging, { recursive: true, force: true });
  try {
    fs.mkdirSync(staging, { recursive: true });
    const names = extractZip(data, staging);
    log(`extracted ${names.length} entries`);
    const want = new Map(browser.files.map((f) => [f.name, f]));
    if (names.length !== browser.fileCount) {
      throw new Error(`extracted file count ${names.length} != manifest ${browser.fileCount}`);
    }
    let total = 0;
    for (const n of names) {
      const e = want.get(n);
      if (!e) throw new Error(`extracted file not in manifest: ${n}`);
      const bad = checkFile(path.join(staging, ...n.split('/')), e);
      if (bad) throw new Error(`extracted ${n} invalid (${bad})`);
      total += e.size;
    }
    if (total !== browser.totalSize) {
      throw new Error(`extracted total size ${total} != manifest ${browser.totalSize}`);
    }
    const exeRel = `${browser.root}/chrome-headless-shell.exe`;
    const exeEntry = want.get(exeRel);
    const exeHash = sha256File(path.join(staging, ...exeRel.split('/')));
    const exeLen = fs.statSync(path.join(staging, ...exeRel.split('/'))).size;
    log(`check staged ${exeRel}: computed=${exeHash} expected=${exeEntry.sha256} size=${exeLen} expected-size=${exeEntry.size}`);
    writeBrowserMarker(staging, browser, asset);
    heartbeatLock(lockDir);
    const moved = await renameWithRetry(staging, cacheDir, 'browser');
    if (!moved.ok) {
      // Same rule as the runtime: if the final dir now matches the manifest
      // exactly, another process won the race and provisioning succeeded.
      const reason = reverifyBrowserCache(cacheDir, browser);
      if (reason !== null) {
        throw new Error(`${moved.error.message} (final dir did not verify: ${reason})`);
      }
      log(`another gladius-mcp process published the browser cache first; using it`);
      writeBrowserMarker(cacheDir, browser, asset);
      fs.rmSync(staging, { recursive: true, force: true });
      return;
    }
  } catch (err) {
    // Staging only: the final path may hold the other process's good cache.
    fs.rmSync(staging, { recursive: true, force: true });
    throw err;
  }
  log(`browser cache ready (provisioned): ${cacheDir}`);
}

function runServer(cacheDir, manifest, browserExe) {
  const exe = path.join(cacheDir, manifest.runtime.server.name);
  if (!fs.existsSync(exe)) {
    // Provisioning reported success, so this is a filesystem surprise, not a
    // download problem: name the tree, its siblings, and what is left in it.
    let listing = '<unreadable>';
    try {
      listing = fs.readdirSync(cacheDir).join(', ') || '<empty>';
    } catch { /* missing entirely */ }
    fail(
      `server binary missing after provisioning: ${exe}. ` +
      `Cache dir ${cacheDir} contains: ${listing}. ` +
      `Delete ${cacheDir} and any sibling ${path.basename(cacheDir)}.lock / .staging-* directories, then retry ` +
      '`npx -y gladius-mcp@latest --doctor`.'
    );
  }
  // Bare gladius with no args serves MCP over stdio (src/main.rs:
  // None | Some("mcp") => serve). stdio is inherited so the child IS the
  // MCP channel; the launcher writes nothing to stdout on this path.
  // Ctrl+C reaches both processes on Windows (shared console), so no
  // signal relay is needed; the exit code is forwarded below.
  const env = { ...process.env };
  if (browserExe) env[BROWSER_PATH_OVERRIDE_ENV] = browserExe;
  env.GLADIUS_EXPECTED_VERSION = loadPackageVersion();
  const child = spawn(exe, [], { stdio: 'inherit', env });
  child.on('error', (err) => fail(`cannot start ${exe}: ${err.message}`));
  child.on('exit', (code, signal) => {
    if (signal) {
      log(`server terminated by signal ${signal}`);
      process.exit(1);
    }
    process.exit(code === null ? 1 : code);
  });
}

async function main() {
  const args = process.argv.slice(2);
  const manifest = loadManifest();
  if (manifest.package !== 'gladius-mcp') fail('engine manifest names the wrong package.');
  const pkgVersion = loadPackageVersion();
  // Fail closed on a stale manifest: the runtime cache leaf derives from
  // manifest.serverVersion, so agreement here is what binds the selected
  // cache directory to this launcher's version. Never take "whatever is
  // cached": a skew refuses with the reinstall line instead of provisioning
  // or serving a mixed-version pair.
  if (manifest.serverVersion !== pkgVersion) {
    fail(
      `version skew: launcher ${pkgVersion} vs manifest serverVersion ${manifest.serverVersion} ` +
      `— reinstall with \`npx -y gladius-mcp@latest\` (then re-run --doctor); refusing to serve`
    );
  }
  if (args.includes('--help') || args.includes('-h')) usage(0, process.stdout);
  if (args.includes('--version') || args.includes('version')) {
    const known = new Set(['--version', 'version']);
    const unknown = args.filter((a) => !known.has(a));
    if (unknown.length > 0) usage(2, process.stderr);
    process.stdout.write(pkgVersion + '\n');
    process.exit(0);
  }
  if (args.includes('--doctor') || args.includes('--print-manifest')) {
    const known = new Set(['--doctor', '--print-manifest']);
    const unknown = args.filter((a) => !known.has(a));
    if (unknown.length > 0) usage(2, process.stderr);
    printDoctor(manifest); // exits 0; no network on this path by construction
  }
  // Every known flag returns above; anything left is unknown.
  if (args.length > 0) usage(2, process.stderr);

  if (process.platform !== 'win32' && process.env[PLATFORM_OVERRIDE_ENV] !== '1') {
    fail(
      `unsupported platform '${process.platform}': gladius-mcp ships the windows-x86_64 runtime only. ` +
      `Set ${PLATFORM_OVERRIDE_ENV}=1 to bypass this gate for diagnostics (the server still will not run elsewhere).`
    );
  }

  const runtime = resolveRuntimeDir(manifest);
  if (runtime.dir.startsWith('<unresolvable>')) {
    fail(`cannot resolve runtime cache dir: ${runtime.notes.join('; ')}. Set ${RUNTIME_CACHE_OVERRIDE_ENV} to an absolute directory.`);
  }
  const readyRuntime = await ensureRuntime(manifest, runtime.dir);

  // An explicit GLADIUS_CHROMIUM wins: the user's binary is their
  // responsibility, so no browser is provisioned at all on that path.
  const override = process.env[BROWSER_PATH_OVERRIDE_ENV];
  if (override && override.trim() !== '') {
    log(`using explicit browser from ${BROWSER_PATH_OVERRIDE_ENV}: ${override}`);
    runServer(readyRuntime, manifest, null);
    return;
  }
  const browser = resolveBrowserDir(manifest);
  if (browser.dir.startsWith('<unresolvable>')) {
    fail(`cannot provision the pinned browser: ${browser.notes.join('; ')}. Set ${BROWSER_CACHE_OVERRIDE_ENV} to an absolute directory.`);
  }
  const browserExe = await ensureBrowser(manifest, browser.dir);
  runServer(readyRuntime, manifest, browserExe);
}

main().catch((err) => fail(err && err.message ? err.message : String(err)));
