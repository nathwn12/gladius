//! Console-flash / headless-backend guard: every process-spawn site must
//! carry the windowless `CREATE_NO_WINDOW` flag, and engine resolution must
//! not fall back to a system (headed-capable) browser. Scans source text so
//! the guarantee cannot silently regress.

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

#[test]
fn self_containment_no_foreign_engine_in_src() {
    // Gladius is self-contained: the owned Obscura engine in `bin\` is the
    // ONLY launchable engine, plus an explicit external attach by URL. Any
    // reintroduction of a foreign-engine reference in `src/` fails the
    // build here. Tokens (case-insensitive):
    // - `chrome` covers chrome-headless-shell / system-Chrome paths and the
    //   `GLADIUS_CHROME` override alike; `GLADIUS_BACKEND_CAPS` is the only
    //   legitimate `GLADIUS_BACKEND*` name and is exempted before matching.
    // - `ms-playwright` / `playwright` cover the shared-cache resolution.
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
        for (i, ln) in text.lines().enumerate() {
            let low = ln.to_lowercase();
            // Exempt the legitimate caps knob before matching.
            let scrubbed = low.replace("gladius_backend_caps", "");
            if scrubbed.contains("chrome")
                || scrubbed.contains("playwright")
                || scrubbed.contains("gladius_chrome")
                || scrubbed.contains("gladius_backend")
            {
                hits.push(format!("src/{name}:{}: {ln}", i + 1));
            }
        }
    }
    assert!(files > 0, "guard scanned no src files");
    assert!(
        hits.is_empty(),
        "foreign-engine reference regressed into src/ (self-containment broken):\n{}",
        hits.join("\n")
    );
}
