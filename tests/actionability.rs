//! Actionability gates (G-1/G-2/G-3): `click`, `type` and `hover` must
//! refuse non-actionable elements as tool errors instead of reporting
//! `{"ok":true}`. Each test owns one server process and must leave no
//! browser behind. Positive controls prove visible elements still work,
//! and missing selectors keep the exact `element not found` behavior.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{json, Value};

fn fixture_url() -> String {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fixture.html");
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
        // Owned-engine resolution (see e2e_mcp.rs): the pinned Chromium
        // resolves from its versioned cache with no env override.
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

fn is_error(v: &Value) -> bool {
    v.get("isError").is_some()
}

/// EOF the server and require a clean exit (proves no hang, no orphan hold).
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

fn launch_on_fixture() -> Client {
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
fn click_refuses_hidden_and_disabled() {
    let mut c = launch_on_fixture();

    // G-1: display:none element errors and the click never lands.
    let hid = c.call("click", json!({"element": "#hidden-box"}));
    assert!(is_error(&hid), "click hidden isError: {hid}");
    let msg = content_text(&hid);
    assert!(
        msg.contains("#hidden-box"),
        "error names the element, got: {msg}"
    );
    assert!(
        msg.contains("not visible") || msg.contains("not clickable"),
        "visibility error, got: {msg}"
    );
    assert!(
        !msg.contains("not found"),
        "distinct from the not-found message, got: {msg}"
    );
    let landed = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('hidden-result').textContent"}),
    );
    assert!(
        content_text(&landed).contains("NOT_CLICKED"),
        "hidden click had no effect: {}",
        content_text(&landed)
    );

    // Click on a visible-but-disabled button errors (programmatic
    // el.click() would otherwise fire through the disabled state).
    let dis = c.call("click", json!({"element": "#disabled-btn"}));
    assert!(is_error(&dis), "click disabled isError: {dis}");
    assert!(
        content_text(&dis).contains("#disabled-btn"),
        "error names the element: {dis}"
    );
    let dlanded = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('disabled-result').textContent"}),
    );
    assert!(
        content_text(&dlanded).contains("NOT_CLICKED"),
        "disabled click had no effect: {}",
        content_text(&dlanded)
    );

    // GLADIUS_NOTFOUND behavior is untouched.
    let miss = c.call("click", json!({"element": "#no-such-element-xyz"}));
    assert!(is_error(&miss), "click missing isError: {miss}");
    assert!(
        content_text(&miss).contains("element not found"),
        "not-found message kept: {miss}"
    );

    // Positive control: a visible button still clicks.
    let ok = c.call("click", json!({"element": "#toggle"}));
    assert!(!is_error(&ok), "click visible ok: {ok}");
    let toggled = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('toggle').textContent"}),
    );
    assert!(
        content_text(&toggled).contains("TOGGLED_OK"),
        "visible click toggled: {}",
        content_text(&toggled)
    );

    finish(c);
}

#[test]
fn type_refuses_disabled_and_readonly() {
    let mut c = launch_on_fixture();

    // G-2: disabled input errors and the value is never written.
    let dis = c.call(
        "type",
        json!({"element": "#disabled-input", "text": "nope-probe"}),
    );
    assert!(is_error(&dis), "type disabled isError: {dis}");
    let msg = content_text(&dis);
    assert!(msg.contains("#disabled-input"), "error names it: {msg}");
    assert!(msg.contains("disabled"), "disabled reason: {msg}");
    assert!(!msg.contains("not found"), "distinct from not-found: {msg}");
    let val = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('disabled-input').value"}),
    );
    assert!(
        !content_text(&val).contains("nope-probe"),
        "disabled value untouched: {}",
        content_text(&val)
    );

    // readOnly input likewise errors and keeps its value.
    let ro = c.call(
        "type",
        json!({"element": "#readonly-input", "text": "ro-probe"}),
    );
    assert!(is_error(&ro), "type readonly isError: {ro}");
    assert!(
        content_text(&ro).contains("#readonly-input"),
        "error names it: {ro}"
    );
    let rval = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('readonly-input').value"}),
    );
    assert!(
        content_text(&rval).contains("orig") && !content_text(&rval).contains("ro-probe"),
        "readonly value kept: {}",
        content_text(&rval)
    );

    // GLADIUS_NOTFOUND behavior is untouched.
    let miss = c.call("type", json!({"element": "#no-such-input", "text": "x"}));
    assert!(is_error(&miss), "type missing isError: {miss}");
    assert!(
        content_text(&miss).contains("element not found"),
        "not-found message kept: {miss}"
    );

    // Positive control: a visible, enabled input still takes text.
    let ok = c.call("type", json!({"element": "#name", "text": "hello-42"}));
    assert!(!is_error(&ok), "type visible ok: {ok}");
    let v = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('name').value"}),
    );
    assert!(
        content_text(&v).contains("hello-42"),
        "typed value present: {}",
        content_text(&v)
    );

    finish(c);
}

#[test]
fn hover_refuses_hidden() {
    let mut c = launch_on_fixture();

    // G-3: display:none element errors instead of driving the pointer to (0,0).
    let hid = c.call("hover", json!({"element": "#hidden-box"}));
    assert!(is_error(&hid), "hover hidden isError: {hid}");
    let msg = content_text(&hid);
    assert!(msg.contains("#hidden-box"), "error names it: {msg}");
    assert!(
        msg.contains("not visible") || msg.contains("not hoverable"),
        "visibility error, got: {msg}"
    );
    assert!(!msg.contains("not found"), "distinct from not-found: {msg}");

    // GLADIUS_NOTFOUND behavior is untouched.
    let miss = c.call("hover", json!({"element": "#no-such-element-xyz"}));
    assert!(is_error(&miss), "hover missing isError: {miss}");
    assert!(
        content_text(&miss).contains("element not found"),
        "not-found message kept: {miss}"
    );

    // Positive control: a visible element still hovers (real mouse move).
    let ok = c.call("hover", json!({"element": "#hover-target"}));
    assert!(!is_error(&ok), "hover visible ok: {ok}");
    let got = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('hover-result').textContent"}),
    );
    assert!(
        content_text(&got).contains("HOVERED_OK"),
        "mouseover fired: {}",
        content_text(&got)
    );

    finish(c);
}
