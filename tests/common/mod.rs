//! Shared path derivation for the pinned `chrome-headless-shell`.
//!
//! The pin has exactly one home: `scripts/chromium-manifest.json`. `gladius` is
//! a binary-only crate, so an integration test cannot reach an internal
//! constant; these tests read the manifest itself rather than restating a
//! version - a literal is how three call sites came to name a browser the
//! manifest had already retired.
//!
//! Rules reproduced here, and where each one comes from:
//! - cache root: `$env:GLADIUS_CHROMIUM_CACHE` if set, else
//!   `%LOCALAPPDATA%\gladius\chromium` - `scripts/fetch-chromium.ps1` 116-122.
//! - cache dir: `<version>-<first 8 hex chars of the zip sha256>` -
//!   `scripts/fetch-chromium.ps1` 46 (`$ZipShort8 = $ExpectedZipSha256.Substring(0, 8)`)
//!   and 124 (`$CacheDir = Join-Path $CacheRoot ("{0}-{1}" -f $PinnedVersion, $ZipShort8)`).
//!   The short-8 rule lives here once, for every test; the script owns it for
//!   provisioning.
//! - exe: `<cache dir>\chrome-headless-shell-win64\chrome-headless-shell.exe` -
//!   `scripts/fetch-chromium.ps1` 50 and 125.

use std::path::{Path, PathBuf};

/// The manifest, addressed from the crate root, never the process cwd.
fn manifest() -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join("chromium-manifest.json");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// `(browser.version, browser.asset.sha256)` - the only two fields the cache
/// directory name is built from.
fn pin() -> (String, String) {
    let m = manifest();
    let browser = m
        .get("browser")
        .unwrap_or_else(|| panic!("chromium-manifest.json: no `browser` object"));
    let version = browser
        .get("version")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("chromium-manifest.json: no `browser.version`"))
        .to_owned();
    let sha256 = browser
        .get("asset")
        .and_then(|a| a.get("sha256"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("chromium-manifest.json: no `browser.asset.sha256`"))
        .to_ascii_lowercase();
    (version, sha256)
}

fn cache_root() -> PathBuf {
    match std::env::var("GLADIUS_CHROMIUM_CACHE") {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v.trim()),
        _ => {
            PathBuf::from(std::env::var("LOCALAPPDATA").expect("LOCALAPPDATA for pinned chromium"))
                .join("gladius")
                .join("chromium")
        }
    }
}

/// The pinned browser the engine and these tests both run. An explicit
/// `GLADIUS_CHROMIUM` exe still wins, as it did before this helper existed.
pub fn pinned_chromium_exe() -> PathBuf {
    if let Ok(p) = std::env::var("GLADIUS_CHROMIUM") {
        if !p.trim().is_empty() {
            return PathBuf::from(p.trim());
        }
    }
    let (version, sha256) = pin();
    let short8 = sha256.get(..8).unwrap_or_else(|| {
        panic!("chromium-manifest.json: browser.asset.sha256 shorter than 8 chars: {sha256}")
    });
    cache_root()
        .join(format!("{version}-{short8}"))
        .join("chrome-headless-shell-win64")
        .join("chrome-headless-shell.exe")
}
