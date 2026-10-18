//! Headless-only Chromium assertions on the owned pinned engine: every test
//! drives the real MCP surface against `chrome-headless-shell` (resolved
//! from the versioned cache with no env override) and proves one property of
//! the launch contract. Each test owns its server process(es) and reaps them.
//!
//! - `headless_owned_e2e_navigate_snapshot_screenshot`: initialize lists 21
//!   tools; navigate a local file; snapshot returns a non-empty tree with the
//!   marker; screenshot returns PNG bytes (base64 PNG signature,
//!   non-trivial size); the UA reads `HeadlessChrome/` (the shell artifact).
//! - `headless_json_version_advertises_loopback_ws_for_our_port`: the
//!   server's stderr names the owned port; `GET /json/version` there
//!   advertises a loopback `webSocketDebuggerUrl` for that same port.
//! - `headless_browser_has_no_visible_window`: OS-level (window handles, not
//!   CDP - CDP cannot observe visibility): no `chrome-headless-shell`
//!   process owns a visible top-level window while the instance runs, with
//!   a positive control that such processes exist.
//! - `headless_profile_is_private_and_swept`: the instance owns a private
//!   profile dir under `%TEMP%`, and a clean `close` sweeps it.
//! - `headless_second_instance_runs_concurrently`: two instances with their
//!   own profiles and ports both respond.
//! - `headless_occupied_profile_does_not_disturb_first_browser`: a second
//!   instance pointed at an OCCUPIED profile never disturbs the first (the
//!   shell permits sharing across distinct ports, which is why production
//!   always mints a fresh profile - negative companion).
//! - `headless_console_log_leg_is_live`: a failed subresource fetch surfaces
//!   a `Log.entryAdded`-sourced console entry (new with the Chromium engine).
//! - `headless_headed_request_fails_closed`: `GLADIUS_HEADED=1` refuses the
//!   launch with the headless-only error and launches nothing; a headed CLI
//!   flag exits 2 without starting a browser.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

// ---------------------------------------------------------------- helpers

fn fixture(name: &str) -> String {
    let p = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let p = p.replace(std::path::MAIN_SEPARATOR, "/");
    format!("file:///{p}")
}

struct Client {
    stdin: ChildStdin,
    rx: Receiver<String>,
    next_id: i64,
    child: Child,
    /// stderr lines captured from the server (launch/port/profile notes).
    err_rx: Receiver<String>,
}

impl Client {
    fn spawn_with_env(envs: &[(&str, &str)]) -> Self {
        let bin = env!("CARGO_BIN_EXE_gladius");
        let mut cmd = Command::new(bin);
        cmd.arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Owned-engine resolution (see e2e_mcp.rs): the pinned Chromium
        // resolves from its versioned cache with no env override.
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
        let stderr = child.stderr.take().expect("child stderr");
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
        let (etx, err_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut r = BufReader::new(stderr);
            let mut line = String::new();
            loop {
                line.clear();
                match r.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        let _ = etx.send(line.trim_end().to_string());
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
            err_rx,
        }
    }

    fn spawn() -> Self {
        Self::spawn_with_env(&[])
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
        self.result("tools/call", json!({"name": name, "arguments": args}), timeout)
    }

    /// stderr lines seen so far (non-blocking drain).
    fn stderr_drain(&self) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(l) = self.err_rx.try_recv() {
            out.push(l);
        }
        out
    }

    fn pid(&self) -> u32 {
        self.child.id()
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

/// Graceful `close` then EOF; requires a clean server exit.
fn finish(mut c: Client) {
    let closed = c.call("close", json!({}), Duration::from_secs(30));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
    drop(c.stdin);
    let mut child = c.child;
    // `c.err_rx` borrows nothing; just drop the receiver via Client drop.
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

/// EOF the server without `close` and require a clean exit.
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

/// `(pid, main_window_handle)` for every live `chrome-headless-shell.exe`
/// process (read-only OS query; never kills anything). CDP cannot observe
/// window visibility, so headlessness is asserted here, from the OS.
fn shell_windows() -> Vec<(u32, i64)> {
    let mut q = Command::new("powershell");
    q.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Get-Process -Name 'chrome-headless-shell' -ErrorAction SilentlyContinue | ForEach-Object { \"$($_.Id):$($_.MainWindowHandle)\" }",
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        q.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let out = q.spawn().expect("spawn window query").wait_with_output();
    let out = match out {
        Ok(o) => o,
        Err(_) => return vec![],
    };
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .filter_map(|t| {
            let (a, b) = t.split_once(':')?;
            Some((a.parse::<u32>().ok()?, b.parse::<i64>().ok()?))
        })
        .collect()
}

/// `GET http://127.0.0.1:<port>/json/version` over plain std HTTP (local
/// only). Returns the parsed JSON body.
fn json_version(port: u16) -> Option<Value> {
    let host = format!("127.0.0.1:{port}");
    let req = format!("GET /json/version HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    let mut s = std::net::TcpStream::connect(&host).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    s.set_write_timeout(Some(Duration::from_secs(5))).ok()?;
    s.write_all(req.as_bytes()).ok()?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
        if raw.len() > 1_000_000 {
            break;
        }
    }
    let head_end = raw.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    let body = String::from_utf8_lossy(&raw[head_end..]).into_owned();
    // Chromium may chunk or length-frame; find the JSON object span.
    let start = body.find('{')?;
    let end = body.rfind('}')?;
    serde_json::from_str(&body[start..=end]).ok()
}

fn pinned_chromium_exe() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("GLADIUS_CHROMIUM") {
        if !p.trim().is_empty() {
            return std::path::PathBuf::from(p.trim());
        }
    }
    let root = match std::env::var("GLADIUS_CHROMIUM_CACHE") {
        Ok(v) if !v.trim().is_empty() => std::path::PathBuf::from(v.trim()),
        _ => std::path::PathBuf::from(
            std::env::var("LOCALAPPDATA").expect("LOCALAPPDATA for pinned chromium"),
        )
        .join("gladius")
        .join("chromium"),
    };
    root.join("155.0.8059.84-5ca590a4")
        .join("chrome-headless-shell-win64")
        .join("chrome-headless-shell.exe")
}

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("free port");
    let p = l.local_addr().expect("local addr").port();
    drop(l);
    p
}

// ---------------------------------------------------------------- tests

#[test]
fn headless_owned_e2e_navigate_snapshot_screenshot() {
    let mut c = Client::spawn();
    let init = c.result(
        "initialize",
        json!({"protocolVersion": "2025-06-18"}),
        Duration::from_secs(30),
    );
    assert_eq!(
        init.get("serverInfo").and_then(|s| s.get("backend")),
        Some(&json!("chromium")),
        "owned backend reports chromium: {init}"
    );
    let listed = c.result("tools/list", json!({}), Duration::from_secs(30));
    let names: Vec<String> = listed
        .get("tools")
        .and_then(|t| t.as_array())
        .expect("tools array")
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
        .collect();
    assert_eq!(names.len(), 21, "exactly the 21 tools: {names:?}");

    let nav = c.call(
        "navigate",
        json!({"url": fixture("fixture.html")}),
        Duration::from_secs(60),
    );
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    let snap = c.call("snapshot", json!({}), Duration::from_secs(30));
    assert!(snap.get("isError").is_none(), "snapshot ok: {snap}");
    let snap_text = content_text(&snap);
    assert!(
        !snap_text.trim().is_empty() && snap_text.contains("GLADIUS_MARKER_hello-12345"),
        "snapshot returns a non-empty tree with the marker"
    );

    // The shell artifact proves itself in the UA string.
    let ua = content_text(&c.call(
        "evaluate",
        json!({"expression": "navigator.userAgent"}),
        Duration::from_secs(30),
    ));
    assert!(
        ua.contains("HeadlessChrome/"),
        "shell token in the UA string, got: {ua}"
    );

    let shot = c.call("screenshot", json!({}), Duration::from_secs(60));
    assert!(shot.get("isError").is_none(), "screenshot ok");
    let img = shot.get("content").and_then(|cc| cc.get(0)).expect("image block");
    assert_eq!(
        img.get("type").and_then(|t| t.as_str()),
        Some("image"),
        "block type image"
    );
    assert_eq!(
        img.get("mimeType").and_then(|m| m.as_str()),
        Some("image/png"),
        "mime png"
    );
    let data = img.get("data").and_then(|d| d.as_str()).unwrap_or("");
    assert!(
        data.starts_with("iVBORw0KGgo"),
        "valid PNG signature (base64 magic): {}",
        data.chars().take(24).collect::<String>()
    );
    assert!(
        data.len() > 1000,
        "non-trivial PNG size: base64 len={}",
        data.len()
    );
    eprintln!(
        "[headless-e2e] png base64 len={} (~{} bytes)",
        data.len(),
        data.len() * 3 / 4
    );
    finish(c);
}

#[test]
fn headless_json_version_advertises_loopback_ws_for_our_port() {
    let mut c = Client::spawn();
    c.result(
        "initialize",
        json!({"protocolVersion": "2025-06-18"}),
        Duration::from_secs(30),
    );
    let nav = c.call(
        "navigate",
        json!({"url": fixture("fixture.html")}),
        Duration::from_secs(60),
    );
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    // The server logs its owned port at launch; find it on stderr.
    let mut port: Option<u16> = None;
    for _ in 0..50 {
        for l in c.stderr_drain() {
            // "[gladius] backend: chromium <path> (port <N>)"
            if let Some(i) = l.rfind("(port ") {
                let rest = &l[i + 6..];
                if let Some(end) = rest.find(')') {
                    if let Ok(p) = rest[..end].parse::<u16>() {
                        port = Some(p);
                    }
                }
            }
        }
        if port.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let port = port.expect("owned chromium port on server stderr");
    let v = json_version(port).expect("/json/version answers on the owned port");
    let ws = v
        .get("webSocketDebuggerUrl")
        .and_then(|u| u.as_str())
        .unwrap_or("");
    assert!(
        ws.starts_with("ws://127.0.0.1:") || ws.starts_with("ws://localhost:"),
        "loopback ws url, got: {ws}"
    );
    assert!(
        ws.contains(&format!(":{port}/")) || ws.ends_with(&format!(":{port}")),
        "ws url carries OUR port {port}, got: {ws}"
    );
    eprintln!("[headless-ws] port={port} ws={ws}");
    finish(c);
}

#[test]
fn headless_browser_has_no_visible_window() {
    let mut c = Client::spawn();
    c.result(
        "initialize",
        json!({"protocolVersion": "2025-06-18"}),
        Duration::from_secs(30),
    );
    let nav = c.call(
        "navigate",
        json!({"url": fixture("fixture.html")}),
        Duration::from_secs(60),
    );
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    // OS-level assertion (CDP cannot observe window visibility): poll until
    // the shell processes are visible to the OS, then require zero visible
    // top-level windows. The non-empty process list is the positive control
    // (an empty list would prove nothing).
    let mut wins = Vec::new();
    for _ in 0..50 {
        wins = shell_windows();
        if !wins.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(
        !wins.is_empty(),
        "positive control: chrome-headless-shell processes exist while the instance runs"
    );
    let visible: Vec<u32> = wins.iter().filter(|(_, h)| *h != 0).map(|(p, _)| *p).collect();
    assert!(
        visible.is_empty(),
        "no visible top-level window for the instance (pid:handle): {wins:?}"
    );
    eprintln!("[headless-window] shell procs={} all handle=0", wins.len());
    finish(c);
}

#[test]
fn headless_profile_is_private_and_swept() {
    let mut c = Client::spawn();
    let spid = c.pid();
    c.result(
        "initialize",
        json!({"protocolVersion": "2025-06-18"}),
        Duration::from_secs(30),
    );
    let nav = c.call(
        "navigate",
        json!({"url": fixture("fixture.html")}),
        Duration::from_secs(60),
    );
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    // Private to the instance: exactly our server pid's dir(s), under TEMP.
    let dirs = profile_dirs(spid);
    assert!(
        !dirs.is_empty(),
        "instance owns a private profile dir (server pid {spid})"
    );
    for d in &dirs {
        assert!(d.is_dir(), "profile dir exists during the run: {}", d.display());
    }
    eprintln!("[headless-profile] private dirs: {dirs:?}");

    // A clean close sweeps it: gone after the tree is reaped.
    let closed = c.call("close", json!({}), Duration::from_secs(30));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
    let mut gone = false;
    for _ in 0..50 {
        if profile_dirs(spid).is_empty() {
            gone = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(gone, "profile dir GONE after a clean close");
    shutdown(c);
}

#[test]
fn headless_second_instance_runs_concurrently() {
    let mut a = Client::spawn();
    let mut b = Client::spawn();
    for (label, c) in [("A", &mut a), ("B", &mut b)] {
        c.result(
            "initialize",
            json!({"protocolVersion": "2025-06-18"}),
            Duration::from_secs(30),
        );
        let _ = label;
    }
    let na = a.call(
        "navigate",
        json!({"url": fixture("fixture.html")}),
        Duration::from_secs(60),
    );
    assert!(na.get("isError").is_none(), "A navigate ok: {na}");
    let nb = b.call(
        "navigate",
        json!({"url": fixture("dialog.html")}),
        Duration::from_secs(60),
    );
    assert!(nb.get("isError").is_none(), "B navigate ok: {nb}");

    // Distinct private profiles for the two instances.
    let da = profile_dirs(a.pid());
    let db = profile_dirs(b.pid());
    assert!(!da.is_empty() && !db.is_empty(), "both own profiles");
    assert_ne!(da, db, "profiles are private per instance: {da:?} vs {db:?}");

    // Both respond over their own CDP sessions.
    let sa = content_text(&a.call("snapshot", json!({}), Duration::from_secs(30)));
    let sb = content_text(&b.call("snapshot", json!({}), Duration::from_secs(30)));
    assert!(sa.contains("GLADIUS_MARKER_hello-12345"), "A responds: {sa:.120}");
    assert!(sb.contains("DIALOG_FIXTURE_MARKER"), "B responds");
    finish(a);
    finish(b);
}

#[test]
fn headless_occupied_profile_does_not_disturb_first_browser() {
    // MEASURED (this build, this machine): two `chrome-headless-shell`
    // processes CAN share one profile dir when their debug ports differ -
    // each binds its own port and answers `/json/version` independently.
    // The shell does NOT enforce profile exclusivity across distinct ports,
    // which is exactly why production always mints a FRESH unique profile
    // per instance (never relying on a lock the shell does not enforce).
    // This negative companion therefore proves the fail-safe direction:
    // a second instance pointed at an OCCUPIED profile never disturbs the
    // first browser - the first endpoint keeps answering and keeps driving
    // exactly its own targets (no hijack in either direction).
    let exe = pinned_chromium_exe();
    assert!(
        exe.is_file(),
        "pinned chromium missing: {}",
        exe.display()
    );
    let profile = std::env::temp_dir().join(format!(
        "gladius-occupied-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&profile).expect("occupied profile dir");

    fn launch_shell(exe: &std::path::Path, profile: &std::path::Path, port: u16) -> Child {
        let mut cmd = Command::new(exe);
        cmd.args([
            "--headless",
            &format!("--remote-debugging-port={port}"),
            &format!("--user-data-dir={}", profile.display()),
            "--no-first-run",
            "--no-default-browser-check",
            &format!("--remote-allow-origins=http://127.0.0.1:{port}"),
            "about:blank",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        cmd.spawn().expect("spawn shell")
    }

    fn stop(mut c: Child) {
        c.kill().ok();
        let _ = c.wait();
    }

    // First instance takes the profile; wait until it is controllable and
    // record its identity (browser ws URL + page target ids).
    let port_a = free_port();
    let mut first = launch_shell(&exe, &profile, port_a);
    let t0 = Instant::now();
    let ver_a = loop {
        if let Some(v) = json_version(port_a) {
            break v;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(30),
            "first instance never went ready on {port_a}"
        );
        assert!(
            first.try_wait().expect("try_wait first").is_none(),
            "first instance died during startup"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    let ws_a = ver_a
        .get("webSocketDebuggerUrl")
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();
    assert!(!ws_a.is_empty(), "first has a ws url");

    // Second instance on the SAME profile with its own port: observe what
    // happens (up to 15s), then put BOTH shells down BEFORE asserting, so
    // no orphan survives any outcome.
    let port_b = free_port();
    let mut second = launch_shell(&exe, &profile, port_b);
    let t1 = Instant::now();
    let mut ver_b: Option<Value> = None;
    let second_exited = loop {
        if let Some(status) = second.try_wait().expect("try_wait second") {
            eprintln!("[occupied-profile] second exited: {status}");
            break true;
        }
        if let Some(v) = json_version(port_b) {
            ver_b = Some(v);
            break false;
        }
        if t1.elapsed() >= Duration::from_secs(15) {
            break false;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    // First endpoint state while the second is (or was) around.
    let ver_a_after = json_version(port_a);
    stop(second);
    stop(first);
    std::fs::remove_dir_all(&profile).ok();

    // The first browser is undisturbed: same ws URL still advertised...
    let ver_a_after = ver_a_after.expect("first endpoint answers with second around");
    assert_eq!(
        ver_a_after
            .get("webSocketDebuggerUrl")
            .and_then(|u| u.as_str())
            .unwrap_or(""),
        ws_a,
        "first keeps its own endpoint (no hijack)"
    );
    // ...and it is a DIFFERENT browser from any second that came up (the
    // shell permits sharing, so production never shares: fresh profile per
    // instance, always).
    if !second_exited {
        let vb = ver_b.expect("second endpoint answered");
        let ws_b = vb
            .get("webSocketDebuggerUrl")
            .and_then(|u| u.as_str())
            .unwrap_or("");
        assert!(
            !ws_b.is_empty() && ws_b != ws_a,
            "second is a distinct browser, first untouched: A={ws_a} B={ws_b}"
        );
        eprintln!("[occupied-profile] sharing permitted by shell; first undisturbed A={ws_a} B={ws_b}");
    }
}

#[test]
fn headless_console_log_leg_is_live() {
    let mut c = Client::spawn();
    c.result(
        "initialize",
        json!({"protocolVersion": "2025-06-18"}),
        Duration::from_secs(30),
    );
    let nav = c.call(
        "navigate",
        json!({"url": fixture("fixture.html")}),
        Duration::from_secs(60),
    );
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");

    // A fetch to a refused loopback port fails at the network layer:
    // Chromium reports it through `Log.entryAdded` ("Failed to load
    // resource"), the leg the old engine never emitted.
    let port = free_port();
    let fired = content_text(&c.call(
        "evaluate",
        json!({"expression": format!("fetch('http://127.0.0.1:{port}/log-probe-xyz',{{cache:'no-store'}}).catch(()=>{{window.__gladiusLogProbe='done';}}); 'fired'")}),
        Duration::from_secs(30),
    ));
    assert!(fired.contains("fired"), "probe fired: {fired}");
    let mut seen = String::new();
    for _ in 0..25 {
        std::thread::sleep(Duration::from_millis(200));
        seen = content_text(&c.call("console", json!({}), Duration::from_secs(30)));
        if seen.contains("Failed to load resource") || seen.contains("log-probe-xyz") {
            break;
        }
    }
    assert!(
        seen.contains("Failed to load resource") || seen.contains("log-probe-xyz"),
        "Log leg is live (failed-resource entry buffered): {seen:.300}"
    );
    finish(c);
}

#[test]
fn headless_headed_request_fails_closed() {
    // A headed request launches NOTHING: the tool error is the headless-only
    // refusal and no profile dir ever appears.
    let mut c = Client::spawn_with_env(&[("GLADIUS_HEADED", "1")]);
    let spid = c.pid();
    c.result(
        "initialize",
        json!({"protocolVersion": "2025-06-18"}),
        Duration::from_secs(30),
    );
    let nav = c.call(
        "navigate",
        json!({"url": fixture("fixture.html")}),
        Duration::from_secs(60),
    );
    assert!(nav.get("isError").is_some(), "headed navigate refuses: {nav}");
    let t = content_text(&nav);
    assert!(
        t.contains("headless-only") || t.contains("not supported"),
        "explicit unsupported error, got: {t}"
    );
    assert!(
        profile_dirs(spid).is_empty(),
        "headed refusal launched nothing: {:?}",
        profile_dirs(spid)
    );
    shutdown(c);

    // The headed CLI flag exits 2 without starting a browser.
    let bin = env!("CARGO_BIN_EXE_gladius");
    let mut cmd = Command::new(bin);
    cmd.arg("mcp")
        .arg("--headed")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().expect("spawn headed cli");
    let mut code: Option<i32> = None;
    for _ in 0..50 {
        if let Some(status) = child.try_wait().expect("try_wait headed cli") {
            code = status.code();
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if code.is_none() {
        child.kill().ok();
        panic!("headed CLI flag did not fail closed (still running)");
    }
    assert_eq!(code, Some(2), "headed CLI exits 2");
}
