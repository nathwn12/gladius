# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

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
- REMOVED the `chrome-headless-shell` fallback/opt-in entirely: `find_chrome`,
  `GLADIUS_BACKEND`, `GLADIUS_CHROME`, the `%LOCALAPPDATA%\ms-playwright`
  resolution, and the chrome launch/profile/sweep machinery are gone.
  Gladius is self-contained: the owned Obscura engine in `bin\` is the only
  launchable engine, plus an explicit external attach via `GLADIUS_CDP_URL`.
  Enforced by the `self_containment_no_foreign_engine_in_src` guard test,
  which fails the build if any chrome/ms-playwright/playwright reference
  reappears in `src/`.
- Obscura is the ONLY engine: unless `GLADIUS_CDP_URL` attaches, gladius
  (`obscura.exe serve --host 127.0.0.1 --port <free> --allow-private-network
  --allow-file-access` beside `gladius.exe`, `CREATE_NO_WINDOW`, readiness by
  polling `GET /json/version`). Provisioned by `scripts/fetch-engine.ps1`
  (pinned v0.2.4, sha256-verified, `bin\obscura.exe` +
  `bin\obscura-worker.exe`); a missing engine is a clear error naming that
  script, never a silent fallback. `GLADIUS_OBSCURA` overrides the engine
  path. `initialize` reports `serverInfo.backend` `obscura` on this path.
- Capability exceptions on the Obscura path: JS dialogs are inert
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
- Self-contained engine resolution: the owned Obscura engine only
  (`GLADIUS_OBSCURA` override, else `bin\obscura.exe` beside `gladius.exe`);
  a missing engine is a clear error naming `scripts/fetch-engine.ps1`.
- New `self_containment_no_foreign_engine_in_src` guard test: fails the build
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
