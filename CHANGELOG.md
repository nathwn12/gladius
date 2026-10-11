# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

- Obscura is now the DEFAULT engine: unless `GLADIUS_CDP_URL` attaches or
  `GLADIUS_BACKEND=chrome` opts in, gladius launches the owned Obscura engine
  (`obscura.exe serve --host 127.0.0.1 --port <free> --allow-private-network
  --allow-file-access` beside `gladius.exe`, `CREATE_NO_WINDOW`, readiness by
  polling `GET /json/version`). Provisioned by `scripts/fetch-engine.ps1`
  (pinned v0.2.4, sha256-verified, `bin\obscura.exe` +
  `bin\obscura-worker.exe`); a missing engine is a clear error naming that
  script, never a silent fallback. `GLADIUS_OBSCURA` overrides the engine
  path. `initialize` reports `serverInfo.backend` `obscura` on this path.
- Chrome-headless-shell is now an explicit opt-in (`GLADIUS_BACKEND=chrome`,
  case-insensitive): `GLADIUS_CHROME` wins inside that path, else the newest
  shell under `%LOCALAPPDATA%\ms-playwright`; the no-system-Chrome-fallback
  rule is unchanged. Full capability record (gate flags, per-tool matrix,
  quirks, numbers): `docs/OBSCURA-COMPAT.md`.
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
- Headless-shell-only backend: engine resolution is `GLADIUS_CHROME`, then
  the newest `chrome-headless-shell.exe` under `%LOCALAPPDATA%\ms-playwright`;
  the system-Chrome fallback is gone, a missing shell is a clear error.
- New `no_console_flash` guard test: every process-spawn site must carry the
  windowless flag, and no system-browser fallback may regress into `src/`.

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
