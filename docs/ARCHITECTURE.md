# Gladius ARCHITECTURE

> Gladius: headless, ultra-lightweight browser-use MCP server in Rust. Drives headless Chrome
> **directly over CDP** - no Node.js, no Playwright in the runtime path. One static binary,
> MCP over stdio, lazy browser lifecycle. Research grounding: `docs/RESEARCH.md`.

## 1. Frozen tool contract (v1 - the surface never moves for backend swaps)

| name | args | returns |
|---|---|---|
| `navigate` | `url: string` | `{ ok: true, url }` after load event (timeout -> error) |
| `snapshot` | `compact?: bool` (default true) | accessibility/DOM-tree text of the current page |
| `screenshot` | `full_page?: bool` (default false) | PNG bytes (base64 in JSON-RPC) of viewport / full page |
| `click` | `selector: string` (css, xpath:, or text:) | `{ ok: true }` after click dispatches |
| `type` | `selector: string, text: string, submit?: bool` | `{ ok: true }` after keystrokes (+Enter if submit) |
| `evaluate` | `expression: string` | JSON-serialised JS result |
| `wait_for` | `text?: string, timeout_ms?: number` | `{ ok: true }` when text appears or timeout elapses |
| `fetch_text` | `url?: string` (default: current page) | rendered text content (readability-style, no tags) |
| `close` | - | `{ ok: true }`; reaps the browser process tree |

Rules: tools are **dumb primitives** - no planner, no LLM, no judgement-retries in the server
(RESEARCH rule 7). Selector grammar is deliberately tiny; anything fancier goes through `evaluate`.

## 2. Tiered backend plan (swap the engine, keep the contract)

| Tier | Backend | When | Notes |
|---|---|---|---|
| **Tier 0 (default)** | `chrome-headless-shell` | now | Official no-UI Chromium (https://googlechromelabs.github.io/chrome-for-testing/).
  Resolved at runtime: `%LOCALAPPDATA%\ms-playwright\chromium_headless_shell-*\chrome-headless-shell-win64\chrome-headless-shell.exe`;
  fallback: full Chrome binary. Full JS/DOM fidelity, no compositor/GPU-process tax. |
| **Tier 1 (future)** | Lightpanda no-render | when Windows-viable | `lightpanda serve --protocol cdp` behind the *same* 9 tools
  (https://github.com/lightpanda-io/browser). Renderless DOM/fetch at a fraction of the RAM.
  Blocked today: no native Windows binary. |
| **Tier 2 (opt-in)** | Full Chrome headed/headless | pixel-perfect only | Flag-gated (`--full-chrome`) for screenshots/canvas/WebGL
  where Tier 0 fidelity is insufficient. Never the default. |

Selection: `--backend {shell, lightpanda, full-chrome}` (default `shell`) + `CDP_URL` env to
**attach** to an existing endpoint instead of launching (RESEARCH rule 5).

## 3. MCP stdio frame shape

- Transport: **JSON-RPC 2.0 over stdio, newline-delimited** (one message per line), via the
  official Rust SDK `rmcp` (https://github.com/modelcontextprotocol/rust-sdk) - no hand-rolled framing.
- Lifecycle: `initialize` -> `tools/list` (exactly the 9 tools above) -> `tools/call` ->
  stdio EOF / `close` -> shutdown.
- Each `tools/call` response is `{ content: [{ type: "text"|"image", ... }] }` per MCP content
  convention; `screenshot` returns an `image` part (base64 PNG), everything else `text`.
- Server answers `initialize` with **no browser running** (lazy start, §4) - cold MCP handshake
  must never pay browser-launch cost.

## 4. Browser lifecycle (lazy start, kill-tree on close)

```
tools/call (first, non-close) --► resolve backend binary --► spawn child
      (chrome-headless-shell --headless --remote-debugging-port=0 --about:blank)
      --► CDP connect --► new target/page --► execute --► keep warm for session
tools/call close  --OR--  stdio EOF / SIGTERM --► CDP Browser.close --► kill-tree (job object) --► reap
```

- **Lazy start:** no child process exists until the first browser-touching tool call.
- **One session per agent connection:** each stdio client gets its own browser context/page;
  contexts never leak across connections (RESEARCH rule 6).
- **Kill-tree on close:** the child is spawned in a Windows **job object**; `close`/EOF kills the
  job (whole `chrome-headless-shell` tree incl. renderers), then reaps. No orphans, no lingering
  profiles (the `@playwright/mcp` failure mode we refuse to copy).
- **Crash recovery:** a dead child surfaces as a tool error; the *next* call re-spawns lazily.
  No watchdog threads, no pre-warming.

## 5. RAM / latency budget + how it is measured

| Budget | Target | Rationale |
|---|---|---|
| MCP `initialize` (no browser) | **< 100 ms** cold | proves the server itself is free; browser cost is never on this path |
| First-action latency (incl. spawn+CDP) | **< 2 s** on dev Windows | `chrome-headless-shell` spawn + CDP handshake dominates; asserted, not hoped |
| Steady-state action p50 | **< 200 ms** (plus site/network) | CDP round-trip over `tokio-tungstenite`; `fastwebsockets` on standby if profiles implicate framing |
| Server RSS (own process) | **< 10 MB** | no GC runtime, no Node/Python next to the browser child |
| Browser child peak RSS | **< 250 MB** single page | `chrome-headless-shell` without compositor/GPU; full-Chrome tier exempt |

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
