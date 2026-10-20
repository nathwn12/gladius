//! Phase 2: predicate waits. `wait_for` gains an `expression` form that
//! polls a computed JS condition on the active page inside the shared
//! timeout budget, returning the typed timeout on expiry. Proved with a
//! fixture that flips a value after a delay, plus a never-flipping value
//! for the timeout payload. Each test owns one server and reaps its engine.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{json, Value};

fn serve() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind localhost");
    let port = listener.local_addr().expect("local addr").port();
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let mut s = s;
            let mut buf = [0u8; 8192];
            let n = s.read(&mut buf).unwrap_or(0);
            let req: String = String::from_utf8_lossy(&buf[..n]).into_owned();
            let path = req
                .lines()
                .next()
                .unwrap_or("")
                .split_whitespace()
                .nth(1)
                .unwrap_or("/")
                .split('?')
                .next()
                .unwrap_or("/")
                .to_string();
            let name = path.trim_start_matches('/');
            let name = if name.is_empty() { "flip.html" } else { name };
            let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            p.push("tests");
            p.push("fixtures");
            p.push(name);
            let body = std::fs::read(&p).unwrap_or_default();
            let _ = s.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            );
            let _ = s.write_all(&body);
        }
    });
    port
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
            cmd.creation_flags(0x08000000);
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

    fn result(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let frame = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{frame}").expect("write stdin");
        self.stdin.flush().expect("flush stdin");
        let line = self
            .rx
            .recv_timeout(Duration::from_secs(120))
            .unwrap_or_else(|_| panic!("timeout waiting for reply to {method}"));
        let resp: Value =
            serde_json::from_str(&line).unwrap_or_else(|_| panic!("bad json for {method}: {line}"));
        assert!(resp.get("id") == Some(&json!(id)), "id echo for {method}");
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

fn finish(mut c: Client) {
    let closed = c.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
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

#[test]
fn wait_for_expression_resolves_when_value_flips() {
    let port = serve();
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let nav = c.call(
        "navigate",
        json!({"url": format!("http://127.0.0.1:{port}/flip.html")}),
    );
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    // The fixture flips `window.__flip` to 'FLIPPED_OK' after ~1.5 s.
    let w = c.call(
        "wait_for",
        json!({"expression": "window.__flip === 'FLIPPED_OK'", "ms": 10000}),
    );
    assert!(w.get("isError").is_none(), "expression wait ok: {w}");
    let t = content_text(&w);
    assert!(t.contains("\"ok\":true"), "ok payload: {t}");

    // Truthiness follows JS rules: a non-empty string is truthy.
    let w2 = c.call(
        "wait_for",
        json!({"expression": "window.__flip", "ms": 2000}),
    );
    assert!(w2.get("isError").is_none(), "truthy string ok: {w2}");

    finish(c);
}

#[test]
fn wait_for_expression_times_out_typed_when_never_true() {
    let port = serve();
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let nav = c.call(
        "navigate",
        json!({"url": format!("http://127.0.0.1:{port}/flip.html")}),
    );
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    let t0 = std::time::Instant::now();
    let w = c.call(
        "wait_for",
        json!({"expression": "window.__stuck === 'FLIPPED_OK'", "ms": 1200}),
    );
    let dt = t0.elapsed();
    assert!(w.get("isError").is_some(), "never-true isError: {w}");
    let t = content_text(&w);
    assert!(t.contains("timeout"), "timeout prose: {w}");
    let sc = structured(&w);
    assert_eq!(
        sc.get("code").and_then(|v| v.as_str()),
        Some("timeout"),
        "code: {w}"
    );
    assert_eq!(
        sc.get("reason").and_then(|v| v.as_str()),
        Some("budget_exceeded"),
        "reason: {w}"
    );
    assert_eq!(
        sc.get("tool").and_then(|v| v.as_str()),
        Some("wait_for"),
        "tool: {w}"
    );
    assert!(
        dt >= Duration::from_millis(1000) && dt < Duration::from_secs(10),
        "budget honored, took {dt:?}"
    );

    // A throwing condition is a real error, not "not yet true".
    let bad = c.call(
        "wait_for",
        json!({"expression": "null.foo.bar", "ms": 2000}),
    );
    assert!(
        bad.get("isError").is_some(),
        "throwing condition isError: {bad}"
    );

    // expression + selector is one condition per call, never both.
    let both = c.call(
        "wait_for",
        json!({"expression": "true", "selector": "#x", "ms": 1000}),
    );
    assert!(
        both.get("isError").is_some(),
        "expression+selector rejected: {both}"
    );
    assert_eq!(
        structured(&both).get("reason").and_then(|v| v.as_str()),
        Some("bad_value"),
        "reason: {both}"
    );

    finish(c);
}
