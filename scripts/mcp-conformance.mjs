#!/usr/bin/env node
// MCP conformance check for the skills extension + function-calling schemas.
// Zero dependencies. Speaks newline-delimited JSON-RPC over stdio.
//
// Usage:
//   node mcp-conformance.mjs --node <script.js>
//   node mcp-conformance.mjs --bin <path.exe>
//   node mcp-conformance.mjs --cmd-file <file.json>   (JSON array of argv strings)
//   add --json for machine output, --timeout <ms> per request (default 20000)
//
// What it asserts (protocol revision 2026-07-28):
//  1. tools/list: every tool's inputSchema ROOT is type "object" and carries none of
//     oneOf/anyOf/allOf/enum/const/not. Property-level combinators are allowed.
//  2. resources capability advertised => resources/list answers (never -32601).
//  3. skills extension advertised => skills/list result carries resultType "complete",
//     ttlMs (number), cacheScope (string), and entries with uri + name.
//  4. skills/get and resources/read answer for the first advertised skill uri.
// Exit: 0 conformant, 1 findings, 2 usage/transport failure.

import { spawn } from 'node:child_process';
import { readFileSync } from 'node:fs';
import process from 'node:process';

const FORBIDDEN_ROOT = ['oneOf', 'anyOf', 'allOf', 'enum', 'const', 'not'];

function parseArgs(argv) {
  const opts = { json: false, timeout: 20000, protocol: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--json') opts.json = true;
    else if (a === '--protocol') opts.protocol = argv[++i];
    else if (a === '--node') opts.argv = [process.execPath, argv[++i]];
    else if (a === '--bin') opts.argv = [argv[++i]];
    else if (a === '--cmd-file') {
      const raw = JSON.parse(readFileSync(argv[++i], 'utf8'));
      if (!Array.isArray(raw) || !raw.every((x) => typeof x === 'string')) {
        throw new Error('--cmd-file must contain a JSON array of strings');
      }
      opts.argv = raw;
    } else if (a === '--timeout') opts.timeout = Number(argv[++i]);
    else throw new Error('unknown argument: ' + a);
  }
  if (!opts.argv || opts.argv.length === 0) throw new Error('one of --node/--bin/--cmd-file is required');
  return opts;
}

const findings = [];
const note = (code, detail) => findings.push({ code, detail });

function rpc(child, state, method, params) {
  const id = ++state.nextId;
  const payload = { jsonrpc: '2.0', id, method };
  if (params !== undefined) payload.params = params;
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      state.pending.delete(id);
      resolve({ error: { code: 'timeout', message: 'no reply to ' + method } });
    }, state.timeout);
    state.pending.set(id, (msg) => {
      clearTimeout(timer);
      resolve(msg);
    });
    child.stdin.write(JSON.stringify(payload) + '\n');
  });
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const [cmd, ...args] = opts.argv;
  const child = spawn(cmd, args, { stdio: ['pipe', 'pipe', 'pipe'] });
  const state = { nextId: 0, timeout: opts.timeout, pending: new Map() };
  let buf = '';
  let stderr = '';

  child.stdout.on('data', (d) => {
    buf += d.toString();
    let nl;
    while ((nl = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, nl);
      buf = buf.slice(nl + 1);
      if (!line.trim()) continue;
      let msg;
      try { msg = JSON.parse(line); } catch { continue; }
      if (msg.id !== undefined && state.pending.has(msg.id)) {
        const resolve = state.pending.get(msg.id);
        state.pending.delete(msg.id);
        resolve(msg);
      }
    }
  });
  child.stderr.on('data', (d) => { stderr += d.toString(); });

  const exited = new Promise((resolve) => child.on('exit', (code) => resolve(code)));

  try {
    const init = await rpc(child, state, 'initialize', {
      protocolVersion: opts.protocol || '2026-07-28',
      capabilities: {},
      clientInfo: { name: 'mcp-conformance', version: '1.0.0' },
    });
    if (init.error) {
      note('initialize-failed', JSON.stringify(init.error));
      return finish(child, opts, null);
    }
    const result = init.result || {};
    const capabilities = result.capabilities || {};
    const advertised = Object.keys(capabilities);

    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n');

    const tools = await rpc(child, state, 'tools/list', {});
    const toolList = (tools.result && tools.result.tools) || [];
    for (const tool of toolList) {
      const schema = tool.inputSchema || {};
      if (schema.type !== 'object') {
        note('tool-schema-root', tool.name + ': root type is ' + JSON.stringify(schema.type) + ', expected "object"');
      }
      for (const key of FORBIDDEN_ROOT) {
        if (Object.prototype.hasOwnProperty.call(schema, key)) {
          note('tool-schema-combinator', tool.name + ': root carries "' + key + '"');
        }
      }
    }

    const hasResources = Object.prototype.hasOwnProperty.call(capabilities, 'resources');
    if (hasResources) {
      const list = await rpc(child, state, 'resources/list', {});
      if (list.error) {
        note('resources-list-missing', 'resources capability advertised but resources/list failed: ' + JSON.stringify(list.error));
      } else if (!Array.isArray(list.result && list.result.resources)) {
        note('resources-list-shape', 'resources/list result has no resources array');
      }
    }

    const extensions = capabilities.extensions || {};
    const hasSkills = Object.prototype.hasOwnProperty.call(extensions, 'io.modelcontextprotocol/skills');
    if (hasSkills) {
      if (!hasResources) note('skills-without-resources', 'skills extension advertised without the resources capability');
      const skills = await rpc(child, state, 'skills/list', {});
      if (skills.error) {
        note('skills-list-error', JSON.stringify(skills.error));
      } else {
        const r = skills.result || {};
        if (r.resultType !== 'complete') note('skills-resulttype', 'skills/list resultType is ' + JSON.stringify(r.resultType) + ', expected "complete"');
        if (typeof r.ttlMs !== 'number') note('skills-ttl', 'skills/list ttlMs is missing or not a number');
        if (typeof r.cacheScope !== 'string') note('skills-cache', 'skills/list cacheScope is missing or not a string');
        const entries = Array.isArray(r.skills) ? r.skills : null;
        if (!entries) {
          note('skills-list-shape', 'skills/list has no skills array');
        } else {
          for (const s of entries) {
            if (!s.uri || !s.name) note('skills-entry-shape', 'skill entry missing uri or name: ' + JSON.stringify(s).slice(0, 120));
          }
          const first = entries.find((s) => typeof s.uri === 'string');
          if (first) {
            const got = await rpc(child, state, 'skills/get', { uri: first.uri });
            if (got.error) note('skills-get-error', first.uri + ': ' + JSON.stringify(got.error));
            else if ((got.result || {}).resultType !== 'complete') note('skills-get-resulttype', first.uri + ': resultType is not "complete"');
            const read = await rpc(child, state, 'resources/read', { uri: first.uri });
            if (read.error) note('resources-read-error', first.uri + ': ' + JSON.stringify(read.error));
          }
        }
      }
    }

    return finish(child, opts, { advertised, tools: toolList.length, skills: hasSkills, resources: hasResources, protocolVersion: result.protocolVersion });
  } finally {
    child.stdin.end();
    const code = await Promise.race([exited, new Promise((r) => setTimeout(() => r('timeout'), 3000))]);
    try { child.kill(); } catch {}
    if (code === 'timeout') note('did-not-exit', 'server did not exit within 3s of stdin close');
    if (stderr.trim() && findings.length) note('stderr', stderr.trim().split('\n').slice(0, 3).join(' | '));
  }
}

function finish(child, opts, info) {
  if (opts.json) {
    console.log(JSON.stringify({ ok: findings.length === 0, info, findings }, null, 2));
  } else if (findings.length === 0) {
    console.log('mcp-conformance: OK (' + (info ? info.tools + ' tools, protocol ' + info.protocolVersion : 'no info') + ')');
  } else {
    console.log('mcp-conformance: ' + findings.length + ' finding(s)');
    for (const f of findings) console.log('  [' + f.code + '] ' + f.detail);
  }
  process.exitCode = findings.length === 0 ? 0 : 1;
}

main().catch((e) => {
  console.log('mcp-conformance: usage error: ' + (e && e.message ? e.message : e));
  process.exitCode = 2;
});
