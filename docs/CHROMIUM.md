# Chromium pin record (chrome-headless-shell)

Gladius ships no browser binary in git. The browser is acquired at provision time by
`scripts/fetch-chromium.ps1` into `%LOCALAPPDATA%\gladius\chromium\` (overridable,
see below).

## Upstream

- Project: Chrome for Testing: `chrome-headless-shell` (headless-only //content wrapper)
- Channel: Stable, resolved to an explicit version (a channel name is NOT a pin)
- Resolved from: https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions.json
- Version: `155.0.8059.84`
- Asset: `chrome-headless-shell-win64.zip`
- Asset URL: https://storage.googleapis.com/chrome-for-testing-public/155.0.8059.84/win64/chrome-headless-shell-win64.zip

## Pin

- Zip sha256: `5ca590a4968864e219ea25ccd134597ef8898a733a4698c07fc3e8b2c480fef4`
- Zip size: `123681250` bytes
- Zip contents: exactly `288` files under `chrome-headless-shell-win64\`, total `290511280` bytes.
- `chrome-headless-shell.exe`: size `217146368`, sha256 `64db9686dab042ba3282f7ab6fef98de266b015fd4d056c30760244425091d39`
- Every file (names are zip-relative, forward slashes) is listed with sha256 + size in
  `scripts/chromium-manifest.json`.

## Why the manifest is ours

The upstream CfT JSON endpoints carry NO sha256 field (0 matches for sha256 across
`known-good-versions-with-downloads.json`). The zip cannot be verified against upstream,
so the manifest is our integrity root: its sha256 values were computed BY US from the
real downloaded archive (download via `Invoke-WebRequest`, hash the zip with
`Get-FileHash` SHA256, extract with `Expand-Archive`, hash every extracted file).
`scripts/fetch-chromium.ps1` verifies the zip AND every listed file (exact set, no extras)
before provisioning, and re-verifies on reuse. Keep the manifest, the script constants,
and the pinned version in lockstep; the script refuses on any drift and writes nothing.

## Cache layout and re-provision

- Root: `%LOCALAPPDATA%\gladius\chromium\` (override with `GLADIUS_CHROMIUM_CACHE`).
- Each pin lives at `<version>-<zip-sha8>\`, e.g. `155.0.8059.84-5ca590a4\`.
- Inside: the extracted `chrome-headless-shell-win64\` tree plus
  `chrome-headless-shell-win64\chrome-headless-shell.exe`.

Provision locally with:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\fetch-chromium.ps1
```

The script is idempotent: if the exe tree already matches the pin it prints
`already present, verified` and changes nothing. It accepts an optional
`-SourcePath <zip>` that skips the download and verifies a local zip instead
(useful for offline or test use); the default path always downloads the pinned asset.
It refuses on any hash/size mismatch and writes nothing.

Force a re-provision by deleting the versioned cache dir (or re-run with `-Force`):

```powershell
Remove-Item -Recurse -Force "$env:LOCALAPPDATA\gladius\chromium\155.0.8059.84-5ca590a4"
powershell -ExecutionPolicy Bypass -File scripts\fetch-chromium.ps1
```

This script never reads, writes, or launches an ambient or shared browser and never
touches the installed Brave/Edge/Chrome profiles.

## Update path

Check without downloading anything:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\fetch-chromium.ps1 -Check
```

prints the pinned version, the newest Stable version resolved from
`last-known-good-versions.json`, and whether an update is available. Exit 0.

Bump when Stable moved:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\fetch-chromium.ps1 -Update
```

resolves the newest Stable (or an explicit `-Version <v>` override for exercising the
path), and if it differs from the pin: downloads it, recomputes the ENTIRE manifest
from the real files (version, download URL, zip sha256 + size, file count, total size,
every per-file name/sha256/size), rewrites `scripts/chromium-manifest.json`, updates the
pin constants in `scripts/fetch-chromium.ps1`, places the new versioned cache dir, and
prints `old -> new`, file count, total size delta, and the zip hash. If it equals the
pin it is a NO-OP: the manifest stays byte-identical (no churn, no rewrite).

Monthly PR workflow: `.github/workflows/browser-update.yml` (`schedule:` monthly plus
`workflow_dispatch`) runs `-Check` then `-Update` on a GitHub-hosted runner and, if the
manifest/script changed, opens or updates a pull request titled for the browser bump.
It NEVER merges, never tags, never publishes.

Rule: a bump lands through the suite and the e2e before any release.

## Licence posture

DECISION (owner can override): the browser is FETCHED from Google's endpoint at
provision time and is NEVER redistributed in our release assets. The release workflow
repacks only the Obscura engine, never the chromium tree.

- The manifest is our integrity root, not a licence.
- The shipped `LICENSE.headless_shell` (inside the CfT zip, recorded in the manifest)
  is carried for attribution if anyone ever bundles the browser.

Open question, stated plainly: no primary statement of CfT redistribution terms was
found, which is why we do not redistribute.
