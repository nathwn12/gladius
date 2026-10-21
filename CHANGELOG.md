# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [0.4.2] - 2026-10-21

- Launcher/engine version handshake (fail closed): the npm launcher sets
  `GLADIUS_EXPECTED_VERSION` to its own `package.json` version in the engine
  child env, and refuses when its `engine-manifest.json` `serverVersion`
  disagrees with it (actionable reinstall line, never "whatever is cached").
  The engine (`src/main.rs`) exits 3 on a mismatch instead of serving a
  mixed-version pair.
- Launcher prints its own version (`--version`, no network) and derives the
  runtime cache dir from the agreed pin (`<serverVersion>-<bundleSha8>`).
- Global installs refused by design: `preinstall` guard fails
  `npm i -g` with the `npx gladius-mcp@0.4.2` line; local installs unaffected.
- Fixed stale engine pin: the manifest's `runtime.source`/`archive` pointed
  at `v0.4.0` while the package was `0.4.1`; both now derive from `0.4.2`.
- Docs: screenshot `path` writes the decoded PNG bytes to the given path
  (see README); `gladius --version` prints `gladius <crate> (<sha>[-dirty])`.

## [0.4.1] - 2026-10-20

- Docs: one-release policy — install with `npx gladius-mcp@latest`; a pinned version is only installable while its GitHub release exists.

## [0.4.0] - 2026-10-20

> Note: entries below marked "Pre-0.2.1 (Obscura era)" describe the former
> engine and are superseded by `[0.2.1]` (owned pinned Chromium); the former
> record is kept for provenance in `docs/OBSCURA-COMPAT.md`. Unmarked entries
> describe the current Chromium engine. (`Cargo.toml`/`npm/package.json` stay
> at 0.3.0 until the X2 version bump; this entry collects everything shipped
> since `[0.3.0]`.)

- MCP protocol truth: strict JSON-RPC 2.0 framing over stdio (batch,
  method-less, scalar, malformed, and invalid-id frames rejected with
  -32700/-32600/-32601), the 2025-06-18/2025-11-25/2026-07-28 version matrix
  (modern `resultType: complete`, legacy without), `server/discover`, a
  `serverInfo.count` derived from the real tool list, and a per-request
  execution lock with call cancellation/supersede. No browser runs before
  the first tool call needs it.
- AX snapshot by default: the real Accessibility tree (one line per
  actionable node with a stable `@eN` ref) is the default `snapshot`;
  `mode:"legacy"` keeps the old DOM heuristic, `mode:"text"` is text-only.
  Hidden/ignored subtrees stay out (an old ref to one is a typed
  `stale_ref`, never a silent click); shadow refs resolve but acting through
  the main-document resolver is a typed error. `snapshot`/`evaluate` take
  `frame=` to scope into an iframe's realm (isolated world, works
  cross-origin); `targets` lists every target plus the frame tree.
- Trusted input: `click` dispatches a real CDP pointer sequence
  (`mouseMoved`/`mousePressed`/`mouseReleased`, `isTrusted:true`) after
  scroll-into-view, an `elementFromPoint` hit-test, and a 120 ms
  navigation-settle watch; `type` focuses + select-alls, then trusted
  `Input.insertText` (the seed is replaced, never appended). Occluded,
  hidden, disabled, or non-editable targets are typed errors - the pointer
  never lands on the wrong surface. `key` handles `Ctrl+A`-style combos
  with an Alt=1/Ctrl=2/Meta=4/Shift=8 modifier mask (`rawKeyDown`, no text
  while Ctrl/Meta/Alt is held).
- Frame-scoped actions: `click`/`type`/`hover`/`select`/`upload` accept
  `frame=` (CDP frameId). Geometry maps leaf-rect -> owner chain (padding +
  border per ancestor, JS rect chain) to main-frame viewport px; every
  ancestor level is hit-tested in its own realm against the true stamped
  owners, so a mid-level overlay is `occluded` too. `@eN` refs stay
  main-frame (ref+`frame` is typed `invalid_args`); unmapped/OOPIF frames are
  typed errors, never silent main-frame actions (this engine exposes no
  separate iframe target, so no OOPIF landing is claimed).
- New `storage` tool: cookies + `localStorage`/`sessionStorage` in one call
  (page-scoped by default, `allSites:true` widens cookie read/clear),
  plus versioned file save/load (`{gladiusStorage:1,...}`; replies carry
  counts/bytes/origin only, never values). Cookie writes are read-back
  verified (`cookie_rejected` otherwise, compared by name/domain/path
  against the whole jar); `load` enforces an origin match unless
  `force:true`; `redact:true` masks values (`••••`).
- Gates: `rust-toolchain.toml` pins the crate MSRV (1.88.0, rustfmt +
  clippy); `cargo fmt --all --check` is clean; input schemas are tightened
  (enums for modes/actions, `minProperties`/`minLength`/`minItems`,
  `oneOf` for `viewport` reset vs width+height, `additionalProperties:
  false` everywhere - output schemas frozen); CI runs fmt, clippy
  `-D warnings`, the full suite, and the release `gladius selftest`.
  Tool count 21 -> 22.

- Build provenance: `gladius --version` (also `version`) now reports the crate
  version **and** the exact commit the binary was built from, stamped at build
  time by `build.rs` (`git rev-parse HEAD`, `-dirty` when the tree held
  uncommitted changes, `unknown` with no `.git` or no `git` on PATH). A
  deployed `gladius.exe` can be compared against `git rev-parse HEAD` instead
  of being judged by its mtime. The unknown-subcommand usage line now names
  `--version`.

- New `viewport` tool: the page viewport is now settable in CSS pixels
  (`width`, `height` required; `deviceScaleFactor` default 1; `mobile`
  default false) over CDP `Emulation.setDeviceMetricsOverride`, with
  `reset:true` clearing it via `Emulation.clearDeviceMetricsOverride` and
  restoring the engine default. Every call returns the LIVE read-back of
  `window.innerWidth`/`window.innerHeight`/`window.devicePixelRatio` beside
  the requested metrics (`matched:false` when the engine did not honour the
  request), so a silently ignored override can never read as success. The
  override is session-scoped in CDP, so it is REPLAYED on every new session
  (`attach_to`): a later `navigate` or a new tab keeps it. Measured engine
  default (pre-Chromium engine v0.2.4): **1280x720** - the "fixed 800x600-ish default"
  claim in `docs/OPT-SURVEY.md` was wrong. Tool count 15 -> 16.

- Machine-readable error contract: every tool error now carries a stable
  `structuredContent` object alongside the existing prose, which is
  byte-for-byte unchanged in `content[0].text`. Each error site mints its
  own `code`/`reason` plus the real offending value (`ref` for an AX ref,
  `element` for a selector, `path` for a local file, `want` for a wanted
  option, `name` for an unknown tool) — the code is never derived by
  re-reading the prose, so caller-controlled payload (a selector, path, or
  option name containing e.g. `readonly` or `@e42`) cannot misclassify it.
  Closed code set: `stale_ref` (`unknown`/`navigated`/`node_gone`),
  `not_actionable` (`hidden`/`disabled`/`readonly`), `invalid_target`
  (`not_select`/`not_found`/`not_file`), `option_not_found` (`no_match`),
  `capability_gated` (`dom_only`, e.g. the `dom-only` `screenshot` refusal),
  `timeout` (`budget_exceeded`), `invalid_args`
  (`missing_arg`/`bad_index`/`unknown_action`/`unknown_tool`/`need_option`),
  `file_not_found` (`missing_path`), and an `error` fallback that carries a
  `code` only (CDP/js-engine failures and anything the site cannot type —
  no reason is invented for those). No tool name, argument, or message
  changed.
- Structured snapshot refs: the AX `snapshot` return gains a second content
  block `{"url","title","truncated","nodes":[{ref,role,name,tag,disabled}]}`,
  built from the node table the snapshot already computed (zero new page
  evaluation; on a truncated page `nodes` holds exactly the shown refs and
  `truncated` is true). The human text block stays `content[0]` and is
  byte-identical (8000-char cap and truncation footer unchanged);
  `snapshot {"mode":"text"}` keeps its single block.
- Caps-aware capability manifest: `initialize` `capabilities` now reports
  `{"tools":{"count":15,"gated":{...}}}` derived from the real tool list and
  `backend_info()` — on `GLADIUS_BACKEND_CAPS=dom-only` it names
  `screenshot: requires caps: render`, and an invalid/unusable caps value
  gates `screenshot` the same way (no rendering pipeline to capture from);
  on a `render` backend `gated` is empty. `serverInfo` is unchanged.
- Pre-0.2.1 (Obscura era, superseded by `[0.2.1]`): REMOVED the `chrome-headless-shell` fallback/opt-in entirely: `find_chrome`,
  `GLADIUS_BACKEND`, `GLADIUS_CHROME`, the `%LOCALAPPDATA%\ms-playwright`
  resolution, and the chrome launch/profile/sweep machinery were gone.
  Gladius was then self-contained on the owned Obscura engine in `bin\` as the only
  launchable engine, plus an explicit external attach via `GLADIUS_CDP_URL`.
  Enforced by the `self_containment_no_foreign_engine_in_src` guard test,
  which fails the build if any chrome/ms-playwright/playwright reference
  reappears in `src/`.
- Pre-0.2.1 (Obscura era, superseded by `[0.2.1]`; see `docs/OBSCURA-COMPAT.md`): Obscura was the ONLY engine: unless `GLADIUS_CDP_URL` attached, gladius
  (`obscura.exe serve --host 127.0.0.1 --port <free> --allow-private-network
  --allow-file-access` beside `gladius.exe`, `CREATE_NO_WINDOW`, readiness by
  polling `GET /json/version`). Provisioned by `scripts/fetch-engine.ps1`
  (pinned v0.2.4, sha256-verified, `bin\obscura.exe` +
  `bin\obscura-worker.exe`); a missing engine is a clear error naming that
  script, never a silent fallback. `GLADIUS_OBSCURA` overrides the engine
  path. `initialize` reports `serverInfo.backend` `obscura` on this path.
- Pre-0.2.1 capability exceptions on the Obscura path (historical, see `docs/OBSCURA-COMPAT.md`): JS dialogs were inert
  (`Page.javascriptDialogOpening` never fires, `Page.handleJavaScriptDialog`
  is unknown, `alert`/`confirm`/`prompt` silent no-ops); the `console` tool's
  `Log.entryAdded` leg is blind (`Runtime.consoleAPICalled` still flows);
  `upload` hard-requires the `--allow-file-access` gate flag (gladius always
  passes it); the engine can commit a blank page on a double-fetch race,
  surfaced as a `warning` field on `navigate` (re-navigate to confirm, no
  auto-retry).
- External-CDP attach path (`GLADIUS_CDP_URL`): when set to a `ws://`
  (`wss://`) endpoint gladius connects to it instead of launching an engine;
  an `http://` (`https://`) value resolves the ws endpoint from the
  documented readiness endpoint `GET <base>/json/version`, with no dependence
  on engine stderr wording. The session/target flow after the connect is the
  same code as the launch path, never a fork.
- Ownership rule (enforced in code): an attached engine is NOT ours. `close`
  detaches the CDP session gladius opened (`Target.detachFromTarget`) and
  never calls `kill_tree`, never kills a pid, never sweeps a profile it did
  not create. The external process stays alive; gladius launches nothing.
- Capability declaration (`GLADIUS_BACKEND_CAPS`): `render` (default) or
  `dom-only`. With `dom-only`, `screenshot` refuses with a typed error naming
  the backend and the missing rendering pipeline, never a fake or blank
  image (`upload` is DOM CDP only and keeps working — verified, not assumed).
  `initialize` reports `serverInfo.backend` and `serverInfo.caps` so a client
  can see which engine and which caps it is talking to.
- Console-flash hardening: the `taskkill` reaper now spawns with
  `CREATE_NO_WINDOW`, so closing never flashes a console window even for
  launchers that give gladius no console of its own.
- Pre-0.2.1 self-contained engine resolution (superseded by `[0.2.1]`): the owned Obscura engine only
  (`GLADIUS_OBSCURA` override, else `bin\obscura.exe` beside `gladius.exe`);
  a missing engine is a clear error naming `scripts/fetch-engine.ps1`.
- Pre-0.2.1 guard (superseded; the current guard is `self_containment_no_ambient_or_shared_browser_in_src`, which forbids ambient/shared resolutions while allowing the owned pinned Chromium): the Obscura-era `self_containment_no_foreign_engine_in_src` guard test failed the build
  if any chrome/ms-playwright/playwright reference reappears in `src/`,
  alongside the `no_console_flash` windowless-spawn guard.
- `wait_for` hard deadline holds across the lazy network snapshot: the
  `Network.enable` / `Fetch.enable` enable was awaited OUTSIDE the deadline,
  so a peer holding the `Network.enable` response made a 100 ms
  `networkIdle` wait answer after `send_raw`'s 30 s per-call timeout, with a
  raw CDP error instead of the typed `timeout`/`budget_exceeded`. The enable
  now sits inside the remaining budget; a genuine CDP failure still reports
  its own message.
- No leaked CDP response slots on a dropped call: a pending-map entry is now
  owned by a guard whose drop removes it, so clipping a call (that enable, a
  caller's timeout, a cancelled task) can no longer leave one stale entry per
  call behind.
- README copy block: the destination directory is created as part of the
  example (`Copy-Item` creates the files, never the destination directory),
  and the example no longer names an absolute machine path.

## [0.3.0] - 2026-10-18

- Full-fledged browser tooling: 16 tools -> 21, schemas backward-compatible
  (every addition is optional; existing callers are unaffected).
- New `targets` tool: every CDP target with its stable id (pages, workers of
  all kinds) plus the active page's frame tree (main + nested iframes, each
  with its frameId), and `select` to make a page target current by id.
- Target-scoped `evaluate`: optional `target` (CDP targetId: page,
  dedicated/shared worker, service worker) and `frame` (CDP frameId:
  isolated-world eval, works cross-origin) route `Runtime.evaluate` to that
  realm; the tracked active page is untouched. Worker realms answer ONLY on
  their auto-attach event session (a direct attach returns a session whose
  commands hang on this engine, measured): page sessions auto-attach
  (session-scoped `Target.setAutoAttach`, no pausing) and the reader records
  `Target.attachedToTarget` targetId -> sessionId for the route.
- Fail loud: unknown arguments are rejected with typed
  `invalid_args`/`unknown_arg` on every tool (new reason in the closed code
  set), never silently ignored - the shape that once routed evaluations to
  the wrong realm with no error.
- `wait_for` gains an `expression` predicate form: polls a computed JS
  condition on the active page with backoff inside the shared hard deadline;
  expiry is the typed timeout, a throwing condition is a real error.
- New `dialog` tool: policy (`accept` + optional `promptText`, default
  accept) applied by the reader's auto-handler plus the last recorded dialog
  (`type`/`message`/`url`). Covers alert/confirm/prompt/beforeunload; a
  session can never deadlock on a dialog. beforeunload needs sticky
  activation to fire on this engine (trusted input first).
- New `history` tool: back/forward/reload on the active page, returning the
  live url+title after the load settle; a step past the boundary is a typed
  error, never a silent no-op.
- New `network` tool: observable capture (method/url/status/resource type/
  timing, cap 500) with get/drain/clear and a url-substring filter. The
  Network domain arms lazily on first `network` use (any action, including
  `clear`); drain removes only the returned entries.
- New `cdp` tool: raw `{method, params, session?}` escape hatch returning the
  raw CDP result; a CDP error reply is a tool error, never a protocol error.
- Main-thread stack: `mcp`/`selftest` now run on a runtime worker instead of
  the 1 MB PE main-thread stack the block_on polled them on (a cold launch
  overflowed main deterministically on one build and ran clean on the next;
  the depth, not the code, was at the edge).
- Version bump 0.2.1 -> 0.3.0 (`Cargo.toml` + `npm/package.json` in step).
  NOT published: the harness keeps consuming `gladius-mcp@latest` until an
  owner-named publish happens.

## [0.2.1] - 2026-10-17

- Engine swap: Obscura is gone. Gladius drives the owned pinned Chromium
  (`chrome-headless-shell` `155.0.8059.84`, headless-only, no headed path)
  provisioned by `scripts/fetch-chromium.ps1` into
  `%LOCALAPPDATA%\gladius\chromium\` (overridable via
  `GLADIUS_CHROMIUM_CACHE`, explicit binary via `GLADIUS_CHROMIUM`).
  `initialize` reports `serverInfo.backend` `chromium`. JS dialogs are real
  (auto-accepted and reported) and the `console` tool's `Log.entryAdded` leg
  is live. `scripts/fetch-engine.ps1` is deleted; the former Obscura record
  is kept for provenance in `docs/OBSCURA-COMPAT.md`.
- Shipping path matches the new engine: CI provisions the pinned browser
  (restored from cache per pin, re-verified) instead of Obscura; the release
  runtime bundle carries only the server plus licences/notices — the browser
  is fetched from Google's Chrome-for-Testing endpoint at provision time and
  never bundled; the `gladius-mcp` launcher provisions and verifies both the
  server runtime and the browser on first run and launches the server with
  `GLADIUS_CHROMIUM` pointed at the pinned browser.

## [0.1.0] - 2026-10-11

Initial release.

- Headless CDP browser-use MCP server: a native Rust binary driving headless
  Chrome directly over CDP via WebSocket — no Node.js, no Playwright in the
  runtime path. MCP served over stdio as hand-rolled newline-delimited
  JSON-RPC 2.0.
- 15-tool surface: `navigate`, `snapshot` (AX snapshot with stable `@eN`
  refs plus legacy text mode), `screenshot`, `click`, `type`, `evaluate`,
  `wait_for` (selector/text, delay, network idle), `console`, `upload`,
  `fetch_text` (browser-free fast path), `hover`, `select`, `key`, `tabs`,
  `close`.
- Measured latency work: click settle watch (~1 s per non-navigating click)
  and navigate load-poll cadence (~100 ms per navigate) identified and
  documented in `docs/OPT-SURVEY.md`, with ranked optimizations.
- Test suite: `cargo test` integration suites (`actionability`, `dialog_tabs`,
  `e2e_mcp`, `hard_kill_reap`, `new_tools`, `refs`, `review_findings`) plus
  the `gladius.exe selftest` end-to-end check.
