# Lens 5 — Does the repo keep its own promises? (gladius, READ-ONLY)

Repo: `Q:\PROJECTS\PERSONAL\gladius`. HEAD `abba9b1`, `git describe` = `v0.7.1-5-gabba9b1` (5 docs-only commits past the release tag); tree dirty (`M README.md`, untracked `scripts/npm-passkey.mjs`). Method: read-only; no build, no browser run, no network.

## 1. Claim vs behaviour

- **"25 tools" — PROVEN.** `src/tools.rs:7-331`: 21-tool literal + 4 inserted programmatically (`storage`, `assert_layout`, `layout`, `visual`; insertion comment at `tools.rs:210-212`). Count is derived at runtime, never hardcoded (`src/mcp.rs:74-79`), and asserted in four suites (`tests/mcp_conformance.rs:220`, `tests/schema_conformance.rs:365`, `tests/envelope.rs:142`, `tests/headless_chromium.rs:354`). Rot: the doc-comment at `tests/headless_chromium.rs:6` still says "initialize lists 21 tools" while the assertion below it says 25.
- **Deterministic screenshots / byte-identical checksums — IMPLEMENTED, UNPROVEN end-to-end.** Mechanism is real: FNV-1a 64 checksum `src/capture.rs:308-311`; deterministic pin (metrics fix, `prefers-reduced-motion`, freeze stylesheet, fonts+images+2×rAF settle, full restore) at `src/capture.rs:393-540`, advertised in `src/tools.rs:37` and `docs/GUIDE.md:267` ("two captures of the same page are provably identical"). But **no test asserts checksum equality between two captures** (zero `checksum ==` hits across `tests/`). `tests/concurrency.rs:775` comments "the two captures are byte-identical" then asserts geometry/dsf/restore only (`concurrency.rs:845-879`) — never the checksums. Cleanup is tested; pixel-identity is not.
- **AA-aware diffing — PROVEN.** `tests/review_findings.rs:353-440`: 2-px edge diff gives `ignoredPixels==2`, `pass==true` by default, `pass==false` with `allowIgnoredAA:false` plus a naming warning. **Threshold/maxDiff pass-bars — UNPROVEN:** zero test hits for `threshold`/`maxDiffPixels`/`maxDiffRatio`; a silently-ignored `threshold` would go red nowhere.
- **Supported CDP domains — PROVEN (narrow + escape hatch).** Direct CDP use spans Page, DOM, Runtime, Input, Network, Emulation, Target, Browser, Storage, Log, Accessibility (grep over `src/`). No chromiumoxide/puppeteer/playwright in the runtime (`Cargo.toml:20-37`: tokio, tokio-tungstenite 0.30, reqwest 0.13, serde_json, tempfile, base64 — CDP hand-rolled over tungstenite). Anything else goes through the raw `cdp` tool (`docs/GUIDE.md:284-287`).
- **Browser auto-download — PROVEN, honestly scoped.** First *run* (not install) provisions runtime + browser, both sha256-verified, re-verified on reuse (`docs/GUIDE.md:24-41`, `npm/bin/gladius-mcp.js:1-60` header, `docs/CHROMIUM.md`). `--doctor` is the no-network diagnostic. No silent background fetch is claimed or present.
- **Cross-platform — NOT CLAIMED (honest).** `README.md:48`, `docs/GUIDE.md:319`, npm README all say Windows x64 only, with `GLADIUS_MCP_ALLOW_NON_WINDOWS` as diagnostics-only bypass.
- **"No OS cursor / no foreground tab" — PROVEN with one documented wrinkle.** Input rides CDP `Input.*` dispatches (`src/tools.rs:52-53`); `headless_chromium.rs` proves no visible window via OS window handles (not CDP). Wrinkle is disclosed: on an attached engine (`GLADIUS_CDP_URL`) `visual compare` opens "a real extra tab for a moment" (`src/tools.rs:288`), and `close` only detaches there (`docs/GUIDE.md:66-69`).

## 2. Test coverage reality

- Shape: 27 test files, ~15.6k lines, **all live-browser integration** (every client spawns the real binary + real `chrome-headless-shell` from cache; e.g. `tests/e2e_mcp.rs:1-40`). Zero unit tests: pure logic (checksum fn, AA 3×3 rule, diff math) is reachable only through a browser.
- Every tool is mentioned somewhere, but mention ≠ coverage. Lowest mention counts (`"tool"` hits per file set): `history` 7, `fetch_text` 8, `assert_layout` 8, `layout` 9, `console` 12, `network` 15. Error-path depth for these is thin by inspection.
- Would a test go red if `deterministic` silently broke? **Partially.** Serialization/restore tests (`tests/concurrency.rs:779-919`) would catch interleave and cleanup failures (freeze style left behind, motion preference changed). A subtle pixel non-determinism with clean restore would pass — see §1.
- Would a test go red if the diff threshold stopped working? **No** (zero threshold tests). Would one go red if `allowIgnoredAA` broke? **Yes** (`review_findings.rs:422-440`).
- New-tool safety net is good: `schema_conformance.rs` header states one case per tool with a completeness assertion, so tool #26 fails loudly instead of slipping past.

## 3. Error taxonomy

- **Typed, and the best part of the codebase.** `ToolErr` at `src/tools.rs:353-379`: `typed(prose, sc)` minted at the failure site + `fallback(prose)` → `{code:"error"}` for genuinely untyped CDP/JS failures. Closed code set documented at `src/tools.rs:398-412`: `stale_ref`, `not_actionable`, `invalid_target`, `option_not_found`, `capability_gated`, `target_changed`, `timeout`, `invalid_args` (+reasons), `file_not_found`, generic `error`. Fail-loud unknown-arg gate at `src/tools.rs:421-433`. Envelope contract (prose byte-identical + `structuredContent`) is assertion-backed across `tests/envelope.rs:319-783`.
- **The hole:** there is no Rust enum — codes are string literals at call sites (no `enum *Error/*Code`, no thiserror/anyhow in `src/`; only `std::io::ErrorKind` matches). A typo'd code compiles; only the per-case envelope tests catch it. And **"browser dead" is untyped**: transport/CDP failures collapse into `{code:"error"}` (`tools.rs:367-396`), so an agent cannot branch "relaunch the browser" vs "page JS threw" vs "selector wrong" on codes alone — `invalid_target/not_found` covers selectors, `timeout` covers waits, but a dead engine is prose-regex territory.

## 4. Dependency and supply-chain posture

- Deps (`Cargo.toml:20-37`, `Cargo.lock`, 181 packages): tokio 1.54.1, tokio-tungstenite/tungstenite 0.30, reqwest 0.13 (rustls, native roots — TLS trust-store split fixed at `7d38798` per log), serde 1.0.229, base64 0.23, tempfile 3.27. No browser-driver crate; the CDP layer is hand-rolled and auditable.
- Browser pin: `chrome-headless-shell 155.0.8059.84`, zip sha256 `5ca590a4…` (`docs/CHROMIUM.md:12-23`), 288-file manifest in `scripts/chromium-manifest.json`, fetched from CfT, **never redistributed** — own manifest is the integrity root because CfT publishes no hashes (stated plainly, `CHROMIUM.md:27-34`). Licence: repo MIT (`LICENSE`, `Cargo.toml:5`), Chromium BSD attribution copy carried without browser bytes (`THIRD-PARTY-NOTICES.md:1-20`, `docs/PUBLISHING.md`).
- MSRV 1.88 (`Cargo.toml:14`, `rust-toolchain.toml`: 1.88.0) with the lockfile-coherence comment — consistent, not aspirational.
- Version coherence: `Cargo.toml` 0.7.1 = `npm/package.json` 0.7.1 = installed 0.7.1; `v0.7.1` tag exists. But HEAD is 5 commits past the tag with a dirty tree, so "git HEAD == published bytes" is already false (docs-only delta today — the mechanism that matters is the release lane asserting versions, `abba9b1`).
- `cargo audit`: **UNPROVEN** — not run (network command, out of scope for this lens); advisories in `Cargo.lock` not assessed. Note reqwest 0.13 / tungstenite 0.30 are very new majors; worth an audit before the next release, not a finding.

## 5. Structural rot

- No `todo!`/`unimplemented!`/`allow(dead_code)`/FIXME anywhere in `src/` (grep: zero). No dumping ground `lib.rs` — binary-only crate (`src/main.rs`, 125 lines, thin: version gate at `main.rs:20-40`, subcommand dispatch).
- But two god files: `src/browser.rs` (5,895 lines) and `src/tools.rs` (5,715 lines) = ~65% of 17.8k src lines. The dispatch in `tools.rs:435+` is formulaic (check_unknown + arg extraction × 25) rather than duplicated logic; CDP plumbing is centralized in `browser.rs` (`send_raw`/`send_session`, `stale session` invalid-target mapping at `tools.rs:387-395`).
- Tool #26 cost: the frozen additive-only contract (`docs/ARCHITECTURE.md:8`) plus the insertion recipe (`tools.rs:210-212`) plus the schema-conformance tripwire make a CDP-thin tool an afternoon; anything touching browser locks/state (the exec-lock discipline in `src/mcp.rs:109+`) is a day, because the reviewer must understand `browser.rs` session/target semantics first. No evidence a newcomer could do it in an afternoon across the board.

## 6. Distribution honesty

- Install is `npx gladius-mcp@latest` (never `npm i -g` — refused by `preinstall` in `npm/package.json`). The tarball holds only launcher + `engine-manifest.json` (no browser/server bytes, no postinstall — correctly so, since npm 12 blocks install scripts under npx). First run writes two versioned cache dirs (`%LOCALAPPDATA%\gladius\engine\<ver>-<sha8>\`, `\chromium\<ver>-<sha8>\`) + `.ready` markers; per-session private profile under `%TEMP%`, swept on `close` (test-backed, `headless_chromium.rs`).
- Windows-specific step: none beyond being on Windows x64; the gate fails closed with a named error and a bypass env for diagnostics only (`docs/GUIDE.md:294-297`).
- `serverInfo` is honest (`src/mcp.rs:57-90`): `name`/`version` from `CARGO_PKG_VERSION`, `backend`/`caps` read live, tool `count` derived from the real list, `gated` computed from current caps. Launcher/engine skew fails closed via `GLADIUS_EXPECTED_VERSION` (`src/main.rs:26-30`).
- npm vs cargo divergence: the user runs the npm launcher → GitHub Release bundle, **not** `cargo install` (crate name `gladius` ≠ package `gladius-mcp`). Local HEAD builds and the running bytes are different artifacts by design; today they agree (0.7.1) only because post-tag commits are docs-only. The `bin/` dir holds local `gladius.exe` builds (gitignored via `/bin`, `git status` clean apart from README + untracked passkey script) — a newcomer should know `bin/gladius.exe` is stale the moment HEAD moves.

## Three defects that most endanger credibility

1. The headline determinism promise ("two captures provably byte-identical") has no checksum-equality test — the one comment claiming it asserts everything except the checksums.
2. Dead-engine failures collapse into generic `{code:"error"}`, so agents cannot programmatically distinguish "relaunch the browser" from "page JS threw."
3. Nothing in-repo pins the Release bundle to a commit hash — HEAD already drifts past the tag, so "which bytes is the user running?" is answerable today only by version-string convention.

## Verdict

**Credible base — extend, don't rebuild.** Promises are mostly honest (Windows-only and no-redistribution stated outright, serverInfo derived not hardcoded, error codes minted at site, browser pin hash-verified). The rot is concentration (two god files), not decay (zero TODOs, static gates documented: `cargo fmt --check` + `clippy --locked -D warnings`, `README.md:59`). Before building on it: add the checksum-equality test and one threshold-flip test, and give transport failure its own code — each is a small, well-located change.
