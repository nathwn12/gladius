# Publishing gladius-mcp (owner only)

`npx gladius-mcp@latest` installs the launcher in `npm/` plus the pinned
browser/runtime manifest. The server runtime is fetched on first RUN from
the tagged GitHub Release, and the browser from Google's
Chrome-for-Testing endpoint — never at install. Do NOT publish, tag, or
release from any automated session: every step below is a human act, in
order.

> POSTURE (settled — no redistribution question remains). No third-party
> binary is redistributed: Obscura no longer ships anywhere. The runtime
> bundle carries only the server plus licences/notices (including the
> Chromium licence attribution copy
> `THIRD-PARTY-LICENSES/LICENSE.headless_shell`, taken from the provisioned,
> sha-verified browser cache). The browser itself is fetched at provision
> time from Google and never bundled — which is also why our own manifest
> is the integrity root (CfT publishes no hashes).

Prerequisites: an npm account (first publish is manual, with 2FA or a
granular token), and locally npm >= 11.5.1 with Node >= 22.14.0
(the workflow asserts both; match them for the manual step).

## Steps

1. Account. Sign in to npmjs.com and create/claim the account that will
   own the package. Package name is unscoped `gladius-mcp`; it publishes
   under whoever publishes first — confirm the handle at sign-in.

2. Push the version tag (e.g. `v0.2.1`). The `release` workflow builds the
   crate with the MSRV toolchain, runs the suite against the provisioned
   pinned browser, assembles the server-only runtime bundle (no browser
   bytes), fills `engine-manifest.json` with hashes computed from the REAL
   artifacts, and creates the GitHub Release with the archives attached. Its
   `npm publish` step FAILS (no trusted publisher yet) — expected, not a
   problem. Nothing reaches npm.

3. FIRST publish (manual — this is what makes the package exist).
   Download the run's `npm-package` artifact, unpack it, and from your own
   machine run `npm publish` on that directory exactly as built (it holds
   the real release hashes; the committed manifest holds placeholders the
   launcher refuses to run with). Verify: `npm view gladius-mcp version`
   and `npx gladius-mcp@latest --doctor`.

4. Trusted publisher (VERIFIED CLI route — trust is NOW REGISTERED).
   From your own machine, in an interactive shell (it needs a one-time
   password: npm prints a web-auth URL for a person's approval in the
   browser), run exactly:

   ```powershell
   npm trust github gladius-mcp --file release.yml --repo nathwn12/gladius --allow-publish --allow-stage-publish
   ```

   This works even before a first publish (no existing version required).
   Fallback: the package's Settings → Trusted Publisher → GitHub Actions
   panel with exactly Organization or user `nathwn12` (required),
   Repository `gladius` (required), Workflow filename `release.yml`
   (required — filename only, exact case; it exists in
   `.github/workflows/`), optionally an Environment and an allowed-actions
   choice (`npm stage publish` is always allowed).

   Status: this trust is NOW REGISTERED. Proven working: a re-run of the
   publish step got past authentication and failed only on the version
   clash (the manually published version cannot be re-published by the
   workflow). Consequently every `v*` tag from here on publishes over OIDC
   with automatic provenance and NO human step — no `--provenance` flag,
   no token, no long-lived tokens stored anywhere.

5. Within 2 DAYS of configuring, land the first workflow publish: cut the
   next version (checklist below) and push its `v*` tag. The publish job
   then publishes over OIDC with automatic provenance — no
   `--provenance` flag, no token. After that no long-lived tokens are
   stored anywhere. If the window lapses, the trust configuration
   expires: reconfigure and retry. Note the manually published version
   cannot be re-published by the workflow, which is why the first trusted
   publish rides the NEXT version tag.

6. Ongoing. Every `v*` tag push is one GitHub Release plus one npm
   publish, always from the same tag and commit. Never `npm publish` by
   hand again, and never publish anything but a workflow-built
   `npm-package` artifact.

## Version checklist (one version everywhere)

A release tag `vX.Y.Z` requires, on that commit: `Cargo.toml` version
`X.Y.Z`, `npm/package.json` version `X.Y.Z`, `npm/engine-manifest.json`
`serverVersion` `X.Y.Z` with matching `runtime.source`/`runtime.archive`
names (the workflow asserts all four equal the tag and fills the hashes),
and a `CHANGELOG.md` entry. The workflow fails closed on any mismatch.
The manifest's `browser` pin must equal `scripts/chromium-manifest.json`'s
(the workflow asserts deep equality plus the `fetch-chromium.ps1` constants
and the provisioned bytes); a browser bump also refreshes the version named
in `THIRD-PARTY-NOTICES.md`.

## Why these exact values

- `repository.url` in `npm/package.json` must match the GitHub repo
  (`https://github.com/nathwn12/gladius`) exactly — trusted
  publishing/provenance requires it.
- `npm/package.json` has no `scripts` field except the global-install-refusal
  `preinstall` guard (the workflow asserts exactly this): a postinstall
  download would silently never run under npx, so
  the engine is acquired on first RUN by the launcher instead.
- Publish runs on a GitHub-hosted runner with `id-token: write`
  (OIDC requirement). Self-hosted runners are not trusted publishers.
