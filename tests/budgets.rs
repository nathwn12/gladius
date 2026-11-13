//! S2.3: one bounded budget per call, and a typed cancel (SPEC.md §6).
//!
//! A wedged renderer must not hold the execution lock past a budget, and a
//! client cancel must be answered immediately (the restore keeps running
//! behind the answer) instead of leaving the caller in silence.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

fn fixture(name: &str) -> String {
    let p = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    #[cfg(windows)]
    let p = p.replace('\\', "/");
    format!("file:///{p}")
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gladius-s23-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

struct Client {
    stdin: ChildStdin,
    rx: Receiver<String>,
    next_id: i64,
    child: Child,
}

impl Client {
    fn spawn(envs: &[(&str, &str)]) -> Self {
        let bin = env!("CARGO_BIN_EXE_gladius");
        let mut cmd = Command::new(bin);
        cmd.arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
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

    fn send(&mut self, v: &Value) {
        let line = v.to_string();
        writeln!(self.stdin, "{line}").expect("write stdin");
        self.stdin.flush().expect("flush stdin");
    }

    fn next(&self, dur: Duration) -> Option<Value> {
        match self.rx.recv_timeout(dur) {
            Ok(line) => Some(
                serde_json::from_str(&line)
                    .unwrap_or_else(|_| panic!("non-JSON stdout line from server: {line:?}")),
            ),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                panic!("stdout channel disconnected (server gone)")
            }
        }
    }

    /// The next frame whose `id` is `id`, ignoring other frames (a cancel
    /// answer and a later response can interleave), up to `dur`.
    fn wait_id(&self, id: &Value, dur: Duration, what: &str) -> Value {
        let end = Instant::now() + dur;
        loop {
            let now = Instant::now();
            assert!(now < end, "timeout waiting for id {id} ({what})");
            if let Some(v) = self.next(end - now) {
                if v.get("id") == Some(id) {
                    return v;
                }
            }
        }
    }

    fn call_id(&mut self, id: Value, name: &str, args: Value) {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": name, "arguments": args}}));
    }

    fn call(&mut self, name: &str, args: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.call_id(json!(id), name, args);
        let resp = self.wait_id(&json!(id), Duration::from_secs(120), name);
        assert!(resp.get("error").is_none(), "{name} protocol error: {resp}");
        resp.get("result").cloned().unwrap_or(Value::Null)
    }

    fn initialize(&mut self) {
        let id = json!(0);
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18"}}));
        let init = self.wait_id(&id, Duration::from_secs(30), "initialize");
        assert!(init.get("error").is_none(), "initialize: {init}");
    }
}

fn code_of(v: &Value) -> String {
    v.pointer("/structuredContent/code")
        .and_then(|c| c.as_str())
        .unwrap_or("<none>")
        .to_string()
}

fn reason_of(v: &Value) -> String {
    v.pointer("/structuredContent/reason")
        .and_then(|c| c.as_str())
        .unwrap_or("<none>")
        .to_string()
}

/// S2.3 (a): a wedged `Runtime.evaluate` returns inside `_budgetMs` with a
/// typed `timeout`/`budget_exceeded`, and the execution lock is free
/// afterwards (a lock-taking call works).
#[test]
fn a_wedged_call_returns_typed_timeout_and_frees_the_lock() {
    let mut c = Client::spawn(&[]);
    c.initialize();
    let nav = c.call("navigate", json!({"url": fixture("wedge.html")}));
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    // Block the renderer for 6 s but ask for a 2 s budget: the call must come
    // back typed at ~2 s, not at the 30 s CDP timeout.
    c.call_id(
        json!(10),
        "evaluate",
        json!({"expression": "window.wedge(6000)", "_budgetMs": 2000}),
    );
    let t0 = Instant::now();
    let timed_out = c.wait_id(&json!(10), Duration::from_secs(30), "wedged evaluate");
    let elapsed = t0.elapsed();
    assert!(
        elapsed < Duration::from_secs(4),
        "the wedged call returned inside its budget ({elapsed:?})"
    );
    let result = timed_out.get("result").cloned().unwrap_or(Value::Null);
    assert!(
        result.get("isError").and_then(|v| v.as_bool()) == Some(true),
        "the timeout is a tool failure, not a protocol error: {timed_out}"
    );
    assert_eq!(
        code_of(&result),
        "timeout",
        "the budget answer is typed timeout: {timed_out}"
    );
    assert_eq!(
        reason_of(&result),
        "budget_exceeded",
        "the reason is budget_exceeded: {timed_out}"
    );
    assert_eq!(
        result
            .pointer("/structuredContent/tool")
            .and_then(|v| v.as_str()),
        Some("evaluate"),
        "the answer names the tool: {timed_out}"
    );
    assert_eq!(
        result
            .pointer("/structuredContent/budgetMs")
            .and_then(|v| v.as_u64()),
        Some(2000),
        "the answer names the budget: {timed_out}"
    );

    // The execution lock was released: a lock-taking call runs (and the
    // renderer is free once the 6 s loop ends).
    let again = c.call("evaluate", json!({"expression": "1 + 1"}));
    assert!(
        again.get("isError").is_none(),
        "the exec lock is free after a budget timeout: {again}"
    );
    assert_eq!(
        again.pointer("/structuredContent/value"),
        Some(&json!(2)),
        "the page answers again: {again}"
    );
    // A read-only call works too.
    let snap = c.call("snapshot", json!({}));
    assert!(
        snap.get("isError").is_none(),
        "snapshot after timeout: {snap}"
    );

    let _ = c.call("close", json!({}));
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

/// S2.3 (b): a `notifications/cancelled` during a deterministic capture is
/// answered in under 2 s with `cancelled`/`client_cancelled`, while the page's
/// viewport override is still restored afterwards.
#[test]
fn cancelled_call_answers_promptly() {
    const STATE: &str = "[window.innerWidth, window.innerHeight, \
        document.getElementById('__gladius_deterministic') === null]";
    let dir = scratch("cancel");
    let vdir = dir.join("visual");
    std::fs::create_dir_all(&vdir).expect("visual dir");
    let vdir_s = vdir.to_string_lossy().to_string();
    let mut c = Client::spawn(&[
        // Widen the mutation window so the cancel lands inside it.
        ("GLADIUS_TEST_CRITICAL_DELAY_MS", "1500"),
        ("GLADIUS_VISUAL_DIR", &vdir_s),
    ]);
    c.initialize();
    let nav = c.call("navigate", json!({"url": fixture("layout.html")}));
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");
    let vp = c.call("viewport", json!({"width": 900, "height": 700}));
    assert!(vp.get("isError").is_none(), "viewport ok: {vp}");

    // Start a deterministic capture, then cancel it inside its window.
    c.call_id(json!(10), "screenshot", json!({"deterministic": true}));
    std::thread::sleep(Duration::from_millis(400));
    c.send(
        &json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
        "params": {"requestId": 10}}),
    );
    let t0 = Instant::now();
    let answer = c.wait_id(&json!(10), Duration::from_secs(10), "cancelled capture");
    let latency = t0.elapsed();
    assert!(
        latency < Duration::from_secs(2),
        "the cancel was answered in {latency:?} (must be under 2 s)"
    );
    let result = answer.get("result").cloned().unwrap_or(Value::Null);
    assert_eq!(
        code_of(&result),
        "cancelled",
        "the cancel answer is typed cancelled: {answer}"
    );
    assert_eq!(
        reason_of(&result),
        "client_cancelled",
        "the reason is client_cancelled: {answer}"
    );

    // The restore still ran: the caller's override is back and no freeze
    // stylesheet is left behind (this read needs the exec lock, so it also
    // proves the lock was released only after the restore).
    let state = c.call("evaluate", json!({"expression": STATE}));
    assert!(
        state.get("isError").is_none(),
        "the page answers after the cancel: {state}"
    );
    let arr = state
        .pointer("/structuredContent/value")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        arr.first().and_then(|v| v.as_f64()),
        Some(900.0),
        "innerWidth restored after the cancel: {state}"
    );
    assert_eq!(
        arr.get(1).and_then(|v| v.as_f64()),
        Some(700.0),
        "innerHeight restored after the cancel: {state}"
    );
    assert_eq!(
        arr.get(2).and_then(|v| v.as_bool()),
        Some(true),
        "no freeze stylesheet left behind: {state}"
    );

    let _ = c.call("close", json!({}));
    std::fs::remove_dir_all(&dir).ok();
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
