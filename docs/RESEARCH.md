# Gladius RESEARCH - browser-automation MCP landscape, by technology

> Archive note (2026-10-12): landscape survey of third-party products
> (their names are their own). Its gladius-side lessons predating
> self-containment (`96e73e3`: owned `bin\obscura.exe` is the only
> launchable engine, plus explicit attach via `GLADIUS_CDP_URL`) are
> historical - follow `docs/ARCHITECTURE.md` for the live backend picture.
> (2026-10-20): "9-tool contract/surface" language below is the
> research-phase surface; the shipped surface is 22 tools
> (`docs/ARCHITECTURE.md` §1, `CHANGELOG.md` 0.4.0).

> Policy: ranked by technical sophistication, not star counts. Stars are hype; architecture is king.
> Method (2026-10-09): web-search was unavailable, so every repo URL below was verified live
> with an HTTP HEAD check (200 OK), and the two most load-bearing READMEs
> (`microsoft/playwright-mcp`, `lightpanda-io/browser`) were fetched in full.
> Two guessed URLs 404'd (`steel-dev/mcp-server-steel`, `denihs/fastwebsockets`) and were
> corrected (`steel.dev` homepage; `littledivy/fastwebsockets`). Cost numbers are the
> projects' own reported figures or qualitative estimates, labelled as such.

## 1. Playwright-stack MCPs (Node.js + browser driver binaries)

### `@playwright/mcp` - https://github.com/microsoft/playwright-mcp
- **What it technically does:** MCP server (TypeScript, run via `npx @playwright/mcp`) that drives
  real Chromium/Firefox/WebKit through the Playwright protocol - *not* raw CDP. The headline
  design choice is **accessibility-tree snapshots, not screenshots**: tools like `browser_snapshot`
  return the a11y tree with element refs, and actions (`browser_click`, `browser_type`, ...) target
  refs from that tree. Optional caps: `vision` (coordinate/screenshot), `pdf`, `devtools`.
  Can also attach to an existing browser via `--cdp-endpoint` or the Playwright Chrome extension
  (extension-relay escape hatch), and supports SSE/HTTP transport (`--port`) plus Docker
  (`mcr.microsoft.com/playwright/mcp`).
- **Cost profile:** heavy. Requires Node.js >= 18 plus Playwright's browser/driver downloads
  (hundreds of MB); a persistent profile dir per workspace (`ms-playwright/mcp-*`); idle
  browsers linger (default `--idle-timeout` 1 h). Startup = `npx` resolve + Node runtime +
  browser launch: seconds, not milliseconds.
- **Transferable lesson:** a11y-snapshot-first (tokens + determinism beat pixels for most steps);
  ref-based action targeting; opt-in capabilities so the base tool schema stays small.

### Puppeteer reference in `modelcontextprotocol/servers` (archived) - https://github.com/modelcontextprotocol/servers
- **What it technically does:** the original MCP reference servers repo; its Puppeteer server drove
  Chromium via Puppeteer (CDP under the hood) over stdio. Now **archived** - the Puppeteer server
  was superseded by `@playwright/mcp` as the recommended browser route.
- **Cost profile:** same weight class as Playwright-stack (Node + full Chromium), slightly less
  machinery than Playwright's multi-browser grid.
- **Transferable lesson:** stdio JSON-RPC framing is the stable contract (gladius keeps it); and
  *reference servers rot* - pin to a maintained driver or own the (thin) CDP layer directly.

## 2. Extension-relay MCPs (attach to YOUR browser, no launched browser)

### `hangwin/mcp-chrome` (a.k.a. chrome-mcp-server) - https://github.com/hangwin/mcp-chrome
- **What it technically does:** a Chrome **extension + MCP bridge**: the extension lives in the
  user's real (headed, logged-in) Chrome and relays tabs/DOM/actions to the MCP server over
  CDP-ish messaging. No browser is launched; automation reuses cookies, logins, and profile.
- **Cost profile:** near-zero startup/RAM *of its own* (the browser is already running), but it
  externalises the cost: requires a human browser, an installed extension, and interactive setup.
  Useless headless/on a server.
- **Transferable lesson:** attaching to an existing CDP endpoint (`--cdp-endpoint` style) is a
  cheap, high-value feature - gladius should accept `CDP_URL` to attach instead of launch. But
  the *default* path must stay self-contained headless (CI/agents have no human browser).

## 3. Python agentic loop (framework, not a driver)

### `browser-use/browser-use` - https://github.com/browser-use/browser-use
- **What it technically does:** Python **agent framework** on top of Playwright/Chromium: the LLM
  plans multi-step tasks, extracts structured data, and recovers from failures. The browser is
  still Playwright-driven Chromium; the sophistication is the *loop* (planning, self-healing,
  output schemas), not the transport.
- **Cost profile:** the heaviest row: Python runtime + LangChain-style deps + Playwright +
  full Chromium + an LLM in the loop for every step. Startup seconds; RAM GBs.
- **Transferable lesson:** keep the *driver* dumb and fast - planning/self-healing belongs in the
  agent (the MCP client), not in gladius. Gladius exposes crisp primitives; it never embeds an LLM.

## 4. Cloud-browser MCPs (browser runs elsewhere, MCP is a thin client)

### Browserbase - https://github.com/Browserbase/mcp-server-browserbase
### Steel - https://steel.dev
### Hyperbrowser - https://github.com/HyperbrowserAI/mcp
- **What they technically do:** each vendor runs fleets of real Chromium instances in the cloud
  and ships a small MCP server that maps tool calls -> remote sessions (CDP over websockets,
  session IDs, stealth/anti-bot handling, proxies). The local process holds almost no browser.
- **Cost profile:** local startup/RAM look great (just an API client), but the cost moved, not
  removed: network round-trip per action (10-100 ms+), per-minute billing, data leaves the
  machine, and offline/CI use is impossible.
- **Transferable lesson:** session isolation per agent (Lightpanda independently converged on the
  same idea: one browsing session per `Mcp-Session-Id`) is the right multi-agent shape. Gladius
  stays *local-first* - that is precisely the gap cloud vendors leave open (no billing, no egress,
  works offline).

## 5. Ultralight CDP browser (the one to watch)

### `lightpanda-io/browser` - https://github.com/lightpanda-io/browser
- **What it technically does:** a **from-scratch headless browser in Zig** - not a Chromium fork:
  its own HTTP loader (libcurl), HTML parser (html5ever), DOM, V8 for JS, adblocker, and a
  **CDP server** (`lightpanda serve --protocol cdp`) plus WebDriver BiDi, a `fetch --dump`
  one-shot mode, and a **native MCP server** (`lightpanda mcp`, stdio *and* HTTP with per-session
  isolation). Deliberately **no graphical/rendering engine**.
- **Cost profile (their reported benchmark, labelled as vendor-claimed):** 933 real pages on
  m5.large - peak 123 MB vs 2 GB headless Chrome (~16* lighter), 5 s vs 46 s (~9* faster).
  Single static binary, no Node, no Chromium download. Caveats: nightly-only binaries, **no
  native Windows build** (WSL/Docker only), and canvas/WebGL/pixel-perfect rendering are out
  of scope by design. Details: https://github.com/lightpanda-io/demo/blob/main/BENCHMARKS.md
- **Transferable lesson:** this validates gladius's whole bet - CDP-compatible, no-render,
  single-binary is the Pareto frontier for agent browsing. Gladius's Tier-1 plan (Lightpanda
  as the no-render backend) is a *backend swap behind the same 9-tool contract* [archived 2026-10-20: research-phase surface; shipped surface is 22 tools], not a rewrite.

## 6. Rust building blocks (what gladius itself is built from)

### `chromiumoxide` (mattsse) - https://github.com/mattsse/chromiumoxide
- **What it technically does:** async Rust CDP client: launches/connects Chromium, owns the
  websocket, exposes typed `Page`/`Browser` APIs over tokio. The fastest route to "Rust talking
  CDP" without hand-rolling the protocol.
- **Cost profile:** the crate itself is light (tokio + websockets); the *browser child process*
  dominates. Gladius's rule: depend on the protocol layer, budget the child process.
- **Transferable lesson:** use it (or its pattern) for the CDP session, but keep the MCP surface
  frozen at 9 tools [archived 2026-10-20: research-phase surface; shipped surface is 22 tools] - chromiumoxide's API is far wider than agents need.

### `rmcp` - official Rust MCP SDK - https://github.com/modelcontextprotocol/rust-sdk
- **What it technically does:** the official `modelcontextprotocol/rust-sdk` crate (`rmcp`):
  MCP transports (stdio/SSE/streamable HTTP), tool routing, schema derivation. The sanctioned
  way to speak MCP from Rust.
- **Cost profile:** pure-Rust, no runtime beyond tokio; negligible vs. the browser child.
- **Transferable lesson:** build the MCP layer on `rmcp` stdio - no hand-rolled JSON-RPC framing.

### `headless_chrome` - https://github.com/rust-headless-chrome/rust-headless-chrome
- **What it technically does:** older/alternative Rust CDP driver: launches Chrome, navigates,
  screenshots via CDP. Largely superseded by `chromiumoxide` for new work.
- **Cost profile / lesson:** surveyed as the fallback option; lesson is *negative*: pick the
  maintained driver (chromiumoxide), one writer per seam, no driver abstraction layer.

### `fastwebsockets` - https://github.com/littledivy/fastwebsockets
### `tokio-tungstenite` - https://github.com/snapview/tokio-tungstenite
- **What they technically do:** the two candidate websocket transports under the CDP session:
  `fastwebsockets` (SIMD, minimal-copy, fastest) vs `tokio-tungstenite` (standard, most
  compatible). Docs: https://docs.rs/fastwebsockets / https://docs.rs/chromiumoxide.
- **Cost profile:** both are noise next to a browser process; choice is about latency-per-frame
  and backpressure behaviour.
- **Transferable lesson:** default to `tokio-tungstenite` for compatibility, benchmark
  `fastwebsockets` as a drop-in if CDP frame latency ever shows up in profiles - measure, don't guess.

### `chrome-headless-shell` + Chrome for Testing - https://googlechromelabs.github.io/chrome-for-testing/
- **What it technically does:** `chrome-headless-shell` is Google's official **no-UI Chromium
  build** (the old `--headless` split out as its own binary), distributed via the Chrome-for-Testing
  versioned endpoints. Background: https://developer.chrome.com/blog/chrome-headless-shell.
  Measured 2026-10-09 on this machine, it resolved via Playwright's cache
  (`%LOCALAPPDATA%\ms-playwright\chromium_headless_shell-*\chrome-headless-shell-win64\chrome-headless-shell.exe`).
  Historical record only: gladius no longer launches this path (self-contained
  on the owned engine since `96e73e3`).
- **Cost profile:** full Blink/V8/JS fidelity at a fraction of headed Chrome's RAM (no compositor,
  no GPU process tree by default); still a ~100-200 MB child process - the thing gladius budgets.
- **Transferable lesson:** historical (pre-self-containment `96e73e3`): the
  default backend was `chrome-headless-shell` (fidelity without the headed
  tax), full Chrome only opt-in for pixel-perfect screenshots. Since
  self-containment the owned `bin\obscura.exe` is the only launchable engine
  (plus explicit attach via `GLADIUS_CDP_URL`) - follow
  `docs/ARCHITECTURE.md` for the live backend picture.

---

## Best practices distilled (gladius implements all 10)

1. **Snapshot-first, screenshots on demand** (from `@playwright/mcp`): structured a11y/DOM text is
   the default observation; pixels only when asked.
2. **Ref-free, selector-simple actions:** 9 frozen tools with plain args (css/xpath/text) - no
   ref graphs leaking across calls.
3. **Lazy browser start:** first tool call launches the backend; process construction costs zero
   at MCP-server startup (from `@playwright/mcp`'s idle-timeout idea, made stricter).
4. **Kill-tree on close:** `close` (and stdio EOF) reaps the whole browser process tree - no
   orphaned engine child processes (pre-self-containment the reaped name was
   `chrome-headless-shell` - the Playwright lingering-profile failure mode).
5. **Attach, don't always launch:** honour a `CDP_URL` env to attach to an existing endpoint
   (from extension-relay + `--cdp-endpoint`).
6. **One session per agent:** independent browsing contexts per client connection (from
   cloud vendors + Lightpanda's `Mcp-Session-Id` design).
7. **Driver stays dumb:** no LLM, no planner, no retries-with-judgement in the server
   (anti-`browser-use` lesson: the loop lives in the client).
8. **Single static binary, no runtime deps:** Rust + `rmcp` + one CDP path - no Node, no Python,
   no Playwright download step (the Lightpanda lesson applied to the server itself).
9. **Budget in CI, on Windows:** cold-start latency + peak child RSS asserted by the gauntlet on
   every change (numbers in ARCHITECTURE.md).
10. **Backend swap behind a frozen contract:** owned Obscura engine today
    (self-contained since `96e73e3` - the `chrome-headless-shell`/full-Chrome
    paths are removed, not opt-ins), Lightpanda no-render tier tomorrow.

## Why Rust

- **Startup:** no interpreter boot (Node/Python pay 100s of ms before doing anything); gladius
  must answer MCP `initialize` in low-ms and lazily spawn the browser.
- **RAM:** no GC runtime next to a browser child that already eats 100+ MB; the server itself
  targets single-digit MB RSS.
- **Distribution:** one static `.exe`, no `npx`/pip/download step - the anti-Playwright story.
- **Correct CDP concurrency:** tokio + typed CDP sessions (via `chromiumoxide`/`rmcp` patterns)
  give backpressure and kill-tree semantics that are awkward in GC'd runtimes.
- **Official SDK exists:** `rmcp` (https://github.com/modelcontextprotocol/rust-sdk) means the MCP
  layer is maintained upstream, not hand-rolled.

## Who we beat / who beats us (axes: startup, RAM, deps, features)

| System | Startup | RAM (local) | Deps | Features | Verdict |
|---|---|---|---|---|---|
| gladius (target) | ~50 ms server; lazy browser | ~5 MB server + ~150 MB shell child | 1 static exe | 22 tools, CDP-full-fidelity DOM | - |
| `@playwright/mcp` https://github.com/microsoft/playwright-mcp | seconds (npx+Node+browser) | GBs w/ browser | Node 18+, browser downloads | 30+ tools, multi-browser, vision/pdf | **we beat it** on startup/RAM/deps; **it beats us** on breadth |
| Puppeteer reference https://github.com/modelcontextprotocol/servers | seconds | GBs | Node + Chromium | archived, minimal | **we beat it** everywhere; it is dead |
| `hangwin/mcp-chrome` https://github.com/hangwin/mcp-chrome | ~0 (browser already open) | ~0 own | extension + human browser | logged-in human profile | **it beats us** for personal browsing; **we beat it** headless/CI |
| `browser-use` https://github.com/browser-use/browser-use | seconds + LLM first-step | GBs + model | Python+Playwright+LLM | autonomous multi-step tasks | different game: **it beats us** on agency, **we beat it** as a substrate (it could drive *us*) |
| Browserbase https://github.com/Browserbase/mcp-server-browserbase / Steel https://steel.dev / Hyperbrowser https://github.com/HyperbrowserAI/mcp | fast local, slow per-action (network) | ~0 local (cloud pays) | account + billing + network | stealth, fleet, sessions | **they beat us** on scale/stealth ops; **we beat them** on offline, $0, data-local |
| Lightpanda https://github.com/lightpanda-io/browser | fastest w/ renderless (~ms) | ~123 MB / 100 pages (vendor claim) | 1 binary (no Win native!) | no-render DOM/fetch/MCP | **it beats us** on RAM *once its tier lands* (we will adopt it); **we beat it** today on Windows + pixel fidelity |
| Raw `chromiumoxide` https://github.com/mattsse/chromiumoxide | n/a (library) | n/a | Rust crate | full CDP API | substrate, not rival - we wrap its pattern |

**Net:** gladius is the local-minimum box: faster/lighter than every Node/Python/Cloud path on a
Windows dev machine, conceding only (a) personal-profile browsing to extension relays, (b) fleet
ops to clouds, and (c) renderless RAM to Lightpanda - which is on our roadmap as a backend tier,
not a competitor.
