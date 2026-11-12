# Lens 3 — Parity with how design/UI work is actually done in 2026
Repo: `Q:\PROJECTS\PERSONAL\gladius` (Rust, `gladius-mcp` 0.7.1). 25 tools (`src/tools.rs:6-7`, +3 inserted at `src/tools.rs:210-330`).
Method: source + `docs/GUIDE.md`, `docs/ARCHITECTURE.md`, `docs/OPT-SURVEY.md`, tool schemas, `git log --oneline -20` (HEAD `abba9b1`), no ROADMAP/TODO/issues file found in repo. No build, no edits.

## 1. Design → code (reference vs live page)

- Designer needs: compare a Figma export / screenshot reference against the built page with tolerance, overlay/onion-skin, per-region deltas, and typography/spacing measurement (family, size, weight, line-height, letter-spacing, spacing scale) — not by eye.
- Gladius gives: `visual save/compare` diffs a **live deterministic capture against a baseline gladius itself saved** (`src/visual.rs:367-396` save calls `capture::capture`; `src/visual.rs:964-1029` compare re-captures then `png_diff`). Returns `pass, diffPixels, totalPixels, ratio, bbox, maxChannelDelta, diffPath, environment{match,baseline,current}` (`src/tools.rs:287-330`; `docs/GUIDE.md:270-273`). Tuning: per-channel `threshold` default 0.1, `maxDiffPixels/maxDiffRatio` default 0/0, `maskRects[]`, AA rule (3×3 luminance ≥32 in BOTH images, component ≤ `minDiffCluster` default 2 — `src/visual.rs:23-43,486-501`), `baseline_environment_mismatch` warning / `requireEnvironmentMatch` gate. `assert_layout computed` does string-exact `getComputedStyle` subset checks (`src/layout.rs:784-810`); `layout` returns boxes/geometry only. Deterministic capture freezes motion, pins DSF=1, waits fonts/images + 2 rAF (`src/capture.rs:5-38,74,96-132,403-559`).
- Missing link: **no external reference input.** Schema has no `import baseline PNG / data-URL / Figma` path (`src/tools.rs:289-304` — only `save/compare/list/delete` by name; `save` always captures). So Figma→code fidelity cannot be proven — only live→live regression. No overlay/onion-skin output (only red/amber diff PNG, `src/visual.rs:711-740`), no per-region table (single `bbox`, no region list), no typography assertion (computed checks are caller-named string-exact one-offs, not a type-ramp diff).
- Scope: **in scope** (visual regression is the product's stated job; `docs/ARCHITECTURE.md:17,220-250`).
- Severity: **major** (blocker for Figma-gated shops; regression-only teams unaffected).
- Smallest change: `visual import {name, bytes|path}` writing PNG + minimal sidecar (env `known:false`), plus `compare` returning per-`maskRects` pixel counts (region table) — no new engine work.

## 2. Design-system conformance / drift detection

- Designer needs: 6 generated pages, one spacing scale / type ramp / palette / radius set — collect every distinct computed font-size/margin/color actually used, flag off-scale outliers.
- Gladius gives: **no inventory tool.** `layout` returns per-match `rect/box/scroll/computed{display,visibility,opacity}/zOrder/overlaps` (`src/layout.rs:899-1021`; schema `src/tools.rs:273-286`) — no font, color, spacing, radius fields. `assert_layout computed` checks a caller-supplied subset string-exact (`src/layout.rs:784-810`) — a gate for known values, not a census. `snapshot` is role/name/value/disabled AX tree + text (`src/tools.rs:15-35`), not styles. `evaluate` (arbitrary JS, realm-routed — `src/tools.rs:68-74`) *can* walk the DOM and return distinct `getComputedStyle` values in one call — so drift detection is **reachable but hand-rolled every time**, with no canonical script, no scale config, no off-scale verdict.
- Missing link: **the default tool.** Highest-value gap for "not slop": agents must invent the census query, the scale definition, and the outlier rule per session → inconsistent, uncomparable verdicts.
- Scope: **in scope, borderline** — "dumb primitives, anything fancier goes through `evaluate`" is doctrine (`docs/ARCHITECTURE.md:39-40`), but a style census is still engine measurement (same class as `layout`), not a design linter.
- Severity: **blocker** (the single capability that decides whether 6 pages look like one system).
- Smallest change: `audit_tokens {selector|ref, properties:[fontFamily,fontSize,fontWeight,lineHeight,letterSpacing,color,backgroundColor,margin,padding,gap,borderRadius], scale?:{...}}` returning distinct-value histogram + off-scale list; `evaluate` stays the escape hatch.

## 3a. States (default/hover/focus-visible/active/disabled/loading/error/empty)

- Designer needs: render and diff the full state matrix of every interactive component in one pass.
- Gladius gives: `hover` = one trusted `mouseMoved` (`src/tools.rs:158-163`); `click hoverFirst:true` adds one positioning move (`src/tools.rs:52-58`); focus emulation (`Emulation.setFocusEmulationEnabled`, `src/browser.rs:2387-2394`; `docs/ARCHITECTURE.md:191-196`) so `:focus` handlers behave; `snapshot` reports `disabled` (`src/tools.rs:22-26`); `assert_layout` proves `visible/hitTarget/computed/stableFrames` per target in **one call, many targets** (`src/tools.rs:246-272`; `src/layout.rs:549-897`). No `:focus-visible` / `:active` forcing, no state-forcing primitive, no matrix runner, no loading/error/empty fixtures.
- Missing link: **state orchestration.** Each state is a manual script (hover → screenshot → focus → screenshot …) — a call storm with no atomicity; `:focus-visible`, `:active`, forced-colors, and skeleton/error states have no first-class path. `stableFrames` (`src/layout.rs:729-760`, sampler `383-423`) proves *absence* of motion, not correct state appearance.
- Scope: **in scope** (browser-control: forcing pseudo-states via CDP is engine work, e.g. `CSS.forcePseudoState` through existing `cdp`).
- Severity: **major**.
- Smallest change: `assert_layout`/`layout` accept `pseudo: {hover,focus,focusVisible,active}` applied via CDP per target and reverted, or one documented `evaluate` recipe + `visual compare` per state; a `states: [...]` batch arg removes the storm.

## 3b. Responsive breakpoints (320/768/1024/1440 sweep + diff)

- Designer needs: sweep widths, diff each against expectation, catch overflow/breakage.
- Gladius gives: `viewport {width,height,DSF,mobile,reset}` with live read-back + `matched` flag, replayed on new tabs (`src/tools.rs:178-193`; `docs/GUIDE.md:171-174`); `visual compare` warns on viewport mismatch via sidecar env (`src/visual.rs:208-227,1048-1076`); `assert_layout noOverflow/inViewport` (`src/layout.rs:812-855`); `wait_for` max 15 s (`src/tools.rs:75-83`).
- Missing link: **no sweep primitive** — 4 widths × navigate/settle/capture/compare is ~16+ manual calls with no shared baseline-set convention (`<name>@320` naming is user invention). Practical to script, tedious and unstandardized; no overflow-per-breakpoint rollup.
- Scope: **in scope** (composition over existing tools).
- Severity: **minor** (scriptable today; pain is boilerplate, not impossibility).
- Smallest change: `visual compare_set {name, widths:[320,768,1024,1440]}` looping viewport→capture→compare and returning per-width pass/overflow — or bless a naming + script convention in GUIDE.

## 4. Accessibility as a gate

- Designer needs: computed-from-rendered-pixels contrast, focus order, focus visibility, ≥44px hit targets, reading order, alt-text presence, ARIA-state correctness, 200% zoom, reduced-motion honesty — as pass/fail, not dumps.
- Gladius gives: **inventories, almost no gates.** `snapshot` AX tree (role/name/value/disabled, hidden subtrees excluded — `src/tools.rs:15-35`; `docs/GUIDE.md:187-192`) + `layout` boxes/overlaps + `assert_layout hitTarget` (**occlusion** at centre point, `src/layout.rs:762-782** — not size**) + `visible` (box + display/visibility/opacity, `src/layout.rs:425-436**). Focus behavior emulated (`src/browser.rs:2387-2394`) but no `:focus-visible` assertion, no tab-order dump, no contrast math anywhere (grep: no `contrast|WCAG|axe` in `src/`), no target-size check, no zoom tool (viewport DSF override is capture-only), no alt-text/ARIA rule. Reduced-motion: capture *forces* `prefers-reduced-motion:reduce` + freeze stylesheet and restores afterwards (`src/capture.rs:440-559`) — honesty of the page's own reduced-motion path is never exercised (the freeze masks it). `oc-rea inspect_web_page` / `oc-puppet elements` are likewise inventories — noted, not gates.
- Missing link: **a gate that computes.** Contrast from *rendered* colors (not declared CSS), tab order from real focus traversal, hit-target size from boxes, focus-visible proof — none exist as pass/fail.
- Scope: **in scope for measurement, out of scope for full audit** — contrast-from-pixels and focus-order probing are engine measurements (gladius's job); a WCAG rule engine / axe-core bundling is a linter's job (cf. non-goals `docs/ARCHITECTURE.md:273-278`).
- Severity: **major** (blocker for any "accessible by default" claim; contrast alone fails most generated UIs).
- Smallest change: `audit_a11y {selector|ref}` returning per-element `{contrastRatio (computed from resolved used colors), targetSize{w,h}, hasAccessibleName, focusable, focusVisible}` with pass vs thresholds — axe-core stays out.

## 5. Performance-adjacent taste (CLS, font swap, jank, above-the-fold waste)

- Designer needs: layout-shift score, font-display swap flash, image decode jank, animation frame budget, unused above-the-fold work — enough to reject a sluggish-feeling page.
- Gladius gives: `network` ring (method/url/status/type/timing, cap 500 — `src/tools.rs:114-123`; entries `src/browser.rs:69-97`), `console` ring (cap 200 — `src/tools.rs:133-141`), `wait_for networkIdle` (`src/tools.rs:75-83`), and raw `cdp` passthrough for **any** CDP method (`src/tools.rs:124-131`; exercised `tests/cdp_raw.rs:129-233`). A real trace/measurement (e.g. `Tracing.start/collect`, `Performance.getMetrics`, `Page.getLayoutMetrics`, layout-shift via `LayoutShift` events) is **reachable** through `cdp` in principle. But: no streaming/trace-session ergonomics (one-shot request/response only), no first-class CLS/LCP/INP tool, no CrUX field data, no frame-budget assertion. `docs/OPT-SURVEY.md:143` explicitly marks "Performance traces / Lighthouse / video … **LACKS** (beyond scope per ARCHITECTURE)".
- Missing link: **practicality, not possibility.** An agent *can* drive Tracing via `cdp` but must own session attach, event buffering, trace parsing, and metric math — far beyond a reasonable agent loop; nothing returns "CLS=0.24 FAIL" in one call.
- Scope: **out of scope by design** (fast local-minimum browser control; perf/lab audits belong to CDT/Chromium tooling — non-goals `docs/ARCHITECTURE.md:273-278`, survey `OPT-SURVEY.md:143`).
- Severity: **minor** (for gladius; the need is real but belongs elsewhere — document the `cdp` recipe and stop).
- Smallest change (if ever): `perf_snapshot` returning `Performance.getMetrics` + cumulative layout-shift observed during a bounded window — explicitly a smoke signal, not Lighthouse.

## 6. Motion and interaction (transitions, easing, clips)

- Designer needs: capture a transition as frames, assert easing/duration, or record a short clip an agent (or owner) can inspect.
- Gladius gives: the opposite — deterministic capture **freezes** animation/transition/caret/scrollbars (`src/capture.rs:69-74` stylesheet; `SETTLE_JS` `src/capture.rs:77-94`), so motion is deliberately destroyed for byte-identical pixels. `stableFrames` asserts bbox identity across N rAFs (`src/layout.rs:383-423,729-760`) — motion *absence*. No screencast (`Page.startScreencast` unused in `src/`), no frame sequence, no duration/easing assertion, no video encode. Reachable only via hand-driven `cdp` screencast frames + manual assembly.
- Missing link: **motion is frozen, never judged.** A transition that is wrong-duration, wrong-easing, or janky passes every gladius check by construction.
- Scope: **mostly out of scope** (video/frame judging is a media pipeline, not browser control; survey marks screencast LACKS/beyond-scope — `docs/OPT-SURVEY.md:143`). A duration/easing assertion via rAF sampling would be in-scope measurement.
- Severity: **nice** (layout-correct + frozen covers most agent work; motion fidelity is polish-tier).
- Smallest change: `assert_motion {ref|selector, property, durationMs, easing?}` sampling computed values over rAF and returning measured duration/delta curve vs tolerance — no video, no frames stored.

---

## The three to build first

1. **Design-token drift inventory (`audit_tokens`)** — turns "do these 6 pages look like one system?" from an eye-test into a one-call census with off-scale outliers; nothing else kills generated-UI slop at the source.
2. **External reference import for `visual` (+ per-region deltas)** — unlocks Figma→code fidelity, the actual designer workflow; today `visual` can only regress against itself.
3. **Rendered-contrast + hit-target + focus-visibility gate (`audit_a11y` minimal)** — the only honest pass/fail quality bar agents can currently fake by quoting declared CSS; computed-from-rendered values make "accessible" provable.

## Explicitly out of scope (boundary, honestly marked)

- Full trace/Lighthouse/CrUX perf lab, screencast/video pipeline, multi-browser, headed operation, PDF, fleet/cloud, in-server planning/LLM — per `docs/ARCHITECTURE.md:273-278`, `docs/GUIDE.md:317-337`, `docs/OPT-SURVEY.md:143`, `docs/HEADED-MODE.md`. The `cdp` escape hatch (`src/tools.rs:124-131`) keeps these *reachable* without making them gladius's contract.

## Not checked

- Live behavior against the pinned `chrome-headless-shell` 155.0.8059.84 (read-only audit; no browser launched, no `cargo test`).
- `oc-rea inspect_web_page` / `oc-puppet elements` internals beyond their role as inventories (named in brief).
- Figma API / plugin surface (no reference-image pipeline exists to integrate with — confirmed absent, not evaluated).
