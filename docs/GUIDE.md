# gladius GUIDE

The manual. `README.md` has the install and the capability table; this file has
the model, every option, every tool and every failure worth knowing.

- [1. How it works](#1-how-it-works)
- [2. Configuration](#2-configuration)
- [3. Tools](#3-tools)
- [4. Troubleshooting](#4-troubleshooting)
- [5. Limitations](#5-limitations)

---

## 1. How it works

### Two processes, one of which you install

`gladius-mcp` is a dependency-free Node launcher. `gladius` is the Rust MCP
server it runs. The launcher owns provisioning and paths; the server owns the
browser. The launcher passes `GLADIUS_EXPECTED_VERSION` to the child, and the
server refuses to serve if its own version differs, so a half-updated cache is
an error rather than a mixed-version pair.

### Provisioning (launcher, first real run only)

On the first run that is not a diagnostics flag, the launcher provisions two
artifacts, each sha256-verified before and after extraction. Any mismatch
refuses and writes nothing.

| Artifact | Source | Cache location |
|---|---|---|
| Server runtime bundle | the tagged GitHub Release for this version | `%LOCALAPPDATA%\gladius\engine\<serverVersion>-<bundleSha8>\` |
| `chrome-headless-shell` `155.0.8059.84` | Google's Chrome-for-Testing endpoint (~118 MB) | `%LOCALAPPDATA%\gladius\chromium\<version>-<zipSha8>\` |

Both are reused on later runs and re-verified each time. `--doctor` and
`--print-manifest` only print the resolved paths and exit 0 - they perform no
network access and provision nothing.

The browser is never redistributed: our own manifest is the integrity root,
because the Chrome-for-Testing endpoints publish no hashes. Pin record:
[`CHROMIUM.md`](CHROMIUM.md).

### Launch (server, first browser-touching tool call)

The server starts no process until a tool actually needs the browser, so the
MCP `initialize` handshake never pays launch cost.

1. Resolve the engine: `GLADIUS_CDP_URL` to **attach** to an endpoint you
   started, otherwise the owned pinned `chrome-headless-shell` (an explicit
   `GLADIUS_CHROMIUM` path wins over the cache).
2. Create one fresh private profile directory, then spawn
   `chrome-headless-shell --headless --remote-debugging-port=<free>
   --user-data-dir=<private> --no-first-run --no-default-browser-check
   --disable-backgrounding-occluded-windows --disable-renderer-backgrounding
   --disable-background-timer-throttling --remote-allow-origins=<port> about:blank`,
   with `CREATE_NO_WINDOW`, and poll `GET /json/version` for
   `webSocketDebuggerUrl` (30 s deadline, `GLADIUS_STARTUP_TIMEOUT_SECS`
   overrides it).
3. Connect over CDP, open a page, run the call, and keep the browser warm for
   the rest of the session.

The three `--disable-*` flags are the background-correctness contract: a tab
that is not the active page still renders, still screenshots and still runs its
timers.

`close` (or stdio EOF) terminates the child and reaps the whole tree through a
Windows job object, then sweeps the private profile. On an **attached** engine
(`GLADIUS_CDP_URL`) `close` only detaches the session it opened - your process
is left running and nothing of yours is deleted.

### Transport

JSON-RPC 2.0 over stdio, newline-delimited, hand-rolled in `src/mcp.rs`. STDOUT
carries only protocol frames; every log line goes to STDERR. Lifecycle:
`initialize` -> `tools/list` -> `tools/call` -> `close` or EOF.

Each `tools/call` result is `{ content: [...] }`; `screenshot` adds an `image`
part (base64 PNG) and everything else is `text`. `initialize` reports
`serverInfo.backend` (`chromium` or `external-cdp`) and `serverInfo.caps`
(`render` or `dom-only`), so a client can see which engine it is talking to.

---

## 2. Configuration

### Launcher flags

| Flag | Default | Effect |
|---|---|---|
| *(none)* | - | Provision the pinned runtime plus browser on first run, then serve MCP over stdio. |
| `--doctor` | off | Print the pinned manifest and resolved cache paths, exit 0, no network. |
| `--print-manifest` | off | Alias of `--doctor`. |
| `--version` | off | Print the launcher version, exit 0, no network. |
| `--help`, `-h` | off | Print usage, exit 0. |

Any other flag prints usage and exits 2.

### Server subcommands

`gladius [mcp|selftest|--version]`. `mcp` is the default (no argument). The
launcher always invokes `mcp`; the other two are for local diagnosis. A headed
request (`--headed`, or `GLADIUS_HEADED` truthy) fails closed with an explicit
unsupported error and a pointer to `GLADIUS_CDP_URL`.

### Environment variables

| Variable | Default | Effect |
|---|---|---|
| `GLADIUS_CHROMIUM` | unset | Absolute path to your own `chrome-headless-shell` binary. When set and non-blank it wins, and browser provisioning is skipped entirely. Must be a file. |
| `GLADIUS_ENGINE_CACHE` | `%LOCALAPPDATA%` | Root for the runtime cache; the versioned `<serverVersion>-<bundleSha8>` folder lives under it. |
| `GLADIUS_CHROMIUM_CACHE` | `%LOCALAPPDATA%` | Root for the browser cache; the versioned `<version>-<zipSha8>` folder lives under it. |
| `GLADIUS_ENGINE_BUNDLE` | unset | Path to a local runtime zip to verify and extract instead of downloading. Pre-publish diagnostics only; size and sha256 are still checked. |
| `GLADIUS_MCP_ALLOW_NON_WINDOWS` | unset | Set to `1` to bypass the Windows-only gate for diagnostics. The server runtime still runs on Windows x64 only. |
| `GLADIUS_EXPECTED_VERSION` | set by launcher | The version the launcher expects. A mismatch makes the server refuse to serve rather than run a mixed pair. Do not set it by hand. |
| `GLADIUS_CDP_URL` | unset | Attach to an engine you already run instead of launching one. `ws://`/`wss://` connects directly; `http://`/`https://` resolves via `GET <base>/json/version`. That engine is yours: `close` detaches and never kills. |
| `GLADIUS_HEADED` | unset | Truthy refuses the launch - there is no headed code path. Use `GLADIUS_CDP_URL` for a visible browser. |
| `GLADIUS_BACKEND_CAPS` | `render` | `render` or `dom-only`. `dom-only` declares no rendering pipeline: `screenshot` and `visual` refuse with `capability_gated` / `dom_only` instead of returning a fake image. Any other value is an error. |
| `GLADIUS_VISUAL_DIR` | `<cwd>/.gladius/visual` | Root for `visual` baselines and their `.meta.json` sidecars. |
| `GLADIUS_STARTUP_TIMEOUT_SECS` | `30` | Deadline for the owned-engine readiness poll, in seconds. |

Test-only hooks, never set in production: `GLADIUS_TEST_CRITICAL_DELAY_MS`,
`GLADIUS_TEST_ENTRY_MS`, `GLADIUS_TEST_ABORT_HOLD_MS`,
`GLADIUS_TEST_ABORT_SKIP`, `GLADIUS_TEST_DROP_MEDIA_CLEAR`,
`GLADIUS_TEST_SCRATCH_REJECT_BACKGROUND`. They widen cancellation windows and
force failure paths so the suite can hit them deterministically.

### Installing a version

Use `@latest`. A pinned version is only installable while its GitHub release
still exists, and the one-release policy deletes older releases, so pinned
versions are not a supported long-term install. Global installs are refused by
the package's `preinstall` script.

### There is no fully offline fresh install

The first run always downloads the pinned runtime and browser. `GLADIUS_CHROMIUM`
and `GLADIUS_ENGINE_BUNDLE` can supply bytes you already hold, but treat them as
overrides, not as a no-network setup.

---

## 3. Tools

25 tools, all dumb primitives: no planner, no LLM, no judgement-retries in the
server. The selector grammar is deliberately tiny; anything fancier goes through
`evaluate`. Every example below is the `arguments` object of a `tools/call`.

### Navigate and manage the session

#### `navigate`
- **Args:** `url: string`
- **Returns:** page `url` and `title` once the load event fires. A load that never settles is a typed `timeout` error.
- **Example:** `{"url": "https://example.com"}`

#### `history`
- **Args:** `action: "back" | "forward" | "reload"`
- **Returns:** live `url` and `title` after the load settles. Stepping past either end is a typed error, not a silent no-op.
- **Example:** `{"action": "back"}`

#### `tabs`
- **Args:** `action: "list" | "new" | "select" | "close"`, `url?: string`, `index?: number`
- **Returns:** the tab list, or the mutation result for `new`/`select`/`close`.
- **Example:** `{"action": "new", "url": "https://example.com"}`

#### `targets`
- **Args:** `action?: "list" | "select"`, `target?: string`
- **Returns:** every CDP target with its stable id, plus the active page's frame tree. `select` makes a page target current.
- **Example:** `{"action": "list"}`

#### `viewport`
- **Args:** exactly one of: `reset: true`; `reset: false` + `width` + `height`; or `width` + `height` (plus optional `deviceScaleFactor?: number`, `mobile?: bool`)
- **Returns:** applied metrics plus a live read-back of `window.innerWidth`/`innerHeight`/`devicePixelRatio`; `matched: false` when the engine ignored the override. `reset: true` clears it.
- **Example:** `{"width": 1280, "height": 720, "deviceScaleFactor": 1}`

#### `dialog`
- **Args:** `action?: "get" | "policy"`, `accept?: bool`, `promptText?: string`
- **Returns:** the last auto-handled dialog plus the live policy; `policy` sets the default handling.
- **Example:** `{"action": "policy", "accept": true}`

#### `close`
- **Args:** none
- **Returns:** `{ "ok": true }`. Reaps the owned browser tree; on an attached engine it detaches and leaves your process running.
- **Example:** `{}`

### Read the page

#### `snapshot`
- **Args:** `mode?: "ax" | "legacy" | "text"` (default `ax`), `frame?: string` (CDP frameId)
- **Returns:** the real Accessibility tree, one line per actionable node with a stable ref (`@e1`, `@e2`, ...), role/name/value, plus page text capped at 8000 chars. Hidden and `aria-hidden` subtrees are excluded; `legacy` keeps the old DOM heuristic, `text` is text-only. `frame` scopes it to that iframe.
- **Example:** `{}`

#### `evaluate`
- **Args:** `expression: string`, `target?: string` (CDP targetId), `frame?: string` (CDP frameId)
- **Returns:** the JSON-serialised result. `undefined` is a tool error, which stays distinct from an explicit `null`.
- **Example:** `{"expression": "document.title"}`

#### `console`
- **Args:** `action?: "get" | "drain" | "clear"`, `limit?: number`
- **Returns:** buffered console messages and uncaught exceptions from a ring buffer capped at 200 entries.
- **Example:** `{"action": "get", "limit": 50}`

#### `network`
- **Args:** `action?: "get" | "drain" | "clear"`, `filter?: string`, `limit?: number`
- **Returns:** captured requests and responses (method, url, status, type, timing), capped at 500 entries.
- **Example:** `{"action": "get", "filter": "api"}`

#### `fetch_text`
- **Args:** `url: string`
- **Returns:** the page text with tags stripped and scripts dropped, from a plain HTTP GET with **no browser involved**.
- **Example:** `{"url": "https://example.com"}`

### Act on the page

#### `click`
- **Args:** `element: string` (CSS or AX ref `@eN`), `frame?: string`, `hoverFirst?: bool` (default `false`)
- **Returns:** `{ "ok": true }` after a trusted `mousePressed` + `mouseReleased` pair at the exact float box centre (`isTrusted: true` events, after scroll and hit-test). `hoverFirst: true` adds one `mouseMoved`. An occluded target is a typed `not_actionable`, never a click into the overlay.
- **Example:** `{"element": "@e3"}`

#### `type`
- **Args:** `element: string`, `text: string`, `submit?: bool`, `frame?: string`
- **Returns:** `{ "ok": true }` after focus, select-all and a trusted `Input.insertText`; the seed is replaced, not appended. File/date-like and non-editable targets are a typed `not_editable`.
- **Example:** `{"element": "#q", "text": "gladius", "submit": true}`

#### `hover`
- **Args:** `element: string`, `frame?: string`
- **Returns:** `{ "ok": true }` after one trusted `mouseMoved` plus a per-level owner hit-test when frame-scoped.
- **Example:** `{"element": "#menu"}`

#### `select`
- **Args:** `element: string`, `value?: string`, `text?: string`, `frame?: string`
- **Returns:** `{ "ok": true }` after choosing a `<select>` option by value (preferred) or visible text. No match is a typed `option_not_found`.
- **Example:** `{"element": "#sort", "value": "price"}`

#### `key`
- **Args:** `key: string` (`Enter`, `Tab`, `Escape`, ..., or a `Ctrl+A`-style combo)
- **Returns:** `{ "ok": true }` after the press on the focused element. Combos hold a modifier mask (Alt=1/Ctrl=2/Meta=4/Shift=8) and dispatch `rawKeyDown` with no text when Ctrl/Meta/Alt is held.
- **Example:** `{"key": "Ctrl+A"}`

#### `upload`
- **Args:** `element: string`, `paths: string[]` (at least one), `frame?: string`
- **Returns:** `{ "ok": true }` after setting files on an `<input type=file>`. A frame-scoped upload resolves the node in the frame realm, never the main document. A missing path is a typed `file_not_found`.
- **Example:** `{"element": "#avatar", "paths": ["C:\\\\tmp\\\\a.png"]}`

### Wait and prove layout

#### `wait_for`
- **Args:** at least one of `selector?: string`, `text?: string`, `ms?: number`, `networkIdle?: bool`, `idleMs?: number`, `expression?: string`
- **Returns:** `{ "ok": true }` when the condition holds, or when the timeout elapses (maximum 15 s). `expression` polls a JS predicate with backoff.
- **Example:** `{"selector": "#results", "ms": 5000}`

#### `assert_layout`
- **Args:** `ref?: string | string[]` or `selector?: string | string[]` (exactly one of them), `checks?: object`, `screenshotOnFail?: bool`, `screenshotPath?: string`
- **Returns:** geometry proved without pixels: `{ pass, checks: [{ target, check, pass, expected, actual, delta }], targets: [...] }`. Checks are `exists`, `visible`, `stableFrames`, `box {x,y,w,h,tolerance}`, `hitTarget`, `computed`, `noOverflow`, `inViewport`. `screenshotOnFail` adds one deterministic capture's path, bytes and checksum.
- **Example:** `{"selector": "#hero", "checks": {"visible": true, "inViewport": true}}`

#### `layout`
- **Args:** `ref?` or `selector?`, `includeOverlaps?: bool` (default `true`)
- **Returns:** every match's viewport `rect`, page-coordinate `box`, content quad count, scroll size, computed `display`/`visibility`/`opacity`, a z-order hint (`paintIndex`, `zIndex`, `position`, `stackingContext`), and every overlapping pair with its intersection rect and area.
- **Example:** `{"selector": ".card", "includeOverlaps": true}`

### Pixels

#### `screenshot`
- **Args:** `fullPage?: bool`, `path?: string` (absolute only), `deterministic?: bool` (default `false`), `width?: number`, `height?: number` (only with `deterministic`)
- **Returns:** an `image` content part with base64 PNG, plus `viewport`, `deviceScaleFactor`, `url`, `bytes` and a FNV-1a `checksum`. `deterministic: true` fixes metrics, forces `prefers-reduced-motion`, freezes animation/transition/caret/scrollbars, waits for fonts and images, and restores everything afterwards - two captures of the same page are provably identical. Refuses with `capability_gated` under `dom-only`.
- **Example:** `{"deterministic": true, "fullPage": true}`

#### `visual`
- **Args:** `action: "save" | "compare" | "list" | "delete"`, `name?: string`, `fullPage?: bool`, `threshold?: number` (per-channel, default `0.1`), `maxDiffPixels?: number`, `maxDiffRatio?: number`, `maskRects?: {x,y,w,h}[]`, `update?: bool`, `antialias?: bool` (default `true`), `includeAA?: bool` (default `false`), `minDiffCluster?: number` (default `2`), `requireEnvironmentMatch?: bool`
- **Returns:** `save` writes `<name>.png` plus a `<name>.meta.json` sidecar (browser, browserMajor, viewport, deviceScaleFactor, url, checksum, createdAt). `compare` returns `{ pass, diffPixels, totalPixels, ratio, bbox, maxChannelDelta, ignoredPixels, ignoredClusters, baselinePath, diffPath, scratchBackground, environment { match, baseline, current } }`. A browser-major, viewport or deviceScaleFactor mismatch is a `baseline_environment_mismatch` warning unless `requireEnvironmentMatch: true`. Anti-aliasing is ignored mechanically: a candidate is suppressed only when its 3x3 luminance neighbourhood has at least 32 levels of contrast in **both** images.
- **Example:** `{"action": "compare", "name": "home", "maxDiffRatio": 0.001}`

### State

#### `storage`
- **Args:** `action?: "get" | "set" | "clear" | "save" | "load"`, `scope?: "cookies" | "localStorage" | "sessionStorage" | "all"`, `cookies?: object[]`, `items?: object`, `path?: string`, `allSites?: bool`, `redact?: bool`, `overwrite?: bool`, `force?: bool`, `confirm?: bool`
- **Returns:** cookies and Web Storage in one call, plus versioned file save/load (`{gladiusStorage: 1, ...}`). Replies carry counts, bytes and origin - never values - unless you ask for them; `redact: true` masks values as `••••`. Cookie writes are read-back-verified (`cookie_rejected` otherwise), and `load` enforces an origin match unless `force: true` (`origin_mismatch`).
- **Example:** `{"action": "get", "scope": "all"}`

### Escape hatch

#### `cdp`
- **Args:** `method: string`, `params?: object`, `session?: string`
- **Returns:** the raw CDP `result`. CDP errors are tool errors, never swallowed.
- **Example:** `{"method": "Page.getLayoutMetrics"}`

---

## 4. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `global install refused` | The package's `preinstall` refuses `npm i -g` on purpose. | Install the way the README's Install section shows: through `npx`, never globally. |
| `platform: windows-x86_64 only` at startup | You are not on Windows x64. | Nothing to install around it - the runtime is Windows x64 only. `GLADIUS_MCP_ALLOW_NON_WINDOWS=1` bypasses the gate for diagnostics only. |
| `version mismatch: launcher X, engine Y` | A stale runtime cache paired with a newer launcher. | Run `--doctor` to see the skew, then reinstall at `@latest` the way the README's Install section shows. |
| Provisioning refuses and writes nothing | A download hash or size did not match the pinned manifest. | Delete the versioned cache folder named by `--doctor` and retry; if it repeats, the network path is rewriting bytes. |
| First tool call takes seconds | The browser is provisioning or spawning. | Expected once. `--doctor` shows the resolved paths without downloading. |
| `stale_ref` | The `@eN` ref came from an older `snapshot`, or points into a hidden subtree. | Re-run `snapshot` and use the new ref. Acting on a shadow-DOM ref is not supported yet. |
| `not_actionable` | The target is covered by another element or never became clickable. | Close the overlay, scroll it into view, or assert with `assert_layout` first. |
| `not_editable` | `type` hit a file/date-like or non-editable node. | Use `upload` for file inputs; use `evaluate` for exotic editors. |
| `capability_gated` / `dom_only` on `screenshot` or `visual` | `GLADIUS_BACKEND_CAPS=dom-only` is set. | Unset it to get the `render` pipeline back. |
| `scratch_background_unsupported` | The attached engine refuses a background target, so `visual compare` will not open a foreground tab. | Compare against the owned browser instead of your attached engine. |
| `headed mode is not supported` | `--headed` or `GLADIUS_HEADED` was set. | Start your own browser and attach with `GLADIUS_CDP_URL`; gladius never owns a visible window. |
| `timeout` from `navigate` or `wait_for` | The load or the predicate never settled (15 s cap on `wait_for`). | Poll with `wait_for`, or read `network` and `console` to see what stalled. |
| Engine readiness never arrives | Spawn worked but `/json/version` stayed silent past the 30 s deadline. | Raise `GLADIUS_STARTUP_TIMEOUT_SECS` on a slow machine; otherwise check `GLADIUS_CHROMIUM` points at a real `chrome-headless-shell` binary, not full Chrome. |
| `cookie_rejected` | The engine refused the cookie write, caught by read-back verification. | Check the domain, `SameSite` and `Secure` attributes for the scheme you are on. |
| `origin_mismatch` on `storage` `load` | The saved file came from a different origin. | Navigate to the origin it was saved from, or pass `force: true` deliberately. |
| `--help` exits 2 | An unknown flag was passed. | Only `--doctor`, `--print-manifest`, `--version`, `--help` and `-h` exist. |

Server logs go to STDERR. If a client shows nothing, check its MCP server log
rather than the chat - STDOUT belongs to the protocol.

---

## 5. Limitations

- **Windows x64 only.** No other platform runs the server runtime.
- **Chromium-family only.** No Firefox, no WebKit, no multi-browser support.
- **Headless only, by owner decision.** There is no headed code path and none is
  planned; the `chrome-headless-shell` artifact cannot go headed at all. The
  visible path is an explicit attach to an engine you own.
- **No OS input, ever.** Everything rides CDP: no cursor, no keyboard injection,
  no focus stealing, no window to raise. This is a guarantee, not a setting.
- **No shadow-DOM acting.** Shadow nodes resolve in `snapshot`, but acting on a
  shadow ref through the main-document resolver is not supported yet (typed
  `stale_ref`).
- **No vision or coordinate grounding.** `snapshot` and `@eN` refs are the
  contract; there is no model in the server.
- **No PDF generation, no fleet or cloud operation, no in-server planning or LLM
  calls.** gladius is the fast local minimum underneath those systems.
- **No fully offline fresh install.** The first run always downloads the pinned
  runtime and browser.
- **Pinned versions are not a long-term install.** The one-release policy deletes
  older GitHub releases, so only `@latest` is supported.
- **One page per connection.** Each stdio client gets its own browser context and
  page; contexts never leak across connections.

Architecture, budgets and the frozen tool contract:
[`ARCHITECTURE.md`](ARCHITECTURE.md). Engine launch contract:
[`ENGINE.md`](ENGINE.md). Headless-only decision:
[`HEADED-MODE.md`](HEADED-MODE.md).
