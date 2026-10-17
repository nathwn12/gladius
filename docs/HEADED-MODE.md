# HEADED MODE - decision record (headless-only)

Owner decision, 2026-10-17: **headless-only. Headed mode is not built and is
not planned.**

- The ONLY engine is the pinned `chrome-headless-shell` (a headless-only
  content embedder). The artifact cannot go headed, and that is a property we
  rely on - not a gap.
- Nothing resolves, provisions, or launches a headed browser. There is no
  headed code path at all: no `GLADIUS_HEADED` launch, no `--headed` launch,
  never an automatic fallback and never a gate.
- A headed request fails closed with an explicit unsupported error:
  `GLADIUS_HEADED` set truthy refuses the launch in `launch()`, and a headed
  CLI flag (`gladius mcp --headed`) exits 2 in `main.rs` before anything
  resolves, provisions, or launches.
- The visible path, if the owner ever needs eyes on a page, is an explicit
  external attach (`GLADIUS_CDP_URL`) to an engine the user started
  themselves. Gladius launches nothing, kills nothing, sweeps nothing there:
  `close` only detaches the CDP session it opened.

## Reasoning (kept)

- Headless is the default and stays the default. No tool, no argument, and no
  page can make a window appear.
- Gladius never launches, selects, or profiles anyone else's browser. No
  auto-selected system browser, no owner-profile path, ever.
- Headlessness is asserted from the OS (no visible top-level window exists
  for the instance), because CDP cannot observe window visibility.

## Retired design (not built)

The earlier design below was superseded by the 2026-10-17 decision and is
kept only so nobody re-proposes it:

- `GLADIUS_HEADED=1` / `gladius mcp --headed` as a visible-window opt-in:
  rejected, fails closed instead.
- Visible fallback by attaching to a user-started headed engine with a debug
  port: still true as an attach (see above), but it is not a headed MODE -
  gladius itself stays headless-only.
- "If the owned engine ever gains a headed mode, this record gets a
  launch-args section": moot - the owned artifact is headless-only by
  construction.

## Validation gate

1. **No regression in default mode:** `cargo test --release`, `gladius selftest` ALL PASS.
2. **Headless really stays headless:** assert from the OS that no visible
   top-level window exists for the instance after navigate
   (`tests/headless_chromium.rs`: `headless_browser_has_no_visible_window`).
3. **Headed fails closed:** `GLADIUS_HEADED=1` refuses the launch and a
   headed CLI flag exits 2 (`headless_headed_request_fails_closed` plus the
   `headed_request_fails_closed` unit test).
4. **Isolation:** no launch ever resolves to a binary gladius does not own
   (the rewritten `no_console_flash` guard fails the build on any ambient or
   shared-browser resolution).
