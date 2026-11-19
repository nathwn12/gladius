#!/usr/bin/env node
'use strict';

const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const zlib = require('node:zlib');
const { spawn, execFileSync } = require('node:child_process');

const PKG_DIR = path.dirname(path.dirname(__filename));
const MANIFEST_PATH = path.join(PKG_DIR, 'engine-manifest.json');
const PKG_JSON_PATH = path.join(PKG_DIR, 'package.json');
const PLATFORM_OVERRIDE_ENV = 'GLADIUS_MCP_ALLOW_NON_WINDOWS';
const RUNTIME_CACHE_OVERRIDE_ENV = 'GLADIUS_ENGINE_CACHE';
const RUNTIME_BUNDLE_OVERRIDE_ENV = 'GLADIUS_ENGINE_BUNDLE';
const BROWSER_CACHE_OVERRIDE_ENV = 'GLADIUS_CHROMIUM_CACHE';
const BROWSER_PATH_OVERRIDE_ENV = 'GLADIUS_CHROMIUM';
const DOWNLOAD_TIMEOUT_MS = 15 * 60 * 1000;
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

function isPlaceholderSha(sha) {
  return typeof sha !== 'string' || !/^[0-9a-fA-F]{64}$/.test(sha);
}

function shortHash(sha) {
  return isPlaceholderSha(sha) ? 'unpublished' : sha.toLowerCase().slice(0, 8);
}

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

function engineCommit(runtimeDir, manifest) {
  if (runtimeDir.startsWith('<unresolvable>')) return null;
  const exe = path.join(runtimeDir, manifest.runtime.server.name);
  if (!fs.existsSync(exe)) return null;
  try {
    const out = execFileSync(exe, ['--version'], {
      encoding: 'utf8',
      timeout: 5000,
      stdio: ['ignore', 'pipe', 'ignore'],
    }).trim();
    const m = /\(([0-9a-f]{40})(-dirty)?\)/.exec(out);
    return m ? m[1] + (m[2] || '') : null;
  } catch {
    return null;
  }
}

function printDoctor(manifest) {
  const runtime = resolveRuntimeDir(manifest);
  const browser = resolveBrowserDir(manifest);
  const commit = typeof manifest.commit === 'string' && /^[0-9a-f]{40}$/.test(manifest.commit)
    ? manifest.commit
    : 'unpublished';
  const report = {
    package: manifest.package,
    serverVersion: manifest.serverVersion,
    commit,
    commitSource: 'engine-manifest.json, written by the release workflow from git rev-parse HEAD (the committed value is the unpublished sentinel)',
    engineCommit: engineCommit(runtime.dir, manifest),
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


function sha256File(filePath) {
  const h = crypto.createHash('sha256');
  h.update(fs.readFileSync(filePath));
  return h.digest('hex');
}

function checkFile(filePath, want) {
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

const LOCK_WAIT_MS = 25 * 60 * 1000;
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
  }
}

function releaseProvisionLock(lockDir) {
  if (!lockDir) return;
  const owner = readLockOwner(lockDir);
  if (owner && Number.isInteger(owner.pid) && owner.pid !== process.pid) return; // someone else's
  try {
    fs.rmSync(lockDir, { recursive: true, force: true });
  } catch {
  }
}

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

async function withProvisionLock(cacheDir, label, fn) {
  const lockDir = await acquireProvisionLock(cacheDir, label);
  try {
    return await fn(lockDir);
  } finally {
    releaseProvisionLock(lockDir);
  }
}

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
    }
  }
}

function describeCacheStatus(status) {
  if (status === 'ok') return 'present and verified';
  if (status === 'absent') return 'absent';
  return `present but did not verify (${status})`;
}

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
      if (reusable(false)) {
        log(`runtime cache ready (reused): ${cacheDir}`);
        return;
      }
      const status = runtimeCacheStatus(cacheDir, rt);
      if (status === 'ok') {
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

  if (data.length !== rt.archive.size) {
    throw new Error(`${origin} size ${data.length} != manifest ${rt.archive.size}; wrote nothing.`);
  }
  const digest = crypto.createHash('sha256').update(data).digest('hex');
  if (digest !== rt.archive.sha256.toLowerCase()) {
    throw new Error(`${origin} sha256 mismatch; wrote nothing.`);
  }

  reapStaleSiblings(cacheDir);

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
      if (reusable(false)) {
        log(`browser cache ready (reused): ${cacheDir}`);
        return;
      }
      const reason = reverifyBrowserCache(cacheDir, browser);
      if (reason === null) {
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

  if (data.length !== asset.size) {
    throw new Error(`browser download size ${data.length} != manifest ${asset.size}; wrote nothing.`);
  }
  const digest = crypto.createHash('sha256').update(data).digest('hex');
  if (digest !== asset.sha256.toLowerCase()) {
    throw new Error('browser download sha256 mismatch; wrote nothing.');
  }

  reapStaleSiblings(cacheDir);

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
    fs.rmSync(staging, { recursive: true, force: true });
    throw err;
  }
  log(`browser cache ready (provisioned): ${cacheDir}`);
}

function runServer(cacheDir, manifest, browserExe) {
  const exe = path.join(cacheDir, manifest.runtime.server.name);
  if (!fs.existsSync(exe)) {
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
