# FREEZE — gladius 0.8.0, design verification

> This file is the freeze record. If `SPEC.md` changes, this hash stops matching and the
> freeze is void until a new record is written that says what changed and why.

## Verifiable identity

| What | Value |
|---|---|
| Product | `gladius-mcp` |
| Baseline release | **0.7.1** |
| Baseline commit | `abba9b1` (5 docs-only commits past tag `v0.7.1`) |
| Release this spec defines | **0.8.0** |
| `SPEC.md` SHA-256 | `FC79D4EBEF973ECFF8083D8F4D7CDA549936F3A87A7E9F35EA77B1DC0C1E12E8` |
| `SPEC.md` size | 83,183 bytes, 1,242 lines |
| Items frozen | 31, across 5 workstreams |
| Total effort | 44.25 implementer-days |
| Evidence | five lens reports, copied verbatim, `audit/` |

Verify the freeze:

```powershell
Get-FileHash docs/specs/design-verification/SPEC.md -Algorithm SHA256
```

The hash must equal the value above. The audit copies must equal their originals; the
SHA-256 of each is recorded here so a reviewer can tell a re-worded report from a verbatim
one:

| File | SHA-256 |
|---|---|
| `audit/L1-ergonomics.md` | `9CEA0435B73BCCB58AACA23672F4BBC8CE00D84FAAC91D7C6704F7390687DF11` |
| `audit/L2-determinism.md` | `68B712F4588273D7EC68234540631B76A7FE7DB99F1157410E8091D5B9625B2F` |
| `audit/L3-design-parity.md` | `28F5C421BC7BEE00A05C0CFF2001DCC36E1D87A4B6FD5BDEAA0E6DFCB6775A8D` |
| `audit/L4-robustness.md` | `64088183947971BB2FF44858D807F63C2E5BE5AA3F106F152F706906E458ED77` |
| `audit/L5-repo-health.md` | `5BC5161F63909637081518F7B7DD0B24534C8911DDF1970C6F29C6D381DA7187` |

---

## In scope

Thirty-one items, five workstreams, in the order they land:

**W1 — truth in the proof (9 items, 13.00 d).** S1.1 identity vs tolerance on `visual
compare`; S1.2 AA suppression becomes an opt-in; S1.3 mask accounting; S1.4 `settled` and
`settleTimeouts`; S1.5 the freeze reaches WAAPI, SMIL, media, open shadow roots and
same-process frames; S1.6 the freeze and the measurement agree; S1.7 font readiness by
status; S1.8 environment fingerprint; S1.9 the claims match the code.

**W2 — lifecycle (5 items, 7.50 d).** S2.1 death is recoverable; S2.2 a pid file for the
job-less launch; S2.3 one bounded budget per call and a prompt typed cancel; S2.4 baselines
out of the caller's repo and capped; S2.5 closed shadow roots typed, not `not_found`.

**W3 — credibility tests (5 items, 4.25 d).** S3.1 checksum equality; S3.2 the pass bar
tested; S3.3 the error taxonomy as a closed enum; S3.4 the release names its commit; S3.5
countable claims asserted.

**W4 — agent ergonomics (6 items, 8.00 d).** S4.1 decision rules in the descriptions and
sibling routing; S4.2 `fill_form`; S4.3 `scroll`; S4.4 `assert_layout` viewport/matrix;
S4.5 refs in frames; S4.6 `navigate.httpStatus`.

**W5 — design parity (6 items, 11.50 d).** S5.1 `audit_tokens`; S5.2 reference import with
per-region deltas; S5.3 contrast; S5.4 typography; S5.5 focus visibility; S5.6 hit-target
size.

The **contract** of each item — behaviour, exact reply fields, the acceptance test that is
red on `abba9b1`, and the `file:line` it touches — is in `SPEC.md` and nowhere else. This
file does not restate it; two copies of a contract drift and the wrong one wins.

---

## Explicitly out of scope

Named so they are decisions rather than omissions. Each is traceable to a lens finding;
`SPEC.md` §13 carries the reason in full.

| Out | Source |
|---|---|
| State matrix / forced `:hover`/`:active` runs | L3-3a |
| Breakpoint capture-and-compare sweep (`compare_set`) | L3-3b |
| CLS, performance traces, Lighthouse, CrUX | L3-5, **D7** |
| Motion capture, clips, easing assertions, video recording | L3-6, **D7** |
| PDF export | **D7** |
| Cross-browser engines (Firefox, WebKit) | **D7**, and already a repo non-goal |
| Cloud / fleet baseline storage | **D7** |
| Vision or coordinate grounding | **D7**, and already a repo non-goal |
| An accessibility audit engine, axe-core bundling, WCAG rule catalogue, accessible-name or ARIA verdicts | **D5** — the `a11y` MCP owns that occupancy |
| Headed browsing, OS input, window control, file-picker automation | **D5** — `oc-puppet` owns that |
| Bulk passive capture / binary and network forensics | **D5** — `oc-rea` owns that |
| `snapshot` `followNext` auto-paging | L1-F9 (second half only) |
| Oversized-result chunking, duplicate-id supersede, `targetCrashed` proactivity | L4 theoretical list |
| `browser.rs` / `tools.rs` decomposition | L5 §5 — no user-visible change, wrong release |
| `cargo audit` gate | L5 §4 — release ceremony, not a spec item |

Also out, for the avoidance of doubt: **no new Rust dependencies** (D6). The 0.8.0
`Cargo.toml` diff is empty unless an unrelated chore lands, and no assertion in W5 may
introduce one.

---

## The semver call

**0.8.0.**

The number is derived, not chosen. `scripts/derive-release.mjs:29` states the rule this
repo already uses: on 0.x a breaking change moves the **minor**. 0.7.1 plus a breaking
behaviour is 0.8.0.

What makes it breaking:

1. **`allowIgnoredAA` defaults to `false`** (S1.2). A compare that passed because AA
   subtraction hid an edge change now fails. The old behaviour is reachable by passing
   `allowIgnoredAA:true` — explicit, documented, no longer free.
2. **`pass` changes meaning.** It is now an alias of `passWithinTolerance`, and the
   documentation says so in those words. A client that read `pass` as "nothing changed" was
   reading it wrong before; after 0.8.0 the field cannot be so read.
3. **`code:"error"` stops covering transport death and JS throws** (S3.3). Clients branching
   on that code must add `engine_dead` and `js_error`.
4. **The default baseline directory moves** out of `<cwd>` (S2.4), so existing baselines are
   not found until `GLADIUS_VISUAL_DIR` points at the old path or they are re-saved.
5. **Calls can time out** (S2.3) where they previously ran until a CDP timeout.
6. **`navigate` replies carry a new key** (S4.6); a client with a strict
   `additionalProperties:false` parse of that reply must widen.

Everything else in the 31 items is additive: new fields, new checks, new actions, two new
tools. `assert_layout hitTarget` keeps its meaning — occlusion — and the new size check is a
different name (S5.6), precisely to avoid a silent redefinition inside a minor-looking
release.

Nothing in this release changes the pinned browser (`chrome-headless-shell` 155.0.8059.84),
the MSRV (1.88), the distribution shape (`npx`, never global), or the Windows x64 boundary.
`scripts/derive-release.mjs`'s derivation rule itself is untouched: S3.4 adds a commit line
to the release body and a fixture case, and changes no arithmetic.

---

## Amendments

A frozen item implemented differently from its item block needs a written amendment naming
what it reaches; that is what the last section below requires, and `SPEC.md` is not edited to
provide it. The register:

| Amendment | Sharpens or supersedes | Effort | Commit |
|---|---|---|---|
| `AMENDMENT-01-sizing.md` — sizing: no cap, declared units, a reported scale | Extends S1.1, S1.3, S1.4, S1.8 and §4's contract table; sharpens S1.3's last bullet and §11 ruling 5; adds a row to §10; records that §4 holds no cap row and gains none. Supersedes no frozen item. | 2.25 d | `3ca8f94` |
| `AMENDMENT-02-timeline-pinning.md` - timeline pinning: a frozen capture is pinned, not paused | Sharpens S1.5 (pin-to-start with restore, not pause-at-position); records the closed-shadow-root coverage limit pending S2.5. Supersedes no frozen item. | 0 d | `13e19c7` |

---

## What would void this freeze

- Any edit to `SPEC.md` after this record (the hash will not match).
- An item implemented differently from its item block without a written amendment naming the
  item it supersedes.
- An item dropped silently. Dropping one means editing the in-scope list here and the
  traceability in `SPEC.md` §13, in the same commit, with the reason.
- A new Rust dependency (D6) reaching `Cargo.toml` for a new assertion.
- Any claim in the docs that outruns `SPEC.md` §3's determinism scope.
