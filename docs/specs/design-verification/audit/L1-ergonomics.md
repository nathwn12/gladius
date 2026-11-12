# Lens 1 — Agent Ergonomics (gladius MCP, v0.7.1)

Repo: `Q:\PROJECTS\PERSONAL\gladius` · HEAD `abba9b1` (20-entry log checked, no build run).
Method: read `src/tools.rs` (tool registrations + handlers), `src/mcp.rs` (instructions, dispatch, locks), `docs/GUIDE.md`, `README.md`, `src/layout.rs` / `src/capture.rs` / `src/visual.rs` headers, `tests/schema_conformance.rs`. No writes, no `cargo build`.

## Q0 — Discoverability baseline (answers Q1–Q5 shared context)

- Single entry point exists: `instructions()` in `src/mcp.rs:53-54` says "Always start with snapshot to get stable @eN refs, then act with @eN refs or CSS selectors" plus the 25-tool typed-error contract. `README.md:34-44` repeats the capability table. `docs/GUIDE.md:142-288` documents all 25 with args/returns/example.
- Names are self-describing for ~18/25 (`navigate`, `snapshot`, `click`, `type`, `hover`, `select`, `key`, `upload`, `viewport`, `dialog`, `history`, `network`, `console`, `storage`, `visual`, `screenshot`, `close`, `fetch_text`). The 7 that are not: `targets` vs `tabs`, `layout` vs `assert_layout`, `evaluate`, `wait_for`, `cdp`.
- Cold-start cost before first useful act: an agent that obeys instructions reads `instructions` + `snapshot` schema and calls `snapshot` — 1 tool read + 1 call. An agent that does not trust instructions must scan all 25 `tools/list` entries.
- Full-schema context cost (estimated, method stated): the `list()` literal is `src/tools.rs:6-333`; that region is **35,139 bytes** by `sed | wc -c`, of which `description` strings alone are **~11 KB**. Serialized `tools/list` JSON is the same order (~30–35 KB, ~8–10k tokens at ~4 chars/token). The four longest descriptions are `visual` (`src/tools.rs:288`), `screenshot` (`src/tools.rs:37`), `assert_layout` (`src/tools.rs:247`), `snapshot` (`src/tools.rs:16`) — each 1–2 KB of prose the agent pays on every `tools/list`. Method limit: estimated from source bytes, not a live `tools/list` call (no build run); the binary may compact whitespace but not the prose.

---

## Ranked findings (frequency × cost)

### F1 — `tabs` vs `targets` forces two reads for one mental model ("where am I?")
- Failing scenario: agent managing two pages calls `tabs list`, sees no frame IDs, then must discover `targets list` separately to get the `frameId` that `snapshot`/`click`/`type` need.
- Evidence: `src/tools.rs:84-94` (`targets`: "List every CDP target … plus the active page's frame tree … or make a page target current"); `src/tools.rs:194-202` (`tabs`: "List, open, close, or select browser tabs (pages)"); handlers `tool_tabs` (`src/tools.rs:5433`) takes `index`, `tool_targets` (`src/tools.rs:5490`) takes `targetId` — two id spaces (numeric index vs stable target id) for the same pages. `docs/GUIDE.md:160-168` documents both with no "use X not Y" rule.
- Severity: **major** (hit on every multi-tab + iframe task).
- Smallest change: merge docs into one entry point — make `tabs list` also return `frames`, or add one line to each description cross-linking (`tabs` for pages by index, `targets` for CDP ids + frame tree).

### F2 — `layout` vs `assert_layout` vs `screenshot` vs `visual`: four geometry/pixel tools with no selection rule
- Failing scenario: agent asked "prove the hero is 64px tall and centred" must guess between `layout` (dump boxes), `assert_layout` (assert boxes), `screenshot` (pixels, no verdict), `visual` (baseline diff) — reads all four schemas (~6 KB of prose) before acting.
- Evidence: `src/tools.rs:246-272` (`assert_layout`: "Prove geometry without pixels … per-check {target, check, pass…}"); `src/tools.rs:273-286` (`layout`: "The where-is-it-actually primitive … every match … overlapping pair"); `src/tools.rs:36-51` (`screenshot` deterministic capture); `src/tools.rs:287-330` (`visual` save/compare). `README.md:39-40` lists them on one line with no decision rule; `docs/GUIDE.md:246-273` documents each but never says "dump → layout, verdict → assert_layout, pixels → screenshot/visual".
- Severity: **major** (hit on every "make this UI correct" task, the core job).
- Smallest change: one sentence in each description's first clause (`layout`: "dump, no verdict — use assert_layout for pass/fail"; `assert_layout`: "verdict, no pixels unless screenshotOnFail"; `visual`: "cross-run regression, not one-off geometry").

### F3 — No atomic form-fill verb; N-field form costs N+2 calls plus a pre-read
- Failing scenario: 5-field signup form needs `snapshot` + 5× `type` + `click` submit (or `key Enter`) — 7 calls, each a CDP round trip, with a `stale_ref` risk between field 1 and field 5 if the page re-renders mid-form.
- Evidence: only single-field writers exist: `type` (`src/tools.rs:59-67`, "Type text into an element … the seed is replaced, not appended"), `select` (`src/tools.rs:164-171`), `key` (`src/tools.rs:172-177`), `click` (`src/tools.rs:52-58`), `upload` (`src/tools.rs:142-150`). No batch/fill-form tool in the `grep -c '"name":'` = 25 inventory; `docs/GUIDE.md:216-244` shows only per-field examples.
- Severity: **major** (forms are the most frequent multi-act task).
- Smallest change: allow `type` to accept an array of `{element, text}` pairs applied in order, or add a thin `fill_form` wrapper over the existing `tool_type`/`tool_select`/`tool_click` path — no new engine work.

### F4 — No scroll primitive; scroll is a hidden side effect of click/hover
- Failing scenario: agent asked "scroll the modal list to the bottom and screenshot the last row" has no `scroll` tool — must abuse `click` (which calls `scrollIntoView` internally) or `evaluate("el.scrollTo…")` via the escape hatch.
- Evidence: `grep scroll` over `src/` hits only internals: `scrollIntoView` inside click/hover/type preflight probes (`src/tools.rs:2673,2680,2690,2694,2698,3833,3871,3895` — e.g. `tool_click` probe at `2673`), `scrollWidth/scrollHeight` reads in `layout.rs:338-348`, `layout`/`assert_layout` `noOverflow` checks (`src/layout.rs:812-828`). No `scroll`, `drag`, `wheel`, `touch`, `doubleClick`, `rightClick`, `focus`/`blur` tool in the 25-name inventory (`src/tools.rs:8-330`).
- Severity: **major** (every infinite-scroll / sticky-header / modal-overflow task).
- Smallest change: add `scroll` (`{element|ref, direction|xy, amount}`) reusing the existing resolver + `scrollIntoView`/`scrollTo` path; document that click/hover already auto-scroll.

### F5 — 3-breakpoint geometry assertion costs 8–10 calls; `viewport` and `assert_layout` cannot compose
- Failing scenario: "assert the hero is 64px tall and centred at 3 breakpoints" takes today: `navigate` (1) + `snapshot` (1) + `wait_for` (1) + per breakpoint `viewport` (1) + `assert_layout` (1) = **9 calls minimum**, 3 sequential viewport↔assert round trips, because `viewport` (`src/tools.rs:178-193`, `tool_viewport` at `src/tools.rs:5341`) and `assert_layout` (`src/tools.rs:246-272`) are separate tools with an execution-lock serialize between them (`needs_exec_lock`, `src/mcp.rs:122-158`), and `assert_layout` has no `viewport` param.
- Evidence: `viewport` schema `oneOf` 3 shapes (`src/tools.rs:184-188`); `assert_layout` accepts `ref|selector + checks` but no viewport/matrix (`src/tools.rs:248-267`); `wait_for` hard cap 15 s (`src/tools.rs:3632`, `GUIDE.md:251`).
- Severity: **major** (the canonical responsive-QA loop; cost multiplies by breakpoint count).
- Smallest change: accept an optional `viewport:{width,height}` inside `assert_layout` (applied + restored like `screenshot deterministic`), or a `matrix:[{width,checks}]` batch form — one call per breakpoint instead of two.

### F6 — `visual` and `storage` and `wait_for` are multi-tools hiding behind one name
- Failing scenario: agent must read the whole `visual` description (~2 KB, `src/tools.rs:288`) to learn `save` vs `compare` vs `list` vs `delete` have different required args and different lock behavior; same for `storage` get/set/clear/save/load (`src/tools.rs:220-238`, 8 optional args, all `additionalProperties:false`) and `wait_for` selector/text/ms/networkIdle/expression (`src/tools.rs:75-83`).
- Evidence: `visual` inputSchema `action: save|compare|list|delete` with `name?`, `threshold?`, `maskRects?`, `update?` (`src/tools.rs:289-304`); `tool_visual` gates render caps + name validation (`src/tools.rs:2511-2654`); `wait_for` enforces "one condition per call: expression never combines with selector/text" (`src/tools.rs:3634-3642`) and phases share one hard deadline (`src/tools.rs:3649-3653`); `needs_exec_lock` treats `visual` as always-locking but `wait_for` as lock-free (`src/mcp.rs:122-158`).
- Severity: **minor** (one extra read per session, then cached; cost is confusion, not round trips).
- Smallest change: split titles/descriptions per action in `tools/list` (keep one handler), or add `action`-specific `required` hints to the first sentence of each description.

### F7 — Error shape is typed and self-healing-ready, except the `error` fallback and two silent-knowledge gaps
- Failing scenario: agent can branch on `code`/`reason` for every common miss (bad selector → `invalid_target/not_found`, covered target → `not_actionable/occluded`, nav race → `stale_ref/navigated`, arg typo → `invalid_args/unknown_arg` via `check_unknown`), but a CDP/js-engine failure arrives as `{"code":"error"}` with only prose, forcing string parsing — and `navigate` never reports HTTP status while `evaluate(undefined)` is a tool error distinct from `null`, both learnable only from GUIDE prose.
- Evidence: typed envelope `ToolErr::typed` vs `fallback` (`src/tools.rs:353-396`); closed code set comment (`src/tools.rs:398-407`); `check_unknown` unknown-arg gate (`src/tools.rs:421-433`); `guard_ref` unknown/navigated (`src/tools.rs:1004-1032`), `stale_gone` node-gone (`src/tools.rs:1034-1040`); `wait_timeout` typed `timeout/budget_exceeded` (`src/tools.rs:3796-3800`); `tabs_err`/`targets_err`, click/type `not_actionable` arms (e.g. `src/tools.rs:2782-2861,3104-3256`); `target_changed/raced` on screenshot (`src/tools.rs:2351`); `capability_gated/dom_only` (`src/tools.rs:2315,2517`). Gaps: `From<String>` fallback is code-only (`src/tools.rs:386-396`); `tool_navigate` comment "no HTTP status … a hardcoded 200 lies on every 404" (`src/tools.rs:924-953`); `evaluate` "undefined is a tool error, distinct from null" (`docs/GUIDE.md:196`, handler `src/tools.rs:3299`).
- Severity: **minor** (the typed 90% already enables self-heal loops; the fallback is rare but forces a parse path).
- Smallest change: add `httpStatus` (nullable, from Network domain) to `navigate`'s outputSchema instead of omitting it; mint a distinct `code` for js-throw vs CDP-transport instead of bare `error`.

### F8 — Missing assert-class verbs for "make this UI correct": no a11y assertion, no contrast/typography metric, no CLS, no state matrix
- Failing scenario: agent can dump the AX tree (`snapshot`) and string-compare `computed` styles (`assert_layout` `computed` is "subset of getComputedStyle, string-exact", `src/tools.rs:247`), but cannot assert "no a11y violations", "contrast ≥ 4.5", "font is Inter 16/24", "CLS < 0.1", or "render these 4 component states and diff" without hand-rolled `evaluate` JS + external packages.
- Evidence: `assert_layout` checks are `exists|visible|stableFrames|box|hitTarget|computed|noOverflow|inViewport` — no `contrast`, `typography`, `a11y`, `cls`, `diff` (`src/tools.rs:248-267`, handler `src/tools.rs:2447`, checks impl `src/layout.rs:5-13,812-828`); `screenshot deterministic` freezes motion and waits fonts/images (`src/tools.rs:37`, `src/capture.rs` header) but records no CLS/trace; `docs/GUIDE.md:317-337` limitations list "No vision or coordinate grounding … No PDF generation" with no a11y/contrast/CLS mention — absence is structural, not an oversight in one tool.
- Severity: **minor** today (workaround via `evaluate` exists), **major** for any design-QA agent fleet.
- Smallest change: add one check at a time to `assert_layout` (`contrast:{ratio}`, reusing the existing `computed` read path) before any new tool; explicitly document "a11y audit lives in oc-rea / axe-via-evaluate, not here" if deferred (see F10).

### F9 — `snapshot` paging + `frame` + ref-main-frame-only rule is three paper cuts in the entry tool
- Failing scenario: first `snapshot` on a large page returns `truncated:true + snapshotId + nextOffset`; the agent must re-call with `offset + snapshotId` (same mode/frame) or get `snapshot_expired`; then acting inside an iframe requires a CSS selector, not the `@eN` ref just read, or gets `invalid_args/bad_value (frame)`.
- Evidence: paging contract `src/tools.rs:16-17` + `docs/GUIDE.md:188-191`; enforcement `offset>0 requires snapshotId` (`src/tools.rs:1356-1359`), mismatch → `snapshot_expired` (`src/tools.rs:1413-1416`), past-end → `bad_value` (`src/tools.rs:1428-1434`); cap 8000 chars (`src/tools.rs:1428`); ref-in-frame rejection `ref_in_frame` (`src/tools.rs:1142-1147`, "refs (@eN) are main-frame only; with frame= use a CSS selector"); `frame_id_ok` blank-frame refusal (`src/tools.rs:1151-1160`).
- Severity: **minor** (typed errors make each cut recoverable, but the entry tool should be the smoothest).
- Smallest change: accept `@eN` with `frame=` by resolving against the frame-scoped snapshot's table (the table already exists per `src/tools.rs:1137-1141` comment); auto-continue `snapshot` paging with a `followNext:true` convenience flag.

### F10 — Sibling overlap: gladius should defer OS/visible/fleet work to puppet, bulk forensics to oc-rea — but two gaps are invisible because the sibling covers them badly
- Failing scenario: agent picks gladius `visual compare` for cross-run regression and `network/console` ring buffers (caps 500/200, `src/tools.rs:114-141`) for traffic forensics, not knowing `oc-rea` (`compare_web_captures`, `capture_browser_scenario`, `inspect_web_page` — 229 passive-forensics tools on the same machine) and `oc-puppet` (`do` burst + `diff`, desktop screenshots) cover adjacent ground with different fidelity; conversely an agent needing a headed/visible check or an OS file-picker dialog tries gladius and gets `headed mode is not supported` / `dom-only` refusals with no pointer to the sibling.
- Evidence: gladius headless-only by owner decision, "no headed code path … visible path is an explicit attach" (`docs/GUIDE.md:317-324`, `HEADED-MODE.md`, `GLADIUS_HEADED` refusal `docs/GUIDE.md:103,309`); "No OS input, ever … no cursor, no keyboard injection, no window" (`docs/GUIDE.md:324`); `screenshot`/`visual` gated under `dom-only` (`src/tools.rs:2315,2517`, `src/mcp.rs:80-89` `gated` map); ring-buffer caps `network` 500 (`src/tools.rs:114-123`), `console` 200 (`src/tools.rs:132-141`); puppet surface verified present at `Q:\PROJECTS\PERSONAL\puppet\src` (`burst.rs`, `diff.rs`, `screen.rs`, `schemas.rs` listed 2026-11-12) — full tool-name parity not verified from source in this pass. oc-rea's 229-tool list taken from the task brief, not re-verified here.
- Severity: **nice** (routing, not correctness — but the wrong choice wastes a whole session).
- Smallest change: two pointers, no code — in `instructions()` (`src/mcp.rs:53-54`) and `GUIDE.md:317` limitations, add "headed/visible/OS-level → oc-puppet; bulk passive capture/forensics → oc-rea; gladius is the live owned headless page".

---

## What was not checked (do not mistake silence for a clean result)

- Live `tools/list` byte/token count (no `cargo build` per brief; size estimated from `src/tools.rs:6-333` source bytes).
- Full `oc-puppet` / `oc-rea` tool schemas (sibling paths listed, not schema-compared; oc-rea's 229-tool claim taken from the brief).
- Runtime behavior: navigation races, `wait_for` backoff cadence, `visual` AA-suppression quality, `screenshot deterministic` byte-identity — handler prose + `tests/` names only (`tests/ax.rs`, `refs.rs`, `frames.rs`, `concurrency.rs`, `schema_conformance.rs` present in `tests/` listing).
- `browser.rs` (246 KB) CDP internals beyond the signatures referenced from `tools.rs`.
- Whether `snapshot` refs survive `viewport` changes (assumed stable for the F5 call count; needs a live check).

## Adjacent notes (one line each, out of lens)

- `fetch_text` (plain HTTP, no browser, `src/fetch.rs:11`) vs `navigate+evaluate` is a footgun for SPA pages — GUIDE documents it (`docs/GUIDE.md:211`) but the `fetch_text` description ("Fast plain-HTTP GET", `src/tools.rs:151-153`) does not warn.
- `screenshot width/height require deterministic:true` is a typed refusal (`src/tools.rs:501-507`) — good fail-loud, but the schema allows the combination; a conditional `required` would prevent the call.
- `close` on an attached engine detaches (`docs/GUIDE.md:66-69`, `src/tools.rs:203-208`) — ergonomically surprising name for "detach"; fine once read, costly if guessed.
