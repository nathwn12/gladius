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

// --------------------------------------------------------------- S3.5 tools

/// A minimal MCP client: `claims.rs` needs the LIVE `tools/list`, and that is
/// answered without launching an engine, so no browser is involved here.
fn tools_list() -> Vec<String> {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    let bin = env!("CARGO_BIN_EXE_gladius");
    let mut cmd = Command::new(bin);
    cmd.arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().expect("spawn gladius mcp");
    let mut stdin = child.stdin.take().expect("child stdin");
    let stdout = child.stdout.take().expect("child stdout");
    let mut reader = BufReader::new(stdout);

    let ask = |stdin: &mut std::process::ChildStdin, reader: &mut BufReader<_>, frame: &str| {
        writeln!(stdin, "{frame}").expect("write stdin");
        stdin.flush().expect("flush stdin");
        let mut line = String::new();
        reader.read_line(&mut line).expect("read stdout");
        serde_json::from_str::<serde_json::Value>(line.trim())
            .unwrap_or_else(|_| panic!("bad json from the server: {line}"))
    };

    let init = ask(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
    );
    assert!(init.get("error").is_none(), "initialize: {init}");
    let list = ask(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
    );
    drop(stdin);
    let _ = child.wait();
    let tools = list
        .pointer("/result/tools")
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(!tools.is_empty(), "tools/list returned nothing: {list}");
    tools
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(str::to_string))
        .collect()
}

/// Every "N tools" statement in the shipped docs must equal the live
/// `tools/list` length.
///
/// A document that opens by declaring itself an archive is evidence, not a
/// current claim: `docs/OPT-SURVEY.md` and `docs/RESEARCH.md` both say so in
/// their leading blockquote ("survey-time surface", "research-phase surface"),
/// and their counts are historical numbers by their own admission. Every other
/// doc is a claim about the binary that ships today.
fn is_archived_doc(text: &str) -> bool {
    text.lines().take(40).any(|l| {
        l.contains("Archive note")
            || l.contains("survey-time surface")
            || l.contains("research-phase surface")
    })
}

/// The numbers immediately before a " tools" word, with a word boundary before
/// them so "9-tool" and "1 toolsmith" are not counts.
fn tool_counts(line: &str) -> Vec<i64> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = line[from..].find(" tools") {
        let pos = from + rel;
        let mut j = pos;
        let mut digits = String::new();
        while j > 0 && bytes[j - 1].is_ascii_digit() {
            digits.insert(0, bytes[j - 1] as char);
            j -= 1;
        }
        let boundary = j == 0 || !bytes[j - 1].is_ascii_alphanumeric();
        if boundary && !digits.is_empty() {
            if let Ok(n) = digits.parse::<i64>() {
                out.push(n);
            }
        }
        from = pos + " tools".len();
    }
    out
}

fn backticked(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else { break };
        out.push(after[..end].to_string());
        rest = &after[end + 1..];
    }
    out
}

/// Drop parenthesised segments: in the README capability list a tool name is
/// backticked in the item itself, while the parenthetical that follows it
/// backticks its arguments and fields (`method`, `params`, `isTrusted:true`).
fn without_parentheses(line: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for c in line.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// SPEC.md S3.5: the countable claims match the binary.
///
/// - every "N tools" statement in `docs/*.md` and `README.md` equals the live
///   `tools/list` length (archived research docs are exempt: they say so);
/// - every tool name in backticks in the README capability list exists;
/// - no doc describes `navigate`'s reply with a `status` field (S4.6 names it
///   `httpStatus`, and the negative half is checkable now).
#[test]
fn countable_claims_match_the_binary() {
    let names = tools_list();
    let live = names.len() as i64;
    assert!(live > 0, "the binary lists its tools");

    let mut files: Vec<PathBuf> = Vec::new();
    markdown_files(&docs_dir(), &mut files);
    files.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("README.md"));

    let mut offenders: Vec<String> = Vec::new();
    for f in &files {
        let text = read(f);
        if is_archived_doc(&text) {
            continue;
        }
        for (i, line) in text.lines().enumerate() {
            for n in tool_counts(line) {
                if n != live {
                    offenders.push(format!(
                        "{}:{}: claims {n} tools, the binary lists {live}: {}",
                        f.display(),
                        i + 1,
                        line.trim()
                    ));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a tool-count claim must match tools/list:\n{}",
        offenders.join("\n")
    );

    // The README capability list names the tools; every name must exist.
    let readme = read(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("README.md"));
    let capability = readme
        .split("## What it does")
        .nth(1)
        .and_then(|rest| rest.split("That is ").next())
        .expect("README has a capability list between 'What it does' and 'That is'");
    let mut mentioned: Vec<String> = Vec::new();
    for line in capability.lines() {
        for tok in backticked(&without_parentheses(line)) {
            let looks_like_a_tool = !tok.is_empty()
                && tok.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && tok
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if looks_like_a_tool {
                mentioned.push(tok);
            }
        }
    }
    assert!(
        !mentioned.is_empty(),
        "the README capability list names tools in backticks"
    );
    for name in &mentioned {
        assert!(
            names.contains(name),
            "README names `{name}`, which the binary does not list: {names:?}"
        );
    }

    // `navigate` reports `httpStatus` (S4.6), never a field called `status`.
    let mut wrong: Vec<String> = Vec::new();
    for f in &files {
        let text = read(f);
        if is_archived_doc(&text) {
            continue;
        }
        for (i, line) in text.lines().enumerate() {
            let low = line.to_ascii_lowercase();
            if low.contains("navigate")
                && (line.contains("`status`") || line.contains("\"status\""))
            {
                wrong.push(format!("{}:{}: {}", f.display(), i + 1, line.trim()));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "no doc may call navigate's reply field `status` (it is `httpStatus`):\n{}",
        wrong.join("\n")
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
