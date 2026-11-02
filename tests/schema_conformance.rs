//! Schema conformance: every tool's success payload matches its own
//! `outputSchema` (required keys present, every present key's JSON type
//! allowed, no undeclared keys). One case per tool against the real handler
//! over MCP stdio; a tool with no case fails via the completeness assertion
//! so a new tool cannot slip past.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{json, Value};

fn fixture_bytes() -> Vec<u8> {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("fixture.html");
    std::fs::read(p).expect("read fixture")
}

fn serve_fixture() -> u16 {
    let body = fixture_bytes();
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
    fn spawn_extra(env_extra: &[(&str, &str)]) -> Self {
        let bin = env!("CARGO_BIN_EXE_gladius");
        let mut cmd = Command::new(bin);
        cmd.arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
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

fn content_text(v: &Value) -> String {
    // screenshot carries image+text; the JSON meta is the text block.
    if let Some(arr) = v.get("content").and_then(|c| c.as_array()) {
        for b in arr {
            if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                    // Prefer a block that parses as JSON (the meta); fall
                    // back to the first text block.
                    if serde_json::from_str::<Value>(t).is_ok() {
                        return t.to_string();
                    }
                }
            }
        }
        // Fallback: first text block (snapshot text mode has one block).
        if let Some(t) = arr
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .next()
        {
            return t.to_string();
        }
    }
    String::new()
}

fn structured(v: &Value) -> Value {
    v.get("structuredContent").cloned().unwrap_or(Value::Null)
}

fn is_error(v: &Value) -> bool {
    v.get("isError").is_some()
}

fn json_type_ok(v: &Value, t: &str) -> bool {
    match t {
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        "null" => v.is_null(),
        _ => true,
    }
}

fn allows(v: &Value, s: &Value) -> bool {
    let Some(obj) = s.as_object() else {
        return true;
    };
    if obj.is_empty() {
        return true;
    }
    if let Some(t) = obj.get("type") {
        if let Some(ts) = t.as_str() {
            return json_type_ok(v, ts);
        }
        if let Some(arr) = t.as_array() {
            return arr
                .iter()
                .any(|x| x.as_str().map(|ts| json_type_ok(v, ts)).unwrap_or(false));
        }
    }
    if let Some(any) = obj.get("anyOf").and_then(|a| a.as_array()) {
        return any.iter().any(|b| allows(v, b));
    }
    if let Some(one) = obj.get("oneOf").and_then(|o| o.as_array()) {
        return one.iter().any(|b| allows(v, b));
    }
    true
}

fn assert_conforms(tool: &str, sc: &Value, schema: &Value, args: &Value) {
    assert!(
        sc.is_object(),
        "{tool} {args} structuredContent is an object: {sc}"
    );
    let props = schema
        .get("properties")
        .and_then(|p| p.as_object())
        .unwrap_or_else(|| panic!("{tool} schema missing properties"));
    assert_eq!(
        schema.get("type"),
        Some(&json!("object")),
        "{tool} outputSchema type object"
    );
    assert_eq!(
        schema.get("additionalProperties"),
        Some(&json!(false)),
        "{tool} additionalProperties false"
    );
    let top_req: Vec<String> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let mut branches: Vec<Vec<String>> = Vec::new();
    for k in ["anyOf", "oneOf"] {
        if let Some(arr) = schema.get(k).and_then(|a| a.as_array()) {
            for b in arr {
                let req: Vec<String> = b
                    .get("required")
                    .and_then(|r| r.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                if !req.is_empty() {
                    branches.push(req);
                }
            }
        }
    }
    let holds = |keys: &[String]| -> bool {
        keys.iter().all(|key| match sc.get(key) {
            None => false,
            Some(v) => match props.get(key) {
                None => false,
                Some(ps) => allows(v, ps),
            },
        })
    };
    if branches.is_empty() {
        for key in &top_req {
            let v = sc
                .get(key)
                .unwrap_or_else(|| panic!("{tool} {args} missing required '{key}': {sc}"));
            let ps = props.get(key).unwrap_or_else(|| {
                panic!("{tool} {args} required '{key}' not in properties: {sc}")
            });
            assert!(
                allows(v, ps),
                "{tool} {args} required '{key}' type mismatch: {v} vs {ps} in {sc}"
            );
        }
    } else {
        // Top required (usually empty) must hold, plus at least one branch.
        for key in &top_req {
            let v = sc
                .get(key)
                .unwrap_or_else(|| panic!("{tool} {args} missing top required '{key}': {sc}"));
            let ps = props
                .get(key)
                .unwrap_or_else(|| panic!("{tool} {args} top required '{key}' not in properties"));
            assert!(
                allows(v, ps),
                "{tool} {args} top required '{key}' type mismatch: {v} vs {ps}"
            );
        }
        assert!(
            branches.iter().any(|b| holds(b)),
            "{tool} {args} matches no anyOf/oneOf branch: {sc} vs {schema}"
        );
    }
    // Every present key must be declared and type-allowed.
    let obj = sc.as_object().expect("object");
    for (key, val) in obj {
        let ps = props.get(key).unwrap_or_else(|| {
            panic!("{tool} {args} undeclared key '{key}' (additionalProperties:false): {sc}")
        });
        assert!(
            allows(val, ps),
            "{tool} {args} key '{key}' type mismatch: {val} vs {ps} in {sc}"
        );
    }
}

fn do_check(
    c: &mut Client,
    schemas: &HashMap<String, Value>,
    covered: &mut HashSet<String>,
    tool: &str,
    args: Value,
) -> Value {
    let r = c.call(tool, args.clone());
    assert!(!is_error(&r), "{tool} {args} ok: {r}");
    let sc = structured(&r);
    assert_conforms(tool, &sc, &schemas[tool], &args);
    let t = content_text(&r);
    if tool != "screenshot" {
        if let Ok(parsed) = serde_json::from_str::<Value>(&t) {
            if parsed.is_object() {
                assert_eq!(
                    sc, parsed,
                    "{tool} {args} structuredContent equals text parse"
                );
            }
        }
    } else {
        let parsed: Value =
            serde_json::from_str(&t).unwrap_or_else(|_| panic!("screenshot meta parses: {t}"));
        assert_eq!(sc, parsed, "screenshot structured equals meta");
    }
    covered.insert(tool.to_string());
    sc
}

#[test]
fn schema_conformance_all_tools() {
    let port = serve_fixture();
    let http_url = format!("http://127.0.0.1:{port}/fixture.html");
    let dir = tempfile::tempdir().expect("tempdir");
    let vdir = dir.path().join("visual").to_string_lossy().to_string();
    let mut c = Client::spawn_extra(&[("GLADIUS_VISUAL_DIR", vdir.as_str())]);
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let listed = c.result("tools/list", json!({}));
    let tools = listed
        .get("tools")
        .and_then(|t| t.as_array())
        .expect("tools array");
    assert_eq!(tools.len(), 25, "25 tools listed");
    let mut schemas: HashMap<String, Value> = HashMap::new();
    for t in tools {
        let n = t.get("name").and_then(|n| n.as_str()).unwrap().to_string();
        schemas.insert(n, t.get("outputSchema").expect("outputSchema").clone());
    }
    let mut covered: HashSet<String> = HashSet::new();

    // navigate first: all later tools act on this page (http origin so storage works).
    do_check(
        &mut c,
        &schemas,
        &mut covered,
        "navigate",
        json!({"url": http_url}),
    );
    // history reload keeps the same http page.
    do_check(
        &mut c,
        &schemas,
        &mut covered,
        "history",
        json!({"action": "reload"}),
    );
    // snapshot both branches (AX + text) cover the anyOf.
    do_check(&mut c, &schemas, &mut covered, "snapshot", json!({}));
    {
        let r = c.call("snapshot", json!({"mode": "text"}));
        assert!(!is_error(&r), "snapshot text ok: {r}");
        assert_conforms(
            "snapshot",
            &structured(&r),
            &schemas["snapshot"],
            &json!({"mode":"text"}),
        );
        covered.insert("snapshot".to_string());
    }
    do_check(&mut c, &schemas, &mut covered, "screenshot", json!({}));
    {
        let r = c.call("screenshot", json!({"deterministic": true}));
        assert!(!is_error(&r), "screenshot deterministic ok: {r}");
        assert_conforms(
            "screenshot",
            &structured(&r),
            &schemas["screenshot"],
            &json!({"deterministic": true}),
        );
        covered.insert("screenshot".to_string());
    }
    do_check(
        &mut c,
        &schemas,
        &mut covered,
        "click",
        json!({"element": "#toggle"}),
    );
    do_check(
        &mut c,
        &schemas,
        &mut covered,
        "type",
        json!({"element": "#name", "text": "hello-42"}),
    );
    do_check(
        &mut c,
        &schemas,
        &mut covered,
        "hover",
        json!({"element": "#toggle"}),
    );
    do_check(
        &mut c,
        &schemas,
        &mut covered,
        "select",
        json!({"element": "#color", "value": "g"}),
    );
    do_check(
        &mut c,
        &schemas,
        &mut covered,
        "key",
        json!({"key": "Escape"}),
    );
    do_check(
        &mut c,
        &schemas,
        &mut covered,
        "evaluate",
        json!({"expression": "1+1"}),
    );
    do_check(&mut c, &schemas, &mut covered, "wait_for", json!({"ms": 0}));
    do_check(
        &mut c,
        &schemas,
        &mut covered,
        "viewport",
        json!({"width": 800, "height": 600}),
    );
    {
        let r = c.call("viewport", json!({"reset": true}));
        assert!(!is_error(&r), "viewport reset ok: {r}");
        assert_conforms(
            "viewport",
            &structured(&r),
            &schemas["viewport"],
            &json!({"reset": true}),
        );
        covered.insert("viewport".to_string());
    }
    do_check(
        &mut c,
        &schemas,
        &mut covered,
        "tabs",
        json!({"action": "list"}),
    );
    // targets both branches: list (targets) + select (tabs).
    do_check(
        &mut c,
        &schemas,
        &mut covered,
        "targets",
        json!({"action": "list"}),
    );
    {
        let list_sc = structured(&c.call("targets", json!({"action": "list"})));
        let tid = list_sc
            .get("targets")
            .and_then(|t| t.as_array())
            .and_then(|a| a.first())
            .and_then(|t| t.get("targetId"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        assert!(!tid.is_empty(), "targets list has a targetId: {list_sc}");
        let r = c.call("targets", json!({"action": "select", "target": tid}));
        assert!(!is_error(&r), "targets select ok: {r}");
        assert_conforms(
            "targets",
            &structured(&r),
            &schemas["targets"],
            &json!({"action": "select"}),
        );
        covered.insert("targets".to_string());
    }
    do_check(
        &mut c,
        &schemas,
        &mut covered,
        "dialog",
        json!({"action": "get"}),
    );
    {
        let r = c.call("dialog", json!({"action": "policy", "accept": true}));
        assert!(!is_error(&r), "dialog policy ok: {r}");
        assert_conforms(
            "dialog",
            &structured(&r),
            &schemas["dialog"],
            &json!({"action": "policy"}),
        );
        covered.insert("dialog".to_string());
    }
    do_check(
        &mut c,
        &schemas,
        &mut covered,
        "network",
        json!({"action": "get"}),
    );
    {
        let r = c.call("network", json!({"action": "clear"}));
        assert!(!is_error(&r), "network clear ok: {r}");
        assert_conforms(
            "network",
            &structured(&r),
            &schemas["network"],
            &json!({"action": "clear"}),
        );
        covered.insert("network".to_string());
    }
    do_check(
        &mut c,
        &schemas,
        &mut covered,
        "console",
        json!({"action": "get"}),
    );
    {
        let r = c.call("console", json!({"action": "clear"}));
        assert!(!is_error(&r), "console clear ok: {r}");
        assert_conforms(
            "console",
            &structured(&r),
            &schemas["console"],
            &json!({"action": "clear"}),
        );
        covered.insert("console".to_string());
    }
    do_check(
        &mut c,
        &schemas,
        &mut covered,
        "cdp",
        json!({"method": "Browser.getVersion"}),
    );
    // upload needs a live file input.
    c.call(
        "evaluate",
        json!({"expression": "document.body.insertAdjacentHTML('beforeend', '<input type=file id=upfile>'); 'added'"}),
    );
    {
        let existing = format!("{}/Cargo.toml", env!("CARGO_MANIFEST_DIR")).replace('\\', "/");
        do_check(
            &mut c,
            &schemas,
            &mut covered,
            "upload",
            json!({"element": "#upfile", "paths": [existing]}),
        );
    }
    do_check(
        &mut c,
        &schemas,
        &mut covered,
        "fetch_text",
        json!({"url": http_url}),
    );
    // storage: every action shape shares {ok,action} but carries different keys.
    do_check(
        &mut c,
        &schemas,
        &mut covered,
        "storage",
        json!({"action": "get", "scope": "all"}),
    );
    {
        let r = c.call(
            "storage",
            json!({"action": "set", "scope": "localStorage", "items": {"conf-key": "conf-val"}}),
        );
        assert!(!is_error(&r), "storage set ok: {r}");
        assert_conforms(
            "storage",
            &structured(&r),
            &schemas["storage"],
            &json!({"action": "set"}),
        );
        covered.insert("storage".to_string());
    }
    {
        let r = c.call(
            "storage",
            json!({"action": "clear", "scope": "localStorage"}),
        );
        assert!(!is_error(&r), "storage clear ok: {r}");
        assert_conforms(
            "storage",
            &structured(&r),
            &schemas["storage"],
            &json!({"action": "clear"}),
        );
    }
    {
        let sp = dir.path().join("store.json").to_string_lossy().to_string();
        let r = c.call("storage", json!({"action": "save", "path": sp}));
        assert!(!is_error(&r), "storage save ok: {r}");
        assert_conforms(
            "storage",
            &structured(&r),
            &schemas["storage"],
            &json!({"action": "save"}),
        );
        let r2 = c.call("storage", json!({"action": "load", "path": sp}));
        assert!(!is_error(&r2), "storage load ok: {r2}");
        assert_conforms(
            "storage",
            &structured(&r2),
            &schemas["storage"],
            &json!({"action": "load"}),
        );
    }
    do_check(
        &mut c,
        &schemas,
        &mut covered,
        "assert_layout",
        json!({"selector": "#toggle"}),
    );
    do_check(
        &mut c,
        &schemas,
        &mut covered,
        "layout",
        json!({"selector": "#toggle"}),
    );
    // visual: all four file/pixel shapes share {ok,action}.
    do_check(
        &mut c,
        &schemas,
        &mut covered,
        "visual",
        json!({"action": "list"}),
    );
    {
        let r = c.call("visual", json!({"action": "save", "name": "conf"}));
        assert!(!is_error(&r), "visual save ok: {r}");
        assert_conforms(
            "visual",
            &structured(&r),
            &schemas["visual"],
            &json!({"action": "save"}),
        );
        covered.insert("visual".to_string());
    }
    {
        let r = c.call("visual", json!({"action": "compare", "name": "conf"}));
        assert!(!is_error(&r), "visual compare ok: {r}");
        assert_conforms(
            "visual",
            &structured(&r),
            &schemas["visual"],
            &json!({"action": "compare"}),
        );
    }
    {
        let r = c.call("visual", json!({"action": "delete", "name": "conf"}));
        assert!(!is_error(&r), "visual delete ok: {r}");
        assert_conforms(
            "visual",
            &structured(&r),
            &schemas["visual"],
            &json!({"action": "delete"}),
        );
    }
    // tabs new/select/close share {tabs,ok}; list already covers, exercise the rest.
    {
        let r = c.call("tabs", json!({"action": "new", "url": http_url}));
        assert!(!is_error(&r), "tabs new ok: {r}");
        assert_conforms(
            "tabs",
            &structured(&r),
            &schemas["tabs"],
            &json!({"action": "new"}),
        );
        covered.insert("tabs".to_string());
    }
    {
        let r = c.call("tabs", json!({"action": "select", "index": 0}));
        assert!(!is_error(&r), "tabs select ok: {r}");
        assert_conforms(
            "tabs",
            &structured(&r),
            &schemas["tabs"],
            &json!({"action": "select"}),
        );
    }
    {
        let r = c.call("tabs", json!({"action": "close", "index": 1}));
        assert!(!is_error(&r), "tabs close ok: {r}");
        assert_conforms(
            "tabs",
            &structured(&r),
            &schemas["tabs"],
            &json!({"action": "close"}),
        );
    }
    do_check(&mut c, &schemas, &mut covered, "close", json!({}));

    let expected: HashSet<String> = [
        "navigate",
        "snapshot",
        "screenshot",
        "click",
        "type",
        "evaluate",
        "wait_for",
        "targets",
        "dialog",
        "history",
        "network",
        "cdp",
        "console",
        "upload",
        "fetch_text",
        "hover",
        "select",
        "key",
        "viewport",
        "tabs",
        "close",
        "storage",
        "assert_layout",
        "layout",
        "visual",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(
        covered,
        expected,
        "every tool has a case (missing: {:?}, extra: {:?})",
        expected.difference(&covered).collect::<Vec<_>>(),
        covered.difference(&expected).collect::<Vec<_>>()
    );
    assert_eq!(schemas.len(), 25, "25 schemas");
    let closed = structured(&c.call("close", json!({})));
    assert_eq!(
        closed.get("ok"),
        Some(&json!(true)),
        "close idempotent: {closed}"
    );
    shutdown(c);
}
