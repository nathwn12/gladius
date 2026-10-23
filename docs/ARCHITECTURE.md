# Gladius ARCHITECTURE

> Gladius: headless, ultra-lightweight browser-use MCP server in Rust. Drives
> the owned pinned Chromium (`chrome-headless-shell`) **directly over CDP** - no
> Node.js, no Playwright in the runtime path. One static binary, MCP over stdio,
> lazy browser lifecycle. Research grounding: `docs/RESEARCH.md`.

## 1. Frozen tool contract (v2 - additive only; every v1 call is unchanged)

| name | args | returns |
|---|---|---|
| `navigate` | `url: string` | text after load event (timeout -> error) |
| `snapshot` | `mode?: "ax"\|"legacy"\|"text"` (default AX) | Real Accessibility tree: one line per actionable node with a stable ref (`@e1`, `@e2`, ...), role/name/value, plus page text (capped 8000 chars). Hidden/ignored subtrees (display:none, aria-hidden) are excluded from the lines; an old ref to such a node is a typed `stale_ref`, never a silent click. `mode:"legacy"` keeps the old DOM heuristic (stamps hidden nodes); `mode:"text"` = text-only output. Shadow-DOM nodes resolve in the snapshot, but acting on a shadow ref through the main-document resolver is not yet supported (typed `stale_ref`, G4). `frame` (CDP frameId) scopes the snapshot to that iframe |
| `screenshot` | `fullPage?: bool`, `path?: string`, `deterministic?: bool` (default false), `width?: number`, `height?: number` (only with `deterministic`) | PNG bytes (base64 `image` part) of viewport / full page; refuses under `dom-only`. `deterministic:true` fixes device metrics (current viewport or `width`/`height`, `deviceScaleFactor:1`, `mobile:false`) and restores them afterwards, sets `prefers-reduced-motion:reduce`, injects a temporary style freezing `animation`/`transition`/`caret-color`/scrollbars, waits on `document.fonts.ready` + every `img` `complete` + two animation frames, then captures `{format:png, fromSurface:true, captureBeyondViewport:fullPage}`. The reply adds `viewport`, `deviceScaleFactor`, `url`, `bytes` and a FNV-1a `checksum`, so two captures of the same page are provably identical |
| `assert_layout` | `ref?: string\|string[]`, `selector?: string\|string[]` (one of them), `checks?: object`, `screenshotOnFail?: bool`, `screenshotPath?: string` | geometry proved without pixels. Checks: `exists`, `visible`, `stableFrames` (N animation frames, identical bbox), `box {x,y,w,h,tolerance}` (VIEWPORT css px from `DOM.getBoxModel`), `hitTarget` (`elementFromPoint` at the centre is the target or a descendant), `computed` (subset of `getComputedStyle`), `noOverflow` (`scrollWidth/Height <= clientWidth/Height + tolerance`), `inViewport`. Returns `{pass, checks:[{target,check,pass,expected,actual,delta}], targets:[...]}`; `screenshotOnFail` adds one deterministic capture's path/bytes/checksum |
| `layout` | `ref?`, `selector?`, `includeOverlaps?: bool` (default true) | every match's viewport `rect`, page-coordinate `box` (`DOM.getBoxModel`), content quad count, scroll size (`scrollWidth/Height` vs `clientWidth/Height`), computed `display`/`visibility`/`opacity`, a z-order hint (`paintIndex`, `zIndex`, `position`, `stackingContext`) and every overlapping pair with its intersection rect and area |
| `visual` | `action: "save"\|"compare"\|"list"\|"delete"`, `name?: string`, `fullPage?: bool`, `threshold?: number` (per-channel 0..1, default 0.1), `maxDiffPixels?: number`, `maxDiffRatio?: number`, `maskRects?: {x,y,w,h}[]`, `update?: bool`, `antialias?: bool` (default true), `includeAA?: bool` (default false, the inverse), `minDiffCluster?: number` (default 2), `requireEnvironmentMatch?: bool` | deterministic baselines under `GLADIUS_VISUAL_DIR` (default `<cwd>/.gladius/visual`), names sanitised to `[A-Za-z0-9._-]`; each baseline writes a `<name>.meta.json` sidecar (browser, browserMajor, viewport, deviceScaleFactor, url, checksum, createdAt). `compare` diffs the current deterministic capture against the baseline **inside a scratch headless target created with `background:true`** (both PNGs ride in as `data:` URLs and are decoded onto a canvas, so no PNG crate is added) and returns `{pass, diffPixels, totalPixels, ratio, bbox, maxChannelDelta, ignoredPixels, ignoredClusters, baselinePath, diffPath, antialias, minDiffCluster, scratchBackground, environment {match, baseline, current}}`. Anti-aliasing is ignored by default with a mechanical rule: a candidate is suppressed only when its 3x3 luminance neighbourhood has ≥32 levels of contrast in **both** images, so a 1 px edge artefact passes while a 1 px change in a flat fill or a 2x2 block fails. A browser-major / viewport / DSF mismatch raises a `baseline_environment_mismatch` warning (not a failure unless `requireEnvironmentMatch:true`); `update:true` rewrites the PNG and the sidecar |
| `click` | `element: string` (CSS, or AX ref `@eN`), `frame?: string` (CDP frameId), `hoverFirst?: bool` (default false) | trusted pointer sequence at the exact float box centre: `mousePressed` + `mouseReleased` only, plus ONE `mouseMoved` positioning event when `hoverFirst:true` - no movement loop, no interpolation; `isTrusted:true` pointer/mouse/click events after scroll + hit-test; occluded targets are typed `not_actionable`, never a click into the overlay. `frame` scopes to that iframe (same-process CSS selectors; per-level owner hit-test up the ancestor chain; OOPIF without a mapped target is a typed error, never a main-frame click) |
| `type` | `element: string, text: string, submit?: bool, frame?: string` | focus + select-all in the (frame) realm, then trusted `Input.insertText` (`isTrusted:true` input event); the seed is replaced, not appended. File/date-like and non-editable targets are typed `not_editable`, never a silent no-op |
| `evaluate` | `expression: string, target?: string, frame?: string` | JSON-serialised JS result (`undefined` -> tool error, distinct from explicit `null`); `target` (CDP targetId) evaluates in that realm, `frame` (CDP frameId) in that iframe's isolated world |
| `wait_for` | `selector?, text?, ms?, networkIdle?: bool, idleMs?: number, expression?: string` (at least one field) | `{ ok: true }` when the condition holds or timeout elapses (max 15s); `expression` polls a JS predicate with backoff |
| `console` | `action?: "get"\|"drain"\|"clear", limit?: number` | buffered console messages + uncaught exceptions (ring buffer, cap 200) |
| `upload` | `element: string, paths: string[]` (minItems 1), `frame?: string` | `{ ok: true }` after setting files on an `<input type=file>`; frame-scoped upload resolves the node in the frame realm, never the main document |
| `fetch_text` | `url: string` | fast path: plain HTTP GET with NO browser; HTML stripped to text (readability-style, no tags) |
| `hover` | `element: string` (CSS, or AX ref `@eN`), `frame?: string` | `{ ok: true }` after the trusted `mouseMoved` (+ per-level owner hit-test when frame-scoped) |
| `select` | `element: string, value?: string, text?: string, frame?: string` | `{ ok: true }` after choosing a `<select>` option by value (preferred) or visible text (same JS selection in the frame realm when scoped) |
| `key` | `key: string` (Enter, Tab, Escape, ..., or `Ctrl+A`-style combos) | `{ ok: true }` after the key press on the focused element; combos hold modifiers (Alt=1/Ctrl=2/Meta=4/Shift=8) and dispatch `rawKeyDown` with no text when Ctrl/Meta/Alt is held |
| `viewport` | `width?: number, height?: number, deviceScaleFactor?: number, mobile?: bool, reset?: bool` (oneOf: `reset:true` alone; `reset:false` + `width`+`height`; no `reset` + `width`+`height` — exactly one branch) | applied metrics + live read-back of `window.innerWidth`/`innerHeight`/`devicePixelRatio` (`matched` false when the engine ignored the override); `reset:true` clears the override |
| `tabs` | `action: "list"|"new"|"select"|"close", url?: string, index?: number` | tab list / mutation result (list, new, close, select) |
| `close` | - | `{ ok: true }`; reaps the owned browser process tree (or detaches from an attached external engine, leaving it running) |
| `storage` | `action?: "get"\|"set"\|"clear"\|"save"\|"load", scope?: "cookies"\|"localStorage"\|"sessionStorage"\|"all", cookies?: object[], items?: object, path?: string, allSites?: bool, redact?: bool, overwrite?: bool, force?: bool, confirm?: bool` | cookies + Web Storage in one call, plus versioned file save/load (`{gladiusStorage:1,...}`; replies carry counts/bytes/origin only, never values). Cookie writes are read-back-verified (`cookie_rejected` otherwise); `load` enforces origin match unless `force:true`; `redact:true` masks values (`••••`) |
| `targets` | `action?: "list"|"select", target?: string` | every CDP target with its stable id + the active page's frame tree; `select` makes a page target current |
| `dialog` | `action?: "get"\|"policy", accept?: bool, promptText?: string` | last auto-handled dialog + live policy; `policy` sets the default handling |
| `history` | `action: "back"\|"forward"\|"reload"` | live url+title after the load settle; boundary steps are typed errors |
| `network` | `action?: "get"\|"drain"\|"clear", filter?: string, limit?: number` | captured requests/responses (method/url/status/type/timing, cap 500) |
| `cdp` | `method: string, params?: object, session?: string` | raw CDP `result`; CDP errors are tool errors |


Rules: tools are **dumb primitives** - no planner, no LLM, no judgement-retries in the server
(RESEARCH rule 7). Selector grammar is deliberately tiny; anything fancier goes through `evaluate`.

## 2. Backend: one owned engine, plus explicit attach (swap the engine, keep the contract)

| Tier | Backend | When | Notes |
|---|---|---|---|
| **Tier 0 (only launch)** | owned pinned Chromium (`chrome-headless-shell`) | now | versioned cache under `%LOCALAPPDATA%\gladius\chromium\`, provisioned by
  `scripts/fetch-chromium.ps1` (pinned 155.0.8059.84, self-hashed manifest).
  Headless-only (owner decision 2026-10-17, no headed path). Launched as
  `chrome-headless-shell --headless --remote-debugging-port <free>
  --user-data-dir <private> --no-first-run --no-default-browser-check
  --disable-backgrounding-occluded-windows
  --disable-renderer-backgrounding --disable-background-timer-throttling
  --remote-allow-origins http://127.0.0.1:<port> about:blank`, readiness via
  `GET /json/version`. The three `--disable-*` flags exist so a tab that is
  NOT the active page still renders, still screenshots and still runs its
  timers (asserted by the selftest). Full pin record: `docs/CHROMIUM.md`,
  launch contract: `docs/ENGINE.md`. |
| **Tier 1 (rejected)** | Lightpanda no-render | — | Rejected: no Windows target; AGPL-3.0 §13-only; text-only raster. |

No full-Chrome tier: there is no headed path and none is planned - the shell
artifact cannot go headed and that is a property we rely on; a fallback can
never be a relic or someone else's.

Selection: `GLADIUS_CDP_URL` to **attach** to an existing endpoint instead of
launching (RESEARCH rule 5) — BUILT 2026-10-11: `ws://`/`wss://` connects
directly, `http://`/`https://` resolves via `GET <base>/json/version`; the
session/target flow after the connect is shared with the launch path —
otherwise the owned Chromium launch (the only launchable engine;
`GLADIUS_CHROMIUM` overrides the browser path,
`GLADIUS_CHROMIUM_CACHE` overrides the cache root). A headed request
(`GLADIUS_HEADED`, `--headed`) fails closed with an explicit unsupported
error. Ownership: an attached engine
is NOT ours — `close` detaches the session
gladius opened and never kills, never sweeps. `GLADIUS_BACKEND_CAPS` declares `render`
(default) or `dom-only` (no rendering pipeline: `screenshot` refuses, never fakes).

## 3. MCP stdio frame shape

- Transport: **JSON-RPC 2.0 over stdio, newline-delimited** (one message per line) -
  **hand-rolled** in `src/mcp.rs` over `tokio` stdio and `serde_json`: no `rmcp` SDK, no
  external framing crate. STDOUT carries only protocol frames; all logs go to STDERR.
- Lifecycle: `initialize` -> `tools/list` (exactly the 25 tools above) -> `tools/call` ->
  stdio EOF / `close` -> shutdown.
- Each `tools/call` response is `{ content: [{ type: "text"|"image", ... }] }` per MCP content
  convention; `screenshot` returns an `image` part (base64 PNG), everything else `text`.
- Server answers `initialize` with **no browser running** (lazy start, §4) - cold MCP handshake
  must never pay browser-launch cost. The response carries `serverInfo.backend`
  (`chromium` or `external-cdp`) and `serverInfo.caps` (`render` or
  `dom-only`) so a client can see which engine and which caps it is talking to.

## 4. Browser lifecycle (lazy start, kill-tree on close)

```
tools/call (first, non-close) --► resolve owned browser binary --► create fresh
      private profile --► spawn child
      (chrome-headless-shell on a free loopback port, private --user-data-dir)
      --► CDP connect --► new target/page --► execute --► keep warm for session
tools/call close  --OR--  stdio EOF --► terminate direct child (non-waiting) --► deadline-wrapped reap (2s) --► bounded kill-tree fallback (5s, job cover when present) --► reap --► bounded profile sweep (10s retries)
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

## 6. Trusted input, frame scoping, storage (G4/G5)

- **Trusted input (no synthetic-event spoofing):** `click` dispatches a real
  CDP pointer sequence (`mouseMoved`/`mousePressed`/`mouseReleased`) at
  main-frame viewport px after scroll-into-view, an
  `elementFromPoint` hit-test, and a 120 ms navigation-settle watch;
  `type` focuses + select-alls in the target realm, then trusted
  `Input.insertText` (real `input` events, `isTrusted:true`). An occluding
  overlay, a hidden/disabled target, or a non-editable input is a typed
  `not_actionable`/`occluded`/`not_editable` - the pointer never moves to
  the wrong surface. `key` presses named keys; `+` combos
  (`Ctrl+A`, `Shift+Tab`, ...) hold modifiers (Alt=1/Ctrl=2/Meta=4/Shift=8)
  and dispatch `rawKeyDown` with no text while Ctrl/Meta/Alt is held.
- **Frame scoping:** `click`/`type`/`hover`/`select`/`upload` (and
  `snapshot`/`evaluate`) take `frame=` (CDP frameId from `targets`). Leaf
  geometry resolves in the frame realm; coordinates map to main-frame
  viewport px through the owner rect chain (owner `left/top` + padding +
  border per ancestor, JS rect chain - never `getContentQuads`). Every
  ancestor level is hit-tested in its own realm against the true stamped
  owners, so a mid-level overlay is `occluded` too. `@eN` refs are
  main-frame only (ref+`frame` is typed `invalid_args`); cross-origin frames
  work through isolated worlds on the page session. No OOPIF landing is
  claimed: this engine exposes no separate iframe target, so unmapped
  frames are typed errors, never silent main-frame actions.
- **Storage:** one `storage` tool for cookies + `localStorage`/
  `sessionStorage` (page-scoped by default, `allSites:true` widens cookie
  read/clear), plus versioned file save/load (`{gladiusStorage:1,...}`).
  Cookie writes are read-back-verified (`cookie_rejected` naming the cookie
  otherwise); `load` enforces an origin match unless `force:true`;
  `save`/`load` replies carry counts/bytes/origin only, never values;
  `redact:true` masks get values (`••••`). `sessionStorage` is tab-scoped
  (clears with the tab); `localStorage` persists.
- **Every tool** carries a versioned `outputSchema` + `annotations`
  (`readOnlyHint`/`destructiveHint`/`idempotentHint`/`openWorldHint`), and
  every error carries a typed `structuredContent` code/reason - see §1 and
  the envelope contract.

## 6a. Determinism, focus and the no-OS-input guarantee

The hard constraint behind every one of these tools: **zero OS cursor
movement, zero OS keyboard injection, zero focus stealing, zero visible
window or terminal for the owned engine.** Everything rides CDP against an
owned `chrome-headless-shell`, which cannot go headed at all. The one
qualification is the explicit attach seam: an engine the user started
(`GLADIUS_CDP_URL`) is theirs and not headless by construction, so
`visual compare` opens and closes one scratch tab there for the in-engine
diff. That is a tab in an engine gladius does not own; gladius itself never
raises, focuses or moves a window on any path.

- **Exact input, no animation.** `click` resolves the box centre as a float
  (sub-pixel) and dispatches `mousePressed` + `mouseReleased` at that point -
  no `mouseMoved` at all unless the caller passes `hoverFirst:true`, which
  adds ONE positioning event at the same point. No interpolation, no stepped
  path, no smoothing loop (`Browser::mouse_click`); `hover` stays an
  explicit, separately requested move. `type` is `Input.insertText`; `key`
  is `dispatchKeyEvent`. No OS input API is referenced anywhere.
- **Focus without foreground.** `attach_to` enables
  `Emulation.setFocusEmulationEnabled` on every session, so `:focus`,
  `document.hasFocus()` and focus-gated handlers behave as if the tab owned
  the focus while the operator's real focus is untouched. No
  `Page.bringToFront` exists on any path: activating a tab requires an
  explicit `targets select` / `tabs select`.
- **Background tabs keep working.** The launch adds
  `--disable-backgrounding-occluded-windows`,
  `--disable-renderer-backgrounding` and
  `--disable-background-timer-throttling` (`chromium_argv`). The selftest
  proves it: with the fixture in a background tab, a capture taken on that
  tab's own session is byte-identical to the foreground capture and its
  `setInterval` keeps firing (25 ticks in 1.2 s instead of ~0-1).
- **Deterministic pixels.** `screenshot {deterministic:true}` pins metrics,
  motion, caret and scrollbars and waits for fonts/images/frames before
  capturing, then restores every override. Two captures of the same page
  return the same checksum - the assertion the selftest makes.
- **Geometry without pixels.** `assert_layout` and `layout` answer "where is
  it / is it really there" from `DOM.getBoxModel` + `Runtime.evaluate`, so a
  layout regression is caught and reported as numbers (`{check, pass,
  expected, actual, delta}`) instead of requiring an image diff.
- **Visual regression without a PNG crate.** `visual compare` opens a
  throwaway target, decodes baseline and current from `data:` URLs onto a
  canvas, and diffs `ImageData` there. No dependency was added for PNG
  decoding. The target is created with `background:true` so it never becomes
  the active tab, and there is no foreground fallback: an engine that rejects
  the parameter produces a typed `scratch_background_unsupported` error and no
  target is created at all (asserted against a recording peer that answers the
  rejection a real engine sends). It is closed on every path. On an external
  attach that scratch target is a real extra tab for the duration of the
  compare (see the qualification above), but it never activates.
- **Anti-aliasing tolerance is a rule, not a bigger threshold.** A candidate
  pixel (largest channel delta over `threshold`, outside `maskRects`) is
  grouped into a 4-connected component; the component is ignored only when
  `antialias` is on, every pixel in it has a 3x3 luminance neighbourhood of at
  least 32 levels in **both** images, and the component is no larger than
  `minDiffCluster` (default 2). So a 1 px edge artefact is ignored, a 1 px
  change in a flat fill is not (no baseline-side contrast), and a 2x2 block is
  not (4 > 2). `ignoredPixels`/`ignoredClusters` report what was suppressed and
  the diff PNG paints those pixels amber instead of red.
- **Baselines carry their environment.** `save` writes `<name>.meta.json`
  (browser, browserMajor, viewport, deviceScaleFactor, url, checksum,
  createdAt from a dependency-free civil-date conversion). `compare` returns
  `environment {match, baseline, current}` and a `baseline_environment_mismatch`
  warning when browser major, viewport or DSF differ; it does not fail by
  default (the pixels decide), and `requireEnvironmentMatch:true` turns the
  mismatch into a failure. A sidecar that is missing, malformed or unreadable
  is reported as `environment.known:false` with a
  `baseline_environment_unknown` warning - and is a typed
  `baseline_environment_unknown` failure under `requireEnvironmentMatch:true`,
  because a strict gate must never pass without evidence. This is what stops
  browser-update rot from being silently absorbed.
- **Captures are serialized.** `screenshot`, `assert_layout` (its
  `screenshotOnFail` capture) and `visual` hold the execution lock, and the
  capture module holds a second connection-level lock across the whole
  mutation-and-restore window. Three concurrent deterministic captures with
  different forced geometry each report their own viewport and leave the
  page's own override intact; without that lock they all report the last
  override written (measured, and asserted by the selftest).
- **Cancellation cannot skip cleanup.** Those same three tools also hold a
  per-call *section gate* for the whole call. `notifications/cancelled` (and a
  duplicate-id supersede) marks the call cancelled and then aborts from a
  detached waiter that first takes the section gate — so the future can never
  be dropped mid-restore, and the execution lock is released only after
  metrics, emulated media, freeze stylesheet and scratch target are back to
  their prior state. The response is still suppressed. The gate is acquired
  BEFORE the decisive liveness recheck, so a cancel that lands in the entry
  window stops the call at the gate instead of admitting it into the section;
  the abort waiter holds the gate across `handle.abort()` for the same reason.
  A cancel that arrives before the section starts (pre-start, or waiting for
  the lock) aborts immediately, exactly as before. `GLADIUS_TEST_CRITICAL_DELAY_MS` is the
  test-only hook that widens the window so the cancellation tests can cancel
  deterministically; it is never set in production.

## 7. Non-goals (explicit)

Multi-browser support (Chromium-family only), vision/coordinate grounding, PDF generation,
fleet/cloud operation, in-server planning or LLM calls, and any dependency on Node.js,
Python, or Playwright at runtime. Those live in other systems (see RESEARCH.md); gladius is
the fast local minimum underneath them.
