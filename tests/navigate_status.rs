//! F1 proof: `navigate` must not fabricate `"status": 200`.
//! A loopback server answers 404; the navigate envelope must NOT carry a
//! `status` field at all (a wrong field is worse than no field).
//! Headless only (real browser via GLADIUS_CHROME).

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
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

/// Loopback server that answers 404 to every request (local only).
fn serve_404() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind localhost");
    let port = listener.local_addr().expect("local addr").port();
    std::thread::spawn(move || {
        let body =
            "<!doctype html><html><head><title>Missing Page</title></head><body>nope-404</body></html>";
        for s in listener.incoming().flatten() {
            let mut s = s;
            let mut r = BufReader::new(s.try_clone().expect("clone socket"));
            let mut line = String::new();
            // Consume the request head so Chrome's GET completes.
            loop {
                line.clear();
                match r.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if line == "\r\n" {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let resp = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = s.write_all(resp.as_bytes());
            let _ = s.flush();
            // `s` (and the buffered clone `r`) drop here: connection closes.
            let _ = r.into_inner();
        }
    });
    port
}

#[test]
fn navigate_does_not_fabricate_status() {
    let mut c = Client::spawn();
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));

    let port = serve_404();
    let url = format!("http://127.0.0.1:{port}/missing-thing");
    let nav = c.call("navigate", json!({"url": url}));
    assert!(nav.get("isError").is_none(), "navigate 404 ok: {nav}");
    let text = content_text(&nav);
    let obj: Value = serde_json::from_str(&text).expect("navigate json");
    assert!(
        obj.get("url").and_then(|u| u.as_str()).unwrap_or("") == url,
        "navigate reports the real url: {text}"
    );
    assert!(
        obj.get("status").is_none(),
        "navigate must not fabricate status (got: {text})"
    );

    let closed = c.call("close", json!({}));
    assert!(closed.get("isError").is_none(), "close ok: {closed}");
    drop(c.stdin);
    let mut child = c.child;
    for _ in 0..100 {
        match child.try_wait() {
            Ok(Some(status)) => {
                assert!(status.success(), "server exit status: {status}");
                return;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => panic!("try_wait: {e}"),
        }
    }
    child.kill().ok();
    panic!("server did not exit after stdin EOF");
}
