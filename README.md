# gladius

FAST, HEADLESS, ULTRA-LIGHTWEIGHT browser-use MCP server. A native binary with
**no Node.js and no Playwright in the runtime path**: it drives headless Chrome
directly over CDP (Chrome DevTools Protocol) via WebSocket.

## Build

```powershell
cargo build --release
```

## Run

```powershell
target\release\gladius.exe mcp      # serve MCP over stdio (default, no args also works)
target\release\gladius.exe selftest # internal end-to-end check, exits 0 on pass
```

MCP transport: hand-rolled JSON-RPC 2.0 over stdin/stdout, newline-delimited
(one JSON object per line). STDOUT carries only protocol frames; all logs go
to STDERR.

## Backend browser

Resolved in order: `GLADIUS_CHROME` env var -> newest
`%LOCALAPPDATA%\ms-playwright\chromium_headless_shell-*\chrome-headless-shell-win64\chrome-headless-shell.exe`
-> `C:\Program Files\Google\Chrome\Application\chrome.exe`.
Spawned headless with `CREATE_NO_WINDOW`; killed as a whole process tree on `close`.
Every spawn is RAII-guarded: ANY startup failure (including a timeout waiting
for the DevTools line) kills the process tree - no survivors.

## Env

- `GLADIUS_STARTUP_TIMEOUT_SECS` - DevTools-line startup deadline (default 30).
- `GLADIUS_TEST_STUB_SLEEP_SECS` - test-only: spawn a silent sleeper instead
  of Chrome to exercise the startup-timeout path.

## Behavior notes

- `wait_for` never exceeds its budget (clamped to 15s max); `ms: 0` is an
  immediate no-op success.
- JSON-RPC notifications (requests with no `id`) get no reply.
- `evaluate` of `undefined` is a tool error (`no value`), distinct from
  explicit `null` which yields `{"value":null}`.
- `snapshot` slices page text in-page before transfer, then caps at 8000 chars.

## Tools

`navigate` `snapshot` (text-only, no element refs) `screenshot` `click`
`hover` `select` (by `value` or visible `text`) `key` (named keys on the
focused element) `tabs` (`list`/`new`/`close`/`select`) `type` `evaluate`
`wait_for` `fetch_text` (plain HTTP, no browser - the fast path) `close`.

## Verify

```powershell
cargo test --release -- --nocapture
target\release\gladius.exe selftest
```

## Unverified / limitations

- Headless Chromium only; no headed-mode support.
- `snapshot` is text-only (no element refs) and capped at 8000 chars.
- `wait_for` budget clamped to 15s max.
