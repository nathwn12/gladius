//! Phase 3: dialogs. The reader auto-handles every `alert` / `confirm` /
//! `prompt` / `beforeunload` with the configured policy (default accept) so
//! a session never deadlocks; the dialog's own text is recorded and the
//! policy's effect is observable in the page. Each test owns one server and
//! reaps its engine.

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
            let name = if name.is_empty() { "dialogs2.html" } else { name };
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

fn press(c: &mut Client, sid: &str, t: &str) -> Value {
    c.call(
        "cdp",
        json!({"method": "Input.dispatchMouseEvent",
               "params": {"type": t, "x": 5, "y": 5, "button": "left", "clickCount": 1},
               "session": sid}),
    )
}

fn eval_text(c: &mut Client, expr: &str) -> String {    let r = c.call("evaluate", json!({"expression": expr}));
    assert!(r.get("isError").is_none(), "evaluate ok: {r}");
    content_text(&r)
}

#[test]
fn dialogs_accept_policy_reports_text_and_effect() {
    let port = serve();
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let url = format!("http://127.0.0.1:{port}/dialogs2.html");
    let nav = c.call("navigate", json!({"url": url}));
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    // Default policy is accept.
    let pol = c.call("dialog", json!({}));
    let pobj = content_obj(&pol);
    assert_eq!(
        pobj.get("policy").and_then(|p| p.get("accept")),
        Some(&json!(true)),
        "default accept: {pobj}"
    );

    // alert: auto-accepted, message recorded, page kept running.
    let a = c.call("click", json!({"element": "#alert2"}));
    assert!(a.get("isError").is_none(), "alert click ok: {a}");
    let at = content_text(&a);
    assert!(at.contains("DLG2_probe-alert"), "alert text reported: {at}");
    assert!(eval_text(&mut c, "document.getElementById('r').textContent").contains("ALERT_DONE"), "page unblocked");

    // confirm under accept answers true.
    let cf = c.call("click", json!({"element": "#confirm2"}));
    assert!(cf.get("isError").is_none(), "confirm click ok: {cf}");
    assert!(content_text(&cf).contains("DLG2_probe-confirm"), "confirm text: {cf}");
    assert!(eval_text(&mut c, "document.getElementById('r').textContent").contains("CONFIRM_IS:true"), "accept effect");

    // prompt with configured prompt text.
    let set = c.call("dialog", json!({"action": "policy", "accept": true, "promptText": "TYPED_X"}));
    assert!(set.get("isError").is_none(), "policy set ok: {set}");
    let pr = c.call("click", json!({"element": "#prompt2"}));
    assert!(pr.get("isError").is_none(), "prompt click ok: {pr}");
    assert!(content_text(&pr).contains("DLG2_probe-prompt"), "prompt text: {pr}");

    // dialog get reports the last dialog plus the live policy (checked
    // before any evaluate drains the record).
    let g = c.call("dialog", json!({"action": "get"}));
    let gobj = content_obj(&g);
    assert_eq!(
        gobj.get("dialog").and_then(|d| d.get("type")).and_then(|t| t.as_str()),
        Some("prompt"),
        "last dialog: {gobj}"
    );
    assert_eq!(
        gobj.get("dialog").and_then(|d| d.get("message")).and_then(|m| m.as_str()),
        Some("DLG2_probe-prompt"),
        "last message: {gobj}"
    );
    assert_eq!(
        gobj.get("policy").and_then(|p| p.get("promptText")).and_then(|t| t.as_str()),
        Some("TYPED_X"),
        "live policy: {gobj}"
    );
    assert!(eval_text(&mut c, "document.getElementById('r').textContent").contains("PROMPT_IS:TYPED_X"), "prompt text effect");

    finish(c);
}

#[test]
fn dialogs_dismiss_policy_observably_refuses() {
    let port = serve();
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let url = format!("http://127.0.0.1:{port}/dialogs2.html");
    let nav = c.call("navigate", json!({"url": url}));
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    let set = c.call("dialog", json!({"action": "policy", "accept": false}));
    assert!(set.get("isError").is_none(), "dismiss policy ok: {set}");

    // confirm under dismiss answers false; prompt returns null.
    let cf = c.call("click", json!({"element": "#confirm2"}));
    assert!(cf.get("isError").is_none(), "confirm click ok: {cf}");
    assert!(eval_text(&mut c, "document.getElementById('r').textContent").contains("CONFIRM_IS:false"), "dismiss effect");
    let pr = c.call("click", json!({"element": "#prompt2"}));
    assert!(pr.get("isError").is_none(), "prompt click ok: {pr}");
    assert!(eval_text(&mut c, "document.getElementById('r').textContent").contains("PROMPT_IS:null"), "dismiss prompt effect");

    // policy without accept is a typed missing-arg; unknown action is typed.
    let noacc = c.call("dialog", json!({"action": "policy"}));
    assert!(noacc.get("isError").is_some(), "policy needs accept: {noacc}");
    assert_eq!(
        structured(&noacc).get("reason").and_then(|v| v.as_str()),
        Some("missing_arg"),
        "reason: {noacc}"
    );
    let bad = c.call("dialog", json!({"action": "bogus"}));
    assert!(bad.get("isError").is_some(), "unknown action: {bad}");
    assert_eq!(
        structured(&bad).get("reason").and_then(|v| v.as_str()),
        Some("unknown_action"),
        "reason: {bad}"
    );

    finish(c);
}

#[test]
fn dialogs_beforeunload_never_deadlocks() {
    let port = serve();
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let url = format!("http://127.0.0.1:{port}/dialogs2.html");
    let nav = c.call("navigate", json!({"url": url}));
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    // A second session on the page target for trusted input (activation).
    // `Input` is session-scoped; the active session's id is not exposed, so
    // attach explicitly and drive it through the `cdp` escape hatch.
    let listed = content_obj(&c.call("targets", json!({})));
    let page_id = listed
        .get("targets")
        .and_then(|t| t.as_array())
        .expect("targets")
        .iter()
        .find(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
        .and_then(|t| t.get("targetId").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    assert!(!page_id.is_empty(), "page id: {listed}");
    let attached = content_obj(&c.call(
        "cdp",
        json!({"method": "Target.attachToTarget", "params": {"targetId": page_id, "flatten": true}}),
    ));
    let sid = attached
        .get("result")
        .and_then(|r| r.get("sessionId"))
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    assert!(!sid.is_empty(), "second session: {attached}");
    // Trusted click on empty page area: sticky activation, no navigation.
    assert!(press(&mut c, &sid, "mousePressed").get("isError").is_none());
    assert!(press(&mut c, &sid, "mouseReleased").get("isError").is_none());

    // Arm beforeunload and leave via an in-page navigation under accept.
    // Needs sticky activation (trusted click above): without it the engine
    // fires nothing and the leave just proceeds. The dialog carries an empty
    // message on this engine (Chrome shows its own generic text instead of
    // the page's string), so the proof is the type + the blocking behavior.
    let arm = c.call("evaluate", json!({"expression": "window.onbeforeunload = (e) => { e.preventDefault(); e.returnValue = 'DLG2_before-leave'; return 'DLG2_before-leave'; }; 'armed'"}));
    assert!(arm.get("isError").is_none(), "arm ok: {arm}");
    let go = c.call("evaluate", json!({"expression": "location.href='/flip.html'; 'leaving'"}));
    assert!(go.get("isError").is_none(), "leave ok (accepted): {go}");
    let w = c.call("wait_for", json!({"expression": "location.href.includes('/flip.html')", "ms": 8000}));
    assert!(w.get("isError").is_none(), "navigated without hanging: {w}");
    let g = content_obj(&c.call("dialog", json!({"action": "get"})));
    assert_eq!(
        g.get("dialog").and_then(|d| d.get("type")).and_then(|t| t.as_str()),
        Some("beforeunload"),
        "beforeunload recorded: {g}"
    );
    assert!(
        g.get("dialog").and_then(|d| d.get("url")).and_then(|u| u.as_str()).unwrap_or("").contains("/dialogs2.html"),
        "beforeunload url: {g}"
    );

    // Same under dismiss: the dialog fires, the policy refuses, and the
    // navigation is cancelled - the page stays.
    let back = c.call("navigate", json!({"url": url}));
    assert!(back.get("isError").is_none(), "back ok: {back}");
    assert!(press(&mut c, &sid, "mousePressed").get("isError").is_none());
    assert!(press(&mut c, &sid, "mouseReleased").get("isError").is_none());
    let _ = c.call("evaluate", json!({"expression": "window.onbeforeunload = (e) => { e.preventDefault(); e.returnValue = 'DLG2_stay'; return 'DLG2_stay'; }; 'armed'"}));
    let set = c.call("dialog", json!({"action": "policy", "accept": false}));
    assert!(set.get("isError").is_none(), "dismiss ok: {set}");
    let stay = c.call("evaluate", json!({"expression": "location.href='/flip.html'; 'leaving'"}));
    assert!(stay.get("isError").is_none(), "stay call ok: {stay}");
    std::thread::sleep(Duration::from_millis(1500));
    let g2 = content_obj(&c.call("dialog", json!({"action": "get"})));
    assert_eq!(
        g2.get("dialog").and_then(|d| d.get("type")).and_then(|t| t.as_str()),
        Some("beforeunload"),
        "dismissed beforeunload recorded: {g2}"
    );
    let href2 = eval_text(&mut c, "window.location.href");
    assert!(href2.contains("/dialogs2.html"), "dismiss cancelled the leave (still here): {href2}");

    finish(c);
}
