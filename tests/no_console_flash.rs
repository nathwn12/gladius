//! Console-flash / headless-backend guard: every process-spawn site must
//! carry the windowless `CREATE_NO_WINDOW` flag, and engine resolution must
//! never depend on an ambient or SHARED browser. Scans source text so the
//! guarantee cannot silently regress.
//!
//! Intent (not a word ban): gladius owns exactly one pinned browser - the
//! `chrome-headless-shell` build provisioned by `scripts/fetch-chromium.ps1`
//! into the versioned cache and resolved ONLY via `GLADIUS_CHROMIUM` (explicit
//! path) or `GLADIUS_CHROMIUM_CACHE`/`%LOCALAPPDATA%` (pinned cache root). A
//! pinned, hash-verified browser we provision ourselves satisfies the intent;
//! resolving a system install, a Playwright/Puppeteer shared cache, a bare
//! binary off `PATH`, or the full headed `chrome-win64` artifact breaks it.

use std::path::PathBuf;

fn read_rel(rel: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// Every `Command::new` line must have a `creation_flags(0x08000000)` line
/// within 30 lines after it (covers the shared `std_cmd` spawn pattern,
/// where two `Command::new` sites feed one flagged builder).
fn assert_flagged(rel: &str, text: &str) {
    let lines: Vec<&str> = text.lines().collect();
    for (i, ln) in lines.iter().enumerate() {
        // `Command::new(` with the paren: matches real spawn sites, not
        // string literals naming the pattern (like this guard's own scan).
        if ln.contains("Command::new(") {
            let window = lines
                .iter()
                .skip(i)
                .take(31)
                .any(|w| w.contains("creation_flags(0x08000000)"));
            assert!(
                window,
                "{rel}: spawn at line {} lacks CREATE_NO_WINDOW within 30 lines: {ln}",
                i + 1
            );
        }
    }
}

#[test]
fn every_src_spawn_is_windowless() {
    for name in [
        "browser.rs",
        "fetch.rs",
        "main.rs",
        "mcp.rs",
        "selftest.rs",
        "tools.rs",
    ] {
        let text = read_rel(&format!("src/{name}"));
        assert_flagged(&format!("src/{name}"), &text);
    }
}

#[test]
fn every_test_spawn_is_windowless() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut checked = 0;
    for e in std::fs::read_dir(&dir).expect("read tests dir").flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("rs") {
            continue;
        }
        // The guard itself only names the pattern in string literals.
        if p.file_name().and_then(|x| x.to_str()) == Some("no_console_flash.rs") {
            continue;
        }
        // The headless assertions spawn the pinned browser and probe the OS
        // (powershell/tasklist): those sites carry the flag like all others.
        let text = std::fs::read_to_string(&p).expect("read test file");
        if text.contains("Command::new(") {
            checked += 1;
            assert_flagged(
                &format!("tests/{}", p.file_name().unwrap().to_string_lossy()),
                &text,
            );
        }
    }
    assert!(checked > 0, "guard test found no test spawns to check");
}

/// Forbidden ambient/shared-browser resolutions (case-insensitive
/// substrings). Each is a shape gladius must never depend on:
///
/// - `ms-playwright` / `playwright`: Playwright's shared browser cache
///   (GC-deleted, someone else's revisions).
/// - `puppeteer` / `.cache/puppeteer`: Puppeteer's shared cache ditto.
/// - `msedge`, `brave`: system browsers (headed-capable, someone else's).
/// - `program files`: system install locations.
/// - `chrome-win64`: the FULL headed-capable CfT artifact (never provisioned;
///   only `chrome-headless-shell-win64` is ever used).
/// - `chrome.exe` (bare): any ambient Chrome binary off `PATH` or an
///   install dir. The owned `chrome-headless-shell.exe` never matches
///   (after `chrome` comes `-headless`, not `.exe`), so this bites ambient
///   resolutions without false-positiving on the pinned shell.
/// - `google/chrome/application`: the per-user install layout
///   (`%LOCALAPPDATA%/Google/Chrome/Application/chrome.exe`, no
///   `program files` in the path, which is why the `program files`
///   pattern alone misses it). Backslashes are normalized to `/` before
///   matching, so the Windows spelling hits the same pattern without a
///   second literal.
/// - `gladius_chrome` / `gladius_backend` (bare): the removed
///   shared-cache-era knobs (`GLADIUS_CHROME`, `GLADIUS_BACKEND`).
///   `GLADIUS_BACKEND_CAPS` is the only legitimate `GLADIUS_BACKEND*` name
///   and is exempted before matching; `GLADIUS_CHROMIUM` /
///   `GLADIUS_CHROMIUM_CACHE` are the owned knobs and never match these.
/// - `--remote-debugging-pipe`: the piped transport with no HTTP surface
///   (the readiness poll cannot work on it).
/// - `--no-sandbox`, `--disable-dev-shm-usage`: Windows-unsupported /
///   Linux-container flags that must never appear in our argv.
/// - `--headless=new`, `--headless=old`: wrong headless selectors (the
///   first is the full browser's flag, the second was removed at M132).
fn forbidden_patterns() -> Vec<&'static str> {
    vec![
        "ms-playwright",
        "playwright",
        "puppeteer",
        ".cache/puppeteer",
        "msedge",
        "brave",
        "program files",
        "chrome-win64",
        "chrome.exe",
        "google/chrome/application",
        "gladius_chrome",
        "gladius_backend",
        "--remote-debugging-pipe",
        "--no-sandbox",
        "--disable-dev-shm-usage",
        "--headless=new",
        "--headless=old",
    ]
}

/// Scan one source text, returning `src/{name}:{line}` hits. The legitimate
/// caps knob (`GLADIUS_BACKEND_CAPS`) is scrubbed before matching so only
/// the removed bare `GLADIUS_BACKEND` knob (and `GLADIUS_CHROME`) can hit.
fn scan_text(name: &str, text: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for (i, ln) in text.lines().enumerate() {
        // Normalize once: backslashes become `/` so the Windows spelling of
        // a path (`AppData\Local\Google\Chrome\Application\...`) hits the
        // same forward-slash patterns (no second literal needed).
        let low = ln.to_lowercase().replace('\\', "/");
        let scrubbed = low
            .replace("gladius_backend_caps", "")
            .replace("gladius_chromium_cache", "")
            .replace("gladius_chromium", "");
        for pat in forbidden_patterns() {
            if scrubbed.contains(pat) {
                hits.push(format!("src/{name}:{}: [{pat}] {ln}", i + 1));
                break;
            }
        }
    }
    hits
}

#[test]
fn self_containment_no_ambient_or_shared_browser_in_src() {
    // The owned pinned browser (`chrome-headless-shell` via `GLADIUS_CHROMIUM`
    // / `GLADIUS_CHROMIUM_CACHE` / `%LOCALAPPDATA%`) is the ONLY resolution;
    // any ambient or shared-browser resolution regressed into `src/` fails
    // the build here.
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits: Vec<String> = Vec::new();
    let mut files = 0;
    for e in std::fs::read_dir(&dir).expect("read src dir").flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("rs") {
            continue;
        }
        files += 1;
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&p).expect("read src file");
        hits.extend(scan_text(&name, &text));
    }
    assert!(files > 0, "guard scanned no src files");
    assert!(
        hits.is_empty(),
        "ambient/shared-browser resolution regressed into src/ (self-containment broken):\n{}",
        hits.join("\n")
    );
}

#[test]
fn guard_bites_known_ambient_resolutions() {
    // Positive control: the scanner MUST flag every forbidden shape. A check
    // that cannot fail proves nothing - if any sample below ever scans
    // clean, the guard is vacuous and this test fails.
    let bad_samples = [
        ("playwright cache", "resolve the newest ms-playwright chromium revision"),
        ("playwright", "find the newest playwright chromium revision"),
        ("puppeteer cache", "resolve from .cache/puppeteer/chrome"),
        ("system edge", "launch msedge headless from its install dir"),
        ("system brave", "launch brave.exe from Program Files"),
        ("full chrome zip", "download chrome-win64.zip for headed use"),
        ("bare chrome exe", "launch chrome.exe from PATH"),
        (
            "per-user chrome path",
            "AppData/Local/Google/Chrome/Application/chrome.exe",
        ),
        (
            "per-user chrome path (windows spelling)",
            r"AppData\Local\Google\Chrome\Application\chrome.exe",
        ),
        ("old chrome knob", "std::env::var(\"GLADIUS_CHROME\")"),
        ("old backend knob", "std::env::var(\"GLADIUS_BACKEND\")"),
        ("debug pipe", "spawn with --remote-debugging-pipe"),
        ("no-sandbox", "push --no-sandbox for containers"),
        ("dev-shm", "push --disable-dev-shm-usage on linux"),
        ("headless=new", "args.push(\"--headless=new\")"),
        ("headless=old", "args.push(\"--headless=old\")"),
    ];
    for (what, sample) in bad_samples {
        let hits = scan_text("sample.rs", sample);
        assert!(
            !hits.is_empty(),
            "guard is vacuous: it does not bite the {what} sample: {sample:?}"
        );
    }
    // And the owned shapes must scan CLEAN, so the guard passes with our
    // pinned binary present and cannot false-positive on it. The accepted
    // browser shape is the COMPLETE versioned pinned layout
    // (`<version>-<zip-sha8>/chrome-headless-shell-win64/chrome-headless-shell.exe`),
    // never a bare exe name or an artifact-relative suffix alone.
    let good_samples = [
        "let exe = find_chromium()?;",
        "std::env::var(\"GLADIUS_CHROMIUM\")",
        "std::env::var(\"GLADIUS_CHROMIUM_CACHE\")",
        "std::env::var(\"GLADIUS_BACKEND_CAPS\")",
        "155.0.8059.84-5ca590a4/chrome-headless-shell-win64/chrome-headless-shell.exe",
        "args.push(\"--headless\")",
    ];
    for sample in good_samples {
        let hits = scan_text("sample.rs", sample);
        assert!(
            hits.is_empty(),
            "guard false-positives on the owned shape: {sample:?}: {hits:?}"
        );
    }
}
