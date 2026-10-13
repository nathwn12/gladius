//! `viewport` over the REAL CDP path: launch the engine, navigate, set the
//! override, and assert the READ-BACK (`window.innerWidth`/`innerHeight`)
//! rather than the acknowledged request. Headless only; the test owns one
//! server process and leaves no browser behind.
//!
//! The engine default is measured here (baseline capture, then the same
//! number required after `reset`) - it settles the "800x600-ish vs
//! 1280x720" question with a measurement instead of a doc claim.

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
        // Default-engine location pin (see e2e_mcp.rs): the cargo test tree
        // has no obscura sibling, so point at the repo-pinned bin/obscura.exe.
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

    /// A viewport call that must succeed; returns the parsed envelope.
    fn viewport(&mut self, args: Value) -> Value {
        let r = self.call("viewport", args);
        assert!(r.get("isError").is_none(), "viewport ok: {r}");
        serde_json::from_str(&content_text(&r)).expect("viewport json")
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

/// `(innerWidth, innerHeight)` read straight from the page, independent of
/// the tool's own read-back (so a lying tool cannot pass its own test).
fn size_of(c: &mut Client) -> (i64, i64) {
    let v = c.call(
        "evaluate",
        json!({"expression": "[window.innerWidth, window.innerHeight]"}),
    );
    let text = content_text(&v);
    let obj: Value = serde_json::from_str(&text).expect("evaluate json");
    let arr = obj.get("value").and_then(|x| x.as_array()).expect("array");
    (
        arr[0].as_f64().expect("innerWidth") as i64,
        arr[1].as_f64().expect("innerHeight") as i64,
    )
}

fn readback(env: &Value) -> (i64, i64) {
    let rb = env.get("readback").expect("readback");
    (
        rb.get("innerWidth").and_then(|v| v.as_i64()).expect("w"),
        rb.get("innerHeight").and_then(|v| v.as_i64()).expect("h"),
    )
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
fn viewport_override_readback_reset_and_new_tab() {
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));

    // The tool is advertised with its parameter schema.
    let listed = c.result("tools/list", json!({}));
    let tools = listed
        .get("tools")
        .and_then(|t| t.as_array())
        .expect("tools");
    let vp = tools
        .iter()
        .find(|t| t.get("name").and_then(|n| n.as_str()) == Some("viewport"))
        .expect("viewport listed");
    let required = vp
        .get("inputSchema")
        .and_then(|s| s.get("required"))
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(required, json!(["width", "height"]), "required args: {vp}");

    let nav = c.call("navigate", json!({"url": fixture_url()}));
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    // Baseline = the engine's own default, measured before any override.
    let (dw, dh) = size_of(&mut c);
    eprintln!("ENGINE-DEFAULT-VIEWPORT: {dw}x{dh}");

    // 1440x900.
    let env = c.viewport(json!({"width": 1440, "height": 900}));
    assert_eq!(readback(&env), (1440, 900), "1440x900 read-back: {env}");
    assert_eq!(env.get("matched"), Some(&json!(true)), "matched: {env}");
    assert_eq!(size_of(&mut c), (1440, 900), "page agrees: 1440x900");

    // 390x844 (phone-sized), with the optional flags named.
    let env =
        c.viewport(json!({"width": 390, "height": 844, "mobile": true, "deviceScaleFactor": 3}));
    assert_eq!(readback(&env), (390, 844), "390x844 read-back: {env}");
    assert_eq!(
        env.get("readback")
            .and_then(|r| r.get("devicePixelRatio"))
            .and_then(|v| v.as_f64()),
        Some(3.0),
        "deviceScaleFactor honoured (a parameter the engine ignores would be a silent lie): {env}"
    );
    assert_eq!(size_of(&mut c), (390, 844), "page agrees: 390x844");

    // A NAVIGATE keeps the override (it lives on the CDP session).
    let nav = c.call("navigate", json!({"url": fixture_url()}));
    assert!(nav.get("isError").is_none(), "re-navigate ok: {nav}");
    assert_eq!(size_of(&mut c), (390, 844), "override survived a navigate");

    // A NEW TAB inherits the override (the session-scoped replay).
    let t = c.call("tabs", json!({"action": "new", "url": "about:blank"}));
    assert!(t.get("isError").is_none(), "tabs new ok: {t}");
    assert_eq!(
        size_of(&mut c),
        (390, 844),
        "new tab inherited the override"
    );

    // reset -> the engine default, exactly the baseline measured above.
    let env = c.viewport(json!({"reset": true}));
    assert_eq!(readback(&env), (dw, dh), "reset read-back: {env}");
    assert_eq!(size_of(&mut c), (dw, dh), "page agrees: default back");

    // A new tab opened AFTER reset gets the default too.
    let t = c.call("tabs", json!({"action": "new", "url": "about:blank"}));
    assert!(t.get("isError").is_none(), "tabs new ok: {t}");
    assert_eq!(
        size_of(&mut c),
        (dw, dh),
        "post-reset new tab is at the engine default"
    );

    // Refusals stay typed tool errors, never a silent no-op.
    let zero = c.call("viewport", json!({"width": 0, "height": 100}));
    assert!(zero.get("isError").is_some(), "zero width refused: {zero}");
    assert!(
        content_text(&zero).contains("must be >= 1"),
        "zero-width prose: {}",
        content_text(&zero)
    );
    let missing = c.call("viewport", json!({}));
    assert!(
        missing.get("isError").is_some(),
        "missing args refused: {missing}"
    );

    let closed = c.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
    shutdown(c);
}
