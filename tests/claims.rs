//! W1 acceptance tests: the claims match the code (SPEC.md §5, S1.9).
//!
//! These are mechanical checks over the docs tree, so a claim cannot outrun
//! what `capture.rs` actually freezes.

use std::path::{Path, PathBuf};

fn docs_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("docs");
    p
}

fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            // The frozen spec bundle is evidence, not product documentation:
            // SPEC.md and the audit reports quote the unscoped claim as the
            // defect under attack, and their hashes are recorded in FREEZE.md,
            // so they cannot be reworded. Only the shipped docs are checked.
            if p.ends_with(Path::new("docs/specs")) {
                continue;
            }
            markdown_files(&p, out);
        } else if p.extension().and_then(|x| x.to_str()) == Some("md") {
            out.push(p);
        }
    }
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// SPEC.md S1.9: no doc may claim byte identity without scoping it.
///
/// Byte identity is over PNG bytes from gladius's OWN encoder, and the freeze
/// has a scope (main document, same-process frames, open shadow roots, CSS
/// animation, WAAPI, SMIL, media). So a paragraph that says "provably
/// identical" or "byte-identical" must also say "encoder" or "same engine",
/// and the guide must carry the `sampled` boundary and the no-scroll sentence.
#[test]
fn docs_scope_the_determinism_claim() {
    let mut files = Vec::new();
    markdown_files(&docs_dir(), &mut files);
    assert!(!files.is_empty(), "docs/ has markdown to check");

    let mut offenders: Vec<String> = Vec::new();
    for f in &files {
        let text = read(f);
        // Paragraph = blank-line separated block, which is the unit a reader
        // takes as one claim.
        for para in text.split("\n\n") {
            let low = para.to_ascii_lowercase();
            let claims = low.contains("provably identical") || low.contains("byte-identical");
            if !claims {
                continue;
            }
            let scoped = low.contains("encoder") || low.contains("same engine");
            if !scoped {
                offenders.push(format!(
                    "{}: unscoped identity claim: {}",
                    f.display(),
                    para.lines().next().unwrap_or("").trim()
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "every identity claim must name its encoder scope:\n{}",
        offenders.join("\n")
    );

    let guide = read(&docs_dir().join("GUIDE.md"));
    assert!(
        guide.contains("sampled"),
        "docs/GUIDE.md must state the 'sampled' boundary"
    );
    assert!(
        guide.contains("no scrolling is performed"),
        "docs/GUIDE.md must state that no scrolling is performed, so below-the-fold lazy content is captured unloaded"
    );
}
