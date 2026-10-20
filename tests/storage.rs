//! G5 storage: one `storage` tool for cookies + Web Storage (localStorage /
//! sessionStorage) plus versioned file save/load. HTTP origins come from a
//! local fixture server (same pattern as safety.rs). Each test owns one
//! server process and reaps its browser; the save/load roundtrip spans two
//! server processes (a fresh profile) against the same fixture server.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{json, Value};

fn fixture_bytes(name: &str) -> Vec<u8> {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push(name);
    std::fs::read(p).expect("read fixture")
}

/// Static fixture server for one file (any path serves it as text/html).
fn serve_file(name: &str) -> u16 {
    let body = fixture_bytes(name);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind localhost");
    let port = listener.local_addr().expect("local addr").port();
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let mut s = s;
            let mut buf = [0u8; 8192];
            let _ = s.read(&mut buf);
            let _ = s.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            );
            let _ = s.write_all(&body);
        }
    });
    port
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

    fn recv(&self, what: &str) -> Value {
        let line = self
            .rx
            .recv_timeout(Duration::from_secs(120))
            .unwrap_or_else(|_| panic!("timeout waiting for reply to {what}"));
        serde_json::from_str(&line).unwrap_or_else(|_| panic!("bad json for {what}: {line}"))
    }

    fn result(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let frame = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{frame}").expect("write stdin");
        self.stdin.flush().expect("flush stdin");
        let resp = self.recv(method);
        assert_eq!(resp.get("id"), Some(&json!(id)), "id echo for {method}");
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

fn structured(v: &Value) -> Value {
    v.get("structuredContent").cloned().unwrap_or(Value::Null)
}

fn is_error(v: &Value) -> bool {
    v.get("isError").is_some()
}

fn finish(mut c: Client) {
    let closed = c.call("close", json!({}));
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

fn cookie_by_name<'a>(v: &'a Value, name: &str) -> Option<&'a Value> {
    v.get("cookies")?
        .as_array()?
        .iter()
        .find(|c| c.get("name").and_then(|n| n.as_str()) == Some(name))
}

fn launch_on_storage() -> (Client, String) {
    let port = serve_file("storage.html");
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let url = format!("http://127.0.0.1:{port}/storage.html");
    let nav = c.call("navigate", json!({"url": url}));
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");
    (c, url)
}

#[test]
fn set_get_roundtrip_with_redact() {
    let (mut c, _url) = launch_on_storage();
    // Set cookies + web-storage items across all scopes.
    let ok = c.call(
        "storage",
        json!({"action": "set", "scope": "all",
               "cookies": [{"name": "g1", "value": "v1"}, {"name": "g2", "value": "v2", "path": "/"}],
               "items": {"sk": "sv", "n2": "w2"}}),
    );
    assert!(!is_error(&ok), "storage set ok: {ok}");
    let sc = structured(&ok);
    assert_eq!(sc.get("action"), Some(&json!("set")), "action: {ok}");
    assert_eq!(
        sc.get("set"),
        Some(&json!({"cookies": 2, "localStorage": 2, "sessionStorage": 2})),
        "set counts (items land in both storage scopes): {ok}"
    );
    // Get everything back.
    let got = c.call("storage", json!({"action": "get", "scope": "all"}));
    assert!(!is_error(&got), "storage get ok: {got}");
    let g = structured(&got);
    assert_eq!(
        cookie_by_name(&g, "g1").and_then(|x| x.get("value")),
        Some(&json!("v1")),
        "cookie g1 roundtrip: {got}"
    );
    assert_eq!(
        g.get("localStorage").and_then(|m| m.get("sk")),
        Some(&json!("sv")),
        "localStorage roundtrip: {got}"
    );
    assert_eq!(
        g.get("sessionStorage").and_then(|m| m.get("n2")),
        Some(&json!("w2")),
        "sessionStorage roundtrip: {got}"
    );
    assert_eq!(
        g.get("counts"),
        Some(&json!({"cookies": 2, "localStorage": 2, "sessionStorage": 2})),
        "get counts: {got}"
    );
    // Redacted get masks every value and says so.
    let red = c.call(
        "storage",
        json!({"action": "get", "scope": "all", "redact": true}),
    );
    assert!(!is_error(&red), "redacted get ok: {red}");
    let r = structured(&red);
    assert_eq!(
        r.get("redacted"),
        Some(&json!(true)),
        "redacted flag: {red}"
    );
    assert_eq!(
        cookie_by_name(&r, "g1").and_then(|x| x.get("value")),
        Some(&json!("••••")),
        "cookie value masked: {red}"
    );
    assert_eq!(
        r.get("localStorage").and_then(|m| m.get("sk")),
        Some(&json!("••••")),
        "storage value masked: {red}"
    );
    assert_eq!(
        r.get("counts"),
        Some(&json!({"cookies": 2, "localStorage": 2, "sessionStorage": 2})),
        "counts stay real under redact: {red}"
    );
    finish(c);
}

#[test]
fn save_load_roundtrip_across_sessions() {
    let (mut c, url) = launch_on_storage();
    let ok = c.call(
        "storage",
        json!({"action": "set", "scope": "all",
               "cookies": [{"name": "ck", "value": "ck-sekret-1"}],
               "items": {"lk": "lk-sekret-2", "sk": "sk-sekret-3"}}),
    );
    assert!(!is_error(&ok), "set ok: {ok}");
    let path = std::env::temp_dir().join("gladius-g5-roundtrip.json");
    let pstr = path.to_string_lossy().to_string();
    let saved = c.call(
        "storage",
        json!({"action": "save", "scope": "all", "path": pstr, "overwrite": true}),
    );
    assert!(!is_error(&saved), "save ok: {saved}");
    let s = structured(&saved);
    assert_eq!(
        s.get("counts"),
        Some(&json!({"cookies": 1, "localStorage": 2, "sessionStorage": 2})),
        "save counts: {saved}"
    );
    assert!(
        s.get("bytes").and_then(|v| v.as_i64()).unwrap_or(0) > 0,
        "save bytes: {saved}"
    );
    // The reply carries counts/bytes/origin only - never values.
    let text = content_text(&saved);
    for secret in ["ck-sekret-1", "lk-sekret-2", "sk-sekret-3"] {
        assert!(!text.contains(secret), "save reply leaks {secret}: {saved}");
    }
    finish(c);
    // Fresh server process = fresh browser profile: nothing survives.
    let mut c2 = Client::spawn();
    c2.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let nav = c2.call("navigate", json!({"url": url}));
    assert!(nav.get("isError").is_none(), "re-navigate ok: {nav}");
    let empty = c2.call("storage", json!({"action": "get", "scope": "all"}));
    let e = structured(&empty);
    assert_eq!(
        e.get("counts"),
        Some(&json!({"cookies": 0, "localStorage": 0, "sessionStorage": 0})),
        "fresh profile starts empty: {empty}"
    );
    let loaded = c2.call(
        "storage",
        json!({"action": "load", "scope": "all", "path": pstr}),
    );
    assert!(!is_error(&loaded), "load ok: {loaded}");
    let l = structured(&loaded);
    assert_eq!(
        l.get("applied"),
        Some(&json!({"cookies": 1, "localStorage": 2, "sessionStorage": 2})),
        "load applied: {loaded}"
    );
    assert_eq!(
        l.get("failed"),
        Some(&json!([])),
        "load failed empty: {loaded}"
    );
    assert!(
        l.get("fileOrigin")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains("127.0.0.1"),
        "load fileOrigin: {loaded}"
    );
    let got = c2.call("storage", json!({"action": "get", "scope": "all"}));
    let g = structured(&got);
    assert_eq!(
        cookie_by_name(&g, "ck").and_then(|x| x.get("value")),
        Some(&json!("ck-sekret-1")),
        "cookie restored: {got}"
    );
    assert_eq!(
        g.get("localStorage").and_then(|m| m.get("lk")),
        Some(&json!("lk-sekret-2")),
        "localStorage restored: {got}"
    );
    assert_eq!(
        g.get("sessionStorage").and_then(|m| m.get("sk")),
        Some(&json!("sk-sekret-3")),
        "sessionStorage restored: {got}"
    );
    std::fs::remove_file(&path).ok();
    finish(c2);
}

#[test]
fn save_file_shape_and_no_values_in_reply() {
    let (mut c, _url) = launch_on_storage();
    let ok = c.call(
        "storage",
        json!({"action": "set", "scope": "all",
               "cookies": [{"name": "sh", "value": "sh-sekret-9"}],
               "items": {"k": "k-sekret-8"}}),
    );
    assert!(!is_error(&ok), "set ok: {ok}");
    let path = std::env::temp_dir().join("gladius-g5-shape.json");
    let pstr = path.to_string_lossy().to_string();
    let saved = c.call(
        "storage",
        json!({"action": "save", "scope": "all", "path": pstr, "overwrite": true}),
    );
    assert!(!is_error(&saved), "save ok: {saved}");
    // The file itself is versioned JSON with values (it is the backup).
    let raw = std::fs::read_to_string(&path).expect("read save file");
    let doc: Value = serde_json::from_str(&raw).expect("save file is JSON");
    assert_eq!(
        doc.get("gladiusStorage"),
        Some(&json!(1)),
        "version tag: {doc}"
    );
    for key in [
        "savedAt",
        "url",
        "origin",
        "cookies",
        "localStorage",
        "sessionStorage",
    ] {
        assert!(doc.get(key).is_some(), "file has {key}: {doc}");
    }
    assert!(
        doc.get("origin")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains("127.0.0.1"),
        "file origin: {doc}"
    );
    // The reply (both blocks) carries counts/bytes/origin only - never values.
    let text = content_text(&saved);
    let sc = structured(&saved);
    for secret in ["sh-sekret-9", "k-sekret-8"] {
        assert!(!text.contains(secret), "reply leaks {secret}: {saved}");
    }
    for key in [
        "cookies",
        "localStorage",
        "sessionStorage",
        "set",
        "applied",
        "failed",
    ] {
        assert!(sc.get(key).is_none(), "reply must not carry {key}: {saved}");
    }
    std::fs::remove_file(&path).ok();
    finish(c);
}

#[test]
fn invalid_inputs_change_nothing() {
    let (mut c, _url) = launch_on_storage();
    let ok = c.call(
        "storage",
        json!({"action": "set", "scope": "all",
               "cookies": [{"name": "keep", "value": "keep-v"}],
               "items": {"kk": "kv"}}),
    );
    assert!(!is_error(&ok), "baseline set ok: {ok}");
    let base = structured(&c.call("storage", json!({"action": "get", "scope": "all"})));
    let base_counts = base.get("counts").cloned().unwrap_or(Value::Null);

    let bad_path = std::env::temp_dir().join("gladius-g5-nope-missing.json");
    let bad_pstr = bad_path.to_string_lossy().to_string();
    let garbage = std::env::temp_dir().join("gladius-g5-garbage.json");
    std::fs::write(&garbage, b"not json{{{").expect("write garbage");
    let garbage_pstr = garbage.to_string_lossy().to_string();
    let missing_dir = std::env::temp_dir()
        .join("gladius-g5-no-such-dir-zzz")
        .join("f.json");
    let missing_dir_pstr = missing_dir.to_string_lossy().to_string();
    let shape_path = std::env::temp_dir().join("gladius-g5-shape2.json");
    let shape_pstr = shape_path.to_string_lossy().to_string();

    let cases: Vec<(&str, Value, &str, &str)> = vec![
        (
            "bad action",
            json!({"action": "bogus"}),
            "invalid_args",
            "unknown_action",
        ),
        (
            "bad scope",
            json!({"action": "get", "scope": "bogus"}),
            "invalid_args",
            "bad_value",
        ),
        (
            "save no path",
            json!({"action": "save"}),
            "invalid_args",
            "missing_arg",
        ),
        (
            "load no path",
            json!({"action": "load"}),
            "invalid_args",
            "missing_arg",
        ),
        (
            "load missing",
            json!({"action": "load", "path": bad_pstr}),
            "file_not_found",
            "missing_path",
        ),
        (
            "load garbage",
            json!({"action": "load", "path": garbage_pstr}),
            "invalid_args",
            "bad_value",
        ),
        (
            "save missing dir",
            json!({"action": "save", "path": missing_dir_pstr}),
            "invalid_args",
            "bad_value",
        ),
        (
            "bad sameSite",
            json!({"action": "set", "cookies": [{"name": "x", "value": "y", "sameSite": "Sometimes"}]}),
            "invalid_args",
            "bad_value",
        ),
        (
            "none w/o secure",
            json!({"action": "set", "cookies": [{"name": "x", "value": "y", "sameSite": "None"}]}),
            "invalid_args",
            "bad_value",
        ),
        (
            "bad secure",
            json!({"action": "set", "cookies": [{"name": "x", "value": "y", "secure": "yes"}]}),
            "invalid_args",
            "bad_value",
        ),
        (
            "set nothing",
            json!({"action": "set"}),
            "invalid_args",
            "need_option",
        ),
        (
            "bad items",
            json!({"action": "set", "items": ["a"]}),
            "invalid_args",
            "bad_value",
        ),
        (
            "bad cookie shape",
            json!({"action": "set", "cookies": [{"name": "x"}]}),
            "invalid_args",
            "bad_value",
        ),
        (
            "unknown arg",
            json!({"action": "get", "bogus": 1}),
            "invalid_args",
            "unknown_arg",
        ),
    ];
    for (label, args, code, reason) in cases {
        let r = c.call("storage", args);
        assert!(is_error(&r), "{label} isError: {r}");
        let sc = structured(&r);
        assert_eq!(sc.get("code"), Some(&json!(code)), "{label} code: {r}");
        assert_eq!(
            sc.get("reason"),
            Some(&json!(reason)),
            "{label} reason: {r}"
        );
    }
    // No-clobber: save once, save again without overwrite fails, with it works.
    let s1 = c.call(
        "storage",
        json!({"action": "save", "path": shape_pstr, "overwrite": true}),
    );
    assert!(!is_error(&s1), "first save ok: {s1}");
    let s2 = c.call("storage", json!({"action": "save", "path": shape_pstr}));
    assert!(is_error(&s2), "clobber refused: {s2}");
    assert_eq!(
        structured(&s2).get("reason"),
        Some(&json!("bad_value")),
        "clobber reason: {s2}"
    );
    let s3 = c.call(
        "storage",
        json!({"action": "save", "path": shape_pstr, "overwrite": true}),
    );
    assert!(!is_error(&s3), "overwrite save ok: {s3}");
    // State is untouched by every failure above.
    let after = structured(&c.call("storage", json!({"action": "get", "scope": "all"})));
    assert_eq!(
        after.get("counts"),
        Some(&base_counts),
        "counts unchanged: {after}"
    );
    assert_eq!(
        cookie_by_name(&after, "keep").and_then(|x| x.get("value")),
        Some(&json!("keep-v")),
        "cookie unchanged: {after}"
    );
    assert_eq!(
        after.get("localStorage").and_then(|m| m.get("kk")),
        Some(&json!("kv")),
        "storage unchanged: {after}"
    );
    std::fs::remove_file(&garbage).ok();
    std::fs::remove_file(&shape_path).ok();
    std::fs::remove_file(&bad_path).ok();
    finish(c);
}

#[test]
fn two_origins_page_scoped_vs_allsites() {
    let port = serve_file("storage.html");
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let url_a = format!("http://127.0.0.1:{port}/storage.html");
    let url_b = format!("http://localhost:{port}/storage.html");
    // Distinct values on origin A (127.0.0.1).
    let nav = c.call("navigate", json!({"url": url_a}));
    assert!(nav.get("isError").is_none(), "navigate A ok: {nav}");
    let ok = c.call(
        "storage",
        json!({"action": "set", "scope": "all",
               "cookies": [{"name": "ca", "value": "va"}],
               "items": {"ia": "iva"}}),
    );
    assert!(!is_error(&ok), "set A ok: {ok}");
    // Distinct values on origin B (localhost): same server, other hostname.
    let nav = c.call("navigate", json!({"url": url_b}));
    assert!(nav.get("isError").is_none(), "navigate B ok: {nav}");
    let ok = c.call(
        "storage",
        json!({"action": "set", "scope": "all",
               "cookies": [{"name": "cb", "value": "vb"}],
               "items": {"ib": "ivb"}}),
    );
    assert!(!is_error(&ok), "set B ok: {ok}");
    // On B, plain get is page-scoped: only B's own cookie.
    let got = c.call("storage", json!({"action": "get", "scope": "cookies"}));
    let g = structured(&got);
    assert!(
        cookie_by_name(&g, "cb").is_some(),
        "own cookie visible: {got}"
    );
    assert!(
        cookie_by_name(&g, "ca").is_none(),
        "other origin cookie hidden page-scoped: {got}"
    );
    // allSites widens the cookie read to every site.
    let all = c.call(
        "storage",
        json!({"action": "get", "scope": "cookies", "allSites": true}),
    );
    let a = structured(&all);
    assert!(
        cookie_by_name(&a, "ca").is_some() && cookie_by_name(&a, "cb").is_some(),
        "allSites returns both origins: {all}"
    );
    assert_eq!(
        a.get("counts").and_then(|x| x.get("cookies")),
        Some(&json!(2)),
        "allSites count: {all}"
    );
    // Web Storage stays page-scoped even under allSites (cookies-only widening).
    let ls = c.call(
        "storage",
        json!({"action": "get", "scope": "localStorage", "allSites": true}),
    );
    let l = structured(&ls);
    assert_eq!(
        l.get("localStorage").and_then(|m| m.get("ib")),
        Some(&json!("ivb")),
        "own storage visible: {ls}"
    );
    assert!(
        l.get("localStorage").and_then(|m| m.get("ia")).is_none(),
        "other origin storage stays hidden: {ls}"
    );
    finish(c);
}

#[test]
fn session_cleared_new_tab_local_persists_with_note() {
    let (mut c, url) = launch_on_storage();
    let ok = c.call(
        "storage",
        json!({"action": "set", "scope": "all", "items": {"tk": "tv"}}),
    );
    assert!(!is_error(&ok), "set ok: {ok}");
    let got = c.call("storage", json!({"action": "get", "scope": "all"}));
    let g = structured(&got);
    assert_eq!(
        g.get("sessionStorage").and_then(|m| m.get("tk")),
        Some(&json!("tv")),
        "sessionStorage set: {got}"
    );
    assert!(
        g.get("note")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains("tab-scoped"),
        "lifetime note present: {got}"
    );
    // A new tab is a new session: sessionStorage is empty, localStorage kept.
    let nt = c.call("tabs", json!({"action": "new", "url": url}));
    assert!(nt.get("isError").is_none(), "new tab ok: {nt}");
    let got = c.call("storage", json!({"action": "get", "scope": "all"}));
    let g = structured(&got);
    assert_eq!(
        g.get("localStorage").and_then(|m| m.get("tk")),
        Some(&json!("tv")),
        "localStorage persists across tabs: {got}"
    );
    assert_eq!(
        g.get("sessionStorage"),
        Some(&json!({})),
        "sessionStorage is fresh in the new tab: {got}"
    );
    assert!(
        g.get("note")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains("tab-scoped"),
        "lifetime note present: {got}"
    );
    finish(c);
}

#[test]
fn attach_clear_confirm_optional() {
    if std::env::var("GLADIUS_CDP_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .is_none()
    {
        eprintln!("NOTE: no attach rig (GLADIUS_CDP_URL unset); skipping");
        return;
    }
    // Against an attached engine, a browser-wide cookie wipe needs confirm.
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let r = c.call(
        "storage",
        json!({"action": "clear", "scope": "cookies", "allSites": true}),
    );
    assert!(is_error(&r), "attach allSites clear needs confirm: {r}");
    let sc = structured(&r);
    assert_eq!(sc.get("code"), Some(&json!("invalid_args")), "code: {r}");
    assert_eq!(sc.get("reason"), Some(&json!("missing_arg")), "reason: {r}");
    let ok = c.call(
        "storage",
        json!({"action": "clear", "scope": "cookies", "allSites": true, "confirm": true}),
    );
    assert!(!is_error(&ok), "confirmed attach clear ok: {ok}");
    finish(c);
}

#[test]
fn set_cookie_with_other_path_roundtrip() {
    // A cookie stored under a non-current path must not be reported as
    // rejected: the read-back has to see the whole jar, not just the
    // current page URL. allSites get shows it; page-scoped get does not.
    let (mut c, _url) = launch_on_storage();
    let ok = c.call(
        "storage",
        json!({"action": "set", "scope": "cookies",
               "cookies": [{"name": "pathprobe", "value": "pv", "path": "/other"}]}),
    );
    assert!(!is_error(&ok), "set path-cookie ok: {ok}");
    assert_eq!(
        structured(&ok).get("set"),
        Some(&json!({"cookies": 1, "localStorage": 0, "sessionStorage": 0})),
        "path-cookie applied: {ok}"
    );
    let all = c.call(
        "storage",
        json!({"action": "get", "scope": "cookies", "allSites": true}),
    );
    let a = structured(&all);
    assert_eq!(
        cookie_by_name(&a, "pathprobe").and_then(|x| x.get("value")),
        Some(&json!("pv")),
        "allSites shows the /other cookie: {all}"
    );
    assert_eq!(
        cookie_by_name(&a, "pathprobe").and_then(|x| x.get("path")),
        Some(&json!("/other")),
        "path preserved: {all}"
    );
    let page = c.call("storage", json!({"action": "get", "scope": "cookies"}));
    let p = structured(&page);
    assert!(
        cookie_by_name(&p, "pathprobe").is_none(),
        "page-scoped get hides the /other cookie: {page}"
    );
    finish(c);
}

#[test]
fn proto_key_roundtrip_both_storages() {
    // "__proto__" is a real key, not a prototype mutation: it must survive
    // the JS bridge in both directions and count like any other key.
    let (mut c, _url) = launch_on_storage();
    let ok = c.call(
        "storage",
        json!({"action": "set", "scope": "all", "items": {"__proto__": "probe", "plain": "v"}}),
    );
    assert!(!is_error(&ok), "set proto items ok: {ok}");
    assert_eq!(
        structured(&ok).get("set"),
        Some(&json!({"cookies": 0, "localStorage": 2, "sessionStorage": 2})),
        "proto items applied: {ok}"
    );
    let got = c.call("storage", json!({"action": "get", "scope": "all"}));
    let g = structured(&got);
    assert_eq!(
        g.get("localStorage").and_then(|m| m.get("__proto__")),
        Some(&json!("probe")),
        "localStorage __proto__ roundtrip: {got}"
    );
    assert_eq!(
        g.get("sessionStorage").and_then(|m| m.get("__proto__")),
        Some(&json!("probe")),
        "sessionStorage __proto__ roundtrip: {got}"
    );
    assert_eq!(
        g.get("counts"),
        Some(&json!({"cookies": 0, "localStorage": 2, "sessionStorage": 2})),
        "proto counts: {got}"
    );
    // And it survives save/load.
    let path = std::env::temp_dir().join("gladius-g5-proto.json");
    let pstr = path.to_string_lossy().to_string();
    let saved = c.call(
        "storage",
        json!({"action": "save", "scope": "all", "path": pstr, "overwrite": true}),
    );
    assert!(!is_error(&saved), "save proto ok: {saved}");
    let cleared = c.call("storage", json!({"action": "clear", "scope": "all"}));
    assert!(!is_error(&cleared), "clear ok: {cleared}");
    let loaded = c.call(
        "storage",
        json!({"action": "load", "scope": "all", "path": pstr}),
    );
    assert!(!is_error(&loaded), "load proto ok: {loaded}");
    let got = c.call("storage", json!({"action": "get", "scope": "all"}));
    let g = structured(&got);
    assert_eq!(
        g.get("localStorage").and_then(|m| m.get("__proto__")),
        Some(&json!("probe")),
        "localStorage __proto__ survives save/load: {got}"
    );
    std::fs::remove_file(&path).ok();
    finish(c);
}

#[test]
fn storage_schema_keys_and_order() {
    // Contract: storage sits immediately after tabs (before close), and its
    // outputSchema carries exactly the documented reply keys.
    let mut c = Client::spawn();
    let listed = c.result("tools/list", json!({}));
    let tools = listed
        .get("tools")
        .and_then(|t| t.as_array())
        .expect("tools array");
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .collect();
    let pos = names
        .iter()
        .position(|n| *n == "storage")
        .expect("storage listed");
    assert_eq!(names[pos - 1], "tabs", "storage follows tabs: {names:?}");
    assert_eq!(names[pos + 1], "close", "close follows storage: {names:?}");
    let schema = tools[pos]
        .get("outputSchema")
        .expect("storage outputSchema");
    let mut keys: Vec<&str> = schema
        .get("properties")
        .and_then(|p| p.as_object())
        .expect("properties")
        .keys()
        .map(|k| k.as_str())
        .collect();
    keys.sort_unstable();
    let mut want = vec![
        "ok",
        "action",
        "scope",
        "url",
        "origin",
        "cookies",
        "localStorage",
        "sessionStorage",
        "counts",
        "set",
        "cleared",
        "failed",
        "applied",
        "path",
        "bytes",
        "fileOrigin",
        "redacted",
        "note",
    ];
    want.sort_unstable();
    assert_eq!(keys, want, "exact reply key set");
    assert_eq!(
        schema.get("required"),
        Some(&json!(["ok", "action"])),
        "required keys"
    );
    assert_eq!(
        schema.get("additionalProperties"),
        Some(&json!(false)),
        "closed schema"
    );
    let closed = c.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
    drop(c.stdin);
    let _ = c.child.wait();
}
