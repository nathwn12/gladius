# gladius

MCP server that hands an agent a real headless browser: one native Rust binary over stdio, driving a pinned `chrome-headless-shell` directly over CDP, with no Node.js, no Playwright and no OS input in the runtime path.

## Install

Add the server to `opencode.jsonc`, then restart OpenCode:

```jsonc
{
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

Check the pin before relying on it (prints the manifest and cache paths, downloads nothing, exits 0):

```powershell
npx -y gladius-mcp@latest --doctor
```

The first real tool call provisions the pinned server runtime and the pinned browser, both hash-verified. Use `npx`, never `npm i -g` — global installs are refused by design.

## What it does

| Group | Tools | Behavior |
|---|---|---|
| Navigate and manage the session | `navigate`, `history`, `tabs`, `targets`, `viewport`, `dialog`, `close` | Opens URLs, walks back/forward/reload, lists and switches tabs and CDP targets, sets viewport metrics, handles JS dialogs, shuts the browser down. |
| Read the page | `snapshot`, `evaluate`, `console`, `network`, `fetch_text` | Real Accessibility tree with stable `@eN` refs, pageable past the 8000-char cap (`offset`/`nextOffset`), shadow-DOM refs actionable, JS evaluation, buffered console and network logs, plain-text HTTP fetches with no browser. |
| Act on the page | `click`, `type`, `hover`, `select`, `key`, `upload` | Trusted CDP pointer and text input (`isTrusted:true`) after scroll and hit-test, keyboard combos, `<select>` choices, file uploads, all optionally scoped into an iframe. |
| Wait and prove layout | `wait_for`, `assert_layout`, `layout` | Waits on a selector, text, network idle or JS predicate; proves geometry without pixels (`exists`, `visible`, `box`, `hitTarget`, `computed`, `noOverflow`, `inViewport`); reports boxes, scroll sizes and overlaps. |
| Pixels | `screenshot`, `visual` | Deterministic screenshots with a content checksum, and stored visual baselines that diff inside the engine and pass or fail on a real threshold. |
| State | `storage` | Cookies and Web Storage read, write, clear, and versioned save/load to a file, with values redacted on request. |
| Escape hatch | `cdp` | Raw CDP `method` + `params` when the typed tools above are not enough. |

25 tools. Every tool carries a versioned `outputSchema` and `readOnly`/`destructive`/`idempotent`/`openWorld` annotations, and every failure returns a typed `structuredContent` code beside the prose.

## Requirements

Node 22.14 or newer, on Windows x64 — the first run downloads two hash-verified pinned artifacts, then reuses and re-verifies them.

## Development

```sh
cargo test
cargo build
```

## Guide

Every tool signature, every option and environment variable, troubleshooting and the explicit limitations: [docs/GUIDE.md](docs/GUIDE.md).

## License

MIT. See [LICENSE](LICENSE).
