# HEADED MODE - decision record

Owner requirement, stated 2026-10-10:

> "we promised headless, it should be headless. (however it CAN open up a browser when it NEEDS to,
> the one visible to the user push comes to shove.)"

Status: **designed, not built.** Owner-facing contract is unchanged until it lands.
Update 2026-10-12: gladius is self-contained (the chrome opt-in path is
removed). Gladius itself never launches a headed-capable binary; the
visible-when-needed fallback is an explicit external attach
(`GLADIUS_CDP_URL`) to an engine the user started themselves.

## The rule

- **Headless is the default and stays the default.** No tool, no argument, and no page can make a
  window appear. Zero behaviour change for existing users.
- **Visible is an explicit opt-in** - armed for the whole process, never per call. An agent cannot
  pop a window mid-task by choosing different arguments.
- **Gladius never launches, selects, or profiles anyone else's browser.**
  No auto-selected system browser, no owner-profile path, ever. A visible
  engine is always one the user started and handed over by URL.

## Interface

| Kind | Name | Meaning |
|---|---|---|
| env | `GLADIUS_HEADED=1` | reserved: run as a visible window (not built) |
| cli | `gladius mcp --headed` | reserved: same, for a client config entry (not built) |
| env | `GLADIUS_OBSCURA=<path>` | explicit engine override for the owned headless launch |
| env | `GLADIUS_CDP_URL=<url>` | BUILT: attach to an externally started engine instead of launching - this is the visible path today (start a headed engine with a debug port, hand gladius the URL) |

Helm only - no MCP tool argument exposes any of it.

## Engine resolution

- **Headless (the only launch gladius performs):** `GLADIUS_OBSCURA` ->
  `obscura.exe` beside `gladius.exe` -> error naming
  `scripts/fetch-engine.ps1`. The owned engine is always launched windowless
  (`CREATE_NO_WINDOW`).
- **Visible (no gladius launch at all):** `GLADIUS_CDP_URL` attaches to an
  engine the user already started (headed or not). Gladius launches nothing,
  kills nothing, sweeps nothing: `close` only detaches the CDP session it
  opened. Driving an already-running visible browser still needs it to have
  been started with a debug port, which cannot be retrofitted.
- Gladius auto-selects no browser, headed or otherwise. There is no
  resolution chain ending in a system install.

## Profile

- **Headless:** the owned engine creates no profile.
- **Visible (attach):** gladius launches nothing, so there is no profile to
  own. Sign-in/captcha state lives in the user's own engine, outside gladius.
- Startup writes the resolved engine path to stderr in both modes.

## Non-goals

- No per-call "show me this one page" argument.
- No change to the tool schemas.
- No gladius-launched headed window: a headed launch would need a
  headed-capable binary gladius does not ship, and reaching for someone
  else's browser violates self-containment. If the owned engine ever gains a
  headed mode, this record gets a launch-args section; until then, attach is
  the whole visible story.

## Validation gate

1. **No regression in default mode:** `cargo test --release`, `gladius selftest` ALL PASS, and the
    visible fallback re-verified via attach (below) - identical pass set, 0 hangs.
2. **Headless really stays headless:** with no `GLADIUS_HEADED`, assert no visible window exists
   after navigate (Windows: the owned-engine process owns no visible top-level window).
3. **Visible smoke via attach:** start a headed engine with a debug port by hand,
   `GLADIUS_CDP_URL=<url>` -> `navigate`/`snapshot`/`screenshot` work against the visible page;
   `close` detaches and the external engine keeps running.
4. **Isolation:** gladius never names the owner's default profile path, and no launch ever
   resolves to a binary gladius does not own.
