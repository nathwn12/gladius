#!/usr/bin/env node
// Transport local bytes as git blobs, without creating or moving a ref.
// The staging workflow turns those exact blobs into an expiring run artifact.
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { need, sha256, verifyReceipt, verifyArtifacts, validateBlobMap } from "./release-handoff.mjs";

const repo = "nathwn12/gladius";
const gh = (args, input) => execFileSync("gh", args, { input, encoding: "utf8", windowsHide: true, maxBuffer: 64 * 1024 * 1024 });
try {
  const download = process.argv[2] === "--download";
  const dir = path.resolve(process.argv[3] || "");
  need(["--upload", "--download"].includes(process.argv[2]) && process.argv.length === 4, "usage: stage-release.mjs --upload <directory> | --download <new-directory>");
  if (download) {
    need(process.env.GITHUB_REPOSITORY === repo && process.env.GITHUB_REF === "refs/heads/main" && process.env.GITHUB_SHA === process.env.COMMIT, "staging requires exact main commit");
    const blobs = validateBlobMap(process.env.BLOB_MAP);
    need(!fs.existsSync(dir), "staging directory already exists");
    fs.mkdirSync(dir, { recursive: true });
    for (const [name, hash] of Object.entries(blobs)) {
      const blob = JSON.parse(gh(["api", `repos/${repo}/git/blobs/${hash}`]));
      need(blob.encoding === "base64" && blob.sha === hash, "blob identity mismatch");
      fs.writeFileSync(path.join(dir, name), Buffer.from(blob.content, "base64"));
    }
    const r = verifyReceipt(dir, { commit: process.env.COMMIT, version: process.env.VERSION, receiptSha256: process.env.RECEIPT_SHA256 });
    verifyArtifacts(dir, r);
    console.log("Staged exact local bytes; no release or registry write.");
  } else {
    const bytes = fs.readFileSync(path.join(dir, "release-receipt.json"));
    const receipt = JSON.parse(bytes);
    const receiptSha256 = sha256(bytes);
    const r = verifyReceipt(dir, { commit: receipt.commit, version: receipt.version, receiptSha256 });
    verifyArtifacts(dir, r);
    need(gh(["api", `repos/${repo}/git/ref/heads/main`, "--jq", ".object.sha"]).trim() === r.commit, "land the exact tested commit on main before staging");
    const blobs = {};
    for (const name of ["release-receipt.json", ...r.artifacts.map((a) => a.name)]) {
      const content = fs.readFileSync(path.join(dir, name)).toString("base64");
      const created = JSON.parse(gh(["api", `repos/${repo}/git/blobs`, "--input", "-"], JSON.stringify({ content, encoding: "base64" })));
      blobs[name] = created.sha;
    }
    const payload = { ref: "main", inputs: { commit: r.commit, version: r.version, receipt_sha256: receiptSha256, blobs: JSON.stringify(blobs) } };
    gh(["api", `repos/${repo}/actions/workflows/stage-release.yml/dispatches`, "--input", "-"], JSON.stringify(payload));
    console.log(`Staging dispatched once. commit=${r.commit} version=${r.version} receipt_sha256=${receiptSha256}`);
    console.log("Read its run ID and result before dispatching release.yml; do not repeat this upload.");
  }
} catch (err) {
  console.error(`stage-release: ${err.message}`);
  process.exitCode = 1;
}
