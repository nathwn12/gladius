//! G0 safety net: the one security leak and the highest-value
//! reliability/fail-loud gaps, each proven against the real engine. Every
//! test owns one server process (and its local fixture server) and reaps it.
//!
//! - `snapshot_redacts_password_and_autocomplete_values`: password inputs
//!   and credential-ish autocomplete tokens never leak their value into
//!   either snapshot block (mask `••••` instead, raw value never used as
//!   the name fallback); a normal input value still appears.
//! - `dead_session_recovers_on_fresh_page`: closing the active target via
//!   `cdp` Target.closeTarget must not wedge later calls; `snapshot` and
//!   `navigate` recover on a fresh page and return ok.
//! - `key_rejects_unknown_name`: every recognized key name dispatches ok; an
//!   unknown multi-char name is a typed `invalid_args`/`bad_value` error
//!   naming the key, never a silent no-op.
//! - `dialog_note_truth_under_dismiss_policy`: the auto-handle note says
//!   dismissed when the recorded dialog ran under a dismiss policy and
//!   accepted only when accepted (click and evaluate paths).
//! - `wait_for_selector_requires_visibility`: a `display:none` node never
//!   satisfies a selector wait (typed timeout); a visible node resolves.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    mpsc::{self, Receiver},
    Arc,
};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

fn serve_body(body: &str) -> u16 {
    let body = body.to_string();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind localhost");
    let port = listener.local_addr().expect("local addr").port();
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let mut s = s;
            let mut buf = [0u8; 8192];
            let _ = s.read(&mut buf);
            let _ = s.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            );
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
        Self::spawn_with_env(&[])
    }

    fn spawn_with_env(envs: &[(&str, &str)]) -> Self {
        let bin = env!("CARGO_BIN_EXE_gladius");
        let mut cmd = Command::new(bin);
        cmd.arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for (k, v) in envs {
            cmd.env(k, v);
        }
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

fn is_error(v: &Value) -> bool {
    v.get("isError").and_then(|e| e.as_bool()).unwrap_or(false)
}

fn finish(mut c: Client) {
    let closed = c.call("close", json!({}));
    assert!(!is_error(&closed), "close ok: {closed}");
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

/// Scripted fake CDP engine (in-process, loopback only): answers the
/// attach flow with canned results. The FIRST `Runtime.evaluate` carrying
/// params fails with a session-death error; every later evaluate answers
/// boolean true. Lets `wait_for` observe a dead session deterministically:
/// closing a real target settles (teardown + detach + re-attach) inside one
/// MCP round trip on this machine (probed), so the real engine always
/// transparently recovers before the wait's first send — no flake can
/// observe the corpse that way. The scripted error is the exact CDP shape
/// the real engine emits, exercising the identical production path
/// (`wait_for` -> `eval` -> `send_session` -> invalidate + typed error).
struct FakeEngine {
    url: String,
    evals: Arc<AtomicUsize>,
    rt: Option<tokio::runtime::Runtime>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl FakeEngine {
    fn spawn() -> Self {
        let evals = Arc::new(AtomicUsize::new(0));
        let ln = std::net::TcpListener::bind("127.0.0.1:0").expect("bind fake engine");
        ln.set_nonblocking(true).expect("nonblocking");
        let port = ln.local_addr().expect("addr").port();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("fake engine runtime");
        let ev = evals.clone();
        let task = rt.spawn(async move {
            let ln = tokio::net::TcpListener::from_std(ln).expect("tokio listener");
            let sid = Arc::new(AtomicU64::new(1));
            loop {
                let (s, _) = match ln.accept().await {
                    Ok(x) => x,
                    Err(_) => break,
                };
                let ev = ev.clone();
                let sid = sid.clone();
                tokio::spawn(async move {
                    use futures_util::{SinkExt, StreamExt};
                    use tokio_tungstenite::tungstenite::Message;
                    let mut ws = match tokio_tungstenite::accept_async(s).await {
                        Ok(w) => w,
                        Err(_) => return,
                    };
                    while let Some(msg) = ws.next().await {
                        let text = match msg {
                            Ok(Message::Text(t)) => t.to_string(),
                            Ok(Message::Binary(b)) => String::from_utf8_lossy(&b).into_owned(),
                            _ => continue,
                        };
                        let v: Value = match serde_json::from_str(&text) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        let id = match v.get("id").and_then(|i| i.as_u64()) {
                            Some(i) => i,
                            None => continue,
                        };
                        let method = v
                            .get("method")
                            .and_then(|m| m.as_str())
                            .unwrap_or("")
                            .to_string();
                        if method == "Runtime.evaluate"
                            && v.get("params")
                                .is_some_and(|p| p.as_object().is_some_and(|o| !o.is_empty()))
                            && ev.fetch_add(1, Ordering::SeqCst) == 0
                        {
                            let _ = ws
                                .send(Message::Text(
                                    json!({"id": id, "error": {"code": -32001,
                                        "message": "Session with given id not found"}})
                                    .to_string()
                                    .into(),
                                ))
                                .await;
                            continue;
                        }
                        let result = match method.as_str() {
                            "Target.getTargets" => json!({"targetInfos": [
                                {"targetId": "T1", "type": "page",
                                 "url": "about:blank", "title": "fake"}
                            ]}),
                            "Target.attachToTarget" => {
                                let n = sid.fetch_add(1, Ordering::SeqCst);
                                json!({"sessionId": format!("S{n}")})
                            }
                            "Runtime.evaluate" => {
                                json!({"result": {"type": "boolean", "value": true}})
                            }
                            _ => json!({}),
                        };
                        let _ = ws
                            .send(Message::Text(
                                json!({"id": id, "result": result}).to_string().into(),
                            ))
                            .await;
                    }
                });
            }
        });
        FakeEngine {
            url: format!("ws://127.0.0.1:{port}/"),
            evals,
            rt: Some(rt),
            task: Some(task),
        }
    }

    fn evals(&self) -> usize {
        self.evals.load(Ordering::SeqCst)
    }

    fn shutdown(mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
        drop(self.rt.take());
    }
}

const SAFETY_HTML: &str = r#"<!doctype html>
<html><head><title>Safety Fixture</title></head><body>
<h1>SAFETY_MARKER_777</h1>
<input type="password" value="Pw-9f3a2c7d-4b8e" id="pw-bare">
<input type="password" value="Pw-ph-1a2b3c4d-5e6f" placeholder="Enter password">
<input type="PASSWORD" value="Pw-up-7e8f9a0b-1c2d">
<input autocomplete="current-password" value="Ac-6a1f9e2b-7c3d" id="cp1">
<input autocomplete="NEW-PASSWORD" value="Ac-nw-3d4e5f6a-7b8c">
<input autocomplete="cc-number" value="Cc-num-9d8e7f6a-5b4c">
<input autocomplete="cc-exp" value="Cc-exp-1f2e3d4a-5b6c">
<input autocomplete="cc-csc" value="Cc-csc-7a8b9c0d-1e2f">
<input autocomplete="one-time-code" value="Ot-4b5c6d7e-8f9a">
<input autocomplete="given-name shipping current-password" value="Ac-mu-2b3c4d5e-6f7a">
<input value="visible" id="normal1">
</body></html>"#;

const SECRETS: &[&str] = &[
    "Pw-9f3a2c7d-4b8e",
    "Pw-ph-1a2b3c4d-5e6f",
    "Pw-up-7e8f9a0b-1c2d",
    "Ac-6a1f9e2b-7c3d",
    "Ac-nw-3d4e5f6a-7b8c",
    "Cc-num-9d8e7f6a-5b4c",
    "Cc-exp-1f2e3d4a-5b6c",
    "Cc-csc-7a8b9c0d-1e2f",
    "Ot-4b5c6d7e-8f9a",
    "Ac-mu-2b3c4d5e-6f7a",
];

#[test]
fn snapshot_redacts_password_and_autocomplete_values() {
    let port = serve_body(SAFETY_HTML);
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let url = format!("http://127.0.0.1:{port}/");
    let nav = c.call("navigate", json!({"url": url}));
    assert!(!is_error(&nav), "navigate ok: {nav}");

    let snap = c.call("snapshot", json!({}));
    assert!(!is_error(&snap), "snapshot ok: {snap}");
    let blocks = snap
        .get("content")
        .and_then(|cc| cc.as_array())
        .expect("content array");
    assert_eq!(blocks.len(), 2, "AX snapshot carries two blocks: {snap}");
    let text = blocks[0].get("text").and_then(|t| t.as_str()).unwrap_or("");
    let second = blocks[1].get("text").and_then(|t| t.as_str()).unwrap_or("");

    for s in SECRETS {
        assert!(!text.contains(s), "human block leaks {s:?}:\n{text}");
        assert!(
            !second.contains(s),
            "structured block leaks {s:?}:\n{second}"
        );
    }
    assert!(
        text.contains("••••"),
        "human block carries the mask:\n{text}"
    );
    assert!(
        second.contains("••••"),
        "structured block carries the mask:\n{second}"
    );
    assert!(
        text.contains("visible"),
        "normal value kept in text:\n{text}"
    );
    assert!(
        second.contains("visible"),
        "normal value kept in structured block:\n{second}"
    );

    // Node-level: every sensitive input reports exactly the mask (never the
    // raw value, never as the name), the placeholder input falls back to
    // its placeholder as the name, and the normal input keeps its value.
    let obj: Value = serde_json::from_str(second).expect("second block parses");
    let nodes = obj
        .get("nodes")
        .and_then(|n| n.as_array())
        .expect("nodes array");
    let mut masked = 0;
    for n in nodes {
        let val = n.get("value").and_then(|x| x.as_str()).unwrap_or("");
        let name = n.get("name").and_then(|x| x.as_str()).unwrap_or("");
        for s in SECRETS {
            assert!(!val.contains(s), "node value leaks {s:?}: {n}");
            assert!(!name.contains(s), "node name leaks {s:?}: {n}");
        }
        if val == "••••" {
            masked += 1;
        }
    }
    assert!(
        masked >= 10,
        "every sensitive input masked (saw {masked}): {second}"
    );
    let ph = nodes
        .iter()
        .find(|n| n.get("name").and_then(|x| x.as_str()) == Some("Enter password"))
        .expect("placeholder fallback names the node");
    assert_eq!(
        ph.get("value").and_then(|x| x.as_str()),
        Some("••••"),
        "placeholder-named password node still masked: {ph}"
    );
    let normal = nodes
        .iter()
        .find(|n| n.get("value").and_then(|x| x.as_str()) == Some("visible"))
        .expect("normal node keeps its value");
    assert_eq!(
        normal.get("name").and_then(|x| x.as_str()),
        Some("visible"),
        "normal node keeps the legacy value-as-name fallback: {normal}"
    );

    finish(c);
}

#[test]
fn dead_session_recovers_on_fresh_page() {
    let port = serve_body(SAFETY_HTML);
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let url = format!("http://127.0.0.1:{port}/");
    let nav = c.call("navigate", json!({"url": url}));
    assert!(!is_error(&nav), "navigate ok: {nav}");
    let snap0 = c.call("snapshot", json!({}));
    assert!(!is_error(&snap0), "baseline snapshot ok: {snap0}");

    // Close the ACTIVE target out from under the cached session.
    let listed: Value =
        serde_json::from_str(&content_text(&c.call("targets", json!({})))).expect("targets json");
    let active_id = listed
        .get("targets")
        .and_then(|t| t.as_array())
        .expect("targets array")
        .iter()
        .find(|t| {
            t.get("type").and_then(|v| v.as_str()) == Some("page")
                && t.get("active").and_then(|v| v.as_bool()) == Some(true)
        })
        .and_then(|t| t.get("targetId").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    assert!(!active_id.is_empty(), "active page target: {listed}");
    let closed = c.call(
        "cdp",
        json!({"method": "Target.closeTarget", "params": {"targetId": active_id}}),
    );
    assert!(!is_error(&closed), "closeTarget ok: {closed}");
    let ok_close: bool = serde_json::from_str::<Value>(&content_text(&closed))
        .ok()
        .and_then(|o| o.get("result")?.get("success")?.as_bool())
        .unwrap_or(false);
    assert!(ok_close, "engine closed the target: {closed}");
    // Let the detach land so the test exercises recovery, not a still-live page.
    std::thread::sleep(Duration::from_secs(1));

    // First navigate: the corpse may fail it with the typed stale-session
    // error (single send, never a silent resend). A permanent failure is
    // the only wrong answer.
    let nav1 = c.call("navigate", json!({"url": url}));
    if is_error(&nav1) {
        let sc = structured(&nav1);
        assert_eq!(
            sc.get("code").and_then(|v| v.as_str()),
            Some("invalid_target"),
            "stale-session code: {nav1}"
        );
        assert_eq!(
            sc.get("reason").and_then(|v| v.as_str()),
            Some("stale_session"),
            "stale-session reason: {nav1}"
        );
        let prose = content_text(&nav1);
        assert!(
            prose.contains("may or may not have executed") && !prose.contains("not sent"),
            "stale-session prose owns the ambiguity: {nav1}"
        );
    }
    // The second navigate and the following snapshot MUST succeed on a
    // fresh page: recovery, not a permanent failure.
    let nav2 = c.call("navigate", json!({"url": url}));
    assert!(
        !is_error(&nav2),
        "second navigate recovers on a fresh page: {nav2}"
    );
    let snap2 = c.call("snapshot", json!({}));
    assert!(!is_error(&snap2), "post-recovery snapshot ok: {snap2}");
    assert!(
        content_text(&snap2).contains("SAFETY_MARKER_777"),
        "fresh page drives the fixture again: {}",
        content_text(&snap2).chars().take(300).collect::<String>()
    );

    finish(c);
}

#[test]
fn wait_for_selector_fails_fast_on_dead_session() {
    // Deterministic dead session via the scripted engine (see FakeEngine):
    // the first session-scoped evaluate fails with the exact session-gone
    // CDP error, every later one answers boolean true.
    let fake = FakeEngine::spawn();
    let mut c = Client::spawn_with_env(&[("GLADIUS_CDP_URL", &fake.url)]);
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));

    // Absent selector, long budget: a live page would poll the full 10s.
    // On the corpse the wait must stop at once with typed stale_session,
    // after exactly one session-scoped send (no resend, no polling).
    let t0 = Instant::now();
    let w = c.call(
        "wait_for",
        json!({"selector": "#no-such-element-xyz", "ms": 10000}),
    );
    let dt = t0.elapsed();
    assert!(is_error(&w), "dead session must fail, not resolve: {w}");
    let sc = structured(&w);
    assert_eq!(
        sc.get("code").and_then(|v| v.as_str()),
        Some("invalid_target"),
        "stale-session code, not a timeout: {w}"
    );
    assert_eq!(
        sc.get("reason").and_then(|v| v.as_str()),
        Some("stale_session"),
        "stale-session reason, not a timeout: {w}"
    );
    assert!(
        dt < Duration::from_secs(2),
        "prompt stale-session, not budget burn: took {dt:?}"
    );
    assert_eq!(
        fake.evals(),
        1,
        "exactly one session-scoped send, never a resend or a poll loop"
    );

    // Recovery: the next wait re-attaches and resolves on the fresh session.
    let w2 = c.call("wait_for", json!({"selector": "#nope", "ms": 2000}));
    assert!(!is_error(&w2), "next wait recovers: {w2}");
    assert!(fake.evals() >= 2, "recovery re-attached and evaluated");

    finish(c);
    fake.shutdown();
}

#[test]
fn key_rejects_unknown_name() {
    let port = serve_body(SAFETY_HTML);
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let url = format!("http://127.0.0.1:{port}/");
    let nav = c.call("navigate", json!({"url": url}));
    assert!(!is_error(&nav), "navigate ok: {nav}");

    // Every currently recognized name keeps working.
    let mut names: Vec<String> = vec![
        "enter",
        "tab",
        "escape",
        "esc",
        "backspace",
        "delete",
        "del",
        "arrowup",
        "up",
        "arrowdown",
        "down",
        "arrowleft",
        "left",
        "arrowright",
        "right",
        "home",
        "end",
        "pageup",
        "pagedown",
        "space",
        "spacebar",
        "shift",
        "control",
        "ctrl",
        "alt",
    ]
    .into_iter()
    .map(|s| s.to_string())
    .collect();
    for n in 1..=12 {
        names.push(format!("f{n}"));
    }
    for s in ["a", "Z", "5", " "] {
        names.push(s.to_string());
    }
    for n in &names {
        let r = c.call("key", json!({"key": n}));
        assert!(!is_error(&r), "recognized key {n:?} dispatches: {r}");
    }

    // One unknown multi-char name fails loud, naming the key.
    let bad = c.call("key", json!({"key": "NoSuchKey"}));
    assert!(is_error(&bad), "unknown key must fail, not no-op: {bad}");
    let sc = structured(&bad);
    assert_eq!(
        sc.get("code").and_then(|v| v.as_str()),
        Some("invalid_args"),
        "unknown key code: {bad}"
    );
    assert_eq!(
        sc.get("reason").and_then(|v| v.as_str()),
        Some("bad_value"),
        "unknown key reason: {bad}"
    );
    assert!(
        content_text(&bad).contains("NoSuchKey"),
        "error names the key: {bad}"
    );

    finish(c);
}

const DIALOG_HTML: &str = r#"<!doctype html>
<html><head><title>Safety Dialog Fixture</title></head><body>
<h1>SAFETY_DIALOG_MARKER</h1>
<button id="confirm2" onclick="document.getElementById('r').textContent='CONFIRM_IS:'+confirm('DLG_probe-confirm')">FIRE_CONFIRM</button>
<div id="r">R:none</div>
</body></html>"#;

fn eval_text(c: &mut Client, expr: &str) -> String {
    let r = c.call("evaluate", json!({"expression": expr}));
    assert!(!is_error(&r), "evaluate ok: {r}");
    content_text(&r)
}

#[test]
fn dialog_note_truth_under_dismiss_policy() {
    let port = serve_body(DIALOG_HTML);
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let url = format!("http://127.0.0.1:{port}/");
    let nav = c.call("navigate", json!({"url": url}));
    assert!(!is_error(&nav), "navigate ok: {nav}");

    // Dismiss policy: both the click path and the evaluate path report dismissed.
    let set = c.call("dialog", json!({"action": "policy", "accept": false}));
    assert!(!is_error(&set), "dismiss policy ok: {set}");
    let cf = c.call("click", json!({"element": "#confirm2"}));
    assert!(!is_error(&cf), "confirm click ok: {cf}");
    let note = content_text(&cf);
    assert!(
        note.contains("(dismissed, page unblocked)"),
        "dismiss note truthful: {note}"
    );
    assert!(
        !note.contains("accepted"),
        "dismiss note never claims accepted: {note}"
    );
    assert!(
        eval_text(&mut c, "document.getElementById('r').textContent").contains("CONFIRM_IS:false"),
        "dismiss effect observed"
    );
    let ev = c.call(
        "evaluate",
        json!({"expression": "confirm('DLG_probe-eval')"}),
    );
    assert!(!is_error(&ev), "eval confirm ok: {ev}");
    let enote = content_text(&ev);
    assert!(
        enote.contains("(dismissed, page unblocked)"),
        "evaluate dismiss note truthful: {enote}"
    );

    // Accept policy: both paths report accepted.
    let back = c.call("dialog", json!({"action": "policy", "accept": true}));
    assert!(!is_error(&back), "accept policy ok: {back}");
    let cf2 = c.call("click", json!({"element": "#confirm2"}));
    assert!(!is_error(&cf2), "confirm click ok: {cf2}");
    let note2 = content_text(&cf2);
    assert!(
        note2.contains("(accepted, page unblocked)"),
        "accept note truthful: {note2}"
    );
    assert!(
        !note2.contains("dismissed"),
        "accept note never claims dismissed: {note2}"
    );
    let ev2 = c.call(
        "evaluate",
        json!({"expression": "confirm('DLG_probe-eval2')"}),
    );
    assert!(!is_error(&ev2), "eval confirm ok: {ev2}");
    assert!(
        content_text(&ev2).contains("(accepted, page unblocked)"),
        "evaluate accept note truthful: {}",
        content_text(&ev2)
    );

    finish(c);
}

const VIS_HTML: &str = r#"<!doctype html>
<html><head><title>Safety Visibility Fixture</title></head><body>
<div id="gone" style="display:none">hidden-text</div>
<div id="here">shown-text</div>
</body></html>"#;

#[test]
fn wait_for_selector_requires_visibility() {
    let port = serve_body(VIS_HTML);
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let url = format!("http://127.0.0.1:{port}/");
    let nav = c.call("navigate", json!({"url": url}));
    assert!(!is_error(&nav), "navigate ok: {nav}");

    // Present-but-hidden never satisfies the wait: the typed timeout.
    let w = c.call("wait_for", json!({"selector": "#gone", "ms": 600}));
    assert!(is_error(&w), "hidden node must time out, not resolve: {w}");
    let sc = structured(&w);
    assert_eq!(
        sc.get("code").and_then(|v| v.as_str()),
        Some("timeout"),
        "hidden wait code: {w}"
    );
    assert_eq!(
        sc.get("reason").and_then(|v| v.as_str()),
        Some("budget_exceeded"),
        "hidden wait reason: {w}"
    );

    // A visible node resolves immediately.
    let v = c.call("wait_for", json!({"selector": "#here", "ms": 2000}));
    assert!(!is_error(&v), "visible node resolves: {v}");

    finish(c);
}
