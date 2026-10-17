# Third-party notices

Gladius itself is MIT — see [LICENSE](LICENSE). No third-party browser binary
ships in the release archives: the pinned browser is fetched from Google's
Chrome-for-Testing endpoint at provision time and never redistributed.

## Chromium via Chrome for Testing (`chrome-headless-shell`, fetched at provision time)

- **What:** the owned headless-only browser gladius drives over CDP
  (Chrome DevTools Protocol). It is launched windowless from its versioned
  cache unless `GLADIUS_CHROMIUM` points elsewhere or `GLADIUS_CDP_URL`
  attaches to an external engine instead.
- **Upstream:** <https://developer.chrome.com/docs/automation-and-testing/chrome-for-testing>
  (asset `chrome-headless-shell-win64.zip`, pinned `155.0.8059.84`)
- **Release provisioned:** `155.0.8059.84`, fetched and sha256-verified by
  [`scripts/fetch-chromium.ps1`](scripts/fetch-chromium.ps1) (zip, per-file,
  and size pins live in `scripts/chromium-manifest.json`).
- **License:** Chromium BSD-style licence. The attribution copy is carried in
  every runtime bundle at `THIRD-PARTY-LICENSES/LICENSE.headless_shell`
  (copied from the provisioned, sha-verified browser cache); the browser
  itself is never bundled.
