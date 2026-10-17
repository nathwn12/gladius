//! Hard-kill reaping (G-4) on the owned Chromium engine: when the MCP server
//! process dies hard (`TerminateProcess`, no `Drop`/`kill_tree`), the OS job
//! object must reap the browser tree. The owned launch owns one fresh private
//! profile dir (`%TEMP%/gladius-<pid>-*`); graceful exit sweeps it, and a
//! later launch sweeps a hard-killed server's stale dir. Only processes
//! spawned by this test are ever killed.

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
        // Owned-engine resolution (see e2e_mcp.rs): the pinned Chromium
        // resolves from its versioned cache with no env override.
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
/// The owned Chromium launch creates exactly one per instance; graceful
/// close sweeps it.
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

/// `(pid, ppid)` for every live owned-browser process
/// (`chrome-headless-shell.exe`; read-only WMI query; never kills anything).
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

/// True while a process with this pid exists (read-only WMI probe; never
/// kills anything).
fn process_alive(pid: u32) -> bool {
    let mut q = Command::new("powershell");
    q.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &format!(
            "Get-CimInstance Win32_Process -Filter \"ProcessId={pid}\" | ForEach-Object {{ $_.ProcessId }}"
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
    let out = q.spawn().expect("spawn liveness query").wait_with_output();
    match out {
        Ok(o) => !String::from_utf8_lossy(&o.stdout).trim().is_empty(),
        Err(_) => true, // any doubt keeps waiting, never declares dead
    }
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
fn hard_kill_reaps_owned_browser_tree_and_sweeps_stale_profile() {
    // --- server A: launch the owned browser via one navigate ---
    let mut a = Client::spawn();
    let apid = a.child.id();
    a.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let nav = a.call("navigate", json!({"url": fixture_url()}));
    assert!(nav.get("isError").is_none(), "A navigate ok: {nav}");

    // The owned launch owns one fresh private profile dir for this instance.
    assert!(
        !profile_dirs(apid).is_empty(),
        "owned launch owns a private profile dir for server A (pid {apid})"
    );

    // The browser tree is up: a chrome-headless-shell.exe process parented
    // under A.
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
        "owned browser running under server A (pid {apid}) before kill"
    );

    // --- hard kill ONLY server A: TerminateProcess, no /T, no Drop ---
    // Then release EVERY handle to A (including the process handle
    // itself): the job object reaps the tree once the last handle closes.
    // Graceful profile cleanup never runs here, so A's profile dir survives
    // the kill as stale debris until the next owned launch sweeps it.
    a.child.kill().expect("hard-kill server A");
    let _ = a.child.wait();
    drop(a);

    // --- the job object reaps the tree: A's browser pids die ---
    let mut deadline = 30;
    while deadline > 0 && kids.iter().any(|p| process_alive(*p)) {
        std::thread::sleep(Duration::from_millis(500));
        deadline -= 1;
    }
    let survivors: Vec<u32> = kids.into_iter().filter(|p| process_alive(*p)).collect();
    assert!(
        survivors.is_empty(),
        "ORPHAN: owned browser survived the hard kill of server A: {survivors:?}"
    );

    // Server B: happy path works on a fresh owned launch, and B's launch
    // sweeps A's stale profile (the recovery path for hard-killed servers).
    let mut b = Client::spawn();
    b.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let navb = b.call("navigate", json!({"url": fixture_url()}));
    assert!(navb.get("isError").is_none(), "B navigate ok: {navb}");
    assert!(
        profile_dirs(apid).is_empty(),
        "B's launch swept A's stale profile: {:?}",
        profile_dirs(apid)
    );
    let snap = b.call("snapshot", json!({}));
    assert!(
        content_text(&snap).contains("GLADIUS_MARKER_hello-12345"),
        "happy path on B: {}",
        content_text(&snap).chars().take(200).collect::<String>()
    );
    let bpid = b.child.id();
    assert!(
        !profile_dirs(bpid).is_empty(),
        "B owns its own private profile dir: {:?}",
        profile_dirs(bpid)
    );

    // Graceful close kills B's browser tree AND sweeps B's profile.
    let bkids: Vec<u32> = engine_parentage()
        .iter()
        .filter(|(_, ppid)| *ppid == bpid)
        .map(|(pid, _)| *pid)
        .collect();
    assert!(!bkids.is_empty(), "B owns a browser before close");
    let closed = b.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
    shutdown(b);
    let mut deadline = 20;
    while deadline > 0 && bkids.iter().any(|p| process_alive(*p)) {
        std::thread::sleep(Duration::from_millis(500));
        deadline -= 1;
    }
    let left: Vec<u32> = bkids.into_iter().filter(|p| process_alive(*p)).collect();
    assert!(
        left.is_empty(),
        "graceful close reaped B's browser tree: {left:?}"
    );
    assert!(
        profile_dirs(bpid).is_empty(),
        "graceful close swept B's profile dir: {:?}",
        profile_dirs(bpid)
    );
}
