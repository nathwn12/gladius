# Gladius ARCHITECTURE

> Gladius: headless, ultra-lightweight browser-use MCP server in Rust. Drives
> the owned Obscura engine **directly over CDP** - no Node.js, no Playwright
> in the runtime path. One static binary, MCP over stdio, lazy browser
> lifecycle. Research grounding: `docs/RESEARCH.md`.

## 1. Frozen tool contract (v1 - the surface never moves for backend swaps)

| name | args | returns |
|---|---|---|
| `navigate` | `url: string` | text after load event (timeout -> error) |
| `snapshot` | `mode?: string` (default AX) | AX tree: one line per actionable node with a stable ref (`@e1`, `@e2`, ...), role/name/value, plus page text (capped 8000 chars); `mode:"text"` = legacy text-only output |
| `screenshot` | `fullPage?: bool` (default false) | PNG bytes (base64 `image` part) of viewport / full page; refuses under `dom-only` |
| `click` | `element: string` (CSS, or AX ref `@eN`) | `{ ok: true }` after click dispatches |
| `type` | `element: string, text: string, submit?: bool` | `{ ok: true }` after keystrokes (+Enter if submit) |
| `evaluate` | `expression: string` | JSON-serialised JS result (`undefined` -> tool error, distinct from explicit `null`) |
| `wait_for` | `selector?, text?, ms?, networkIdle?: bool, idleMs?: number` | `{ ok: true }` when the condition holds or timeout elapses (max 15s) |
| `console` | `action?: get\|drain\|clear, limit?: number` | buffered console messages + uncaught exceptions (ring buffer, cap 200) |
| `upload` | `element: string, paths: string[]` | `{ ok: true }` after setting files on an `<input type=file>` |
| `fetch_text` | `url: string` | fast path: plain HTTP GET with NO browser; HTML stripped to text (readability-style, no tags) |
| `hover` | `element: string` (CSS, or AX ref `@eN`) | `{ ok: true }` after hover |
| `select` | `element: string, value?: string, text?: string` | `{ ok: true }` after choosing a `<select>` option by value (preferred) or visible text |
| `key` | `key: string` (Enter, Tab, Escape, ...) | `{ ok: true }` after the named key press on the focused element |
| `viewport` | `width?: number, height?: number, deviceScaleFactor?: number, mobile?: bool, reset?: bool` | applied metrics + live read-back of `window.innerWidth`/`innerHeight`/`devicePixelRatio` (`matched` false when the engine ignored the override); `reset:true` clears the override |
| `tabs` | `action: string, url?: string, index?: number` | tab list / mutation result (list, new, close, select) |
| `close` | - | `{ ok: true }`; reaps the owned browser process tree (or detaches from an attached external engine, leaving it running) |


Rules: tools are **dumb primitives** - no planner, no LLM, no judgement-retries in the server
(RESEARCH rule 7). Selector grammar is deliberately tiny; anything fancier goes through `evaluate`.

## 2. Backend: one owned engine, plus explicit attach (swap the engine, keep the contract)

| Tier | Backend | When | Notes |
|---|---|---|---|
| **Tier 0 (only launch)** | owned Obscura engine | now | `bin\obscura.exe` beside `gladius.exe`, provisioned by
  `scripts/fetch-engine.ps1` (pinned v0.2.4, sha256-verified). Launched as
  `obscura serve --host 127.0.0.1 --port <free> --allow-private-network
  --allow-file-access`, readiness via `GET /json/version`. Full capability
  record: `docs/OBSCURA-COMPAT.md`. |
| **Tier 1 (rejected)** | Lightpanda no-render | — | Rejected: no Windows target; AGPL-3.0 §13-only; text-only raster. |

No full-Chrome tier: there is no `--full-chrome` flag and none is planned — a fallback can never be a relic or someone else's.

Selection: `GLADIUS_CDP_URL` to **attach** to an existing endpoint instead of
launching (RESEARCH rule 5) — BUILT 2026-10-11: `ws://`/`wss://` connects
directly, `http://`/`https://` resolves via `GET <base>/json/version`; the
session/target flow after the connect is shared with the launch path —
otherwise the owned Obscura launch (the only launchable engine;
`GLADIUS_OBSCURA` overrides the engine path). Ownership: an attached engine
is NOT ours — `close` detaches the session
gladius opened and never kills, never sweeps. `GLADIUS_BACKEND_CAPS` declares `render`
(default) or `dom-only` (no rendering pipeline: `screenshot` refuses, never fakes).

## 3. MCP stdio frame shape

- Transport: **JSON-RPC 2.0 over stdio, newline-delimited** (one message per line) -
  **hand-rolled** in `src/mcp.rs` over `tokio` stdio and `serde_json`: no `rmcp` SDK, no
  external framing crate. STDOUT carries only protocol frames; all logs go to STDERR.
- Lifecycle: `initialize` -> `tools/list` (exactly the 16 tools above) -> `tools/call` ->
  stdio EOF / `close` -> shutdown.
- Each `tools/call` response is `{ content: [{ type: "text"|"image", ... }] }` per MCP content
  convention; `screenshot` returns an `image` part (base64 PNG), everything else `text`.
- Server answers `initialize` with **no browser running** (lazy start, §4) - cold MCP handshake
  must never pay browser-launch cost. The response carries `serverInfo.backend`
  (`obscura` or `external-cdp`) and `serverInfo.caps` (`render` or
  `dom-only`) so a client can see which engine and which caps it is talking to.

## 4. Browser lifecycle (lazy start, kill-tree on close)

```
tools/call (first, non-close) --► resolve owned engine binary --► spawn child
      (obscura serve on a free loopback port, both gate flags)
      --► CDP connect --► new target/page --► execute --► keep warm for session
tools/call close  --OR--  stdio EOF / SIGTERM --► CDP Browser.close --► kill-tree (job object) --► reap
```

Attach variant (same flow after the connect, §2): `GLADIUS_CDP_URL` set --► resolve ws
endpoint (http:// via `/json/version`) --► CDP connect (no spawn, engine not
owned) --► shared target/session flow --► execute --► keep warm for session.
`close`/EOF on an attached engine detaches the session gladius opened
(`Target.detachFromTarget`) and drops the connection; the external process is left
running by design - no `kill_tree`, no pid kill, nothing deleted.

- **Lazy start:** no child process exists until the first browser-touching tool call.
- **One session per agent connection:** each stdio client gets its own browser context/page;
  contexts never leak across connections (RESEARCH rule 6).
- **Kill-tree on close:** the child is spawned in a Windows **job object**; `close`/EOF kills the
  job (whole owned-engine tree), then reaps. No orphans.
- **Crash recovery:** a dead child surfaces as a tool error; the *next* call re-spawns lazily.
  No watchdog threads, no pre-warming.

## 5. RAM / latency budget + how it is measured

| Budget | Target | Rationale |
|---|---|---|
| MCP `initialize` (no browser) | **< 100 ms** cold | proves the server itself is free; browser cost is never on this path |
| First-action latency (incl. spawn+CDP) | **< 2 s** on dev Windows | owned-engine spawn + CDP handshake dominates; asserted, not hoped |
| Steady-state action p50 | **< 200 ms** (plus site/network) | CDP round-trip over `tokio-tungstenite`; `fastwebsockets` on standby if profiles implicate framing |
| Server RSS (own process) | **< 10 MB** | no GC runtime, no Node/Python next to the browser child |
| Browser child peak RSS | **< 250 MB** single page | owned engine, single page |

**Measurement (Windows-local, CI-runnable):**
- Cold start: `Measure-Command { Start-Process gladius.exe ... }` to first `initialize` response;
  timestamp deltas printed by the harness, asserted `< 100 ms`.
- Child RSS: sample the job-object child's `WorkingSet64` via `Get-Process` at 100 ms cadence
  during the gauntlet run; record peak, assert `< 250 MB`.
- Action latency: gauntlet (owning worker: `gauntlet/`) stamps `tools/call` send vs response
  per tool; p50/p95 reported per run.
- No cloud, no publish, headless/quiet - all measurement is local (`local only` constraint).

## 6. Non-goals (explicit)

Multi-browser support (Chromium-family only), vision/coordinate grounding, PDF generation,
fleet/cloud operation, in-server planning or LLM calls, and any dependency on Node.js,
Python, or Playwright at runtime. Those live in other systems (see RESEARCH.md); gladius is
the fast local minimum underneath them.
