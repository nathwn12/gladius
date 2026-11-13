//! W3 acceptance tests: the pass bar is tested (SPEC.md §7, S3.2).
//!
//! Four arguments decide a `visual compare` verdict - `threshold`,
//! `maxDiffPixels`, `maxDiffRatio`, `maskRects` - and before S3.2 no test
//! exercised any of them (L5 §2), so one silently ignored argument would have
//! gone red nowhere. Each case below is one assertion wide: the SAME diff
//! passes on one side of the bar and fails on the other, which is only
//! possible if the argument is read.
//!
//! Every test drives the release binary in `mcp` mode over real stdio and the
//! owned pinned `chrome-headless-shell`.

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
        "gladius-w3-{tag}-{}-{}",
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

/// Spawn a server whose baseline store is the test's own scratch dir, write
/// `html` to a page inside it, navigate and pin the viewport.
fn open(tag: &str, html: &str, width: i64, height: i64) -> (Client, PathBuf, PathBuf) {
    let dir = scratch(tag);
    let vdir = dir.join("visual");
    std::fs::create_dir_all(&vdir).expect("visual dir");
    let mut c = Client::spawn_with_env(&[("GLADIUS_VISUAL_DIR", &vdir.to_string_lossy())]);
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    let page = dir.join("page.html");
    std::fs::write(&page, html).expect("write page");
    let nav = c.call("navigate", json!({"url": file_url(&page)}));
    assert!(nav.get("isError").is_none(), "navigate: {nav}");
    let vp = c.call(
        "viewport",
        json!({"width": width, "height": height, "deviceScaleFactor": 1}),
    );
    assert!(vp.get("isError").is_none(), "viewport: {vp}");
    (c, dir, page)
}

fn num(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(|x| x.as_i64()).unwrap_or(-1)
}

fn flag(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(|x| x.as_bool())
}

fn ratio(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(|x| x.as_f64()).unwrap_or(-1.0)
}

// --------------------------------------------------------------------- S3.2

/// SPEC.md S3.2 case 1: `threshold` flips the verdict.
///
/// The page is one flat block repainted from `rgb(100,100,100)` to
/// `rgb(120,120,120)`: every channel moves 20 levels. At `threshold:0.1`
/// (25.5 levels) nothing counts and the compare passes; at `threshold:0.05`
/// (12.75 levels) every pixel counts and it fails. A compare that ignored the
/// argument would answer the same both times.
#[test]
fn threshold_flip_changes_the_verdict() {
    let (mut c, dir, _page) = open(
        "threshold",
        "<!doctype html><html><head><style>html,body{margin:0;padding:0;background:#fff}\
         #blk{position:absolute;left:0;top:0;width:200px;height:100px;background:rgb(100,100,100)}\
         </style></head><body><div id=blk></div></body></html>",
        200,
        100,
    );

    c.call_ok("visual", json!({"action": "save", "name": "t"}));
    let painted = c.call(
        "evaluate",
        json!({"expression": "document.getElementById('blk').style.background='rgb(120,120,120)'; 'repainted'"}),
    );
    assert!(painted.get("isError").is_none(), "evaluate: {painted}");

    let loose = c.call_ok(
        "visual",
        json!({"action": "compare", "name": "t", "threshold": 0.1}),
    );
    assert_eq!(
        flag(&loose, "passWithinTolerance"),
        Some(true),
        "a 20-level shift is under a 0.1 threshold: {loose}"
    );
    assert_eq!(
        num(&loose, "diffPixels"),
        0,
        "and no pixel counts as changed: {loose}"
    );
    assert_eq!(
        flag(&loose, "pixelsIdentical"),
        Some(false),
        "the PNGs are not identical, and that is reported separately: {loose}"
    );

    let strict = c.call_ok(
        "visual",
        json!({"action": "compare", "name": "t", "threshold": 0.05}),
    );
    assert_eq!(
        flag(&strict, "passWithinTolerance"),
        Some(false),
        "the same shift is over a 0.05 threshold: {strict}"
    );
    assert_eq!(
        num(&strict, "diffPixels"),
        200 * 100,
        "every pixel of the 200x100 block counts: {strict}"
    );

    std::fs::remove_dir_all(&dir).ok();
    shutdown(c);
}

/// SPEC.md S3.2 case 2: `maxDiffPixels` is the boundary it says it is.
///
/// A 10x10 black square appears on white: N pixels change. The same diff
/// passes at `maxDiffPixels:N` and fails at `N-1`.
#[test]
fn max_diff_pixels_boundary_flips_the_verdict() {
    let (mut c, dir, _page) = open(
        "maxpx",
        "<!doctype html><html><head><style>html,body{margin:0;padding:0;background:#fff}\
         </style></head><body></body></html>",
        200,
        100,
    );

    c.call_ok("visual", json!({"action": "save", "name": "px"}));
    let painted = c.call(
        "evaluate",
        json!({"expression": "const d=document.createElement('div'); d.id='sq'; d.style='position:absolute;left:20px;top:20px;width:10px;height:10px;background:#000'; document.body.appendChild(d); 'painted'"}),
    );
    assert!(painted.get("isError").is_none(), "evaluate: {painted}");

    // Read N off the engine rather than assuming it: the boundary is the
    // claim, and the exact pixel count is the engine's business.
    let probe = c.call_ok(
        "visual",
        json!({"action": "compare", "name": "px", "maxDiffPixels": 100000, "maxDiffRatio": 1.0}),
    );
    let n = num(&probe, "diffPixels");
    assert!(n > 0, "the square is a real diff of some size: {probe}");

    let at = c.call_ok(
        "visual",
        json!({"action": "compare", "name": "px", "maxDiffPixels": n, "maxDiffRatio": 1.0}),
    );
    assert_eq!(
        flag(&at, "passWithinTolerance"),
        Some(true),
        "a diff of {n} pixels passes at maxDiffPixels:{n}: {at}"
    );

    let under = c.call_ok(
        "visual",
        json!({"action": "compare", "name": "px", "maxDiffPixels": n - 1, "maxDiffRatio": 1.0}),
    );
    assert_eq!(
        flag(&under, "passWithinTolerance"),
        Some(false),
        "and fails at maxDiffPixels:{}: {under}",
        n - 1
    );
    assert_eq!(
        num(&under, "diffPixels"),
        n,
        "the same diff is measured either way: {under}"
    );

    std::fs::remove_dir_all(&dir).ok();
    shutdown(c);
}

/// SPEC.md S3.2 case 3: `maxDiffRatio` is the boundary it says it is.
///
/// The same 10x10 change in a 200x100 viewport is a ratio of about 0.005.
/// The compare passes above that ratio and fails below it, with
/// `maxDiffPixels` set wide so the pixel cap cannot be what decided.
#[test]
fn max_diff_ratio_boundary_flips_the_verdict() {
    let (mut c, dir, _page) = open(
        "maxratio",
        "<!doctype html><html><head><style>html,body{margin:0;padding:0;background:#fff}\
         </style></head><body></body></html>",
        200,
        100,
    );

    c.call_ok("visual", json!({"action": "save", "name": "ra"}));
    let painted = c.call(
        "evaluate",
        json!({"expression": "const d=document.createElement('div'); d.id='sq'; d.style='position:absolute;left:20px;top:20px;width:10px;height:10px;background:#000'; document.body.appendChild(d); 'painted'"}),
    );
    assert!(painted.get("isError").is_none(), "evaluate: {painted}");

    let probe = c.call_ok(
        "visual",
        json!({"action": "compare", "name": "ra", "maxDiffPixels": 100000, "maxDiffRatio": 1.0}),
    );
    let r = ratio(&probe, "ratio");
    assert!(r > 0.0, "the change is a real ratio of the frame: {probe}");

    let above = c.call_ok(
        "visual",
        json!({"action": "compare", "name": "ra", "maxDiffPixels": 100000,
               "maxDiffRatio": r * 2.0}),
    );
    assert_eq!(
        flag(&above, "passWithinTolerance"),
        Some(true),
        "a ratio of {r} passes at maxDiffRatio:{}: {above}",
        r * 2.0
    );

    let below = c.call_ok(
        "visual",
        json!({"action": "compare", "name": "ra", "maxDiffPixels": 100000,
               "maxDiffRatio": r * 0.5}),
    );
    assert_eq!(
        flag(&below, "passWithinTolerance"),
        Some(false),
        "and fails at maxDiffRatio:{}: {below}",
        r * 0.5
    );
    assert_eq!(
        num(&below, "diffPixels"),
        num(&above, "diffPixels"),
        "the same diff is measured either way: {below}"
    );

    std::fs::remove_dir_all(&dir).ok();
    shutdown(c);
}

/// SPEC.md S3.2 case 4 / S1.3: a mask passes the compare AND reports itself.
///
/// A 200x100 black block (16.7 % of a 400x300 frame) appears inside a mask
/// rect. The compare passes because the change is masked - and the reply says
/// how much of the frame the mask covered, so a regression hidden inside a
/// mask is metered rather than silent. The unmasked control proves the mask,
/// not the tolerance, is what produced the pass.
#[test]
fn mask_rects_hide_a_change_but_report_it() {
    let (mut c, dir, _page) = open(
        "mask",
        "<!doctype html><html><head><style>html,body{margin:0;padding:0;background:#fff}\
         </style></head><body></body></html>",
        400,
        300,
    );

    c.call_ok("visual", json!({"action": "save", "name": "mk"}));
    let painted = c.call(
        "evaluate",
        json!({"expression": "const d=document.createElement('div'); d.id='big'; d.style='position:absolute;left:50px;top:50px;width:200px;height:100px;background:#000'; document.body.appendChild(d); 'painted'"}),
    );
    assert!(painted.get("isError").is_none(), "evaluate: {painted}");

    // Control: unmasked and at the default pass bar, this is a large, failing
    // change.
    let open_diff = c.call_ok("visual", json!({"action": "compare", "name": "mk"}));
    assert_eq!(
        flag(&open_diff, "passWithinTolerance"),
        Some(false),
        "without a mask the change fails at the default pass bar: {open_diff}"
    );
    assert_eq!(
        num(&open_diff, "diffPixels"),
        200 * 100,
        "the unmasked change is counted in full: {open_diff}"
    );

    let masked = c.call_ok(
        "visual",
        json!({"action": "compare", "name": "mk",
               "maskRects": [{"x": 50, "y": 50, "w": 200, "h": 100}]}),
    );
    assert_eq!(
        flag(&masked, "passWithinTolerance"),
        Some(true),
        "a change entirely inside the mask does not fail the compare: {masked}"
    );
    assert_eq!(
        flag(&masked, "pixelsIdentical"),
        Some(false),
        "and the pixels are still reported as different: {masked}"
    );
    assert_eq!(
        num(&masked, "maskedPixels"),
        200 * 100,
        "the mask's covered area is counted, not inferred: {masked}"
    );
    let coverage = ratio(&masked, "maskCoverageRatio");
    assert!(
        (coverage - (200.0 * 100.0) / (400.0 * 300.0)).abs() < 0.001,
        "maskCoverageRatio is the applied mask area over the image area: {coverage} in {masked}"
    );
    assert_eq!(
        masked
            .get("maskRectsApplied")
            .and_then(|v| v.as_array())
            .map(|a| a.len()),
        Some(1),
        "the rect that was actually applied is echoed back: {masked}"
    );
    let warning = masked.get("warning").and_then(|w| w.as_str()).unwrap_or("");
    assert!(
        warning.contains("mask_coverage"),
        "a mask over 10 % of the frame is announced, not silent: {masked}"
    );

    std::fs::remove_dir_all(&dir).ok();
    shutdown(c);
}
