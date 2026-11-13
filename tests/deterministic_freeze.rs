//! W1 acceptance tests: the deterministic freeze (SPEC.md §5, S1.5 + S1.6).
//!
//! Every test drives the release binary in `mcp` mode over real stdio and the
//! owned pinned `chrome-headless-shell`.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{json, Value};

struct Client {
    stdin: ChildStdin,
    rx: Receiver<String>,
    next_id: i64,
    child: Child,
}

impl Client {
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
            .recv_timeout(Duration::from_secs(180))
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
}

fn content_text(v: &Value) -> String {
    v.get("content")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string()
}

fn shutdown(c: Client) {
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

/// `file:///` URL for a repo fixture, with forward slashes on every platform.
fn fixture_url(name: &str) -> String {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push(name);
    #[cfg(windows)]
    let s = p.to_string_lossy().replace('\\', "/");
    #[cfg(not(windows))]
    let s = p.to_string_lossy().to_string();
    format!("file:///{s}")
}

/// The `text` part of a screenshot reply, parsed: the deterministic metadata.
fn shot_meta(reply: &Value) -> Value {
    let parts = reply
        .get("content")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();
    let text = parts
        .iter()
        .find(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
        .and_then(|p| p.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    serde_json::from_str(text).unwrap_or_else(|_| panic!("screenshot meta is not json: {text}"))
}

// --------------------------------------------------------------------- S1.5

/// SPEC.md S1.5 / D2: the freeze reaches what actually moves.
///
/// The fixture carries every motion source the stylesheet alone cannot stop:
/// a WAAPI animation inside an open shadow root, an SVG `animateTransform`, a
/// playing muted `<video>`, and a same-process iframe with its own CSS
/// spinner. Two deterministic captures 500 ms apart must be byte-identical,
/// and the reply must say what the walk reached.
#[test]
fn freeze_reaches_shadow_waapi_smil_and_media() {
    let mut c = Client::spawn_with_env(&[]);
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let nav = c.call("navigate", json!({"url": fixture_url("freeze.html")}));
    assert!(nav.get("isError").is_none(), "navigate: {nav}");
    let vp = c.call("viewport", json!({"width": 400, "height": 300}));
    assert!(vp.get("isError").is_none(), "viewport: {vp}");
    // Let the fixture's script run (shadow root, WAAPI, video.play()).
    let ready = c.call(
        "evaluate",
        json!({"expression": "new Promise((r) => setTimeout(() => r(window.__freezeReady === true), 300))"}),
    );
    assert!(ready.get("isError").is_none(), "evaluate ready: {ready}");

    let first = c.call("screenshot", json!({"deterministic": true}));
    assert!(first.get("isError").is_none(), "first capture: {first}");
    let meta1 = shot_meta(&first);

    std::thread::sleep(Duration::from_millis(500));

    let second = c.call("screenshot", json!({"deterministic": true}));
    assert!(second.get("isError").is_none(), "second capture: {second}");
    let meta2 = shot_meta(&second);

    let c1 = meta1.get("checksum").and_then(|v| v.as_str()).unwrap_or("");
    let c2 = meta2.get("checksum").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        !c1.is_empty() && c1 == c2,
        "two captures 500 ms apart are identical (pixelsIdentical): {c1} vs {c2}\n\
         first={meta1}\nsecond={meta2}"
    );

    let freeze = meta1.get("freeze").cloned().unwrap_or(Value::Null);
    let count = |k: &str| freeze.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
    assert!(
        count("waapiPaused") >= 1,
        "the shadow-root WAAPI animation is paused: {freeze}"
    );
    assert!(
        count("smilPaused") >= 1,
        "the SVG SMIL timeline is paused: {freeze}"
    );
    assert!(
        count("mediaPaused") >= 1,
        "the playing video is paused: {freeze}"
    );
    assert!(
        count("shadowRoots") >= 1,
        "the open shadow root was walked: {freeze}"
    );
    assert!(
        count("frames") >= 1,
        "the same-process iframe was walked: {freeze}"
    );

    // Restoration: the walk is undone, so the page is left playing, not frozen.
    let resumed = c.call(
        "evaluate",
        json!({"expression": "(() => { const v = document.getElementById('vid'); const a = document.getAnimations().length; return {paused: v.paused, animations: a, style: document.getElementById('__gladius_deterministic') !== null}; })()"}),
    );
    assert!(
        resumed.get("isError").is_none(),
        "evaluate resumed: {resumed}"
    );
    let state: Value = serde_json::from_str(&content_text(&resumed)).expect("resume json");
    let value = state.get("value").cloned().unwrap_or(Value::Null);
    assert_eq!(
        value.get("style").and_then(|v| v.as_bool()),
        Some(false),
        "the freeze stylesheet is removed after the capture: {state}"
    );
    assert_eq!(
        value.get("paused").and_then(|v| v.as_bool()),
        Some(false),
        "a video that was playing is playing again: {state}"
    );

    shutdown(c);
}
