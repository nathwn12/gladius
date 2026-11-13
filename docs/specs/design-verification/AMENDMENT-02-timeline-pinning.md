# AMENDMENT-02 - timeline pinning: a frozen capture is pinned, not paused

> **Status: additive amendment to the frozen [`SPEC.md`](SPEC.md).** `SPEC.md` is not
> edited: its SHA-256 is still
> `FC79D4EBEF973ECFF8083D8F4D7CDA549936F3A87A7E9F35EA77B1DC0C1E12E8`, exactly what
> [`FREEZE.md`](FREEZE.md) records, so the freeze holds. This file is the written
> amendment naming the item it sharpens, as `FREEZE.md` requires.
>
> Read after `SPEC.md` §5 (S1.5). It supersedes **S1.5's second sentence** only. The
> item's acceptance test is unchanged and remains the authority.

---

## The conflict, recorded

S1.5's text says the freeze pauses each animation and media element and **freezes
`currentTime`**. Its own acceptance requires two captures taken 500 ms apart to be
**byte-identical**.

Those two requirements cannot both hold. A timeline paused at its wall-clock position sits
at a different position on each capture, so the second capture paints a different frame.
Reproduced before the fix, on the `abba9b1` baseline: **1853 bytes versus 1643 bytes** for
the same page 500 ms apart — a determinism claim failing on the one test that defines it.

## The implemented reading

Each animation and media timeline is **pinned to its start** for the duration of the
capture and **restored afterwards**: a timeline that was paused keeps its pre-capture
position, a timeline that was playing resumes playing. `freeze.waapiPaused`,
`freeze.smilPaused` and `freeze.mediaPaused` count what was pinned, so the reply still
tells an agent what the freeze reached.

Pinning is what makes the acceptance test true; pausing is what made the claim false.
Where the item's prose and its acceptance disagree, **the acceptance wins**.

## Coverage limit, recorded

Closed shadow roots remain **undetectable from the page realm** — `el.shadowRoot` is
`null` — so `freeze.skipped` names cross-origin iframes only, until S2.5 supplies CDP
ground truth. S1.9 scopes the determinism claim to what the freeze actually reaches and
does not claim otherwise; this amendment records the gap rather than papering over it.

## Ledger

| Item | Relation to the frozen spec | Clause it names |
|---|---|---|
| **S1.5** | Implemented as pin-to-start with restore, not pause-at-position | second sentence ("freeze `currentTime`") |
| **S1.9** | Unchanged: the docs scope the claim to the reachable set | — |

Implementation commit: `13e19c7` — *feat: the deterministic freeze reaches frames, shadow
roots and motion*.
