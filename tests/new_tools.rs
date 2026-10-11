//! New-tool coverage: `hover`, `select`, `key`, `tabs` over the real CDP
//! path against the shared fixture page. Headless only; each test owns one
//! server process and must leave no browser behind.

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
        // Default-engine location pin (see e2e_mcp.rs): cargo tree has no
        // obscura sibling, so point at the repo-pinned bin/obscura.exe.
        cmd.env(
            "GLADIUS_OBSCURA",
            format!("{}/bin/obscura.exe", env!("CARGO_MANIFEST_DIR")),
        );
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

fn tabs_obj(v: &Value) -> Value {
    let t = content_text(v);
    serde_json::from_str(&t).unwrap_or_else(|_| panic!("tabs json: {t}"))
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

#[test]
fn hover_dispatches_mouse_move() {
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let nav = c.call("navigate", json!({"url": fixture_url()}));
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    let hov = c.call("hover", json!({"element": "#hover-target"}));
    assert!(hov.get("isError").is_none(), "hover ok: {hov}");
    let hov_text = content_text(&hov);
    let hov_obj: Value = serde_json::from_str(&hov_text).expect("hover json");
    assert_eq!(hov_obj.get("ok"), Some(&json!(true)), "hover ok:true");

    let got = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('hover-result').textContent"}),
    );
    let got_text = content_text(&got);
    assert!(
        got_text.contains("HOVERED_OK"),
        "mouseover fired via real mouse move: {got_text}"
    );

    // Missing element is a tool error, not a protocol error.
    let miss = c.call("hover", json!({"element": "#no-such-element-xyz"}));
    assert!(
        miss.get("isError").is_some(),
        "hover missing isError: {miss}"
    );

    let closed = c.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
    shutdown(c);
}

#[test]
fn select_by_value_and_text() {
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let nav = c.call("navigate", json!({"url": fixture_url()}));
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    // By value (preferred).
    let s1 = c.call("select", json!({"element": "#color", "value": "g"}));
    assert!(s1.get("isError").is_none(), "select value ok: {s1}");
    let v1 = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('color').value"}),
    );
    assert!(
        content_text(&v1).contains('g'),
        "value landed: {}",
        content_text(&v1)
    );
    let r1 = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('select-result').textContent"}),
    );
    assert!(
        content_text(&r1).contains("SELECTED:g"),
        "change event fired: {}",
        content_text(&r1)
    );

    // By visible text.
    let s2 = c.call("select", json!({"element": "#color", "text": "Blue"}));
    assert!(s2.get("isError").is_none(), "select text ok: {s2}");
    let v2 = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('color').value"}),
    );
    assert!(
        content_text(&v2).contains('b'),
        "text->value landed: {}",
        content_text(&v2)
    );

    // Missing option / element / non-select / no selector args.
    let no_opt = c.call("select", json!({"element": "#color", "value": "zzz"}));
    assert!(
        no_opt.get("isError").is_some(),
        "missing option isError: {no_opt}"
    );
    let no_el = c.call("select", json!({"element": "#nope", "value": "g"}));
    assert!(
        no_el.get("isError").is_some(),
        "missing el isError: {no_el}"
    );
    let not_sel = c.call("select", json!({"element": "#toggle", "value": "g"}));
    assert!(
        not_sel.get("isError").is_some(),
        "non-select isError: {not_sel}"
    );
    let no_args = c.call("select", json!({"element": "#color"}));
    assert!(
        no_args.get("isError").is_some(),
        "need value-or-text isError: {no_args}"
    );

    let closed = c.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
    shutdown(c);
}

#[test]
fn key_presses_real_keys() {
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let nav = c.call("navigate", json!({"url": fixture_url()}));
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    // Backspace deletes the last char of the focused input.
    let typed = c.call("type", json!({"element": "#name", "text": "ab"}));
    assert!(typed.get("isError").is_none(), "type ok: {typed}");
    let back = c.call("key", json!({"key": "Backspace"}));
    assert!(back.get("isError").is_none(), "key Backspace ok: {back}");
    let val = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('name').value"}),
    );
    let val_text = content_text(&val);
    assert!(
        val_text.contains('a') && !val_text.contains('b'),
        "Backspace deleted last char, got: {val_text}"
    );

    // Enter on the focused input submits the form (same default action
    // the `type` submit path relies on).
    let typed2 = c.call("type", json!({"element": "#name", "text": "hello-key"}));
    assert!(typed2.get("isError").is_none(), "type ok: {typed2}");
    let enter = c.call("key", json!({"key": "Enter"}));
    assert!(enter.get("isError").is_none(), "key Enter ok: {enter}");
    let got = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('submit-result').textContent"}),
    );
    assert!(
        content_text(&got).contains("SUBMITTED:hello-key"),
        "Enter submitted form: {}",
        content_text(&got)
    );

    // Escape is accepted (no-op on this page, but proves the mapping).
    let esc = c.call("key", json!({"key": "Escape"}));
    assert!(esc.get("isError").is_none(), "key Escape ok: {esc}");

    let closed = c.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
    shutdown(c);
}

#[test]
fn tabs_list_new_select_close() {
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let nav = c.call("navigate", json!({"url": fixture_url()}));
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    let listed = c.call("tabs", json!({"action": "list"}));
    assert!(listed.get("isError").is_none(), "tabs list ok: {listed}");
    let l0 = tabs_obj(&listed);
    assert_eq!(l0.get("ok"), Some(&json!(true)), "tabs ok:true");
    let arr0 = l0
        .get("tabs")
        .and_then(|t| t.as_array())
        .expect("tabs array");
    assert_eq!(arr0.len(), 1, "one tab at start: {l0}");
    assert_eq!(
        arr0[0].get("active"),
        Some(&json!(true)),
        "sole tab active: {l0}"
    );
    assert_eq!(arr0[0].get("index"), Some(&json!(0)), "index 0: {l0}");

    // Open a second tab on the same fixture page.
    let url = fixture_url();
    let opened = c.call("tabs", json!({"action": "new", "url": url}));
    assert!(opened.get("isError").is_none(), "tabs new ok: {opened}");
    let l1 = tabs_obj(&opened);
    let arr1 = l1
        .get("tabs")
        .and_then(|t| t.as_array())
        .expect("tabs array");
    assert_eq!(arr1.len(), 2, "two tabs after new: {l1}");
    let active1: Vec<&Value> = arr1
        .iter()
        .filter(|t| t.get("active") == Some(&json!(true)))
        .collect();
    assert_eq!(active1.len(), 1, "exactly one active: {l1}");
    assert_eq!(
        active1[0].get("index"),
        Some(&json!(1)),
        "new tab becomes active: {l1}"
    );

    // The new (active) tab loaded the fixture: wait + snapshot prove the
    // other tools follow the tracked tab.
    let w = c.call("wait_for", json!({"selector": "#toggle", "ms": 10000}));
    assert!(w.get("isError").is_none(), "new tab loaded: {w}");
    let snap = c.call("snapshot", json!({}));
    assert!(snap.get("isError").is_none(), "snapshot on new tab: {snap}");
    assert!(
        content_text(&snap).contains("GLADIUS_MARKER_hello-12345"),
        "marker on new tab"
    );

    // Back to the first tab: still the fixture, still tracked.
    let sel = c.call("tabs", json!({"action": "select", "index": 0}));
    assert!(sel.get("isError").is_none(), "tabs select ok: {sel}");
    let l2 = tabs_obj(&sel);
    let arr2 = l2
        .get("tabs")
        .and_then(|t| t.as_array())
        .expect("tabs array");
    assert_eq!(
        arr2.iter()
            .filter(|t| t.get("active") == Some(&json!(true)))
            .count(),
        1,
        "exactly one active after select: {l2}"
    );
    assert_eq!(
        arr2[0].get("active"),
        Some(&json!(true)),
        "tab 0 active: {l2}"
    );
    let snap0 = c.call("snapshot", json!({}));
    assert!(
        content_text(&snap0).contains("GLADIUS_MARKER_hello-12345"),
        "marker on tab 0 after select"
    );

    // Bad index / action are tool errors.
    let bad_idx = c.call("tabs", json!({"action": "select", "index": 99}));
    assert!(
        bad_idx.get("isError").is_some(),
        "bad select index isError: {bad_idx}"
    );
    let bad_act = c.call("tabs", json!({"action": "bogus"}));
    assert!(
        bad_act.get("isError").is_some(),
        "bad action isError: {bad_act}"
    );

    // Close the second tab; one tracked tab remains.
    let closed_tab = c.call("tabs", json!({"action": "close", "index": 1}));
    assert!(
        closed_tab.get("isError").is_none(),
        "tabs close ok: {closed_tab}"
    );
    let l3 = tabs_obj(&closed_tab);
    let arr3 = l3
        .get("tabs")
        .and_then(|t| t.as_array())
        .expect("tabs array");
    assert_eq!(arr3.len(), 1, "one tab after close: {l3}");
    assert_eq!(
        arr3[0].get("active"),
        Some(&json!(true)),
        "survivor active: {l3}"
    );

    let closed = c.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
    shutdown(c);
}
