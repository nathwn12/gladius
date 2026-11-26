# Publishing gladius-mcp

Build and run the complete gate locally. GitHub Actions verifies the handoff and
publishes those exact bytes through the existing `release.yml` npm trusted publisher.
It does not compile, repack, provision a browser or rerun the suite.

npm IS the release: the built runtime ZIP ships inside the npm tarball under
`bin/`, and the launcher provisions that bundled copy on first run. No GitHub
Release page exists; nothing creates, patches, uploads, backs up or deletes one.
The `vX.Y.Z` tags are history markers only. Versions published before the
tarball carried a ZIP are legacy and unsupported: their manifest URL points at
an empty Releases page and may 404; it is not a supported source.

Publication requires the owner's authorization. npm login, manual publishing and
trusted-publisher configuration changes require the owner's physical passkey approval.
Do not add a token or change the publisher identity to bypass that boundary.

## Local gate

Use Windows with PowerShell 7, the pinned Rust 1.88.0 toolchain, Node >= 22.14.0, npm >= 11.5.1,
Git, and the system libarchive `tar`. Start at a clean committed checkout with
complete release history and the required version already in all manifests.

```powershell
$commit = git rev-parse HEAD
node scripts/prepare-release.mjs --commit $commit --out target/release-candidate
```

The output directory must not exist. A failed gate produces no publishable receipt.
Logs go to standard output; capture them outside the output directory if needed.
The command checks history/version derivation, formatting, clippy, pinned browser
provisioning and every provisioned browser file, locked release build and full tests,
real-browser selftest, attribution, final tarball preflight and fresh-profile install/boot.
It packs once, stages the built runtime ZIP into the package's `bin/`, and tests that
exact tarball against that exact ZIP. It does not edit tracked manifests or close the
working tree's changelog.

`release-receipt.json` records the exact commit, version, required passing checks,
and artifact sizes and SHA-256 values. Retain its separately printed SHA-256.
The receipt is a local assertion, not independent evidence that CI built the executable.
The build provisions a fresh browser inside its owned work directory, never the
shared cache. Ambient Gladius engine/browser overrides are scrubbed. The cleanroom
installs the staged tarball with no external bundle override, so it proves the
bundled runtime provisions from the tarball a stranger receives; the browser is
still fetched from Google into a fresh profile, and no browser bytes are bundled.

## Stage once, then rehearse once

Land the tested commit on `main` without modifying it. If landing changes the
commit, prepare again against the landed revision. Do not reuse a receipt for a new SHA.

```powershell
node scripts/stage-release.mjs --upload target/release-candidate
gh run list --workflow stage-release.yml --limit 1
```

Before any upload or dispatch, `--upload` runs a read-only preflight against the
receipt's commit. It reads live `main` and the `vX.Y.Z` tag, and refuses with
`landed mismatch` when `main` is not that commit or with `conflicting version tag`
when the tag binds a different commit. Both refusals happen before artifact
hashing, blob upload and staging dispatch. An absent tag, or a tag already bound
to the planned commit, passes. The preflight creates no ref, release page, asset
or registry entry.

The staging command uploads public, credential-free git blobs without creating
or moving a ref. They contain only the already public build artifacts (the runtime
ZIP, the npm tarball with its bundled ZIP, the manifest, checksum and notes). A
read-only staging workflow verifies their receipt and produces the one-day
`release-handoff` workflow artifact. Git blobs are not a private or expiring store;
the workflow artifact expires, but the uploaded blob objects may remain accessible.
Never stage confidential material.

Read the staging run's result and ID. Dispatch `release.yml` on `main` with
`product_tag=gladius` (an identity assertion only - it names no page), the exact
`commit`, asserted `version`, `staging_run_id`, `receipt_sha256` and `dry_run=true`.
Inputs are assertions, not build instructions. The publisher checks the staging
workflow path, main branch, exact SHA, successful conclusion, receipt, every staged
byte, packaged manifest and executable, the bundled runtime ZIP inside the tarball,
notes, browser pin, registry integrity conflicts and existing version-tag binding.
Its only npm dry run is `npm publish <exact-tarball> --dry-run --ignore-scripts`.

A dry run makes no tag or registry writes; no GitHub Release page exists to touch.
It does not prove that npm will authenticate an actual OIDC publish, or that npm
will issue attestations. Do not claim CI compiled a locally built executable.
Inspect the one rehearsal before the authorized `dry_run=false` dispatch; do not
repeatedly rehearse or automatically retry an uncertain write.

## npm is the only runtime source

The `gladius-mcp@X.Y.Z` tarball is the release. It contains the launcher, the
pinned manifest and the built runtime ZIP at `bin/gladius-vX.Y.Z-windows-x86_64.zip`.
The launcher provisions that ZIP on first run with unconditional size and SHA-256
verification, then verifies the extracted server. Only tarballs built by this
lane after bundling contain that ZIP: npm versions are immutable, so versions
published earlier cannot gain it, are legacy and unsupported, and consumers
must verify a tarball's `bin/` contents before pinning it. There is no current download
alias, no product page, no accumulating release assets and no backup workflow.

The committed `runtime.source` URL in `engine-manifest.json` is a retained legacy
marker only. No lane writes it, and no Release page exists to serve it; for
pre-bundle versions it may 404. It is not a supported fallback for current versions.

The publisher creates an absent `vX.Y.Z` tag and refuses to move an existing one
to a different commit. Version derivation reads the highest reachable version tag.
No workflow in this lane moves shared refs, deletes historical tags, removes
historical release objects, or creates a release page. Do not bypass a refusal
with a force push. An existing immutable registry version with different bytes is
a conflict, never an overwrite candidate. After publication the publisher fetches
and hashes the registry tarball, checks `latest`, and reports the exact integrity;
it verifies no release metadata because there is no release page.

## Cheap checks

```powershell
node --test scripts/release-handoff.test.mjs
node scripts/derive-release.mjs --check
node scripts/check-release-workflow.mjs
```

These do not replace the full local preparation gate or the single CI rehearsal.
