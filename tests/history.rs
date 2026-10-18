//! Phase 4: navigation history. `history` steps back/forward through the
//! active page's entries and reloads the current one, returning the live
//! url+title after the load settle; a step past the boundary is a typed
//! error, never a silent no-op. Each test owns one server and reaps its
//! engine.

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
            let name = if name.is_empty() { "h1.html" } else { name };
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
        Client { stdin, rx, next_id: 1, child }
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
        assert!(resp.get("error").is_none(), "{method} protocol error: {resp}");
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

fn content_obj(v: &Value) -> Value {
    let t = content_text(v);
    serde_json::from_str(&t).unwrap_or_else(|_| panic!("tool json: {t}"))
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

fn hist_url(c: &mut Client, action: &str) -> Value {
    c.call("history", json!({"action": action}))
}

#[test]
fn history_walks_back_forward_and_reloads() {
    let port = serve();
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    for page in ["h1.html", "h2.html", "h3.html"] {
        let nav = c.call("navigate", json!({"url": format!("http://127.0.0.1:{port}/{page}")}));
        assert!(nav.get("isError").is_none(), "navigate {page} ok: {nav}");
    }

    // Back twice: h3 -> h2 -> h1, reading location.href after each step.
    let b1 = hist_url(&mut c, "back");
    assert!(b1.get("isError").is_none(), "back ok: {b1}");
    let o1 = content_obj(&b1);
    assert!(
        o1.get("url").and_then(|u| u.as_str()).unwrap_or("").ends_with("/h2.html"),
        "back lands on h2: {o1}"
    );
    let b2 = hist_url(&mut c, "back");
    assert!(b2.get("isError").is_none(), "back ok: {b2}");
    assert!(
        content_obj(&b2).get("url").and_then(|u| u.as_str()).unwrap_or("").ends_with("/h1.html"),
        "back lands on h1: {b2}"
    );

    // One more back reaches the session's initial about:blank entry, and
    // past THAT is the boundary: typed error, never a silent no-op.
    let b3 = hist_url(&mut c, "back");
    assert!(b3.get("isError").is_none(), "back to blank ok: {b3}");
    assert!(
        content_obj(&b3).get("url").and_then(|u| u.as_str()).unwrap_or("") == "about:blank",
        "back lands on about:blank: {b3}"
    );
    let bend = hist_url(&mut c, "back");
    assert!(bend.get("isError").is_some(), "boundary back isError: {bend}");
    assert_eq!(
        structured(&bend).get("reason").and_then(|v| v.as_str()),
        Some("not_found"),
        "boundary reason: {bend}"
    );

    // Forward three times: blank -> h1 -> h2 -> h3.
    for (want, n) in [("/h1.html", 1), ("/h2.html", 2), ("/h3.html", 3)] {
        let f = hist_url(&mut c, "forward");
        assert!(f.get("isError").is_none(), "forward {n} ok: {f}");
        assert!(
            content_obj(&f).get("url").and_then(|u| u.as_str()).unwrap_or("").ends_with(want),
            "forward lands on {want}: {f}"
        );
    }
    let fend = hist_url(&mut c, "forward");
    assert!(fend.get("isError").is_some(), "boundary forward isError: {fend}");

    // Reload replays the current entry: still h3, marker intact.
    let rel = hist_url(&mut c, "reload");
    assert!(rel.get("isError").is_none(), "reload ok: {rel}");
    assert!(
        content_obj(&rel).get("url").and_then(|u| u.as_str()).unwrap_or("").ends_with("/h3.html"),
        "reload stays on h3: {rel}"
    );
    let ev = c.call("evaluate", json!({"expression": "document.querySelector('h1').textContent"}));
    assert!(
        content_text(&ev).contains("HIST_THREE_MARKER_ccc"),
        "reloaded page intact: {ev}"
    );

    // Unknown action and missing action are typed.
    let bad = hist_url(&mut c, "bogus");
    assert!(bad.get("isError").is_some(), "unknown action: {bad}");
    assert_eq!(
        structured(&bad).get("reason").and_then(|v| v.as_str()),
        Some("unknown_action"),
        "reason: {bad}"
    );
    let missing = c.call("history", json!({}));
    assert!(missing.get("isError").is_some(), "missing action: {missing}");

    finish(c);
}
