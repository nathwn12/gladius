# Lens 4 — Robustness & Lifecycle (read-only audit)

Repo `Q:\PROJECTS\PERSONAL\gladius`, HEAD `abba9b1`. Method: source read only
(`browser.rs`, `mcp.rs`, `tools.rs`, `capture.rs`, `visual.rs`, `fetch.rs`,
`main.rs`, `npm/bin/gladius-mcp.js`); no build, no launch. Numbers below are
`file:line` in `src/` unless noted.

## Will bite this week (ranked: probability x damage)

1. **Browser death is terminal for the process; nothing respawns, and the error
   doesn't name the way out.** Scenario: renderer crash / OOM / browser killed
   → reader exits → `dead=true` is stored once and never cleared
   (`browser.rs:3122` is the only `dead.store` in src; `ensure_session`
   fail-fasts at `2321-2323`). Every later tool call fails with
   `cdp: engine connection closed (reader exited)`. Recovery exists
   (`close` tool → `browser.close()` takes state to `None`,
   `tools.rs:810-818` + `browser.rs:1360-1439`; next `shared()` relaunches via
   `537-552`) but no error string mentions it. Symptom: agent sees a dead
   session and retries the same call forever. Severity: **blocker**.
   Fix: append `; recover with the close tool (next call relaunches)` to the
   reader-exited error, or auto-relaunch owned engines on next call.

2. **Hard-kill of the server (this machine's service restart loop) orphans the
   owned Chromium; zombies accumulate, profiles linger.** Graceful stdin-EOF is
   clean (`mcp.rs:615-630` aborts calls, `browser.close()`, exit 0). But
   `kill_tree` runs only on graceful paths (`close()` at `browser.rs:1402`,
   `Drop for Browser` at `2096-2110`, `ChildGuard` Drop at `3148-3154`).
   SIGKILL skips all of it: the chromium tree keeps running and its `gladius-
   <pid>-*` profile (`ProfileDir::create`, `3650-3667`, under `temp_dir()`)
   stays locked. Next launch's `sweep_stale_profiles` (`3741+`) deletes *dirs*
   only, never kills pids — an orphan still holding files blocks the sweep, so
   both the process (~tens of MB RSS) and the dir leak per kill. Symptom: one
   zombie `chrome-headless-shell.exe` + one temp dir per restart-loop kill.
   Severity: **major** (known 80-258 s restart loop on NECESSE makes this
   weekly, not theoretical). Fix: on launch, kill processes whose command line
   carries our stale `--user-data-dir=...gladius-<deadpid>-*` before sweeping;
   or record child pid in a lockfile and reap-then-sweep it.

3. **No overall per-tool timeout: one wedged call blocks the whole server.**
   Every CDP round-trip allows 30 s (`send_raw`, `browser.rs:2209`); tools chain
   many (`navigate` = enable + navigate + 10 s `wait_for_load` + 2 evals,
   `tools.rs:924-949`). Only `wait_for` has a hard budget (15 s, `3632`).
   A stalled renderer turns one call into minutes *while holding the exec lock*
   (`needs_exec_lock`, `mcp.rs:122-159`), serialising all mutating tools behind
   it; the 4-slot semaphore (`mcp.rs:23`) then refuses everything else with
   `Server busy`. Symptom: agent's turn eaten by one hung call, cascading
   busy errors. Severity: **major**. Fix: wrap `tools::call` in a total budget
   (e.g. 60-90 s → typed `timeout`), or at minimum don't hold `exec` across
   the unbounded settle phases.

4. **Cancellation of a capture can eat up to 60 s of the agent's turn.**
   `abort_after_critical` waits `CANCEL_GRACE = 60 s` (`mcp.rs:186, 222-254`)
   before aborting a wedged deterministic capture/visual. Correct for state
   safety, but the agent that cancelled sees silence for a minute. Minor-major.
   Fix: answer the cancel promptly with a typed `cancelled` result while the
   waiter still guards the restore in the background.

5. **`GLADIUS_VISUAL_DIR` defaults into the repo and never GCs.**
   Default `<cwd>/.gladius/visual` (`visual.rs:82-91`); `save`/`compare` write
   `<name>.png` + `.meta.json` + `.diff.png` per mismatch with no cap, LRU, or
   age-out — only manual `delete`. Symptom: baselines + diffs grow unbounded
   in whatever dir the agent ran from (often a repo). Severity: **minor**
   (disk, plus surprise repo writes). Fix: default-point the dir at
   `%LOCALAPPDATA%/gladius/visual` and cap entries (warn + refuse, or LRU).

6. **Closed shadow roots are the silent hole; selectors can't reach any shadow
   DOM.** Zero mentions of "closed" in `src/`. Selector probes run
   `document.querySelector` in the frame realm with no piercing (e.g. hover
   probe `tools.rs:3662`; same shape for click/type/select). Geometry resolves
   from `DOM.getDocument(pierce:false)` + `querySelectorAll`
   (`layout.rs:136-151, 195-208`). Only the `@eN` ref path rides backend node
   ids (`browser.rs:400-401` "shadow nodes resolve like light DOM"), which
   covers *open* shadow via the AX tree; closed-shadow content has no proven
   path (can't query, unconfirmed AX exposure, untested). Symptom: click/type
   on a web-component interior returns `not_found` and the agent has no lever.
   Severity: **major** when it hits, **minor** frequency. Fix: one
   closed-shadow test page in selftest to establish ground truth, then either
   pierce where CDP allows or a typed `shadow_closed` error.

## What actually recovers well (no action)

- **Mid-call navigation / `window.close` / crashed tab:** sends are
  single-shot, never retried; session-death errors invalidate the cache and
  surface typed `stale_session` (`browser.rs:2287-2316`); the reader clears the
  tracked session on `detachedFromTarget` (`3022-3048`); next call re-attaches
  and creates a page when the farm is empty (`ensure_session`, `2318-2359`).
  `wait_for` is pinned to its starting session and stops instead of migrating
  (`check_session_pin`, `1871-1886`).
- **Races:** mutating tools + deterministic captures serialize on the exec
  lock (`mcp.rs:122-159`, `exec_lock_tests` gate); plain screenshots stay
  lock-free but refuse with typed `target_changed` if the page moved under
  them (`tools.rs:2327-2355`). `tabs` ops can't interleave.
- **Buffers are bounded where it matters:** console ring 200 (`browser.rs:60`),
  network ring 500 (`:99`), `console get/drain` clamped 1-200
  (`tools.rs:3346`), `fetch_text` 256 KB + 8000 chars (`fetch.rs:8,104`),
  snapshot paging with char cap (`tools.rs:1478-1513`). Panics in tool tasks
  become `-32603` with id echo, never a dropped reply (`mcp.rs:826-874`).
  No production `unwrap` on untrusted input found (`.expect` sites are
  startup/test scaffolding or `ChildGuard` internals); `PendingGuard`
  (`2192-2201`) prevents pending-map leaks on timeout/abort.
- **Pre-`navigate` calls:** first tool lazily launches and lands on a created
  `about:blank` (`shared()` `537-552`, `ensure_session` `2345-2358`) — no dead
  session, no special-casing needed.
- **`evaluate` addressing is complete:** `target=` / `frame=` mutually exclusive
  with typed refusal (`tools.rs:3305-3311`); cross-origin same-process frames
  via isolated worlds; OOPIF without a mapped session is a typed error, never
  a main-frame mis-fire. `<dialog>`, portals-to-`body`, top layer are plain DOM
  and visible to AX/snapshot/hit-test.

## Theoretical / low-probability (one line each)

- **Oversized results untruncated:** full-page PNG rides inline as base64 in one
  JSON-RPC line (`tools.rs:2368`, no cap — tens of MB possible); network
  entries are ring-bounded but per-entry URL strings are not (a `data:` URL
  leg bloats one entry). No MCP chunking anywhere.
- **Two agents, one server:** exec lock + semaphore serialize, but the single
  active page + replayed viewport override (`browser.rs:2437-2446`) mean agent B
  inherits agent A's tab/viewport — fight-by-design; separate servers (the
  normal shape) are isolated.
- **Duplicate JSON-RPC id kills its own in-flight call** (supersede +
  `abort_after_critical`, `mcp.rs:571-586`) — correct for retries, but an agent
  reusing ids across parallel calls cancels itself; sequential reuse is safe.
- **CDP-side work is not cancelled on client cancel** — the renderer finishes
  dropped work; harmless (results unrouted) except wasted engine time.
- **Version skew refuses to serve** (`GLADIUS_EXPECTED_VERSION` gate,
  `main.rs:26-35`, exit 3; launcher pin check `gladius-mcp.js:1071-1076`) —
  correct fail-closed, but after an upgrade the symptom is a dead tool
  namespace until reinstall; the stderr line names the fix.
- **Windows/packaging verified sane:** ~124 MB zip (`fetch-chromium.ps1:45`)
  → ~290 MB under `%LOCALAPPDATA%/gladius/...` (launcher header); blocked
  download fails typed with remediation; 15-min fetch timeout, no resume;
  `LOCALAPPDATA` unset → clear error (`browser.rs:3236-3240`); argv-based
  launch, no shell quoting issues with spaces/non-ASCII;
  `CREATE_NO_WINDOW` on chromium + taskkill (`3506, 3793, 3840`), stdio nulled;
  loopback-confined ws endpoint (`3302-3337`). Gaps: no free-space checks
  anywhere (C: full → typed write failures only); AV stalls covered by
  launcher EPERM retries, engine-side file ops retry only in the profile sweep
  (10 s, `3708-3733`); profile follows `temp_dir()` so another drive works via
  `TEMP`/overrides (`GLADIUS_CHROMIUM_CACHE`, `GLADIUS_VISUAL_DIR`,
  `GLADIUS_ENGINE_CACHE` all honoured).
- **Renderer crash without detach is detected late:** no `targetCrashed`
  handler in the reader — only `detachedFromTarget`; a crash surfaces on the
  *next send* via the session-death class (`2263-2269`) or the 30 s response
  timeout, not proactively. Benign given single-shot + re-attach, but the
  in-flight call pays the full timeout.

## Not checked

- Actual runtime behaviour (no build/launch per brief): zombie counts, real
  closed-shadow AX exposure, oversized-screenshot client impact, AV-on-write
  paths — all inferred from code, flagged above where uncertain.
- `tests/` dir and `selftest.rs` coverage depth beyond the cited hooks; the
  release workflow and signing; Linux/macOS paths (Windows-only product).
- Adjacent note: `bin/` holds stale `gladius.exe.old-*` files (~10) — hygiene,
  not robustness.
