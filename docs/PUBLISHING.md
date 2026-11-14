# Publishing gladius-mcp

Build and run the complete gate locally. GitHub Actions verifies the handoff and
publishes those exact bytes through the existing `release.yml` npm trusted publisher.
It does not compile, repack, provision a browser or rerun the suite.

Publication requires the owner's authorization. npm login, manual publishing and
trusted-publisher configuration changes require the owner's physical passkey approval.
Do not add a token or change the publisher identity to bypass that boundary.

## Local gate

Use Windows, the pinned Rust 1.88.0 toolchain, Node >= 22.14.0, npm >= 11.5.1,
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
It packs once and tests that exact tarball against the exact runtime ZIP.
It does not edit tracked manifests or close the working tree's changelog.

`release-receipt.json` records the exact commit, version, required passing checks,
and artifact sizes and SHA-256 values. Retain its separately printed SHA-256.
The receipt is a local assertion, not independent evidence that CI built the executable.
The build provisions a fresh browser inside its owned work directory, never the
shared cache. Ambient Gladius engine/browser overrides are scrubbed. The separate
cleanroom fetches the browser from Google into its own fresh profile; it never
inherits the build cache or bundles a browser.

## Stage once, then rehearse once

Land the tested commit on `main` without modifying it. If landing changes the
commit, prepare again against the landed revision. Do not reuse a receipt for a new SHA.

```powershell
node scripts/stage-release.mjs --upload target/release-candidate
gh run list --workflow stage-release.yml --limit 1
```

The staging command uploads public, credential-free git blobs without creating
or moving a ref. They contain only the already public release artifacts. A
read-only staging workflow verifies their receipt and produces the one-day
`release-handoff` workflow artifact. Git blobs are not a private or expiring store;
the workflow artifact expires, but the uploaded blob objects may remain accessible.
Never stage confidential material.

Read the staging run's result and ID. Dispatch `release.yml` on `main` with
`product_tag=gladius`, the exact `commit`, asserted `version`, `staging_run_id`,
`receipt_sha256` and `dry_run=true`. Inputs are assertions, not build instructions.
The publisher checks the staging workflow path, main branch, exact SHA, successful
conclusion, receipt, every staged byte, packaged manifest and executable, notes,
browser pin, registry integrity conflicts and existing product-tag binding.
Its only npm dry run is `npm publish <exact-tarball> --dry-run --ignore-scripts`.

A dry run makes no tag, release, asset or registry writes. It does not prove that
npm will authenticate an actual OIDC publish, or that npm will issue attestations.
Do not claim CI compiled a locally built executable. Inspect the one rehearsal
before the authorized `dry_run=false` dispatch; do not repeatedly rehearse or
automatically retry an uncertain write.

## Product release and immutable npm archives

The product page is `/releases/tag/gladius`. Current human-facing downloads use:

```text
https://github.com/nathwn12/gladius/releases/download/gladius/gladius-windows-x86_64.zip
```

That alias is mutable. Future npm tarballs instead pin the version-specific
`gladius-vX.Y.Z-windows-x86_64.zip` asset under the `gladius` release and its exact
size and SHA-256. These version-specific assets and their checksums accumulate
in the same release and are never overwritten or removed. New npm versions
therefore do not depend on mutable current bytes. Existing manifest sentinel
URLs in the source tree remain derivation inputs; preparation fills the product
URL only in the staged package, never repairs a source version.

The publisher backs up an existing product release's metadata and every asset,
verifies their hashes, and uploads that backup as a 90-day workflow artifact
before replacement. Retain a durable local copy before that artifact expires.
It verifies remote state again before writing. Existing immutable assets or
registry versions with different bytes are conflicts, not overwrite candidates.
After publication it fetches and hashes all current/immutable downloads and the
registry tarball, checks `latest`, and verifies release metadata.

The publisher can create an absent product tag. It deliberately refuses to move
an existing tag to a different commit. A later update needs a specific approved
tag-movement operation, with the previous target recorded and backed up first.
No workflow in this lane moves shared refs, deletes historical tags or removes
historical release objects. Do not bypass that refusal with a force push.
Version derivation reads the package version at the reachable product tag, while
retaining legacy version tags as historical anchors.

## First-cutover audit and backup

`gladius-mcp@0.7.1` embeds:

```text
https://github.com/nathwn12/gladius/releases/download/v0.7.1/gladius-v0.7.1-windows-x86_64.zip
size: 5166733
sha256: 6cb9d77a9e30e45907898e5ad61d5946cb79b02214bdeecab0f687e1a4182d9c
```

Removing or retagging its release breaks fresh installs of that immutable package.
Keeping the old Git tag alone does not retain release assets. The owner accepted
that compatibility risk, but this implementation does not perform removal.
Until a separately authorized cutover handles that historical object, the old
entry remains; creating a product entry would temporarily produce two entries.
Do not report the one-entry migration complete in that state.

The audited original release ID is `431913217`. Its three assets were downloaded
to `release-backups/v0.7.1` and verified against GitHub asset digests and the
published npm runtime pin. This ignored local directory is operator state, not
included in a clone. Retain it before authorizing any historical removal. `verified-backup.json` records
the original tag target and all asset sizes/hashes; `release.json` records the body
and metadata. Unknown external consumers remain unverified.

Recovery, if explicitly approved, recreates the old release at its preserved tag
and uploads the exact original basenames and bytes. Verify the original URLs and
hashes afterward. A backup does not preserve the release ID or uninterrupted
service. Never delete or recreate a published npm version.

## Cheap checks

```powershell
node --test scripts/release-handoff.test.mjs
node scripts/derive-release.mjs --check
node scripts/check-release-workflow.mjs
```

These do not replace the full local preparation gate or the single CI rehearsal.
