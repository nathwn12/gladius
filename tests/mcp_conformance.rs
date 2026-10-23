//! G1 protocol-truth conformance: strict JSON-RPC + version matrix.
//! Spawns the `gladius mcp` binary over stdio. No Chromium needed:
//! only handshake methods, tools/list, and tools/call close are used.

use std::io::{BufRead, BufReader, Write};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{json, Value};

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
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        let mut child = cmd.spawn().expect("spawn gladius mcp");
        let stdin = child.stdin.take().expect("child stdin");
        let stdout = child.stdout.take().expect("child stdout");
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

    fn send_line(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").expect("write stdin");
        self.stdin.flush().expect("flush stdin");
    }

    fn recv_timeout(&self, dur: Duration, what: &str) -> Value {
        let line = self
            .rx
            .recv_timeout(dur)
            .unwrap_or_else(|_| panic!("timeout waiting for reply to {what}"));
        serde_json::from_str(&line).unwrap_or_else(|_| panic!("bad json for {what}: {line}"))
    }

    fn try_recv(&self, dur: Duration) -> Option<Value> {
        let line = self.rx.recv_timeout(dur).ok()?;
        if line.trim().is_empty() {
            return None;
        }
        serde_json::from_str(&line).ok()
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let frame = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.send_line(&frame.to_string());
        let resp = self.recv_timeout(Duration::from_secs(15), method);
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
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn exact_tools_capabilities(caps: &Value) {
    assert_eq!(
        caps,
        &json!({"tools": {"listChanged": false}}),
        "capabilities must be exactly tools/listChanged"
    );
}

#[test]
fn initialize_matrix() {
    let mut c = Client::spawn();
    // 2025-06-18 echo
    let r = c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    assert_eq!(
        r.get("protocolVersion"),
        Some(&json!("2025-06-18")),
        "2025-06-18 echo"
    );
    exact_tools_capabilities(r.get("capabilities").expect("capabilities present"));
    let info = r.get("serverInfo").expect("serverInfo present");
    assert_eq!(info.get("name"), Some(&json!("gladius")), "server name");
    assert_eq!(
        info.get("version").and_then(|v| v.as_str()),
        Some(env!("CARGO_PKG_VERSION")),
        "server version"
    );
    let instr = r.get("instructions").and_then(|v| v.as_str()).unwrap_or("");
    assert!(!instr.is_empty(), "instructions non-empty");

    // 2025-11-25 echo
    let r = c.result("initialize", json!({"protocolVersion": "2025-11-25"}));
    assert_eq!(
        r.get("protocolVersion"),
        Some(&json!("2025-11-25")),
        "2025-11-25 echo"
    );
    exact_tools_capabilities(r.get("capabilities").expect("capabilities present"));

    // 2026-07-28 -> 2025-11-25
    let r = c.result("initialize", json!({"protocolVersion": "2026-07-28"}));
    assert_eq!(
        r.get("protocolVersion"),
        Some(&json!("2025-11-25")),
        "modern negotiates down to latest legacy"
    );
    exact_tools_capabilities(r.get("capabilities").expect("capabilities present"));
    assert!(
        r.get("resultType").is_none(),
        "initialize never carries resultType: {r}"
    );

    // unknown -> 2025-11-25
    let r = c.result("initialize", json!({"protocolVersion": "1999-01-01"}));
    assert_eq!(
        r.get("protocolVersion"),
        Some(&json!("2025-11-25")),
        "unknown negotiates down to latest legacy"
    );
    exact_tools_capabilities(r.get("capabilities").expect("capabilities present"));
    assert!(
        r.get("resultType").is_none(),
        "initialize never carries resultType: {r}"
    );
}

#[test]
fn discover_contract() {
    let mut c = Client::spawn();
    let r = c.result("server/discover", json!({}));
    assert_eq!(
        r.get("supportedVersions"),
        Some(&json!(["2025-06-18", "2025-11-25", "2026-07-28"])),
        "supported versions"
    );
    exact_tools_capabilities(r.get("capabilities").expect("capabilities present"));
    let info = r.get("serverInfo").expect("serverInfo present");
    assert_eq!(info.get("name"), Some(&json!("gladius")), "server name");
    assert_eq!(
        info.get("version").and_then(|v| v.as_str()),
        Some(env!("CARGO_PKG_VERSION")),
        "server version"
    );
    let instr = r.get("instructions").and_then(|v| v.as_str()).unwrap_or("");
    assert!(!instr.is_empty(), "instructions non-empty");
    assert_eq!(
        r.get("resultType"),
        Some(&json!("complete")),
        "discover resultType complete"
    );
}

#[test]
fn tools_list_versioning() {
    let mut c = Client::spawn();
    // modern -> resultType complete + 25 tools
    let modern = c.result(
        "tools/list",
        json!({"_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28"}}),
    );
    assert_eq!(
        modern.get("resultType"),
        Some(&json!("complete")),
        "modern tools/list resultType"
    );
    let tools = modern
        .get("tools")
        .and_then(|t| t.as_array())
        .expect("tools array");
    assert_eq!(tools.len(), 25, "25 tools, got {}", tools.len());

    // legacy -> no resultType
    let legacy = c.result("tools/list", json!({}));
    assert!(
        legacy.get("resultType").is_none(),
        "legacy tools/list no resultType: {legacy}"
    );
    let tools = legacy
        .get("tools")
        .and_then(|t| t.as_array())
        .expect("tools array");
    assert_eq!(tools.len(), 25, "25 tools legacy");
}

#[test]
fn tools_call_close_versioning() {
    let mut c = Client::spawn();
    // modern close
    let modern = c.result(
        "tools/call",
        json!({"name": "close", "arguments": {}, "_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28"}}),
    );
    assert_eq!(
        modern.get("resultType"),
        Some(&json!("complete")),
        "modern tools/call resultType"
    );
    assert!(
        modern.get("content").is_some(),
        "close content present: {modern}"
    );

    // legacy close
    let legacy = c.result("tools/call", json!({"name": "close", "arguments": {}}));
    assert!(
        legacy.get("resultType").is_none(),
        "legacy tools/call no resultType: {legacy}"
    );
    assert!(
        legacy.get("content").is_some(),
        "close content present: {legacy}"
    );
}

#[test]
fn unsupported_meta_version() {
    let mut c = Client::spawn();
    let resp = c.request(
        "tools/list",
        json!({"_meta": {"io.modelcontextprotocol/protocolVersion": "2099-01-01"}}),
    );
    let err = resp.get("error").expect("must be protocol error");
    assert_eq!(err.get("code"), Some(&json!(-32022)), "code -32022: {resp}");
    let data = err.get("data").expect("data present");
    assert_eq!(
        data.get("supported"),
        Some(&json!(["2025-06-18", "2025-11-25", "2026-07-28"])),
        "data.supported: {resp}"
    );
    assert_eq!(
        data.get("requested"),
        Some(&json!("2099-01-01")),
        "data.requested: {resp}"
    );
}

#[test]
fn unknown_methods_rejected() {
    let mut c = Client::spawn();
    for m in ["prompts/list", "resources/list", "logging/setLevel"] {
        let resp = c.request(m, json!({}));
        let err = resp.get("error").expect("must be protocol error");
        assert_eq!(err.get("code"), Some(&json!(-32601)), "{m} code: {resp}");
    }
}

#[test]
fn strict_framing_batch() {
    let mut c = Client::spawn();
    c.send_line(r#"[{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}]"#);
    let resp = c.recv_timeout(Duration::from_secs(10), "batch");
    let err = resp.get("error").expect("batch must be error");
    assert_eq!(err.get("code"), Some(&json!(-32600)), "batch code: {resp}");
    assert_eq!(resp.get("id"), Some(&json!(null)), "batch id null: {resp}");
}

#[test]
fn strict_framing_invalid_jsonrpc() {
    let mut c = Client::spawn();
    let id = c.next_id;
    c.next_id += 1;
    c.send_line(&format!(
        r#"{{"jsonrpc":"1.0","id":{id},"method":"ping","params":{{}}}}"#
    ));
    let resp = c.recv_timeout(Duration::from_secs(10), "invalid jsonrpc");
    let err = resp.get("error").expect("must be error");
    assert_eq!(err.get("code"), Some(&json!(-32600)), "code: {resp}");
    assert_eq!(resp.get("id"), Some(&json!(id)), "id echo: {resp}");
}

#[test]
fn strict_framing_methodless() {
    let mut c = Client::spawn();
    let id = c.next_id;
    c.next_id += 1;
    c.send_line(&format!(r#"{{"jsonrpc":"2.0","id":{id},"params":{{}}}}"#));
    let resp = c.recv_timeout(Duration::from_secs(10), "method-less");
    let err = resp.get("error").expect("must be error");
    assert_eq!(err.get("code"), Some(&json!(-32600)), "code: {resp}");
    assert_eq!(resp.get("id"), Some(&json!(id)), "id echo: {resp}");
}

#[test]
fn strict_framing_scalar() {
    let mut c = Client::spawn();
    c.send_line("42");
    let resp = c.recv_timeout(Duration::from_secs(10), "scalar 42");
    let err = resp.get("error").expect("scalar must be error");
    assert_eq!(err.get("code"), Some(&json!(-32600)), "code: {resp}");
    assert_eq!(resp.get("id"), Some(&json!(null)), "scalar id null: {resp}");
}

#[test]
fn strict_framing_malformed() {
    let mut c = Client::spawn();
    c.send_line("{not json");
    let resp = c.recv_timeout(Duration::from_secs(10), "malformed");
    let err = resp.get("error").expect("malformed must be error");
    assert_eq!(err.get("code"), Some(&json!(-32700)), "code: {resp}");
    assert_eq!(
        resp.get("id"),
        Some(&json!(null)),
        "malformed id null: {resp}"
    );
}

#[test]
fn strict_framing_invalid_id_types() {
    for raw in ["true", "{}", "[]"] {
        let mut c = Client::spawn();
        c.send_line(&format!(
            r#"{{"jsonrpc":"2.0","id":{raw},"method":"ping","params":{{}}}}"#
        ));
        let resp = c.recv_timeout(Duration::from_secs(10), &format!("invalid id {raw}"));
        let err = resp
            .get("error")
            .unwrap_or_else(|| panic!("invalid id {raw} must be error: {resp}"));
        assert_eq!(
            err.get("code"),
            Some(&json!(-32600)),
            "invalid id {raw} code: {resp}"
        );
        assert_eq!(
            resp.get("id"),
            Some(&json!(null)),
            "invalid id {raw} id null: {resp}"
        );
        assert!(
            resp.get("result").is_none(),
            "invalid id {raw} no result: {resp}"
        );
    }
}

#[test]
fn valid_id_types_echo() {
    let mut c = Client::spawn();
    // string id executes and echoes
    c.send_line(r#"{"jsonrpc":"2.0","id":"abc-1","method":"ping","params":{}}"#);
    let resp = c.recv_timeout(Duration::from_secs(10), "string id");
    assert!(
        resp.get("error").is_none(),
        "string id must execute: {resp}"
    );
    assert_eq!(
        resp.get("id"),
        Some(&json!("abc-1")),
        "string id echo: {resp}"
    );
    assert_eq!(
        resp.get("result"),
        Some(&json!({})),
        "string id result: {resp}"
    );
    // numeric id executes and echoes
    c.send_line(r#"{"jsonrpc":"2.0","id":42,"method":"ping","params":{}}"#);
    let resp = c.recv_timeout(Duration::from_secs(10), "numeric id");
    assert!(
        resp.get("error").is_none(),
        "numeric id must execute: {resp}"
    );
    assert_eq!(resp.get("id"), Some(&json!(42)), "numeric id echo: {resp}");
    assert_eq!(
        resp.get("result"),
        Some(&json!({})),
        "numeric id result: {resp}"
    );
}

/// G2b: every tool carries a version-independent outputSchema + annotations.
fn expected_hints(name: &str) -> (bool, bool, bool, bool) {
    match name {
        "navigate" => (false, false, true, true),
        "snapshot" => (true, false, false, false),
        "screenshot" => (true, false, true, false),
        "click" => (false, true, false, true),
        "type" => (false, true, false, true),
        "evaluate" => (false, true, false, true),
        "wait_for" => (true, false, true, false),
        "targets" => (false, false, true, false),
        "dialog" => (false, false, true, false),
        "history" => (false, false, false, true),
        "network" => (false, false, false, false),
        "cdp" => (false, true, false, true),
        "console" => (false, false, false, false),
        "upload" => (false, true, true, false),
        "fetch_text" => (true, false, true, true),
        "hover" => (false, false, true, true),
        "select" => (false, false, true, false),
        "key" => (false, true, false, true),
        "viewport" => (false, false, true, false),
        "tabs" => (false, true, false, true),
        "close" => (false, true, true, false),
        "storage" => (false, true, false, false),
        // Geometry without pixels: read-only. `assert_layout` may write
        // one screenshot on failure, which is still a read of the page.
        "assert_layout" => (true, false, true, false),
        "layout" => (true, false, true, false),
        // Writes baseline files (and can delete them): not read-only.
        "visual" => (false, true, false, false),
        other => panic!("unexpected tool {other}"),
    }
}

fn assert_tool_schemas(listed: &Value, label: &str) {
    let tools = listed
        .get("tools")
        .and_then(|t| t.as_array())
        .expect("tools array");
    assert_eq!(tools.len(), 25, "25 tools ({label})");
    for t in tools {
        let name = t.get("name").and_then(|n| n.as_str()).unwrap_or("?");
        let out = t
            .get("outputSchema")
            .unwrap_or_else(|| panic!("{name} missing outputSchema ({label}): {t}"));
        assert_eq!(
            out.get("type"),
            Some(&json!("object")),
            "{name} outputSchema type object ({label})"
        );
        let props = out
            .get("properties")
            .and_then(|p| p.as_object())
            .unwrap_or_else(|| panic!("{name} outputSchema.properties missing ({label})"));
        assert!(
            !props.is_empty(),
            "{name} outputSchema.properties non-empty ({label})"
        );
        assert!(
            out.get("required").and_then(|r| r.as_array()).is_some(),
            "{name} outputSchema.required present ({label})"
        );
        assert_eq!(
            out.get("additionalProperties"),
            Some(&json!(false)),
            "{name} additionalProperties false ({label})"
        );
        let title = t.get("title").and_then(|x| x.as_str()).unwrap_or("");
        assert!(
            !title.is_empty(),
            "{name} Tool-level title present ({label})"
        );
        let ann = t
            .get("annotations")
            .unwrap_or_else(|| panic!("{name} missing annotations ({label})"));
        let (r, d, i, o) = expected_hints(name);
        assert_eq!(
            ann.get("readOnlyHint"),
            Some(&json!(r)),
            "{name} readOnlyHint ({label})"
        );
        assert_eq!(
            ann.get("destructiveHint"),
            Some(&json!(d)),
            "{name} destructiveHint ({label})"
        );
        assert_eq!(
            ann.get("idempotentHint"),
            Some(&json!(i)),
            "{name} idempotentHint ({label})"
        );
        assert_eq!(
            ann.get("openWorldHint"),
            Some(&json!(o)),
            "{name} openWorldHint ({label})"
        );
    }
}

#[test]
fn tools_list_schemas_annotations() {
    let mut c = Client::spawn();
    let modern = c.result(
        "tools/list",
        json!({"_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28"}}),
    );
    assert_tool_schemas(&modern, "modern");
    let legacy = c.result("tools/list", json!({}));
    assert_tool_schemas(&legacy, "legacy");
}

#[test]
fn tools_call_close_structured() {
    let mut c = Client::spawn();
    let modern = c.result(
        "tools/call",
        json!({"name": "close", "arguments": {}, "_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28"}}),
    );
    assert_eq!(
        modern.get("resultType"),
        Some(&json!("complete")),
        "modern close resultType"
    );
    assert!(modern.get("isError").is_none(), "modern close ok: {modern}");
    let mtext = modern
        .get("content")
        .and_then(|cc| cc.get(0))
        .and_then(|b| b.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    assert_eq!(mtext, "{\"ok\":true}", "modern close text byte-identical");
    assert_eq!(
        modern.get("structuredContent"),
        Some(&json!({"ok": true})),
        "modern close structuredContent"
    );

    let legacy = c.result("tools/call", json!({"name": "close", "arguments": {}}));
    assert!(
        legacy.get("resultType").is_none(),
        "legacy close no resultType: {legacy}"
    );
    assert!(legacy.get("isError").is_none(), "legacy close ok: {legacy}");
    let ltext = legacy
        .get("content")
        .and_then(|cc| cc.get(0))
        .and_then(|b| b.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    assert_eq!(ltext, "{\"ok\":true}", "legacy close text byte-identical");
    assert_eq!(
        legacy.get("structuredContent"),
        Some(&json!({"ok": true})),
        "legacy close structuredContent"
    );
}

#[test]
fn notification_silence_then_ping() {
    let mut c = Client::spawn();
    // notification: no id -> no response
    c.send_line(r#"{"jsonrpc":"2.0","method":"ping","params":{}}"#);
    let got = c.try_recv(Duration::from_secs(2));
    assert!(got.is_none(), "notification must be silent, got {got:?}");
    // server still alive
    let pong = c.result("ping", json!({}));
    assert_eq!(pong, json!({}), "ping replies {{}}");
}
