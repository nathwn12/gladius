//! Hard-kill reaping (G-4): when the MCP server process dies hard
//! (`TerminateProcess`, no `Drop`/`kill_tree`), the OS job object must reap
//! the browser tree, and the next launch must sweep the orphaned profile
//! dir. Only processes spawned by this test are ever killed.

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
            .stderr(Stdio::null());
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
}

fn content_text(v: &Value) -> String {
    v.get("content")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string()
}

/// Profile dirs owned by one gladius pid: `%TEMP%/gladius-<pid>-*`.
fn profile_dirs(owner_pid: u32) -> Vec<std::path::PathBuf> {
    let prefix = format!("gladius-{owner_pid}-");
    std::fs::read_dir(std::env::temp_dir())
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with(prefix.as_str()))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default()
}

/// PIDs of live browser processes (chrome-headless-shell.exe or chrome.exe) whose command line mentions
/// `token` (read-only WMI query; never kills anything). Excludes the query
/// process itself by construction (powershell.exe, not chrome).
fn chrome_pids_with(token: &str) -> Vec<u32> {
    let mut q = Command::new("powershell");
    q.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &format!(
            "Get-CimInstance Win32_Process -Filter \"(Name='chrome-headless-shell.exe' OR Name='chrome.exe')\" | Where-Object {{ $_.CommandLine -like '*{token}*' }} | ForEach-Object {{ $_.ProcessId }}"
        ),
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        q.creation_flags(0x08000000);
    }
    let out = q.spawn().expect("spawn chrome query").wait_with_output();
    let out = match out {
        Ok(o) => o,
        Err(_) => return vec![],
    };
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .filter_map(|t| t.parse::<u32>().ok())
        .collect()
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

#[test]
fn hard_kill_reaps_browser_tree_and_next_launch_sweeps_profile() {
    // --- server A: launch a real browser via one navigate ---
    let mut a = Client::spawn();
    let apid = a.child.id();
    a.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let nav = a.call("navigate", json!({"url": fixture_url()}));
    assert!(nav.get("isError").is_none(), "A navigate ok: {nav}");

    let dirs = profile_dirs(apid);
    assert_eq!(
        dirs.len(),
        1,
        "exactly one profile dir for server A (pid {apid}): {dirs:?}"
    );
    let token = dirs[0]
        .file_name()
        .expect("profile dir name")
        .to_string_lossy()
        .into_owned();
    assert!(
        dirs[0].is_dir(),
        "A profile dir exists before the kill: {dirs:?}"
    );

    // The browser tree is up and carries our profile token.
    let mut chrome = chrome_pids_with(&token);
    for _ in 0..20 {
        if !chrome.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
        chrome = chrome_pids_with(&token);
    }
    assert!(
        !chrome.is_empty(),
        "browser tree running under A's profile before kill"
    );

    // --- hard kill ONLY server A: TerminateProcess, no /T, no Drop ---
    // Then release EVERY handle to A (including the process handle
    // itself): a dead-but-unreaped pid still answers OpenProcess, and the
    // sweep conservatively treats that as alive. Real orphans (parent
    // gone, handles closed) read as dead.
    a.child.kill().expect("hard-kill server A");
    let _ = a.child.wait();
    drop(a);

    // --- (a) the job object reaps the tree: no chrome with our token ---
    let mut survivors = chrome_pids_with(&token);
    for _ in 0..30 {
        if survivors.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
        survivors = chrome_pids_with(&token);
    }
    assert!(
        survivors.is_empty(),
        "ORPHAN: chrome survived the hard kill of server A: {survivors:?}"
    );

    // Nothing was swept yet (sweep runs at next launch): honest checkpoint.
    assert!(
        dirs[0].is_dir(),
        "orphaned profile dir still on disk until next launch"
    );

    // --- server B: happy path works AND its launch sweeps A's profile ---
    let mut b = Client::spawn();
    b.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let navb = b.call("navigate", json!({"url": fixture_url()}));
    assert!(navb.get("isError").is_none(), "B navigate ok: {navb}");
    let snap = b.call("snapshot", json!({}));
    assert!(
        content_text(&snap).contains("GLADIUS_MARKER_hello-12345"),
        "happy path on B: {}",
        content_text(&snap).chars().take(200).collect::<String>()
    );
    assert!(
        !dirs[0].exists(),
        "B's launch swept A's orphaned profile dir: {dirs:?}"
    );

    // Graceful close still removes B's own profile dir.
    let bpid = b.child.id();
    let bdirs = profile_dirs(bpid);
    assert_eq!(bdirs.len(), 1, "one profile dir for B: {bdirs:?}");
    let closed = b.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
    shutdown(b);
    for _ in 0..20 {
        if !bdirs[0].exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    assert!(
        !bdirs[0].exists(),
        "graceful close removed B's profile dir: {bdirs:?}"
    );
    assert!(
        chrome_pids_with(
            &bdirs[0]
                .file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned()
        )
        .is_empty(),
        "no chrome left for B's profile after close"
    );
}
