//! Phase 6: raw CDP escape hatch. `cdp` sends any protocol method with
//! params on the browser-level connection or on one named session, returning
//! the raw CDP result; a CDP error reply is a tool error (`isError`), never
//! a protocol error. Each test owns one server and reaps its engine.

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

#[test]
fn cdp_browser_level_and_session_scoped() {
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let nav = c.call("navigate", json!({"url": fixture_url()}));
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    // Browser-level: no session. Browser.getVersion names the product.
    let ver = c.call("cdp", json!({"method": "Browser.getVersion"}));
    assert!(ver.get("isError").is_none(), "getVersion ok: {ver}");
    let vobj = content_obj(&ver);
    assert!(
        vobj.get("result")
            .and_then(|r| r.get("product"))
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .contains("Chrome"),
        "chromium product: {vobj}"
    );

    // Browser-level target query reaches every target.
    let tgt = c.call("cdp", json!({"method": "Target.getTargets"}));
    assert!(tgt.get("isError").is_none(), "getTargets ok: {tgt}");
    assert!(
        content_obj(&tgt)
            .get("result")
            .and_then(|r| r.get("targetInfos"))
            .and_then(|t| t.as_array())
            .is_some(),
        "targetInfos: {tgt}"
    );

    // Attach a second session to the page and evaluate on it: the raw
    // session route, distinct from the tracked active session.
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
    let att = content_obj(&c.call(
        "cdp",
        json!({"method": "Target.attachToTarget", "params": {"targetId": page_id, "flatten": true}}),
    ));
    let sid = att
        .get("result")
        .and_then(|r| r.get("sessionId"))
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    assert!(!sid.is_empty(), "second session: {att}");
    let ev = c.call(
        "cdp",
        json!({"method": "Runtime.evaluate",
               "params": {"expression": "6*7", "returnByValue": true},
               "session": sid}),
    );
    assert!(ev.get("isError").is_none(), "session eval ok: {ev}");
    assert_eq!(
        content_obj(&ev)
            .get("result")
            .and_then(|r| r.get("result"))
            .and_then(|r| r.get("value")),
        Some(&json!(42)),
        "raw result: {ev}"
    );

    // Unknown methods and bad sessions are tool errors, never protocol errors.
    let bad = c.call("cdp", json!({"method": "Nope.nope"}));
    assert!(
        bad.get("isError").is_some(),
        "unknown method isError: {bad}"
    );
    assert_eq!(
        structured(&bad).get("code").and_then(|v| v.as_str()),
        Some("error"),
        "fallback code: {bad}"
    );
    let badsess = c.call(
        "cdp",
        json!({"method": "Runtime.evaluate",
               "params": {"expression": "1", "returnByValue": true},
               "session": "NO_SUCH_SESSION"}),
    );
    assert!(
        badsess.get("isError").is_some(),
        "bad session isError: {badsess}"
    );

    // Missing method and non-object params are typed argument errors.
    let nomethod = c.call("cdp", json!({}));
    assert!(
        nomethod.get("isError").is_some(),
        "missing method: {nomethod}"
    );
    assert_eq!(
        structured(&nomethod).get("reason").and_then(|v| v.as_str()),
        Some("missing_arg"),
        "reason: {nomethod}"
    );
    let badparams = c.call("cdp", json!({"method": "Target.getTargets", "params": [1]}));
    assert!(
        badparams.get("isError").is_some(),
        "bad params: {badparams}"
    );
    assert_eq!(
        structured(&badparams)
            .get("reason")
            .and_then(|v| v.as_str()),
        Some("bad_value"),
        "reason: {badparams}"
    );

    finish(c);
}
