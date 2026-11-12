# SPEC — gladius 0.8.0, design verification

> Status: **frozen**. The contract below was settled before any code was written and
> is not reopened by an implementer. Where reality contradicts it, the spec is
> corrected by a new decision that names the item it supersedes — never by a quiet
> deviation in a diff.
>
> Baseline: `gladius-mcp` **0.7.1**, HEAD `abba9b1`. Every "today" statement in this
> file describes that tree.
>
> Evidence: the five read-only lens reports in [`audit/`](audit/) are the source of
> every claim. Each item cites `L<n>-<finding>` and carries the `file:line` those
> lenses recorded. Nothing in this spec is invented; where a lens was wrong, §11
> says so and names the winner.

---

## 1. How to resume this in a month

1. Read this section, then `PLAN.md` (order and effort), then `FREEZE.md` (what is
   in and out, and the SHA-256 of this file).
2. Pick the next unlanded item in workstream order. Read only that item and the
   `audit/` findings it cites.
3. The acceptance test named in the item is the definition of done. It must fail on
   `abba9b1` for the reason the item states, and pass after.
4. Do not re-derive the design. If an item looks wrong, that is a `FREEZE.md`
   amendment, not an implementation choice.

Vocabulary used below is the repo's own: *deterministic capture* (`screenshot`
`{deterministic:true}`), *scratch target* (the background CDP target the pixel diff
runs in), *sidecar* (`<name>.meta.json`), *exec lock* (`needs_exec_lock`),
*critical section* (the restore guard a cancel waits on).

---

## 2. Frozen decisions

These are the owner's rulings. They bind every item.

| ID | Decision |
|---|---|
| **D1** | **Truth over compatibility.** `visual compare` must never report a pass while pixels differ. Add `pixelsIdentical` (checksum equality), `ignoredPixels` (exists, now load-bearing), `maskedPixels`, `maskCoverageRatio`; rename the verdict to `passWithinTolerance` with `pass` retained as an alias whose meaning is documented exactly. `allowIgnoredAA` defaults to **false**; opting in is explicit. A mask covering more than 10 % of the diff area emits a warning. Semver **0.8.0**, breaking semantics documented here. |
| **D2** | **The claim follows the code.** Where the freeze cannot be complete (canvas, WebGL, video frames, cross-process iframes) the docs stop claiming determinism and say **sampled**. Where it can be fixed cheaply — WAAPI `getAnimations().pause()`, SVG SMIL, `video.pause()`, open shadow roots, same-process iframes — it is fixed. |
| **D3** | **`settled` is reported.** The deterministic capture reply carries `settled: bool` and `settleTimeouts: [...]`. A timeout stops being a stderr line and becomes a field. |
| **D4** | **Lifecycle is recoverable, not terminal.** A dead browser auto-respawns once and, if that fails, returns a typed error that names the recovery. Hard kill reaps the child process tree (pid file, sweep on next launch). Every tool gets a bounded timeout with typed `timeout`/`cancelled`. |
| **D5** | **gladius does not become an a11y engine or a headed browser.** Accessibility audits stay external (the `a11y` MCP owns occupancy); gladius adds only rendered-contrast, hit-target, focus-visibility and typography assertions inside `assert_layout`. Headed/OS work stays with `oc-puppet`; bulk forensics with `oc-rea`. §3 states the boundary as doctrine. |
| **D6** | **No new Rust dependencies for the new assertions.** Contrast, typography and token drift are computed from values already read via `getComputedStyle`. |
| **D7** | **Non-goals are named, not implied**: performance traces, video/clip recording, PDF export, cross-browser engines, cloud baseline storage, vision-coordinate grounding. |

---

## 3. Doctrine — what gladius is, and what it refuses to become

This section exists so the boundary stops being re-litigated as scope. It is quoted
from D5 and from the constraints the audit found already honoured in the repo.

- **gladius is one owned headless browser, measured, over CDP.** No headed path, no
  OS input, no visible window (`docs/GUIDE.md:317-324`, `HEADED-MODE.md`,
  `docs/ARCHITECTURE.md:273-278`). Nothing in 0.8.0 introduces one.
- **gladius is not an accessibility auditor.** No axe-core bundling, no WCAG rule
  engine, no accessible-name or ARIA-state verdicts. The four permitted assertion
  classes are rendered contrast, hit-target size, focus visibility, and typography
  (D5). An a11y verdict that needs a rule catalogue belongs to the `a11y` MCP, which
  owns that occupancy. `assert_layout` may *report* the values it measured; it must
  not grow a rule set.
- **gladius is not a desktop driver.** Headed/visible/OS-level work belongs to
  `oc-puppet`; bulk passive capture and forensic work belong to `oc-rea`
  (L1-F10). The routing pointer is written once in `instructions()` and once in
  `docs/GUIDE.md`, and that is the whole feature (S4.1).
- **Determinism is a claim with a scope.** The scope is: the main document, its
  same-process descendant frames, its **open** shadow roots, CSS animation and
  transition, WAAPI, SVG SMIL, and media playback — after 0.8.0. Outside that scope
  (canvas, WebGL, cross-process iframes, lazy content that needs a scroll to exist)
  the word is **sampled**, and the docs say so (S1.9).
- **No new dependencies** for measurement (D6). If a value cannot be read from
  `getComputedStyle`, `getBoundingClientRect`, or the AX tree, gladius reports
  `null` with a reason. It never estimates and it never infers.

---

## 4. Contract changes in one table

Every reply field added or renamed by this spec. "Tool" is the MCP tool; "when" is
the condition under which the field appears.

| Tool | Field | Type | Change | Item |
|---|---|---|---|---|
| `visual` (compare) | `pixelsIdentical` | bool | **new** — `checksum == baselineChecksum` | S1.1 |
| `visual` (compare) | `passWithinTolerance` | bool | **new name** for today's `pass` predicate | S1.1 |
| `visual` (compare) | `pass` | bool | **alias**, always equal to `passWithinTolerance`; documented as "within tolerance, never identical" | S1.1 |
| `visual` (compare) | `ignoredPixels` | int | exists (`src/visual.rs:1105`); now a pass condition by default | S1.1, S1.2 |
| `visual` (compare) | `maskedPixels` | int | **new** — pixels inside applied masks | S1.3 |
| `visual` (compare) | `maskCoverageRatio` | number | **new** — applied mask area ÷ image area, 0..1 | S1.3 |
| `visual` (compare) | `maskRectsApplied` | array | **new** — the rects actually used, after validation and clamping | S1.3 |
| `visual` (compare) | `settled` | bool | **new** — this capture settled | S1.4 |
| `visual` (compare) | `settleTimeouts` | string[] | **new** — closed set: `fonts`, `images`, `raf`, `settleCap`, `waapi`, `smil`, `media` | S1.4 |
| `visual` (compare) | `environment.match` | bool | widened — every new axis participates | S1.8 |
| `visual` (save, list) | `settled`, `settleTimeouts` | bool, string[] | **new** — recorded so a baseline cannot lie about its own capture | S1.4 |
| `visual` (compare, save) | `<name>.meta.json` | object | widened — new environment axes plus `settled` | S1.4, S1.8 |
| `screenshot` | `settled` | bool | **new** (deterministic only) | S1.4 |
| `screenshot` | `settleTimeouts` | string[] | **new** (deterministic only) | S1.4 |
| `screenshot` | `freeze` | object | **new** — `{waapiPaused, smilPaused, mediaPaused, shadowRoots, frames, skipped[]}` | S1.5 |
| `screenshot` | `freezeMode` | string | **new** — constant `"animation-none"` | S1.6 |
| `screenshot` | `fontsPending` | int | **new** — `FontFace`s still `loading` at capture time | S1.7 |
| `assert_layout` | `deterministic` | bool | **new arg** — measure under the freeze stylesheet and restore | S1.6 |
| `assert_layout` | `freezeApplied` | bool | **new** — echoes that the freeze was on | S1.6 |
| `navigate` | `httpStatus` | int\|null | **new** — measured main-frame response status, `null` when unobserved | S4.6 |
| `scroll` | *(new tool)* | — | **new** — `{element\|ref, dx, dy\|direction, amount, behavior}` | S4.3 |
| `fill_form` | *(new tool)* | — | **new** — ordered `{fields:[{element,value,kind}], submit?}` | S4.2 |
| `assert_layout` | `viewport` | object | **new arg** — apply, measure, restore | S4.4 |
| `assert_layout` | `matrix` | array | **new arg** — `[{width, height?, checks}]`, one call per breakpoint set | S4.4 |
| `assert_layout` | `checks.contrast` | object | **new check** — `{ratio, background?}` | S5.3 |
| `assert_layout` | `checks.typography` | object | **new check** — family/size/weight/line-height/letter-spacing, with tolerance | S5.4 |
| `assert_layout` | `checks.focusVisible` | bool | **new check** — focus the target and prove a visible focus indicator | S5.5 |
| `assert_layout` | `checks.targetSize` | object | **new check** — `{w, h}` minimum box, distinct from `hitTarget` | S5.6 |
| `visual` | `action:"import"` | — | **new action** — install an external reference PNG as a baseline | S5.2 |
| `visual` (compare) | `regions` | array | **new** — per-region `{x,y,w,h,diffPixels,ratio,bbox}` | S5.2 |
| `audit_tokens` | *(new tool)* | — | **new** — computed-style census plus off-scale outliers | S5.1 |
| any tool | `code` | string | **extended closed set** — `engine_dead`, `cancelled`, `shadow_closed`, `js_error` (S2.1, S2.3, S2.5, S3.3) | W2, W3 |

`additionalProperties:false` on every `outputSchema` means each new field must be
added to the schema in the same commit as the value: a field that is emitted but not
declared is a schema-conformance failure (`tests/schema_conformance.rs`).

### 4.1 Error-code additions

The closed set is documented at `src/tools.rs:398-412`. 0.8.0 adds exactly four
codes and one reason, and removes nothing:

| code | reasons | meaning | item |
|---|---|---|---|
| `engine_dead` | `respawn_failed`, `transport` | the owned engine died and the single automatic relaunch did not recover | S2.1 |
| `cancelled` | `client_cancelled`, `budget_exceeded` | the call was cancelled or ran past its total budget; the page was restored | S2.3 |
| `shadow_closed` | `unreachable` | the target lives in a **closed** shadow root and cannot be resolved from the page realm | S2.5 |
| `js_error` | `thrown`, `undefined_result` | the page's JavaScript threw, or returned `undefined` where a value was required | S3.3 |
| `invalid_args` | + `budget_mismatch` | a budget argument cannot be honoured (e.g. a per-tool budget above the server cap) | S2.3 |

`code:"error"` survives as the last resort for anything that genuinely knows nothing,
but transport death and JS throws stop falling into it (S3.3). Clients matching
`code == "error"` must be told: this is a breaking change for them (see §12).

---

## 5. W1 — Truth in the proof

Workstream problem: the checksum proves two captures went through the same encoder,
not that the page was stable; and the default `visual compare` can return `pass`
while pixels differ (L2-1).

### S1.1 — `pixelsIdentical` and `passWithinTolerance` on `visual compare`

**Closes** L2-1, L2-2.
**Today (fails):** the reply carries `pass` only (`src/visual.rs:1084-1087`,
`:1097`). `pass` is computed from `aa_tolerated(...) && !size_mismatch &&
diff_pixels <= max_diff_pixels && ratio <= max_diff_ratio`
(`src/visual.rs:1084-1087`), where `aa_tolerated` ignores subtracted AA pixels
(`src/visual.rs:949-951`) and the default `allow_ignored_aa` is `true`
(`src/visual.rs:487-501`, `src/tools.rs:2632-2635`). A 1 px edge change therefore
reports `pass:true` with `ignoredPixels:1` — proven by the selftest at
`src/selftest.rs:876-899` and by `tests/review_findings.rs:348-440`. There is no
field anywhere that states whether the two PNGs are identical.
**Behaviour:**
- `pixelsIdentical = (checksum == baselineChecksum)`. Both values already exist
  (`src/visual.rs:1111-1112`). It is a fact about the bytes, never a judgement.
- `passWithinTolerance` is the whole predicate above, unchanged in arithmetic:
  `aaTolerated && !sizeMismatch && diffPixels <= maxDiffPixels && ratio <= maxDiffRatio
  && (environment.match || !requireEnvironmentMatch)`.
- `pass` is an alias: `pass = passWithinTolerance`, always, byte-for-byte. It exists
  for clients written against 0.7.x. Its documented meaning is "the diff is within
  the requested tolerance", never "the images are the same". The tool description,
  `docs/GUIDE.md` and `docs/ARCHITECTURE.md` must say this in those words.
- No third verdict field. A reader who wants "is it identical" reads
  `pixelsIdentical`; a reader who wants "did it meet my bar" reads
  `passWithinTolerance`. Two questions, two booleans, no summary that can disagree
  with either.
**Acceptance — `tests/visual_truth.rs::compare_separates_identity_from_tolerance`:**
save a baseline; shift the fixture's text by 1 px horizontally; compare with default
arguments; assert `pixelsIdentical == false`, `passWithinTolerance == false`,
`pass == false`, `ignoredPixels > 0`. **Red on `abba9b1`** because `pixelsIdentical`
does not exist and `pass` is `true` (AA subtraction, L2-1).
**Touches** `src/visual.rs:1084-1130` (predicate + reply), `src/tools.rs:287-330`
(`visual` schema and description), `docs/GUIDE.md:266-274`,
`docs/ARCHITECTURE.md` (`visual` row and §6).
**Depends on** nothing. **Blocks** S1.2, S3.2.

### S1.2 — `allowIgnoredAA` defaults to false

**Closes** L2-1.
**Today (fails):** omitting `allowIgnoredAA` passes `true`
(`src/tools.rs:2632-2635`, default at `src/visual.rs:499`), so an AA-subtracted edge
change passes by default. The opt-in escape hatch exists and is tested
(`tests/review_findings.rs:424-440`) but is inverted: strictness is the opt-in.
**Behaviour:** omitting `allowIgnoredAA` means **strict** — `ignoredPixels > 0`
fails `passWithinTolerance`. `allowIgnoredAA:true` re-admits the AA rule, and when it
does, the reply's `warning` names the count and the clusters (already implemented,
`src/visual.rs:955-962`). The AA rule itself is unchanged (3×3 luminance ≥ 32 in both
images, component ≤ `minDiffCluster`) — this item changes only which side is the
default.
**Acceptance — `tests/visual_truth.rs::aa_suppression_is_opt_in`:** the
`review_findings.rs:353-440` fixture, compared with **no** `allowIgnoredAA` argument;
assert `ignoredPixels == 2`, `passWithinTolerance == false`, and `warning` contains
`allowIgnoredAA:false`. **Red on `abba9b1`** because that exact call returns
`pass:true` (L2-1, `src/visual.rs:949-951`).
**Touches** `src/tools.rs:2632-2635`, `src/visual.rs:487-501`, the `visual`
description's AA paragraph (`src/tools.rs:288`), `docs/GUIDE.md:268-271`.
**Depends on** S1.1 (the new verdict must exist before its default changes meaning).
**Blocks** nothing.

### S1.3 — Mask accounting: `maskedPixels`, `maskCoverageRatio`, rect validation

**Closes** L2-3.
**Today (fails):** masked pixels are excluded at the candidate stage
(`src/visual.rs:622-628, 631-644`), the diff image paints them amber
(`src/visual.rs:719-721`), and the JSON reply carries **no count** — the mask is an
unmetered hole with no area cap and no coordinate validation (L2-3). `maskRects` is
raw passthrough (`src/tools.rs:2613-2617`).
**Behaviour:**
- `maskRectsApplied` — each requested rect clipped to the image bounds and rounded to
  integers; rects that are degenerate after clipping are dropped and named in
  `warning` as `mask_rect_dropped`.
- `maskedPixels` — the number of pixels covered by at least one applied rect (union,
  not sum). Computed geometrically in the same harness pass that builds the mask.
- `maskCoverageRatio = maskedPixels / totalPixels`.
- When `maskCoverageRatio > 0.10`, append `mask_coverage: <pct>% of the diff area was
  masked; a regression inside a mask is invisible` to `warning`. It is a warning, not
  a failure: masking dynamic content is legitimate, unmetered masking is not.
- Coordinate space: **device pixels of the captured PNG**. Deterministic capture pins
  `deviceScaleFactor:1` (`src/capture.rs:433-439`), so device pixels coincide with CSS
  pixels; when the read-back `deviceScaleFactor` is not 1 and `maskRects` is non-empty,
  append `mask_scale_mismatch` to `warning` naming the ratio. Do not refuse — the
  counts are still true; the geometry is what is suspect.
**Acceptance — `tests/visual_truth.rs::mask_coverage_is_reported_and_warns`:** compare
on an 800×600 viewport with one mask rect covering 15 % of the area; assert
`maskedPixels == 72000 ± 1`, `maskCoverageRatio ≈ 0.15 ± 0.005`, `maskRectsApplied`
is an array of one rect equal to the input, and `warning` contains `mask_coverage`.
**Red on `abba9b1`** because none of the three fields exists (reply set at
`src/visual.rs:1097-1130`).
**Touches** the diff harness JS (`src/visual.rs:572-741`), the reply
(`src/visual.rs:1097-1130`), `src/tools.rs:2601-2617`, the schema at
`src/tools.rs:289-310`, `docs/GUIDE.md:268-273`.
**Depends on** S1.1 (same reply block). **Blocks** S3.2.

### S1.4 — `settled` and `settleTimeouts` are fields (D3)

**Closes** L2-7.
**Today (fails):** every settle wait is raced and every race resolves `true` anyway;
the only record of a give-up is `eprintln!` (`src/capture.rs:165-167, 476-481`), which
never reaches the MCP reply. The deterministic metadata block
(`src/capture.rs:136-160`) carries no settle signal, so an agent cannot distinguish
"stable" from "gave up waiting" (L2-7).
**Behaviour:**
- The settle script returns `{settled: bool, timeouts: string[]}` instead of `true`.
  Each race that loses appends its name: `fonts`, `images`, `raf`, and `settleCap`
  when the outer `SETTLE_CAP` (`src/capture.rs:47`) fires.
- `Capture` gains `settled: bool` and `settle_timeouts: Vec<String>`; `meta()` inserts
  both whenever `deterministic` is true (`src/capture.rs:136-160`).
- `settled == settle_timeouts.is_empty()`. A capture with a lost race still returns
  its PNG and its checksum — the pixels are real; only the *claim* changes.
- **The artifact carries its own honesty:** the sidecar records `settled` and
  `settleTimeouts` at `save` time. On `compare`, a baseline whose sidecar says
  `settled:false` appends `baseline_unsettled` to `warning` (and the same under
  `requireEnvironmentMatch:true`, where it is a typed
  `baseline_environment_unsettled` failure, because a strict gate must not pass on an
  unsettled baseline). A sidecar from 0.7.x has no `settled`: treat it as
  `unknown` and say so in `warning` — never as `true`.
**Acceptance — `tests/visual_truth.rs::settle_timeout_is_a_field`:** a loopback server
that accepts the connection and never answers serves one `<img>`; a deterministic
capture of that page returns `settled == false`, `settleTimeouts` contains `images`,
and a non-empty `image` part (the capture still succeeds). **Red on `abba9b1`** because
both fields are absent from the reply (L2-7).
**Touches** `src/capture.rs:77-94` (script), `:47` (cap), `:136-160` (meta),
`:476-481` (the warn site this replaces), `src/visual.rs:168-190` and `:208-226`
(sidecar write/compare), `src/tools.rs:36-51` and `:287-330` (both output schemas),
`docs/GUIDE.md:266-273`.
**Depends on** nothing. **Blocks** S1.7 (fonts report through it).

### S1.5 — The freeze reaches what moves (D2, fix-where-cheap)

**Closes** L2-5.
**Today (fails):** the freeze stylesheet is appended to `document.head` only
(`src/capture.rs:52-60`) and covers CSS `animation`/`transition`/`caret`/scrollbars
(`src/capture.rs:69-74`). WAAPI (`element.animate()`), SVG SMIL
(`<animateTransform>`), `<video autoplay>`, open shadow roots and iframes keep moving;
there is no `getAnimations().pause()`, no `pauseAnimations()`, no `video.pause()` and
no shadow/iframe piercing anywhere in `src/capture.rs` (L2-5). Reduced-motion
emulation is best-effort and only *requests* reduction
(`src/capture.rs:440-469`).
**Behaviour:** the freeze becomes a walk, applied before settling and undone after the
capture on every path (success, failure, cancel — it lives inside the existing
critical section):
1. Main document, each same-process descendant frame (`iframe.contentDocument`, guarded
   by try/catch), and each **open** shadow root (`element.shadowRoot`), recursively.
2. **WAAPI:** `document.getAnimations({subtree:true})` → record each animation's
   `playState`, then `pause()` all. On restore, `play()` only those that were
   `running`.
3. **SMIL:** every `<svg>` root's `pauseAnimations()`; restore with
   `unpauseAnimations()`.
4. **Media:** every `<video>`/`<audio>` — record `paused` and `currentTime`, call
   `pause()`, freeze `currentTime`. On restore, `play()` only where it was playing.
5. **Skips are recorded, never silent.** A cross-origin frame (the `contentDocument`
   access throws) and a closed shadow root are unreachable; both append to
   `freeze.skipped` (`"cross-origin-iframe"`, `"closed-shadow-root"`).
- The reply gains `freeze: {waapiPaused, smilPaused, mediaPaused, shadowRoots, frames,
  skipped[]}` with the counts actually touched.
- Reduced-motion emulation keeps its current best-effort behaviour and its
  `reducedMotionEmulated` field; the walk does not depend on it.
**Acceptance — `tests/deterministic_freeze.rs::freeze_reaches_shadow_waapi_smil_and_media`:**
a new fixture page (open shadow root containing a WAAPI-animated box, an SVG with
`<animateTransform>`, an autoplaying muted `<video>`, and a same-process iframe with a
CSS spinner) captured twice 500 ms apart; assert `pixelsIdentical == true`,
`freeze.waapiPaused >= 1`, `freeze.smilPaused >= 1`, `freeze.mediaPaused >= 1`,
`freeze.shadowRoots >= 1`, `freeze.frames >= 1`. **Red on `abba9b1`** because the
second capture differs (shadow/WAAPI/SMIL/video motion is outside the freeze) and the
`freeze` field does not exist (L2-5).
**Touches** `src/capture.rs:52-94` (injection becomes a walk), `:440-560` (restore
ordering), `:136-160` (meta), new fixture `tests/fixtures/freeze.html`.
**Depends on** nothing. **Blocks** S1.6, S1.9 (docs can only claim what this reaches).

### S1.6 — The freeze and the measurement agree

**Closes** L2-6.
**Today (fails):** `animation:none` resets a mid-flight transform to its untransformed
base box, so a deterministic capture does not show any state the user sees; and
`assert_layout` measures **without** the freeze, so the pixel proof and the geometry
proof disagree by construction (L2-6, fixture `tests/fixtures/layout.html:14-16`).
**Behaviour:**
- `assert_layout` accepts `deterministic: true`, applying the same freeze walk and
  stylesheet inside the existing critical section, restoring afterwards, and echoing
  `freezeApplied: true`. `computed`, `box`, `stableFrames`, `hitTarget`, `noOverflow`
  and `inViewport` are then measured in the same state the camera sees.
- `screenshot` reports `freezeMode: "animation-none"` — a constant that names the mode
  rather than leaving it implicit, and the one word that makes "geometry under freeze"
  searchable.
- The docs state the consequence plainly: **a deterministic capture measures the
  animation-off state.** `assert_layout {deterministic:true}` is the only way to
  compare geometry with that capture. `animation-play-state:paused` is deliberately
  **not** used as the default: a paused animation keeps a wall-clock-dependent frame,
  which is exactly the nondeterminism the freeze exists to remove.
**Acceptance — `tests/deterministic_freeze.rs::assert_layout_can_measure_the_frozen_state`:**
a fixture whose element carries `transform: translateX(200px)` applied by a
`step-end` animation of one hour's duration, so the live box is translated and the
animation-off box is the base box. Assert: plain `assert_layout` `box.x` is the
translated value; `assert_layout {deterministic:true}` `box.x` is the base value and
`freezeApplied == true`; a deterministic `screenshot` taken in the same call reports
`freezeMode:"animation-none"`. **Red on `abba9b1`** because `deterministic` is not an
accepted argument (typed `invalid_args`/`unknown_arg`, `src/tools.rs:421-433`) and
`freezeMode` does not exist (L2-6).
**Touches** `src/tools.rs:246-272` (schema), the `tool_assert_layout` arm
(`src/tools.rs:2447`), `src/layout.rs:5-13` (module doc), `src/capture.rs` (the freeze
walk becomes callable from `layout`), `src/mcp.rs:122-163` (`needs_exec_lock` and
`needs_critical_section` must treat `assert_layout{deterministic:true}` as locked),
`docs/GUIDE.md:246-259`.
**Depends on** S1.5 (one freeze implementation, not two).

### S1.7 — Font readiness is a status, not a race

**Closes** L2-4.
**Today (fails):** settle waits `Promise.race([document.fonts.ready, 3s])` and then two
rAF ticks (`src/capture.rs:77-94`). `fonts.ready` resolves when the *current* font
loads finish, not when a `font-display:swap` page has swapped; a timed-out race only
warns (`src/capture.rs:476-481`). Nothing checks `FontFace.status` and nothing
re-settles after a swap (L2-4).
**Behaviour:**
- Wait for `document.fonts.status === 'loaded'` **and** every `FontFace` in
  `document.fonts` to reach `loaded` or `error`, then one extra rAF.
- If any `FontFace` is still `loading` when the 3 s budget expires, record `fonts` in
  `settleTimeouts` (S1.4) and report `fontsPending` (count) — the capture proceeds.
- No scroll, no re-layout and no `font-display` rewriting: a page that swaps after the
  budget is reported, not repaired.
**Acceptance — `tests/visual_truth.rs::fonts_report_pending_not_silent`:** a fixture
with `@font-face` served by a loopback that delays 4 s; assert `fontsPending >= 1`,
`settleTimeouts` contains `fonts`, `settled == false`. A second fixture whose font
resolves in <100 ms asserts the clean path: `settled == true`, `fontsPending == 0`, and
two captures `pixelsIdentical`. **Red on `abba9b1`** because neither field exists and a
timed-out font wait is invisible in the reply (L2-4).
**Touches** `src/capture.rs:77-94`, `:136-160`, `docs/GUIDE.md:267`.
**Depends on** S1.4.

### S1.8 — The environment fingerprint covers the axes that actually differ

**Closes** L2-8.
**Today (fails):** `EnvMeta` records browser string, major, viewport, DSF, url,
checksum, createdAt (`src/visual.rs:168-190`) and `differences()` compares only major,
viewport and DSF (`src/visual.rs:208-226`); identity comes from `Browser.getVersion`'s
product string alone (`src/visual.rs:292-317`). Platform, full version, colour scheme,
forced colours, contrast preference and the installed-font set are invisible, so a
baseline saved on another machine matches and the warning never fires (L2-8).
**Behaviour:** `EnvMeta` gains, read live from the page and the engine, never assumed:
`browserFullVersion`, `platform` (`navigator.platform`), `userAgent`,
`colorScheme` (`matchMedia('(prefers-color-scheme: dark)')`),
`forcedColors`, `contrast` (`prefers-contrast`), `reducedMotion`,
`fontCountHash` (sorted `family|weight|style` of `document.fonts` joined and hashed
with the existing FNV-1a), `scrollbarWidth`
(`innerWidth - documentElement.clientWidth`).
Every one participates in `differences()`, and the mismatch warning names each axis.
A 0.7.x sidecar lacking these keys reports `environment.known:true` with
`unknownAxes: [...]` and a warning — never a silent match.
**Acceptance — `tests/visual_truth.rs::environment_fingerprint_names_every_axis`:**
save a baseline, hand-edit the sidecar's `platform` to a different value, compare;
assert `environment.match == false` and `warning` contains `platform`; then compare
unmodified and assert `environment.current` carries every field above, non-null.
**Red on `abba9b1`** because the fields do not exist and the edited `platform` changes
nothing (L2-8).
**Touches** `src/visual.rs:168-190`, `:208-226`, `:292-317`, the `save` reply
(`src/visual.rs:380-400`) and the compare `environment` block
(`src/visual.rs:1113-1122`), `docs/GUIDE.md:269`.
**Depends on** nothing.

### S1.9 — The claims match the code (D2, docs half)

**Closes** L2-5 (claim half), L2-9, L2-10; supports L5-1.
**Today (fails):** `src/tools.rs:37` says two deterministic captures "can be proven
byte-identical", `docs/GUIDE.md:267` says "provably identical",
`docs/ARCHITECTURE.md:14` says the same, and none of them scopes the claim. Below-the-
fold lazy content is captured unloaded and nothing says so (`src/capture.rs:320-326,
483-485`); full-page is `captureBeyondViewport:true` with no scroll-through (L2-9);
byte-identity is over PNG bytes from *this* encoder, so an attached
`GLADIUS_CDP_URL` engine with a different encoder can differ while pixels match
(L2-10).
**Behaviour — the exact edits:**
- `src/tools.rs:37`: replace the byte-identity sentence with: "deterministic pins the
  capture so two captures of the same page in the same engine produce the same PNG
  bytes; the scope of the freeze is the main document, same-process frames, open shadow
  roots, CSS animation, WAAPI, SMIL and media (see `freeze`). Canvas, WebGL, video
  *frames* and cross-process iframes are **sampled**, not frozen."
- `docs/GUIDE.md:266-274`: same rewrite, plus one sentence on lazy content — "no
  scrolling is performed: below-the-fold `loading=\"lazy\"` content is captured
  unloaded" — and one on the encoder — "byte-identity assumes gladius's own PNG
  encoder; an attached engine may encode differently."
- `docs/ARCHITECTURE.md:14` and the `screenshot` row: same. Add an explicit "sampled"
  boundary sentence to §7 non-goals.
- `instructions()` (`src/mcp.rs:53-54`) gains the one-line boundary: "deterministic
  captures are frozen for CSS/WAAPI/SMIL/media in the main document and same-process
  frames; canvas, WebGL and cross-process iframes are sampled."
- New `tests/claims.rs` asserts the mechanical half: no markdown file under `docs/`
  contains `provably identical` or `byte-identical` unless the same paragraph contains
  `encoder` or `same engine`; and `docs/GUIDE.md` contains the strings `sampled` and
  `no scrolling is performed`.
**Acceptance — `tests/claims.rs::docs_scope_the_determinism_claim`** (the first test in
that file, which S3.5 extends). **Red on `abba9b1`** because `docs/GUIDE.md:267`
contains "provably identical" with no qualification and no `sampled` boundary exists
(L2-9, L2-10, L5-1).
**Touches** `src/tools.rs:37`, `src/tools.rs:288` (visual description's determinism
sentence), `src/mcp.rs:53-54`, `docs/GUIDE.md:266-274` and `:317-337`,
`docs/ARCHITECTURE.md:14`, `:220-254`, `:273-278`.
**Depends on** S1.5, S1.6, S1.7, S1.8 (a claim cannot be written before the behaviour
it describes lands).

---

## 6. W2 — Lifecycle

Workstream problem: one dead engine, one wedged call or one hard kill leaves the server
unable to recover and unable to say so (L4-1, L4-2, L4-3).

### S2.1 — Death is recoverable, and says how

**Closes** L4-1.
**Today (fails):** the reader sets `dead=true` once (`src/browser.rs:3119-3122`);
`ensure_session` fail-fasts forever after (`src/browser.rs:2317-2323`) and every later
call returns `cdp: engine connection closed (reader exited)`. Recovery exists but is
undiscoverable: `close` takes the state to `None` (`src/tools.rs:810-818`,
`src/browser.rs:1360-1439`) and the next `shared()` relaunches
(`src/browser.rs:537-552`); no error string names it (L4-1).
**Behaviour:**
- On a transport-dead engine, the *owned* engine relaunches **once**, lazily, inside the
  failing call — the same `launch()` path `shared()` uses when `state` is `None`.
  Relaunch is attempted at most once per death; the flag is cleared only by a
  successful launch.
- If the relaunch succeeds, the original call is **not** silently retried: it returns
  the typed error for what it was doing, and a `warning`-class field on the next
  successful call is not required. Silent retry of a mutation is exactly what the
  single-shot discipline forbids (`src/browser.rs:2287-2316`).
- If the relaunch fails, return `code:"engine_dead"`, `reason:"respawn_failed"`, with
  prose that names the recovery: "the owned browser did not come back; call `close` and
  retry (the next call launches a fresh engine)". Transport death that is not an owned
  engine (attached `GLADIUS_CDP_URL`) returns `reason:"transport"` and says the engine
  is not gladius's to restart.
- `close` on a dead engine still succeeds (it already takes `None`).
**Acceptance — `tests/engine_recovery.rs::dead_engine_respawns_once_then_types_the_recovery`:**
navigate, then kill the owned `chrome-headless-shell.exe` tree from the test (WMI
lookup by parent pid, the `tests/hard_kill_reap.rs:117-176` pattern), then call
`snapshot`; assert it is **not** a transport prose failure but a typed
`engine_dead`/`respawn_failed` or a successful relaunch whose *following* `navigate`
works. **Red on `abba9b1`** because every call after the kill returns
`code:"error"` with `engine connection closed` and no relaunch happens (L4-1, L5-2).
**Touches** `src/browser.rs:2317-2323`, `:3119-3122`, `:537-552`; `src/mcp.rs` dispatch
surface for error mapping; `src/tools.rs:398-412` (code list);
`docs/GUIDE.md:294-300` (troubleshooting row).
**Depends on** nothing. **Blocks** S3.3 (the code this mints joins the closed set).

### S2.2 — A pid file, so a job-less launch can still be reaped

**Closes** L4-2 (the surviving half — see §11).
**Today (fails):** the Windows job object already reaps the owned Chromium tree on a
hard kill, and the next launch sweeps the stale profile — both asserted by
`tests/hard_kill_reap.rs:209-268`. What is **not** covered is the launch path where job
assignment failed: `close()` itself logs "no job cover (assignment failed at launch)"
(`src/browser.rs:1391-1405`), and in that state a hard kill leaves a live browser
holding its `gladius-<pid>-*` profile. `sweep_stale_profiles`
(`src/browser.rs:3737-3760`) then cannot delete the dir because a live process holds
its files, and it only ever deletes dirs — it never kills pids (L4-2).
**Behaviour:**
- The owned launch writes `%TEMP%/gladius-<pid>.json` containing
  `{pid, enginePid, profileDir, startedAt}` before readiness is declared, and removes it
  on graceful close.
- Launch-time recovery becomes **kill-then-sweep**: for each stale record whose owner
  pid is provably dead (`owner_alive` already exists, `src/browser.rs:3741+`), if the
  recorded `enginePid` is alive **and** its command line carries our
  `--user-data-dir=<recorded profile>`, terminate it (bounded, `kill_tree_bounded`
  `:3840`), then delete the dir. Any doubt — unknown command line, unreadable record,
  live owner — leaves both alone, exactly as today's sweep is conservative.
- The job object stays the primary mechanism. The pid file is the fallback authority,
  not a replacement; nothing about the job-cover path changes and
  `hard_kill_reap.rs` must keep passing untouched.
**Acceptance — `tests/hard_kill_reap.rs::pid_file_reaps_a_jobless_orphan`** (new test in
the existing file): launch, then from the test (a) drop the job cover's effect by
detaching the record's meaning — implemented as a test-only env
`GLADIUS_TEST_NO_JOB=1` that skips job assignment — (b) hard-kill the server, (c) start
a second server and assert the recorded `enginePid` is dead **and** the profile dir is
gone within 30 s. **Red on `abba9b1`** because no pid file is written, so the orphan and
its dir both survive (`sweep_stale_profiles` is dir-only).
**Touches** `src/browser.rs:3650-3667` (profile create), `:3737-3760` (sweep),
`:1380-1440` (close), `:3270-3290` (argv, for the command-line match).
**Depends on** nothing. Note: shares `src/browser.rs` with S2.1 — serialise them.

### S2.3 — One bounded budget per call, and a typed cancel

**Closes** L4-3, L4-4.
**Today (fails):** each CDP round-trip allows 30 s (`src/browser.rs:2209`), tools chain
many (`navigate` = enable + navigate + 10 s load wait + 2 evals,
`src/tools.rs:924-949`), and only `wait_for` has a hard budget (15 s,
`src/tools.rs:3632`). A stalled renderer turns one call into minutes **while holding
the exec lock** (`src/mcp.rs:122-163`), and the 4-slot semaphore
(`src/mcp.rs:23`) then answers `-32603 Server busy` to everything else (L4-3).
Cancelling a capture can take up to `CANCEL_GRACE` = 60 s before the abort lands
(`src/mcp.rs:185-190, 222-254`), so the cancelling agent waits in silence (L4-4).
**Behaviour:**
- Every `tools/call` runs inside a total budget: default **90 s**, overridable per call
  by an `_budgetMs` argument (2000..600000) — an unknown-argument gate exception that
  S3.3's taxonomy documents — and by `GLADIUS_TOOL_BUDGET_MS` for tests.
- Exceeding it returns `code:"timeout"`, `reason:"budget_exceeded"` with `tool` and
  `budgetMs`, and releases the exec lock. This is the existing `timeout` code, not a
  new one; the *guarantee* that no call exceeds a budget is what is new.
- A client `notifications/cancelled` answers **immediately** with
  `code:"cancelled"`, `reason:"client_cancelled"`; the restore still runs to completion
  in the detached waiter that already guards it (`src/mcp.rs:222-254`), which keeps
  `CANCEL_GRACE` as the backstop rather than the observed latency.
- The exec lock is not held across the settle phases where it can be avoided: the
  deterministic capture's mutation window keeps it, the settle wait does not.
- `needs_exec_lock` / `needs_critical_section` (`src/mcp.rs:122-197`) are unchanged in
  *which* tools lock; only the hold window shrinks.
**Acceptance — `tests/budgets.rs::a_wedged_call_returns_typed_timeout_and_frees_the_lock`:**
point the server at a page whose `Runtime.evaluate` never returns (a fixture that
blocks the main thread with a long loop), call `evaluate` on it, and in parallel call
`snapshot`; assert `evaluate` returns within `_budgetMs` with
`code:"timeout"`/`reason:"budget_exceeded"`, and that `snapshot` afterwards succeeds
(the lock was released). Second test,
`cancelled_call_answers_promptly`: send `notifications/cancelled` during a deterministic
capture and assert the response arrives with `code:"cancelled"` in under 2 s while the
page's viewport override is still restored afterwards. **Red on `abba9b1`** because no
budget exists (the call runs to the 30 s CDP timeout, multiplied by the phases) and a
cancel produces silence for up to 60 s (L4-3, L4-4).
**Touches** `src/mcp.rs:23`, `:122-197`, `:222-254`, `:571-630`; `src/tools.rs:398-412`
(code list) and every dispatch arm's `check_unknown` allow-list for `_budgetMs`.
**Depends on** nothing. **Blocks** S3.3.

### S2.4 — Baselines stop landing in the caller's repo and stop growing

**Closes** L4-5.
**Today (fails):** `visual_dir()` defaults to `<cwd>/.gladius/visual`
(`src/visual.rs:82-91`); `save`/`compare` write `<name>.png`, `<name>.meta.json` and a
`.diff.png` per mismatch with no cap, no LRU and no age-out — only manual `delete`
(L4-5).
**Behaviour:**
- Default moves to `%LOCALAPPDATA%/gladius/visual` on Windows, honouring
  `GLADIUS_VISUAL_DIR` exactly as today and falling back to the cwd default when
  `LOCALAPPDATA` is unset (the same fallback discipline as
  `src/browser.rs:3236-3240`).
- A cap of **500 entries** (PNG count): at the cap, `save` refuses with a typed
  `invalid_args`/`bad_value` naming `visual delete` / `GLADIUS_VISUAL_CAP`; it never
  evicts silently. `.diff.png` files are capped separately at 500 and are the only
  files eligible for automatic age-out (oldest first), because they are by-products.
- `list` reports `dir`, `count`, `cap`, `diffCount`.
**Acceptance — `tests/visual_truth.rs::baselines_do_not_default_into_the_cwd`:** with
`GLADIUS_VISUAL_DIR` unset and a temp cwd, `visual list` reports a `dir` under
`LOCALAPPDATA` and the temp cwd is empty; save 501 baselines and assert the 501st is a
typed refusal naming the cap. **Red on `abba9b1`** because the default is `<cwd>/.gladius/visual`
and no cap exists (L4-5).
**Touches** `src/visual.rs:82-91`, `:240-260` (write path), `:400-430` (`list`),
`src/tools.rs:287-330`.
**Depends on** W1 (shares `src/visual.rs`; land after S1.1-S1.8).

### S2.5 — Closed shadow roots get ground truth and a typed name

**Closes** L4-6.
**Today (fails):** selector probes run `document.querySelector` in the frame realm with
no piercing (hover probe `src/tools.rs:3662`; same shape for click/type/select), and
geometry resolves from `DOM.getDocument(pierce:false)` +
`querySelectorAll` (`src/layout.rs:136-151, 195-208`). Only the `@eN` path rides backend
node ids (`src/browser.rs:400-401`), which covers **open** shadow via the AX tree.
The word "closed" appears nowhere in `src/`; closed-shadow behaviour is untested and
unproven (L4-6).
**Behaviour:**
- First, a fixture with a closed shadow root establishes ground truth, and the answer is
  whatever the engine does — recorded in the selftest, not guessed.
- Where the target is unreachable, the failure is `code:"shadow_closed"`,
  `reason:"unreachable"`, prose naming the lever: "this element is inside a closed
  shadow root; the AX `@eN` ref from `snapshot` is the only path that can reach it".
- Where the engine *does* resolve a closed root through a backend node id (`@eN`), that
  path is documented as the supported one and covered by a test.
- No selector piercing is invented. A `>>>`-style syntax is explicitly rejected: it
  would promise what `querySelector` cannot do.
**Acceptance — `tests/shadow_closed.rs::closed_shadow_target_is_typed_not_not_found`:**
the fixture's closed-root inner button, addressed by selector, returns
`code:"shadow_closed"`; addressed by its `@eN` ref, it clicks (or returns the same typed
error if the engine proves unreachable — the test asserts *one* of the two and the
selftest records which). **Red on `abba9b1`** because a selector there returns
`invalid_target`/`not_found` with no mention of shadow DOM (L4-6).
**Touches** `src/tools.rs` resolver paths (click/type/hover/select probes),
`src/layout.rs:136-208`, `src/browser.rs:400-401`, new fixture
`tests/fixtures/shadow-closed.html`.
**Depends on** nothing.

---

## 7. W3 — Credibility tests

Workstream problem: the repo's headline claims are unasserted, so a silent regression
in the proof machinery goes red nowhere (L5-1, L5-2, L5-3).

### S3.1 — A checksum-equality test exists

**Closes** L5-1.
**Today (fails):** the word `checksum` appears **zero** times in `tests/*.rs`
(`findstr /i checksum tests\*.rs` exits 1 at `abba9b1`). `tests/concurrency.rs:775`
claims "the two captures are byte-identical" and then asserts geometry, DSF and restore
only (`:845-879`) — never the checksums (L5-1).
**Behaviour:** a test that captures the same page twice with `deterministic:true` and
asserts `checksum` equality, on a page with a CSS animation, a WAAPI animation and a
media element (so the assertion is about the freeze, not about a static page).
**Acceptance — `tests/deterministic_freeze.rs::two_deterministic_captures_have_equal_checksums`:**
two captures 750 ms apart; assert `c1.checksum == c2.checksum` and `c1.bytes == c2.bytes`.
**Red on `abba9b1`** only if the page includes motion outside the freeze — which is why
this test uses the S1.5 fixture and lands after it. The *coverage* gap it closes is the
missing assertion itself (L5-1).
**Touches** `tests/concurrency.rs:763-879` (comment corrected to match what it asserts),
new test in `tests/deterministic_freeze.rs`.
**Depends on** S1.5.

### S3.2 — The pass bar is tested

**Closes** L5 (§2), supports L2-1/L2-2/L2-3.
**Today (fails):** `findstr /i "threshold" tests\*.rs` and
`findstr /i "maxDiff" tests\*.rs` both exit 1 — zero tests exercise the pass bar. A
silently ignored `threshold`, `maxDiffPixels`, `maxDiffRatio` or `maskRects` would go red
nowhere (L5 §2).
**Behaviour:** four cases, each one assertion wide:
1. `threshold` flip — a uniform colour shift of 20 levels passes at `threshold:0.1` and
   fails at `threshold:0.05`.
2. `maxDiffPixels` — a diff of N pixels passes at `maxDiffPixels:N` and fails at `N-1`.
3. `maxDiffRatio` — same shape, ratio-based.
4. `maskRects` — a large real change entirely inside a mask yields
   `passWithinTolerance:true` **and** `maskCoverageRatio > 0` (the mask is reported, not
   hidden).
**Acceptance — `tests/visual_tolerance.rs`**, four tests named for the four cases.
**Red on `abba9b1`** because none exists; cases 1-3 also fail behaviourally if any
argument is dropped, which is the point.
**Touches** new `tests/visual_tolerance.rs`; fixture additions under `tests/fixtures/`.
**Depends on** S1.3 (mask fields), S1.2 (AA default).

### S3.3 — Transport and JS failures stop being `code:"error"`

**Closes** L5-2, L1-F7 (error-shape half).
**Today (fails):** `ToolErr::fallback` mints `{"code":"error"}` for everything untyped
(`src/tools.rs:386-396`), and `From<String>` collapses every engine string except the
`stale session: ` prefix into it (`src/tools.rs:386-396`). Codes are string literals at
call sites, so a typo compiles and only per-case envelope tests catch it (L5 §3). An
agent cannot branch "relaunch the browser" vs "page JS threw" vs "selector wrong" (L5-2,
L1-F7).
**Behaviour:**
- A **closed Rust enum** (`ToolCode` + `ToolReason`) in `src/tools.rs`, with the
  documented set as variants; every call site mints through it and `fallback` is
  restricted to sites that genuinely know nothing.
- Three reclassifications: engine transport death → `engine_dead` (S2.1); JS throws →
  `js_error`/`thrown`; `evaluate` returning `undefined` → `js_error`/`undefined_result`
  (docs already state the distinction at `docs/GUIDE.md:196`, handler
  `src/tools.rs:3299`).
- `code:"error"` remains for the residue, and a test asserts the residue is only
  reachable through `fallback` sites.
**Acceptance — `tests/envelope.rs::transport_and_js_failures_are_typed`:** three calls —
one against a killed engine, one `evaluate` of `throw new Error('x')`, one `evaluate` of
`undefined` — assert codes `engine_dead`, `js_error`/`thrown`, `js_error`/`undefined_result`
respectively, and none is `error`. **Red on `abba9b1`** because all three return
`code:"error"` (L5-2, L1-F7).
**Touches** `src/tools.rs:353-433` (the whole error type), every dispatch arm that mints
a code, `tests/envelope.rs`.
**Depends on** S2.1, S2.3, S2.5 (the codes they mint must exist first).

### S3.4 — The release names the commit it was built from

**Closes** L5-3.
**Today (fails):** nothing in-repo pins the Release bundle to a commit hash; HEAD is
already 5 commits past `v0.7.1`, so "which bytes is the user running?" is answerable only
by version-string convention (L5-3).
**Behaviour:**
- The release workflow writes `commit` (`git rev-parse HEAD`) and the `Cargo.lock`
  digest into the release body and into `dist/release-notes.md`, and
  `scripts/derive-release.mjs --check` gains a case asserting the body carries a
  40-hex commit line matching the tag's commit.
- `--doctor` prints the built commit when available, and the npm launcher's engine
  manifest records it, so a skew report names both sides.
**Acceptance — `scripts/derive-release.mjs --check`** gains fixture case
`body-carries-commit`; and `npx gladius-mcp --doctor` output is asserted (in
`tests/claims.rs`) to contain a commit line. **Red on `abba9b1`** because no commit line
exists in the body, the manifest or `--doctor` (L5-3).
**Touches** `scripts/derive-release.mjs`, `.github/workflows/*` (release lane),
`npm/bin/gladius-mcp.js` (`--doctor`), `npm/engine-manifest.json`, `build.rs`.
**Depends on** nothing. Note: this is the only item that touches the release lane; it
must not change the derivation rule (§12).

### S3.5 — The claims test covers the countable claims

**Closes** L5-1 (rot half).
**Today (fails):** `tests/headless_chromium.rs:6` still says "initialize lists 21 tools"
while the assertion below it says 25 (L5-1). Counts and claims are asserted in four
suites but the prose is not.
**Behaviour:** `tests/claims.rs` (created by S1.9) gains:
- every `docs/*.md` and `README.md` "N tools" statement equals the live
  `tools/list` length;
- every tool name mentioned in backticks in the README capability list exists in
  `tools/list`;
- no `docs/*.md` says `status` for `navigate` (it is `httpStatus`, S4.6);
- the stale comment at `tests/headless_chromium.rs:6` is corrected.
**Acceptance — `tests/claims.rs::countable_claims_match_the_binary`.** **Red on
`abba9b1`** because the `headless_chromium.rs:6` comment contradicts its own assertion and
no prose-count check exists (L5-1).
**Touches** new `tests/claims.rs`, `tests/headless_chromium.rs:6`, `README.md:44`,
`docs/GUIDE.md` (tool-count sentences).
**Depends on** S1.9 (same file), S4.6 (the `httpStatus` wording).

---

## 8. W4 — Agent ergonomics

Workstream problem: the agent pays extra reads and extra round trips to discover which of
four geometry tools to use, to fill a five-field form, to scroll, and to learn what a
navigation returned (L1-F1…F7, F9).

### S4.1 — The descriptions carry the decision rule

**Closes** L1-F1, L1-F2, L1-F6, L1-F10 (routing half).
**Today (fails):** `tabs` and `targets` are two id spaces for the same pages with no "use
X not Y" rule (`src/tools.rs:84-94`, `:194-202`; `docs/GUIDE.md:160-168`); four
geometry/pixel tools sit on one README line with no selection rule
(`README.md:39-40`, `docs/GUIDE.md:246-273`); `visual`, `storage` and `wait_for` hide
several actions behind one name (`src/tools.rs:288`, `:220-238`, `:75-83`); and nothing
routes headed/OS work to `oc-puppet` or forensics to `oc-rea` (L1-F10).
**Behaviour — text only, no handler change:**
- `tabs` first clause: "pages by numeric index — use `targets` for CDP ids and the frame
  tree". `targets` first clause: "CDP target ids and the active page's frame tree — use
  `tabs` for page index operations".
- `layout`: "dump, no verdict — use `assert_layout` for pass/fail". `assert_layout`:
  "verdict, no pixels unless `screenshotOnFail`". `screenshot`: "one capture, no
  verdict". `visual`: "cross-run regression against a stored baseline, not one-off
  geometry".
- `visual`, `storage`, `wait_for`: the first sentence names the required args per action.
- `instructions()` (`src/mcp.rs:53-54`) and `docs/GUIDE.md:317-337` gain: "headed or
  OS-level work → `oc-puppet`; bulk passive capture and forensics → `oc-rea`; gladius is
  the live owned headless page."
**Acceptance — `tests/claims.rs::descriptions_carry_the_decision_rule`:** assert each of
the nine description strings contains its rule phrase, and that `instructions()` contains
`oc-puppet` and `oc-rea`. **Red on `abba9b1`** because none of those phrases exists
(L1-F1, F2, F6, F10).
**Touches** `src/tools.rs:16-330` (descriptions only), `src/mcp.rs:53-54`,
`README.md:34-44`, `docs/GUIDE.md:160-168, 246-273, 317-337`.
**Depends on** S1.9 (same `instructions()` edit; serialise).

### S4.2 — `fill_form`: one call, N fields, in order

**Closes** L1-F3.
**Today (fails):** only single-field writers exist — `type` (`src/tools.rs:59-67`),
`select` (`:164-171`), `key` (`:172-177`), `click` (`:52-58`), `upload` (`:142-150`) —
so a five-field form costs `snapshot` + 5×`type` + submit, with a `stale_ref` risk
between fields if the page re-renders (L1-F3).
**Behaviour:**
- New tool `fill_form {fields: [{element, value, kind?}], submit?: bool|string,
  frame?: string}` where `kind` is `text` (default) | `select` | `check`.
- Fields are applied **in order**, each through the existing typed paths: `type` for
  text, `select` for `select`, `click` for `check` (only when the state differs).
- A failure stops the batch and reports `applied: n`, `failedAt: {index, element, code,
  reason}` — mutation is not rolled back, because undoing a half-filled form is worse
  than saying where it stopped.
- `submit` accepts `true` (press Enter on the last field), a CSS/`@eN` target to click,
  or is absent.
- Reply: `{ok, applied, failedAt, submitted, fields: [{index, element, kind, ok, code?}]}`.
**Acceptance — `tests/new_tools.rs::fill_form_applies_in_order_and_reports_the_first_failure`:**
a five-field fixture form; assert one call leaves all five values set, `applied == 5`,
`ok == true`; then a fixture whose third field is `disabled`; assert `ok == false`,
`applied == 2`, `failedAt.index == 2`, `failedAt.code == "not_actionable"`. **Red on
`abba9b1`** because the tool does not exist (`invalid_args`/`unknown_tool`, L1-F3).
**Touches** `src/tools.rs` (list literal + dispatch arm), `src/mcp.rs:122-163`
(exec-locked, mutating), `docs/GUIDE.md` (Acts section), `tests/schema_conformance.rs`
(one case per tool, completeness assertion).
**Depends on** nothing. **Blocks** nothing. Note: tool #26/#27 — the schema-conformance
tripwire and the `tools/list` count in `tests/*` all move together (L5-1).

### S4.3 — `scroll`: motion as a first-class verb

**Closes** L1-F4.
**Today (fails):** there is no `scroll`, `drag`, `wheel`, `doubleClick`, `rightClick` or
`focus`/`blur` tool in the 25-name inventory (`src/tools.rs:8-330`); scrolling is a
hidden side effect of `scrollIntoView` inside click/hover/type preflight probes
(`src/tools.rs:2673, 2680, 2690, 2694, 2698, 3833, 3871, 3895`) and reading
`scrollWidth/scrollHeight` (`src/layout.rs:338-348`). An agent told to scroll a modal list
must abuse `click` or `evaluate` (L1-F4).
**Behaviour:**
- `scroll {element?: string, ref?: string, dx?: number, dy?: number, direction?:
  "top"|"bottom"|"left"|"right", amount?: number, container?: "element"|"page",
  behavior?: "instant"|"smooth"}` — exactly one of `{element|ref}` or `container:"page"`;
  `direction` and `dy` are mutually exclusive.
- Defaults: `container:"element"` when a target is given, `"page"` otherwise;
  `behavior:"instant"` (a smooth scroll is a race against the next capture, so opting in
  is explicit).
- Reply: `{ok, scrolled: {x, y}, max: {x, y}, atEnd: bool}` read back from the page after
  the scroll — never computed from the arguments.
- Waits for one rAF after the scroll so a following `screenshot` is not mid-scroll.
**Acceptance — `tests/new_tools.rs::scroll_moves_a_container_and_reports_readback`:** a
fixture with a 400 px tall list inside a 200 px container; scroll `direction:"bottom"`;
assert `scrolled.y == max.y`, `atEnd == true`, and a following `assert_layout` on the last
row passes `inViewport`. **Red on `abba9b1`** because the tool does not exist (L1-F4).
**Touches** `src/tools.rs` (list + arm, reusing the existing resolver and
`scrollIntoView` path), `src/mcp.rs:122-163` (mutating, exec-locked),
`tests/schema_conformance.rs`, `docs/GUIDE.md`.
**Depends on** nothing.

### S4.4 — `assert_layout` composes with viewport and with a breakpoint matrix

**Closes** L1-F5.
**Today (fails):** `viewport` (`src/tools.rs:178-193`, handler `:5341`) and
`assert_layout` (`:246-272`) are separate tools with an execution-lock serialise between
them (`src/mcp.rs:122-158`), and `assert_layout` has no viewport or matrix parameter — so
a three-breakpoint assertion is 9 calls minimum, 3 of them viewport↔assert round trips
(L1-F5).
**Behaviour:**
- `assert_layout` accepts `viewport: {width, height, deviceScaleFactor?, mobile?}`,
  applied before measurement and **restored** afterwards through the same
  mutation-and-restore discipline the deterministic capture uses (it already holds the
  exec lock and the critical section, `src/mcp.rs:122-197`). The reply echoes
  `viewportReadback: {width, height, deviceScaleFactor, matched}` — `matched:false` when
  the engine ignored the override, exactly like the `viewport` tool.
- `assert_layout` accepts `matrix: [{width, height?, checks}]`, applied in order; the
  reply gains `matrix: [{width, pass, checks, viewportReadback}]`. `pass` at top level is
  the conjunction. `checks` at top level and `matrix` are mutually exclusive (typed
  `invalid_args`).
- Viewport changes invalidate `@eN` refs? No — refs are backend node ids and survive
  layout changes; the spec states this and the test proves it rather than assuming it
  (L1's "not checked" list flagged it).
**Acceptance — `tests/new_tools.rs::assert_layout_matrix_restores_and_repeats`:** one
call with `matrix:[{width:320},{width:768},{width:1024}]` asserting the same element's
`box.h`; assert three entries each with `viewportReadback.matched == true`, top-level
`pass`, and the pre-existing viewport override unchanged afterwards. **Red on `abba9b1`**
because both arguments are rejected as `invalid_args`/`unknown_arg` (L1-F5).
**Touches** `src/tools.rs:246-272`, the `tool_assert_layout` arm (`:2447`),
`src/layout.rs`, `src/capture.rs` (shared metrics-apply/restore helper), `src/mcp.rs`
(lock policy for the new argument shapes).
**Depends on** S1.6 (both mutate metrics/freeze through one helper).

### S4.5 — Refs resolve inside frames

**Closes** L1-F9 (refs-in-frames half only — see §13).
**Today (fails):** `ref_in_frame` rejects any `@eN` when `frame=` is set, with a typed
`invalid_args`/`bad_value`, even though a frame-scoped snapshot stamps its own ref table
(`src/tools.rs:1134-1160`). The agent reads a ref from a frame snapshot and then cannot
use it (L1-F9).
**Behaviour:**
- The per-frame snapshot table becomes addressable: `@eN` resolves against the snapshot
  table of the frame named in `frame=`, and against the main document's table when
  `frame` is absent. A ref from a *different* frame's table is a typed
  `stale_ref`/`unknown` naming the frame mismatch — never a main-frame action.
- `ref_in_frame` disappears as a refusal and survives as a resolver branch. The blank
  `frame` refusal (`frame_id_ok`, `src/tools.rs:1151-1160`) is unchanged.
- Applies to `click`, `type`, `hover`, `select`, `upload`, `layout`, `assert_layout` —
  every tool that already takes both `element`/`ref` and `frame`.
**Acceptance — `tests/frames.rs::ref_from_frame_snapshot_acts_in_that_frame`:**
`snapshot {frame:<iframe>}` then `click {element:"@e3", frame:<iframe>}`; assert `ok` and
that a handler in the iframe observed the click; then the same ref with a different
`frame` asserts `stale_ref`. **Red on `abba9b1`** because the first call returns
`invalid_args`/`bad_value` with prose "refs (@eN) are main-frame only" (L1-F9).
**Touches** `src/tools.rs:1134-1160` and the ref resolution branches of the five
handlers, `src/browser.rs` (session/realm routing already supports frames),
`docs/GUIDE.md:187-192`, `:216-244`.
**Depends on** nothing.

### S4.6 — `navigate` reports the status it actually saw

**Closes** L1-F7 (navigate half).
**Today (fails):** `tool_navigate` states plainly "no HTTP status is reported … a
hardcoded 200 lies on every 404" and returns `url` + `title` only
(`src/tools.rs:924-953`); `tests/navigate_status.rs:141-160` enforces the absence of a
fabricated `status` (L1-F7).
**Behaviour:**
- `navigate` enables the `Network` domain for the navigation only, records the status of
  the **main-frame** response (`Network.responseReceived` where `type == "Document"` and
  the request id matches the navigation), and reports `httpStatus: int|null`.
- `httpStatus` is `null` when no response was observed (a `file:` URL, a data URL, a
  cache hit with no response event, or a network failure) — never a guess, never 200.
- The field is named `httpStatus`, **not** `status`: the existing test forbids a
  fabricated `status`, and that prohibition stands.
- A non-2xx status is **not** an error by default — the page loaded is the fact, the
  status is the context. `navigate` gains no new failure mode.
**Acceptance — `tests/navigate_status.rs::navigate_reports_the_observed_http_status`:**
the existing 404 loopback server; assert `httpStatus == 404`; and a `file:` URL assert
`httpStatus == null`. The existing `navigate_does_not_fabricate_status` test stays green
untouched. **Red on `abba9b1`** because `httpStatus` is absent from the reply
(L1-F7).
**Touches** `src/tools.rs:924-953`, the `navigate` schema (`src/tools.rs` list entry),
`tests/navigate_status.rs`, `docs/GUIDE.md` (navigate section), `docs/ARCHITECTURE.md`
(navigate row).
**Depends on** nothing. **Blocks** S3.5 (the claims test asserts the wording).

---

## 9. W5 — Design parity

Workstream problem: gladius can regress a live page against itself, but cannot census a
design system, cannot compare against a designer's reference, and cannot gate the four
quality bars an agent is most likely to fake (L3-1, L3-2, L3-4).

### S5.1 — `audit_tokens`: the computed-style census

**Closes** L3-2.
**Today (fails):** no inventory tool exists. `layout` returns rect/box/scroll/computed
`{display,visibility,opacity}`/z-order/overlaps (`src/layout.rs:899-1021`, schema
`src/tools.rs:273-286`) — no font, colour, spacing or radius. `assert_layout computed`
checks a caller-supplied subset string-exact (`src/layout.rs:784-810`) — a gate for known
values, not a census. Drift detection is reachable through `evaluate` but hand-rolled
every session with no canonical script, no scale and no off-scale verdict (L3-2).
**Behaviour:**
- New tool `audit_tokens {root?: string|string[], properties?: string[],
  scale?: {fontSize?: number[], spacing?: number[], radius?: number[], color?:
  string[]}, maxValues?: number}`.
- Scope: the main document, every same-process frame and every **open** shadow root
  reachable by the S1.5 walk (the walk is reused, so the census sees what the freeze
  sees). `root` limits it to a subtree.
- For each requested property (default: `fontFamily, fontSize, fontWeight, lineHeight,
  letterSpacing, color, backgroundColor, margin*, padding*, gap, borderRadius`), return
  the distinct **computed** values with the element count and one example
  `path` (a stable CSS-ish ancestry string, not an AX ref — refs are snapshot-scoped).
- `scale` makes it a verdict: a value is off-scale when it is not within **0.5 px**
  (`fontSize`, `spacing`, `radius`) or is not an exact member (`color`) of the supplied
  scale. Reply gains `offScale: [{property, value, count, nearest}]`.
- No linter, no opinion: no "should be" values, no severity ranking. The caller supplies
  the scale or gets a census.
**Acceptance — `tests/audit_tokens.rs::census_finds_the_off_scale_outlier`:** a fixture
whose six elements use `font-size: 16px` and one uses `17px`, with
`scale:{fontSize:[16]}`; assert `fontSize` histogram has two entries with counts 6 and 1,
`offScale` has exactly one entry `{property:"fontSize", value:"17px", nearest:"16px"}`, and
`elementsScanned == 7`. **Red on `abba9b1`** because the tool does not exist (L3-2).
**Touches** new `src/tokens.rs` (or a `layout.rs` module — implementer's call, one file),
`src/tools.rs` (list + arm), `src/mcp.rs:122-163` (read-only, exec-lock-free),
`tests/schema_conformance.rs`, `docs/GUIDE.md`, `README.md:34-44` (tool list).
**Depends on** S1.5 (the walk). **Blocks** nothing.

### S5.2 — `visual import` and per-region deltas

**Closes** L3-1.
**Today (fails):** `visual` has `save|compare|list|delete` only, and `save` always
captures (`src/tools.rs:289-304`, `src/visual.rs:367-396`). There is no external
reference path, so Figma→code fidelity cannot be proven — only live→live regression —
and `compare` returns a single `bbox` with no region table (L3-1).
**Behaviour:**
- New action `visual {action:"import", name, path|bytes}`: copies an external PNG into
  the baseline dir and writes a sidecar with `environment.known:false`,
  `source:"imported"`, the image's real dimensions, and the file's SHA-256. A
  pre-existing baseline requires `overwrite:true`; names keep the existing sanitiser.
- `compare` on an imported baseline works unchanged (the diff is dimensional: a
  size mismatch is already reported as `sizeMismatch`, which must now be `true` and
  explained rather than silently scaled).
- New optional argument `regions: [{x,y,w,h,name?}]` on `compare`; the reply gains
  `regions: [{x,y,w,h,name?,diffPixels,ratio,bbox}]`, computed in the same harness pass
  as the global diff. Regions are **in addition to** masks, not a replacement: a masked
  region still counts toward `maskedPixels` (S1.3) and reports `diffPixels:0`.
- No overlay/onion-skin output in this release. The diff PNG remains red/amber. **Why:**
  an overlay is a second encoded artifact and a second thing that can lie; the per-region
  numbers are the machine-readable form of the same information.
**Acceptance — `tests/visual_truth.rs::imported_reference_compares_with_per_region_deltas`:**
import a reference PNG generated from the fixture (800×600), change one 100×50 region of
the page, compare with two `regions` (one covering the change, one not); assert
`pixelsIdentical == false`, `regions[0].diffPixels == 5000 ± 1`, `regions[1].diffPixels
== 0`, and `environment.known == false` with a `baseline_environment_unknown` warning.
**Red on `abba9b1`** because `action:"import"` is an unknown action typed
`invalid_args`/`unknown_action` (L3-1).
**Touches** `src/visual.rs:360-400` (save/import), `:964-1030` (compare),
`:572-741` (region accumulator in the harness), `src/tools.rs:287-330`, `docs/GUIDE.md`,
`docs/ARCHITECTURE.md`.
**Depends on** S1.1, S1.3 (same reply block and the same accounting pass).

### S5.3 — Rendered contrast inside `assert_layout`

**Closes** L3-4 (contrast), L1-F8 (measurement half).
**Today (fails):** there is no contrast math anywhere in `src/` (no `contrast|WCAG|axe`
match), and `assert_layout`'s check set is
`exists|visible|stableFrames|box|hitTarget|computed|noOverflow|inViewport`
(`src/tools.rs:248-267`, `src/layout.rs:5-13, 812-828`). `computed` is string-exact, so
"contrast ≥ 4.5" is a hand-rolled `evaluate` per session (L3-4, L1-F8).
**Behaviour:**
- New check `contrast: {ratio: number, largeText?: bool}` on `assert_layout`.
- The ratio is computed from **resolved used colours**: the element's computed `color`
  and the nearest ancestor with a non-transparent `background-color`, with alpha
  composited down the chain to the opaque backdrop, using the sRGB luminance formula
  (`0.2126 R + 0.7152 G + 0.0722 B` on linearised channels) — WCAG 2.x relative
  luminance and the `(L1+0.05)/(L2+0.05)` ratio. `largeText` is a caller claim, not an
  inference: 0.8.0 does not guess a font size threshold.
- **Refusals, not guesses.** If the resolved backdrop is not a flat colour — a
  `background-image`, a gradient, `backdrop-filter`, or a transparent chain that reaches
  the canvas without a paint — the check returns
  `pass:false, actual:{contrast:null, reason:"background-not-flat"}` and the overall
  `pass` is `false`. A null is never a pass.
- Per-check record shape is unchanged: `{target, check, pass, expected, actual, delta}`;
  `actual` carries `{ratio, fg, bg, reason?}`.
**Acceptance — `tests/a11y_gates.rs::contrast_is_computed_from_used_colours`:** four
fixtures — 21:1 (pass at 7), 4.6:1 (pass at 4.5, fail at 7), 2.1:1 (fail at 4.5), and one
with `background-image:linear-gradient(...)` asserting `actual.ratio == null`,
`actual.reason == "background-not-flat"`, `pass == false`. **Red on `abba9b1`** because
`contrast` is not an accepted check (`invalid_args`/`unknown_arg`, L3-4).
**Touches** `src/tools.rs:248-267`, `src/layout.rs:5-13` and the check impl block,
`docs/GUIDE.md:246-259`, `docs/ARCHITECTURE.md` §6.
**Depends on** S1.6 (same measurement surface). **Blocks** nothing.

### S5.4 — Typography assertion inside `assert_layout`

**Closes** L3-4 (typography).
**Today (fails):** typography can only be asserted through `computed`, which is
string-exact (`src/layout.rs:784-810`), so `font-size: 16px` vs `16.00px` vs a
`1rem`-resolved `16px` is a caller's problem, and there is no type-ramp diff (L3-4).
**Behaviour:**
- New check `typography: {family?: string|string[], size?: number, weight?: number,
  lineHeight?: number|"normal", letterSpacing?: number, tolerance?: number}`.
- Values are read from `getComputedStyle` and **normalised** before comparison:
  `fontSize`/`lineHeight`/`letterSpacing` to px floats (`normal` reported as `"normal"`,
  never coerced), `fontWeight` to a number (`bold` → 700, `normal` → 400), `fontFamily`
  to its first family, unquoted and case-folded.
- Numeric comparisons use `tolerance` (default **0.5 px**); family/weight are exact
  after normalisation.
- `actual` reports every normalised value whether it passed or not.
**Acceptance — `tests/a11y_gates.rs::typography_normalises_units_and_names`:** a fixture
with `font: 700 1rem/1.5 Inter, sans-serif` asserts
`{family:"inter", size:16, weight:700, lineHeight:24}` passes at defaults; the same with
`size:17` fails with `delta:1`; a `font-family` quoting difference (`"Inter"` vs `Inter`)
passes. **Red on `abba9b1`** because `typography` is not an accepted check (L3-4).
**Touches** `src/tools.rs:248-267`, `src/layout.rs` check impl, `docs/GUIDE.md:246-259`.
**Depends on** S5.3 (same normalisation helpers).

### S5.5 — Focus visibility inside `assert_layout`

**Closes** L3-4 (focus visibility).
**Today (fails):** focus emulation is enabled (`Emulation.setFocusEmulationEnabled`,
`src/browser.rs:2387-2394`) so `:focus` handlers behave, but there is no
`:focus-visible` forcing, no indicator measurement and no assertion — `assert_layout`
cannot say whether a keyboard user can see where they are (L3-4).
**Behaviour:**
- New check `focusVisible: true` on `assert_layout`.
- Procedure per target: record the previously focused element; `focus()` the target via
  the trusted path; read `:focus-visible` state (`element.matches(':focus-visible')`) and
  the computed `outline-*`, `box-shadow`, `border-*` deltas against the unfocused
  snapshot; restore the previous focus afterwards.
- Pass requires both: `matches(':focus-visible') == true` **and** at least one visual
  delta (outline width/colour/style, box-shadow, or border colour/width) that is not
  `none`/`0px`/`transparent`.
- `actual` reports `{focusVisible, outline, boxShadow, borderDelta}` so a failure names
  what was missing.
- This is a visibility proof, **not** a tab-order or focus-trap audit (D5 boundary).
**Acceptance — `tests/a11y_gates.rs::focus_visibility_is_proven_from_computed_style`:**
three fixtures — a button with `:focus-visible{outline:2px solid}` (pass), one with
`outline:none` and no other delta (fail), one with `:focus{outline:...}` only so
`:focus-visible` does not match a mouse-originated focus (fail). Assert the pass/fail
triple and that the previously focused element is focused again afterwards. **Red on
`abba9b1`** because `focusVisible` is not an accepted check (L3-4).
**Touches** `src/tools.rs:248-267`, `src/layout.rs` check impl, `src/browser.rs` (no new
CDP domain — `CSS.forcePseudoState` is **not** used: it forces `:focus` state without a
real focus, which would make the proof false), `docs/GUIDE.md:246-259`.
**Depends on** S5.3.

### S5.6 — Hit-target size inside `assert_layout`

**Closes** L3-4 (hit targets, ≥ 44 px).
**Today (fails):** `assert_layout hitTarget` proves **occlusion** — that
`elementFromPoint` at the centre is the target or a descendant (`src/layout.rs:762-782`) —
not size. Nothing measures how big a target is, and `visible` reports a box without a
minimum (L3-4).
**Behaviour:**
- New check `targetSize: {w?: number, h?: number, tolerance?: number}` (defaults
  `44/44`, tolerance 0.5) measured from the target's **viewport rect**, not the padding
  box, so the number matches what a finger can hit.
- The existing `hitTarget` check is **not** redefined. Its description gains the clause
  "occlusion only — use `targetSize` for size", so the two cannot be confused.
- `actual` reports `{width, height, rect}`.
**Acceptance — `tests/a11y_gates.rs::target_size_is_separate_from_occlusion`:**
a 24×24 button that is fully visible and unoccluded: `hitTarget:true` **and**
`targetSize:{w:44,h:44}` false with `actual:{width:24,height:24}`; a 48×48 button passes
both; a 48×48 button covered by an overlay fails `hitTarget` and passes `targetSize`.
**Red on `abba9b1`** because `targetSize` is not an accepted check and `hitTarget` returns
true for the 24 px button (L3-4).
**Touches** `src/tools.rs:246-272` (schema + description), `src/layout.rs:762-782`
(adjacent check impl), `docs/GUIDE.md:246-259`, `docs/ARCHITECTURE.md` (assert_layout row).
**Depends on** nothing.

---

## 10. Reply shapes, written out

The three replies a reviewing agent reads most. `?` marks a field that can be `null`.

```jsonc
// visual compare (0.8.0)
{
  "ok": true, "action": "compare", "name": "home", "updated": false,
  "pixelsIdentical": false,          // NEW  checksum == baselineChecksum
  "passWithinTolerance": false,      // NEW  the whole predicate
  "pass": false,                     // ALIAS of passWithinTolerance, always equal
  "diffPixels": 142, "totalPixels": 480000, "ratio": 0.000296,
  "bbox": {"x": 120, "y": 64, "w": 300, "h": 40},
  "maxChannelDelta": 255,
  "ignoredPixels": 0, "ignoredClusters": 0,          // now a pass condition by default
  "maskedPixels": 72000, "maskCoverageRatio": 0.15,  // NEW
  "maskRectsApplied": [{"x": 0, "y": 0, "w": 800, "h": 90}],  // NEW
  "settled": true, "settleTimeouts": [],             // NEW
  "sizeMismatch": false, "size": {"width": 800, "height": 600},
  "baselineChecksum": "…", "checksum": "…",
  "viewport": {"width": 800, "height": 600}, "deviceScaleFactor": 1,
  "url": "http://127.0.0.1:…/",
  "environment": {"match": true, "known": true, "baseline": {…}, "current": {…}},
  "regions": [{"x":0,"y":0,"w":800,"h":90,"name":"header","diffPixels":0,"ratio":0,"bbox":null}],
  "warning": "mask_coverage: 15.0% of the diff area was masked; …",  // ?
  "scratchBackground": true, "baselinePath": "…", "diffPath": "…", "metaPath": "…"
}

// screenshot deterministic (0.8.0)
{
  "mimeType": "image/png", "bytes": 91234, "deterministic": true,
  "viewport": {"width": 800, "height": 600}, "deviceScaleFactor": 1,
  "url": "http://…", "checksum": "…", "reducedMotionEmulated": false,
  "settled": true, "settleTimeouts": [],                 // NEW
  "fontsPending": 0,                                     // NEW
  "freezeMode": "animation-none",                        // NEW
  "freeze": {"waapiPaused": 2, "smilPaused": 1, "mediaPaused": 0,
             "shadowRoots": 3, "frames": 1, "skipped": []} // NEW
}

// assert_layout (0.8.0), one target, one new check
{
  "pass": false,
  "freezeApplied": true,                                  // NEW (deterministic:true)
  "viewportReadback": {"width": 320, "height": 640, "deviceScaleFactor": 1, "matched": true}, // NEW
  "checks": [
    {"target": "#save", "check": "targetSize", "pass": false,
     "expected": {"w": 44, "h": 44}, "actual": {"width": 24, "height": 24}, "delta": null},
    {"target": "#save", "check": "contrast", "pass": true,
     "expected": {"ratio": 4.5}, "actual": {"ratio": 7.1, "fg": "rgb(17,17,17)", "bg": "rgb(255,255,255)"},
     "delta": null}
  ],
  "targets": […],
  "matrix": [{"width": 320, "pass": false, "checks": […], "viewportReadback": {…}}]  // NEW, when matrix is used
}
```

---

## 11. Where the lenses disagree

Two lenses each saw part of the truth; these are the rulings. Every one is decided, and
the loser is named.

| # | Disagreement | Ruling | Why |
|---|---|---|---|
| 1 | **L4-2** says a hard kill orphans the owned Chromium and the next launch only deletes dirs, so a zombie accumulates per kill. `tests/hard_kill_reap.rs:209-268` asserts the Windows job object reaps the tree and the next launch sweeps the stale profile. | **The test wins.** The job object is the primary reaper and that path is already proven. L4-2 survives only for the launch path where job assignment failed — a path `close()` itself logs ("no job cover (assignment failed at launch)", `src/browser.rs:1391-1405`). S2.2 closes that path with a pid file and kill-then-sweep; it does not replace the job object. | A lens reading `kill_tree` call sites cannot see the OS job object; a test that watches process ids can. Believing the lens would have re-implemented a working mechanism. |
| 2 | **L3-4** proposes a new `audit_a11y {selector}` tool returning contrast, target size, accessible name, focusable and focus-visible per element. **D5** forbids a new a11y tool: only four assertion classes, inside `assert_layout`. | **D5 wins.** S5.3–S5.6 add checks, not a tool. `hasAccessibleName` and `focusable` are **not** built — that is the a11y MCP's occupancy. | The boundary is the owner's ruling and the audit's own §4 note agrees a WCAG rule engine is a linter's job. The four measurements survive because they are engine measurements with a pass/fail bar. |
| 3 | **L3-4** asks for contrast "computed-from-rendered-pixels". **D6** forbids a new dependency, and pixel sampling cannot be done from `getComputedStyle`. | **D6 wins.** Contrast is computed from resolved used colours (S5.3), and a non-flat backdrop returns `contrast:null` with `background-not-flat` rather than a sampled guess. | Pixel-space contrast needs the PNG decoded and a compositing model; that is a new dependency and a second source of truth. A null with a reason is honest; a sampled average is not. |
| 4 | **L1-F7** asks `navigate` to report `httpStatus`. `tests/navigate_status.rs:141-160` forbids a `status` field. | **Both stand.** They are different fields with different provenance: `status` (forbidden) would be a constant 200; `httpStatus` (S4.6) is measured from `Network.responseReceived` and is `null` when unobserved. | The existing test is right about the thing it tests. Renaming and measuring satisfies it instead of deleting it. |
| 5 | **L2-3** places `maskRects` in device pixels; **L2-10** reports a live DSF read-back that can be ≠ 1 on an attached engine. | **L2-3 wins on the coordinate space** (device pixels of the captured PNG), with an added `mask_scale_mismatch` warning when the read-back DSF is not 1 and masks are present. No refusal. | Deterministic capture pins DSF=1 (`src/capture.rs:433-439`), so the two spaces coincide in the supported path; the mismatch case is warnable and the counts stay true either way. |
| 6 | **L4-1** treats a dead engine as terminal; **L5-2** treats the same failure as an untyped-code problem. | **Both are real and neither alone is enough.** S2.1 owns the respawn and mints `engine_dead`; S3.3 owns the taxonomy and makes sure a transport failure never lands in `error`. S3.3 depends on S2.1. | Fixing only the string leaves the agent retrying a corpse; fixing only the respawn leaves "relaunch the browser" vs "page JS threw" indistinguishable. |
| 7 | **L1-F8** calls contrast/typography gaps "minor today, major for a design-QA fleet"; **L3-4** calls them "blocker". | **L3-4 wins on severity** for this release, because the audit's purpose is design-work parity. The *shape* stays inside `assert_layout`, so the "major" does not become a new subsystem. | The severity that matters is the one attached to the work being done; the work is design verification. |

---

## 12. Semver and migration

**0.8.0.** The repo derives versions from history
(`scripts/derive-release.mjs:29`: on 0.x a breaking change moves the minor), so 0.7.1 plus
breaking semantics is exactly 0.8.0 — the number is not a preference.

Breaking, in the order a user will feel them:

1. **A compare can now fail where it passed.** `allowIgnoredAA` defaults to `false`
   (S1.2). Every stored baseline whose only diffs were AA-subtracted now fails at
   defaults. Migration: pass `allowIgnoredAA:true`, which is the documented, explicit
   opt-in — the old behaviour is reachable, it is just no longer free.
2. **`code:"error"` no longer covers transport death or JS throws** (S3.3). Clients
   branching on `code == "error"` must add the new codes. The prose is unchanged, so
   clients that only read prose are unaffected.
3. **`navigate` gains `httpStatus`** (S4.6) — additive, but a client with
   `additionalProperties:false` on its own parsing of navigate will now see an extra key.
   The repo's own outputSchema discipline means the key is declared; a strict external
   client must widen.
4. **`visual`'s default baseline directory moves** to `%LOCALAPPDATA%/gladius/visual`
   (S2.4). Baselines saved under `<cwd>/.gladius/visual` are not found until the user
   sets `GLADIUS_VISUAL_DIR` to the old path or re-saves. This is a data migration with a
   one-line escape hatch, and it is deliberate: writing into the caller's repo was the bug.
5. **A call can now time out** (S2.3) where it previously ran until a CDP timeout. A tool
   that legitimately needs longer raises its own `_budgetMs`; `wait_for`'s 15 s cap is
   unchanged and still below the default budget.
6. **`assert_layout hitTarget` is unchanged**, and the new `targetSize` is the size check
   (S5.6). A user who assumed `hitTarget` meant size was wrong before; the description now
   says so.

Additive and non-breaking otherwise: every new field, both new tools, the new
`assert_layout` checks and arguments, `visual`'s `import` action and `regions` array, the
`settled`/`freeze`/`fontsPending` reply fields, `audit_tokens`.

Not a breaking change, but worth stating: **no new Rust dependencies** (D6), so the
`Cargo.toml`/`Cargo.lock` diff for 0.8.0 is empty unless an unrelated chore lands.

---

## 13. Findings deliberately not closed

Named, with the reason, so nobody re-discovers them as gaps.

| Finding | Why not in 0.8.0 |
|---|---|
| **L3-3a** state matrix (forcing `:hover`/`:active`, state-diff runs) | `CSS.forcePseudoState` produces a state the user never entered, and a state runner is orchestration, not a primitive. It also collides with S5.5's decision not to force pseudo-state. Deferred with no version. |
| **L3-3b** breakpoint sweep `compare_set` | S4.4's `matrix` covers the assertion half of the same need in one call. A capture-and-compare sweep is a scripted loop over existing tools and does not earn a tool slot yet. |
| **L3-5** CLS / perf traces | D7 non-goal. `cdp` remains the documented escape hatch. |
| **L3-6** motion capture / clip recording / easing assertion | D7 non-goal. A duration assertion without a clip has no viewer; a clip is a media pipeline. |
| **L1-F9** `snapshot followNext` auto-paging | The typed paging contract works and is recoverable; S4.5 fixes the half that blocks work (refs in frames). Auto-paging hides the cost of a large snapshot from the caller. |
| **L4** theoretical list (oversized results, duplicate JSON-RPC ids, `targetCrashed` latency) | Each is bounded or self-correcting today (L4 §"Theoretical"). Real work, no urgency, and every one touches framing rather than browser control. |
| **L5 §5** god files (`browser.rs` 5,895 lines, `tools.rs` 5,715) | A refactor with no user-visible change, and the audit's own verdict is "rot is concentration, not decay". Mixing it into a truth-and-lifecycle release would bury the behavioural diff. |
| **L5 §4** `cargo audit` before release | A network command and an advisory judgement, not a code change. Tracked as release ceremony, not as a spec item. |

Two findings are **partly** closed and say so in their items: L1-F9 (refs in frames only)
and L2-9 (claims corrected, no scroll-through implemented).
