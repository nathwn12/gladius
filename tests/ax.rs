//! G3 real-AX snapshot: roles/names/states from Accessibility.getFullAXTree,
//! backend-id stamping, shadow DOM, password redaction, and frame snapshots.
//! Each test owns one server process and reaps its browser.
//! RED PROOF: on the pre-G3 tree `snapshot {"mode":"ax"}` falls back to the
//! legacy heuristic (no shadow piercing), so the shadow-button assertion
//! fails there and passes here.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{json, Value};

fn fixture_url(name: &str) -> String {
    let p = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
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

fn launch_on_ax() -> Client {
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let nav = c.call("navigate", json!({"url": fixture_url("ax.html")}));
    assert!(nav.get("isError").is_none(), "navigate ok: {nav}");
    // The child iframe commits after the main load; wait for the element,
    // then for its frameId to appear in the frame tree (commit race).
    let w = c.call("wait_for", json!({"selector": "#ax-frame", "ms": 10000}));
    assert!(!is_error(&w), "frame element settled: {w}");
    for _ in 0..100 {
        let listed = c.call("targets", json!({}));
        if !is_error(&listed) {
            let t = content_text(&listed);
            if t.contains("ax_frame.html") {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    c
}

fn finish(mut c: Client) {
    let closed = c.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
    shutdown(c);
}

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

/// First quoted segment of a node line: `- role "NAME" [@eN] (tag) ...`.
fn line_name(line: &str) -> String {
    let mut it = line.splitn(3, '"');
    it.next();
    it.next().unwrap_or("").to_string()
}

fn probe_line(snap: &str, needle: &str) -> String {
    snap.lines()
        .filter(|l| l.starts_with("- "))
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no node line with {needle:?} in snapshot:\n{snap}"))
        .to_string()
}

/// Stamped ref (`eN`) of the element matched by this JS expression, via
/// the `data-gladius-ref` stamp. Read back right after the snapshot under
/// test: each snapshot restamps the whole page, so refs are table-local.
fn ref_of(c: &mut Client, expr: &str) -> String {
    let t = content_text(&c.call(
        "evaluate",
        json!({"expression": format!("({expr}).dataset.gladiusRef || 'NOREF'")}),
    ));
    let rid: String = t
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .trim_start_matches("value")
        .to_string();
    assert!(
        !rid.is_empty() && rid != "NOREF",
        "stamped ref present for {expr}"
    );
    rid
}

/// Node line bearing this ref.
fn line_for_ref(snap: &str, rid: &str) -> String {
    snap.lines()
        .filter(|l| l.starts_with("- "))
        .find(|l| l.contains(&format!("[@{rid}]")))
        .unwrap_or_else(|| panic!("no line for ref @{rid} in snapshot:\n{snap}"))
        .to_string()
}

#[test]
fn ax_legacy_name_parity_unlabeled_inputs() {
    // P2 parity: an unlabeled control's fallback name must match legacy
    // naming exactly - full raw value (not the 60-char display cut),
    // whitespace-normalized, capped at 80, legacy order, sensitive values
    // never used as names.
    let mut c = launch_on_ax();
    let long75 = format!("LONG_{}", "x".repeat(70));
    assert_eq!(long75.len(), 75);
    c.call(
        "evaluate",
        json!({"expression": format!(
            "document.body.insertAdjacentHTML('beforeend', \
             `<input id=\"par-long\" type=\"text\" value=\"{long75}\">\
              <input id=\"par-ws\" type=\"text\" value=\"  alpha   beta  \">\
              <input id=\"par-ph\" type=\"text\" placeholder=\"Enter name here\">\
              <input id=\"par-idonly\" type=\"text\" value=\"\">\
              <input name=\"par-nameonly\" type=\"text\" value=\"\">\
              <input id=\"par-sens\" type=\"password\" value=\"secret-xyz-123\">`); 'injected'"
        )}),
    );
    let def = content_text(&c.call("snapshot", json!({})));
    let leg = content_text(&c.call("snapshot", json!({"mode": "legacy"})));

    // 75-char value: fallback name is the FULL value, not the 60-char cut.
    let (dlong, llong) = (
        line_name(&probe_line(&def, "LONG_")),
        line_name(&probe_line(&leg, "LONG_")),
    );
    assert_eq!(llong, long75, "legacy 75-char name");
    assert_eq!(
        dlong, llong,
        "default matches legacy on 75-char value:\n{def}"
    );

    // Whitespace value: normalized exactly like legacy.
    let (dws, lws) = (
        line_name(&probe_line(&def, "alpha   beta")),
        line_name(&probe_line(&leg, "alpha   beta")),
    );
    assert_eq!(lws, "alpha beta", "legacy normalizes whitespace");
    assert_eq!(
        dws, lws,
        "default normalizes whitespace like legacy:\n{def}"
    );

    // Placeholder / id / name fallbacks.
    for (needle, want) in [
        ("Enter name here", "Enter name here"),
        ("par-idonly", "par-idonly"),
        ("par-nameonly", "par-nameonly"),
    ] {
        let (d, l) = (
            line_name(&probe_line(&def, needle)),
            line_name(&probe_line(&leg, needle)),
        );
        assert_eq!(l, want, "legacy {needle} name");
        assert_eq!(d, l, "default matches legacy for {needle}:\n{def}");
    }

    // Sensitive value: masked display, id as name, secret nowhere.
    let (ds, ls) = (
        line_name(&probe_line(&def, "par-sens")),
        line_name(&probe_line(&leg, "par-sens")),
    );
    assert_eq!(ls, "par-sens", "legacy never names from a password value");
    assert_eq!(ds, ls, "default never names from a password value:\n{def}");
    assert!(
        !def.contains("secret-xyz-123"),
        "raw secret absent from default:\n{def}"
    );
    let sens_line = probe_line(&def, "par-sens");
    assert!(
        sens_line.contains("••••"),
        "mask on the sensitive line: {sens_line}"
    );
    finish(c);
}

#[test]
fn ax_legacy_name_parity_whitespace_prefix() {
    // Truncation-before-normalization probe: 79 spaces + TOKEN in id,
    // name, placeholder, and selected-option text. Legacy normalizes the
    // FULL candidate then caps at 80, so the trailing token survives on
    // both paths; all-whitespace stays empty; passwords never name.
    let mut c = launch_on_ax();
    let pad = " ".repeat(79);
    c.call(
        "evaluate",
        json!({"expression": format!(
            "document.body.insertAdjacentHTML('beforeend', \
             `<input id=\"{pad}ID_END\" type=\"text\" value=\"\">\
              <input name=\"{pad}NAME_END\" type=\"text\" value=\"\">\
              <input id=\"par-wsph\" type=\"text\" placeholder=\"{pad}PH_END\">\
              <select id=\"par-wssel\"><option>{pad}OPT_END</option></select>\
              <input id=\"par-wsempty\" type=\"text\" value=\"   \">\
              <input id=\"par-wspass\" type=\"password\" value=\"   secret-keep-out\">`); 'injected'"
        )}),
    );
    let def = content_text(&c.call("snapshot", json!({})));
    // Refs are table-local: read back before the next snapshot restamps.
    let by_id = |c: &mut Client, id: &str| {
        let lit = serde_json::to_string(id).unwrap();
        ref_of(c, &format!("document.getElementById({lit})"))
    };
    let d_id = by_id(&mut c, &format!("{pad}ID_END"));
    let d_ph = by_id(&mut c, "par-wsph");
    let d_sel = by_id(&mut c, "par-wssel");
    let d_we = by_id(&mut c, "par-wsempty");
    let d_pw = by_id(&mut c, "par-wspass");
    let d_nm = ref_of(
        &mut c,
        &format!("document.querySelector('[name=\"{pad}NAME_END\"]')"),
    );
    let leg = content_text(&c.call("snapshot", json!({"mode": "legacy"})));
    let l_id = by_id(&mut c, &format!("{pad}ID_END"));
    let l_ph = by_id(&mut c, "par-wsph");
    let l_sel = by_id(&mut c, "par-wssel");
    let l_we = by_id(&mut c, "par-wsempty");
    let l_pw = by_id(&mut c, "par-wspass");
    let l_nm = ref_of(
        &mut c,
        &format!("document.querySelector('[name=\"{pad}NAME_END\"]')"),
    );

    // Trailing token survives normalization on both paths.
    for (d, l, want) in [
        (d_id, l_id, "ID_END"),
        (d_nm, l_nm, "NAME_END"),
        (d_ph, l_ph, "PH_END"),
        (d_sel, l_sel, "OPT_END"),
    ] {
        let (dn, ln) = (
            line_name(&line_for_ref(&def, &d)),
            line_name(&line_for_ref(&leg, &l)),
        );
        assert_eq!(ln, want, "legacy keeps the trailing token for {want}");
        assert_eq!(
            dn, ln,
            "default keeps the trailing token for {want}:\n{def}"
        );
    }

    // All-whitespace value: empty name (whitespace-only is truthy in the
    // legacy chain, so it blocks the id fallback and normalizes to "").
    let (dw, lw) = (
        line_name(&line_for_ref(&def, &d_we)),
        line_name(&line_for_ref(&leg, &l_we)),
    );
    assert_eq!(lw, "", "legacy all-whitespace name is empty");
    assert_eq!(dw, lw, "default all-whitespace name is empty:\n{def}");

    // Password with a leading-space secret: id names it, secret nowhere.
    let (dp, lp) = (
        line_name(&line_for_ref(&def, &d_pw)),
        line_name(&line_for_ref(&leg, &l_pw)),
    );
    assert_eq!(lp, "par-wspass", "legacy never names from a password value");
    assert_eq!(dp, lp, "default never names from a password value:\n{def}");
    assert!(
        !def.contains("secret-keep-out"),
        "raw secret absent from default:\n{def}"
    );
    finish(c);
}

#[test]
fn ax_real_roles_names_states() {
    let mut c = launch_on_ax();
    let snap = c.call("snapshot", json!({"mode": "ax"}));
    assert!(!is_error(&snap), "ax snapshot ok: {snap}");
    let t = content_text(&snap);
    assert!(t.contains("[@e"), "AX refs present, got:\n{t}");
    assert!(t.contains("[snapshot:"), "node-count footer, got tail");
    assert!(t.contains("AX_FIXTURE_MARKER_ax-67890"), "marker kept");
    // Real AX roles/names (not tag-derived guesses).
    assert!(
        t.contains("button") && t.contains("AX Real Button"),
        "button role+name:\n{t}"
    );
    assert!(
        t.contains("link") && t.contains("AX_LINK_LABEL_nav"),
        "link role+name:\n{t}"
    );
    assert!(
        t.contains("textbox") && t.contains("AX Name Field"),
        "textbox role+name:\n{t}"
    );
    assert!(
        t.contains("checkbox") && t.contains("AX Check Me"),
        "checkbox role+name:\n{t}"
    );
    assert!(
        t.contains("combobox") && t.contains("AX Choice"),
        "select role+name:\n{t}"
    );
    // Disabled state surfaces on the line.
    let dis_line = t
        .lines()
        .find(|l| l.contains("AX_DISABLED_BTN"))
        .unwrap_or_else(|| panic!("disabled button line missing:\n{t}"))
        .to_string();
    assert!(
        dis_line.contains("[disabled]"),
        "disabled state: {dis_line}"
    );
    assert!(t.chars().count() <= 8000, "bounded to 8000 chars");
    finish(c);
}

#[test]
fn ax_hidden_excluded_password_redacted_shadow() {
    let mut c = launch_on_ax();
    let snap = c.call("snapshot", json!({"mode": "ax"}));
    assert!(!is_error(&snap), "ax snapshot ok: {snap}");
    let t = content_text(&snap);
    // Hidden subtree is ignored by the AX tree: excluded from lines.
    assert!(
        !t.contains("AX_HIDDEN_SHOULD_NOT_APPEAR"),
        "hidden node excluded, got:\n{t}"
    );
    // Password value never leaks: literal mask, raw secret absent.
    assert!(t.contains("••••"), "password mask present, got:\n{t}");
    assert!(!t.contains("supersecret123"), "raw password absent");
    // Open shadow root pierced: shadow button appears with a usable ref.
    assert!(
        t.contains("SHADOW_BTN_LABEL_inner-abc"),
        "shadow button present, got:\n{t}"
    );
    let r = find_ref(&t, "SHADOW_BTN_LABEL_inner-abc");
    assert!(r.starts_with("@e"), "shadow ref shape: {r}");
    // Structured block carries the same node with role/name/value/tag.
    let blocks = snap
        .get("content")
        .and_then(|cc| cc.as_array())
        .expect("content array");
    assert_eq!(blocks.len(), 2, "ax snapshot two blocks: {snap}");
    let second = blocks[1].get("text").and_then(|x| x.as_str()).unwrap_or("");
    let obj: Value = serde_json::from_str(second).expect("block2 parses");
    let nodes = obj.get("nodes").and_then(|n| n.as_array()).expect("nodes");
    let hit = nodes
        .iter()
        .find(|n| {
            n.get("name")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .contains("SHADOW_BTN_LABEL_inner-abc")
        })
        .unwrap_or_else(|| panic!("shadow node missing in block2: {second}"));
    assert_eq!(
        hit.get("role").and_then(|x| x.as_str()),
        Some("button"),
        "shadow role: {hit}"
    );
    finish(c);
}

#[test]
fn ax_frame_snapshot_works() {
    let mut c = launch_on_ax();
    let listed = c.call("targets", json!({}));
    assert!(!is_error(&listed), "targets ok: {listed}");
    let t = content_text(&listed);
    let obj: Value = serde_json::from_str(&t).expect("targets json");
    let frames = obj
        .get("frames")
        .and_then(|f| f.as_array())
        .expect("frames array");
    let child = frames
        .iter()
        .find(|f| {
            f.get("url")
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .contains("ax_frame.html")
        })
        .unwrap_or_else(|| panic!("ax child frame missing: {obj}"));
    let fid = child.get("frameId").and_then(|v| v.as_str()).unwrap_or("");
    assert!(!fid.is_empty(), "child frameId present: {child}");
    let snap = c.call("snapshot", json!({"mode": "ax", "frame": fid}));
    assert!(!is_error(&snap), "frame ax snapshot ok: {snap}");
    let ft = content_text(&snap);
    assert!(
        ft.contains("FRAME_BTN_LABEL_in-frame"),
        "frame button in frame snapshot, got:\n{ft}"
    );
    assert!(
        ft.contains("AX_FRAME_MARKER_child-33333"),
        "frame text scoped to frame, got:\n{ft}"
    );
    assert!(
        !ft.contains("AX_HIDDEN_SHOULD_NOT_APPEAR"),
        "frame snapshot clean"
    );
    finish(c);
}

#[test]
fn ax_default_is_ax_legacy_differs() {
    // G3b: the DEFAULT snapshot is the real AX tree. `mode:"ax"` is
    // identical; explicit `mode:"legacy"` keeps the old DOM heuristic
    // (no shadow piercing, hidden nodes stamped); `mode:"text"` is text-only.
    let mut c = launch_on_ax();
    let def = content_text(&c.call("snapshot", json!({})));
    assert!(
        def.contains("SHADOW_BTN_LABEL_inner-abc"),
        "default pierces the open shadow root (real AX):\n{def}"
    );
    assert!(
        !def.lines()
            .any(|l| l.starts_with("- ") && l.contains("AX_HIDDEN_SHOULD_NOT_APPEAR")),
        "default excludes the hidden subtree from node lines:\n{def}"
    );
    assert!(def.contains("••••"), "default masks the password:\n{def}");
    assert!(
        !def.contains("supersecret123"),
        "raw password absent:\n{def}"
    );
    let ax = content_text(&c.call("snapshot", json!({"mode": "ax"})));
    assert!(
        ax.contains("SHADOW_BTN_LABEL_inner-abc"),
        "explicit ax pierces shadow:\n{ax}"
    );
    let leg = content_text(&c.call("snapshot", json!({"mode": "legacy"})));
    assert!(
        !leg.contains("SHADOW_BTN_LABEL_inner-abc"),
        "explicit legacy misses shadow content"
    );
    assert!(
        leg.lines()
            .any(|l| l.starts_with("- ") && l.contains("AX_HIDDEN_SHOULD_NOT_APPEAR")),
        "legacy still stamps the hidden node (byte-compatible G3a path):\n{leg}"
    );
    let txt = content_text(&c.call("snapshot", json!({"mode": "text"})));
    assert!(txt.contains("[text-only snapshot:"), "text mode kept");
    finish(c);
}

#[test]
fn default_ax_full_semantics() {
    // New default-AX proof on one ARIA fixture: roles/names from the AX
    // tree, hidden subtree absent, open shadow root present with a usable
    // ref, password masked, same-process iframe scoped via frame=.
    let mut c = launch_on_ax();
    let def = content_text(&c.call("snapshot", json!({})));
    // ARIA roles/names (not tag-derived guesses).
    assert!(
        def.contains("button") && def.contains("AX Real Button"),
        "button role+name:\n{def}"
    );
    assert!(
        def.contains("link") && def.contains("AX_LINK_LABEL_nav"),
        "link role+name:\n{def}"
    );
    assert!(
        def.contains("textbox") && def.contains("AX Name Field"),
        "textbox role+name:\n{def}"
    );
    assert!(
        def.contains("checkbox") && def.contains("AX Check Me"),
        "checkbox role+name:\n{def}"
    );
    assert!(
        def.contains("combobox") && def.contains("AX Choice"),
        "combobox role+name:\n{def}"
    );
    // Disabled state still surfaces on the line.
    let dis = def
        .lines()
        .find(|l| l.contains("AX_DISABLED_BTN"))
        .unwrap_or_else(|| panic!("disabled button line missing:\n{def}"))
        .to_string();
    assert!(dis.contains("[disabled]"), "disabled state: {dis}");
    // Hidden subtree: no node line (legacy is the only path that stamps it).
    assert!(
        !def.lines()
            .any(|l| l.starts_with("- ") && l.contains("AX_HIDDEN_SHOULD_NOT_APPEAR")),
        "hidden node absent from default node lines:\n{def}"
    );
    // Shadow button present with a usable ref - the marker legacy cannot produce.
    assert!(
        def.contains("SHADOW_BTN_LABEL_inner-abc"),
        "shadow present:\n{def}"
    );
    let r = find_ref(&def, "SHADOW_BTN_LABEL_inner-abc");
    assert!(r.starts_with("@e"), "shadow ref shape: {r}");
    // Password masked, raw secret absent.
    assert!(def.contains("••••"), "password mask:\n{def}");
    assert!(!def.contains("supersecret123"), "raw password absent");
    assert!(def.chars().count() <= 8000, "bounded to 8000 chars");
    finish(c);
}
