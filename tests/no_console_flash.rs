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
fn no_system_chrome_fallback() {
    let text = read_rel("src/browser.rs");
    assert!(
        !text.contains(r"Program Files\Google\Chrome"),
        "system-Chrome fallback regressed into src/browser.rs"
    );
    assert!(
        !text.contains(r"Program Files (x86)"),
        "system-browser fallback regressed into src/browser.rs"
    );
}
