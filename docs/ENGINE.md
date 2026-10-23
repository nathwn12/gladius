# Engine pin record (chromium)

Gladius ships no browser binary in git. The browser is acquired at provision time by
`scripts/fetch-chromium.ps1` into `%LOCALAPPDATA%\gladius\chromium\` (overridable,
see below). The full pin (upstream, version, hashes, cache layout, update path)
lives in `docs/CHROMIUM.md`; this document records how gladius runs it.

## Resolution

`find_chromium()` in `src/browser.rs`, in strict order:

1. `GLADIUS_CHROMIUM` set and non-blank: that path (must be a file).
2. Otherwise the pinned build under the owned cache root:
   `GLADIUS_CHROMIUM_CACHE` when set and non-blank, otherwise
   `%LOCALAPPDATA%\gladius\chromium\155.0.8059.84-5ca590a4\`
   (`chrome-headless-shell-win64\chrome-headless-shell.exe` inside).
3. Otherwise a clear error naming `scripts/fetch-chromium.ps1`.

There is no third step: gladius never falls back to any ambient, shared, or
system browser. `chrome-win64.zip` (the full headed-capable artifact) is never
used.

## How gladius runs it (launch contract)

Headless-only by owner decision (2026-10-17): there is no headed path at all,
and a headed request fails closed with an explicit unsupported error. Each
owned instance launches the pinned `chrome-headless-shell.exe` as:

```
chrome-headless-shell.exe --headless --remote-debugging-port=<free> --user-data-dir=<private> --no-first-run --no-default-browser-check --disable-backgrounding-occluded-windows --disable-renderer-backgrounding --disable-background-timer-throttling --remote-allow-origins=http://127.0.0.1:<port> about:blank
```

each flag exactly once, spawned with `CREATE_NO_WINDOW`, with readiness
discovered by polling `GET http://127.0.0.1:<port>/json/version` for
`webSocketDebuggerUrl`. Backend selection is `GLADIUS_CDP_URL` (attach, never
owned) first, otherwise this Chromium launch is the only engine.

The three `--disable-*` flags are the background-correctness contract: a tab
that is not the active page must still render, still screenshot and still run
its timers, and none of the three touch the OS (no cursor, no keyboard, no
window, no focus). The `selftest` asserts it on the real engine - a capture
taken on a backgrounded tab's own session is byte-identical to the foreground
capture and its `setInterval` keeps firing (25 ticks in 1.2 s).

The profile is functional, not hygiene: since Chrome 136 the
remote-debugging switches are ignored on the default data directory, so a
launch without a private profile binds no port. One fresh private profile
directory per owned instance, created before launch and swept after the tree
is reaped. Never passed: `--no-sandbox`, `--disable-dev-shm-usage`,
keychain/password-store flags, `--remote-debugging-pipe`.

Full capability record: `docs/CHROMIUM.md`.

## Historical note

Before the Chromium swap, gladius drove an owned Obscura engine
(`bin\obscura.exe` + `bin\obscura-worker.exe`, provisioned by
`scripts/fetch-engine.ps1`, pinned v0.2.4). That record is retained for
provenance in `docs/OBSCURA-COMPAT.md` and no longer describes current
behavior: JS dialogs were inert on Obscura (never emitted
`Page.javascriptDialogOpening`) and the `Log.entryAdded` leg was blind.
Both are live on Chromium.
