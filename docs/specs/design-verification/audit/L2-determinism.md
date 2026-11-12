# Lens 2 — Is the visual proof actually true? (read-only audit)

Repo: `Q:\PROJECTS\PERSONAL\gladius` (Rust, `gladius-mcp` 0.7.1). Promise under attack: `screenshot {deterministic:true}` pins everything so "two captures of the same page are provably identical / byte-identical" (tools.rs:37, GUIDE.md:267, ARCHITECTURE.md:14, capture.rs:37-38), and `visual compare` returns `pass` on that basis.

Verdict: the checksum proves two captures went through the same encoder, not that the page was stable. The freeze/settle path handles exactly one case well (CSS animation/transition on the main document, proven by selftest layout fixture with a spinning `#spinner`). Everything else that moves pixels — web fonts arriving late, JS-driven painting, shadow-DOM/iframe motion, video/canvas/WebGL, lazy content below the fold — is either waited on with a timeout that silently gives up, or never touched. Worst: the default `visual compare` can return `pass:true` while pixels differ (sub-threshold drift, AA-subtracted edge changes, anything inside a mask), with the evidence relegated to a `warning` string most agents will not read. Ranked below by how likely an agent's "verified" is wrong.

## Ranked findings

### 1. Default compare passes while pixels differ — AA subtraction is silent unless you read `warning` [LIE — claims pass]
- Scenario: 1px text/glyph shift produces many isolated 1–2px high-contrast-edge components. Each is AA-eligible and ≤ `minDiffCluster` (default 2), so all are subtracted; `diffPixels: 0`, `pass: true`, with only `ignoredPixels: N` + a `warning` string naming it.
- Evidence:
  - AA rule subtracts eligible components ≤ budget: `src/visual.rs:663-696` (ignore iff `aaOn && allEdge && comp.length <= minCluster`).
  - Pass ignores subtracted pixels by default: `src/visual.rs:949-951` (`aa_tolerated`: `allow_ignored_aa || ignored==0`), `src/visual.rs:1084-1087` (`pixel_pass` uses `aa_tolerated`, not raw diff), default `allow_ignored_aa: true` in `src/visual.rs:487-501` / `src/tools.rs:2632-2635`.
  - Warning exists but does not flip pass: `src/visual.rs:955-962` (`aa_warning`), merged into `warning` at `src/visual.rs:1077-1083`.
  - Selftest proves the mechanism: 1px edge pixel passes with `ignored==1` at `src/selftest.rs:876-899`.
- Severity: highest. An agent checking `pass` (the documented verdict) reports "no visual change" when a real 1–2px edge change happened.
- Honest fix (pick one): default `allowIgnoredAA:false` (breaking but truthful), OR make `pass` require `warning == null` in docs/schema, OR at minimum rename reply `pass` → `passWithinTolerance` and document "pass:true + ignoredPixels>0 means pixels differed."

### 2. `threshold: 0.1` means a whole-page colour drift passes with `diffPixels: 0` [LIE by omission]
- Scenario: P3-vs-sRGB rendering, gamma/GPU dithering, or anti-aliasing shade shift moves every channel by ≤25 levels. No candidate exceeds `thr = 0.1*255 = 25.5`, so `diffPixels: 0`, `maxChannelDelta` small-but-nonzero, `pass: true` — while checksums differ.
- Evidence: `thr` computation + candidate gate `src/visual.rs:620,631-644` (`d > thr`, `thr = max(0,min(1,threshold))*255`); defaults `threshold 0.1, maxDiffPixels 0, maxDiffRatio 0` at `src/visual.rs:487-501`, `src/tools.rs:2601-2612`.
- Severity: high. Checksum says "different", compare says "pass" — the two proofs contradict and the agent trusts `pass`.
- Fix: document that `pass` means "no channel moved > threshold", never "identical"; or add a `pixelsIdentical: checksum==baselineChecksum` field so checksum-vs-pass disagreement is machine-readable.

### 3. `maskRects` hides unlimited regressions with no trace [GAP that reads as a LIE]
- Scenario: mask the timestamp/carousel/avatar region; a completely broken widget inside the mask still yields `pass:true, diffPixels:0`, no warning, no masked-pixel count.
- Evidence: masked pixels excluded at candidate stage `src/visual.rs:622-628,631-644` (`if d > thr && !masked`); diff image paints masked amber `src/visual.rs:719-721` but the JSON reply carries no `maskedPixels` count; no area cap, no coordinate validation, no warning (`warning` only covers env + AA at `src/visual.rs:1077-1083`); mask parsing is raw passthrough `src/tools.rs:2613-2617`.
- Also: mask coordinates are compared against device pixels `x,y` in the harness loop (`src/visual.rs:633-634,622-628`) while docs say `{x,y,w,h}` with no space specified (`src/tools.rs:288-299`); true only because deterministic pins DSF=1. A non-1 DSF capture (external engine ignoring the override) silently misplaces every mask.
- Severity: high. Masking is the recommended workflow for dynamic content, and it is an unmetered hole.
- Fix: return `maskedPixels` + `maskCoverageRatio`, warn when coverage exceeds e.g. 10%, validate/clamp rects and state the coordinate space (device px at DSF 1).

### 4. Fonts: `document.fonts.ready` + 3s timeout is not a webfont-ready signal [LIE — "waits for fonts"]
- Scenario: `font-display: swap` renders fallback, swaps after settle; slow/unreachable font server hits the 3s race and captures fallback; variable-font weight arrives late; second capture gets the real font → checksums differ, or worse, both capture fallback at different swap stages and match while glyphs are wrong.
- Evidence: settle `src/capture.rs:77-94` — `Promise.race([document.fonts.ready, timeout 3000])`, images raced 3s, then only 2 rAF ticks (250ms each). Proceeds on timeout with only a stderr `warn` (`src/capture.rs:476-481`). No per-`FontFace` status check, no layout-shift re-settle after fonts, no `font-display` handling anywhere in `src/`.
- Severity: high. Fonts are the commonest source of cross-run 1px text diffs.
- Fix (behaviour): wait `document.fonts.status === 'loaded'` + one extra rAF after, and fail/flag (not warn-only) when the race times out. Fix (claim): "waits up to 3s for initial fonts" instead of "waits for fonts."

### 5. Freeze stylesheet touches the main document only — shadow DOM, iframes, SMIL, WAAPI, video/canvas keep moving [LIE — "freezing animation"]
- Scenario: component library renders inside shadow roots; spinner there keeps spinning. `element.animate()` (Web Animations API), SVG `<animateTransform>`, `<video autoplay>`, canvas/WebGL/rAF loops, `<marquee>`, `performance.now()`-driven rendering are all unaffected by `animation:none`.
- Evidence:
  - Injection is `document.head`-only: `src/capture.rs:52-60` (`document.getElementById` / `document.head`), CSS text `src/capture.rs:69-74` covers only CSS `animation`/`transition`/`caret`/`scroll-behavior`/scrollbars.
  - Reduced-motion emulation is best-effort and engine-dependent (`src/capture.rs:440-469`; reports `reducedMotionEmulated:false` on engines without the command) and in any case only *requests* motion reduction — it does not pause rAF/video/canvas.
  - No WAAPI (`getAnimations().pause()`), no SVG-SMIL (`pauseAnimations()`), no `video.pause()`, no iframe/shadow piercing anywhere in `src/capture.rs`.
- Severity: high. Modern pages put motion exactly where the freeze does not reach.
- Fix: pierce same-process iframes + open shadow roots with the freeze (or document "main-document CSS only"); pause WAAPI/SMIL/video in the settle script; state that canvas/WebGL/video frames are sampled, never deterministic.

### 6. `animation:none` changes the layout it claims to measure [LIE — freeze alters geometry]
- Scenario: mid-flight CSS transform/transition (e.g. selftest `#spinner` at `rotate(137°)`) is reset to its untransformed base box by `animation:none; transition:none` (`src/capture.rs:74`, fixture `tests/fixtures/layout.html:14-16`). The captured box is not any state the user ever sees — it is the freeze artifact. `assert_layout box` is measured without the freeze, so geometry-proof and pixel-proof disagree by construction.
- Severity: medium-high. The determinism bodges the subject.
- Fix: prefer `animation-play-state: paused` snapshots over `animation: none` where possible, or document that deterministic captures measure the animation-off state, not the live state.

### 7. Settle timeouts silently give up — determinism degrades to "best effort" with only a stderr line [LIE]
- Scenario: background tab starves rAF (each tick races 250ms), images hang past 3s, renderer wedged past `SETTLE_CAP` 5s (`src/capture.rs:47,89-94`). Every path resolves `true` anyway and the capture proceeds; the only record is `eprintln!` (`src/capture.rs:165-167,480`), which never reaches the MCP reply.
- Evidence: `src/capture.rs:77-94` (all waits raced), `src/capture.rs:476-481` (timeout → warn + proceed).
- Severity: medium-high. The reply carries no `settled: false` signal, so the agent cannot distinguish "stable" from "gave up waiting."
- Fix: add `settled: bool` + `settleTimeouts: [...]` to the deterministic metadata block (`src/capture.rs:136-160`).

### 8. Environment fingerprint misses every font/OS/GPU/colour axis [GAP — warning exists but is blind]
- Scenario: baseline saved on machine A (ClearType tuning, font set, GPU raster, display-p3 CSS, `forced-colors`, dark `color-scheme`), compared on machine B or after a Chrome minor bump. Fingerprint matches → `environment.match: true`, no warning — yet sub-pixel rasterisation differs.
- Evidence: `EnvMeta` records only browser string/major/viewport/dsf/url/checksum/createdAt (`src/visual.rs:168-190`), `differences()` keys only on major/viewport/dsf (`src/visual.rs:208-226`), identity from `Browser.getVersion` product string only (`src/visual.rs:292-317`). Not captured: Chrome minor/patch, OS/platform, GPU vs software raster, installed-font set, `prefers-color-scheme`, `forced-colors`, `prefers-contrast`, scrollbar width/overlay, zoom, colour profile. Launch argv has no font/colour/GPU pins (`src/browser.rs:3270-3289`). Docs correctly say the sidecar catches "browser-update rot" (`src/visual.rs:163-166`) but only at major granularity.
- Severity: medium. The warning mechanism is honest; its coverage is not.
- Fix: record at least full version string, platform (`navigator.platform` + `navigator.userAgent`), `colorScheme`/`forcedColors`/`contrast` media states, and font-count hash; mismatch → same warning path.

### 9. Coverage: full-page is one composited shot with lazy content never triggered [GAP]
- Scenario: below-the-fold `loading="lazy"` images / `IntersectionObserver` content never loads because nothing scrolls into view; settle only waits for `document.images` *currently present* (`src/capture.rs:81-88`). Sticky headers / `position:fixed` overlays composite per Chromium's full-page behaviour (duplicated or pinned) — captured faithfully but not what a user sees. In-element scrolled content (`overflow:scroll` scrolled halfway) captures only the visible window. Cross-origin iframes composite (they do appear), but OOPIF-crash/permission-blocked frames render blank with no signal.
- Evidence: full-page is just `captureBeyondViewport:true` (`src/capture.rs:320-326,483-485`); no scroll-through, no lazy-load trigger, no per-frame readiness in settle; `fromSurface:true` captures the compositor surface as-is.
- Severity: medium. Agents asserting "whole page verified" miss everything that needs a scroll to exist.
- Fix: document "no scrolling is performed; lazy content below the fold is captured unloaded"; optionally scroll-through before full-page captures.

### 10. Checksum is over PNG bytes, not pixels — and non-deterministic capture has no target-race guard gap but deterministic path trusts DSF read-back [minor GAPs, mostly handled]
- Checksum: FNV-1a over encoded PNG bytes (`src/capture.rs:311-318`). Correct in practice (Chrome's PNG encoder is deterministic for identical pixels; no timestamps in the stream), cheap, adequate. Not a finding except: never state "byte-identical" without "same encoder" — an external engine (`GLADIUS_CDP_URL`) with a different PNG encoder could break byte-identity while pixels match. Code already confines owned path (`src/browser.rs:3291-3337`) but attach path accepts any engine.
- DSF: deterministic pins `deviceScaleFactor:1` (`src/capture.rs:433-439`) and reports live `devicePixelRatio` read-back (`src/capture.rs:96-103,548-557`). If an external engine ignores the override, `dsf` reports the truth (good — `src/browser.rs:662-679` read-back discipline), but the mask-space mismatch in finding 3 follows.
- Dropped: `target_changed` race guard covers plain captures (`src/tools.rs:2327-2355`); deterministic path is serialised by the capture lock (`src/capture.rs:403-407`, `src/browser.rs:186-195`) — handled, tested (`src/selftest.rs:450-509`).

## What the code already handles (do not file)
- Restore confirmation from the page, not the CDP ACK, incl. dropped-clear retry (`src/capture.rs:26-34,273-306,515-544`; selftest `restore-confirmed-from-page` at `src/selftest.rs:1293-1343`).
- Capture serialisation across concurrent deterministic captures (`src/browser.rs:186-195`, selftest `concurrent-captures-serialized`).
- AA rule is mechanical and documented, `allowIgnoredAA:false` escape hatch exists (`src/visual.rs:23-43,944-962`).
- Unknown/missing/malformed sidecar is `match:false`, typed failure under `requireEnvironmentMatch` (`src/visual.rs:240-287,977-991,1048-1075`).
- Scratch diff tab always `background:true`, typed refusal otherwise (`src/visual.rs:528-558,766-779`; selftest `scratch-background-unsupported`).
- Scrollbar gutter zeroed so its presence cannot shift the content box (`src/capture.rs:69-74`).

## Method note
Read `src/capture.rs` (full), `src/visual.rs` (full incl. harness JS at :572-741), `src/tools.rs` screenshot/visual arms (:2296-2372, :2490-2657) and tool descriptions (:37, :288), `src/browser.rs` launch argv (:3270-3289) + viewport read-back (:662-690), `src/selftest.rs` deterministic/visual/AA/env blocks (:400-509, :705-836, :838-970, :972-1093), fixture `tests/fixtures/layout.html`, docs `GUIDE.md:266-272`, `ARCHITECTURE.md:14,202-254`. `git log --oneline -20` shows only release/chore/docs/schema work — no recent change to the freeze/settle/diff/fingerprint paths, so findings are current. No code modified, nothing built.
