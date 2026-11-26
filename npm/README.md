<h1 align="center">gladius-mcp</h1>
<p align="center"><strong>A stdio MCP server that drives a pinned headless browser over CDP with typed tools.</strong></p>

<p align="center">
  <a href="https://www.npmjs.com/package/gladius-mcp"><img src="https://img.shields.io/npm/v/gladius-mcp" alt="npm version" /></a>
  <img src="https://img.shields.io/badge/platform-windows--64bit-2b3137" alt="Windows x64" />
  <img src="https://img.shields.io/badge/tools-25-brightgreen" alt="25 tools" />
  <img src="https://img.shields.io/badge/license-MIT-green" alt="MIT licence" />
</p>

## Install

```sh
npx gladius-mcp@latest
```

With no arguments this starts the long-lived stdio server. `--doctor` and `--version` are one-shot offline checks that exit; they do not start the server. Global installs are refused; pin an exact build by replacing `latest` with a version.

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

Pin the npm package with `npx gladius-mcp@0.8.1`. That pins the package version; the bundled `engine-manifest.json` records the runtime asset the launcher fetches. For the published 0.8.1 build that record points under the `gladius` release tag to `gladius-v0.8.1-windows-x86_64.zip`. The release workflow performs no deletion; both the `gladius` and `v0.7.1` releases are present. The package ships no browser or server bytes, so a fresh run still needs network. Set `GLADIUS_CHROMIUM` to an existing `chrome-headless-shell` binary to skip browser provisioning.

## Tools

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
- [npm package](https://www.npmjs.com/package/gladius-mcp)
- [MIT licence](LICENSE)
