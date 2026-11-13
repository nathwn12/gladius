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

/// The `text` part of a screenshot reply, parsed: the deterministic metadata.
fn shot_meta(reply: &Value) -> Value {
    let parts = reply
        .get("content")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();
    let text = parts
        .iter()
        .find(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
        .and_then(|p| p.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    serde_json::from_str(text).unwrap_or_else(|_| panic!("screenshot meta is not json: {text}"))
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

// --------------------------------------------------------------------- S1.3

/// SPEC.md S1.3: a mask is metered, not an invisible hole.
///
/// One rect covering 15 % of an 800x600 capture must be counted (union, not
/// sum), echoed back clipped and rounded, and announced as a warning that a
/// regression inside it would be invisible. A rect that is degenerate after
/// clipping is dropped and named instead of silently masking nothing.
#[test]
fn mask_coverage_is_reported_and_warns() {
    let dir = scratch("mask");
    let vdir = dir.join("visual");
    std::fs::create_dir_all(&vdir).expect("visual dir");
    let vdir_s = vdir.to_string_lossy().to_string();
    let mut c = Client::spawn_with_env(&[("GLADIUS_VISUAL_DIR", &vdir_s)]);
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));

    let page = dir.join("mask.html");
    std::fs::write(
        &page,
        "<!doctype html><html><head><style>html,body{margin:0;padding:0;background:#fff}\
         #box{position:absolute;left:0;top:0;width:800px;height:90px;background:#eee}\
         </style></head><body><div id=\"box\"></div></body></html>",
    )
    .expect("write mask fixture");
    goto(&mut c, &page, 800, 600);
    c.call_ok("visual", json!({"action": "save", "name": "masked"}));

    // 800 x 90 = 72,000 px = 15 % of 800 x 600.
    let rect = json!({"x": 0, "y": 0, "w": 800, "h": 90});
    let cmp = c.call_ok(
        "visual",
        json!({"action": "compare", "name": "masked", "maskRects": [rect]}),
    );
    let masked = num(&cmp, "maskedPixels").unwrap_or(-1);
    assert!(
        (masked - 72000).abs() <= 1,
        "the masked area is counted, not assumed: {masked} (reply {cmp})"
    );
    let coverage = cmp
        .get("maskCoverageRatio")
        .and_then(|v| v.as_f64())
        .unwrap_or(-1.0);
    assert!(
        (coverage - 0.15).abs() <= 0.005,
        "maskCoverageRatio is maskedPixels/totalPixels: {coverage} (reply {cmp})"
    );
    let applied = cmp
        .get("maskRectsApplied")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        applied.len(),
        1,
        "the applied rect list echoes what was actually used: {cmp}"
    );
    assert_eq!(
        applied[0], rect,
        "an in-bounds integer rect is applied unchanged: {cmp}"
    );
    let warning = cmp.get("warning").and_then(|w| w.as_str()).unwrap_or("");
    assert!(
        warning.contains("mask_coverage"),
        "a 15 % mask is over the 10 % line and must say so: {cmp}"
    );

    // A rect that clips away to nothing is dropped and named, never silently
    // treated as a mask that covered something.
    let degenerate = c.call_ok(
        "visual",
        json!({"action": "compare", "name": "masked",
               "maskRects": [{"x": 900, "y": 0, "w": 50, "h": 50}]}),
    );
    assert_eq!(
        degenerate
            .get("maskRectsApplied")
            .and_then(|v| v.as_array())
            .map(|a| a.len()),
        Some(0),
        "an out-of-bounds rect clips to nothing and is not applied: {degenerate}"
    );
    let dwarning = degenerate
        .get("warning")
        .and_then(|w| w.as_str())
        .unwrap_or("");
    assert!(
        dwarning.contains("mask_rect_dropped"),
        "a dropped rect is named in the warning: {degenerate}"
    );

    std::fs::remove_dir_all(&dir).ok();
    shutdown(c);
}

// --------------------------------------------------------------------- S1.4

/// SPEC.md S1.4 / D3: a settle timeout is a field, not a stderr line.
///
/// A loopback server accepts the connection and never answers, so the `<img>`
/// it serves can never finish. The deterministic capture must still return its
/// PNG (the pixels are real) while saying plainly that it gave up: `settled`
/// false and `images` named in `settleTimeouts`.
#[test]
fn settle_timeout_is_a_field() {
    let dir = scratch("settle");
    let vdir = dir.join("visual");
    std::fs::create_dir_all(&vdir).expect("visual dir");
    let vdir_s = vdir.to_string_lossy().to_string();

    // Accept and hold: never write a byte, so the image request hangs.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for s in listener.incoming().flatten() {
            held.push(s);
        }
    });

    let page = dir.join("slowimg.html");
    std::fs::write(
        &page,
        format!(
            "<!doctype html><html><body style=\"margin:0;background:#fff\">\
             <img src=\"http://127.0.0.1:{port}/never.png\" width=\"40\" height=\"40\">\
             <p>SETTLE_MARKER</p></body></html>"
        ),
    )
    .expect("write settle fixture");

    let mut c = Client::spawn_with_env(&[("GLADIUS_VISUAL_DIR", &vdir_s)]);
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    goto(&mut c, &page, 400, 300);

    let shot = c.call("screenshot", json!({"deterministic": true}));
    assert!(shot.get("isError").is_none(), "screenshot: {shot}");
    let parts = shot
        .get("content")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();
    let image = parts
        .iter()
        .find(|p| p.get("type").and_then(|t| t.as_str()) == Some("image"))
        .cloned()
        .unwrap_or_else(|| panic!("the capture still returns its image part: {shot}"));
    assert!(
        image
            .get("data")
            .and_then(|d| d.as_str())
            .map(|d| !d.is_empty())
            .unwrap_or(false),
        "a lost race does not throw the pixels away: {shot}"
    );
    let meta_text = parts
        .iter()
        .find(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
        .and_then(|p| p.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    let meta: Value = serde_json::from_str(meta_text).expect("screenshot meta json");
    assert_eq!(
        flag(&meta, "settled"),
        Some(false),
        "the reply says the capture did not settle: {meta}"
    );
    let timeouts: Vec<String> = meta
        .get("settleTimeouts")
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|t| t.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        timeouts.iter().any(|t| t == "images"),
        "the lost race names itself: {meta}"
    );

    // The artifact carries its own honesty: the sidecar records the settle
    // state, and a compare against an unsettled baseline says so - as a typed
    // failure under a strict environment gate.
    let saved = c.call_ok("visual", json!({"action": "save", "name": "unsettled"}));
    assert_eq!(
        flag(&saved, "settled"),
        Some(false),
        "save reports the settle state it recorded: {saved}"
    );
    let sidecar: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("visual").join("unsettled.meta.json")).expect("sidecar"),
    )
    .expect("sidecar json");
    assert_eq!(
        flag(&sidecar, "settled"),
        Some(false),
        "the sidecar itself says the baseline did not settle: {sidecar}"
    );
    let cmp = c.call_ok("visual", json!({"action": "compare", "name": "unsettled"}));
    let warning = cmp.get("warning").and_then(|w| w.as_str()).unwrap_or("");
    assert!(
        warning.contains("baseline_unsettled"),
        "a compare names an unsettled baseline: {cmp}"
    );
    let strict = c.call(
        "visual",
        json!({"action": "compare", "name": "unsettled", "requireEnvironmentMatch": true}),
    );
    assert!(
        strict.get("isError").is_some(),
        "a strict environment gate must not pass on an unsettled baseline: {strict}"
    );
    let sc = strict
        .get("structuredContent")
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(
        sc.get("code").and_then(|c| c.as_str()),
        Some("baseline_environment_unsettled"),
        "and the refusal is typed: {strict}"
    );

    std::fs::remove_dir_all(&dir).ok();
    shutdown(c);
}

// --------------------------------------------------------------------- S1.7

/// SPEC.md S1.7: font readiness is a status, not a race.
///
/// `document.fonts.ready` resolves when the CURRENT loads finish, so a
/// `font-display:swap` page can capture mid-swap and a timed-out wait used to
/// leave only a stderr line. The fixture's `@font-face` is served by a
/// loopback that stalls past the 3 s budget: the capture must say
/// `fontsPending >= 1`, name `fonts` in `settleTimeouts`, and be unsettled.
/// A page whose font resolves at once is the clean path.
#[test]
fn fonts_report_pending_not_silent() {
    let dir = scratch("fonts");
    let vdir = dir.join("visual");
    std::fs::create_dir_all(&vdir).expect("visual dir");
    let vdir_s = vdir.to_string_lossy().to_string();

    // Accept the request and stall well past the settle budget: the FontFace
    // stays `loading`, which is exactly what must be reported.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for s in listener.incoming().flatten() {
            held.push(s);
            std::thread::sleep(Duration::from_secs(30));
        }
    });

    let slow = dir.join("slowfont.html");
    std::fs::write(
        &slow,
        format!(
            "<!doctype html><html><head><style>\
             @font-face {{ font-family: SlowFace; src: url('http://127.0.0.1:{port}/slow.woff2'); }}\
             body {{ margin: 0; background: #fff; font: 16px/20px SlowFace, sans-serif }}\
             </style></head><body><p>FONT_MARKER</p></body></html>"
        ),
    )
    .expect("write slow font fixture");

    let mut c = Client::spawn_with_env(&[("GLADIUS_VISUAL_DIR", &vdir_s)]);
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));
    goto(&mut c, &slow, 400, 200);

    let shot = c.call("screenshot", json!({"deterministic": true}));
    assert!(shot.get("isError").is_none(), "screenshot: {shot}");
    let meta = shot_meta(&shot);
    let pending = meta
        .get("fontsPending")
        .and_then(|v| v.as_i64())
        .unwrap_or(-1);
    assert!(
        pending >= 1,
        "a face still loading at capture time is counted: {meta}"
    );
    let timeouts: Vec<String> = meta
        .get("settleTimeouts")
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|t| t.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        timeouts.iter().any(|t| t == "fonts"),
        "the font wait names itself in settleTimeouts: {meta}"
    );
    assert_eq!(
        flag(&meta, "settled"),
        Some(false),
        "and the capture is honest about it: {meta}"
    );

    // Clean path: a font that resolves at once leaves nothing pending and two
    // captures of the same page are byte-identical.
    let fast = dir.join("fastfont.html");
    std::fs::write(
        &fast,
        "<!doctype html><html><head><style>\
         @font-face { font-family: FastFace; src: local('Arial'); }\
         body { margin: 0; background: #fff; font: 16px/20px FastFace, sans-serif }\
         </style></head><body><p>FONT_MARKER</p></body></html>",
    )
    .expect("write fast font fixture");
    goto(&mut c, &fast, 400, 200);
    let first = c.call("screenshot", json!({"deterministic": true}));
    assert!(first.get("isError").is_none(), "fast capture: {first}");
    let m1 = shot_meta(&first);
    assert_eq!(
        flag(&m1, "settled"),
        Some(true),
        "a page whose font resolves at once settles: {m1}"
    );
    assert_eq!(
        m1.get("fontsPending").and_then(|v| v.as_i64()),
        Some(0),
        "nothing is pending: {m1}"
    );
    std::thread::sleep(Duration::from_millis(120));
    let second = c.call("screenshot", json!({"deterministic": true}));
    assert!(second.get("isError").is_none(), "fast capture 2: {second}");
    let m2 = shot_meta(&second);
    assert_eq!(
        m1.get("checksum").and_then(|v| v.as_str()),
        m2.get("checksum").and_then(|v| v.as_str()),
        "two captures of the settled page are identical: {m1} vs {m2}"
    );

    std::fs::remove_dir_all(&dir).ok();
    shutdown(c);
}

// --------------------------------------------------------------------- S1.8

/// SPEC.md S1.8: the environment fingerprint covers the axes that differ.
///
/// A baseline saved on another machine used to match, because only browser
/// major, viewport and DSF were compared. Every axis is now read live and
/// participates: editing the sidecar's `platform` alone must fail the match and
/// name `platform` in the warning, and an untouched compare must report every
/// axis with a real value.
#[test]
fn environment_fingerprint_names_every_axis() {
    let dir = scratch("env");
    let vdir = dir.join("visual");
    std::fs::create_dir_all(&vdir).expect("visual dir");
    let vdir_s = vdir.to_string_lossy().to_string();
    let mut c = Client::spawn_with_env(&[("GLADIUS_VISUAL_DIR", &vdir_s)]);
    c.result("initialize", json!({"protocolVersion": "2025-06-18"}));

    let page = dir.join("env.html");
    std::fs::write(
        &page,
        "<!doctype html><html><head><style>html,body{margin:0;background:#fff}\
         p{font:16px/20px sans-serif}</style></head><body><p>ENV_MARKER</p></body></html>",
    )
    .expect("write env fixture");
    goto(&mut c, &page, 400, 200);
    c.call_ok("visual", json!({"action": "save", "name": "env"}));

    // The untouched compare: every axis is present and real.
    let clean = c.call_ok("visual", json!({"action": "compare", "name": "env"}));
    assert_eq!(
        clean
            .pointer("/environment/match")
            .and_then(|v| v.as_bool()),
        Some(true),
        "an untouched baseline matches: {clean}"
    );
    let current = clean
        .pointer("/environment/current")
        .cloned()
        .unwrap_or(Value::Null);
    for axis in [
        "browserFullVersion",
        "platform",
        "userAgent",
        "colorScheme",
        "forcedColors",
        "contrast",
        "reducedMotion",
        "fontCountHash",
        "scrollbarWidth",
    ] {
        assert!(
            current.get(axis).map(|v| !v.is_null()).unwrap_or(false),
            "environment.current carries a real {axis}: {current}"
        );
    }

    // Hand-edit the sidecar's platform: that alone must break the match.
    let meta_file = vdir.join("env.meta.json");
    let text = std::fs::read_to_string(&meta_file).expect("sidecar");
    let mut meta: Value = serde_json::from_str(&text).expect("sidecar json");
    meta["platform"] = json!("SomeOtherPlatform");
    std::fs::write(
        &meta_file,
        serde_json::to_string_pretty(&meta).expect("encode sidecar"),
    )
    .expect("rewrite sidecar");

    let edited = c.call_ok("visual", json!({"action": "compare", "name": "env"}));
    assert_eq!(
        edited
            .pointer("/environment/match")
            .and_then(|v| v.as_bool()),
        Some(false),
        "a different platform is a different environment: {edited}"
    );
    let warning = edited.get("warning").and_then(|w| w.as_str()).unwrap_or("");
    assert!(
        warning.contains("platform"),
        "the mismatch names the axis that differs: {edited}"
    );

    std::fs::remove_dir_all(&dir).ok();
    shutdown(c);
}
