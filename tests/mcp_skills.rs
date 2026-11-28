//! MCP skills (SEP-2640) integration test.

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
        cmd.creation_flags(0x08000000);
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

    fn recv_timeout(&self, dur: Duration, what: &str) -> Value {
        let line = self
            .rx
            .recv_timeout(dur)
            .unwrap_or_else(|_| panic!("timeout waiting for reply to {what}"));
        serde_json::from_str(&line).unwrap_or_else(|_| panic!("bad json for {what}: {line}"))
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let frame = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{frame}").expect("write stdin");
        self.stdin.flush().expect("flush stdin");
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

fn sha256_hex(bytes: &[u8]) -> String {
    use ring::digest;
    let d = digest::digest(&digest::SHA256, bytes);
    let mut out = String::with_capacity(64);
    for b in d.as_ref() {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[test]
fn skills_capability_and_list() {
    let mut c = Client::spawn();
    let r = c.result("initialize", json!({"protocolVersion": "2025-11-25"}));
    let caps = r.get("capabilities").expect("caps");
    assert!(
        caps.get("resources").is_some(),
        "resources capability present"
    );
    assert!(
        caps.get("extensions").is_some(),
        "extensions capability present"
    );
    let ext = caps.get("extensions").unwrap();
    assert!(
        ext.get("io.modelcontextprotocol/skills").is_some(),
        "skills extension present"
    );

    let r = c.result("skills/list", json!({}));
    assert_eq!(
        r.get("resultType"),
        Some(&json!("complete")),
        "skills/list resultType: {r}"
    );
    let skills = r
        .get("skills")
        .and_then(|v| v.as_array())
        .expect("skills array");
    assert_eq!(skills.len(), 1, "one seed skill");
    let skill = &skills[0];
    assert_eq!(skill.get("uri"), Some(&json!("skill://sessions/SKILL.md")));
    assert_eq!(
        skill.get("name"),
        Some(&json!("sessions")),
        "entry-level name"
    );
    let fm = skill.get("frontmatter").expect("frontmatter");
    assert_eq!(fm.get("name"), Some(&json!("sessions")));
    let desc = fm.get("description").and_then(|v| v.as_str()).unwrap_or("");
    assert!(!desc.is_empty() && desc.len() <= 1024);

    let expected_bytes: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/skills/sessions/SKILL.md"
    ));
    let resources = skill
        .get("resources")
        .and_then(|v| v.as_array())
        .expect("resources");
    // includes SKILL.md
    assert!(resources
        .iter()
        .any(|r| r.get("uri") == Some(&json!("skill://sessions/SKILL.md"))));
    // sorted by uri
    let uris: Vec<&str> = resources
        .iter()
        .filter_map(|r| r.get("uri").and_then(|u| u.as_str()))
        .collect();
    let mut sorted = uris.clone();
    sorted.sort();
    assert_eq!(uris, sorted, "resources sorted by uri");
    // digest/size for SKILL.md
    let md_res = resources
        .iter()
        .find(|r| r.get("uri") == Some(&json!("skill://sessions/SKILL.md")))
        .unwrap();
    assert_eq!(md_res.get("size"), Some(&json!(expected_bytes.len())));
    assert_eq!(
        md_res.get("digest"),
        Some(&json!(format!("sha256:{}", sha256_hex(expected_bytes))))
    );

    assert_eq!(r.get("ttlMs"), Some(&json!(0)));
    assert_eq!(r.get("cacheScope"), Some(&json!("private")));
}

#[test]
fn skills_get_and_resources_read() {
    let mut c = Client::spawn();
    let _ = c.result("initialize", json!({"protocolVersion": "2025-11-25"}));

    let r = c.result("skills/get", json!({"uri": "skill://sessions/SKILL.md"}));
    assert_eq!(
        r.get("resultType"),
        Some(&json!("complete")),
        "skills/get resultType: {r}"
    );
    let skill = r.get("skill").expect("skill");
    assert_eq!(skill.get("uri"), Some(&json!("skill://sessions/SKILL.md")));
    assert_eq!(
        skill.get("name"),
        Some(&json!("sessions")),
        "get entry-level name"
    );
    assert_eq!(r.get("ttlMs"), Some(&json!(0)));
    assert_eq!(r.get("cacheScope"), Some(&json!("private")));

    let r = c.result(
        "resources/read",
        json!({"uri": "skill://sessions/SKILL.md"}),
    );
    let contents = r
        .get("contents")
        .and_then(|v| v.as_array())
        .expect("contents");
    assert_eq!(contents.len(), 1);
    let c0 = &contents[0];
    assert_eq!(c0.get("uri"), Some(&json!("skill://sessions/SKILL.md")));
    assert_eq!(c0.get("mimeType"), Some(&json!("text/markdown")));
    let text = c0.get("text").and_then(|v| v.as_str()).expect("text");
    let expected = std::str::from_utf8(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/skills/sessions/SKILL.md"
    )))
    .unwrap();
    assert_eq!(text, expected);

    // unknown skill uri -> -32602
    let resp = c.request("skills/get", json!({"uri": "skill://nope/SKILL.md"}));
    assert!(resp.get("error").is_some(), "error present");
    assert_eq!(resp.get("error").unwrap().get("code"), Some(&json!(-32602)));

    let resp = c.request("resources/read", json!({"uri": "skill://nope/SKILL.md"}));
    assert!(resp.get("error").is_some());
    assert_eq!(resp.get("error").unwrap().get("code"), Some(&json!(-32602)));
}
