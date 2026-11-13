//! S2.1: a dead owned engine is recoverable, and says how (SPEC.md §6).
//!
//! Kill the owned `chrome-headless-shell` tree under a live server: the call
//! that finds the engine dead relaunches the *owned* engine **once**, lazily,
//! inside that call - and is **not** silently retried against the replacement.
//! It returns the typed error for what it was doing (`engine_dead`, never a
//! bare transport prose failure in `code:"error"`); the *following* call works
//! on the fresh engine. A failed relaunch spends the death's single attempt:
//! the next call cannot launch again, and only an explicit `close` re-arms it.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{json, Value};

/// One line per *relaunch* attempt (never a cold start): the launch count this
/// test asserts against.
const RELAUNCH_LINE: &str = "relaunching the owned engine";

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
    /// Server STDERR (the launch counter lives there).
    log: PathBuf,
}

impl Client {
    fn spawn() -> Self {
        let log =
            std::env::temp_dir().join(format!("gladius-fix-a-recovery-{}.log", std::process::id()));
        let stderr = std::fs::File::create(&log).expect("create stderr log");
        let bin = env!("CARGO_BIN_EXE_gladius");
        let mut cmd = Command::new(bin);
        cmd.arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(stderr));
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
            log,
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

    /// Relaunch attempts this server has logged so far. Read after each call
    /// that could have launched, so a second attempt is caught immediately.
    fn relaunch_attempts(&self) -> usize {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(RELAUNCH_LINE))
            .count()
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

fn error_code(v: &Value) -> String {
    v.pointer("/structuredContent/code")
        .and_then(|c| c.as_str())
        .unwrap_or("<none>")
        .to_string()
}

fn error_reason(v: &Value) -> String {
    v.pointer("/structuredContent/reason")
        .and_then(|c| c.as_str())
        .unwrap_or("<none>")
        .to_string()
}

/// `(pid, ppid)` for every live owned-browser process
/// (`chrome-headless-shell.exe`; read-only WMI query).
fn engine_parentage() -> Vec<(u32, u32)> {
    let mut q = Command::new("powershell");
    q.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Get-CimInstance Win32_Process -Filter \"Name='chrome-headless-shell.exe'\" | ForEach-Object { \"$($_.ProcessId):$($_.ParentProcessId)\" }",
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        q.creation_flags(0x08000000);
    }
    let out = q.spawn().expect("spawn engine query").wait_with_output();
    let out = match out {
        Ok(o) => o,
        Err(_) => return vec![],
    };
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .filter_map(|t| {
            let (a, b) = t.split_once(':')?;
            Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?))
        })
        .collect()
}

/// Kill one process tree (only ever pids this test observed as its own
/// server's children).
fn kill_tree(pid: u32) {
    let mut cmd = Command::new("taskkill");
    cmd.args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let _ = cmd.spawn().and_then(|mut c| c.wait());
}

#[test]
fn dead_engine_respawns_once_then_types_the_recovery() {
    let mut a = Client::spawn();
    let apid = a.child.id();
    a.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let nav = a.call("navigate", json!({"url": fixture_url()}));
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");
    // A cold start is not a relaunch: the counter starts at zero.
    assert_eq!(
        a.relaunch_attempts(),
        0,
        "the first launch is a cold start, not a relaunch"
    );

    // The owned engine tree under this server.
    let mut kids: Vec<u32> = engine_parentage()
        .iter()
        .filter(|(_, ppid)| *ppid == apid)
        .map(|(pid, _)| *pid)
        .collect();
    for _ in 0..20 {
        if !kids.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
        kids = engine_parentage()
            .iter()
            .filter(|(_, ppid)| *ppid == apid)
            .map(|(pid, _)| *pid)
            .collect();
    }
    assert!(
        !kids.is_empty(),
        "owned engine running under server (pid {apid}) before the kill"
    );
    for pid in &kids {
        kill_tree(*pid);
    }

    // The first call that FINDS the engine dead relaunches it once and returns
    // typed for what it was doing - the replacement never executes that call.
    // (The call that first *observes* the death in flight is the one exception
    // the spec allows: it already failed typed and is not resent, so the
    // relaunch lands on the next call.)
    let mut answers: Vec<String> = Vec::new();
    let mut attempts = 0;
    let mut reason = String::new();
    for _ in 0..6 {
        let s = a.call("snapshot", json!({}));
        attempts = a.relaunch_attempts();
        let prose = content_text(&s);
        assert!(
            s.get("isError").is_some(),
            "the call that finds the engine dead is NOT executed against the \
             replacement (that would be a silent retry): {s}"
        );
        assert_eq!(
            error_code(&s),
            "engine_dead",
            "a dead engine is typed, never prose-only: {s}"
        );
        reason = error_reason(&s);
        assert!(
            reason == "respawn_failed" || reason == "transport",
            "engine_dead reason is from the closed set: {s}"
        );
        if reason == "respawn_failed" {
            assert!(
                prose.contains("close"),
                "the respawn_failed prose names the recovery (`close`): {prose}"
            );
        }
        assert!(
            attempts <= 1,
            "one death gets at most one relaunch attempt; got {attempts} after: {answers:?} / {prose}"
        );
        answers.push(format!("{reason}: {prose}"));
        if attempts == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    assert_eq!(
        attempts, 1,
        "the death got exactly one relaunch attempt; typed answers seen: {answers:?}"
    );

    if reason == "respawn_failed" {
        // The single attempt failed: the next call must NOT launch again.
        let s = a.call("snapshot", json!({}));
        assert!(
            s.get("isError").is_some(),
            "a failed relaunch is still typed on the next call: {s}"
        );
        assert_eq!(error_reason(&s), "respawn_failed", "{s}");
        assert_eq!(
            a.relaunch_attempts(),
            1,
            "no second relaunch attempt after a failed one"
        );
        // The prose's own recovery: `close`, then the next call launches.
        let closed = a.call("close", json!({}));
        assert!(closed.get("isError").is_none(), "close ok: {closed}");
        let nav2 = a.call("navigate", json!({"url": fixture_url()}));
        assert!(
            nav2.get("isError").is_none(),
            "close then retry launches a fresh engine: {nav2}"
        );
        assert_eq!(
            a.relaunch_attempts(),
            1,
            "an explicit close re-arms recovery; the fresh launch is a cold start"
        );
    } else {
        // The single attempt succeeded: the FOLLOWING call works on it.
        let nav2 = a.call("navigate", json!({"url": fixture_url()}));
        assert!(
            nav2.get("isError").is_none(),
            "the following call works on the relaunched engine: {nav2}"
        );
        assert_eq!(
            a.relaunch_attempts(),
            1,
            "the recovered engine serves calls without another relaunch"
        );
    }

    // The engine serving calls now is real: it renders the page.
    let snap = a.call("snapshot", json!({}));
    assert!(
        content_text(&snap).contains("GLADIUS_MARKER_hello-12345"),
        "the engine renders the page: {}",
        content_text(&snap).chars().take(200).collect::<String>()
    );

    // Graceful close still succeeds after the recovery.
    let closed = a.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
}
