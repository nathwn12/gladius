//! External-CDP attach path: gladius drives an engine it did not launch.
//!
//! - `GLADIUS_CDP_URL` (ws:// or http://) attaches instead of launching.
//! - Ownership: `close` detaches and never kills the external engine, never
//!   sweeps a profile it did not create, and launches nothing of its own.
//! - `GLADIUS_BACKEND_CAPS=dom-only` refuses `screenshot` (no rendering
//!   pipeline) while DOM tools keep working; `upload` needs no rendering
//!   (DOM CDP only) so it keeps working too - proved live here.
//! - Default path (neither var set) is the owned Obscura launch: covered by
//!   `e2e_mcp` plus the `initialize_reports_default_backend` fast check below
//!   (no browser).

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

// ---------------------------------------------------------------- helpers

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

/// Resolve the headless-shell binary exactly like the server does:
/// `GLADIUS_CHROME` wins, else the newest `chrome-headless-shell.exe` under
/// `%LOCALAPPDATA%\ms-playwright`. No system-browser fallback, ever.
fn find_shell() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("GLADIUS_CHROME") {
        if !p.trim().is_empty() {
            return std::path::PathBuf::from(p);
        }
    }
    let local = std::env::var("LOCALAPPDATA").expect("LOCALAPPDATA");
    let base = std::path::PathBuf::from(local).join("ms-playwright");
    let mut cands: Vec<std::path::PathBuf> = Vec::new();
    for e in std::fs::read_dir(&base)
        .expect("read ms-playwright")
        .flatten()
    {
        let p = e
            .path()
            .join("chrome-headless-shell-win64")
            .join("chrome-headless-shell.exe");
        if p.is_file() {
            cands.push(p);
        }
    }
    cands.sort();
    cands
        .pop()
        .expect("no chrome-headless-shell under ms-playwright")
}

/// Externally started engine. Owned by the TEST (Drop kills it); gladius
/// must never touch it.
struct External {
    child: Child,
    pid: u32,
    port: u16,
    dir: std::path::PathBuf,
}

impl External {
    fn start() -> Self {
        let shell = find_shell();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("free port");
        let port = listener.local_addr().expect("local addr").port();
        drop(listener);
        let dir = std::env::temp_dir().join(format!(
            "gladius-ext-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&dir).expect("external profile dir");
        let mut cmd = Command::new(&shell);
        let err_log = dir.join("chrome-stderr.log");
        let err_file = std::fs::File::create(&err_log).expect("stderr log");
        cmd.args([
            "--headless",
            "--disable-gpu",
            "--no-first-run",
            "--no-default-browser-check",
            &format!("--remote-debugging-port={port}"),
            "--disable-popup-blocking",
            &format!("--user-data-dir={}", dir.display()),
            "about:blank",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(err_file);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let child = cmd.spawn().expect("spawn external chrome-headless-shell");
        let pid = child.id();
        let mut ext = External {
            child,
            pid,
            port,
            dir,
        };
        ext.wait_ready();
        ext
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Block until `/json/version` answers (max ~30s), proving the external
    /// engine is up independently of gladius.
    fn wait_ready(&mut self) {
        let t0 = Instant::now();
        let mut n = 0;
        loop {
            n += 1;
            match version_ws_dbg(&self.base_url()) {
                Ok(_) => return,
                Err(e) => {
                    // Chrome is usually up in <2s; log only when a wait
                    // drags, so happy runs stay quiet.
                    if t0.elapsed() > Duration::from_secs(5) && n % 20 == 1 {
                        eprintln!(
                            "[attach-test] wait_ready pid={} port={} attempt={n} err={e}",
                            self.pid, self.port
                        );
                    }
                }
            }
            assert!(
                t0.elapsed() < Duration::from_secs(30),
                "external engine never became ready"
            );
            assert!(
                self.child.try_wait().expect("try_wait external").is_none(),
                "external engine died during startup"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn alive(&mut self) -> bool {
        self.child.try_wait().expect("try_wait").is_none() && version_ws(&self.base_url()).is_some()
    }
}

impl Drop for External {
    fn drop(&mut self) {
        self.child.kill().ok();
        let _ = self.child.wait();
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

/// GET `<base>/json/version` over plain std HTTP (local only). Returns the
/// `webSocketDebuggerUrl` when the engine answers.
fn version_ws(base: &str) -> Option<String> {
    version_ws_dbg(base).ok()
}

fn version_ws_dbg(base: &str) -> Result<String, String> {
    let host = base
        .strip_prefix("http://")
        .unwrap_or(base)
        .trim_end_matches('/');
    // Chrome builds `webSocketDebuggerUrl` from the Host header: it must
    // carry the port, or the returned ws url has none (measured).
    let req = format!("GET /json/version HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    let mut s = std::net::TcpStream::connect(host).map_err(|e| format!("connect {host}: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| format!("read timeout: {e}"))?;
    s.set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| format!("write timeout: {e}"))?;
    // NOTE: chrome-headless-shell's DevTools server ignores HTTP/1.0 (it
    // closes with zero bytes; measured on 155) - HTTP/1.1 is required. And
    // it keeps HTTP/1.1 connections open past the reply despite
    // `Connection: close`, so the body MUST be framed by Content-Length -
    // read-to-end would wait for a close that never comes.
    s.write_all(req.as_bytes())
        .map_err(|e| format!("write: {e}"))?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    // Headers first.
    loop {
        let n = s.read(&mut buf).map_err(|e| format!("read head: {e}"))?;
        if n == 0 {
            return Err("eof in headers".to_string());
        }
        raw.extend_from_slice(&buf[..n]);
        if raw.windows(4).any(|w| w == b"\r\n\r\n") || raw.len() > 65536 {
            break;
        }
    }
    let head_end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("no header end")?
        + 4;
    let head = String::from_utf8_lossy(&raw[..head_end]).into_owned();
    if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
        return Err(format!("status: {}", head.lines().next().unwrap_or("")));
    }
    let len: usize = head
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            if k.trim().eq_ignore_ascii_case("content-length") {
                v.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .ok_or("no content-length")?;
    if len > 1_000_000 {
        return Err(format!("absurd content-length {len}"));
    }
    while raw.len() < head_end + len {
        let n = s.read(&mut buf).map_err(|e| format!("read body: {e}"))?;
        if n == 0 {
            return Err("eof in body".to_string());
        }
        raw.extend_from_slice(&buf[..n]);
    }
    let body = String::from_utf8_lossy(&raw[head_end..head_end + len]);
    let v: Value = serde_json::from_str(&body).map_err(|e| format!("json: {e} body={body:?}"))?;
    v.get("webSocketDebuggerUrl")
        .and_then(|u| u.as_str())
        .map(|u| u.to_string())
        .ok_or_else(|| "no webSocketDebuggerUrl".to_string())
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

/// `(pid, ppid)` for every live chrome-headless-shell process (read-only WMI
/// query; never kills anything).
fn shell_parentage() -> Vec<(u32, u32)> {
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
    let out = q.spawn().expect("spawn parentage query").wait_with_output();
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

const DATA_URL: &str = "data:text/html,<html><head><title>AttachTitle</title></head><body><h1>ATTACH_MARKER_9z7x</h1><button id='b' onclick=\"this.textContent='CLICKED_OK'\">GO</button><input type=file id='f'></body></html>";

fn assert_initialize(c: &mut Client, want_backend: &str, want_caps: &str) {
    let init = c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    assert_eq!(
        init.get("serverInfo").and_then(|s| s.get("name")),
        Some(&json!("gladius")),
        "serverInfo name"
    );
    assert_eq!(
        init.get("serverInfo")
            .and_then(|s| s.get("backend"))
            .and_then(|b| b.as_str()),
        Some(want_backend),
        "serverInfo backend"
    );
    assert_eq!(
        init.get("serverInfo")
            .and_then(|s| s.get("caps"))
            .and_then(|x| x.as_str()),
        Some(want_caps),
        "serverInfo caps"
    );
}

#[test]
fn attach_over_ws_drives_and_leaves_external_alive() {
    let mut ext = External::start();
    let ws = version_ws(&ext.base_url()).expect("external ws url");
    assert!(ws.starts_with("ws://"), "ws endpoint: {ws}");

    let mut c = Client::spawn_with_env(&[("GLADIUS_CDP_URL", ws.as_str())]);
    let gpid = c.child.id();
    assert_initialize(&mut c, "external-cdp", "render");

    let nav = c.call("navigate", json!({"url": DATA_URL}));
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");
    let snap = c.call("snapshot", json!({}));
    assert!(snap.get("isError").is_none(), "snapshot ok: {snap}");
    assert!(
        content_text(&snap).contains("ATTACH_MARKER_9z7x"),
        "snapshot sees the data: page"
    );
    let shot = c.call("screenshot", json!({}));
    assert!(
        shot.get("isError").is_none(),
        "screenshot ok on render: {shot}"
    );
    let data = shot
        .get("content")
        .and_then(|cc| cc.get(0))
        .and_then(|b| b.get("data"))
        .and_then(|d| d.as_str())
        .unwrap_or("");
    assert!(data.starts_with("iVBORw0KGgo"), "real png bytes");

    // gladius created no engine of its own: no owned profile, and no
    // chrome-headless-shell process parented under the gladius pid.
    assert!(
        profile_dirs(gpid).is_empty(),
        "attached gladius owns no profile: {:?}",
        profile_dirs(gpid)
    );
    for (pid, ppid) in shell_parentage() {
        assert!(
            ppid != gpid,
            "gladius (pid {gpid}) launched chrome pid {pid}: ancestry violation"
        );
    }

    let closed = c.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
    shutdown(c);

    // Ownership: the external engine is STILL ALIVE after gladius exits.
    assert!(ext.alive(), "external engine must survive gladius close");
    assert!(
        profile_dirs(gpid).is_empty(),
        "attached close sweeps nothing"
    );
}

#[test]
fn attach_over_http_resolves_via_json_version() {
    let mut ext = External::start();
    let base = ext.base_url();

    let mut c = Client::spawn_with_env(&[("GLADIUS_CDP_URL", base.as_str())]);
    assert_initialize(&mut c, "external-cdp", "render");

    let nav = c.call("navigate", json!({"url": DATA_URL}));
    assert!(
        nav.get("isError").is_none(),
        "navigate ok via http attach: {nav}"
    );
    let snap = c.call("snapshot", json!({}));
    assert!(snap.get("isError").is_none(), "snapshot ok: {snap}");
    assert!(
        content_text(&snap).contains("ATTACH_MARKER_9z7x"),
        "http attach drives the page"
    );
    let ev = c.call("evaluate", json!({"expression": "document.title"}));
    assert!(ev.get("isError").is_none(), "evaluate ok: {ev}");
    assert!(
        content_text(&ev).contains("AttachTitle"),
        "title round-trip: {}",
        content_text(&ev)
    );

    let closed = c.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
    shutdown(c);
    assert!(
        ext.alive(),
        "external engine must survive http-attach close"
    );
}

#[test]
fn dom_only_refuses_screenshot_but_keeps_dom_tools() {
    let mut ext = External::start();
    let ws = version_ws(&ext.base_url()).expect("external ws url");

    let mut c = Client::spawn_with_env(&[
        ("GLADIUS_CDP_URL", ws.as_str()),
        ("GLADIUS_BACKEND_CAPS", "dom-only"),
    ]);
    assert_initialize(&mut c, "external-cdp", "dom-only");

    // DOM tools keep working on a renderless backend.
    let nav = c.call("navigate", json!({"url": DATA_URL}));
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");
    let snap = c.call("snapshot", json!({}));
    assert!(snap.get("isError").is_none(), "snapshot ok: {snap}");
    assert!(
        content_text(&snap).contains("ATTACH_MARKER_9z7x"),
        "snapshot works dom-only"
    );

    // screenshot refuses with the backend and the reason, never an image.
    let shot = c.call("screenshot", json!({}));
    assert!(
        shot.get("isError").is_some(),
        "dom-only screenshot must refuse: {shot}"
    );
    let msg = content_text(&shot);
    assert!(msg.contains("dom-only"), "names the backend: {msg}");
    assert!(
        msg.contains("no rendering pipeline"),
        "names the reason: {msg}"
    );
    assert!(
        shot.get("content")
            .and_then(|cc| cc.get(0))
            .and_then(|b| b.get("data"))
            .is_none(),
        "refusal carries no image bytes: {shot}"
    );

    // upload speaks DOM CDP only (no compositor path), so it keeps working
    // dom-only: real temp files onto the data: page's file input.
    let dir = std::env::temp_dir().join(format!(
        "gladius-domup-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&dir).expect("domup dir");
    let f = dir.join("dom-only-upload.txt");
    std::fs::write(&f, "gladius-dom-only-upload-probe").expect("write upload file");
    let up = c.call(
        "upload",
        json!({"element": "#f", "paths": [f.to_string_lossy()]}),
    );
    assert!(up.get("isError").is_none(), "upload works dom-only: {up}");
    let flen = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('f').files.length"}),
    );
    assert!(
        content_text(&flen).contains('1'),
        "file landed: {}",
        content_text(&flen)
    );
    std::fs::remove_dir_all(&dir).ok();

    let closed = c.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
    shutdown(c);
    assert!(ext.alive(), "external engine must survive dom-only close");
}

#[test]
fn initialize_reports_default_backend_without_launching() {
    // Neither var set: cold handshake reports the owned Obscura render
    // backend and launches nothing (no navigate here, so no browser spawns).
    let mut c = Client::spawn_with_env(&[]);
    assert_initialize(&mut c, "obscura", "render");
    let pong = c.result("ping", json!({}));
    assert_eq!(pong, json!({}), "ping replies {{}}");
    shutdown(c);
}
