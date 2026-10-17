//! End-to-end test: spawns the release binary in `mcp` mode over real
//! stdio and drives the frozen tool contract against a local fixture page.
//! Headless only; the `close` tool (plus server EOF) must leave no browser
//! process behind.

use std::io::{BufRead, BufReader, Write};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
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
            .stderr(Stdio::inherit())
            .creation_flags(0x08000000); // CREATE_NO_WINDOW
                                         // Owned-engine resolution: the server resolves the pinned
                                         // Chromium from its versioned cache
                                         // (`%LOCALAPPDATA%\gladius\chromium\`, honouring
                                         // `GLADIUS_CHROMIUM_CACHE`) with no env override, so
                                         // integration tests set nothing here and exercise the
                                         // default path end to end.
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
        writeln!(self.stdin, "{}", frame).expect("write stdin");
        self.stdin.flush().expect("flush stdin");
        let resp = self.recv(method);
        assert_eq!(resp.get("id"), Some(&json!(id)), "id echo for {method}");
        resp
    }

    fn notify(&mut self, method: &str) {
        let frame = json!({"jsonrpc": "2.0", "method": method});
        writeln!(self.stdin, "{}", frame).expect("write stdin");
        self.stdin.flush().expect("flush stdin");
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

/// One-shot localhost HTTP server for the `fetch_text` test (std only).
/// Serves `body` once, then exits. Returns the bound port.
fn serve_once(body: &'static str) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind localhost");
    let port = listener.local_addr().expect("local addr").port();
    std::thread::spawn(move || {
        if let Ok((mut s, _)) = listener.accept() {
            use std::io::Read;
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf); // consume the request head
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            use std::io::Write;
            let _ = s.write_all(resp.as_bytes());
        }
    });
    port
}

#[test]
fn e2e_mcp_fixture_flow() {
    let mut c = Client::spawn();

    // initialize (echo protocolVersion) + tools/list
    let init = c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    assert_eq!(
        init.get("protocolVersion"),
        Some(&json!("2025-06-18")),
        "protocolVersion echo"
    );
    assert_eq!(
        init.get("serverInfo").and_then(|s| s.get("name")),
        Some(&json!("gladius")),
        "serverInfo"
    );
    c.notify("notifications/initialized");

    let listed = c.result("tools/list", json!({}));
    let names: Vec<String> = listed
        .get("tools")
        .and_then(|t| t.as_array())
        .expect("tools array")
        .iter()
        .filter_map(|t| {
            t.get("name")
                .and_then(|n| n.as_str())
                .map(|s| s.to_string())
        })
        .collect();
    for need in [
        "navigate",
        "snapshot",
        "screenshot",
        "click",
        "type",
        "evaluate",
        "wait_for",
        "fetch_text",
        "close",
    ] {
        assert!(
            names.contains(&need.to_string()),
            "tool {need} listed: {names:?}"
        );
    }
    assert!(names.len() >= 8, ">=8 tools");

    // ping
    let pong = c.result("ping", json!({}));
    assert_eq!(pong, json!({}), "ping replies {{}}");

    // navigate to fixture
    let url = fixture_url();
    let nav = c.call("navigate", json!({"url": url}));
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");
    let nav_text = content_text(&nav);
    let nav_obj: Value = serde_json::from_str(&nav_text).expect("navigate json");
    assert!(nav_obj.get("url").is_some(), "navigate has url: {nav_text}");
    assert!(
        nav_obj.get("title").is_some(),
        "navigate has title: {nav_text}"
    );

    // snapshot contains marker
    let snap = c.call("snapshot", json!({}));
    assert!(snap.get("isError").is_none(), "snapshot ok: {snap}");
    let snap_text = content_text(&snap);
    assert!(
        snap_text.contains("GLADIUS_MARKER_hello-12345"),
        "snapshot has marker: {}",
        snap_text.chars().take(200).collect::<String>()
    );

    // evaluate returns known value
    let ev = c.call("evaluate", json!({"expression": "window.__gladiusValue"}));
    assert!(ev.get("isError").is_none(), "evaluate ok: {ev}");
    let ev_text = content_text(&ev);
    let ev_obj: Value = serde_json::from_str(&ev_text).expect("evaluate json");
    // Numeric compare (as_f64): engines differ in number shape (some return
    // `42`, others `42.0`) while the VALUE is identical.
    assert_eq!(
        ev_obj.get("value").and_then(|v| v.as_f64()),
        Some(42.0),
        "known value: {ev_text}"
    );

    // click toggles the element
    let clk = c.call("click", json!({"element": "#toggle"}));
    assert!(clk.get("isError").is_none(), "click ok: {clk}");
    let toggled = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('toggle').textContent"}),
    );
    let toggled_text = content_text(&toggled);
    assert!(
        toggled_text.contains("TOGGLED_OK"),
        "click toggled: {toggled_text}"
    );

    // screenshot returns a non-empty PNG image block
    let shot = c.call("screenshot", json!({}));
    assert!(shot.get("isError").is_none(), "screenshot ok: {shot}");
    let img = shot
        .get("content")
        .and_then(|cc| cc.get(0))
        .expect("image block");
    assert_eq!(
        img.get("type").and_then(|t| t.as_str()),
        Some("image"),
        "block type image: {shot}"
    );
    assert_eq!(
        img.get("mimeType").and_then(|m| m.as_str()),
        Some("image/png"),
        "mime png: {shot}"
    );
    let data = img.get("data").and_then(|d| d.as_str()).unwrap_or("");
    assert!(
        data.len() > 1000,
        "non-empty png base64: len={}",
        data.len()
    );
    assert!(
        data.starts_with("iVBORw0KGgo"),
        "png magic (base64): {}",
        data.chars().take(24).collect::<String>()
    );

    // type into the input, value lands via JS property path
    let typed = c.call("type", json!({"element": "#name", "text": "hello-42"}));
    assert!(typed.get("isError").is_none(), "type ok: {typed}");
    let val = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('name').value"}),
    );
    let val_text = content_text(&val);
    assert!(
        val_text.contains("hello-42"),
        "typed value present: {val_text}"
    );

    // type with submit=true submits the form (Enter key -> onsubmit)
    let sub = c.call(
        "type",
        json!({"element": "#name", "text": "world", "submit": true}),
    );
    assert!(sub.get("isError").is_none(), "type+submit ok: {sub}");
    let got = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('submit-result').textContent"}),
    );
    let got_text = content_text(&got);
    assert!(
        got_text.contains("SUBMITTED:world"),
        "form submitted: {got_text}"
    );

    // wait_for: existing selector and existing text resolve ok
    let wsel = c.call("wait_for", json!( {"selector": "#toggle"}));
    assert!(
        wsel.get("isError").is_none(),
        "wait_for selector ok: {wsel}"
    );
    let wtxt = c.call("wait_for", json!({"text": "GLADIUS_MARKER_hello-12345"}));
    assert!(wtxt.get("isError").is_none(), "wait_for text ok: {wtxt}");
    // wait_for: missing selector times out as a tool error (not a protocol error)
    let wmiss = c.call(
        "wait_for",
        json!({"selector": "#no-such-element-xyz", "ms": 500}),
    );
    assert!(
        wmiss.get("isError").is_some(),
        "wait_for missing isError: {wmiss}"
    );
    assert!(
        content_text(&wmiss).contains("timeout"),
        "wait_for timeout message: {wmiss}"
    );

    // fetch_text against a one-shot localhost server (plain HTTP, no browser)
    let port = serve_once(
        "<!doctype html><html><head><title>t</title></head><body><h1>FETCH_HELLO marker-999</h1><script>var hidden_xyz=1;</script></body></html>",
    );
    let fetched = c.call(
        "fetch_text",
        json!({"url": format!("http://127.0.0.1:{port}/")}),
    );
    assert!(fetched.get("isError").is_none(), "fetch_text ok: {fetched}");
    let fetched_text = content_text(&fetched);
    assert!(
        fetched_text.contains("FETCH_HELLO marker-999"),
        "fetch_text body: {fetched_text}"
    );
    assert!(
        !fetched_text.contains('<'),
        "fetch_text strips tags: {fetched_text}"
    );
    assert!(
        !fetched_text.contains("hidden_xyz"),
        "fetch_text drops script: {fetched_text}"
    );

    // close kills the browser
    let closed = c.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");

    // EOF the server and expect a clean exit (proves no hang, no orphan hold).
    drop(c.stdin);
    let mut exited = false;
    for _ in 0..100 {
        match c.child.try_wait() {
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
        c.child.kill().ok();
        panic!("server did not exit after stdin EOF");
    }
}
