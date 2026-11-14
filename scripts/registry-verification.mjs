import { setTimeout as delay } from "node:timers/promises";

export async function publishAndVerify({ publish, verify }) {
  await publish();
  return verify();
}

export async function verifyRegistry({ fetch, version, integrity, attempts = 30, intervalMs = 5000, download, hash }) {
  let lastStatus = "not found";
  let verified;
  let latest;
  for (let attempt = 0; attempt < attempts; attempt++) {
    for (const [url, kind] of [
      [`https://registry.npmjs.org/gladius-mcp/${version}`, "exact"],
      ["https://registry.npmjs.org/gladius-mcp/latest", "latest"],
    ]) {
      const response = await fetch(url);
      if (response.status === 401 || response.status === 403) throw new Error(`registry ${kind} authentication failed (HTTP ${response.status})`);
      if (response.status === 404 || response.status >= 500) {
        lastStatus = `${kind} HTTP ${response.status}`;
        continue;
      }
      if (!response.ok) throw new Error(`registry ${kind} lookup HTTP ${response.status}`);
      const metadata = await response.json();
      if (kind === "exact") {
        if (metadata.dist?.integrity && metadata.dist.integrity !== integrity) throw new Error("published registry integrity mismatch");
        if (metadata.version === version && metadata.dist?.integrity === integrity) verified = metadata;
        else lastStatus = `exact version ${metadata.version ?? "unavailable"} without matching integrity`;
      } else {
        if (metadata.dist?.integrity && metadata.dist.integrity !== integrity && metadata.version === version) throw new Error("registry latest integrity mismatch");
        if (metadata.version === version && metadata.dist?.integrity === integrity) latest = metadata;
        else lastStatus = `latest version ${metadata.version ?? "unavailable"}`;
      }
    }
    if (verified && latest) {
      const bytes = await download(verified.dist.tarball);
      if (hash(bytes) !== integrity) throw new Error("registry tarball bytes mismatch");
      return verified;
    }
    if (attempt + 1 < attempts) await delay(intervalMs);
  }
  throw new Error(`registry verification timed out after ${attempts} attempts; last observed: ${lastStatus}`);
}
