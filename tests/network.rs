//! Phase 5: network capture. The `network` tool returns captured
//! requests/responses (method, url, status, resource type, timing) with an
//! optional filter - the observable counterpart to the `wait_for networkIdle`
//! quiet-window sense. Proved against a fixture that issues fetch + XHR plus
//! a subresource load. Each test owns one server and reaps its engine.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{json, Value};

// 1x1 transparent PNG.
const PIXEL_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x60, 0x18, 0x05, 0xa3,
    0x60, 0x14, 0x8c, 0x02, 0x00, 0x00, 0x09, 0x00, 0x03, 0x9e, 0x0a, 0x7e, 0x34, 0x00, 0x00, 0x00,
    0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

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
            let (ctype, body): (&str, Vec<u8>) = match path.as_str() {
                "/api/data" => ("application/json", b"{\"hello\":\"world\"}".to_vec()),
                "/api/xhr" => ("text/plain", b"XHR_PAYLOAD_777".to_vec()),
                "/pixel.png" => ("image/png", PIXEL_PNG.to_vec()),
                p => {
                    let name = p.trim_start_matches('/');
                    let name = if name.is_empty() { "net.html" } else { name };
                    let mut fp = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
                    fp.push("tests");
                    fp.push("fixtures");
                    fp.push(name);
                    ("text/html", std::fs::read(&fp).unwrap_or_default())
                }
            };
            let _ = s.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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

fn entries_of(v: &Value) -> Vec<Value> {
    content_obj(v)
        .get("entries")
        .and_then(|e| e.as_array())
        .cloned()
        .unwrap_or_default()
}

#[test]
fn network_captures_fetch_xhr_and_subresource() {
    let port = serve();
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let nav = c.call(
        "navigate",
        json!({"url": format!("http://127.0.0.1:{port}/net.html")}),
    );
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    // Start clean (and enable the Network domain), then reload so the
    // document + subresource load inside the capture window.
    let clr = c.call("network", json!({"action": "clear"}));
    assert!(clr.get("isError").is_none(), "clear ok: {clr}");
    let nav2 = c.call(
        "navigate",
        json!({"url": format!("http://127.0.0.1:{port}/net.html")}),
    );
    assert!(nav2.get("isError").is_none(), "renavigate ok: {nav2}");

    // Fire fetch + XHR from the page.
    let fire = c.call("click", json!({"element": "#fire"}));
    assert!(fire.get("isError").is_none(), "fire ok: {fire}");
    let w = c.call(
        "wait_for",
        json!({"expression": "document.getElementById('r').textContent.startsWith('FETCH_GOT') && document.getElementById('x').textContent.startsWith('XHR_GOT')", "ms": 10000}),
    );
    assert!(w.get("isError").is_none(), "fetch+xhr landed: {w}");

    // The ring holds the subresource, the fetch, and the XHR - each with
    // method, url, 200 status, resource type, and timing.
    let mut got = Vec::new();
    for _ in 0..50 {
        let g = c.call("network", json!({}));
        got = entries_of(&g);
        let urls: Vec<String> = got
            .iter()
            .filter_map(|e| e.get("url").and_then(|u| u.as_str()).map(|s| s.to_string()))
            .collect();
        if urls.iter().any(|u| u.contains("/api/data"))
            && urls.iter().any(|u| u.contains("/api/xhr"))
            && urls.iter().any(|u| u.contains("/pixel.png"))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let find = |sub: &str| {
        got.iter()
            .find(|e| {
                e.get("url")
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .contains(sub)
            })
            .unwrap_or_else(|| panic!("entry for {sub} captured: {got:?}"))
            .clone()
    };
    let fetch = find("/api/data?x=1");
    assert_eq!(
        fetch.get("method").and_then(|m| m.as_str()),
        Some("GET"),
        "fetch method: {fetch}"
    );
    assert_eq!(
        fetch.get("status").and_then(|s| s.as_i64()),
        Some(200),
        "fetch status: {fetch}"
    );
    assert!(
        !fetch
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .is_empty(),
        "fetch type: {fetch}"
    );
    assert!(
        fetch.get("ts").and_then(|t| t.as_u64()).unwrap_or(0) > 0,
        "fetch ts: {fetch}"
    );
    let xhr = find("/api/xhr?y=2");
    assert_eq!(
        xhr.get("status").and_then(|s| s.as_i64()),
        Some(200),
        "xhr status: {xhr}"
    );
    let img = find("/pixel.png");
    assert_eq!(
        img.get("status").and_then(|s| s.as_i64()),
        Some(200),
        "img status: {img}"
    );

    // Filter narrows to the matching url substring.
    let f = c.call("network", json!({"action": "get", "filter": "/api/data"}));
    assert!(f.get("isError").is_none(), "filter ok: {f}");
    let fgot = entries_of(&f);
    assert!(!fgot.is_empty(), "filter matches: {fgot:?}");
    assert!(
        fgot.iter().all(|e| e
            .get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .contains("/api/data")),
        "filter exact: {fgot:?}"
    );

    // Drain returns and removes; clear drops the rest.
    let d = c.call("network", json!({"action": "drain"}));
    assert!(d.get("isError").is_none(), "drain ok: {d}");
    let dobj = content_obj(&d);
    assert!(
        dobj.get("cleared").and_then(|v| v.as_i64()).unwrap_or(0) > 0,
        "drained some: {dobj}"
    );
    let g2 = c.call("network", json!({}));
    assert_eq!(
        content_obj(&g2).get("total").and_then(|v| v.as_i64()),
        Some(0),
        "ring empty after drain: {g2}"
    );

    // Unknown action is typed; limit is honored.
    let bad = c.call("network", json!({"action": "bogus"}));
    assert!(bad.get("isError").is_some(), "unknown action: {bad}");
    assert_eq!(
        structured(&bad).get("reason").and_then(|v| v.as_str()),
        Some("unknown_action"),
        "reason: {bad}"
    );

    finish(c);
}
