# gladius
**Stop walking pages by hand. Give your agent a browser it can drive.**

## Let your AI install it

```text
Install the npm package gladius-mcp from nathwn12/gladius into my MCP client config, served with npx gladius-mcp@latest over stdio.
Then verify with npx gladius-mcp@latest --doctor and npm view gladius-mcp version and show me both outputs.
```

## Easy install

```powershell
npx gladius-mcp@latest --doctor
```

## Prefer to wire it yourself

Add this to your MCP client config (Claude Desktop shape):

```json
{
  "mcpServers": {
    "gladius": {
      "command": "npx",
      "args": [
        "gladius-mcp@latest"
      ]
    }
  }
}
```

Check the install with the same two commands from the prompt above. You should see version 0.2.1 and a manifest pinning chrome-headless-shell 155.0.8059.84.

## Use it

### Open a page

```json
{
  "url": "https://example.com"
}
```

Send that as the `navigate` tool input. You get back JSON carrying the page `url` and `title` once the load event fires. The suite asserts exactly this shape in tests/e2e_mcp.rs.

### Read a value from the page

```json
{
  "expression": "window.__gladiusValue"
}
```

Send that as the `evaluate` tool input against the repo test page. You get back:

```json
{
  "value": 42
}
```

That value is set in tests/fixtures/fixture.html, and tests/e2e_mcp.rs asserts the round trip.

### Grab text without the browser

```json
{
  "url": "https://example.com"
}
```

Send that as the `fetch_text` tool input. You get back the page text with tags stripped and scripts dropped. The suite proves this with a marker page in tests/e2e_mcp.rs.

## What you get

- An agent that works the page for you: 16 tools to open pages, snapshot structure, click, type, screenshot, and read the console (docs/ARCHITECTURE.md).
- The same browser every run: pinned chrome-headless-shell, hash-verified and provisioned on first run, shown by --doctor (docs/CHROMIUM.md).
- A small local footprint: one native binary over stdio that starts the browser only when the first tool call needs it (docs/ARCHITECTURE.md).
- Errors you can act on: typed tool errors, buffered console messages plus uncaught exceptions, and a headless-only engine with no silent fallback (docs/ENGINE.md).

## Requirements

- Node 22.14 or newer (package engines, checked with node --version).
- Windows x64. Other platforms are refused unless you set the diagnostics override named in --help.
- First run downloads two pinned artifacts, both hash-verified: the server runtime from the tagged GitHub Release and the browser from the Chrome for Testing endpoint (docs/PUBLISHING.md).

## Links

- Repo: https://github.com/nathwn12/gladius
- Package: https://www.npmjs.com/package/gladius-mcp
- Releases: https://github.com/nathwn12/gladius/releases
- Issues: https://github.com/nathwn12/gladius/issues
- License: MIT, see [LICENSE](LICENSE)
- Browser pin: [docs/CHROMIUM.md](docs/CHROMIUM.md) - Engine notes: [docs/ENGINE.md](docs/ENGINE.md) - Changelog: [CHANGELOG.md](CHANGELOG.md)
