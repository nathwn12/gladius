<p align="center"><img src="assets/icon.svg" width="96" alt="gladius"></p>

<p align="center"><h1 align="center">gladius</h1></p>

<p align="center">Fast, headless, ultra-lightweight browser-use MCP server — a native binary with no Node.js and no Playwright in the runtime path.</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-green.svg" alt="license: MIT"></a>
  <img src="https://img.shields.io/badge/language-Rust-orange.svg" alt="language: Rust">
  <img src="https://img.shields.io/badge/platform-Windows-blue.svg" alt="platform: Windows">
  <a href="https://github.com/nathwn12/gladius/actions/workflows/ci.yml"><img src="https://github.com/nathwn12/gladius/actions/workflows/ci.yml/badge.svg" alt="CI status"></a>
</p>

## What it is

Gladius is a browser-use MCP server written in Rust. It drives headless Chrome
directly over CDP (Chrome DevTools Protocol) via WebSocket and serves MCP over
stdio — one static native binary, no Node.js and no Playwright in the runtime
path.

MCP transport: hand-rolled JSON-RPC 2.0 over stdin/stdout, newline-delimited
(one JSON object per line). STDOUT carries only protocol frames; all logs go
to STDERR.

## Why

- **No Node.js.** The entire server is a single native binary.
- **No Playwright.** Chrome is driven directly over CDP via WebSocket —
  no driver binaries, no Node runtime between the agent and the browser.
- **Native binary.** `cargo build --release` produces one static
  `gladius.exe`; the browser lifecycle (spawn headless, reap the whole
  process tree on `close`) is managed inside the binary.

## Features

15 tools over the active page (plus a browser-free fast path):

- **Navigate & observe:** `navigate`, `snapshot` (AX snapshot: one line per
  actionable node with a stable ref `@e1`, `@e2`, … plus page text; pass
  `{"mode":"text"}` for the legacy text-only output; slices page text in-page
  before transfer, then caps at 8000 chars), `screenshot` (viewport PNG,
  `fullPage` for full page), `fetch_text` (plain HTTP, no browser — the fast path)
- **Act:** `click` (CSS selector or AX ref), `type` (with optional submit),
  `hover`, `select` (by `value` or visible `text`), `key` (named keys on the
  focused element), `upload` (set files on an `<input type=file>`)
- **Script & sync:** `evaluate` (JS in the page, awaits promises),
  `wait_for` (selector/text, delay, or network idle; never exceeds its budget,
  clamped to 15s max; `ms: 0` is an immediate no-op success), `console`
  (buffered console messages + uncaught exceptions, ring buffer cap 200)
- **Tabs & lifecycle:** `tabs` (`list`/`new`/`close`/`select`), `close`
  (shuts down the browser process tree)

Behavior notes:

- JSON-RPC notifications (requests with no `id`) get no reply.
- `evaluate` of `undefined` is a tool error (`no value`), distinct from
  explicit `null` which yields `{"value":null}`.
- Every browser spawn is RAII-guarded: ANY startup failure (including a
  timeout waiting for the DevTools line) kills the process tree — no survivors.

## Install

A prebuilt Windows binary (`gladius.exe`) is attached to the
[GitHub Releases](https://github.com/nathwn12/gladius/releases) page.
Download it, then point your MCP client at it:

```jsonc
{
  "mcp": {
    "gladius": {
      "type": "local",
      "command": ["<path-to>\\gladius.exe", "mcp"]
    }
  }
}
```

The alternative is building from source (see
[Build from source](#build-from-source)) and pointing the `command` at your
local `target\release\gladius.exe`.

## Usage

Serve MCP over stdio (the default — no args also works):

```powershell
gladius.exe mcp
```

Wire it into your MCP client config:

```jsonc
{
  "mcp": {
    "gladius": {
      "type": "local",
      "command": ["<path-to>\\gladius.exe", "mcp"]
    }
  }
}
```

Self-check without a client:

```powershell
gladius.exe selftest # internal end-to-end check, exits 0 on pass
```

Backend browser resolution order: `GLADIUS_CHROME` env var → newest
`%LOCALAPPDATA%\ms-playwright\chromium_headless_shell-*\chrome-headless-shell-win64\chrome-headless-shell.exe`
→ `C:\Program Files\Google\Chrome\Application\chrome.exe`.
Spawned headless with `CREATE_NO_WINDOW`; killed as a whole process tree on
`close`.

Environment:

- `GLADIUS_CHROME` — explicit browser binary path (overrides detection).
- `GLADIUS_STARTUP_TIMEOUT_SECS` — DevTools-line startup deadline (default 30).
- `GLADIUS_TEST_STUB_SLEEP_SECS` — test-only: spawn a silent sleeper instead
  of Chrome to exercise the startup-timeout path.

## Build from source

```powershell
cargo build --release
```

Then run from the repo:

```powershell
target\release\gladius.exe mcp      # serve MCP over stdio (default, no args also works)
target\release\gladius.exe selftest # internal end-to-end check, exits 0 on pass
```

Verify:

```powershell
cargo test --release -- --nocapture
target\release\gladius.exe selftest
```

## Docs

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — architecture and tool contract
- [docs/RESEARCH.md](docs/RESEARCH.md) — browser-automation MCP landscape survey
- [docs/HEADED-MODE.md](docs/HEADED-MODE.md) — headed-mode decision record (designed, not built)
- [docs/OPT-SURVEY.md](docs/OPT-SURVEY.md) — optimization and feature survey with measured timings

## Unverified / limitations

- Headless Chromium only; no headed-mode support.
- `snapshot` is text-only (no element refs) and capped at 8000 chars.
- `wait_for` budget clamped to 15s max.

## License

MIT — see [LICENSE](LICENSE).
