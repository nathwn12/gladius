//! Review-findings proof tests: each drives the release binary in `mcp`
//! mode over real stdio and proves one hardened behavior.
//!
//! - `startup_timeout_kills_stub_tree` (P1): forces the startup-timeout path
//!   via the `GLADIUS_TEST_STUB_SLEEP_SECS` / `GLADIUS_STARTUP_TIMEOUT_SECS`
//!   hooks and asserts no stub process survives.
//! - `notifications_get_no_reply` (P2): id-less requests get NO response.
//! - `evaluate_distinguishes_null_from_undefined` (P2): `null` yields
//!   `{"value":null}`; `undefined` is an explicit `no value` tool error.
//! - `snapshot_caps_huge_page_at_source` (P2): a ~400KB page snapshots to
//!   <=8000 chars.
//! - `wait_for_hard_deadline` (P2): `ms:0` is an immediate no-op success;
//!   missing selectors fail at the budget; over-max budgets clamp to 15s.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

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

    fn spawn() -> Self {
        // Default-engine location pin (see e2e_mcp.rs): cargo tree has no
        // obscura sibling, so point at the repo-pinned bin/obscura.exe.
        // (The stub-timeout test below overrides with GLADIUS_BACKEND=chrome.)
        Self::spawn_with_env(&[(
            "GLADIUS_OBSCURA",
            &format!("{}/bin/obscura.exe", env!("CARGO_MANIFEST_DIR")),
        )])
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
        writeln!(self.stdin, "{frame}").expect("write stdin");
        self.stdin.flush().expect("flush stdin");
        let resp = self.recv(method);
        assert_eq!(resp.get("id"), Some(&json!(id)), "id echo for {method}");
        resp
    }

    /// Raw JSON-RPC notification (NO `id`): per spec it must get NO reply.
    fn notify_raw(&mut self, method: &str, params: Value) {
        let frame = json!({"jsonrpc": "2.0", "method": method, "params": params});
        writeln!(self.stdin, "{frame}").expect("write stdin");
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

/// EOF the server and require a clean exit (proves no hang, no orphan hold).
fn shutdown(c: Client) {
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

/// PIDs of live stub-sleeper processes (command line carries the
/// `GLADIUS_STARTUP_STUB` marker). Excludes our own query process, whose
/// command line also contains the marker string.
fn stub_pids() -> Vec<u32> {
    let mut q = Command::new("powershell");
    q.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Get-CimInstance Win32_Process -Filter \"Name='powershell.exe' OR Name='pwsh.exe'\" | Where-Object { $_.CommandLine -like '*GLADIUS_STARTUP_STUB*' } | ForEach-Object { $_.ProcessId }",
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        q.creation_flags(0x08000000);
    }
    let me = q.spawn().expect("spawn stub query");
    let me_pid = me.id();
    let out = me.wait_with_output().expect("stub query output");
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace()
        .filter_map(|t| t.parse::<u32>().ok())
        .filter(|p| *p != me_pid)
        .collect()
}

#[test]
fn startup_timeout_kills_stub_tree() {
    // Force the startup-timeout path: silent stub + 2s deadline. Chrome
    // opt-in: the GLADIUS_TEST_STUB_SLEEP_SECS hook lives in the
    // chrome-headless-shell launch path.
    let mut c = Client::spawn_with_env(&[
        ("GLADIUS_BACKEND", "chrome"),
        ("GLADIUS_TEST_STUB_SLEEP_SECS", "90"),
        ("GLADIUS_STARTUP_TIMEOUT_SECS", "2"),
    ]);
    let init = c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    assert_eq!(
        init.get("serverInfo").and_then(|s| s.get("name")),
        Some(&json!("gladius")),
        "serverInfo"
    );

    // Any browser tool triggers launch; it must fail with the timeout error
    // (proves we exercised the timeout path, not some other failure).
    let t0 = Instant::now();
    let snap = c.call("snapshot", json!({}));
    let dt = t0.elapsed();
    assert!(
        snap.get("isError").is_some(),
        "stub launch must fail as a tool error: {snap}"
    );
    let msg = content_text(&snap);
    assert!(
        msg.contains("timeout"),
        "timeout-path error message, got: {msg}"
    );
    assert!(
        dt < Duration::from_secs(30),
        "injected deadline respected, took {dt:?}"
    );

    // No stub process may survive: poll for the tree kill to land.
    let mut survivors = stub_pids();
    for _ in 0..30 {
        if survivors.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
        survivors = stub_pids();
    }
    assert!(
        survivors.is_empty(),
        "ORPHAN: stub processes survived startup timeout: {survivors:?}"
    );

    // Server itself is still alive and serving.
    let pong = c.result("ping", json!({}));
    assert_eq!(pong, json!({}), "ping replies {{}}");
    shutdown(c);

    // End state: still zero survivors after server exit.
    assert!(
        stub_pids().is_empty(),
        "ORPHAN after server exit: {:?}",
        stub_pids()
    );
}

#[test]
fn notifications_get_no_reply() {
    let mut c = Client::spawn();
    let pong = c.result("ping", json!({}));
    assert_eq!(pong, json!({}), "ping replies {{}}");

    // Id-less requests for every request-method must stay silent.
    c.notify_raw("ping", json!({}));
    c.notify_raw("tools/list", json!({}));
    c.notify_raw("tools/call", json!({"name": "snapshot", "arguments": {}}));
    c.notify_raw("initialize", json!({"protocolVersion": "2025-06-18"}));
    match c.rx.recv_timeout(Duration::from_secs(2)) {
        Err(mpsc::RecvTimeoutError::Timeout) => {}
        Ok(line) => panic!("notification got a reply: {line}"),
        Err(e) => panic!("stdout closed unexpectedly: {e}"),
    }

    // Server still alive: a real (id-bearing) ping answers.
    let pong2 = c.result("ping", json!({}));
    assert_eq!(pong2, json!({}), "ping replies {{}}");
    shutdown(c);
}

#[test]
fn evaluate_distinguishes_null_from_undefined() {
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));

    // Explicit JS null -> {"value":null}.
    let null_ev = c.call("evaluate", json!({"expression": "null"}));
    assert!(
        null_ev.get("isError").is_none(),
        "evaluate null ok: {null_ev}"
    );
    let null_text = content_text(&null_ev);
    let null_obj: Value = serde_json::from_str(&null_text).expect("evaluate null json");
    assert_eq!(
        null_obj.get("value"),
        Some(&Value::Null),
        "explicit null preserved: {null_text}"
    );

    // JS undefined (CDP omits `value`) -> explicit tool error, NOT null.
    let undef = c.call("evaluate", json!({"expression": "undefined"}));
    assert!(
        undef.get("isError").is_some(),
        "undefined must be a tool error, not null: {undef}"
    );
    let undef_text = content_text(&undef);
    assert!(
        undef_text.contains("no value"),
        "explicit no-value marker, got: {undef_text}"
    );

    let closed = c.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
    shutdown(c);
}

#[test]
fn snapshot_caps_huge_page_at_source() {
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));

    // ~400KB body: would balloon memory if transferred whole.
    let dir = std::env::temp_dir().join(format!(
        "gladius-snapcap-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&dir).expect("snapcap dir");
    let page = dir.join("big.html");
    let chunk = "LOREM0123456789 ".repeat(2000);
    let body = chunk.repeat(12);
    std::fs::write(
        &page,
        format!(
            "<!doctype html><html><head><title>big</title></head><body><p>SNAPCAP_START</p><p>{body}</p></body></html>"
        ),
    )
    .expect("write big page");
    #[cfg(windows)]
    let ps = page.to_string_lossy().replace('\\', "/");
    #[cfg(not(windows))]
    let ps = page.to_string_lossy().to_string();
    let url = format!("file:///{ps}");

    let nav = c.call("navigate", json!({"url": url}));
    assert!(nav.get("isError").is_none(), "navigate big page: {nav}");

    let t0 = Instant::now();
    let snap = c.call("snapshot", json!({}));
    let dt = t0.elapsed();
    assert!(snap.get("isError").is_none(), "snapshot big page ok");
    let text = content_text(&snap);
    assert!(text.contains("SNAPCAP_START"), "page head present");
    let n = text.chars().count();
    assert!(n <= 8000, "snapshot capped at 8000 chars, got {n}");
    assert!(n > 1000, "snapshot carries real content, got {n}");
    eprintln!("[snapcap] {n} chars in {dt:?}");

    std::fs::remove_dir_all(&dir).ok();
    let closed = c.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
    shutdown(c);
}

#[test]
fn wait_for_hard_deadline() {
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));

    // ms:0 is a no-op success: immediate, even for a missing selector.
    let t0 = Instant::now();
    let z = c.call(
        "wait_for",
        json!({"selector": "#no-such-element-xyz", "ms": 0}),
    );
    let d0 = t0.elapsed();
    assert!(z.get("isError").is_none(), "ms:0 no-op ok: {z}");
    assert!(
        d0 < Duration::from_secs(5),
        "ms:0 returns immediately, took {d0:?}"
    );

    // Missing selector with a 1.5s budget: timeout error AT the budget,
    // far under any per-await CDP timeout.
    let t1 = Instant::now();
    let w = c.call(
        "wait_for",
        json!({"selector": "#no-such-element-xyz", "ms": 1500}),
    );
    let d1 = t1.elapsed();
    assert!(w.get("isError").is_some(), "missing selector isError: {w}");
    assert!(content_text(&w).contains("timeout"), "timeout message: {w}");
    assert!(
        d1 >= Duration::from_millis(1300),
        "budget honored, took {d1:?}"
    );
    assert!(d1 < Duration::from_secs(8), "hard deadline, took {d1:?}");

    // Over-max budget clamps to 15s, never beyond.
    let t2 = Instant::now();
    let w2 = c.call(
        "wait_for",
        json!({"selector": "#no-such-element-xyz", "ms": 60000}),
    );
    let d2 = t2.elapsed();
    assert!(w2.get("isError").is_some(), "clamped timeout isError: {w2}");
    assert!(
        d2 >= Duration::from_secs(13),
        "clamp honors the full 15s, took {d2:?}"
    );
    assert!(d2 < Duration::from_secs(25), "15s clamp cap, took {d2:?}");

    let closed = c.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
    shutdown(c);
}
