use crate::browser::Browser;

/// Internal end-to-end check: drives a temp fixture page through the real
/// browser path (navigate -> snapshot -> evaluate -> click -> close).
/// Prints PASS/FAIL lines; returns the process exit code.
pub async fn run() -> i32 {
    let mut failures = 0;
    let mut check = |name: &str, ok: bool, detail: &str| {
        if ok {
            println!("SELFTEST {name}: PASS {detail}");
        } else {
            println!("SELFTEST {name}: FAIL {detail}");
            failures += 1;
        }
    };

    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            println!("SELFTEST setup: FAIL tempdir: {e}");
            return 1;
        }
    };
    let page = dir.path().join("selftest.html");
    if let Err(e) = std::fs::write(&page, FIXTURE_HTML) {
        println!("SELFTEST setup: FAIL write fixture: {e}");
        return 1;
    }
    let url = format!("file:///{}", page.to_string_lossy().replace('\\', "/"));

    let b = Browser::new();

    // navigate
    let v = crate::tools::call("navigate", &serde_json::json!({"url": url}), &b).await;
    let nav_text = v
        .get("content")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    check("navigate", nav_text.contains("Gladius Fixture"), &nav_text);

    // snapshot contains marker
    let snap = content_text(&crate::tools::call("snapshot", &serde_json::json!({}), &b).await);
    check(
        "snapshot",
        snap.contains("GLADIUS_MARKER_hello-12345"),
        &truncate(&snap, 120),
    );

    // evaluate returns known value
    let ev = content_text(
        &crate::tools::call(
            "evaluate",
            &serde_json::json!({"expression": "window.__gladiusValue"}),
            &b,
        )
        .await,
    );
    check("evaluate", ev.contains("42"), &ev);

    // click toggles the element
    let clk = crate::tools::call("click", &serde_json::json!({"element": "#toggle"}), &b).await;
    let clk_ok = clk.get("isError").is_none();
    check("click", clk_ok, &clk.to_string());
    let toggled = content_text(
        &crate::tools::call(
            "evaluate",
            &serde_json::json!({"expression": "document.getElementById('toggle').textContent"}),
            &b,
        )
        .await,
    );
    check("click-toggle", toggled.contains("TOGGLED_OK"), &toggled);

    // console: log probe is buffered with its shape fields
    let le = content_text(
        &crate::tools::call(
            "evaluate",
            &serde_json::json!({"expression": "console.log('GLADIUS_CONSOLE_PROBE_7f3a'); 'logged'"}),
            &b,
        )
        .await,
    );
    check("console-log-eval", le.contains("logged"), &le);
    let cg = content_text(&crate::tools::call("console", &serde_json::json!({}), &b).await);
    check(
        "console",
        cg.contains("GLADIUS_CONSOLE_PROBE_7f3a") && cg.contains("\"type\""),
        &truncate(&cg, 200),
    );

    // console: uncaught exceptions are buffered (async throw, poll bounded)
    let _ = crate::tools::call(
        "evaluate",
        &serde_json::json!({"expression": "setTimeout(function(){ throw new Error('GLADIUS_EXC_PROBE_9c1e'); }, 0); 'armed'"}),
        &b,
    )
    .await;
    let mut exc_seen = false;
    let mut exc_text = String::new();
    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        exc_text = content_text(&crate::tools::call("console", &serde_json::json!({}), &b).await);
        if exc_text.contains("GLADIUS_EXC_PROBE_9c1e") {
            exc_seen = true;
            break;
        }
    }
    check("console-exc", exc_seen, &truncate(&exc_text, 200));

    // console: clear drains, then the ring reads empty
    let clr = content_text(
        &crate::tools::call("console", &serde_json::json!({"action": "clear"}), &b).await,
    );
    check("console-clear", clr.contains("cleared"), &clr);
    let empty = content_text(&crate::tools::call("console", &serde_json::json!({}), &b).await);
    check("console-empty", empty.contains("\"total\":0"), &empty);

    // console: drain returns messages AND clears
    let _ = crate::tools::call(
        "evaluate",
        &serde_json::json!({"expression": "console.warn('GLADIUS_DRAIN_PROBE_4b2d'); 'warned'"}),
        &b,
    )
    .await;
    let dr = content_text(
        &crate::tools::call("console", &serde_json::json!({"action": "drain"}), &b).await,
    );
    check(
        "console-drain",
        dr.contains("GLADIUS_DRAIN_PROBE_4b2d") && dr.contains("cleared"),
        &truncate(&dr, 200),
    );
    let empty2 = content_text(&crate::tools::call("console", &serde_json::json!({}), &b).await);
    check("console-drained", empty2.contains("\"total\":0"), &empty2);

    // upload: real temp files onto the file input by selector
    let up1 = dir.path().join("upload.txt");
    let up2 = dir.path().join("upload2.txt");
    if let Err(e) = std::fs::write(&up1, "gladius-upload-probe-1") {
        println!("SELFTEST setup: FAIL write upload file: {e}");
        return 1;
    }
    if let Err(e) = std::fs::write(&up2, "gladius-upload-probe-2") {
        println!("SELFTEST setup: FAIL write upload file: {e}");
        return 1;
    }
    let up1s = up1.to_string_lossy().replace('\\', "/");
    let up2s = up2.to_string_lossy().replace('\\', "/");
    let up = crate::tools::call(
        "upload",
        &serde_json::json!({"element": "#file", "paths": [up1s]}),
        &b,
    )
    .await;
    check("upload", up.get("isError").is_none(), &up.to_string());
    let flen = content_text(
        &crate::tools::call(
            "evaluate",
            &serde_json::json!({"expression": "document.getElementById('file').files.length"}),
            &b,
        )
        .await,
    );
    check("upload-files", flen.contains('1'), &flen);
    let fname = content_text(
        &crate::tools::call(
            "evaluate",
            &serde_json::json!({"expression": "document.getElementById('file').files[0].name"}),
            &b,
        )
        .await,
    );
    check("upload-name", fname.contains("upload.txt"), &fname);

    // upload: the same input by AX ref, two files at once. (The fixture has
    // exactly one <input>, so match the tag alone: after the first upload
    // the input's name/value carry the fakepath, not the id.)
    let snap2 = content_text(&crate::tools::call("snapshot", &serde_json::json!({}), &b).await);
    let uref = snap2
        .lines()
        .find(|l| l.contains("(input)"))
        .and_then(|l| {
            let i = l.find("[@")? + 2;
            let j = l[i..].find(']')?;
            Some(l[i..i + j].to_string())
        })
        .unwrap_or_default();
    check("upload-ref-found", !uref.is_empty(), &uref);
    if !uref.is_empty() {
        let upr = crate::tools::call(
            "upload",
            &serde_json::json!({"element": format!("@{uref}"), "paths": [up1s.clone(), up2s.clone()]}),
            &b,
        )
        .await;
        check("upload-ref", upr.get("isError").is_none(), &upr.to_string());
        let flen2 = content_text(
            &crate::tools::call(
                "evaluate",
                &serde_json::json!({"expression": "document.getElementById('file').files.length"}),
                &b,
            )
            .await,
        );
        check("upload-ref-files", flen2.contains('2'), &flen2);
    }

    // upload: typed failures (non-file element, missing element, missing file)
    let f1 = crate::tools::call(
        "upload",
        &serde_json::json!({"element": "#toggle", "paths": [up1s]}),
        &b,
    )
    .await;
    let f1t = f1.to_string();
    check(
        "upload-notfile",
        f1.get("isError").is_some() && f1t.contains("not a file input"),
        &truncate(&f1t, 160),
    );
    let f2 = crate::tools::call(
        "upload",
        &serde_json::json!({"element": "#nope-missing", "paths": [up1s]}),
        &b,
    )
    .await;
    let f2t = f2.to_string();
    check(
        "upload-missing",
        f2.get("isError").is_some() && f2t.contains("not found"),
        &truncate(&f2t, 160),
    );
    let f3 = crate::tools::call(
        "upload",
        &serde_json::json!({"element": "#file", "paths": ["C:/no-such-gladius-file-xyz.txt"]}),
        &b,
    )
    .await;
    let f3t = f3.to_string();
    check(
        "upload-nofile",
        f3.get("isError").is_some() && f3t.contains("file not found"),
        &truncate(&f3t, 160),
    );

    // network-idle: the fixture's own on-load fetch settles locally first
    let loadflag = content_text(
        &crate::tools::call(
            "evaluate",
            &serde_json::json!({"expression": "window.__gladiusNetLoad"}),
            &b,
        )
        .await,
    );
    check(
        "netload-settled",
        loadflag.contains("resolved") || loadflag.contains("rejected"),
        &loadflag,
    );

    // network-idle: loopback traffic source (local only, no external net).
    // /fast answers at once (drain path); /hang holds the response open
    // (never-idle path). Plain std-style HTTP over tokio (already a dep).
    let (port, srv) = loopback_server().await;
    let t0 = std::time::Instant::now();
    let fired = content_text(
        &crate::tools::call(
            "evaluate",
            &serde_json::json!({"expression": format!("window.__gladiusNet='no'; fetch('http://127.0.0.1:{port}/fast',{{cache:'no-store'}}).then(()=>{{window.__gladiusNet='done';}}).catch(()=>{{window.__gladiusNet='done';}}); 'fired'")}),
            &b,
        )
        .await,
    );
    check("netidle-fired", fired.contains("fired"), &fired);
    let w = crate::tools::call(
        "wait_for",
        &serde_json::json!({"networkIdle": true, "idleMs": 300, "ms": 8000}),
        &b,
    )
    .await;
    let dt = t0.elapsed();
    check(
        "network-idle",
        w.get("isError").is_none() && dt >= std::time::Duration::from_millis(250),
        &format!("{} elapsed={dt:?}", truncate(&w.to_string(), 120)),
    );
    let nflag = content_text(
        &crate::tools::call(
            "evaluate",
            &serde_json::json!({"expression": "window.__gladiusNet"}),
            &b,
        )
        .await,
    );
    check("network-idle-fetch", nflag.contains("done"), &nflag);

    // network-idle timeout: a held response keeps the page busy forever;
    // the bounded budget must return the typed timeout, never hang.
    let _ = crate::tools::call(
        "evaluate",
        &serde_json::json!({"expression": format!("fetch('http://127.0.0.1:{port}/hang',{{cache:'no-store'}}).catch(()=>{{}}); 'held'")}),
        &b,
    )
    .await;
    let t1 = std::time::Instant::now();
    let w2 = crate::tools::call(
        "wait_for",
        &serde_json::json!({"networkIdle": true, "idleMs": 300, "ms": 800}),
        &b,
    )
    .await;
    let dt2 = t1.elapsed();
    let w2t = w2.to_string();
    check(
        "network-idle-timeout",
        w2.get("isError").is_some()
            && w2t.contains("timeout")
            && dt2 >= std::time::Duration::from_millis(600)
            && dt2 < std::time::Duration::from_secs(8),
        &format!("{} elapsed={dt2:?}", truncate(&w2t, 120)),
    );
    srv.abort();

    // close
    let closed = crate::tools::call("close", &serde_json::json!({}), &b).await;
    check(
        "close",
        closed.get("isError").is_none(),
        &closed.to_string(),
    );

    if failures == 0 {
        println!("SELFTEST: ALL PASS");
        0
    } else {
        println!("SELFTEST: {failures} FAILURES");
        1
    }
}

fn content_text(v: &serde_json::Value) -> String {
    v.get("content")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string()
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        s.chars().take(n).collect()
    } else {
        s.to_string()
    }
}

/// Loopback HTTP traffic source for the network-idle selftest (local only).
/// `/fast` answers at once; `/hang` holds the response open past every
/// test budget so the wait must take its timeout path. Returns the port
/// and the accept-loop handle (aborted by the caller when done).
async fn loopback_server() -> (u16, tokio::task::JoinHandle<()>) {
    let ln = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback bind");
    let port = ln.local_addr().expect("loopback addr").port();
    let h = tokio::spawn(async move {
        loop {
            let (mut s, _) = match ln.accept().await {
                Ok(x) => x,
                Err(_) => break,
            };
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = vec![0u8; 4096];
                let mut req = Vec::new();
                loop {
                    match s.read(&mut buf).await {
                        Ok(0) => return,
                        Ok(n) => {
                            req.extend_from_slice(&buf[..n]);
                            if req.windows(4).any(|w| w == b"\r\n\r\n") || req.len() > 8192 {
                                break;
                            }
                        }
                        Err(_) => return,
                    }
                }
                let hang = String::from_utf8_lossy(&req).contains("/hang");
                if hang {
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                }
                let body = "ok";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: text/plain\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });
    (port, h)
}

pub const FIXTURE_HTML: &str = r#"<!doctype html>
<html>
<head><title>Gladius Fixture</title></head>
<body>
<h1 id="marker">GLADIUS_MARKER_hello-12345</h1>
<button id="toggle" onclick="this.textContent='TOGGLED_OK'">CLICK_ME</button>
<input type="file" id="file" multiple>
<script>
window.__gladiusValue = 42;
window.__gladiusNetLoad = 'pending';
fetch(window.location.href, {cache: 'no-store'})
  .then(function() { window.__gladiusNetLoad = 'resolved'; })
  .catch(function() { window.__gladiusNetLoad = 'rejected'; });
</script>
</body>
</html>
"#;
