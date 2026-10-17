//! Envelope contract: machine-readable `structuredContent` on every tool
//! error (prose in `content[0].text` byte-identical), a structured second
//! content block on AX `snapshot`, and a caps-aware `capabilities` manifest
//! on `initialize`. Tool names, args, and prose are frozen - only the
//! envelope grows. Each test owns one server process and reaps its browser.

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
    fn spawn_extra(env_extra: &[(&str, &str)]) -> Self {
        let bin = env!("CARGO_BIN_EXE_gladius");
        let mut cmd = Command::new(bin);
        cmd.arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        // Owned-engine resolution (see e2e_mcp.rs): the pinned Chromium
        // resolves from its versioned cache with no env override.
        for (k, v) in env_extra {
            cmd.env(k, v);
        }
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

    fn spawn() -> Self {
        Self::spawn_extra(&[])
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

fn structured(v: &Value) -> Value {
    v.get("structuredContent").cloned().unwrap_or(Value::Null)
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

/// `{"value":"eN"}` evaluate text -> `@eN`.
fn value_to_ref(t: &str) -> String {
    let id: String = t
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .trim_start_matches("value")
        .to_string();
    assert!(!id.is_empty() && id != "NOREF", "stamped ref present: {t}");
    format!("@{id}")
}

/// Refs (`@eN`) carried by the human text block, in order (node lines only).
fn text_refs(t: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in t.lines() {
        if !line.starts_with("- ") {
            continue;
        }
        if let Some(a) = line.find("[@") {
            let rest = &line[a + 2..];
            if let Some(b) = rest.find(']') {
                out.push(format!("@{}", &rest[..b]));
            }
        }
    }
    out
}

#[test]
fn error_codes_stale_ref() {
    let mut c = launch_on_fixture();

    // Unknown ref: never issued (no snapshot taken on this client yet).
    let unk = c.call("click", json!({"element": "@e99999"}));
    assert!(is_error(&unk), "unknown ref isError: {unk}");
    assert_eq!(
        content_text(&unk),
        "stale ref: click: unknown ref @e99999 (no such ref; take a fresh snapshot)",
        "unknown prose byte-identical"
    );
    assert_eq!(
        structured(&unk),
        json!({"code": "stale_ref", "reason": "unknown",
               "tool": "click", "ref": "@e99999"}),
        "unknown code"
    );

    // Node gone: remove the stamped node, keep the URL.
    let snap = content_text(&c.call("snapshot", json!({})));
    let r = find_ref(&snap, "CLICK_ME");
    let id = r.trim_start_matches('@').to_string();
    c.call(
        "evaluate",
        json!({"expression": "document.getElementById('toggle').remove(); 'removed'"}),
    );
    let gone = c.call("click", json!({"element": r}));
    assert!(is_error(&gone), "gone ref isError: {gone}");
    assert_eq!(
        content_text(&gone),
        format!("stale ref: click: ref @{id} is stale (node gone; take a fresh snapshot)"),
        "node-gone prose byte-identical"
    );
    assert_eq!(
        structured(&gone),
        json!({"code": "stale_ref", "reason": "node_gone",
               "tool": "click", "ref": r}),
        "node-gone code"
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
        s.starts_with(&format!(
            "stale ref: click: ref {r2} is stale (page navigated from "
        )) && s.ends_with("to about:blank; take a fresh snapshot)")
            && s.contains("fixture.html"),
        "navigated prose byte-identical shape: {s}"
    );
    let sc = structured(&st);
    assert_eq!(
        sc.get("code"),
        Some(&json!("stale_ref")),
        "navigated code: {st}"
    );
    assert_eq!(
        sc.get("reason"),
        Some(&json!("navigated")),
        "navigated reason: {st}"
    );
    assert_eq!(sc.get("ref"), Some(&json!(r2)), "navigated ref: {st}");

    // Back on the fixture for a clean close.
    let back = c.call("navigate", json!({"url": fixture_url()}));
    assert!(!is_error(&back), "navigate back ok: {back}");
    finish(c);
}

#[test]
fn error_codes_not_actionable() {
    let mut c = launch_on_fixture();

    // Click hidden (selector path).
    let hid = c.call("click", json!({"element": "#hidden-box"}));
    assert!(is_error(&hid), "click hidden isError: {hid}");
    assert_eq!(
        content_text(&hid),
        "click: element not visible or not clickable: #hidden-box",
        "click-hidden prose byte-identical"
    );
    assert_eq!(
        structured(&hid),
        json!({"code": "not_actionable", "reason": "hidden",
               "tool": "click", "element": "#hidden-box"}),
        "click-hidden code"
    );

    // Click disabled (selector path).
    let dis = c.call("click", json!({"element": "#disabled-btn"}));
    assert!(is_error(&dis), "click disabled isError: {dis}");
    assert_eq!(
        content_text(&dis),
        "click: element is disabled: #disabled-btn",
        "click-disabled prose byte-identical"
    );
    assert_eq!(
        structured(&dis),
        json!({"code": "not_actionable", "reason": "disabled",
               "tool": "click", "element": "#disabled-btn"}),
        "click-disabled code"
    );

    // Type disabled / readonly (selector paths).
    let td = c.call("type", json!({"element": "#disabled-input", "text": "x"}));
    assert!(is_error(&td), "type disabled isError: {td}");
    assert_eq!(
        content_text(&td),
        "type: element is disabled: #disabled-input",
        "type-disabled prose byte-identical"
    );
    assert_eq!(
        structured(&td),
        json!({"code": "not_actionable", "reason": "disabled",
               "tool": "type", "element": "#disabled-input"}),
        "type-disabled code"
    );
    let tro = c.call("type", json!({"element": "#readonly-input", "text": "x"}));
    assert!(is_error(&tro), "type readonly isError: {tro}");
    assert_eq!(
        content_text(&tro),
        "type: element is read-only: #readonly-input",
        "type-readonly prose byte-identical"
    );
    assert_eq!(
        structured(&tro),
        json!({"code": "not_actionable", "reason": "readonly",
               "tool": "type", "element": "#readonly-input"}),
        "type-readonly code"
    );

    // Hover hidden (selector path).
    let hov = c.call("hover", json!({"element": "#hidden-box"}));
    assert!(is_error(&hov), "hover hidden isError: {hov}");
    assert_eq!(
        content_text(&hov),
        "hover: element not visible or not hoverable: #hidden-box",
        "hover-hidden prose byte-identical"
    );
    assert_eq!(
        structured(&hov),
        json!({"code": "not_actionable", "reason": "hidden",
               "tool": "hover", "element": "#hidden-box"}),
        "hover-hidden code"
    );

    // Ref paths: hidden click and non-select select.
    let snap = content_text(&c.call("snapshot", json!({})));
    let rhid_raw = content_text(&c.call(
        "evaluate",
        json!({"expression": "document.getElementById('hidden-box').dataset.gladiusRef || 'NOREF'"}),
    ));
    let rhid = value_to_ref(&rhid_raw);
    let hidr = c.call("click", json!({"element": rhid}));
    assert!(is_error(&hidr), "click hidden-by-ref isError: {hidr}");
    assert_eq!(
        content_text(&hidr),
        format!("click: {rhid} not visible or not clickable"),
        "click-hidden-ref prose byte-identical"
    );
    assert_eq!(
        structured(&hidr),
        json!({"code": "not_actionable", "reason": "hidden",
               "tool": "click", "ref": rhid}),
        "click-hidden-ref code"
    );
    let rbtn = find_ref(&snap, "CLICK_ME");
    let nsel = c.call("select", json!({"element": rbtn, "value": "r"}));
    assert!(is_error(&nsel), "select non-select isError: {nsel}");
    assert_eq!(
        content_text(&nsel),
        format!("select: {rbtn} is not a <select>"),
        "select-not-select prose byte-identical"
    );
    assert_eq!(
        structured(&nsel),
        json!({"code": "invalid_target", "reason": "not_select",
               "tool": "select", "ref": rbtn}),
        "select-not-select code"
    );

    // Option not found (selector path).
    let noopt = c.call(
        "select",
        json!({"element": "#color", "value": "no-such-opt-xyz"}),
    );
    assert!(is_error(&noopt), "select no-option isError: {noopt}");
    assert_eq!(
        content_text(&noopt),
        "select: option not found: no-such-opt-xyz",
        "select-noopt prose byte-identical"
    );
    assert_eq!(
        structured(&noopt),
        json!({"code": "option_not_found", "reason": "no_match",
               "tool": "select", "want": "no-such-opt-xyz"}),
        "select-noopt code"
    );

    // Upload with a missing local file: path context, not an element.
    let nf = c.call(
        "upload",
        json!({"element": "#file", "paths": ["C:/no-such-gladius-file-xyz.txt"]}),
    );
    assert!(is_error(&nf), "upload no-file isError: {nf}");
    assert_eq!(
        content_text(&nf),
        "upload: file not found: C:/no-such-gladius-file-xyz.txt",
        "upload-nofile prose byte-identical"
    );
    assert_eq!(
        structured(&nf),
        json!({"code": "file_not_found", "reason": "missing_path",
               "tool": "upload", "path": "C:/no-such-gladius-file-xyz.txt"}),
        "upload-nofile code"
    );

    finish(c);
}

#[test]
fn snapshot_structured_blocks() {
    let mut c = launch_on_fixture();

    // Normal page: two blocks, text first and unchanged, nodes one-for-one.
    let snap = c.call("snapshot", json!({}));
    assert!(!is_error(&snap), "snapshot ok: {snap}");
    let blocks = snap
        .get("content")
        .and_then(|cc| cc.as_array())
        .expect("content array");
    assert_eq!(blocks.len(), 2, "AX snapshot carries two blocks: {snap}");
    let text = blocks[0].get("text").and_then(|t| t.as_str()).unwrap_or("");
    assert!(
        text.starts_with("AX snapshot:"),
        "human block first, got:\n{text}"
    );
    assert!(
        text.contains("GLADIUS_MARKER_hello-12345"),
        "marker kept in text block"
    );
    assert!(text.contains("[snapshot:"), "node-count footer kept");
    assert!(text.chars().count() <= 8000, "8000-char cap kept");
    let second = blocks[1].get("text").and_then(|t| t.as_str()).unwrap_or("");
    let obj: Value = serde_json::from_str(second).expect("second block parses as JSON");
    assert!(
        obj.get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .contains("fixture.html"),
        "structured url: {second}"
    );
    assert_eq!(
        obj.get("title").and_then(|t| t.as_str()),
        Some("Gladius Fixture"),
        "structured title: {second}"
    );
    assert_eq!(
        obj.get("truncated"),
        Some(&json!(false)),
        "not truncated: {second}"
    );
    let nodes = obj
        .get("nodes")
        .and_then(|n| n.as_array())
        .expect("nodes array");
    let refs = text_refs(text);
    assert!(!refs.is_empty(), "text block bears refs");
    assert_eq!(
        nodes.len(),
        refs.len(),
        "nodes match text refs one-for-one ({} vs {})",
        nodes.len(),
        refs.len()
    );
    for (i, n) in nodes.iter().enumerate() {
        let r = n.get("ref").and_then(|x| x.as_str()).unwrap_or("");
        assert_eq!(format!("@{r}"), refs[i], "node {i} ref matches text");
        for k in ["ref", "role", "name", "tag", "disabled"] {
            assert!(n.get(k).is_some(), "node has '{k}': {n}");
        }
    }

    // Truncated page: footer announced, truncated true, still one-for-one.
    c.call(
        "evaluate",
        json!({"expression": "let h=''; for (let i=0;i<800;i++) h+=`<button id=\"bulk-${i}\">BULK_BUTTON_${i}_WITH_A_LONG_LABEL</button>`; document.body.insertAdjacentHTML('beforeend', h); 'bulked'"}),
    );
    let big = c.call("snapshot", json!({}));
    assert!(!is_error(&big), "big snapshot ok");
    let bblocks = big
        .get("content")
        .and_then(|cc| cc.as_array())
        .expect("content array");
    assert_eq!(bblocks.len(), 2, "truncated snapshot still two blocks");
    let btext = bblocks[0]
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or("");
    assert!(
        btext.chars().count() <= 8000,
        "bounded: {} chars",
        btext.chars().count()
    );
    assert!(
        btext.contains("[truncated:") && btext.contains("chars omitted"),
        "truncation announced in text block"
    );
    let bsecond = bblocks[1]
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or("");
    let bobj: Value = serde_json::from_str(bsecond).expect("second block parses");
    assert_eq!(
        bobj.get("truncated"),
        Some(&json!(true)),
        "truncated flag set"
    );
    let bnodes = bobj
        .get("nodes")
        .and_then(|n| n.as_array())
        .expect("nodes array");
    let brefs = text_refs(btext);
    assert_eq!(
        bnodes.len(),
        brefs.len(),
        "truncated nodes match shown refs one-for-one ({} vs {})",
        bnodes.len(),
        brefs.len()
    );
    for (i, n) in bnodes.iter().enumerate() {
        let r = n.get("ref").and_then(|x| x.as_str()).unwrap_or("");
        assert_eq!(format!("@{r}"), brefs[i], "truncated node {i} ref matches");
    }
    // The footer names more nodes than the structured block carries.
    let total: usize = btext
        .lines()
        .rev()
        .find(|l| l.contains("nodes, refs"))
        .and_then(|l| {
            l.trim_start_matches("[snapshot: ")
                .split_whitespace()
                .next()
                .and_then(|n| n.parse().ok())
        })
        .unwrap_or(0);
    assert!(
        total > bnodes.len(),
        "footer total ({total}) exceeds carried nodes ({})",
        bnodes.len()
    );

    // Text mode: legacy single block, no second block.
    let txt = c.call("snapshot", json!({"mode": "text"}));
    assert!(!is_error(&txt), "text snapshot ok: {txt}");
    let tblocks = txt
        .get("content")
        .and_then(|cc| cc.as_array())
        .expect("content array");
    assert_eq!(tblocks.len(), 1, "text mode keeps a single block: {txt}");
    let ttext = content_text(&txt);
    assert!(ttext.contains("[text-only snapshot:"), "legacy suffix kept");
    assert!(
        ttext.contains("GLADIUS_MARKER_hello-12345"),
        "marker in text mode"
    );

    finish(c);
}

#[test]
fn capabilities_manifest() {
    // Default backend: count 16, nothing gated, serverInfo untouched.
    let mut c = Client::spawn();
    let init = c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    assert_eq!(
        init.get("protocolVersion"),
        Some(&json!("2025-06-18")),
        "protocolVersion echo"
    );
    let caps = init
        .get("capabilities")
        .and_then(|cc| cc.get("tools"))
        .expect("capabilities.tools");
    assert_eq!(caps.get("count"), Some(&json!(16)), "tool count: {init}");
    assert_eq!(caps.get("gated"), Some(&json!({})), "nothing gated: {init}");
    let listed = c.result("tools/list", json!({}));
    let n = listed
        .get("tools")
        .and_then(|t| t.as_array())
        .expect("tools array")
        .len();
    assert_eq!(n, 16, "listed tools match the manifest count");
    assert_eq!(
        caps.get("count"),
        Some(&json!(n)),
        "manifest count is derived"
    );
    assert_eq!(
        init.get("serverInfo").and_then(|s| s.get("name")),
        Some(&json!("gladius")),
        "serverInfo name untouched: {init}"
    );
    shutdown(c);

    // dom-only backend: screenshot gated, and the refusal is typed.
    let mut d = Client::spawn_extra(&[("GLADIUS_BACKEND_CAPS", "dom-only")]);
    let init2 = d.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let caps2 = init2
        .get("capabilities")
        .and_then(|cc| cc.get("tools"))
        .expect("capabilities.tools");
    assert_eq!(
        caps2.get("count"),
        Some(&json!(16)),
        "dom-only count: {init2}"
    );
    assert_eq!(
        caps2.get("gated"),
        Some(&json!({"screenshot": "requires caps: render"})),
        "screenshot gated: {init2}"
    );
    assert_eq!(
        init2.get("serverInfo").and_then(|s| s.get("name")),
        Some(&json!("gladius")),
        "serverInfo name untouched: {init2}"
    );
    let shot = d.call("screenshot", json!({}));
    assert!(is_error(&shot), "dom-only screenshot refuses: {shot}");
    assert_eq!(
        content_text(&shot),
        "screenshot: unavailable with GLADIUS_BACKEND_CAPS=dom-only backend (no rendering pipeline; refusing instead of returning a fake image)",
        "refusal prose byte-identical"
    );
    assert_eq!(
        structured(&shot),
        json!({"code": "capability_gated", "reason": "dom_only", "tool": "screenshot"}),
        "refusal code"
    );
    shutdown(d);
}

#[test]
fn error_sites_caller_payload() {
    // Caller-controlled payload must not steer classification: each site
    // mints its own code/reason, so a selector/path/option name containing
    // a marker word keeps the site's type.
    let mut c = launch_on_fixture();

    // A missing selector containing "readonly" is a miss, not a readonly node.
    let miss = c.call("click", json!({"element": "#readonly-does-not-exist"}));
    assert!(is_error(&miss), "readonly-named miss isError: {miss}");
    assert_eq!(
        content_text(&miss),
        "click: element not found: #readonly-does-not-exist",
        "readonly-named miss prose byte-identical"
    );
    assert_eq!(
        structured(&miss),
        json!({"code": "invalid_target", "reason": "not_found",
               "tool": "click", "element": "#readonly-does-not-exist"}),
        "readonly-named miss code"
    );

    // A missing option literally named "is disabled" is a miss, not a disabled node.
    let opt = c.call(
        "select",
        json!({"element": "#color", "value": "is disabled"}),
    );
    assert!(is_error(&opt), "disabled-named option isError: {opt}");
    assert_eq!(
        content_text(&opt),
        "select: option not found: is disabled",
        "disabled-named option prose byte-identical"
    );
    assert_eq!(
        structured(&opt),
        json!({"code": "option_not_found", "reason": "no_match",
               "tool": "select", "want": "is disabled"}),
        "disabled-named option code"
    );

    // A missing file with "readonly" in its name is a missing path.
    let f = c.call(
        "upload",
        json!({"element": "#file", "paths": ["C:/not-existing/readonly.txt"]}),
    );
    assert!(is_error(&f), "readonly-named file isError: {f}");
    assert_eq!(
        content_text(&f),
        "upload: file not found: C:/not-existing/readonly.txt",
        "readonly-named file prose byte-identical"
    );
    assert_eq!(
        structured(&f),
        json!({"code": "file_not_found", "reason": "missing_path",
               "tool": "upload", "path": "C:/not-existing/readonly.txt"}),
        "readonly-named file code"
    );

    finish(c);
}

#[test]
fn error_context_exact_args() {
    // Context carries the real argument value whole: no invented refs, no
    // truncation at ": ".
    let mut c = launch_on_fixture();

    // A missing selector containing "@e42" reports the selector, not a ref.
    let miss = c.call("click", json!({"element": "[data-test=\"@e42\"]"}));
    assert!(is_error(&miss), "at-ref selector miss isError: {miss}");
    assert_eq!(
        content_text(&miss),
        "click: element not found: [data-test=\"@e42\"]",
        "at-ref selector miss prose byte-identical"
    );
    assert_eq!(
        structured(&miss),
        json!({"code": "invalid_target", "reason": "not_found",
               "tool": "click", "element": "[data-test=\"@e42\"]"}),
        "at-ref selector miss code"
    );
    assert!(
        structured(&miss).get("ref").is_none(),
        "no invented ref: {miss}"
    );

    // A missing file containing "@e42" reports the path whole, with no ref.
    let f = c.call(
        "upload",
        json!({"element": "#file", "paths": ["C:/not-existing/@e42.txt"]}),
    );
    assert!(is_error(&f), "at-ref file isError: {f}");
    assert_eq!(
        content_text(&f),
        "upload: file not found: C:/not-existing/@e42.txt",
        "at-ref file prose byte-identical"
    );
    assert_eq!(
        structured(&f),
        json!({"code": "file_not_found", "reason": "missing_path",
               "tool": "upload", "path": "C:/not-existing/@e42.txt"}),
        "at-ref file code"
    );
    assert!(structured(&f).get("ref").is_none(), "no invented ref: {f}");

    // A missing option containing ": " reports the whole want value.
    let opt = c.call(
        "select",
        json!({"element": "#color", "value": "alpha: beta"}),
    );
    assert!(is_error(&opt), "colon option isError: {opt}");
    assert_eq!(
        content_text(&opt),
        "select: option not found: alpha: beta",
        "colon option prose byte-identical"
    );
    assert_eq!(
        structured(&opt),
        json!({"code": "option_not_found", "reason": "no_match",
               "tool": "select", "want": "alpha: beta"}),
        "colon option code"
    );

    finish(c);
}

#[test]
fn error_typed_arg_failures() {
    // Every typed argument-failure site carries its better code - none falls
    // back to the generic `{"code":"error"}`. None of these touches the
    // browser except the out-of-range tab select at the end.
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));

    for (tool, args) in [
        ("navigate", json!({})),
        ("evaluate", json!({})),
        ("fetch_text", json!({})),
        ("key", json!({})),
    ] {
        let r = c.call(tool, args);
        assert!(is_error(&r), "{tool} missing-arg isError: {r}");
        assert_eq!(
            structured(&r),
            json!({"code": "invalid_args", "reason": "missing_arg", "tool": tool}),
            "{tool} missing-arg code"
        );
    }

    let con = c.call("console", json!({"action": "invalid"}));
    assert!(is_error(&con), "console unknown-action isError: {con}");
    assert_eq!(
        content_text(&con),
        "console: unknown action 'invalid' (get|drain|clear)",
        "console unknown-action prose byte-identical"
    );
    assert_eq!(
        structured(&con),
        json!({"code": "invalid_args", "reason": "unknown_action", "tool": "console"}),
        "console unknown-action code"
    );

    let bad = c.call("tabs", json!({"action": "select", "index": -1}));
    assert!(is_error(&bad), "tabs bad-index isError: {bad}");
    assert_eq!(
        content_text(&bad),
        "tabs: bad index -1",
        "tabs bad-index prose byte-identical"
    );
    assert_eq!(
        structured(&bad),
        json!({"code": "invalid_args", "reason": "bad_index", "tool": "tabs"}),
        "tabs bad-index code"
    );

    let unk = c.call("tabs", json!({"action": "bogus"}));
    assert!(is_error(&unk), "tabs unknown-action isError: {unk}");
    assert_eq!(
        content_text(&unk),
        "tabs: unknown action 'bogus'",
        "tabs unknown-action prose byte-identical"
    );
    assert_eq!(
        structured(&unk),
        json!({"code": "invalid_args", "reason": "unknown_action", "tool": "tabs"}),
        "tabs unknown-action code"
    );

    let need = c.call("select", json!({"element": "#color"}));
    assert!(is_error(&need), "select need-option isError: {need}");
    assert_eq!(
        content_text(&need),
        "select: need 'value' or 'text'",
        "select need-option prose byte-identical"
    );
    assert_eq!(
        structured(&need),
        json!({"code": "invalid_args", "reason": "need_option", "tool": "select"}),
        "select need-option code"
    );

    let empty = c.call("upload", json!({"element": "#file", "paths": []}));
    assert!(is_error(&empty), "upload empty-paths isError: {empty}");
    assert_eq!(
        content_text(&empty),
        "upload: 'paths' is empty",
        "upload empty-paths prose byte-identical"
    );
    assert_eq!(
        structured(&empty),
        json!({"code": "invalid_args", "reason": "missing_arg", "tool": "upload"}),
        "upload empty-paths code"
    );

    let ut = c.call("no-such-tool-xyz", json!({}));
    assert!(is_error(&ut), "unknown tool isError: {ut}");
    assert_eq!(
        content_text(&ut),
        "unknown tool: no-such-tool-xyz",
        "unknown-tool prose byte-identical"
    );
    assert_eq!(
        structured(&ut),
        json!({"code": "invalid_args", "reason": "unknown_tool", "name": "no-such-tool-xyz"}),
        "unknown-tool code"
    );

    // Out-of-range tab index (engine-raised, integer suffix): still typed.
    let far = c.call("tabs", json!({"action": "select", "index": 99}));
    assert!(is_error(&far), "tabs far-index isError: {far}");
    assert_eq!(
        content_text(&far),
        "tabs: no tab at index 99",
        "tabs far-index prose byte-identical"
    );
    assert_eq!(
        structured(&far),
        json!({"code": "invalid_target", "reason": "not_found", "tool": "tabs"}),
        "tabs far-index code"
    );

    shutdown(c);
}

#[test]
fn capabilities_invalid_caps() {
    // An invalid/unusable caps value gates the render-dependent tools
    // instead of declaring none gated while `screenshot` refuses.
    let mut c = Client::spawn_extra(&[("GLADIUS_BACKEND_CAPS", "invalid")]);
    let init = c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    assert_eq!(
        init.get("serverInfo").and_then(|s| s.get("caps")),
        Some(&json!("invalid")),
        "invalid caps reported: {init}"
    );
    let caps = init
        .get("capabilities")
        .and_then(|cc| cc.get("tools"))
        .expect("capabilities.tools");
    assert_eq!(
        caps.get("gated"),
        Some(&json!({"screenshot": "requires caps: render"})),
        "invalid caps gate screenshot: {init}"
    );
    let shot = c.call("screenshot", json!({}));
    assert!(is_error(&shot), "invalid-caps screenshot refuses: {shot}");
    assert_eq!(
        content_text(&shot),
        "screenshot: GLADIUS_BACKEND_CAPS: unknown value 'invalid' (want 'render' or 'dom-only')",
        "invalid-caps refusal prose byte-identical"
    );
    assert_eq!(
        structured(&shot),
        json!({"code": "error"}),
        "invalid-caps refusal keeps the code-only fallback"
    );
    shutdown(c);
}

#[test]
fn upload_ref_disabled_reports_ref() {
    // AX-ref upload to a disabled element: the typed not_actionable/disabled
    // error names the ref in `ref`, never the selector-shaped `element`.
    // The selector path keeps `element`; prose is byte-identical either way.
    let mut c = launch_on_fixture();
    let existing = format!("{}/Cargo.toml", env!("CARGO_MANIFEST_DIR")).replace('\\', "/");
    let snap = content_text(&c.call("snapshot", json!({})));
    let rdis = find_ref(&snap, "DISABLED_BTN");

    let by_ref = c.call("upload", json!({"element": rdis, "paths": [existing]}));
    assert!(
        is_error(&by_ref),
        "upload disabled-by-ref isError: {by_ref}"
    );
    assert_eq!(
        content_text(&by_ref),
        format!("upload: element is disabled: {rdis}"),
        "upload-disabled-ref prose byte-identical"
    );
    assert_eq!(
        structured(&by_ref),
        json!({"code": "not_actionable", "reason": "disabled",
               "tool": "upload", "ref": rdis}),
        "upload-disabled-ref code"
    );
    assert!(
        structured(&by_ref).get("element").is_none(),
        "upload-disabled-ref carries no selector field: {by_ref}"
    );

    let by_sel = c.call(
        "upload",
        json!({"element": "#disabled-btn", "paths": [existing]}),
    );
    assert!(
        is_error(&by_sel),
        "upload disabled-by-selector isError: {by_sel}"
    );
    assert_eq!(
        content_text(&by_sel),
        "upload: element is disabled: #disabled-btn",
        "upload-disabled-selector prose byte-identical"
    );
    assert_eq!(
        structured(&by_sel),
        json!({"code": "not_actionable", "reason": "disabled",
               "tool": "upload", "element": "#disabled-btn"}),
        "upload-disabled-selector code"
    );

    finish(c);
}

#[test]
fn upload_ref_notfile_reports_ref() {
    // AX-ref upload to an enabled non-file element: the typed
    // invalid_target/not_file error names the ref in `ref`, never the
    // selector-shaped `element`. The selector path keeps `element`; prose
    // is byte-identical either way.
    let mut c = launch_on_fixture();
    let existing = format!("{}/Cargo.toml", env!("CARGO_MANIFEST_DIR")).replace('\\', "/");
    let snap = content_text(&c.call("snapshot", json!({})));
    let rbtn = find_ref(&snap, "CLICK_ME");

    let by_ref = c.call("upload", json!({"element": rbtn, "paths": [existing]}));
    assert!(is_error(&by_ref), "upload notfile-by-ref isError: {by_ref}");
    assert_eq!(
        content_text(&by_ref),
        format!("upload: element is not a file input: {rbtn}"),
        "upload-notfile-ref prose byte-identical"
    );
    assert_eq!(
        structured(&by_ref),
        json!({"code": "invalid_target", "reason": "not_file",
               "tool": "upload", "ref": rbtn}),
        "upload-notfile-ref code"
    );
    assert!(
        structured(&by_ref).get("element").is_none(),
        "upload-notfile-ref carries no selector field: {by_ref}"
    );

    let by_sel = c.call("upload", json!({"element": "#toggle", "paths": [existing]}));
    assert!(
        is_error(&by_sel),
        "upload notfile-by-selector isError: {by_sel}"
    );
    assert_eq!(
        content_text(&by_sel),
        "upload: element is not a file input: #toggle",
        "upload-notfile-selector prose byte-identical"
    );
    assert_eq!(
        structured(&by_sel),
        json!({"code": "invalid_target", "reason": "not_file",
               "tool": "upload", "element": "#toggle"}),
        "upload-notfile-selector code"
    );

    finish(c);
}
