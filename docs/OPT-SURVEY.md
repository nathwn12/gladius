# OPT-SURVEY - gladius optimization + feature survey

> Archive note (2026-10-12): this recon measured the pre-self-containment
> build, so it names the removed chrome launch path. Gladius is
> self-contained since `96e73e3`: the owned `bin\obscura.exe` is the only
> launchable engine, plus explicit attach via `GLADIUS_CDP_URL`. Timings and
> hot-path structure below are historical evidence, not current instructions.

> Recon artifact for session `ses_ed8bd551cffeHmdMpTecyGZT4W` (fleet ses_ed8bd551cffeHmdMpTecyGZT4W), 2026-10-11,
> local Windows, headless only, no source edits (one new doc: this file).
> Scratch evidence (driver, fixtures, logs, raw JSON): `%TEMP%\opencode\ses_ed8bd551cffeHmdMpTecyGZT4W\`
> (marked `--drop--ses_ed8bd551cffeHmdMpTecyGZT4W`; disposition in the Appendix).
> All numbers in this doc were measured on this machine unless explicitly labelled otherwise.

---

## 1. Provenance

No `.git` in `<repo>` (directory listing: `.gitignore` only). Provenance is
established by hashes + mtimes + an isolated fresh rebuild.

| Artifact | Size (B) | mtime (local) | SHA256 |
|---|---|---|---|
| `bin\gladius.exe` | 5,497,344 | 2026-10-10 5:23:32 PM | `DCD0CE9F82C36BE2365B4EE7B8C5CF439D80E40C66F2A9B059BF7F038C2F8C63` |
| `target\release\gladius.exe` | 5,497,344 | 2026-10-10 5:23:32 PM | `DCD0CE9F82C36BE2365B4EE7B8C5CF439D80E40C66F2A9B059BF7F038C2F8C63` (byte-identical to `bin`) |
| fresh isolated build (`CARGO_TARGET_DIR=%TEMP%\opencode\ses_ed8bd551cffeHmdMpTecyGZT4W\fresh-target`) | 5,497,344 | 2026-10-11 3:34:45 AM | `456F56816CDE9A2B87F9ED8564140A83C3E014DE3A710ADD2A599632C5BC5BDC` |

Crate identity: `gladius` **v0.1.0** (edition 2021), `[[bin]] gladius -> src/main.rs`
(`Cargo.toml:1-8`). Last source edit: `src\tools.rs` 2026-10-10 5:23:16 PM; `src\browser.rs` 8:48:15 AM.
The 5:23:32 PM binary postdates the last edit by 16 s.

**Fresh-vs-shipped byte diff:** 24 differing bytes out of 5,497,344 - all link metadata, zero code/data:

| Offset(s) | Bytes | What |
|---|---|---|
| 264-265 (PE header) | 2 | `TimeDateStamp`: bin `0x6aca0414` vs fresh `0x6aca9355` |
| 4609876, 4609904, 4609932 | 3 x 2 | debug-directory date stamps |
| 4610292-4610307 | 16 | CodeView RSDS GUID (RSDS record at 4610288, `gladius.pdb` path at 4610312) |

**Verdict: VERIFIED.** `bin\gladius.exe` is the cargo release build of the current source tree;
`cargo build --release` (exit 0, twice, freshness no-op) confirms the tree compiles to exactly this
binary. The isolated scratch rebuild reproduces the same size and identical code/data, differing only
in link timestamps/GUID (normal MSVC linker metadata).

## 2. Build + selftest

| Command | Exit | Evidence |
|---|---|---|
| `cargo build --release` (repo target) | **0** | first run 4.12 s refresh, second run 0.25 s fresh no-op (log: `%TEMP%\...\build.log`) |
| `cargo clean -p gladius; cargo build --release` | **0** | clean removed 86 files / 29.7 MiB, rebuild no-op (freshness re-confirmed; log in shell transcript) |
| fresh isolated build (`CARGO_TARGET_DIR` scratch) | **0** | 34.15 s full dependency rebuild (`%TEMP%\...\freshbuild.log`) |
| `target\release\gladius.exe selftest` | **0** | `SELFTEST navigate/snapshot/evaluate/click/click-toggle/close: PASS` + `SELFTEST: ALL PASS` |
| fresh-build `gladius.exe selftest` | **0** | same six PASS lines from the scratch build |

Resolved backend (stderr): `<ms-playwright>/chromium_headless_shell-<rev>/chrome-headless-shell.exe`,
job object armed `KILL_ON_JOB_CLOSE`, throwaway profile `%TEMP%\gladius-<pid>-...`. All test browser
trees were reaped: **zero `chrome-headless-shell` processes survive** (checked after every run).

## 3. Measured timings

Method: scratch stdio driver `%TEMP%\opencode\ses_ed8bd551cffeHmdMpTecyGZT4W\timing\driver2.py`
spawns `bin\gladius.exe mcp` (CREATE_NO_WINDOW), newline JSON-RPC, ops against local `file://`
fixtures (`timing\fixture.html` - 13 actionable nodes; `timing\big.html` - 3,000 links).
Phase A = 5 full cold cycles (spawn -> initialize -> navigate -> close -> stdin EOF -> exit).
Phase B = one warm session, 5 repeats/op (op latency = send -> response, ms).
Raw: `%TEMP%\...\timing\results.json`; stderr logs `stderr-cold1..5.log`, `stderr-warm.log`, `stderr-big.log`.

| Op | n | min | **median** | max |
|---|---|---|---|---|
| cold initialize (spawn -> resp, no browser) | 5 | 19.3 | **24.5** | 27.0 |
| cold navigate (spawn -> resp; chrome spawn + CDP connect + attach + Page.enable + load-poll + 2 evals) | 5 | 175.4 | **281.4** | 333.7 |
| warm navigate | 5 | 107.4 | **107.6** | 108.5 |
| snapshot (13-node page, refs path) | 5 | 0.8 | **0.8** | 2.3 |
| click `#btn` (non-navigating) | 5 | 1039.5 | **1041.3** | 1042.9 |
| type `#name` | 5 | 0.4 | **0.4** | 1.0 |
| screenshot (viewport PNG; 20,379 B wire) | 5 | 31.6 | **32.7** | 42.5 |
| evaluate (`1+1`) | 5 | 0.3 | **0.3** | 0.5 |
| close tool | 7 | 186.1 | **220.7** | 258.5 |
| process exit after stdin EOF (exit code 0) | 7 | 2.0 | **2.6** | 3.6 |
| snapshot on 3,000-node page (truncation path; 7,960 B out) | 3 | 108.4 | **114.5** | 150.6 |

Notes (measured unless labelled):
- Two earlier cold-navigate samples under the identical method: 197.5 ms and 390.8 ms -> cold path is
  spawn/profile-variance dominated (range across all runs 175-391 ms).
- The 0.3-0.5 ms evaluate floor is the loopback CDP RTT: JSON framing and websocket plumbing are NOT
  the bottleneck at this scale.
- `click` = ~1.0 s even though the page does nothing: that is the settle watch at `src/tools.rs:519-549`
  (1 s of 50 ms polls on every click where the URL/readyState snapshot looks unchanged).
- `navigate` = ~107.6 ms while CDP RTT is 0.3 ms: that is the 100 ms load-poll cadence at
  `src/tools.rs:187-193`.
- Snapshot on the 3,000-node fixture is 140x the 13-node snapshot (114.5 vs 0.8 ms) - the cap/truncate
  loop cost scales with node count (see hot paths).
- **Unmeasured:** server RSS, browser-child peak RSS, memory growth over long sessions, real-site
  network variance, and anything behind a network (all ops here are local `file://`).
- :warning: **Re-benchmark warning:** the repo's own H2H run (`real-world\bench\H2H-RESULT.md`, 2026-10-10
  9:10 AM, mark `ses_eddb1a393ffeZSMGkF6653DKF2`: gladius 27/30 pass, 114,573 ms total vs cdt 47,268 ms
  and playwright 76,436 ms) predates the current binary (sources edited 5:23 PM same day). The current
  build adds the ~1 s per-click settle tax this survey measured. Any re-benchmark must run the current
  bin, and click-heavy tasks will look ~1 s/click worse until the settle is reworked.

## 4. Hot-path map

Each row is readable at the cited lines (verified by direct reads of `src\` during this recon).

| Hot path | Where | Why it is hot |
|---|---|---|
| Click settle watch (1 s) | `src/tools.rs:519-549` | Non-navigating click pays `1 s` of 50 ms `Runtime.evaluate` polls (`click` measured 1041.3 ms vs evaluate floor 0.3 ms). Single biggest latency defect in the surface. |
| Navigate load poll (100 ms cadence) | `src/tools.rs:187-193` | `readyState` poll sleeps 100 ms between checks; page is load-complete in ~1 ms on `file://`, so every navigate pays ~100 ms (measured warm navigate 107.6 ms). |
| Chrome spawn + DevTools-line wait + CDP connect | `src/browser.rs:760-846` (spawn 793-800, stderr line scan loop 816-842, `connect_async` 844-846) | The whole cold path: deadline-bounded stderr line scan for `ws://`, then websocket connect + `attach_to` + `Page.enable` (`browser.rs:415-434`). Cold navigate median 281.4 ms. |
| CDP send / routing | `src/browser.rs:354-386` (`send_raw`), `619-691` (`reader_loop`) | Every op builds a JSON envelope, takes the sink mutex, parks a `oneshot`, and the reader parses every frame with `serde_json` + locks the pending map. Sub-ms on loopback (evaluate 0.3 ms), so not a bottleneck now; it sets the ceiling for larger payloads. Dialog auto-accept does an inline send under the same sink lock (`browser.rs:685-688`). |
| Snapshot AX enumeration + cap loop | `src/tools.rs:254-400` (enumeration eval 260-303; cap loop 364-399) | Two in-page evals (querySelectorAll over candidates + 12 KB innerText slice) then a shrink loop that re-joins lines and recounts chars one line at a time. Measured 0.8 ms on 13 nodes, 108-151 ms on 3,000 nodes. |
| Close (taskkill.exe spawn) | `src/browser.rs:984-989` | `close` spawns a `taskkill /T /F` process; measured 186-259 ms per close even though the job object (`browser.rs:994-1116`) already guarantees tree kill at handle close. |
| Tab settle polls | `src/browser.rs:288-294` (close settle 50 ms), `572-617` (`settle_new_tab` 100 ms, 20 s cap) | Tab ops poll `Runtime.evaluate` at fixed cadence; contributes to any tabs action on slow pages. |
| Click presence poll (3 s cap) | `src/tools.rs:472-485` | Only the missing-element path; returns the same error at 3 s as it would at 10 s (measured in repo docs: H2H err-02/click-03). |
| Request/response framing | `src/mcp.rs:23-37` (parse), `88-95` (write+flush) | Per-line serde parse + flush; measured sub-ms at current payload sizes (largest wire object here: 20,379 B screenshot, 32.7 ms dominated by PNG capture, not serialization). |
| Memory growth | `src/browser.rs:73-76` (refs replaced per snapshot), `631-635` (pending entries removed), `440-464` (`tab_order` retains live ids only) | Static review finds no unbounded structure (refs table is replaced per snapshot; pending/cells cleaned). Growth itself **unmeasured** (no RSS sampling). |

## 5. Feature gaps vs current rivals

Gladius current surface (from `src/tools.rs:6-42`, authoritative): **13 tools** -
`navigate, snapshot, screenshot, click, type, evaluate, wait_for, fetch_text, hover, select, key, tabs, close`;
snapshot is a light-DOM AX enumeration with stable `@eN` refs (`tools.rs:254-400`); dialog auto-accept +
typed report (`browser.rs:637-689`, `tools.rs:570-579`); stale-ref guards (`tools.rs:226-247`).
(Note: `real-world\REFERENCE-FIELD.md`, 2026-10-10, still shows "no refs / no dialogs" - it predates
the refs+dialog work and is stale on the gladius column.)

Rivals read this pass (official docs, webfetch): Microsoft `@playwright/mcp`,
Google `chrome-devtools-mcp`, `browser-use`, `browsermcp.io`, plus Lightpanda and Browserbase.

| Feature | Who has it (evidence URL) | Gladius status |
|---|---|---|
| Network-idle / auto-wait on navigate+actions | Playwright MCP (`--timeout-settle`, `--timeout-navigation` - https://github.com/microsoft/playwright-mcp/blob/main/README.md); CDT (puppeteer auto-wait; `wait_for` tool - https://github.com/ChromeDevTools/chrome-devtools-mcp/blob/main/docs/tool-reference.md) | **LACKS** - fixed 100 ms `readyState` polling (`tools.rs:187-193`); no idle signal anywhere |
| Console capture | Playwright MCP `browser_console_messages` (README); CDT `list_console_messages` / `get_console_message` (tool-ref); browsermcp.io "Get Console Logs" (https://browsermcp.io) | **LACKS** - reader loop drops all non-dialog events (`browser.rs:619-691`) |
| Network capture / mocking / offline | Playwright MCP `browser_network_requests` + `--caps=network` (`browser_route`, offline) (README); CDT `list_network_requests` / `get_network_request` + `emulate` networkConditions (tool-ref) | **LACKS** - no Network domain usage at all |
| File upload | Playwright MCP `browser_file_upload` (README); CDT `upload_file` (tool-ref); browser-use (31 actions - https://github.com/browser-use/browser-use/blob/main/README.md) | **LACKS** - no file-chooser handling (`Input.*` use is mouse/keys only) |
| Download capture | Playwright MCP download/output-dir model (`--output-dir`, `--output-max-size`, README); browser-use cloud files (README) | **LACKS** - downloads land in the throwaway profile |
| Cookies / storage / session state | Playwright MCP `--caps=storage` + `--storage-state` + `--isolated` (README); CDT `new_page isolatedContext` (tool-ref); Browserbase contexts (`--contextId`, https://github.com/browserbase/mcp-server-browserbase/blob/main/README.md) | **LACKS** - throwaway profile per session (`browser.rs:903-936`); persistence only designed (`docs\HEADED-MODE.md`) |
| PDF export | Playwright MCP `--caps=pdf` (`browser_pdf_save`, README) | **LACKS** (explicit non-goal in `docs\ARCHITECTURE.md:88-93`) |
| Screenshot element / formats / quality | Playwright MCP `browser_take_screenshot` (`element`, `type`, `scale`, README); CDT `take_screenshot` (`uid`, `format`, `quality`, tool-ref) | **PARTIAL** - viewport/fullPage PNG only (`tools.rs:424-439`) |
| Viewport / device / geolocation / UA / media emulation | Playwright MCP `browser_resize`, `--device`, `--mobile`, `browser_emulate_media` (README); CDT `resize_page`, `emulate` (geolocation, userAgent, cpu/net throttle, tool-ref) | **PARTIAL** - the `viewport` tool sets `width`/`height`/`deviceScaleFactor`/`mobile` over `Emulation.setDeviceMetricsOverride` (with a `window.innerWidth`/`innerHeight` read-back, `reset` to clear, and a replay on every new tab). Measured engine default 1280x720 (read back in `tests/viewport.rs`, which records the baseline and requires it again after `reset`). Still LACKS device presets, user-agent, geolocation, and media emulation |
| Performance traces / Lighthouse / video | CDT `performance_start_trace`/`stop`/`analyze_insight`, `lighthouse_audit`, `screencast_start/stop`, heap-snapshot suite (tool-ref) | **LACKS** (beyond scope per `docs\ARCHITECTURE.md`) |
| Iframe / frame targeting | Playwright MCP refs across frames (README); CDT `pageId` + snapshot uids (tool-ref); steel/skyvern per `real-world\REFERENCE-FIELD.md` | **LACKS** - main frame only |
| Drag & drop | Playwright MCP `browser_drag`, `browser_drop` (README); CDT `drag` (tool-ref); browsermcp.io "Drag & Drop" | **LACKS** |
| Key combos / modifiers | Playwright MCP `browser_press_key` (`Control+A`, README); CDT `press_key` combos (tool-ref) | **PARTIAL** - single named keys only (`tools.rs:846-899`), no modifier grammar |
| Batch form fill | Playwright MCP `browser_fill_form` (README); CDT `fill_form` (tool-ref) | **LACKS** - per-field `type`/`select` only |
| Back / forward / reload / ignoreCache | Playwright MCP `browser_navigate_back` (README); CDT `navigate_page` type=back/forward/reload (tool-ref); browsermcp.io Go Back/Forward | **LACKS** - URL navigation only |
| Attach to an existing browser (CDP) | Playwright MCP `--cdp-endpoint` (README); CDT advanced usage "connect to a running Chrome"; browser-use remote CDP (README) | **LACKS** - designed, deferred (`docs\HEADED-MODE.md:62-66`) |
| HTTP/SSE transport | Playwright MCP `--port` (README); CDT WebSocket config (docs/configuration.md); Lightpanda `mcp --port` with per-session isolation (https://github.com/lightpanda-io/browser/blob/main/README.md) | **LACKS** - stdio only (`src/mcp.rs`) |
| Stealth / proxies / CAPTCHA | browser-use cloud (stealth, proxies - README); browsermcp.io (real profile fingerprint); Browserbase `--proxies`/`--verified` (README); Lightpanda proxy support (README) | **LACKS** (expected; local rivals also lack) |
| Snapshot scoping / search / file output | Playwright MCP `browser_find`, snapshot `depth`/`boxes`/`filename` (README); CDT `take_snapshot` `verbose` + filePath, pagination (tool-ref) | **PARTIAL** - always full-dump, 8,000-char cap + truncation footer (`tools.rs:356-399`) |
| Renderless ultra-light tier | Lightpanda (16x lighter / 9x faster vendor-claim, CDP + MCP, https://github.com/lightpanda-io/browser/blob/main/README.md) | **PLANNED** Tier-1 backend swap (`docs\ARCHITECTURE.md:31-38`); no native Windows binary today |
| Multi-tab depth | All four primary rivals ship tab list/new/select/close; gladius parity | **PARITY** - `tabs list/new/close/select` with stable indices (`browser.rs:178-330`) |
| Accessibility-ref actions | Playwright MCP refs, CDT uids, browsermcp.io snapshot refs; gladius `@eN` data-gladius-ref | **PARITY (current build)** - stable refs + typed stale-ref errors (`tools.rs:226-247, 441-505`) |
| Dialog handling | Playwright MCP `browser_handle_dialog` (manual); CDT `handle_dialog` (manual); gladius auto-accepts and reports | **AHEAD** - auto-accept means dialogs can never wedge a run (measured rationale at `browser.rs:637-641`) |

## 6. Ranked optimizations + first 3 picks

Ranking arithmetic: `score = impact(1-5) x confidence(1-5) / effort(1-5)`; every input is anchored to a
measured number or a cited hot path above.

| Rank | Optimization | Impact | Confidence | Effort | Score | Measured/projected basis |
|---|---|---|---|---|---|---|
| 1 | **Rework `click_settle` (tools.rs:519-549):** make the late-commit watch event-driven (in-page commit flag / MutationObserver / `location.href` change detected inside the click eval itself, or watch <= 250 ms only after dispatch) | 5 | 5 | 1 | **25.0** | click measured 1041.3 ms median; ~1,000 ms is pure watch. Projected click ~40-250 ms. |
| 2 | **Event-driven navigate load wait (tools.rs:187-193 + reader_loop browser.rs:619-691):** resolve on `Page.loadEventFired`/`readyState` event instead of the 100 ms sleep cadence | 4 | 4 | 2 | **8.0** | warm navigate 107.6 ms vs 0.3 ms RTT floor; projected ~10-25 ms. Shaves ~100 ms off every navigation incl. the cold cycle. |
| 3 | **Close without `taskkill.exe` (browser.rs:984-989):** use the already-open job handle (`TerminateJobObject`) or child-only kill; keep taskkill as fallback | 3 | 4 | 1.5 | **8.0** | close measured 186-259 ms; projected ~10-40 ms. Also removes a system-process spawn from every close. |
| 4 | **Snapshot single-pass cap (tools.rs:364-399) + optional line/selector scoping:** one characters-budget pass instead of iterative join+count | 3 | 4 | 3 | **4.0** | 3,000-node snapshot 108-151 ms vs 0.8 ms at 13 nodes; project large-page snapshot 2-4x faster. |
| 5 | **Trim fixed poll cadences in tab settle + click presence paths (browser.rs:288-294, 572-617; tools.rs:472-485):** 20-50 ms polls and early-exit on same-state | 2 | 4 | 1.5 | **5.3** | cadence costs are certain (structure), per-op payoff unmeasured (tabs were not in the timing matrix). |
| 6 | **Cold-path shave (browser.rs:760-846, 943-970):** `sweep_stale_profiles` walks `%TEMP%` on every launch; profile dir + spawn variance dominates the 175-333 ms cold spread. Keep sweep but make it lazy/rare or prefix-indexed. | 2 | 3 | 2 | **3.0** | cold navigate spread 175-391 ms across runs; sweep cost itself **unmeasured** (label: inferred from structure). |

**Provenance of scores:** impact and confidence derive from the measured medians in §3; effort is a
structural judgement (lines above are a single-function change each).

**Ship-first QOL/feature picks (ordered):**

1. **Settle rework (optimizations #1+#2 as one change-set).** Reasoning: click is the highest-frequency
   agent action and currently pays ~1 s each (measured); navigate pays ~100 ms each (measured). This is
   the only change here that shows up in *every* task and directly attacks the cost leg that sank the
   repo's own H2H (gladius 114.6 s vs cdt 47.3 s total latency). Expected payoff: ~0.8-1.0 s saved per
   non-navigating click and ~85 ms per navigate; on a 30-step task with 10 clicks that is ~10 s wall
   clock per task versus today's build.
2. **Console capture (`Runtime.enable` + ring buffer in `reader_loop`; expose last N with level/stack).**
   Reasoning: every primary rival ships it (Playwright `browser_console_messages`, CDT
   `list_console_messages`, browsermcp "Get Console Logs"); gladius's reader already consumes the same
   websocket event stream, so it is a small addition (`browser.rs:619-691` handles one event type today).
   Expected payoff: the first debugging signal for "why did my click do nothing" without a screenshot;
   near-zero latency risk.
3. **File upload (`DOM.setFileInputFiles` via `DOM.querySelector`, or `Page.setInterceptFileChooserDialog`
   + `DOM.setFileInputFiles`).** Reasoning: all four primary rivals expose upload (Playwright
   `browser_file_upload`, CDT `upload_file`, browser-use action set, browsermcp task docs) and it is a
   hard gate on whole task classes (forms, invoices, CVs) regardless of speed; gladius currently has no
   file-chooser path at all. Expected payoff: unblocks that task class; effort is contained to one new
   CDP domain pair behind the existing session plumbing.

Runner-up (deferred): network-idle waits as a first-class `wait_for` condition (feature form of
optimization #2); it duplicates much of #2's payoff and lands naturally with the same event-wiring work.

## 7. Caveats

- All timings are local loopback (`file://`); site network cost is excluded by construction.
- Cold-start samples show real variance (175-391 ms) - reproduce with n>=5 before quoting a single number.
- The repo's `docs\ARCHITECTURE.md` budget table ("9 tools", `<100 ms` initialize, `p50 < 200 ms`) is
  partially stale against the 13-tool build; initialize measured 24.5 ms median (within budget), steady
  state measured per §3.
- Everything unmeasured is labelled `unmeasured` in-line; nothing in this doc is estimated silently.

## Appendix - scratch and disposition

All scratch under `%TEMP%\opencode\ses_ed8bd551cffeHmdMpTecyGZT4W\` (marker `--drop--ses_ed8bd551cffeHmdMpTecyGZT4W`):

| Path | What | Disposition |
|---|---|---|
| `timing\driver2.py`, `timing\driver.py` | stdio JSON-RPC timing drivers | KEEP (fleet re-benchmark may reuse) |
| `timing\fixture.html`, `timing\big.html` | 13-node and 3,000-node file:// fixtures | KEEP (same reason) |
| `timing\results.json` | raw measured events + summary | KEEP (evidence) |
| `timing\stderr-*.log` | gladius stderr per phase (backend/profile/pid lines) | KEEP (evidence) |
| `timing\byte_diff.py` | provenance byte-diff tool | KEEP (evidence) |
| `build.log`, `freshbuild.log` | build transcripts + exit codes | KEEP (evidence) |
| `fresh-target\` (~330 MB) | isolated scratch cargo target (build proof) | DELETE after this doc - regenerable; hashes recorded above |

No chrome processes survive; no `gladius-<pid>-*` profile dirs remain in `%TEMP%`; the 18 long-lived
`gladius.exe mcp` processes seen during recon are owned by the OpenCode service (`opencode.exe serve`,
parent pid 16868) - not this session's and deliberately not touched.
