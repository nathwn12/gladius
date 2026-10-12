# Obscura capability record

Owned Obscura engine as the gladius default and only launch. Shipped at HEAD
`f9f3d6b` (on `43e35d2`) as the default alongside a chrome opt-in; that
opt-in was removed on 2026-10-12 (self-containment: gladius launches nothing
it does not own). What remains: the owned launch below, plus `GLADIUS_CDP_URL`
attaching to an external engine gladius never kills. `bin\obscura.exe` plus
`bin\obscura-worker.exe` are provisioned by `scripts/fetch-engine.ps1`
(pinned v0.2.4, sha256-verified) and ship beside `gladius.exe`.

Gladius launches the engine as:

```
obscura.exe serve --host 127.0.0.1 --port <free> --allow-private-network --allow-file-access
```

spawned with `CREATE_NO_WINDOW`, with readiness discovered by polling
`GET http://127.0.0.1:<port>/json/version` for `webSocketDebuggerUrl`.
Both gate flags are MANDATORY and measured (section 2).

Evidence sources (read, not re-derived here):
`obscura-report.md`, `obscura-result.json`, `supp-result.json`,
`launch-flag-matrix.md`, `launch-flag-matrix.json`,
`obscura-default-proof-summary.json`, `obscura-default-proof-result.json`,
plus control arm `baseline-result.json` / `baseline-summary.json`.

## 1. Backend selection

Strict two-way order:

1. `GLADIUS_CDP_URL` set (non-blank) -> attach to the external engine instead
   of launching. A `ws://` (`wss://`) value connects directly; an `http://`
   (`https://`) value resolves the ws endpoint via the documented readiness
   endpoint `GET <base>/json/version` (`webSocketDebuggerUrl`). Ownership rule:
   the attached engine is NOT ours. `close` detaches the CDP session gladius
   opened (`Target.detachFromTarget`) and never calls `kill_tree`, never kills
   a pid, never sweeps a profile. The external process stays alive; gladius
   launches nothing.
2. Anything else -> the owned Obscura engine, the only launchable engine.
   Resolution: `GLADIUS_OBSCURA` wins when set and non-blank, otherwise
   `obscura.exe` as a sibling of the running `gladius.exe` (engine ships
   beside the binary).

A missing Obscura engine is a clear error naming the provision script, never a
silent fallback to Chrome:

- explicit override pointing nowhere:
  `obscura engine not found: GLADIUS_OBSCURA='<path>' is not a file; run
  scripts/fetch-engine.ps1 to provision the pinned engine`
- no sibling beside the running binary:
  `obscura engine not found: expected <path> beside the running gladius
  executable; run scripts/fetch-engine.ps1 to provision the pinned engine`

## 2. Why the two gate flags are mandatory

Each flag appears EXACTLY ONCE on the serve argv: the engine's clap parser
hard-fails on a repeated flag
(`error: the argument '<flag>' cannot be used multiple times`).

Without `--allow-private-network`, `Page.navigate` to a loopback URL fails:

```
-32000 Network error: Access to private/internal IP address 127.0.0.1 is not allowed
```

A LAN address behaves the same (measured against `http://192.168.1.187:<p>/`):

```
-32000 Network error: Access to private/internal IP address 192.168.1.187 is not allowed
```

Without `--allow-file-access`, `Page.navigate` to a `file://` URL fails:

```
-32601 Page.navigate to file:// is disabled. Restart with `obscura serve --allow-file-access` to enable.
```

and `DOM.setFileInputFiles` is refused on the identical node path:

```
DOM.setFileInputFiles is disabled. Restart with `obscura serve --allow-file-access` to enable local file uploads.
```

(code `-32601` in both cases).

These gates apply at the NAVIGATION layer: the blocked request never leaves
the engine (no marker-server hit, no `responseReceived`, an all-white
1-color screenshot). Passing the flags re-opens the engine's SSRF protection
and allows local-file reads. That is a deliberate trade for parity with a
loopback-bound (`127.0.0.1`) CDP engine, not an
accidental hole: without the flags, local test fixtures over loopback and
`file://` simply cannot load.

## 3. Per-tool capability matrix

Measured with both gate flags passed. The live default-engine run drove every
tool successfully (20/20 steps in `obscura-default-proof-result.json`, no
capability-gated failure), and the flag-matrix run proved the gated paths both
ways.

| tool | verdict | reason |
|---|---|---|
| `navigate` | supported | warm-session round trip works; see the blank-load race (section 5), surfaced as a `warning` field, not a failure |
| `snapshot` | supported | AX snapshot with stable refs works against the engine |
| `screenshot` | supported | viewport and full-page (`captureBeyondViewport`) PNGs are genuine render pixels, not placeholders |
| `click` | supported | `Input.dispatchMouseEvent` path works |
| `type` | supported | `Input.dispatchKeyEvent` path works |
| `evaluate` | supported | `Runtime.evaluate` works, awaiting promises; slower than the control arm (section 6) |
| `wait_for` | supported | selector/text/delay/network-idle waits work |
| `console` | degraded | the tool works, but its `Log.entryAdded` leg is BLIND: zero `Log.entryAdded` events ever observed despite `Log.enable`, across a 19-event load-error window and a 31-event main-run window; `Runtime.consoleAPICalled` still flows, so console output is captured through the Runtime leg only |
| `tabs` | supported | `Target.createTarget` yields genuinely separate pages, `activateTarget`/`closeTarget` (`{"success": true}`) work per target |
| `close` | supported | kills the owned engine tree; on an attached engine (`GLADIUS_CDP_URL`) detaches only and leaves the process running |
| `upload` | gated | hard-requires `--allow-file-access`: without it, `DOM.setFileInputFiles` is refused (verbatim error, section 2); gladius always passes the flag, and the exact gladius DOM chain (`DOM.getDocument` -> `DOM.querySelector` -> `DOM.setFileInputFiles`) functions as written (node quirk: `DOM.getDocument` root `nodeId` is 0 even on populated pages, but `querySelector` from 0 works) |

JS dialogs are INERT (not a hang, not an error): `Page.javascriptDialogOpening`
never fires and `Page.handleJavaScriptDialog` returns `Unknown Page method`
(`-32601`). `alert()`/`confirm()`/`prompt()` are silent no-ops (alert returns
undefined, confirm true, prompt null), so nothing ever blocks waiting for a
dialog that cannot be handled. Auto-accept/auto-dismiss dialog flows are
impossible against this engine.

## 4. Known engine quirks that affect operation

- clap hard-fails on a repeated flag
  (`error: the argument '<flag>' cannot be used multiple times`), so gladius
  builds the serve argv with each flag exactly once.
- The server drops idle browser WS connections at ~60-70 s (recv thread
  death). Gladius connects lazily on first tool use and bounds every wait, so
  an idle gap does not strand a session.
- The `--ready-file` atomic publish flakes (observed twice: engine serving
  with no file published). Gladius never trusts the ready file and discovers
  the endpoint over `GET /json/version` (`webSocketDebuggerUrl`) instead.
- The engine double-fetches documents (two `requestWillBeSent` per navigate);
  the losing request can surface as a synthetic Document `404` with
  `encodedDataLength` 0 and commit a blank page. Gladius surfaces this as a
  `warning` field on `navigate` (section 5).

## 5. The blank-load race

What a caller observes: `navigate` returns the correct committed URL, but the
document is empty - empty title and empty body text - and a screenshot is an
all-white 1-color PNG. The envelope carries a `warning` field in exactly this
shape:

```json
{"url": "<committed url>", "title": "",
 "warning": "possible blank-load race: committed URL with empty document (empty title and empty body text); re-navigate to confirm"}
```

What to do about it: re-navigate to confirm. The flake is retryable - retry of
the identical bytes succeeds (pixel run attempt 1, navprobes). Detection costs
one extra eval, and only on suspicious pages (non-blank committed URL with an
empty title).

No retry magic exists in gladius: it reports the `warning` and stops there. Do
not claim any automatic recovery.

## 6. Head-to-head numbers vs the historical control arm

Control arm: `baseline-result.json` / `baseline-summary.json` (spawn-to-ready
via the stderr ready line, spawn-to-response navigate wall; that engine path
has been removed - numbers kept for provenance, never as a live option).
Obscura arm:
`obscura-result.json` (ready-file publish timing, warm-session round trips),
`supp-result.json` (corrected console drain), `launch-flag-matrix.json`.
Definitional mismatches are named per row, never suppressed.

| metric | control (historical arm, path removed) | Obscura | reading |
|---|---|---|---|
| process tree (WorkingSet sum) | 7 procs / 189,063,168 B | 2 procs (`obscura.exe` + its windowless `conhost.exe`) / 44,920,832 B | like-for-like, ~4.2x smaller |
| profile+storage churn | 1,755,014 B across 47 files | 0 B / 0 files (explicit `--storage-dir` run wrote nothing; default runs show no profile bytes anywhere) | like-for-like |
| screenshot median (viewport) | 30.4 ms | 17.8 ms | like-for-like, ~1.7x faster |
| console drain | 0.1 ms median | <0.03 ms immediate-drain (all five corrected samples; the main-run 200.6 ms figure was the harness's own 0.2 s drain wait, corrected and disclosed in `supp-result.json`) | like-for-like |
| engine ready median | 99 ms | 55.7 ms | DIFFERENT TRIGGER: stderr ready line (control) vs ready-file atomic publish (Obscura). Faster, but not the same stopwatch |
| evaluate median (`1+1`) | 0.4 ms | 0.71 ms | like-for-like, and Obscura is SLOWER |
| snapshot | 0.7 ms median | 1.40 ms median | NOT COMPARABLE: the Obscura figure is a 2-eval proxy, not the snapshot tool |
| navigate | 133 / 143.9 / 208.6 ms (min/median/max) | 6.5 / 7.4 / 8.1 ms (min/median/max) | NOT COMPARABLE: the control figure is spawn-to-response wall, the Obscura figure is a warm-session round trip. No navigate speedup is claimed |

The navigate row is an explicit refusal: presenting those two columns as a
speedup would compare cold spawn against a warm socket. They are reported here
only so nobody else makes that comparison.
