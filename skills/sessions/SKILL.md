---
name: sessions
description: Attach and ensure lifecycle for the gladius browser session, one session per target, and when to use visual versus layout checks.
---

# gladius sessions

gladius drives one browser over CDP. A session is the single page target the
server is currently attached to; every tool call either reuses it or
re-attaches. Treat the session as a scarce resource: one target at a time.

## Attach vs owned engine

- If GLADIUS_CDP_URL is set, gladius attaches to that engine. close
  detaches only the session it opened; your process keeps running and nothing
  of yours is deleted.
- Otherwise gladius launches its owned pinned chrome-headless-shell on the
  first browser-touching tool call, creates one fresh private profile, and
  reaps the whole tree on close or stdio EOF.
- An explicit GLADIUS_CHROMIUM path wins over the cache.

## Session lifecycle

- The server starts no process during initialize; launch happens on the
  first tool that needs the browser.
- ensure_session attaches to an existing page target when one exists, else
  creates about:blank. A dead session is never reused: the next call
  re-attaches and may create a page.
- On a send failure of class session-death, the cached session is dropped and
  the call returns stale session: ...; the command is not retried
  automatically. Repeat it only by choice.
- close terminates the owned tree through the Windows job object and sweeps
  the private profile; on an attached engine it detaches only.

## One session per target

- Only one page target is active at a time. Switching targets invalidates
  the cached session and re-attaches.
- Use targets and tabs to inspect; do not assume a target survives
  navigation or close.
- engine_dead means the connection died: for an owned engine there is one
  automatic relaunch attempt; an attached engine is never restarted by
  gladius.

## Visual vs layout checks

- assert_layout / layout prove geometry (boxes, visibility, overflow,
  computed styles) without pixels. Use them for structural assertions.
- screenshot (deterministic) and visual prove pixels. Use them for
  appearance and regression baselines.
- A deterministic capture freezes CSS/WAAPI/SMIL/media in the main document
  and same-process frames; canvas, WebGL and cross-process iframes are
  sampled, not frozen.
- screenshot with deterministic: true fixes metrics and restores them;
  byte-identity assumes gladius own PNG encoder.
- dom-only backend caps make screenshot and visual refuse with
  capability_gated; geometry checks still work.

## Cleanup

- Always call close (or end the stdio session) so the owned browser tree
  and private profile are reaped.
- On an attached engine, close leaves your process running.
- No PDF generation and no fleet/cloud operation is provided.
