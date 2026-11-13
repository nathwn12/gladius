//! W1/W3 acceptance tests: the claims match the code (SPEC.md §5 S1.9, §7
//! S3.4 + S3.5).
//!
//! These are mechanical checks over the docs tree and the shipped launcher, so
//! a claim cannot outrun what the binary and the release lane actually do.

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

// --------------------------------------------------------------- S3.4 commit

/// Run the shipped launcher's diagnostic and parse its JSON report.
fn doctor_report() -> serde_json::Value {
    use std::process::Command;

    let mut cmd = Command::new("node");
    cmd.arg("npm/bin/gladius-mcp.js")
        .arg("--doctor")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().unwrap_or_else(|e| {
        panic!("run `node npm/bin/gladius-mcp.js --doctor` (node on PATH): {e}")
    });
    assert!(
        out.status.success(),
        "--doctor exits 0: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "--doctor prints JSON: {e}\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// SPEC.md S3.4: the release names the commit it was built from.
///
/// The launcher's `--doctor` must carry a commit line, the engine manifest is
/// where that value lives (the release workflow fills it from
/// `git rev-parse HEAD`), and the release lane must assert the published body
/// names the same commit. On a working checkout the committed value is the
/// `unpublished` sentinel - a commit cannot name its own sha - so the
/// assertion is that the line EXISTS and is either a real 40-hex commit or that
/// sentinel, never absent.
#[test]
fn doctor_reports_the_commit_it_was_built_from() {
    let report = doctor_report();
    let commit = report
        .get("commit")
        .and_then(|c| c.as_str())
        .unwrap_or_else(|| panic!("--doctor carries a commit line: {report}"));
    let real = commit.len() == 40 && commit.chars().all(|c| c.is_ascii_hexdigit());
    assert!(
        real || commit == "unpublished",
        "--doctor's commit is a 40-hex sha or the unpublished sentinel, got {commit:?}"
    );
    assert!(
        report
            .get("commitSource")
            .and_then(|s| s.as_str())
            .is_some(),
        "--doctor says where the commit came from: {report}"
    );

    // The value the doctor reports is the manifest's, and the manifest is
    // where the release workflow writes it.
    let man_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("npm/engine-manifest.json");
    let man: serde_json::Value =
        serde_json::from_str(&read(&man_path)).expect("engine-manifest.json is JSON");
    assert_eq!(
        man.get("commit").and_then(|c| c.as_str()),
        Some(commit),
        "--doctor reports the manifest's commit"
    );

    // The lane that fills it: the manifest commit comes from HEAD, and the
    // body is asserted to carry it before anything is published.
    let workflow =
        read(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/release.yml"));
    assert!(
        workflow.contains("$man.commit = $commit"),
        "the release workflow writes the commit into the engine manifest"
    );
    assert!(
        workflow.contains("--assert-body dist/release-notes.md"),
        "the release workflow asserts the body names its commit"
    );
}
