# HEADED MODE - decision record

Owner requirement, stated 2026-10-10:

> "we promised headless, it should be headless. (however it CAN open up a browser when it NEEDS to,
> the one visible to the user push comes to shove.)"

Status: **designed, not built.** Owner-facing contract is unchanged until it lands.

## The rule

- **Headless is the default and stays the default.** No tool, no argument, and no page can make a
  window appear. Zero behaviour change for existing users.
- **Visible is an explicit opt-in** - armed for the whole process, never per call. An agent cannot
  pop a window mid-task by choosing different arguments.
- **gladius never uses a browser profile it does not own.** The owner's default profile
  (`Brave-Browser-Nightly\User Data`, Chrome's `User Data`) is never passed as `--user-data-dir`.

## Interface

| Kind | Name | Meaning |
|---|---|---|
| env | `GLADIUS_HEADED=1` | run the backend as a visible window |
| cli | `gladius mcp --headed` | same, for a client config entry |
| env | `GLADIUS_CHROME=<path>` | explicit engine override (wins in both modes) |
| env | `GLADIUS_PROFILE_DIR=<path>` | override the headed profile location |

Precedence: `GLADIUS_CHROME` > mode default. Helm only - no MCP tool argument exposes it.

## Engine resolution

`chrome-headless-shell` **cannot display a window** (it is headless-only), and the full Playwright
chromium build was deliberately reclaimed, so headed mode needs a real browser binary.

- **Headless (unchanged):** `GLADIUS_CHROME` -> newest `chrome-headless-shell-*` under
  `%LOCALAPPDATA%\ms-playwright` -> error. Always launched with `--headless`.
- **Headed:** `GLADIUS_CHROME` -> `C:\Program Files\Google\Chrome\Application\chrome.exe` ->
  `C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe` -> error naming what was searched.
- Measured on this machine: Chrome **155.0.8059.39** present, Edge **154.0.4258.62** present.
- **Brave is never auto-selected.** It is the owner's own browser; picking it up by accident is the
  exact failure this design exists to prevent.

## Profile

- **Headless:** throwaway `tempfile::TempDir` (unchanged) - destroyed with the process.
- **Headed:** persistent, gladius-owned: `%LOCALAPPDATA%\gladius\profile-headed`.
  Rationale: if a human has to sign in or clear a captcha once inside the visible window, that
  session must survive the next run; a throwaway profile would make headed mode useless.
- Startup writes the resolved engine path and profile dir to stderr in both modes.

## Launch args (headed)

Drop `--headless`, keep the memory and hygiene flags, add a real window:

```
--disable-gpu --no-first-run --no-default-browser-check --remote-debugging-port=0
--process-per-site --renderer-process-limit=1 --disable-background-networking --disable-sync
--disable-default-apps --mute-audio --window-size=1280,900
--user-data-dir=%LOCALAPPDATA%\gladius\profile-headed
```

## Non-goals

- **Attach mode is BUILT (2026-10-11):** `GLADIUS_CDP_URL` attaches to an
  externally started engine instead of launching (ws:// connects directly,
  http:// resolves via `/json/version`). Ownership rule: the attached engine
  is NOT ours — `close` detaches the session gladius opened and never kills,
  never sweeps. Driving the owner's already-running browser still needs it to
  have been started with a debug port, which cannot be retrofitted.
- No per-call "show me this one page" argument.
- No change to the 13 tool schemas.

## Validation gate

1. **No regression in default mode:** `cargo test --release`, `gladius selftest` ALL PASS, and the
   full `real-world\driver.py` suite re-run **headless** - identical pass set, 0 hangs.
2. **Headless really stays headless:** with no `GLADIUS_HEADED`, assert no visible window exists
   after navigate (Windows: the backend process owns no visible top-level window).
3. **Headed smoke:** `GLADIUS_HEADED=1` -> a visible window appears; `navigate`/`snapshot`/
   `screenshot` work; a value written into a form persists across a restart of the same profile.
4. **Isolation:** in headed mode the process command line contains gladius's own
   `--user-data-dir`, and no launch ever names the owner's default profile path.
