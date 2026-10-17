//! dlg-01 + tab-02 regressions on the owned Chromium engine: JS dialogs never
//! hang (bounded wait; `alert()`/`confirm()` fire REAL `Page.javascriptDialogOpening`
//! events on Chromium, so the reader auto-accepts and clicks return a typed
//! dialog report while the page keeps running) and a freshly opened tab settles
//! so the next tool observes its committed URL. Each test owns one server
//! process and reaps its engine.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

fn fixture(name: &str) -> String {
    let p = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
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

    fn recv(&self, what: &str, timeout: Duration) -> Value {
        let line = self
            .rx
            .recv_timeout(timeout)
            .unwrap_or_else(|_| panic!("HANG: timeout waiting for reply to {what}"));
        serde_json::from_str(&line).unwrap_or_else(|_| panic!("bad json for {what}: {line}"))
    }

    fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let frame = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{frame}").expect("write stdin");
        self.stdin.flush().expect("flush stdin");
        let resp = self.recv(method, timeout);
        assert_eq!(resp.get("id"), Some(&json!(id)), "id echo for {method}");
        resp
    }

    fn result(&mut self, method: &str, params: Value, timeout: Duration) -> Value {
        let resp = self.request(method, params, timeout);
        assert!(
            resp.get("error").is_none(),
            "{method} protocol error: {resp}"
        );
        resp.get("result").cloned().unwrap_or(Value::Null)
    }

    fn call(&mut self, name: &str, args: Value, timeout: Duration) -> Value {
        self.result(
            "tools/call",
            json!({"name": name, "arguments": args}),
            timeout,
        )
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

fn finish(mut c: Client) {
    let closed = c.call("close", json!({}), Duration::from_secs(30));
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

fn boot() -> Client {
    let mut c = Client::spawn();
    c.result(
        "initialize",
        json!({"protocolVersion": "2025-06-18"}),
        Duration::from_secs(30),
    );
    c
}

/// dlg-01 slow-page face: the alert button is injected 3s AFTER load (cold
/// page outlasts navigate's load poll). The click must WAIT (bounded) for
/// it and fire it - never miss it instantly. On Chromium the alert fires a
/// real dialog: the reader auto-accepts and the click carries a typed dialog
/// report naming the probe message.
#[test]
fn dialog_click_waits_for_late_button() {
    let mut c = boot();
    let nav = c.call(
        "navigate",
        json!({"url": fixture("dialog.html")}),
        Duration::from_secs(60),
    );
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");
    // Late button: appears after 3s, fires an alert naming itself.
    let inj = c.call(
        "evaluate",
        json!({"expression": "setTimeout(() => { const b = document.createElement('button'); b.id='late-alert'; b.textContent='LATE'; b.onclick=()=>alert('DLG_PROBE_late-xyz'); document.body.appendChild(b); }, 3000); 'injected'"}),
        Duration::from_secs(30),
    );
    assert!(inj.get("isError").is_none(), "inject ok: {inj}");

    let t0 = Instant::now();
    let clk = c.call(
        "click",
        json!({"element": "#late-alert"}),
        Duration::from_secs(30),
    );
    let dt = t0.elapsed();
    assert!(
        clk.get("isError").is_none(),
        "late-button click ok (took {dt:?}): {clk}"
    );
    assert!(
        dt >= Duration::from_secs(2),
        "click actually waited for the late button: {dt:?}"
    );
    let t = content_text(&clk);
    assert!(
        t.contains("dialog") && t.contains("DLG_PROBE_late-xyz"),
        "Chromium fires a real dialog with the probe message, got: {t}"
    );
    finish(c);
}

/// dlg-01: clicking a button that fires `alert()` must return BOUNDED (well
/// under the 15s bench ceiling) with a typed dialog report - Chromium fires
/// `Page.javascriptDialogOpening`, the reader auto-accepts, and the click
/// reports the dialog's own message text. Never a hang.
#[test]
fn dialog_alert_reports_real_dialog_and_never_hangs() {
    let mut c = boot();
    let nav = c.call(
        "navigate",
        json!({"url": fixture("dialog.html")}),
        Duration::from_secs(60),
    );
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    let t0 = Instant::now();
    let clk = c.call(
        "click",
        json!({"element": "#alert-btn"}),
        Duration::from_secs(30),
    );
    let dt = t0.elapsed();
    assert!(
        clk.get("isError").is_none(),
        "alert click bounded ok (took {dt:?}): {clk}"
    );
    assert!(
        dt < Duration::from_secs(15),
        "bounded well under the 15s bench ceiling: {dt:?}"
    );
    let t = content_text(&clk);
    assert!(
        t.contains("dialog") && t.contains("DLG_PROBE_alert-xyz"),
        "real dialog report with the probe message, got: {t}"
    );
    // The page kept running: the click's own handler ran.
    let res = content_text(&c.call(
        "evaluate",
        json!({"expression": "document.getElementById('result').textContent"}),
        Duration::from_secs(30),
    ));
    assert!(
        res.contains("RESULT:accepted"),
        "page unblocked, handler completed: {res}"
    );
    finish(c);
}

/// dlg-01 second face: `confirm()` fires a real dialog on Chromium (auto-
/// accepted, so it answers true) and the page keeps running. Bounded, never
/// a hang.
#[test]
fn dialog_confirm_reports_real_dialog_and_never_hangs() {
    let mut c = boot();
    let nav = c.call(
        "navigate",
        json!({"url": fixture("dialog.html")}),
        Duration::from_secs(60),
    );
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    let clk = c.call(
        "click",
        json!({"element": "#confirm-btn"}),
        Duration::from_secs(30),
    );
    assert!(clk.get("isError").is_none(), "confirm click ok: {clk}");
    let t = content_text(&clk);
    assert!(
        t.contains("dialog") && t.contains("DLG_PROBE_confirm-xyz"),
        "real dialog report with the probe message, got: {t}"
    );
    let res = content_text(&c.call(
        "evaluate",
        json!({"expression": "document.getElementById('result').textContent"}),
        Duration::from_secs(30),
    ));
    assert!(
        res.contains("CONFIRM:true"),
        "confirm answered true and the page kept running: {res}"
    );
    finish(c);
}

/// tab-02 slow-commit face: the new tab's navigation takes ~4s to answer.
/// `tabs new` must WAIT through the blank phase (bounded) and then observe
/// the committed URL - never strand the next tool on about:blank. New tabs
/// open via the `tabs new` tool (engine-level target creation).
#[test]
fn new_tab_waits_for_slow_commit() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind localhost");
    let port = listener.local_addr().expect("local addr").port();
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        for s in listener.incoming().flatten() {
            let mut s = s;
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let req: String = String::from_utf8_lossy(&buf).into_owned();
            let path = req
                .lines()
                .next()
                .unwrap_or("")
                .split_whitespace()
                .nth(1)
                .unwrap_or("/")
                .to_string();
            // The target page answers SLOWLY (the uncommitted-blank phase);
            // anything else (favicon) answers at once.
            if path.starts_with("/slow") {
                std::thread::sleep(Duration::from_secs(4));
            }
            let body = "<!doctype html><html><head><title>slow</title></head>\
                        <body><h1>SLOW_COMMIT_MARKER</h1></body></html>";
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

    let mut c = boot();
    let nav = c.call(
        "navigate",
        json!({"url": fixture("fixture.html")}),
        Duration::from_secs(60),
    );
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    let t0 = Instant::now();
    let new = c.call(
        "tabs",
        json!({"action": "new", "url": format!("http://127.0.0.1:{port}/slow")}),
        Duration::from_secs(40),
    );
    let dt = t0.elapsed();
    assert!(new.get("isError").is_none(), "tabs new ok: {new}");
    assert!(
        dt >= Duration::from_secs(3),
        "tabs new waited for the slow commit: {dt:?}"
    );
    let st = content_text(&new);
    assert!(st.contains("/slow"), "new tab waited for commit, got: {st}");
    let href = content_text(&c.call(
        "evaluate",
        json!({"expression": "location.href"}),
        Duration::from_secs(30),
    ));
    assert!(
        href.contains("/slow"),
        "fresh tab observes committed slow URL, got: {href}"
    );
    finish(c);
}

/// tab-02: open a new tab via `tabs new` and IMMEDIATELY read its URL - it
/// must be the committed page, not "" / about:blank.
#[test]
fn new_tab_settles_to_committed_url() {
    let mut c = boot();
    let nav = c.call(
        "navigate",
        json!({"url": fixture("fixture.html")}),
        Duration::from_secs(60),
    );
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    let new = c.call(
        "tabs",
        json!({"action": "new", "url": fixture("tab-target.html")}),
        Duration::from_secs(30),
    );
    assert!(new.get("isError").is_none(), "tabs new ok: {new}");
    let st = content_text(&new);
    assert!(
        st.contains("tab-target.html"),
        "new-tab report already shows the committed URL, got: {st}"
    );

    let href = content_text(&c.call(
        "evaluate",
        json!({"expression": "location.href"}),
        Duration::from_secs(30),
    ));
    assert!(
        href.contains("tab-target.html"),
        "fresh tab observes committed URL (not blank), got: {href}"
    );
    // wait_for observes the fresh tab too.
    let w = c.call(
        "wait_for",
        json!({"text": "TAB_TARGET_MARKER_abc-789", "ms": 10000}),
        Duration::from_secs(30),
    );
    assert!(
        w.get("isError").is_none(),
        "wait_for sees new-tab text: {w}"
    );

    let cls = c.call(
        "tabs",
        json!({"action": "close", "index": 1}),
        Duration::from_secs(30),
    );
    assert!(cls.get("isError").is_none(), "tabs close ok: {cls}");
    let back = c.call(
        "tabs",
        json!({"action": "select", "index": 0}),
        Duration::from_secs(30),
    );
    assert!(back.get("isError").is_none(), "tabs back ok: {back}");
    let href0 = content_text(&c.call(
        "evaluate",
        json!({"expression": "location.href"}),
        Duration::from_secs(30),
    ));
    assert!(
        href0.contains("fixture.html"),
        "returned to opener: {href0}"
    );
    finish(c);
}
