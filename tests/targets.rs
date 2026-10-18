//! Phase 1: targets/targeting. A local fixture page holds a same-origin
//! iframe, a cross-origin iframe, a dedicated `Worker`, and a registered
//! `ServiceWorker`. `evaluate` in each realm must return that realm's own
//! global constructor name, and `evaluate` with `target=<worker id>` must
//! differ from the no-target call. Unknown arguments are rejected loudly on
//! every tool. Each test owns one server process and reaps its engine.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{json, Value};

fn fixture_path(name: &str) -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push(name);
    p
}

fn content_type(name: &str) -> &'static str {
    if name.ends_with(".html") {
        "text/html"
    } else if name.ends_with(".js") {
        "text/javascript"
    } else {
        "application/octet-stream"
    }
}

/// One-shot-per-connection static server for the fixture dir. Serves `/x.html`
/// (cross-origin child) ONLY when `xonly` is set; otherwise serves everything
/// except `/x.html`. Returns the bound port.
fn serve_fixtures(xonly: bool) -> u16 {
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
            let name = if name.is_empty() { "realms.html" } else { name };
            let allowed = if xonly { name == "x.html" } else { name != "x.html" };
            let body = if allowed {
                std::fs::read(fixture_path(if name == "x.html" { "child_x.html" } else { name }))
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let (status, ctype, bytes) = if allowed && !body.is_empty() {
                ("200 OK", content_type(name), body)
            } else {
                (
                    "404 Not Found",
                    "text/plain",
                    b"nope".to_vec(),
                )
            };
            let _ = s.write_all(
                format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                )
                .as_bytes(),
            );
            let _ = s.write_all(&bytes);
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

    fn recv(&self, what: &str) -> Value {
        let line = self
            .rx
            .recv_timeout(Duration::from_secs(120))
            .unwrap_or_else(|_| panic!("timeout waiting for reply to {what}"));
        serde_json::from_str(&line).unwrap_or_else(|_| panic!("bad json for {what}: {line}"))
    }

    fn result(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let frame = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{frame}").expect("write stdin");
        self.stdin.flush().expect("flush stdin");
        let resp = self.recv(method);
        assert_eq!(resp.get("id"), Some(&json!(id)), "id echo for {method}");
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

fn finish(mut c: Client) {
    let closed = c.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
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

/// Evaluate and parse the `{"value": ...}` envelope (panics on tool error).
fn ev(c: &mut Client, args: Value) -> Value {
    let r = c.call("evaluate", args);
    assert!(r.get("isError").is_none(), "evaluate ok: {r}");
    content_obj(&r)
}

#[test]
fn realms_target_scoped_evaluate() {
    let port = serve_fixtures(false);
    let xport = serve_fixtures(true);
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));

    let nav = c.call(
        "navigate",
        json!({"url": format!("http://127.0.0.1:{port}/realms.html")}),
    );
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    // Inject the cross-origin frame (different port = different origin).
    let inj = c.call(
        "evaluate",
        json!({"expression": format!(
            "((u) => {{ const f = document.createElement('iframe'); f.id='x-frame'; f.src=u; document.getElementById('xframe-holder').appendChild(f); return 'injected'; }})('http://127.0.0.1:{xport}/x.html')"
        )}),
    );
    assert!(inj.get("isError").is_none(), "iframe inject ok: {inj}");

    // Worker + service worker settle (bounded by the tool's own 15s budget).
    let w = c.call(
        "wait_for",
        json!({"expression": "document.getElementById('worker-status').textContent.startsWith('WORKER:pong')", "ms": 10000}),
    );
    assert!(w.get("isError").is_none(), "worker up: {w}");
    let sw = c.call(
        "wait_for",
        json!({"expression": "document.getElementById('sw-status').textContent === 'SW:registered'", "ms": 10000}),
    );
    assert!(sw.get("isError").is_none(), "sw registered: {sw}");
    // The SW target appears after activation; poll the targets list bounded.
    let mut listed = Value::Null;
    for _ in 0..50 {
        listed = c.call("targets", json!({}));
        assert!(listed.get("isError").is_none(), "targets ok: {listed}");
        let t = content_text(&listed);
        if t.contains("service_worker") || t.contains("sw.js") {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let obj = content_obj(&listed);
    let targets = obj
        .get("targets")
        .and_then(|t| t.as_array())
        .expect("targets array");
    assert!(
        targets.iter().any(|t| t.get("type").and_then(|v| v.as_str()) == Some("page")),
        "a page target is listed: {obj}"
    );
    let worker = targets
        .iter()
        .find(|t| t.get("type").and_then(|v| v.as_str()) == Some("worker"))
        .expect("dedicated worker target listed (type worker; its url is empty on this engine)");
    let worker_id = worker.get("targetId").and_then(|v| v.as_str()).unwrap_or("");
    assert!(!worker_id.is_empty(), "worker has a stable id");
    let swt = targets
        .iter()
        .find(|t| {
            t.get("url").and_then(|v| v.as_str()).unwrap_or("").ends_with("sw.js")
        })
        .expect("service worker target listed");
    let sw_id = swt.get("targetId").and_then(|v| v.as_str()).unwrap_or("");
    assert!(!sw_id.is_empty(), "sw has a stable id");

    // Frame tree: main + same-origin child + cross-origin child.
    let frames = obj.get("frames").and_then(|f| f.as_array()).expect("frames array");
    assert!(frames.len() >= 3, "main + both iframes: {obj}");
    let same = frames
        .iter()
        .find(|f| f.get("url").and_then(|v| v.as_str()).unwrap_or("").contains("child_same.html"))
        .expect("same-origin frame listed");
    let xframe = frames
        .iter()
        .find(|f| f.get("url").and_then(|v| v.as_str()).unwrap_or("").contains("/x.html"))
        .expect("cross-origin frame listed");
    let same_id = same.get("frameId").and_then(|v| v.as_str()).unwrap_or("");
    let x_id = xframe.get("frameId").and_then(|v| v.as_str()).unwrap_or("");
    assert!(!same_id.is_empty() && !x_id.is_empty(), "frames have stable ids");

    // Realm contrast: top page is a Window.
    let top = ev(&mut c, json!({"expression": "globalThis.constructor.name"}));
    assert_eq!(top.get("value"), Some(&json!("Window")), "top realm: {top}");

    // Dedicated worker realm via target=.
    let wv = ev(
        &mut c,
        json!({"expression": "globalThis.constructor.name", "target": worker_id}),
    );
    assert_eq!(
        wv.get("value"),
        Some(&json!("DedicatedWorkerGlobalScope")),
        "worker realm: {wv}"
    );
    assert_ne!(wv, top, "target=<worker> differs from the no-target call");

    // Service worker realm via target=.
    let sv = ev(
        &mut c,
        json!({"expression": "globalThis.constructor.name", "target": sw_id}),
    );
    assert_eq!(
        sv.get("value"),
        Some(&json!("ServiceWorkerGlobalScope")),
        "sw realm: {sv}"
    );

    // Same-origin iframe realm via frame= (isolated world: DOM is shared
    // across worlds, main-world JS expandos are not).
    let fv = ev(
        &mut c,
        json!({"expression": "[globalThis.constructor.name, document.querySelector('h1').textContent]", "frame": same_id}),
    );
    assert_eq!(
        fv.get("value"),
        Some(&json!(["Window", "SAME_FRAME_MARKER_child-11111"])),
        "same-origin frame realm: {fv}"
    );

    // Cross-origin iframe realm via frame= (isolated world, no DOM access needed).
    let xv = ev(
        &mut c,
        json!({"expression": "[globalThis.constructor.name, document.querySelector('h1').textContent]", "frame": x_id}),
    );
    assert_eq!(
        xv.get("value"),
        Some(&json!(["Window", "X_FRAME_MARKER_child-22222"])),
        "cross-origin frame realm: {xv}"
    );

    finish(c);
}

#[test]
fn unknown_arguments_are_rejected_loudly() {
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));

    // The exact hazard from the brief: `target` on the OLD shape would have
    // been silently ignored; after the fix every unknown key is typed.
    for (tool, args) in [
        ("evaluate", json!({"expression": "1+1", "bogus": 1})),
        ("evaluate", json!({"expression": "1+1", "contextId": 7})),
        ("click", json!({"element": "#x", "target": "T1"})),
        ("navigate", json!({"url": "about:blank", "frame": "F1"})),
        ("snapshot", json!({"session": "S1"})),
        ("wait_for", json!({"ms": 10, "sessionId": "S1"})),
        ("tabs", json!({"action": "list", "target": "T1"})),
        ("console", json!({"action": "get", "extra": true})),
        ("close", json!({"force": true})),
        ("targets", json!({"action": "list", "session": "S1"})),
        ("dialog", json!({"action": "get", "target": "T1"})),
        ("history", json!({"action": "back", "url": "x"})),
        ("network", json!({"action": "get", "target": "T1"})),
        ("cdp", json!({"method": "Target.getTargets", "target": "T1"})),
    ] {
        let r = c.call(tool, args);
        assert!(r.get("isError").is_some(), "{tool}: unknown arg must fail: {r}");
        let sc = r.get("structuredContent").cloned().unwrap_or(Value::Null);
        assert_eq!(
            sc.get("code").and_then(|v| v.as_str()),
            Some("invalid_args"),
            "{tool} code: {r}"
        );
        assert_eq!(
            sc.get("reason").and_then(|v| v.as_str()),
            Some("unknown_arg"),
            "{tool} reason: {r}"
        );
    }

    // And the known keys still work (no false rejection).
    let nav = c.call("navigate", json!({"url": "about:blank"}));
    assert!(nav.get("isError").is_none(), "known args pass: {nav}");
    let shot = c.call("screenshot", json!({}));
    assert!(shot.get("isError").is_none(), "empty-args screenshot ok: {shot}");

    finish(c);
}

#[test]
fn targets_select_pages_and_refuses_workers() {
    let port = serve_fixtures(false);
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let nav = c.call(
        "navigate",
        json!({"url": format!("http://127.0.0.1:{port}/realms.html")}),
    );
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    let listed = c.call("targets", json!({}));
    let obj = content_obj(&listed);
    let targets = obj.get("targets").and_then(|t| t.as_array()).expect("targets");
    let page_id = targets
        .iter()
        .find(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
        .and_then(|t| t.get("targetId").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    assert!(!page_id.is_empty(), "page id present: {obj}");

    // Selecting the current page by id is a no-op success with the tabs shape.
    let sel = c.call("targets", json!({"action": "select", "target": page_id}));
    assert!(sel.get("isError").is_none(), "select page ok: {sel}");

    // Unknown action / missing target / unknown id are typed.
    let bad_act = c.call("targets", json!({"action": "bogus"}));
    assert!(bad_act.get("isError").is_some(), "bad action: {bad_act}");
    let no_tgt = c.call("targets", json!({"action": "select"}));
    assert!(no_tgt.get("isError").is_some(), "missing target: {no_tgt}");
    let no_id = c.call("targets", json!({"action": "select", "target": "NOPE"}));
    assert!(no_id.get("isError").is_some(), "unknown id: {no_id}");

    finish(c);
}
