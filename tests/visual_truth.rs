//! W1 acceptance tests: truth in the proof (SPEC.md §5, items S1.1-S1.8).
//!
//! Every test drives the release binary in `mcp` mode over real stdio and the
//! owned pinned `chrome-headless-shell`, like the rest of the runtime suite.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
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

    /// A tool call that must succeed; returns the parsed reply JSON.
    fn call_ok(&mut self, name: &str, args: Value) -> Value {
        let r = self.call(name, args);
        assert!(r.get("isError").is_none(), "{name} failed: {r}");
        let text = content_text(&r);
        serde_json::from_str(&text).unwrap_or_else(|_| panic!("{name} reply is not json: {text}"))
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

/// A scratch dir unique to this process and test name.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gladius-w1-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// `file:///` URL for a local path, with forward slashes on every platform.
fn file_url(p: &std::path::Path) -> String {
    #[cfg(windows)]
    let s = p.to_string_lossy().replace('\\', "/");
    #[cfg(not(windows))]
    let s = p.to_string_lossy().to_string();
    format!("file:///{s}")
}

/// Navigate + pin the viewport, failing loudly.
fn goto(c: &mut Client, page: &std::path::Path, width: i64, height: i64) {
    let nav = c.call("navigate", json!({"url": file_url(page)}));
    assert!(nav.get("is_error").is_none() && nav.get("isError").is_none());
    let vp = c.call("viewport", json!({"width": width, "height": height}));
    assert!(vp.get("isError").is_none(), "viewport: {vp}");
}

fn num(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| x.as_i64())
}

fn flag(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(|x| x.as_bool())
}

// --------------------------------------------------------------------- S1.1

/// SPEC.md S1.1: identity and tolerance are two questions with two booleans.
///
/// The fixture is one text glyph (`H`, 20 px monospace) inside a 1 px-tall
/// clipping band, so the visible ink is two short stems. Shifting the text
/// 1 px horizontally changes two 1-2 px edge clusters - exactly the case L2-1
/// describes: on 0.7.1 the AA rule subtracts them and the compare reports
/// `pass:true` while the pixels differ. After S1.1/S1.2 the same call must say
/// `pixelsIdentical:false`, `passWithinTolerance:false`, `pass:false` and
/// still report the ignored pixels it no longer tolerates by default.
#[test]
fn compare_separates_identity_from_tolerance() {
    let dir = scratch("identity");
    let vdir = dir.join("visual");
    std::fs::create_dir_all(&vdir).expect("visual dir");
    let vdir_s = vdir.to_string_lossy().to_string();
    let mut c = Client::spawn_with_env(&[("GLADIUS_VISUAL_DIR", &vdir_s)]);
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));

    let page = dir.join("text.html");
    std::fs::write(
        &page,
        "<!doctype html><html><head><style>\
         html,body{margin:0;padding:0;background:#fff;color:#000}\
         #t{position:absolute;left:40px;top:40px;width:120px;height:1px;overflow:hidden;\
            font:20px/20px monospace}\
         #t span{position:relative;top:-8px}\
         </style></head><body><div id=\"t\"><span>H</span></div></body></html>",
    )
    .expect("write text fixture");
    goto(&mut c, &page, 800, 600);

    c.call_ok("visual", json!({"action": "save", "name": "shift"}));

    // Shift the text one whole pixel to the right (the clipping band carries
    // the transform: CSS transforms do not apply to a non-replaced inline).
    let shifted = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('t').style.transform='translateX(1px)'; 'shifted'"}),
    );
    assert!(shifted.get("isError").is_none(), "evaluate: {shifted}");

    let cmp = c.call_ok("visual", json!({"action": "compare", "name": "shift"}));

    assert_eq!(
        flag(&cmp, "pixelsIdentical"),
        Some(false),
        "the two PNGs are not identical, and the reply must say so: {cmp}"
    );
    assert_eq!(
        flag(&cmp, "passWithinTolerance"),
        Some(false),
        "an AA-subtracted edge change is not within the default tolerance: {cmp}"
    );
    assert_eq!(
        flag(&cmp, "pass"),
        Some(false),
        "`pass` is the alias of passWithinTolerance and must agree with it: {cmp}"
    );
    assert!(
        num(&cmp, "ignoredPixels").unwrap_or(0) > 0,
        "the AA rule still subtracted pixels; the reply must report them: {cmp}"
    );

    std::fs::remove_dir_all(&dir).ok();
    shutdown(c);
}

// --------------------------------------------------------------------- S1.2

/// SPEC.md S1.2: AA suppression is an opt-in, not the default.
///
/// The fixture is the one `tests/review_findings.rs:353-440` proved produces
/// exactly two AA-subtracted pixels: a 10x10 black block on white, then a 1x2
/// grey patch over the block's edge column. Compared with NO `allowIgnoredAA`
/// argument the compare must now be strict - the two ignored pixels fail it -
/// and the warning must name the flag whose absence did it.
#[test]
fn aa_suppression_is_opt_in() {
    let dir = scratch("aaopt");
    let vdir = dir.join("visual");
    std::fs::create_dir_all(&vdir).expect("visual dir");
    let vdir_s = vdir.to_string_lossy().to_string();
    let mut c = Client::spawn_with_env(&[("GLADIUS_VISUAL_DIR", &vdir_s)]);
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));

    let page = dir.join("aa.html");
    std::fs::write(
        &page,
        "<!doctype html><html><head><style>html,body{margin:0;padding:0;background:#fff}\
         #blk{position:absolute;left:0;top:0;width:10px;height:10px;background:#000}\
         </style></head><body><div id=blk></div></body></html>",
    )
    .expect("write aa fixture");
    goto(&mut c, &page, 200, 100);

    c.call_ok("visual", json!({"action": "save", "name": "aa"}));

    let painted = c.call(
        "evaluate",
        json!({"expression": "const d=document.createElement('div'); d.id='aa'; d.style='position:absolute;left:9px;top:4px;width:1px;height:2px;background:#808080'; document.body.appendChild(d); 'painted'"}),
    );
    assert!(painted.get("isError").is_none(), "evaluate: {painted}");

    // No `allowIgnoredAA` argument: the default is strict.
    let cmp = c.call_ok("visual", json!({"action": "compare", "name": "aa"}));
    assert_eq!(
        num(&cmp, "ignoredPixels"),
        Some(2),
        "the AA rule subtracts the same two pixels it always did: {cmp}"
    );
    assert_eq!(
        flag(&cmp, "passWithinTolerance"),
        Some(false),
        "with allowIgnoredAA omitted, an AA-subtracted diff does not pass: {cmp}"
    );
    let warning = cmp.get("warning").and_then(|w| w.as_str()).unwrap_or("");
    assert!(
        warning.contains("allowIgnoredAA:false"),
        "the warning names the opt-in that would re-admit the rule: {cmp}"
    );

    // The escape hatch still exists and is explicit.
    let loose = c.call_ok(
        "visual",
        json!({"action": "compare", "name": "aa", "allowIgnoredAA": true}),
    );
    assert_eq!(
        flag(&loose, "passWithinTolerance"),
        Some(true),
        "allowIgnoredAA:true re-admits the AA rule: {loose}"
    );
    assert_eq!(
        flag(&loose, "pixelsIdentical"),
        Some(false),
        "and identity is still reported honestly: {loose}"
    );

    std::fs::remove_dir_all(&dir).ok();
    shutdown(c);
}
