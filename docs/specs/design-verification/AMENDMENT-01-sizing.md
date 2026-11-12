# AMENDMENT-01 — sizing: no cap, declared units, a reported scale

> **Status: additive amendment to the frozen [`SPEC.md`](SPEC.md).** `SPEC.md` is not
> edited: its SHA-256 is still
> `FC79D4EBEF973ECFF8083D8F4D7CDA549936F3A87A7E9F35EA77B1DC0C1E12E8`, exactly what
> [`FREEZE.md`](FREEZE.md) records, so the freeze holds. This file is the "written
> amendment naming the item it supersedes" that `FREEZE.md` requires, and it names the
> clause per item — including one item that names **no clause**, because the premise it
> arrived with was wrong.
>
> Read after `SPEC.md` §4 (the contract table) and §10 (reply shapes). Item IDs `A1`–`A5`
> are new; they do not renumber the 31 frozen items and they add no workstream. Five
> items, **2.25 implementer-days**, on top of the 44.25 `FREEZE.md` records.
>
> Baseline: `gladius-mcp` 0.7.1, HEAD `abba9b1` — the same tree every "today" statement in
> `SPEC.md` describes. Every `file:line` below was re-read on the branch that froze the
> spec, not carried over from a lens report.

---

## Supersession ledger

| Item | Relation to the frozen spec | Clause it names |
|---|---|---|
| **A1** | Supersedes **nothing**. Records a verified absence. | §4 (no cap row exists, and none is added) |
| **A2** | **Extends** S1.3, S1.4, S1.8 and §4's contract table. | the unitless entries: `src/tools.rs:47`, `:299-302`, `:254-257`, `:281-285` |
| **A3** | **Sharpens** S1.3's last bullet and §11 ruling 5. Does not re-implement S1.3. | the `mask_scale_mismatch` sentence in S1.3 |
| **A4** | **Extends** S1.1's "no third field" discipline to dimensions; adds a row to §10. | `screenshot`, `visual save`/`compare` reply shapes |
| **A5** | Supersedes **nothing**, and corrects the premise it came with. | no `SPEC.md` clause claims a crop exists; this records that it does not |

---

## A1 — There is no pixel cap, and there is not going to be one

**Supersedes** nothing. No clause of `SPEC.md` promises a pixel cap or forbids one. This
item writes the absence down so that "should we uncap to system resolution?" stops being a
question with an answer other than "there is nothing to uncap".

**Today (verified absent):** there is no render gate anywhere in this tree that shrinks,
crops or refuses a large image.

| Search | Result |
|---|---|
| `rg -i "downscal\|resize\|jpeg\|quality\|max_px\|maxPx" src/` | exit 1 — zero hits |
| `rg '"scale"' src/` | exit 1 — zero hits |
| `rg "css-px\|device-px" src/` | exit 1 — zero hits |
| `screenshot` `width`/`height` bounds | bare `integer`, no `maximum` (`src/tools.rs:41`); a floor of 1 and nothing else (`src/capture.rs:431-432`) |
| `viewport` upper bound | none (`src/tools.rs:5363-5393`); the engine decides and the reply reports `matched` |

Everything else follows: every mode is PNG by construction (`src/capture.rs:322,324`;
`src/visual.rs:725`), the byte length is whatever the encoder produced and is published
whole (`src/capture.rs:139`), full-page height is uncapped (`src/capture.rs:320-326`,
`:483-485`), and the diff is drawn on a canvas the size of the natural image
(`src/visual.rs:603-606`) and written at full resolution (`:723-725`), with `bbox` taken
from that same pixel loop (`:672-733`). The characterisation that matters: gladius
already emits native resolution with no interpolation, so the "emitted image is smaller
than the box it describes" bug is **not** present here — the gap is that nothing says
which pixel space the box is in (A2) and nothing reports the ratio (A4).

**Consequence, stated so it is not re-argued.** A reply may be viewport-sized — 2560×1440
on this owner's display, taller when full-page — and that is correct. A model client
downsizes to its own budget before tokens are counted, and that budget is not gladius's to
know: Anthropic's vision tier resizes to a 1568 px long edge (2576 px and 4784 tokens on
the high-resolution tier) and its token budget usually binds *before* the edge budget, so
a 1920×1080 image becomes 1456×819, not 1568×882. The reply's job is to be **honest**,
not small. A cap here would buy context-window relief the client already provides and pay
for it with a silent coordinate skew — exactly the defect class S1.1 and S1.3 exist to end.

**The industry position, briefly** (primary sources: Anthropic vision and vision-coordinates
docs; `microsoft/playwright` `tools/backend/screenshot.ts` and `tools/mcp/config.ts`;
`ChromeDevTools/chrome-devtools-mcp` `src/tools/screenshot.ts`):

| System | Cap | Reports a scale factor? |
|---|---|---|
| Anthropic vision | 1568 px long edge / 1568 tokens standard, 2576 px / 4784 tokens high-res; oversized `tool_result` images are **rejected**, not resized, and the guidance is to resize client-side and publish the resized size | no envelope field — the application tracks it |
| Playwright MCP | none; 1280×720 headless viewport default; `scale: css\|device` | no; coordinate tools are raw CSS px and the 1:1 claim is asserted, never qualified by `deviceScaleFactor` |
| Chrome DevTools MCP | none by default; opt-in `--screenshot-max-width`/`-height` via `clip.scale`; 2 MB inline→file spill | no; the response text is one of three fixed strings with no dimensions and no "downscaled from N to M" |

**Ruling.** No pixel cap, no byte cap, no quality knob, no `max_px` argument, no automatic
downscale — not in 0.8.0 and not by a quiet later addition. Adding one is an amendment to
this file that names A1 and states how A4's `scale` absorbs it. A2 and A4 exist so that
such an amendment is the only way it can happen.

**Acceptance — `tests/claims.rs::no_pixel_cap_exists_and_none_is_documented`:** walk
`tools/list`; fail if any tool's `inputSchema` declares a property named
`maxPx|maxPixels|maxWidth|maxHeight|downscale|quality|jpegQuality|resize`; then fail if any
markdown file under `docs/` or `README.md` contains `downscal`, `capped at` or `shrinks the
image`. **Red on `abba9b1`** because `tests/claims.rs` does not exist (S1.9 creates it), so
the assertion has never been made. The behaviour is already correct — that is the point:
this item is a ratchet, and its cost is the hour it takes to install it.

**Effort** 0.25 d. **Touches** `tests/claims.rs` (new, shared with S1.9), one sentence in
`docs/GUIDE.md`'s screenshot section naming the absence.

---

## A2 — Every pixel-bearing schema property declares its unit

**Extends** S1.3 (the coordinate-space paragraph), S1.4 and S1.8, and §4's contract table.
**Supersedes** the observable status quo, which is that no schema states a unit anywhere:
`screenshot.size` is two integers (`src/tools.rs:47`), `visual.bbox` is `object|null`
(`:313`), `maskRects` is four numbers (`:299-302`), `assert_layout.box` is four numbers
(`:254-257`), and `layout`'s geometry is prose-only in the description (`:274`) and
unitless in the schema (`:281-285`). The coordinate spaces are real and documented in Rust
comments (`src/layout.rs:68-70`); they are simply not visible to the client that consumes
the JSON.

**Changes.**

1. A **closed set of three tokens**, each one line of arithmetic:
   - `css-px` — CSS pixels, viewport-relative (live scroll removed). Unaffected by
     `deviceScaleFactor`.
   - `page-px` — CSS pixels, document-relative (scroll included): `documentBox`.
   - `device-px` — pixels of the decoded PNG: `device-px = css-px × deviceScaleFactor`.
     Deterministic capture pins DSF 1 (`src/capture.rs:433-439`) so there the two
     coincide; a plain capture inherits the live DSF and reports it (`src/capture.rs:552`);
     an attached engine that ignores the override reports its own instead of assuming.
   A fourth token, or a bare `px`, is an amendment to this file.
2. The annotation is a **schema keyword** on the outermost pixel-bearing property, and
   nested geometry inherits the nearest annotated ancestor. One optional companion,
   `unitOverrides`, names fields inside that subtree that are in a different unit:
   ```jsonc
   "size":        {"type": "object", "unit": "device-px", ...}
   "viewport":    {"type": "object", "unit": "css-px"}
   "bbox":        {"type": ["object", "null"], "unit": "device-px"}
   "maskRects":   {"type": "array", "unit": "device-px", ...}
   "box":         {"type": "object", "unit": "css-px"}
   "documentBox": {"type": "object", "unit": "page-px"}
   "elements":    {"type": "array", "unit": "css-px",
                   "unitOverrides": {"documentBox": "page-px"}}
   ```
   `unit` and `unitOverrides` are JSON Schema *annotation* keywords: validators ignore
   keywords they do not know, so no client can break, and `additionalProperties:false`
   constrains instances rather than keywords. The reason it lives here and not in prose is
   the acceptance test — prose cannot be asserted over `tools/list`, and a unit that is
   not machine-readable is a unit the next contributor will restate from memory.
3. The assignment, surface by surface:

   | Tool | Property | Unit | Why that one |
   |---|---|---|---|
   | `screenshot` | `size` | `device-px` | IHDR of the returned PNG (`src/capture.rs:140-143`, `:356-364`) |
   | `screenshot` | `viewport` | `css-px` | live `innerWidth`/`innerHeight` read-back |
   | `visual` | `size`, `bbox`, `regions[]` | `device-px` | the diff canvas is the natural image (`src/visual.rs:603-606`) |
   | `visual` | `maskRects` (argument), `maskRectsApplied` | `device-px` | masks are tested inside that same pixel loop (`:622-628`) |
   | `visual` | `viewport` | `css-px` | the capture viewport |
   | `assert_layout` | `box`, `targets[].box` | `css-px` | `DOM.getBoxModel` minus the live scroll (`src/layout.rs:68-70`) |
   | `assert_layout` | `targets[].documentBox` | `page-px` | the pre-scroll box, same source |
   | `layout` | `elements[].rect`, `elements[].box` | `css-px` | viewport rect and scrolled box (`src/layout.rs:309-358`) |
   | `layout` | `elements[].documentBox` | `page-px` | pre-scroll box |
   | `assert_layout` | `viewport`, `matrix` arguments (S4.4) | `css-px` | the override is CSS px (`src/tools.rs:5363-5420`) |

   `maskedPixels` and `totalPixels` are counts of pixels, not coordinates, and a count is
   in the unit of the rects that produced it — stated once in the description, never
   annotated.
4. Each description gains one sentence naming its unit and how `deviceScaleFactor` moves
   it. The sentence is for the human; the token is what the test holds.

**Acceptance — `tests/claims.rs::every_pixel_property_names_its_unit`:** walk every tool
in `tools/list`; for each property in `inputSchema` and `outputSchema` whose name matches
`^(bbox|rect|box|documentBox|viewport|size|region|regions|maskRects|clip|crop)$`, or whose
subschema declares all of `x, y, w, h`, assert a `unit` key exists and its value is in the
closed set; for the four pixel-bearing tools (`screenshot`, `visual`, `layout`,
`assert_layout`) assert at least one token appears at all. **Red on `abba9b1`** because
`unit`, `css-px`, `page-px` and `device-px` appear nowhere in `src/` (`rg` exits 1) and
`tests/claims.rs` does not exist.

**Effort** 1.0 d. **Touches** `src/tools.rs` — `screenshot` (`:43-50`), `assert_layout`
(`:248-272`), `visual` (`:289-330`), `layout` (`:281-285`) — the coordinate paragraph in
`docs/GUIDE.md`, and `tests/claims.rs`. **Depends on** nothing; lands before A4, which
adds a field to the same schemas.

---

## A3 — `maskRects` is metered in the unit it is evaluated in, and a DSF mismatch is typed

**Sharpens S1.3's last bullet and §11 ruling 5.** This item does **not** restate S1.3 and
creates no second implementation item. S1.3 already specifies `maskRectsApplied` with
clipping and `mask_rect_dropped`, `maskedPixels` as a union count, `maskCoverageRatio`, the
`> 0.10` coverage warning, device-pixel evaluation, and a `mask_scale_mismatch` string in
`warning`. All of that stays S1.3's, lands in S1.3's commit, and keeps S1.3's 1.5 d. A3
adds exactly three things to that commit:

1. **The unit declaration.** `maskRects` and `maskRectsApplied` carry `unit: "device-px"`
   under A2, and the description says the arithmetic the caller must apply at DSF≠1:
   `deviceRect = cssRect × deviceScaleFactor`.
2. **`maskScale`**, present whenever `maskRects` is non-empty on a compare:
   `{"deviceScaleFactor": <number>, "requestedIn": "device-px", "evaluatedIn": "device-px"}`.
   It echoes the measurement the masks were evaluated against, so a caller who passed CSS
   rects at DSF 2 can see the number that says their mask landed at half size without
   re-reading prose.
3. **`warningCodes: string[]`** — a closed set of machine-readable warning tokens, whose
   first and only 0.8.0 member is `mask_scale_mismatch`. Present whenever the read-back
   `deviceScaleFactor` is not 1 and `maskRects` is non-empty. The prose `warning` keeps
   S1.3's sentence, unchanged; the token is the typed half of the same event. The set is
   closed and additive-safe: nothing is ever removed, a client that ignores the field sees
   no change, and a test can branch on it — which is what "typed" has to mean while
   `warning` remains a string. Later warnings may migrate into the list without an
   amendment.

No refusal, and this is deliberate: a DSF≠1 capture with masks is measured, metered and
flagged. The counts stay true (they are device-px quantities of the image that was
actually diffed); the geometry is what the caller must correct.

**Acceptance — `tests/visual_truth.rs::mask_scale_mismatch_is_typed_not_silent`:**
(a) compare with one `maskRects` entry at DSF 1 — `maskScale.deviceScaleFactor == 1.0`,
`warningCodes` contains no `mask_scale_mismatch`, and every entry of `warningCodes` is in
the documented set; (b) the classifier as a pure function, fed a read-back DSF of 2.0 and
one mask rect, returns the token and the multiply-by-two sentence. (b) is the honest
coverage: deterministic capture pins DSF 1, so a *live* DSF≠1 compare needs an attached
engine that ignores the override, and the classifier is the part a test can hold to
account. **Red on `abba9b1`** because `maskScale`, `warningCodes` and all mask fields are
absent — `src/visual.rs:1084-1110` is the entire reply — and no classifier exists.

**Effort** 0.25 d, inside S1.3's commit. **Touches** the reply block S1.3 already touches
(`src/visual.rs:1097-1130`), the same schema (`src/tools.rs:305-329`),
`tests/visual_truth.rs`. **Depends on** S1.3; must not be scheduled as separate work.

---

## A4 — Every image-bearing reply reports its scale, even when it is 1.0

**Extends** S1.1's discipline — two questions, two booleans, and no summary that can
disagree with either — to dimensions. **Supersedes** today's silence: `"scale"` has zero
hits in `src/`, `visual save` returns `viewport` and `bytes` but no image dimensions at all
(`src/visual.rs:385-395`), and nothing in any reply states the ratio between the pixels
that were captured and the pixels that are described.

**Changes.**

1. **`scale: number`** — returned-image pixels per CSS pixel. Today `scale` equals
   `deviceScaleFactor` on every path, because no downscale exists (A1). The field is
   emitted anyway, at 1.0, so that a future cap cannot become silent by construction: if
   one is ever added by amendment, `scale` folds the downscale in
   (`scale = deviceScaleFactor × downscaleFactor`) and `size` shrinks with it. The two
   move together or the reply is a bug.
2. **`size {width, height}`** on every reply that returns or describes a captured image,
   in `device-px` under A2. `screenshot` already has it (`src/capture.rs:140-143`) and
   `visual compare` already has it (`src/visual.rs:1105`). **`visual save` does not**
   (`:385-395`) and gains it. `assert_layout`'s `screenshotOnFail`/`screenshotPath`
   sub-object returns path, bytes and checksum only (`src/tools.rs:247`) and gains
   `size` + `scale`.
3. **The name is `size`, not `image`.** The proposal this amendment came from said
   `image{w,h}`. A second name for the same two numbers is the "no third field" defect
   S1.1 forbids for verdicts, and it is worse for dimensions, which get multiplied. `size`
   stays canonical; if a later amendment prefers `image`, it is a **rename** of `size` in
   one commit — never an addition.
4. **The invariant is asserted, not documented.** For a non-full-page capture,
   `size.width == round(viewport.width × scale)` and the same for height. For `fullPage`,
   the width relation still holds and the height is the document height times `scale`.
   Full-page is the one case where `scale` cannot be derived from `size`, which is exactly
   why it must be reported rather than inferred.

**Acceptance — `tests/claims.rs::pixel_replies_report_their_scale`:** call `screenshot`
(plain, deterministic, and `fullPage:true`), `visual save`, `visual compare`; for each
reply assert `scale` is a finite number `> 0`, `size.width`/`size.height` are positive
integers, `scale == deviceScaleFactor`, and the invariant in (4). **Red on `abba9b1`**
because `scale` is absent from every reply and from both `outputSchema`s
(`src/tools.rs:43-50`, `:305-329`), and `visual save` has no `size` at all.

**Effort** 0.5 d. **Touches** `src/capture.rs:136-159` (`meta()`), `src/visual.rs:385-395`
(save reply) and `:1097-1110` (compare reply), `src/tools.rs:43-50` and `:305-329`
(schemas — a field emitted but undeclared is a conformance failure, so schema and value
land in the same commit), `docs/GUIDE.md`. **Depends on** A2 (the token the new fields are
declared in).

---

## A5 — Detail is a native-resolution crop, never an upscaled preview

**Supersedes** nothing, and corrects the premise it arrived with. The premise was that
gladius already supports a region crop on `screenshot`. **It does not.** `rg clip src/`
finds no capture clip — only comments about clipping a *future*; `rg '"region"' src/`
exits 1; both capture call sites pass `plain_params(...)`, which is
`{format:"png", fromSurface:true, captureBeyondViewport?}` and nothing else
(`src/capture.rs:320-326`, `:410`, `:484`). `src/layout.rs:68-70` mentions "a screenshot
clip" in a comment about coordinate spaces, and that is the only occurrence in the tree.

**What is true, and therefore what this item records.**

1. **No upscale path exists anywhere.** `rg -i "resize|downscal|upscal|zoom|magnif" src/`
   exits 1; no tool takes a `scale`, `zoom` or `quality` argument; no returned image is
   ever resampled. The prohibition is a ratchet, not a change.
2. **Detail today comes from moving the camera, at native resolution.** `screenshot
   {width,height}` (`src/tools.rs:41`, floored at 1 in `src/capture.rs:431-432`) and
   S4.4's `assert_layout {viewport}` re-render the page at the requested size and return
   its pixels at DSF 1. That is a native re-render of a smaller frame, **not** a crop of a
   larger one: reflow changes what the region's layout is, and the docs will not conflate
   the two.
3. **The crop lever is named and not built.** `screenshot {region: {x, y, w, h}}`, mapped
   to a CDP `clip` in `device-px`, is the missing half: ~0.5 d, unversioned, out of 0.8.0.
   S5.2's `regions` is a per-region **diff** readout on `visual` and must never be
   presented as a crop.
4. **The rule, binding on every future item.** When the crop arrives it returns native
   pixels at DSF 1: `size` equals the requested rect times the DSF exactly, `scale` reports
   the total factor per A4, and no tool may ever return a preview enlarged to look
   detailed. An upscaled preview is a fabricated sample — it spends tokens to invent pixels
   that were never rendered, and it makes every coordinate derived from it wrong in a way
   no field reports.

**Acceptance — `tests/claims.rs::detail_is_a_crop_never_an_upscale`:** (a) walk
`tools/list` and fail if any `inputSchema` declares `zoom|upscale|magnify|imageScale`
(the legitimate `viewport.deviceScaleFactor` is not in that set and is not being removed);
(b) assert `docs/GUIDE.md` contains the sentence "detail is a native-resolution crop, never
an upscaled preview". **Red on `abba9b1`** because no file under `docs/` contains that
sentence (`rg -i "upscal" docs/` exits 1 outside the frozen audit prose) and
`tests/claims.rs` does not exist.

**Effort** 0.25 d. **Touches** `docs/GUIDE.md` and `tests/claims.rs`. The "named, not
built" paragraph above is the record; when the crop lands, it is a new item in a new
amendment, not an edit to this one.

---

## Totals, and what this amendment does not do

| Item | Change | Days |
|---|---|---|
| A1 | The verified absence of a pixel cap, turned into a ratchet | 0.25 |
| A2 | A closed unit vocabulary, annotated in the schemas and asserted over `tools/list` | 1.00 |
| A3 | `maskScale` plus a typed `mask_scale_mismatch` token — inside S1.3's commit | 0.25 |
| A4 | `scale` everywhere, `size` where it is missing, and the invariant asserted | 0.50 |
| A5 | The no-upscale prohibition, and the correction that no crop exists yet | 0.25 |
| | **Total (5 items)** | **2.25** |

This amendment adds **no** Rust dependency (D6 untouched), **no** tool, **no** error `code`
— `mask_scale_mismatch` is a warning *token* in a warning list, not a member of the closed
`code`/`reason` set in `SPEC.md` §4.1 — and **no** change to any of the 31 frozen items'
contracts. It edits neither `SPEC.md` (hash unchanged) nor `FREEZE.md` nor `PLAN.md`. It
sets no effort against S1.3, which is still 1.5 d and still owns the mask accounting it
already specifies.
