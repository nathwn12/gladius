<p align="center">
  <img src="assets/icon.svg" alt="gladius" width="120" height="120">
</p>
<h1 align="center">gladius</h1>
<p align="center"><strong>Drive the owned Obscura engine over CDP from any MCP client — Rust.</strong></p>
<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/github/license/nathwn12/gladius" alt="License" /></a>
  <img src="https://img.shields.io/github/languages/top/nathwn12/gladius" alt="Language" />
  <img src="https://img.shields.io/badge/platform-windows--64bit-2b3137?logo=windows&logoColor=white" alt="Platform" />
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

15 tools over the active page, listed here in the order `tools/list` emits
them (`src/tools.rs`). `fetch_text` is the browser-free fast path and is one
of these 15 — not an extra:

- `navigate` — navigate the active page to a URL.
- `snapshot` — AX snapshot: one line per actionable node with a stable ref (`@e1`, `@e2`, …), role/name/value, plus page text. Pass `{"mode":"text"}` for the legacy text-only output; slices page text in-page before transfer, then caps at 8000 chars.
- `screenshot` — PNG screenshot of the active page; `fullPage` for the full page.
- `click` — click an element by CSS selector or AX ref (@eN from snapshot; exact, never first-match).
- `type` — type text into an element by CSS selector or AX ref, with optional submit.
- `evaluate` — evaluate a JS expression in the page, awaiting promises; evaluating `undefined` is a tool error (`no value`), distinct from explicit `null` which yields `{"value":null}`.
- `wait_for` — wait for a selector/text, a delay, or network idle; never exceeds its budget, clamped to 15s max; `ms: 0` is an immediate no-op success; `networkIdle:true` waits for a quiet window of `idleMs` (default 500).
- `console` — buffered console messages plus uncaught exceptions (ring buffer, cap 200); `action` get (default) | drain | clear, `limit` caps the returned count (default 100).
- `upload` — set files on an `<input type=file>` by CSS selector or AX ref; `paths[]` must exist locally.
- `fetch_text` — the fast path: a plain HTTP GET with no browser, HTML stripped to text.
- `hover` — hover over an element by CSS selector or AX ref.
- `select` — choose a `<select>` option by `value` (preferred) or visible `text`.
- `key` — press a named key (Enter, Tab, Escape, Backspace, ArrowDown, …) on the focused element.
- `tabs` — list, open, close, or select browser tabs.
- `close` — shut down the owned browser process tree, or detach from an attached external engine (which is left running).

JSON-RPC notifications (requests with no `id`) get no reply. Every owned
spawn is RAII-guarded: any startup failure (including a timeout waiting for
engine readiness) kills the process tree, so no survivors.

## Quick start

**Supported platforms: Windows x64 only.** The owned Obscura engine ships as
Windows binaries, so gladius runs on Windows today and nowhere else. Be
honest about the requirement: you need `gladius.exe` **plus** the engine
binaries (`obscura.exe`, `obscura-worker.exe`) beside it.

**Install from source (the current route).** Build the binary, then provision
the pinned engine on the fresh machine — this downloads Obscura `v0.2.4` and
verifies it by sha256 before placing anything in `bin\`:

```powershell
cargo build --release
powershell -ExecutionPolicy Bypass -File scripts/fetch-engine.ps1
```

Then get the engine beside the running executable. Engine resolution is
exactly the rule in `src/browser.rs` `find_obscura`: `GLADIUS_OBSCURA` wins
when set and non-blank, otherwise `obscura.exe` as a sibling of the running
executable (`std::env::current_exe()` parent dir). A repo run therefore needs
one of:

```powershell
# copy the engine + binary into one directory and run from there
Copy-Item target\release\gladius.exe, bin\obscura.exe, bin\obscura-worker.exe -Destination C:\tools\gladius
C:\tools\gladius\gladius.exe mcp

# or keep the repo layout and point GLADIUS_OBSCURA at the provisioned engine
$env:GLADIUS_OBSCURA = "bin\obscura.exe"
target\release\gladius.exe mcp
```

A missing engine is a clear error naming `scripts/fetch-engine.ps1` — gladius
never downloads anything by itself and never falls back to another browser.

**Requirements:** Windows x64, a stable Rust toolchain to build (running needs
none). No Node.js, no Python, no Playwright at runtime — one static binary
plus the two engine files.

Point your MCP client at it.

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

## Configure

One owned engine (Obscura, pinned in `bin\`), plus an explicit external
attach by URL. The owned engine is spawned with `CREATE_NO_WINDOW` and killed
as a whole process tree on `close`; an attached engine is only detached,
never killed. Environment:

| Variable | Meaning |
| --- | --- |
| `GLADIUS_OBSCURA` | Explicit Obscura engine binary path; overrides the sibling-of-exe default lookup. |
| `GLADIUS_STARTUP_TIMEOUT_SECS` | Owned-engine readiness deadline (default 30). |
| `GLADIUS_CDP_URL` | Attach instead of launching: a `ws://` or `wss://` endpoint, or an `http://`/`https://` base resolved via `GET <base>/json/version`. The attached engine is NOT owned: `close` detaches the CDP session gladius opened and leaves the external process running. |
| `GLADIUS_BACKEND_CAPS` | `render` (default) or `dom-only`. With `dom-only` there is no rendering pipeline: `screenshot` refuses with a typed error instead of a fake image (`upload` needs no rendering — DOM CDP only — so it keeps working). `initialize` reports `serverInfo.backend` (`obscura` or `external-cdp`) and `serverInfo.caps` (`render` or `dom-only`). |

On `wss://`: the client TLS stack is compiled in (rustls) so a `wss://`
endpoint is accepted. A live `ws://` attach is proven end-to-end; `wss://` is
verified only to the TLS-handshake stage against loopback — a full trusted
`wss://` session against a public endpoint remains unexercised.

## Build from source

Prerequisite: a stable Rust toolchain (`rustup`). MSRV is **1.88**
(`Cargo.toml`, the lockfile-coherent floor).

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
cargo test
cargo clippy --all-targets
target\release\gladius.exe selftest
```

## Releases

No release is published yet — the repository owns no tag and no GitHub
release. When one is deliberately cut from a `v*` tag, the release workflow
(`.github/workflows/release.yml`) will build the binaries, run the suite
against the provisioned engine, assemble
`gladius-<tag>-windows-x86_64.zip`, and attach a matching `.sha256`. The zip
is a 7-file bundle:

- `gladius.exe`, `obscura.exe`, `obscura-worker.exe`
- `LICENSE`, `THIRD-PARTY-NOTICES.md`, `README.md`
- `THIRD-PARTY-LICENSES/Apache-2.0.txt`

Keep the three `.exe` files together in one directory. Verify a downloaded zip
with its `.sha256`:

```powershell
(Get-FileHash -Algorithm SHA256 gladius-<tag>-windows-x86_64.zip).Hash.ToLowerInvariant() -eq (Get-Content gladius-<tag>-windows-x86_64.zip.sha256).Split()[0]
```

## Docs

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — architecture and tool contract
- [docs/RESEARCH.md](docs/RESEARCH.md) — browser-automation MCP landscape survey
- [docs/HEADED-MODE.md](docs/HEADED-MODE.md) — headed-mode decision record (designed, not built)
- [docs/OPT-SURVEY.md](docs/OPT-SURVEY.md) — optimization and feature survey with measured timings

## Status

**Verified 2026-10-13** at `4e9b92d` (working tree):

- `cargo test` — 59 passed, 0 failed (exit 0)
- `cargo build --release` — finished (exit 0)
- release `gladius.exe selftest` from a temp dir with the engines beside the exe — `SELFTEST: ALL PASS` (exit 0)
- `cargo clippy --all-targets` — 0 warnings (exit 0)

Windows x64 only. Use at your own risk beyond the checks above.

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
