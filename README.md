<p align="center">
  <img src="assets/icon.svg" width="120" height="120" alt="gladius icon" />
</p>
<h1 align="center">gladius</h1>
<p align="center"><strong>Drive the owned Obscura engine over CDP from any MCP client — Rust.</strong></p>
<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/github/license/nathwn12/gladius" alt="License" /></a>
  <img src="https://img.shields.io/github/languages/top/nathwn12/gladius" alt="Language" />
  <img src="https://img.shields.io/badge/platform-windows--64bit-2b3137?logo=windows&logoColor=white" alt="Platform" />
</p>

## Project name and description

Gladius is a browser-use MCP server written in Rust. It drives the owned
Obscura engine directly over CDP (Chrome DevTools Protocol) via WebSocket
and serves MCP over stdio — one static native binary, no Node.js and no
Playwright in the runtime path. MCP transport is hand-rolled JSON-RPC 2.0
over stdin/stdout, newline-delimited (one JSON object per line); STDOUT
carries only protocol frames and all logs go to STDERR. The entire server
is a single native binary: the owned Obscura engine is driven directly over
CDP via WebSocket, so there are no driver binaries and no Node runtime
between the agent and the browser, and the browser lifecycle (spawn the owned
engine, reap the whole process tree on `close`) is managed inside the binary.

## Technology stack

| Layer | Technology | Version |
| --- | --- | --- |
| Language | Rust | MSRV 1.88 (`Cargo.toml` `rust-version`, lockfile-coherent floor) |
| Async runtime | tokio | 1 (`full` features) |
| CDP transport | tokio-tungstenite | 0.26 (`connect`, `stream`, `rustls-tls-webpki-roots`) |
| HTTP fast path | reqwest | 0.12 (`rustls-tls`, no default features) |
| Serialization | serde_json | 1 |
| Temp files | tempfile | 3 |
| Engine | Obscura (`obscura.exe`, `obscura-worker.exe`) | pinned v0.2.4, sha256-verified via `scripts/fetch-engine.ps1` |
| Protocol | MCP over stdio, hand-rolled JSON-RPC 2.0 | no `rmcp` SDK, no external framing crate |

## Architecture

One static binary, three layers: the MCP stdio frame (`src/mcp.rs`), the
tool dispatch (`src/tools.rs`), and the owned-engine CDP session
(`src/browser.rs`), plus the browser-free HTTP fast path (`src/fetch.rs`).

- **Transport:** JSON-RPC 2.0 over stdio, newline-delimited, hand-rolled over
  `tokio` stdio and `serde_json`. Notifications (requests with no `id`) get
  no reply. Lifecycle: `initialize` → `tools/list` → `tools/call`.
- **Backend:** one owned engine plus explicit attach. The owned Obscura
  engine is launched as `obscura serve --host 127.0.0.1 --port <free>
  --allow-private-network --allow-file-access` beside `gladius.exe`
  (`CREATE_NO_WINDOW`), readiness via `GET /json/version`. Setting
  `GLADIUS_CDP_URL` attaches to an existing endpoint instead (`ws://`/`wss://`
  connects directly; `http://`/`https://` resolves via `GET <base>/json/version`).
- **Ownership rule (enforced in code):** an attached engine is NOT ours —
  `close` detaches the CDP session and never kills, never sweeps. Every owned
  spawn is RAII-guarded: any startup failure kills the process tree, so no
  survivors.
- **Capability declaration:** `GLADIUS_BACKEND_CAPS` is `render` (default) or
  `dom-only`. With `dom-only` there is no rendering pipeline: `screenshot`
  refuses with a typed error instead of a fake image. `initialize` reports
  `serverInfo.backend` (`obscura` or `external-cdp`) and `serverInfo.caps`
  (`render` or `dom-only`).

> [!NOTE]
> Known engine quirks on the Obscura path: JS dialogs are inert
> (`alert`/`confirm`/`prompt` are silent no-ops), the `console` tool's
> `Log.entryAdded` leg is blind (`Runtime.consoleAPICalled` still flows), and
> `upload` hard-requires the `--allow-file-access` gate flag (gladius always
> passes it). See [docs/OBSCURA-COMPAT.md](docs/OBSCURA-COMPAT.md).

Deeper reading: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md),
[docs/ENGINE.md](docs/ENGINE.md), [docs/OBSCURA-COMPAT.md](docs/OBSCURA-COMPAT.md).

## Getting started

**Prerequisites:** Windows x64. A stable Rust toolchain (`rustup`) to build;
running needs none. No Node.js, no Python, no Playwright at runtime — one
static binary plus the two engine files.

**Install from source (the current route)** — build the binary, then provision
the pinned engine (downloads Obscura v0.2.4, sha256-verified, into `bin\`):

```powershell
cargo build --release
powershell -ExecutionPolicy Bypass -File scripts/fetch-engine.ps1
```

**Run** — the engine must sit beside the running executable. Engine
resolution (`src/browser.rs` `find_obscura`): `GLADIUS_OBSCURA` wins when set
and non-blank, otherwise `obscura.exe` as a sibling of the running executable:

```powershell
# copy the engine + binary into one directory and run from there
New-Item -ItemType Directory -Force -Path gladius-dist | Out-Null
Copy-Item target\release\gladius.exe, bin\obscura.exe, bin\obscura-worker.exe -Destination gladius-dist
gladius-dist\gladius.exe mcp

# or keep the repo layout and point GLADIUS_OBSCURA at the provisioned engine
$env:GLADIUS_OBSCURA = "bin\obscura.exe"
target\release\gladius.exe mcp
```

A missing engine is a clear error naming `scripts/fetch-engine.ps1` — gladius
never downloads anything by itself and never falls back to another browser.

**Wire up your MCP client** (flags verified against `src/main.rs`: `mcp` is
the default, no `args`-style flags exist — the binary takes a single
positional subcommand):

```jsonc
// OpenCode V2 — opencode.json — mcp.servers.<name>, type: "local", command array
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

```jsonc
// Claude Desktop — claude_desktop_config.json — mcpServers
{
  "mcpServers": {
    "gladius": {
      "command": "C:\\tools\\gladius\\gladius.exe",
      "args": ["mcp"]
    }
  }
}
```

### Configuration

One owned engine (Obscura, pinned in `bin\`), plus an explicit external attach
by URL:

| Variable | Meaning |
| --- | --- |
| `GLADIUS_OBSCURA` | Explicit Obscura engine binary path; overrides the sibling-of-exe default lookup. |
| `GLADIUS_STARTUP_TIMEOUT_SECS` | Owned-engine readiness deadline (default 30). |
| `GLADIUS_CDP_URL` | Attach instead of launching: a `ws://` or `wss://` endpoint, or an `http://`/`https://` base resolved via `GET <base>/json/version`. The attached engine is NOT owned. |
| `GLADIUS_BACKEND_CAPS` | `render` (default) or `dom-only`. |

> [!NOTE]
> On `wss://`: the client TLS stack is compiled in (rustls) so a `wss://`
> endpoint is accepted. A live `ws://` attach is proven end-to-end; `wss://` is
> verified only to the TLS-handshake stage against loopback.

## Project structure

```text
src/
  main.rs      # CLI: mcp (default) | selftest | --version|version
  mcp.rs       # hand-rolled JSON-RPC 2.0 stdio server
  tools.rs     # the 16-tool surface (tools/list order) + typed error contract
  browser.rs   # owned-engine spawn/readiness + CDP session, engine resolution
  fetch.rs     # browser-free fetch_text fast path
  selftest.rs  # internal end-to-end check (gladius.exe selftest)
tests/         # integration suites: actionability, dialog_tabs, e2e_mcp,
               # hard_kill_reap, navigate_status, new_tools, refs, envelope,
               # attach_external, viewport, no_console_flash, review_findings
scripts/fetch-engine.ps1  # provision pinned Obscura v0.2.4 (sha256-verified)
docs/          # ARCHITECTURE, ENGINE, OBSCURA-COMPAT, RESEARCH, OPT-SURVEY, HEADED-MODE
bin\           # provisioned engine (obscura.exe + obscura-worker.exe, gitignored)
```

Deeper reading:

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — architecture and tool contract
- [docs/ENGINE.md](docs/ENGINE.md) — engine notes
- [docs/OBSCURA-COMPAT.md](docs/OBSCURA-COMPAT.md) — Obscura capability record
- [docs/RESEARCH.md](docs/RESEARCH.md) — browser-automation MCP landscape survey
- [docs/OPT-SURVEY.md](docs/OPT-SURVEY.md) — optimization and feature survey with measured timings
- [docs/HEADED-MODE.md](docs/HEADED-MODE.md) — headed-mode decision record (designed, not built)

## Key features

16 tools over the active page, in the order `tools/list` emits them
(`src/tools.rs`). `fetch_text` is the browser-free fast path and is one of
these 16 — not an extra:

- `navigate` — navigate the active page to a URL.
- `snapshot` — AX snapshot: one line per actionable node with a stable ref
  (`@e1`, `@e2`, …), role/name/value, plus page text (capped at 8000 chars).
  Pass `{"mode":"text"}` for the legacy text-only output.
- `screenshot` — PNG screenshot of the active page; `fullPage` for the full
  page. Refuses under `dom-only`.
- `click` — click an element by CSS selector or AX ref (exact, never first-match).
- `type` — type text into an element by CSS selector or AX ref, with optional submit.
- `evaluate` — evaluate a JS expression in the page, awaiting promises;
  evaluating `undefined` is a tool error (`no value`), distinct from explicit
  `null` which yields `{"value":null}`.
- `wait_for` — wait for a selector/text, a delay, or network idle; never exceeds
  its budget, clamped to 15s max; `ms: 0` is an immediate no-op success;
  `networkIdle:true` waits for a quiet window of `idleMs` (default 500).
- `console` — buffered console messages plus uncaught exceptions (ring buffer,
  cap 200); `action` get (default) | drain | clear, `limit` caps the returned
  count (default 100).
- `upload` — set files on an `<input type=file>` by CSS selector or AX ref;
  `paths[]` must exist locally.
- `fetch_text` — the fast path: a plain HTTP GET with no browser, HTML stripped
  to text.
- `hover` — hover over an element by CSS selector or AX ref.
- `select` — choose a `<select>` option by `value` (preferred) or visible `text`.
- `key` — press a named key (Enter, Tab, Escape, Backspace, ArrowDown, …) on
  the focused element.
- `viewport` — set the page viewport in CSS pixels (`width`, `height`, optional
  `deviceScaleFactor` default 1 and `mobile` default false), returning the
  applied metrics next to a live read-back of
  `window.innerWidth`/`innerHeight`/`devicePixelRatio` (`matched` is false when
  the engine did not honour the request). `reset:true` clears the override.
  The override is replayed on every new tab.
- `tabs` — list, open, close, or select browser tabs.
- `close` — shut down the owned browser process tree, or detach from an
  attached external engine (which is left running).

Every tool error carries a stable machine-readable `structuredContent`
`{code, reason, …}` alongside the unchanged prose in `content[0].text` — the
prose is byte-for-byte what it always was, so clients that pattern-match
English keep working.

## Development workflow

```powershell
cargo build --release
cargo test
cargo clippy --all-targets
target\release\gladius.exe selftest # internal end-to-end check, exits 0 on pass
target\release\gladius.exe --version # crate version + the commit it was built from
```

`gladius.exe selftest` runs the internal end-to-end check without a client
(exit 0 on pass). Changes are recorded in [CHANGELOG.md](CHANGELOG.md) (Keep a
Changelog format). No release is published yet — releases are cut deliberately
from a `v*` tag as a 7-file bundle (`gladius.exe`, `obscura.exe`,
`obscura-worker.exe`, `LICENSE`, `THIRD-PARTY-NOTICES.md`, `README.md`,
`THIRD-PARTY-LICENSES/Apache-2.0.txt`) with a matching `.sha256`.

## Coding standards

- Rust, edition 2021; MSRV 1.88 is the lockfile-coherent floor — do not lower it.
- `cargo clippy --all-targets` ships with 0 warnings; `cargo test` green.
- Tools stay dumb primitives — no planner, no LLM, no judgement-retries in the
  server. The selector grammar stays tiny; anything fancier goes through `evaluate`.
- Tool errors are typed at the site (`code`/`reason` plus the real offending
  value, never derived by re-reading the prose); untyped engine failures use
  the `error` fallback with a code only.
- A guard test fails the build if any `chrome`/`ms-playwright`/`playwright`
  reference reappears in `src/` — gladius is self-contained on the owned
  Obscura engine.
- No `[profile.release]` block: the release defaults are already the
  build-time-friendly set; dev keeps `line-tables-only` debuginfo for
  file:line backtraces.

## Testing

- `cargo test` — integration suites in `tests/` (`actionability`,
  `dialog_tabs`, `e2e_mcp`, `hard_kill_reap`, `navigate_status`, `new_tools`,
  `refs`, `envelope`, `attach_external`, `viewport`, `no_console_flash`,
  `review_findings`).
- `gladius.exe selftest` — internal end-to-end check against the provisioned
  engine, exit 0 on pass (`SELFTEST: ALL PASS`).
- `cargo clippy --all-targets` — 0 warnings.

**Unverified** — as-is, use at your own risk.

## Contributing

Issues welcome. Keep the tool contract stable: no tool rename, no argument or
message change without a matching guard-test and CHANGELOG entry. Verify with
`cargo test`, `cargo clippy --all-targets`, and `gladius.exe selftest` before
opening a PR. The bundled engine is third-party (Apache-2.0, upstream
`h4ckf0r0day/obscura`) — see [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

## License

MIT — see [LICENSE](LICENSE). The shipped engine binaries are third-party —
see [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

## Credits

The shipped engine binaries are third-party: Obscura by `h4ckf0r0day`
(Apache-2.0), unaffiliated with this project — see
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

<!-- nathwn12 house footer: identical in every repo. Do not edit. -->
<p align="center">
  <a href="https://github.com/nathwn12"><img src="assets/signature.svg" width="96" alt="nathwn12" /></a>
</p>
<p align="center">
  <sub>Built by <a href="https://github.com/nathwn12">nathwn12</a> · <a href="https://github.com/nathwn12?tab=repositories">All projects</a></sub>
</p>
