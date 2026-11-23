<h1 align="center">gladius-mcp</h1>

<p align="center"><strong>A stdio MCP server that drives a pinned headless browser over CDP with typed tools; the launcher provisions and verifies the runtime and browser on first run.</strong></p>

<p align="center">
  <a href="https://www.npmjs.com/package/gladius-mcp"><img src="https://img.shields.io/npm/v/gladius-mcp" alt="npm version" /></a>
  <img src="https://img.shields.io/badge/platform-windows--64bit-2b3137?logo=windows&logoColor=white" alt="Windows x64" />
  <img src="https://img.shields.io/badge/tools-25-brightgreen" alt="25 tools" />
  <img src="https://img.shields.io/badge/license-MIT-green" alt="MIT license" />
</p>

## Install

```sh
npx gladius-mcp@latest
```

A global install is refused. To pin an exact build, use the version number instead of `latest`.

### Or let your AI install it

```text
Install nathwn12/gladius (gladius-mcp) with npx gladius-mcp@latest, then prove it works.
1. Run: npx gladius-mcp@latest
2. First run: provisions the pinned runtime and browser, verifies both, then serves MCP over stdio; needs Windows x64, Node >= 22.14.0 and network access.
3. Done when: npx gladius-mcp@latest --doctor exits 0 and prints "network": "none: this diagnostic performs no fetch and exits before any download code".
If it fails, stop and paste the full output; do not improvise a fix.
```

### Wire it yourself

Add the server to an OpenCode config and restart the client:

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

The `-y` flag keeps the non-interactive spawn from prompting. Any stdio MCP client can use the same command.

### Offline or pinned

Pin an exact build with `npx gladius-mcp@0.8.0`. A pinned version fetches its runtime from that version's GitHub release; published release assets are retained, not deleted. The package ships no browser or server bytes, so a fresh run still needs network. `npx gladius-mcp@latest --doctor` and `--version` are offline. Set `GLADIUS_CHROMIUM` to an existing `chrome-headless-shell` binary to skip browser provisioning.

## Capabilities

Twenty-five tools over one stdio connection: navigate, snapshot, click, type, screenshot and assert layout. Each tool returns a versioned schema with typed error codes; layout checks report measured boxes and deterministic captures carry a content checksum. The launcher verifies the pinned runtime and browser by sha256 before and after extraction and reuses both on later runs.

## Requirements

- Windows x64. Other platforms refuse at startup; `GLADIUS_MCP_ALLOW_NON_WINDOWS=1` bypasses the gate for diagnostics only.
- Node.js >= 22.14.0 for the launcher; no package dependencies.
- An MCP client that speaks stdio.
- Network on first run to fetch the pinned runtime and browser. There is no fully offline fresh install; headless only.

## Status

**Experimental** — pre-release; interfaces may change.

## Links

- [Issues](https://github.com/nathwn12/gladius/issues)
- [Releases](https://github.com/nathwn12/gladius/releases)
- [Guide](https://github.com/nathwn12/gladius/blob/main/docs/GUIDE.md)
- [npm package](https://www.npmjs.com/package/gladius-mcp)
- [MIT](LICENSE)

<p align="center">
  <img src="signature-glyph.svg" width="120" alt="nathwn12 signature" />
  <br />Built by nathwn12
</p>
