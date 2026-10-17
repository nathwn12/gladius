# gladius-mcp

MCP browser-use server: a native Rust binary driving the owned Obscura
engine over CDP, served over stdio. This package is a dependency-free Node
launcher plus a pinned engine manifest — it contains no engine bytes.

## Run

```sh
npx gladius-mcp@latest
```

Or pin it in an MCP client as a stdio server with command `npx` and args
`["gladius-mcp@latest"]`.

First run provisions the pinned runtime (server + engine, ~160 MB download)
into `%LOCALAPPDATA%\gladius\engine\<serverVersion>-<bundleSha8>\`, verifies
every sha256 in `engine-manifest.json` before and after extraction, and
reuses that directory on later runs. All launcher chatter goes to stderr;
stdout belongs to the MCP channel.

## Diagnostics (no network)

```sh
npx gladius-mcp@latest --doctor
```

Prints the pinned manifest and the resolved engine cache path, exits 0,
performs no fetch. `--print-manifest` is an alias.

## Requirements

- Windows x86_64. Any other platform refuses at startup
  (override for diagnostics only: `GLADIUS_MCP_ALLOW_NON_WINDOWS=1`).
- Node.js >= 22.14.0 (global `fetch` plus `crypto`/`zlib` builtins only;
  zero dependencies).

## Engine provenance

The engine is Obscura `v0.2.4` (Apache-2.0,
https://github.com/h4ckf0r0day/obscura), repackaged with its licence into
the release archives. The in-package manifest pins the upstream archive
hash, both engine binary hashes, and the runtime bundle hash the release
workflow computed from the real build artifacts.

Redistribution note for the owner: the release archives redistribute a
third-party engine binary. Confirm the redistribution question BEFORE the
first public release — see `docs/PUBLISHING.md` in the repo.

## License

MIT — see LICENSE.
