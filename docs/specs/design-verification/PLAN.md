# PLAN — gladius 0.8.0, design verification

> **What this is.** The execution order for the 0.8.0 audit, derived from the five lens
> reports in [`audit/`](audit/) and frozen by [`SPEC.md`](SPEC.md) and
> [`FREEZE.md`](FREEZE.md). The spec says *what*; this file says *in what order*, *by what
> smallest honest change*, *proved by what test*, and *how long*.
>
> Workstream order is fixed: **W1 → W2 → W3 → W4 → W5**. It is a dependency order, not a
> preference: W3's tests assert W1's and W2's behaviour, W4's tools reuse W1's freeze and
> metrics helpers, and W5 rides on W1's visual reply and W4's `assert_layout` arguments.
>
> Effort is in **implementer-days** — one person, this repo, no browser-hunting. The total
> is 44.25 days for 31 items, and the total is stated again at the end so it can be
> checked against the item table.

---

## W1 — Truth in the proof

**Problem.** The checksum proves two captures went through the same encoder, not that the
page held still, and the default `visual compare` can answer `pass` while pixels differ.

**Findings closed.** L2-1, L2-2, L2-3, L2-4, L2-5, L2-6, L2-7, L2-8, L2-9, L2-10 (with
L5-1's claim half).

**Smallest honest change.** Stop letting a tolerance answer the identity question: add the
identity boolean, make the tolerance verdict say what it is, make AA suppression an
explicit opt-in, and make every give-up in the settle path a field instead of a stderr
line. Then extend the freeze to the places motion actually lives (WAAPI, SMIL, media,
open shadow roots, same-process frames) and cut the docs' claim to exactly that scope.

**Provoked test.** `tests/visual_truth.rs`, `tests/deterministic_freeze.rs`, and the first
test in `tests/claims.rs`. Each item names its own; the workstream is done when a 1 px edge
shift, a late font, a shadow-root spinner and a stale baseline all produce a *true answer*
rather than a comfortable one.

| Item | What lands | Days |
|---|---|---|
| S1.1 | `pixelsIdentical`, `passWithinTolerance`, `pass` as a documented alias | 1.5 |
| S1.2 | `allowIgnoredAA` defaults to false (strict is free, tolerance is opt-in) | 0.5 |
| S1.3 | `maskedPixels`, `maskCoverageRatio`, `maskRectsApplied`, >10 % warning | 1.5 |
| S1.4 | `settled` + `settleTimeouts` on the reply and in the sidecar; `baseline_unsettled` | 1.5 |
| S1.5 | Freeze walk: WAAPI, SMIL, media, open shadow roots, same-process frames, `freeze` field | 3.0 |
| S1.6 | `assert_layout {deterministic:true}` measures the state the camera sees; `freezeMode` | 1.5 |
| S1.7 | Font readiness by `FontFace.status`, `fontsPending`, timeouts reported | 1.5 |
| S1.8 | Environment fingerprint covers platform, full version, colour/contrast, fonts | 1.0 |
| S1.9 | The claim corrected in tool description, GUIDE, ARCHITECTURE, `instructions()`; `tests/claims.rs` begins | 1.0 |
| | **W1 total (9 items)** | **13.0** |

**Exit criteria.** (a) A 1 px text shift fails at defaults and `pixelsIdentical` is `false`.
(b) Every settle race that loses is named in `settleTimeouts`. (c) A page with a shadow-root
spinner, a WAAPI animation, an SVG SMIL animation and an autoplaying video yields two
`pixelsIdentical` captures. (d) No doc under `docs/` says "provably identical" without the
scope sentence.

---

## W2 — Lifecycle

**Problem.** One dead engine, one wedged call or one hard kill leaves the server unable to
recover and unable to say what the agent should do.

**Findings closed.** L4-1, L4-2 (the surviving half — see SPEC §11), L4-3, L4-4, L4-5, L4-6.

**Smallest honest change.** Make death recoverable instead of terminal: one automatic
relaunch of the owned engine, and a typed error that names `close` when the relaunch fails.
Give every call one total budget so a wedged renderer cannot hold the exec lock for
minutes, and answer a cancel immediately while the restore keeps running behind it. Add
the pid file that the job object does not cover, move baselines out of the caller's repo,
and give closed shadow roots a typed name instead of a misleading `not_found`.

**Provoked test.** `tests/engine_recovery.rs`, `tests/hard_kill_reap.rs` (one new case),
`tests/budgets.rs`, `tests/shadow_closed.rs`. Done when killing the engine mid-session
produces a working next call or an error that says what to do, and when a blocked renderer
cannot exceed the budget.

| Item | What lands | Days |
|---|---|---|
| S2.1 | Owned engine relaunches once; `engine_dead`/`respawn_failed` naming the recovery | 1.5 |
| S2.2 | `%TEMP%/gladius-<pid>.json` + kill-then-sweep, closing the job-cover-false path | 1.0 |
| S2.3 | 90 s per-call budget (`_budgetMs` override), typed `timeout`; prompt typed `cancelled` | 2.5 |
| S2.4 | Baseline dir under `%LOCALAPPDATA%`, 500-entry cap, diff-image age-out | 1.0 |
| S2.5 | Closed-shadow ground truth, `shadow_closed`/`unreachable`, `@eN` path documented | 1.5 |
| | **W2 total (5 items)** | **7.5** |

**Exit criteria.** (a) Engine killed → next call either works or returns `engine_dead` whose
prose names `close`. (b) A job-less launch hard-killed leaves no orphan after the next
launch. (c) A blocked `evaluate` returns inside its budget and the exec lock is free
afterwards; a cancel answers in under 2 s. (d) No baseline is written into a cwd by
default.

---

## W3 — Credibility tests

**Problem.** The repo's headline claims — byte-identical captures, a working pass bar, a
typed error surface, a release built from a known commit — are asserted nowhere, so the one
thing that would catch a silent regression is missing.

**Findings closed.** L5-1, L5 §2, L5-2, L5-3, L5 §3; and L1-F7's error-shape half.

**Smallest honest change.** Write the assertions that would have caught the audit's
findings, and replace the string-literal error codes with a closed enum so a typo cannot
compile. Add the commit line the release body has never carried.

**Provoked test.** The workstream *is* tests, except S3.3 and S3.4. Done when
`findstr /i checksum tests\*.rs` returns hits, `threshold`/`maxDiff` each have a flip test,
a killed engine and a JS throw never report `code:"error"`, and the release body carries a
40-hex commit.

| Item | What lands | Days |
|---|---|---|
| S3.1 | Checksum-equality test on a moving page; the false `concurrency.rs:775` comment corrected | 0.5 |
| S3.2 | `threshold` / `maxDiffPixels` / `maxDiffRatio` / `maskRects` flip tests | 1.0 |
| S3.3 | Closed `ToolCode`/`ToolReason` enum; transport → `engine_dead`, JS → `js_error` | 1.5 |
| S3.4 | Commit + lock digest in the release body and `--doctor`; derivation fixture case | 1.0 |
| S3.5 | `tests/claims.rs` covers countable claims; the "21 tools" comment corrected | 0.25 |
| | **W3 total (5 items)** | **4.25** |

**Exit criteria.** (a) A deliberately broken freeze (freeze stylesheet removed) turns S3.1
red. (b) A dropped `threshold` argument turns S3.2 red. (c) Killing the engine, throwing in
the page, and evaluating `undefined` produce three distinct codes, none `error`. (d)
`scripts/derive-release.mjs --check` fails when the body has no commit line.

---

## W4 — Agent ergonomics

**Problem.** An agent pays extra schema reads and extra round trips to learn which of four
geometry tools to use, cannot fill a form or scroll in one call, cannot use a ref it just
read from a frame, and cannot see the status code a navigation returned.

**Findings closed.** L1-F1, F2, F3, F4, F5, F6, F7 (navigate half), F9 (refs half), F10
(routing half).

**Smallest honest change.** Sentences for the four tool-choice confusions and the sibling
routing; then four small verbs/arguments built on resolvers that already exist
(`fill_form` over `type`/`select`/`click`, `scroll` over the existing `scrollIntoView`
path, `assert_layout` viewport/matrix over the capture's metrics helper, `httpStatus` over
one `Network.responseReceived`). No new engine capability, no new dependency.

**Provoked test.** `tests/new_tools.rs`, `tests/frames.rs`, `tests/navigate_status.rs`,
`tests/claims.rs`. Done when each confusion in L1's "failing scenario" column has a call
sequence that is one shorter, and a five-field form is one call.

| Item | What lands | Days |
|---|---|---|
| S4.1 | Decision rules in nine descriptions; `oc-puppet`/`oc-rea` routing in `instructions()` | 1.0 |
| S4.2 | `fill_form {fields[], submit?}` — ordered, stops at the first failure with `failedAt` | 1.5 |
| S4.3 | `scroll {element\|ref, direction\|dy, container, behavior}` with read-back | 1.0 |
| S4.4 | `assert_layout {viewport}` + `{matrix[]}`, applied and restored, with read-back | 1.5 |
| S4.5 | `@eN` resolves against the named frame's snapshot table; no more blanket refusal | 2.0 |
| S4.6 | `navigate.httpStatus` measured from the main-frame response, `null` when unobserved | 1.0 |
| | **W4 total (6 items)** | **8.0** |

**Exit criteria.** (a) A 5-field form is one call with per-field results. (b) A container
scroll reports the position it read back, not the delta it was given. (c) One
`assert_layout` call covers three widths and leaves the previous override intact. (d) A ref
from a frame snapshot acts in that frame; the same ref with another frame is `stale_ref`.
(e) A 404 reports `httpStatus:404` and the existing "no fabricated status" test still
passes.

---

## W5 — Design parity

**Problem.** gladius can regress a live page against itself, but it cannot census a design
system, cannot compare a designer's reference against the build, and cannot gate the four
quality bars an agent is most likely to fake.

**Findings closed.** L3-1, L3-2, L3-4 (contrast, typography, focus visibility, hit-target
size), L1-F8 (measurement half).

**Smallest honest change.** One census tool that reports distinct computed values and
off-scale outliers against a caller-supplied scale; one import path so a reference PNG can
be a baseline, plus per-region deltas from the same pixel pass; and four new
`assert_layout` checks computed from values already read — no new crate (D6), no a11y rule
engine (D5), no overlay artifact.

**Provoked test.** `tests/audit_tokens.rs`, `tests/visual_truth.rs` (import case),
`tests/a11y_gates.rs`. Done when "do these six pages look like one system?" is one call,
a Figma export can be the baseline, and contrast/typography/focus/size each fail a fixture
that should fail.

| Item | What lands | Days |
|---|---|---|
| S5.1 | `audit_tokens` census + `offScale` verdict, main doc + open shadow + same-process frames | 2.5 |
| S5.2 | `visual {action:"import"}` + `regions[]` per-region deltas on `compare` | 3.0 |
| S5.3 | `checks.contrast` from resolved used colours; `contrast:null` on a non-flat backdrop | 2.0 |
| S5.4 | `checks.typography` with unit/weight/family normalisation and 0.5 px tolerance | 1.5 |
| S5.5 | `checks.focusVisible` proven by real focus + computed indicator delta, then restored | 1.5 |
| S5.6 | `checks.targetSize`, kept distinct from `hitTarget` (occlusion stays occlusion) | 1.0 |
| | **W5 total (6 items)** | **11.5** |

**Exit criteria.** (a) A 17 px outlier among 16 px text is named with its nearest scale
value. (b) An imported reference compares and reports per-region pixel counts, with
`environment.known:false` and its warning. (c) A gradient backdrop yields `contrast:null`
and a failing check, never a pass. (d) A 24 px button passes `hitTarget` and fails
`targetSize` in the same reply.

---

## Effort, summed

| Workstream | Items | Days |
|---|---|---|
| W1 truth in the proof | 9 | 13.00 |
| W2 lifecycle | 5 | 7.50 |
| W3 credibility tests | 5 | 4.25 |
| W4 agent ergonomics | 6 | 8.00 |
| W5 design parity | 6 | 11.50 |
| **Total** | **31** | **44.25** |

For scale: the two largest single items are S1.5 (the freeze walk, 3.0) and S5.2 (reference
import + regions, 3.0). Neither is large because it is hard to type; both are large because
they must be proven against a live engine and restored correctly on every exit path,
including cancel.

---

## What can run in parallel

The workstream order fixes *landing* order, not authorship. The disjointness is by file,
and the files are the seam.

| Lane | Items | Files | Runs |
|---|---|---|---|
| **A — visual truth** | S1.1 → S1.2, S1.3 | `src/visual.rs`, `src/tools.rs` (`visual` arm) | from day 1 |
| **B — capture truth** | S1.4 → S1.7; S1.5 → S1.6 | `src/capture.rs`, `src/layout.rs` (S1.6) | from day 1, parallel to A |
| **C — environment + docs** | S1.8; S1.9 last | `src/visual.rs` (S1.8), docs, `src/mcp.rs:53-54` | S1.8 parallel to A/B; S1.9 closes W1 |
| **D — lifecycle** | S2.1 → S2.2; S2.3; S2.5 | `src/browser.rs`, `src/mcp.rs`, `src/layout.rs` | parallel to W1, but **S2.2 after S2.1** (same file) |
| **E — credibility** | S3.1, S3.2 | new test files | authored during W1 as the red-first drivers, landed after |
| **F — ergonomics** | S4.1…S4.6 | `src/tools.rs` (disjoint arms), `src/frames` paths | must be **serial within itself** (one file, one dispatch) |
| **G — design parity** | S5.1…S5.6 | `src/tokens.rs` (new), `src/visual.rs`, `src/layout.rs` | after A and B; S5.1 may start once S1.5 lands |

Three collisions to respect, and they are the whole coordination cost:

1. **`src/visual.rs` is one lane.** S1.1–S1.3, S1.8, S2.4 and S5.2 all touch it. Order:
   S1.8 → S1.1 → S1.2 → S1.3 → (W2) S2.4 → (W5) S5.2.
2. **`src/tools.rs` is one lane.** Every W4 item and every schema change in W1/W2/W5 edits
   the tool list literal and a dispatch arm. Two implementers in this file will conflict on
   the insertion point at `src/tools.rs:240-243`, every time.
3. **`src/mcp.rs` is one lane.** S2.3, S4.2/S4.3 (lock policy), S1.6 and S4.4 all edit
   `needs_exec_lock`/`needs_critical_section`. S2.3 must land before the W4 lock-policy
   edits so the budget wrapper is not written twice.

Everything else is genuinely disjoint: `src/capture.rs` (B), `src/browser.rs` (D),
`src/visual.rs` (A), new test files (E).

**Cheapest real sequence for one person:** B (S1.4, S1.5, S1.6, S1.7, S1.1→S1.3 interleaved
in A as the same reply is finalised), then S1.8, then S1.9, then D, then E, then F, then G.
The freeze walk first, because four later items measure through it.

---

## Ordering rules that are not negotiable

- **No item lands without its acceptance test being red on `abba9b1` first.** The spec names
  the red reason; if the test is green before the change, the test is wrong.
- **S1.1 before S1.2.** A verdict cannot change meaning before it has a name.
- **S1.4 before S1.7.** Fonts report through the settle fields.
- **S1.5 before S1.6, S1.9, S5.1.** One freeze walk, one walk callable from three places.
- **S2.1 before S3.3.** The code S2.1 mints must exist before the taxonomy closes around it.
- **S2.3 before W4's lock-policy edits.** Otherwise the budget wrapper is written twice.
- **S1.9 and S4.1 serialise on `src/mcp.rs:53-54`.** One edit to `instructions()`, not two.
- **Every schema change ships with its `outputSchema` entry in the same commit.** A field
  emitted but undeclared fails `tests/schema_conformance.rs`.
