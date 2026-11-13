//! S2.5: a closed shadow root gets ground truth and a typed name (SPEC.md §6).
//!
//! A closed shadow root is invisible to the page realm: `document.querySelector`
//! can never match inside it, so the old answer (`invalid_target`/`not_found`)
//! was a lie - the element exists. The answer must be `shadow_closed`/
//! `unreachable`, whose prose names the one lever that works: the AX `@eN` ref
//! from `snapshot`.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{json, Value};

fn fixture_url() -> String {
    let p = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/shadow-closed.html"
    );
    #[cfg(windows)]
    let p = p.replace('\\', "/");
    format!("file:///{p}")
}

struct Client {
    stdin: ChildStdin,
    rx: Receiver<String>,
    next_id: i64,
    child: Child,
}

impl Client {
    fn spawn() -> Self {
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
        let stdin = child.stdin.take().expect("child stdin");
        let stdout: ChildStdout = child.stdout.take().expect("child stdout");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match r.read_line(&mut line) {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        let _ = tx.send(line.trim_end().to_string());
                    }
                    Err(_) => break,
                }
            }
        });
        Client {
            stdin,
            rx,
            next_id: 1,
            child,
        }
    }

    fn recv(&self, what: &str) -> Value {
        let line = self
            .rx
            .recv_timeout(Duration::from_secs(120))
            .unwrap_or_else(|_| panic!("timeout waiting for reply to {what}"));
        serde_json::from_str(&line).unwrap_or_else(|_| panic!("bad json for {what}: {line}"))
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let frame = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{frame}").expect("write stdin");
        self.stdin.flush().expect("flush stdin");
        let resp = self.recv(method);
        assert_eq!(resp.get("id"), Some(&json!(id)), "id echo for {method}");
        resp
    }

    fn result(&mut self, method: &str, params: Value) -> Value {
        let resp = self.request(method, params);
        assert!(
            resp.get("error").is_none(),
            "{method} protocol error: {resp}"
        );
        resp.get("result").cloned().unwrap_or(Value::Null)
    }

    fn call(&mut self, name: &str, args: Value) -> Value {
        self.result("tools/call", json!({"name": name, "arguments": args}))
    }
}

fn content_text(v: &Value) -> String {
    v.get("content")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string()
}

fn code_of(v: &Value) -> String {
    v.pointer("/structuredContent/code")
        .and_then(|c| c.as_str())
        .unwrap_or("<none>")
        .to_string()
}

fn reason_of(v: &Value) -> String {
    v.pointer("/structuredContent/reason")
        .and_then(|c| c.as_str())
        .unwrap_or("<none>")
        .to_string()
}

/// The ref `snapshot` issued for the element whose accessible name is
/// `needle`, or None when the AX tree does not carry it.
fn ref_for(snapshot_text: &str, needle: &str) -> Option<String> {
    for line in snapshot_text.lines() {
        if line.contains(needle) {
            // The rendered line shape is `@eN ... name ...`; take the first
            // `@e` token on it.
            if let Some(i) = line.find('@') {
                let rest = &line[i + 1..];
                let end = rest
                    .find(|c: char| !c.is_ascii_alphanumeric())
                    .unwrap_or(rest.len());
                if end > 0 {
                    return Some(rest[..end].to_string());
                }
            }
        }
    }
    None
}

#[test]
fn closed_shadow_target_is_typed_not_not_found() {
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let nav = c.call("navigate", json!({"url": fixture_url()}));
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    // --- 1. the selector path cannot reach it: typed, never `not_found` ---
    let by_selector = c.call("click", json!({"element": "#inner"}));
    assert!(
        by_selector.get("isError").is_some(),
        "a selector cannot reach into a closed shadow root: {by_selector}"
    );
    assert_eq!(
        code_of(&by_selector),
        "shadow_closed",
        "the answer is shadow_closed, not not_found: {by_selector}"
    );
    assert_eq!(
        reason_of(&by_selector),
        "unreachable",
        "the reason is unreachable: {by_selector}"
    );
    let prose = content_text(&by_selector);
    assert!(
        prose.contains("closed shadow root") && prose.contains("@eN"),
        "the prose names the closed root and the @eN lever: {prose}"
    );

    // The same for `type`, `hover` and `layout`, so no action tool lies about
    // a closed-root target.
    for (tool, args) in [
        ("type", json!({"element": "#inner", "text": "x"})),
        ("hover", json!({"element": "#inner"})),
        ("layout", json!({"selector": "#inner"})),
    ] {
        let r = c.call(tool, args);
        assert_eq!(
            code_of(&r),
            "shadow_closed",
            "{tool} on a closed-root selector is typed: {r}"
        );
    }

    // --- 2. a genuine miss is still not_found (the probe is precise) ---
    let missing = c.call("click", json!({"element": "#definitely-not-here"}));
    assert_eq!(
        code_of(&missing),
        "invalid_target",
        "a selector that matches nothing anywhere is still not_found: {missing}"
    );
    assert_eq!(reason_of(&missing), "not_found", "{missing}");

    // --- 3. the AX `@eN` ref is the supported path ---
    let snap = c.call("snapshot", json!({}));
    let snap_text = content_text(&snap);
    let reference = ref_for(&snap_text, "CLOSED_INNER_OK");
    assert!(
        reference.is_some(),
        "the AX tree carries the closed-root button: {}",
        snap_text.chars().take(600).collect::<String>()
    );
    let reference = reference.expect("ref");
    let by_ref = c.call("click", json!({"element": format!("@{reference}")}));
    // The engine either resolves it (the documented supported path) or proves
    // it unreachable; the test accepts exactly those two answers and records
    // which one this engine gives.
    if by_ref.get("isError").is_none() {
        println!("closed-shadow @eN path: CLICKS (ref @{reference})");
        let clicks = c.call("evaluate", json!({"expression": "window.__closedClicks"}));
        assert!(
            content_text(&clicks).contains('1'),
            "the @eN click reached the closed-root button: {clicks}"
        );
    } else {
        println!("closed-shadow @eN path: TYPED UNREACHABLE ({by_ref})");
        assert_eq!(
            code_of(&by_ref),
            "shadow_closed",
            "the @eN path answers shadow_closed when the engine proves unreachable: {by_ref}"
        );
    }

    let _ = c.call("close", json!({}));
    drop(c.stdin);
    let mut child = c.child;
    for _ in 0..100 {
        match child.try_wait() {
            Ok(Some(status)) => {
                assert!(status.success(), "server exit status: {status}");
                return;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => panic!("try_wait: {e}"),
        }
    }
    child.kill().ok();
    panic!("server did not exit after stdin EOF");
}
