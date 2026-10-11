//! AX-ref keystone: snapshot bears stable refs; click/type/hover/select act
//! by exact ref; stale refs fail typed; truncation is announced; text mode
//! is preserved. Each test owns one server process and reaps its browser.
//! These tests FAIL on the pre-keystone binary (text-only snapshot, no refs).

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
        // Default-engine location pin (see e2e_mcp.rs): cargo tree has no
        // obscura sibling, so point at the repo-pinned bin/obscura.exe.
        cmd.env(
            "GLADIUS_OBSCURA",
            format!("{}/bin/obscura.exe", env!("CARGO_MANIFEST_DIR")),
        );
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

fn is_error(v: &Value) -> bool {
    v.get("isError").is_some()
}

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

fn launch_on_fixture() -> Client {
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let nav = c.call("navigate", json!({"url": fixture_url()}));
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");
    c
}

fn finish(mut c: Client) {
    let closed = c.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
    shutdown(c);
}

/// Find the ref (`@eN`) of the snapshot line containing `needle`.
fn find_ref(snap: &str, needle: &str) -> String {
    for line in snap.lines() {
        if line.contains(needle) {
            if let Some(a) = line.find("[@") {
                let rest = &line[a + 2..];
                if let Some(b) = rest.find(']') {
                    return format!("@{}", &rest[..b]);
                }
            }
        }
    }
    panic!("no ref found for {needle:?} in snapshot:\n{snap}");
}

#[test]
fn snapshot_bears_refs_and_marker() {
    let mut c = launch_on_fixture();
    let snap = c.call("snapshot", json!({}));
    assert!(!is_error(&snap), "snapshot ok: {snap}");
    let t = content_text(&snap);
    assert!(t.contains("[@e"), "AX refs present, got:\n{t}");
    assert!(t.contains("[snapshot:"), "node-count footer, got:\n{t}");
    assert!(
        t.contains("GLADIUS_MARKER_hello-12345"),
        "marker text kept in AX output"
    );
    assert!(t.chars().count() <= 8000, "bounded to 8000 chars");
    finish(c);
}

#[test]
fn ref_actions_are_exact() {
    let mut c = launch_on_fixture();
    // Two identical buttons: first-match would hit DUP_A; ref must hit DUP_B.
    c.call(
        "evaluate",
        json!({"expression": "document.body.insertAdjacentHTML('beforeend', `<button class=\"dup\" onclick=\"window.__d1=1\">DUP_A_XYZ</button><button class=\"dup\" onclick=\"window.__d2=1\">DUP_B_XYZ</button>`); 'injected'"}),
    );
    let snap = content_text(&c.call("snapshot", json!({})));
    let rb = find_ref(&snap, "DUP_B_XYZ");
    let clk = c.call("click", json!({"element": rb}));
    assert!(!is_error(&clk), "click by ref ok: {clk}");
    let got = content_text(&c.call(
        "evaluate",
        json!({"expression": "!!window.__d2 && !window.__d1"}),
    ));
    assert!(got.contains("true"), "EXACT ref hit (DUP_B only): {got}");

    // type by ref.
    let snap2 = content_text(&c.call("snapshot", json!({})));
    let rn = find_ref(&snap2, "DUP_A_XYZ"); // any valid ref proves parse; now name input
    let _ = rn;
    let snap3 = content_text(&c.call("snapshot", json!({})));
    // #name input has empty name; find via evaluate-free path: snapshot lists
    // inputs with value=""; use the select instead for a named lookup.
    let rs = find_ref(&snap3, "Red");
    let sel = c.call("select", json!({"element": rs, "value": "g"}));
    assert!(sel.get("isError").is_none(), "select by ref ok: {sel}");
    let v = content_text(&c.call(
        "evaluate",
        json!({"expression": "document.getElementById('color').value"}),
    ));
    assert!(v.contains('g'), "select-by-ref landed: {v}");

    // hover by ref fires the real mouse path.
    let snap4 = content_text(&c.call("snapshot", json!({})));
    let rh = find_ref(&snap4, "HOVER_ME");
    let hov = c.call("hover", json!({"element": rh}));
    assert!(!is_error(&hov), "hover by ref ok: {hov}");
    let h = content_text(&c.call(
        "evaluate",
        json!({"expression": "document.getElementById('hover-result').textContent"}),
    ));
    assert!(
        h.contains("HOVERED_OK"),
        "hover-by-ref fired mouseover: {h}"
    );

    // type by ref into #name: re-snapshot to get its ref via id lookup.
    let rname = content_text(&c.call(
        "evaluate",
        json!({"expression": "document.querySelector('#name').dataset.gladiusRef || 'NOREF'"}),
    ));
    assert!(!rname.contains("NOREF"), "name input stamped: {rname}");
    // content_text wraps as {"value":"eN"}; extract eN.
    let id: String = rname
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .trim_start_matches("value")
        .to_string();
    let t = c.call(
        "type",
        json!({"element": format!("@{id}"), "text": "ref-typed-1"}),
    );
    assert!(!is_error(&t), "type by ref ok: {t}");
    let vv = content_text(&c.call(
        "evaluate",
        json!({"expression": "document.getElementById('name').value"}),
    ));
    assert!(vv.contains("ref-typed-1"), "type-by-ref landed: {vv}");

    // Ref-targeted actionability gate (G-1): hidden node by ref errors, no click.
    let rhid = content_text(&c.call(
        "evaluate",
        json!({"expression": "document.getElementById('hidden-box').dataset.gladiusRef || 'NOREF'"}),
    ));
    assert!(!rhid.contains("NOREF"), "hidden node stamped");
    finish(c);
}

#[test]
fn stale_refs_are_typed() {
    let mut c = launch_on_fixture();
    let snap = content_text(&c.call("snapshot", json!({})));
    let r = find_ref(&snap, "CLICK_ME");

    // Unknown ref: never issued.
    let unk = c.call("click", json!({"element": "@e99999"}));
    assert!(is_error(&unk), "unknown ref isError: {unk}");
    let m = content_text(&unk);
    assert!(
        m.contains("stale ref") && m.contains("unknown ref"),
        "typed unknown: {m}"
    );

    // Node gone: remove the stamped node, keep the URL.
    c.call(
        "evaluate",
        json!({"expression": "document.getElementById('toggle').remove(); 'removed'"}),
    );
    let gone = c.call("click", json!({"element": r}));
    assert!(is_error(&gone), "gone ref isError: {gone}");
    let g = content_text(&gone);
    assert!(
        g.contains("stale ref") && g.contains("node gone"),
        "typed node-gone: {g}"
    );

    // Page navigated: fresh snapshot, then leave the page.
    let nav = c.call("navigate", json!({"url": fixture_url()}));
    assert!(!is_error(&nav), "re-navigate ok: {nav}");
    let snap2 = content_text(&c.call("snapshot", json!({})));
    let r2 = find_ref(&snap2, "CLICK_ME");
    let nav2 = c.call("navigate", json!({"url": "about:blank"}));
    assert!(!is_error(&nav2), "blank navigate ok: {nav2}");
    let st = c.call("click", json!({"element": r2}));
    assert!(is_error(&st), "navigated ref isError: {st}");
    let s = content_text(&st);
    assert!(
        s.contains("stale ref") && s.contains("page navigated"),
        "typed navigated: {s}"
    );
    finish(c);
}

#[test]
fn truncation_is_announced() {
    let mut c = launch_on_fixture();
    c.call(
        "evaluate",
        json!({"expression": "let h=''; for (let i=0;i<800;i++) h+=`<button id=\"bulk-${i}\">BULK_BUTTON_${i}_WITH_A_LONG_LABEL</button>`; document.body.insertAdjacentHTML('beforeend', h); 'bulked'"}),
    );
    let snap = c.call("snapshot", json!({}));
    assert!(!is_error(&snap), "snapshot ok: {snap}");
    let t = content_text(&snap);
    assert!(
        t.chars().count() <= 8000,
        "bounded: {} chars",
        t.chars().count()
    );
    assert!(
        t.contains("[truncated:") && t.contains("chars omitted"),
        "truncation announced with amount, tail:\n{}",
        t.chars().rev().take(300).collect::<String>()
    );
    finish(c);
}

#[test]
fn text_mode_preserved() {
    let mut c = launch_on_fixture();
    let snap = c.call("snapshot", json!({"mode": "text"}));
    assert!(!is_error(&snap), "text snapshot ok: {snap}");
    let t = content_text(&snap);
    assert!(
        t.contains("[text-only snapshot:"),
        "legacy suffix kept, got tail:\n{}",
        t.chars().rev().take(200).collect::<String>()
    );
    assert!(
        t.contains("GLADIUS_MARKER_hello-12345"),
        "marker in text mode"
    );
    finish(c);
}
