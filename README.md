<p align="center">
  <img src="assets/icon.svg" width="120" height="120" alt="gladius icon" />
</p>
<h1 align="center">gladius</h1>
<p align="center"><strong>An MCP server that gives your agent a real headless browser: one native Rust binary over stdio, driving a pinned Chromium directly over CDP.</strong></p>

## Install

Add the server to your MCP client and restart it:

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "servers": {
      "gladius": {
        "type": "local",
        "command": ["cmd", "/c", "npx", "-y", "gladius-mcp@latest"]
      }
    }
  }
}
```

The client runs `npx -y gladius-mcp@latest` and speaks MCP over stdio, so that process is spawned by the client, not typed in a terminal. Your first tool call provisions the pinned server runtime and the pinned `chrome-headless-shell`, both sha256-verified before and after extraction, and later runs reuse and re-verify them. A global install is refused by the package's `preinstall` script.

Check the pin first: `npx -y gladius-mcp@latest --doctor` prints the manifest and the resolved cache paths, downloads nothing and exits 0.

### Or let your AI install it

```text
Install nathwn12/gladius (gladius-mcp) as a local stdio MCP server with npx -y gladius-mcp@latest, then prove it works.
1. Run: npx -y gladius-mcp@latest
2. First run: the client spawns that command, which provisions the pinned runtime and browser (both sha256-verified); it needs Windows x64 and Node 22.14 or newer.
3. Done when: npx -y gladius-mcp@latest --doctor exits 0 and prints "serverVersion": "0.8.0" with the pinned browser version.
If it fails, stop and paste the full output; do not improvise a fix.
```

## Use it

Ask the agent to open a page, take a snapshot, then act on a ref. The loop is the same for every task: `navigate`, `snapshot`, then a tool that acts on an `@eN` ref.

- Session: `navigate`, `history`, `tabs`, `targets`, `viewport`, `dialog`, `close`.
- Read: `snapshot` (the accessibility tree with stable `@eN` refs, pageable past 8000 chars), `evaluate`, `console`, `network`, `fetch_text`.
- Act: `click`, `type`, `hover`, `select`, `key`, `upload` (trusted CDP input, `isTrusted:true`).
- Wait and prove layout: `wait_for`, `assert_layout`, `layout` (boxes and geometry, no pixels).
- Pixels: `screenshot` (deterministic, with a content checksum), `visual` (stored baselines, threshold diff).
- State: `storage` (cookies and Web Storage, versioned save and load).
- Escape hatch: `cdp` (raw `method` plus `params`).

That is 25 tools. Each carries a versioned output schema and `readOnly`/`destructive`/`idempotent`/`openWorld` annotations, and every failure returns a typed code beside the prose.

Every argument, option and typed error: [docs/GUIDE.md](docs/GUIDE.md).

## Configure it

No flag serves MCP. The launcher's other flags are `--doctor`, `--print-manifest`, `--version` and `--help`.

| Variable | Default | Effect |
|---|---|---|
| `GLADIUS_CDP_URL` | unset | Attach to an engine you already run instead of launching one; `close` detaches and never kills it. |
| `GLADIUS_CHROMIUM` | unset | Absolute path to your own `chrome-headless-shell`; browser provisioning is skipped entirely. |
| `GLADIUS_VISUAL_DIR` | per-user cache, under `gladius/visual` | Root for `visual` baselines and their sidecars. |
| `GLADIUS_VISUAL_CAP` | `500` | Stored baselines, and separately the diff by-products. A save at the cap is a typed refusal. |
| `GLADIUS_TOOL_BUDGET_MS` | `90000` | Budget for one tool call; a per-call `_budgetMs` overrides it. |
| `GLADIUS_STARTUP_TIMEOUT_SECS` | `30` | Deadline for the owned-engine readiness poll. |
| `GLADIUS_BACKEND_CAPS` | `render` | `dom-only` makes `screenshot` and `visual` refuse with a typed error instead of returning a fake image. |

Attach to a browser you run yourself, so the engine is yours and gladius only drives it. Add this to the `gladius` entry from Install:

```jsonc
"environment": { "GLADIUS_CDP_URL": "http://127.0.0.1:9222" }
```

## Requirements

- Windows 10/11 x64. The server runtime runs on no other platform.
- Node 22.14 or newer, for the `npx` launcher.
- An MCP client that speaks stdio.
- The first run downloads the pinned browser, about 118 MB, and the runtime bundle, and verifies both. There is no fully offline fresh install.
- Headless only: gladius never opens a visible window. To watch a real browser, start your own and attach with `GLADIUS_CDP_URL`.

## Development

```sh
cargo build --locked
cargo test --locked
```

The toolchain is pinned by `rust-toolchain.toml`. `cargo fmt --all --check` and `cargo clippy --all-targets --locked -- -D warnings` are the static gate. Architecture, budgets and the frozen tool contract: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). Release lane: [docs/PUBLISHING.md](docs/PUBLISHING.md).

## Status

**Experimental** — pre-release; interfaces may change.

## Links

- [Issues](https://github.com/nathwn12/gladius/issues)
- [Releases](https://github.com/nathwn12/gladius/releases)
- [Guide](docs/GUIDE.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Publishing](docs/PUBLISHING.md)
- [Third-party notices](THIRD-PARTY-NOTICES.md)
- [MIT](LICENSE)
