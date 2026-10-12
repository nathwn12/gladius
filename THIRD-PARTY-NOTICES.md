# Third-party notices

Gladius itself is MIT — see [LICENSE](LICENSE). The distributable release zip
bundles the following third-party component beside `gladius.exe`:

## Obscura browser engine (`obscura.exe`, `obscura-worker.exe`)

- **What:** the owned headless browser engine gladius drives over CDP
  (Chrome DevTools Protocol). It is launched windowless beside `gladius.exe`
  unless `GLADIUS_OBSCURA` points elsewhere or `GLADIUS_CDP_URL` attaches to
  an external engine instead.
- **Upstream:** <https://github.com/h4ckf0r0day/obscura>
- **Release provisioned:** `v0.2.4`, fetched and sha256-verified by
  [`scripts/fetch-engine.ps1`](scripts/fetch-engine.ps1) (zip, binary, and
  size pins live in that script).
- **License:** Apache-2.0, per the upstream project. The full license text is
  not reproduced here — see the upstream repository.
