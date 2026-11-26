#!/usr/bin/env node
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
const index = process.argv.indexOf("--file");
const file = index < 0 ? fileURLToPath(new URL("../.github/workflows/release.yml", import.meta.url)) : process.argv[index + 1];
const text = readFileSync(file, "utf8");
const here = dirname(fileURLToPath(import.meta.url));
const readSibling = (rel) => readFileSync(join(here, rel), "utf8");
const publishText = readSibling("publish-release.mjs");
const stageText = readSibling("../.github/workflows/stage-release.yml");
const prepText = readSibling("prepare-release.mjs");
// A release page is a remote write we no longer make anywhere in the lane.
const pageWrite = /(?:releases\/tags|api\(\s*["']releases|release\s*["'],\s*["']upload|browser_download_url|product-backup|["']PATCH["']|sharedRef|verifyReleaseAppend)/;
const noPages = (s) => !pageWrite.test(s);
const checks = [
  ["manual-only", /workflow_dispatch:/.test(text) && !/^  push:/m.test(text)],
  ["one-publish-job", /^  publish:/m.test(text) && !/^  build:/m.test(text)],
  ["serialized", text.includes("group: release-${{ github.repository }}") && text.includes("cancel-in-progress: false")],
  ["main-only", text.includes("github.ref == 'refs/heads/main'")],
  ["exact-commit", text.includes("COMMIT: ${{ inputs.commit }}") && text.includes("fetch-depth: 0")],
  ["receipt-inputs", text.includes("staging_run_id:") && text.includes("receipt_sha256:")],
  ["trusted-publisher", text.includes("id-token: write") && text.includes("registry-url: 'https://registry.npmjs.org'")],
  ["verify-before-publish", text.indexOf("publish-release.mjs --prepare") > 0 && text.indexOf("publish-release.mjs --prepare") < text.indexOf("publish-release.mjs --publish")],
  ["zero-release-pages", noPages(text) && noPages(publishText) && noPages(stageText)],
  ["runtime-ships-in-npm", (() => {
    const zipStaged = prepText.includes('path.join(pkg, "bin", bundle)') && /sha256\(read\(stagedZip\)\)/.test(prepText);
    const fallbackOnly = !/manifest\.runtime\.source\s*=/.test(prepText) && prepText.includes("pre-bundle fallback");
    const cleanroomFromTarball = !/cleanroom-check\.mjs[^)]*--bundle/.test(prepText) && /cleanroom-check\.mjs", "--tarball", tarball\]/.test(prepText);
    return zipStaged && fallbackOnly && cleanroomFromTarball;
  })()],
  ["dry-run", text.includes("default: true") && text.includes("if: inputs.dry_run == false") && text.includes("--dry-run --ignore-scripts")],
  ["no-rebuild", !/cargo (build|test)|selftest|fetch-chromium|cleanroom-check|npm pack/.test(text)],
  ["no-history-destruction", !/release delete|git push|--force|overwrite_files: true/.test(text)],
];
for (const [name, pass] of checks) console.log(`${pass ? "PASS" : "FAIL"} ${name}`);
console.log(`${checks.filter(([, pass]) => pass).length}/${checks.length} invariants hold`);
process.exitCode = checks.every(([, pass]) => pass) ? 0 : 1;
