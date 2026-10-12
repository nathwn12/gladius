<p align="center">
  <img src="assets/icon.svg" alt="gladius" width="120" height="120">
</p>
<h1 align="center">gladius</h1>
<p align="center"><strong>Drive the owned Obscura engine over CDP from any MCP client — Rust.</strong></p>
<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/github/license/nathwn12/gladius" alt="License" /></a>
  <img src="https://img.shields.io/github/languages/top/nathwn12/gladius" alt="Language" />
  <img src="https://img.shields.io/badge/platform-windows--64bit-2b3137?logo=windows&logoColor=white" alt="Platform" />
  <a href="https://github.com/nathwn12/gladius/actions/workflows/ci.yml"><img src="https://github.com/nathwn12/gladius/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
</p>

## What it is

Gladius is a browser-use MCP server written in Rust. It drives the owned
Obscura engine directly over CDP (Chrome DevTools Protocol) via WebSocket
and serves MCP over stdio — one static native binary, no Node.js and no
Playwright in the runtime path. MCP transport is hand-rolled JSON-RPC 2.0
over stdin/stdout, newline-delimited (one JSON object per line); STDOUT
carries only protocol frames and all logs go to STDERR.

## Why

The entire server is a single native binary: the owned Obscura engine is
driven directly over CDP via WebSocket, so there are no driver binaries and
no Node runtime between the agent and the browser, and the browser lifecycle
(spawn the owned engine, reap the whole process tree on `close`) is managed
inside the binary.

## Tools

15 tools over the active page (plus a browser-free fast path):

- `navigate` — load a URL in the active tab.
- `snapshot` — AX snapshot: one line per actionable node with a stable ref (`@e1`, `@e2`, …) plus page text; pass `{"mode":"text"}` for the legacy text-only output; slices page text in-page before transfer, then caps at 8000 chars.
- `screenshot` — viewport PNG of the active page; `fullPage` for the full page.
- `fetch_text` — plain HTTP fetch with no browser (the fast path).
- `click` — click by CSS selector or AX ref.
- `type` — type text, with optional submit.
- `hover` — hover over a CSS selector or AX ref.
- `select` — choose an option by `value` or visible `text`.
- `key` — send named keys to the focused element.
- `upload` — set files on an `<input type=file>`.
- `evaluate` — run JS in the page, awaiting promises; evaluating `undefined` is a tool error (`no value`), distinct from explicit `null` which yields `{"value":null}`.
- `wait_for` — wait for a selector/text, a delay, or network idle; never exceeds its budget, clamped to 15s max; `ms: 0` is an immediate no-op success.
- `console` — buffered console messages plus uncaught exceptions (ring buffer, cap 200).
- `tabs` — `list`, `new`, `close`, and `select` browser tabs.
- `close` — shut down the browser process tree.

JSON-RPC notifications (requests with no `id`) get no reply. Every owned
spawn is RAII-guarded: any startup failure (including a timeout waiting for
engine readiness) kills the process tree, so no survivors.

## Install

**Supported platforms: Windows x64 only.** The owned Obscura engine ships as
Windows binaries, so gladius runs on Windows today and nowhere else. Be
honest about the requirement: you need `gladius.exe` **plus** the engine
binaries (`obscura.exe`, `obscura-worker.exe`) beside it — a release zip
already contains all three; a source build needs one provisioning step.

**Requirements:** Windows x64. No Node.js, no Python, no Playwright at
runtime — one static binary plus the two engine files. (Building from source
needs a Rust toolchain; running does not.)

1. **Release install (recommended):** download the
   `gladius-<version>-windows-x86_64.zip` from the
   [GitHub Releases](https://github.com/nathwn12/gladius/releases) page and
   extract it anywhere. The zip contains `gladius.exe`, `obscura.exe`,
   `obscura-worker.exe`, `LICENSE`, `THIRD-PARTY-NOTICES.md`, and `README.md`
   — keep the three `.exe` files together in one directory. Verify with the
   attached `.sha256` file:
   ```powershell
   (Get-FileHash -Algorithm SHA256 gladius-<version>-windows-x86_64.zip).Hash.ToLowerInvariant() -eq (Get-Content gladius-<version>-windows-x86_64.zip.sha256).Split()[0]
   ```
2. **Source install:** build (see
   [Build from source](#build-from-source)), then provision the pinned engine
   on the fresh machine — this downloads Obscura `v0.2.4` and verifies it by
   sha256 before placing anything in `bin\`:
   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts/fetch-engine.ps1
   ```
   Copy `target\release\gladius.exe`, `bin\obscura.exe`, and
   `bin\obscura-worker.exe` into one directory (or set `GLADIUS_OBSCURA` to
   the engine path). A missing engine is a clear error naming that script —
   gladius never downloads anything by itself and never falls back to another
   browser.
3. Point your MCP client at it.

**OpenCode V2** (`opencode.json` — `mcp.servers.<name>`, `type: "local"`,
`command` array):

```jsonc
{
  "mcp": {
    "servers": {
      "gladius": {
        "type": "local",
        "command": ["C:\\tools\\gladius\\gladius.exe", "mcp"]
      }
    }
  }
}
```

**Claude Desktop** (`claude_desktop_config.json` — `mcpServers`):

```jsonc
{
  "mcpServers": {
    "gladius": {
      "command": "C:\\tools\\gladius\\gladius.exe",
      "args": ["mcp"]
    }
  }
}
```

The bundled engine is third-party (Apache-2.0, upstream
`h4ckf0r0day/obscura`) — see [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

## Usage

Serve MCP over stdio (the default — no args also works):

```powershell
gladius.exe mcp
```

Self-check without a client:

```powershell
gladius.exe selftest # internal end-to-end check, exits 0 on pass
```

One owned engine (Obscura, pinned in `bin\`), plus an explicit external
attach by URL.

Backend: gladius launches the owned Obscura engine (`GLADIUS_OBSCURA` path
override, else `bin\obscura.exe` beside `gladius.exe`, provisioned by
`scripts/fetch-engine.ps1`; a missing engine is a clear error naming that
script), or attaches to an external engine when `GLADIUS_CDP_URL` is set
(never owned: `close` only detaches). The owned engine is spawned with
`CREATE_NO_WINDOW` and killed as a whole process tree on `close` (an
attached engine is only detached, never killed).

Environment:

- `GLADIUS_OBSCURA` — explicit Obscura engine binary path (overrides the
  default sibling lookup).
- `GLADIUS_STARTUP_TIMEOUT_SECS` — owned-engine readiness deadline (default 30).
- `GLADIUS_CDP_URL` — attach instead of launching: a `ws://`/`wss://` endpoint
  to connect to, or an `http://`/`https://` base resolved via
  `GET <base>/json/version`. The attached engine is NOT owned: `close`
  detaches the CDP session gladius opened and leaves the external process
  running; gladius creates no profile and kills nothing.
- `GLADIUS_BACKEND_CAPS` — `render` (default) or `dom-only`. With `dom-only`
  there is no rendering pipeline: `screenshot` refuses with a typed error
  instead of returning a fake image (`upload` needs no rendering — DOM CDP
  only — so it keeps working). `initialize` reports
  `serverInfo.backend` (`obscura` or `external-cdp`) and
  `serverInfo.caps` (`render` or `dom-only`).

## Build from source

Prerequisite: a stable Rust toolchain (`rustup`).

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

## Status

**Unverified** — as-is, use at your own risk.

## License

MIT — see [LICENSE](LICENSE). The shipped engine binaries are third-party —
see [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

<!-- nathwn12 house footer: identical in every repo. Do not edit. -->
<p align="center">
  <a href="https://github.com/nathwn12"><img src="assets/signature.svg" width="96" alt="nathwn12" /></a>
</p>
<p align="center">
  <sub>Built by <a href="https://github.com/nathwn12">nathwn12</a> · <a href="https://github.com/nathwn12?tab=repositories">All projects</a></sub>
</p>
