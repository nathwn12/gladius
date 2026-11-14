#!/usr/bin/env node
// Structural check, not a YAML parser. Behavioral validation lives in the handoff tests.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
const index = process.argv.indexOf("--file");
const file = index < 0 ? fileURLToPath(new URL("../.github/workflows/release.yml", import.meta.url)) : process.argv[index + 1];
const text = readFileSync(file, "utf8");
const checks = [
  ["manual-only", /workflow_dispatch:/.test(text) && !/^  push:/m.test(text)],
  ["one-publish-job", /^  publish:/m.test(text) && !/^  build:/m.test(text)],
  ["serialized", text.includes("group: release-${{ github.repository }}") && text.includes("cancel-in-progress: false")],
  ["main-only", text.includes("github.ref == 'refs/heads/main'")],
  ["exact-commit", text.includes("COMMIT: ${{ inputs.commit }}") && text.includes("fetch-depth: 0")],
  ["receipt-inputs", text.includes("staging_run_id:") && text.includes("receipt_sha256:")],
  ["trusted-publisher", text.includes("id-token: write") && text.includes("registry-url: 'https://registry.npmjs.org'")],
  ["verify-before-publish", text.indexOf("publish-release.mjs --prepare") > 0 && text.indexOf("publish-release.mjs --prepare") < text.indexOf("publish-release.mjs --publish")],
  ["backup-before-write", text.indexOf("name: product-backup") > 0 && text.indexOf("name: product-backup") < text.indexOf("publish-release.mjs --publish")],
  ["dry-run", text.includes("default: true") && text.includes("if: inputs.dry_run == false") && text.includes("--dry-run --ignore-scripts")],
  ["no-rebuild", !/cargo (build|test)|selftest|fetch-chromium|cleanroom-check|npm pack/.test(text)],
  ["no-history-destruction", !/release delete|git push|--force|overwrite_files: true/.test(text)],
];
for (const [name, pass] of checks) console.log(`${pass ? "PASS" : "FAIL"} ${name}`);
console.log(`${checks.filter(([, pass]) => pass).length}/${checks.length} invariants hold`);
process.exitCode = checks.every(([, pass]) => pass) ? 0 : 1;
