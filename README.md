<p align="center">
  <img src="assets/icon.svg" width="120" height="120" alt="gladius icon" />
</p>
<h1 align="center">gladius</h1>
<p align="center"><strong>Stop walking pages by hand. Give your agent a browser it can drive.</strong></p>

## Install

```powershell
npx -y gladius-mcp@latest --doctor
```
You need Node 22.14 or newer on Windows x64 first. The first run provisions the pinned server runtime (0.2.1) plus the pinned chrome-headless-shell browser (155.0.8059.84), both hash-verified, then `--doctor` prints the manifest with the resolved cache paths and exits 0.

### Or let your AI install it

```text
Install the npm package gladius-mcp from nathwn12/gladius into my MCP client config, served with npx gladius-mcp@latest over stdio.
Then verify with npx gladius-mcp@latest --doctor and npm view gladius-mcp version and show me both outputs.
```

### Wire it yourself

Wire the server into your MCP client config:

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

### Offline or pinned

Pin an exact version instead of `@latest`:
```powershell
npx gladius-mcp@0.2.1 --doctor
```
Point the launcher at a browser you already hold to skip the browser download. Set `GLADIUS_CHROMIUM` to your chrome-headless-shell binary before you start the server:
```powershell
$env:GLADIUS_CHROMIUM = "<path-to-your-chrome-headless-shell-binary>"
```
Redirect the caches with `GLADIUS_ENGINE_CACHE` and `GLADIUS_CHROMIUM_CACHE`, or inspect the pin with no network access:
```powershell
npx -y gladius-mcp@latest --print-manifest
```
The names above come from `gladius-mcp --help`. There is no fully offline fresh install: the first run still downloads the pinned runtime, so treat these as pins and overrides, not as a no-network setup.

## Use it

Open a page with the `navigate` tool:
```json
{
  "url": "https://example.com"
}
```
You get back JSON carrying the page `url` and `title` once the load event fires. Read a value with the `evaluate` tool:
```json
{
  "expression": "window.__gladiusValue"
}
```
Against the repo test page you get back:
```json
{
  "value": 42
}
```
Grab text without driving the browser with the `fetch_text` tool:
```json
{
  "url": "https://example.com"
}
```
You get back the page text with tags stripped and scripts dropped.

## What you get

- An agent that works the page for you: 21 tools to open pages, snapshot structure, click, type, screenshot, read the console, target frames and workers, wait on JS predicates, handle dialogs, walk history, read the network, and drive raw CDP (docs/ARCHITECTURE.md).
- The same browser every run: pinned chrome-headless-shell, hash-verified and provisioned on first run, shown by --doctor (docs/CHROMIUM.md).
- A small local footprint: one native binary over stdio that starts the browser only when the first tool call needs it (docs/ARCHITECTURE.md).
- Errors you can act on: typed tool errors, buffered console messages plus uncaught exceptions, and a headless-only engine with no silent fallback (docs/ENGINE.md).

## Requirements

- Node 22.14 or newer (package engines, checked with `node --version`).
- Windows x64. Other platforms are refused unless you set the diagnostics override named in `--help`.
- The first run downloads two pinned artifacts, both hash-verified: the server runtime from the tagged GitHub Release and the browser from the Chrome for Testing endpoint (docs/PUBLISHING.md).

## Links

- Repo: https://github.com/nathwn12/gladius
- Package: https://www.npmjs.com/package/gladius-mcp
- Releases: https://github.com/nathwn12/gladius/releases
- Issues: https://github.com/nathwn12/gladius/issues
- License: MIT, see [LICENSE](LICENSE)
- Docs: [browser pin](docs/CHROMIUM.md), [engine notes](docs/ENGINE.md), [changelog](CHANGELOG.md)

<!-- nathwn12 house footer: identical in every repo. Do not edit. -->
<p align="center">
  <a href="https://github.com/nathwn12"><img src="assets/signature.svg" width="96" alt="nathwn12" /></a>
</p>
<p align="center">
  <sub>Built by <a href="https://github.com/nathwn12">nathwn12</a> · <a href="https://github.com/nathwn12?tab=repositories">All projects</a></sub>
</p>
