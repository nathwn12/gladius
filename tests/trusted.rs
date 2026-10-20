//! G4a trusted main-frame input: `click` dispatches a real CDP pointer
//! sequence (isTrusted, pointerdown/mousedown/pointerup/mouseup/click) with
//! an elementFromPoint occlusion guard; `type` uses `Input.insertText` into
//! a focused + select-all target. Each test owns one server process and
//! reaps its browser.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{json, Value};

fn fixture_url() -> String {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/trusted.html");
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
            .stderr(Stdio::inherit());
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
                    Ok(0) => break,
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

fn structured(v: &Value) -> Value {
    v.get("structuredContent").cloned().unwrap_or(Value::Null)
}

fn is_error(v: &Value) -> bool {
    v.get("isError").is_some()
}

fn shutdown(c: Client) {
    drop(c.stdin);
    let mut child = c.child;
    let mut exited = false;
    for _ in 0..100 {
        match child.try_wait() {
            Ok(Some(status)) => {
                assert!(status.success(), "server exit status: {status}");
                exited = true;
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => panic!("try_wait: {e}"),
        }
    }
    if !exited {
        child.kill().ok();
        panic!("server did not exit after stdin EOF");
    }
}

fn launch_on_trusted() -> Client {
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let nav = c.call("navigate", json!({"url": fixture_url()}));
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");
    c
}

fn finish(mut c: Client) {
    let closed = c.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
    shutdown(c);
}

#[test]
fn click_is_trusted_pointer_sequence() {
    let mut c = launch_on_trusted();
    // Reset the recorder (a prior run on the same page must not leak).
    c.call(
        "evaluate",
        json!({"expression": "window.__clickEvents=[]; window.__clickTrusted=[]; 'reset'"}),
    );
    let ok = c.call("click", json!({"element": "#trusted-btn"}));
    assert!(!is_error(&ok), "click trusted ok: {ok}");
    let landed = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('trusted-result').textContent"}),
    );
    assert!(
        content_text(&landed).contains("CLICKED_OK"),
        "trusted click landed: {}",
        content_text(&landed)
    );
    // Event order: pointerdown -> mousedown -> pointerup -> mouseup -> click.
    let order = c.call(
        "evaluate",
        json!({"expression": "JSON.stringify(window.__clickEvents)"}),
    );
    let ot = content_text(&order);
    let (mut last, mut ok_order) = (0usize, true);
    for t in ["pointerdown", "mousedown", "pointerup", "mouseup", "click"] {
        match ot[last..].find(t) {
            Some(i) => last += i + t.len(),
            None => {
                ok_order = false;
                break;
            }
        }
    }
    // Exactly one of each: five events total.
    let len = c.call(
        "evaluate",
        json!({"expression": "window.__clickEvents.length"}),
    );
    assert!(ok_order, "trusted pointer order, got: {ot}");
    assert!(
        content_text(&len).contains('5'),
        "one of each event, got: {} / {ot}",
        content_text(&len)
    );
    // Every event in the sequence is trusted (exactly one of each).
    let trusted = c.call(
        "evaluate",
        json!({"expression": "JSON.stringify(window.__clickTrusted)"}),
    );
    let tt = content_text(&trusted);
    assert!(
        tt.contains("[true,true,true,true,true]"),
        "all five events isTrusted:true, got: {tt}"
    );
    finish(c);
}

#[test]
fn click_occluded_is_typed() {
    let mut c = launch_on_trusted();
    let r = c.call("click", json!({"element": "#victim"}));
    assert!(is_error(&r), "occluded click isError: {r}");
    let sc = structured(&r);
    assert_eq!(
        sc.get("code"),
        Some(&json!("not_actionable")),
        "occluded code: {r}"
    );
    assert_eq!(
        sc.get("reason"),
        Some(&json!("occluded")),
        "occluded reason: {r}"
    );
    assert!(
        content_text(&r).contains("#victim"),
        "occluded prose names the element: {r}"
    );
    let landed = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('victim-result').textContent"}),
    );
    assert!(
        content_text(&landed).contains("NOT_CLICKED"),
        "occluded click never landed: {}",
        content_text(&landed)
    );
    finish(c);
}

#[test]
fn type_is_trusted_insert_and_replaces() {
    let mut c = launch_on_trusted();
    let ok = c.call("type", json!({"element": "#t-input", "text": "hello-42"}));
    assert!(!is_error(&ok), "type trusted ok: {ok}");
    let v = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('t-input').value"}),
    );
    // select-all preflight: the AAA-BEFORE seed is REPLACED, not appended.
    assert!(
        content_text(&v).contains("hello-42") && !content_text(&v).contains("AAA-BEFORE"),
        "value replaced via select-all, got: {}",
        content_text(&v)
    );
    let trusted = c.call(
        "evaluate",
        json!({"expression": "JSON.stringify(window.__typeTrusted)"}),
    );
    assert!(
        content_text(&trusted).contains("true"),
        "input event isTrusted:true, got: {}",
        content_text(&trusted)
    );
    finish(c);
}

#[test]
fn type_contenteditable_keeps_siblings() {
    let mut c = launch_on_trusted();
    let ok = c.call("type", json!({"element": "#ce", "text": "CE_NEW"}));
    assert!(!is_error(&ok), "contenteditable type ok: {ok}");
    let v = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('ce').textContent"}),
    );
    assert!(
        content_text(&v).contains("CE_NEW"),
        "contenteditable received text, got: {}",
        content_text(&v)
    );
    // Insert replaces WITHIN the editable element (select-all of its own
    // contents); unrelated nodes outside it are untouched.
    let sib = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('ce-sibling').textContent"}),
    );
    assert!(
        content_text(&sib).contains("SIBLING_KEEP"),
        "sibling node kept, got: {}",
        content_text(&sib)
    );
    let trusted = c.call(
        "evaluate",
        json!({"expression": "JSON.stringify(window.__ceTrusted)"}),
    );
    assert!(
        content_text(&trusted).contains("true"),
        "contenteditable input event isTrusted:true, got: {}",
        content_text(&trusted)
    );
    finish(c);
}

#[test]
fn type_rejects_file_and_date() {
    let mut c = launch_on_trusted();
    for el in ["#f-file", "#f-date"] {
        let r = c.call("type", json!({"element": el, "text": "x"}));
        assert!(is_error(&r), "type {el} isError: {r}");
        let sc = structured(&r);
        assert!(
            sc.get("code") == Some(&json!("not_actionable"))
                || sc.get("code") == Some(&json!("invalid_target")),
            "typed code for {el}: {r}"
        );
        assert_eq!(
            sc.get("reason"),
            Some(&json!("not_editable")),
            "not_editable reason for {el}: {r}"
        );
    }
    finish(c);
}
