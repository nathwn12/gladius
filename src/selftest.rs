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

    // Hover is opt-in (owner constraint "hover explicit only"): a default
    // click dispatches NO mouseMoved, so the page sees zero mousemove events;
    // hoverFirst:true sends exactly one positioning event at the click point.
    let _ = crate::tools::call(
        "evaluate",
        &serde_json::json!({"expression": "window.__gladiusMM = 0; window.__gladiusDown = 0; \
            document.addEventListener('mousemove', function () { window.__gladiusMM++; }, true); \
            document.addEventListener('mousedown', function () { window.__gladiusDown++; }, true); 'armed'"}),
        &b,
    )
    .await;
    let _ = crate::tools::call("click", &serde_json::json!({"element": "#toggle"}), &b).await;
    let after_default = crate::tools::call(
        "evaluate",
        &serde_json::json!({"expression": "[window.__gladiusMM, window.__gladiusDown]"}),
        &b,
    )
    .await;
    let d = sc_of(&after_default)
        .get("value")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let dn = |i: usize| d.get(i).and_then(|v| v.as_i64()).unwrap_or(-1);
    check(
        "click-no-hover-by-default",
        dn(0) == 0 && dn(1) == 1,
        &format!("mousemove={} mousedown={}", dn(0), dn(1)),
    );
    let _ = crate::tools::call(
        "click",
        &serde_json::json!({"element": "#toggle", "hoverFirst": true}),
        &b,
    )
    .await;
    let after_hover = crate::tools::call(
        "evaluate",
        &serde_json::json!({"expression": "[window.__gladiusMM, window.__gladiusDown]"}),
        &b,
    )
    .await;
    let h = sc_of(&after_hover)
        .get("value")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let hn = |i: usize| h.get(i).and_then(|v| v.as_i64()).unwrap_or(-1);
    check(
        "click-hover-first-sends-one-move",
        hn(0) == 1 && hn(1) == 2,
        &format!("mousemove={} mousedown={}", hn(0), hn(1)),
    );

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

    // ---------------------------------------------------------------- layout
    // Geometry without pixels, deterministic pixels, and baselines. The
    // fixture is a local file (tests/fixtures/layout.html): no network.
    let fixture = std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/layout.html"
    ));
    if !fixture.is_file() {
        println!(
            "SELFTEST setup: FAIL layout fixture missing: {}",
            fixture.display()
        );
        return 1;
    }
    let lurl = format!("file:///{}", fixture.to_string_lossy().replace('\\', "/"));
    let nav2 = crate::tools::call("navigate", &serde_json::json!({"url": lurl}), &b).await;
    check(
        "layout-navigate",
        nav2.get("isError").is_none(),
        &truncate(&nav2.to_string(), 120),
    );

    // Deterministic capture twice: identical checksum AND identical byte
    // count over a page with a running CSS animation, which is exactly what
    // the freeze has to neutralise.
    let d1 = crate::tools::call(
        "screenshot",
        &serde_json::json!({"deterministic": true}),
        &b,
    )
    .await;
    let d2 = crate::tools::call(
        "screenshot",
        &serde_json::json!({"deterministic": true}),
        &b,
    )
    .await;
    let m1 = sc_of(&d1);
    let m2 = sc_of(&d2);
    let c1 = m1.get("checksum").and_then(|v| v.as_str()).unwrap_or("");
    let c2 = m2.get("checksum").and_then(|v| v.as_str()).unwrap_or("");
    check(
        "deterministic-checksum",
        c1.len() == 16 && c1 == c2,
        &format!("{c1} vs {c2}"),
    );
    let b1 = m1.get("bytes").and_then(|v| v.as_i64()).unwrap_or(0);
    let b2 = m2.get("bytes").and_then(|v| v.as_i64()).unwrap_or(0);
    check(
        "deterministic-bytes",
        b1 > 1000 && b1 == b2,
        &format!("{b1} vs {b2}"),
    );
    let vw = m1
        .pointer("/viewport/width")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let dsf = m1
        .get("deviceScaleFactor")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let durl = m1.get("url").and_then(|v| v.as_str()).unwrap_or("");
    check(
        "deterministic-meta",
        vw > 100 && dsf == 1.0 && durl.contains("layout.html"),
        &format!("viewport={vw} dsf={dsf} url={durl}"),
    );

    // Concurrent deterministic captures (blocking fix 2): three captures with
    // DIFFERENT forced geometry run at once. Serialized by the capture lock,
    // each reports exactly its own width/height; if the overrides interleaved,
    // a capture would report someone else's geometry and the restoration
    // afterwards would leave a stale override or freeze stylesheet behind.
    let pre = crate::tools::call(
        "evaluate",
        &serde_json::json!({"expression": "[window.innerWidth, window.innerHeight]"}),
        &b,
    )
    .await;
    let pv = sc_of(&pre)
        .get("value")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let pre_w = pv.get(0).and_then(|v| v.as_i64()).unwrap_or(-1);
    let pre_h = pv.get(1).and_then(|v| v.as_i64()).unwrap_or(-1);
    let cap_a = serde_json::json!({"deterministic": true, "width": 640, "height": 480});
    let cap_b = serde_json::json!({"deterministic": true, "width": 720, "height": 540});
    let cap_c = serde_json::json!({"deterministic": true, "width": 900, "height": 700});
    let (q1, q2, q3) = tokio::join!(
        crate::tools::call("screenshot", &cap_a, &b),
        crate::tools::call("screenshot", &cap_b, &b),
        crate::tools::call("screenshot", &cap_c, &b),
    );
    let mut conc_ok = true;
    let mut conc_detail = String::new();
    for (r, want) in [(&q1, (640, 480)), (&q2, (720, 540)), (&q3, (900, 700))] {
        let sc = sc_of(r);
        let w = sc.pointer("/viewport/width").and_then(|v| v.as_i64());
        let h = sc.pointer("/viewport/height").and_then(|v| v.as_i64());
        conc_ok &= r.get("isError").is_none() && w == Some(want.0) && h == Some(want.1);
        conc_detail.push_str(&format!("{w:?}x{h:?} "));
    }
    let restored = crate::tools::call(
        "evaluate",
        &serde_json::json!({"expression": "[window.innerWidth, window.innerHeight, \
            document.getElementById('__gladius_deterministic') === null, \
            matchMedia('(prefers-reduced-motion: reduce)').matches]"}),
        &b,
    )
    .await;
    let rv = sc_of(&restored)
        .get("value")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let rn = |i: usize| rv.get(i).cloned().unwrap_or(serde_json::Value::Null);
    let state_ok = rn(0).as_i64() == Some(pre_w)
        && rn(1).as_i64() == Some(pre_h)
        && rn(2).as_bool() == Some(true)
        && rn(3).as_bool() == Some(false);
    check(
        "concurrent-captures-serialized",
        conc_ok && state_ok,
        &format!("each={conc_detail} restored={rv}"),
    );

    // assert_layout: the static probe passes a box/visibility/stable-frames
    // assertion, and the measured box agrees with getBoundingClientRect (the
    // two geometry sources are cross-checked, not assumed to agree).
    let probe = serde_json::json!({
        "selector": "#probe",
        "checks": {
            "box": {"x": 40, "y": 40, "w": 140, "h": 60, "tolerance": 0.5},
            "stableFrames": 2, "hitTarget": true, "inViewport": true, "noOverflow": true,
            "computed": {"display": "block", "position": "absolute"}
        }
    });
    let a1 = crate::tools::call("assert_layout", &probe, &b).await;
    let a1sc = sc_of(&a1);
    check(
        "assert-layout-pass",
        a1.get("isError").is_none() && a1sc.get("pass").and_then(|v| v.as_bool()) == Some(true),
        &truncate(&a1.to_string(), 200),
    );
    let rect = content_text(
        &crate::tools::call(
            "evaluate",
            &serde_json::json!({"expression": "(() => { const r = document.querySelector('#probe').getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; })()"}),
            &b,
        )
        .await,
    );
    let rv: serde_json::Value = serde_json::from_str(&rect)
        .ok()
        .and_then(|v: serde_json::Value| v.get("value").cloned())
        .unwrap_or(serde_json::Value::Null);
    let rn = |i: usize| rv.get(i).and_then(|v| v.as_f64()).unwrap_or(-1.0);
    let measured = a1sc
        .pointer("/targets/0/box")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let mn = |k: &str| measured.get(k).and_then(|v| v.as_f64()).unwrap_or(-9999.0);
    check(
        "layout-box-convention",
        (mn("x") - rn(0)).abs() < 0.01
            && (mn("y") - rn(1)).abs() < 0.01
            && (mn("w") - rn(2)).abs() < 0.01
            && (mn("h") - rn(3)).abs() < 0.01,
        &format!(
            "boxmodel={measured} js=({},{},{},{})",
            rn(0),
            rn(1),
            rn(2),
            rn(3)
        ),
    );

    // The same assertion by AX ref: the ref path must resolve the identical
    // box (ref and selector are two doors to one measurement). A button is
    // used because the AX snapshot always stamps a ref on a focusable
    // control.
    let snap_l = content_text(&crate::tools::call("snapshot", &serde_json::json!({}), &b).await);
    let probe_ref = snap_l
        .lines()
        .find(|l| l.contains("ACT_MARKER") && l.contains("[@"))
        .and_then(|l| {
            let i = l.find("[@")? + 1;
            let j = l[i..].find(']')?;
            Some(l[i..i + j].to_string())
        })
        .unwrap_or_default();
    check("assert-ref-found", probe_ref.starts_with("@e"), &probe_ref);
    if !probe_ref.is_empty() {
        let ar = crate::tools::call(
            "assert_layout",
            &serde_json::json!({"ref": probe_ref,
                "checks": {"box": {"x": 300, "y": 280, "w": 80, "h": 30, "tolerance": 0.5}}}),
            &b,
        )
        .await;
        check(
            "assert-layout-by-ref",
            sc_of(&ar).get("pass").and_then(|v| v.as_bool()) == Some(true),
            &truncate(&ar.to_string(), 200),
        );
    }

    // Shift the probe by exactly 3px: the same assertion must fail with
    // delta 3, and pass again once the page is restored.
    let _ = crate::tools::call(
        "evaluate",
        &serde_json::json!({"expression": "document.getElementById('probe').style.left = '43px'; 'shifted'"}),
        &b,
    )
    .await;
    let a2 = crate::tools::call("assert_layout", &probe, &b).await;
    let a2sc = sc_of(&a2);
    let box_check = find_check(&a2sc, "box");
    let delta = box_check
        .and_then(|c| c.get("delta"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    check(
        "assert-layout-3px-fail",
        a2sc.get("pass").and_then(|v| v.as_bool()) == Some(false) && (delta - 3.0).abs() < 0.001,
        &format!("delta={delta} {}", truncate(&a2.to_string(), 160)),
    );
    let _ = crate::tools::call(
        "evaluate",
        &serde_json::json!({"expression": "document.getElementById('probe').style.left = '40px'; 'restored'"}),
        &b,
    )
    .await;
    let a3 = crate::tools::call("assert_layout", &probe, &b).await;
    check(
        "assert-layout-restored",
        sc_of(&a3).get("pass").and_then(|v| v.as_bool()) == Some(true),
        &truncate(&a3.to_string(), 160),
    );

    // Overflow is measured (400x300 content in a 120x60 box): the check must
    // FAIL and report the real overshoot, not a guess.
    let of = crate::tools::call(
        "assert_layout",
        &serde_json::json!({"selector": "#scroll", "checks": {"noOverflow": true}}),
        &b,
    )
    .await;
    let ofsc = sc_of(&of);
    let ofdelta = find_check(&ofsc, "noOverflow")
        .and_then(|c| c.get("delta"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    check(
        "assert-nooverflow-fail",
        ofsc.get("pass").and_then(|v| v.as_bool()) == Some(false)
            && (ofdelta - 280.0).abs() < 0.001,
        &format!("delta={ofdelta} {}", truncate(&of.to_string(), 160)),
    );

    // The overlay covers the centre of #under: hitTarget must fail there.
    let hit = crate::tools::call(
        "assert_layout",
        &serde_json::json!({"selector": "#under", "checks": {"hitTarget": true}}),
        &b,
    )
    .await;
    check(
        "assert-hit-occluded",
        sc_of(&hit).get("pass").and_then(|v| v.as_bool()) == Some(false),
        &truncate(&hit.to_string(), 160),
    );

    // A missing element is a failing check with an answer, never an error -
    // and screenshotOnFail produces a real, deterministic capture on disk.
    let shot_path = dir.path().join("assert-fail.png");
    let miss = crate::tools::call(
        "assert_layout",
        &serde_json::json!({"selector": "#no-such-element", "screenshotOnFail": true,
                            "screenshotPath": shot_path.to_string_lossy()}),
        &b,
    )
    .await;
    let miss_sc = sc_of(&miss);
    let shot_bytes = std::fs::metadata(&shot_path).map(|m| m.len()).unwrap_or(0);
    check(
        "assert-missing-fails",
        miss.get("isError").is_none()
            && miss_sc.get("pass").and_then(|v| v.as_bool()) == Some(false)
            && miss_sc.pointer("/checks/0/check").and_then(|v| v.as_str()) == Some("exists")
            && shot_bytes > 1000,
        &format!("file={shot_bytes} {}", truncate(&miss.to_string(), 140)),
    );

    // layout: three boxes, two of which overlap (the overlay pair), each with
    // a z-order hint and a scroll size.
    let lay = crate::tools::call(
        "layout",
        &serde_json::json!({"selector": ["#under", "#over", "#probe"]}),
        &b,
    )
    .await;
    let lsc = sc_of(&lay);
    let nel = lsc
        .get("elements")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let nov = lsc
        .get("overlaps")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let has_z = lsc.pointer("/elements/0/zOrder/zIndex").is_some();
    check(
        "layout-overlaps",
        nel == 3 && nov >= 1 && has_z,
        &format!("elements={nel} overlaps={nov} zHint={has_z}"),
    );

    // Visual baselines: save, an immediate identical compare must pass, a
    // 2px shift must fail, a full-frame mask must pass, and delete cleans up.
    // GLADIUS_VISUAL_DIR keeps the baselines out of the repo.
    let vdir = dir.path().join("visual");
    std::env::set_var("GLADIUS_VISUAL_DIR", &vdir);
    let vs = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "save", "name": "layout fixture"}),
        &b,
    )
    .await;
    let vs_sc = sc_of(&vs);
    let vs_path = vs_sc
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    check(
        "visual-save",
        vs.get("isError").is_none()
            && vs_sc.get("name").and_then(|v| v.as_str()) == Some("layout_fixture")
            && std::path::Path::new(&vs_path).is_file(),
        &truncate(&vs.to_string(), 200),
    );
    let vc1 = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "layout_fixture"}),
        &b,
    )
    .await;
    let vc1sc = sc_of(&vc1);
    check(
        "visual-identical-pass",
        vc1sc.get("pass").and_then(|v| v.as_bool()) == Some(true)
            && vc1sc.get("diffPixels").and_then(|v| v.as_i64()) == Some(0),
        &truncate(&vc1.to_string(), 200),
    );
    let _ = crate::tools::call(
        "evaluate",
        &serde_json::json!({"expression": "document.getElementById('probe').style.left = '42px'; 'shifted'"}),
        &b,
    )
    .await;
    let vc2 = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "layout_fixture"}),
        &b,
    )
    .await;
    let vc2sc = sc_of(&vc2);
    let dp = vc2sc
        .get("diffPixels")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let dp_path = vc2sc
        .get("diffPath")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    check(
        "visual-2px-fail",
        vc2sc.get("pass").and_then(|v| v.as_bool()) == Some(false)
            && dp > 0
            && !dp_path.is_empty()
            && std::path::Path::new(&dp_path).is_file(),
        &format!("diffPixels={dp} diff={dp_path}"),
    );
    let vc3 = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "layout_fixture",
                            "maskRects": [{"x": 0, "y": 0, "w": 4000, "h": 4000}]}),
        &b,
    )
    .await;
    check(
        "visual-mask-pass",
        sc_of(&vc3).get("pass").and_then(|v| v.as_bool()) == Some(true),
        &truncate(&vc3.to_string(), 160),
    );
    // update:true rewrites the baseline instead of comparing: the shifted
    // state becomes the new reference, and the old state then fails.
    let vup = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "layout_fixture", "update": true}),
        &b,
    )
    .await;
    let vup_sc = sc_of(&vup);
    let vup_ok = vup_sc.get("updated").and_then(|v| v.as_bool()) == Some(true)
        && vup_sc.get("pass").and_then(|v| v.as_bool()) == Some(true);
    let vup_same = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "layout_fixture"}),
        &b,
    )
    .await;
    let vup_same_ok = sc_of(&vup_same).get("pass").and_then(|v| v.as_bool()) == Some(true);
    let _ = crate::tools::call(
        "evaluate",
        &serde_json::json!({"expression": "document.getElementById('probe').style.left = '40px'; 'restored'"}),
        &b,
    )
    .await;
    let vup_fail = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "layout_fixture"}),
        &b,
    )
    .await;
    let vup_fail_ok = sc_of(&vup_fail).get("pass").and_then(|v| v.as_bool()) == Some(false);
    check(
        "visual-update",
        vup_ok && vup_same_ok && vup_fail_ok,
        &format!("updated={vup_ok} rebaseline-matches={vup_same_ok} old-state-fails={vup_fail_ok}"),
    );
    let vl = crate::tools::call("visual", &serde_json::json!({"action": "list"}), &b).await;
    check(
        "visual-list",
        sc_of(&vl).get("count").and_then(|v| v.as_i64()) == Some(1),
        &truncate(&vl.to_string(), 160),
    );
    let vd = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "delete", "name": "layout_fixture"}),
        &b,
    )
    .await;
    check(
        "visual-delete",
        vd.get("isError").is_none() && !std::path::Path::new(&vs_path).is_file(),
        &truncate(&vd.to_string(), 160),
    );

    // ------------------------------------------------ anti-aliasing tolerance
    // The rule (documented in src/visual.rs): a candidate pixel is ignored
    // only when its 3x3 luminance neighbourhood is high-contrast in BOTH
    // images, it is antialias-eligible, and its 4-connected component is no
    // larger than minDiffCluster (default 2).
    let canvas = std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/canvas.html"
    ));
    if !canvas.is_file() {
        println!(
            "SELFTEST setup: FAIL canvas fixture missing: {}",
            canvas.display()
        );
        return 1;
    }
    let curl = format!("file:///{}", canvas.to_string_lossy().replace('\\', "/"));
    let cnav = crate::tools::call("navigate", &serde_json::json!({"url": curl}), &b).await;
    check(
        "canvas-navigate",
        cnav.get("isError").is_none(),
        &truncate(&cnav.to_string(), 120),
    );
    let asave = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "save", "name": "aa_probe"}),
        &b,
    )
    .await;
    check(
        "aa-baseline",
        asave.get("isError").is_none(),
        &truncate(&asave.to_string(), 160),
    );

    // One anti-aliased pixel: grey on the black side of the hard edge. Its
    // neighbourhood is high-contrast in both images, so it is AA-eligible and
    // alone in its component.
    let _ = crate::tools::call(
        "evaluate",
        &serde_json::json!({"expression": "(() => { const g = document.getElementById('c').getContext('2d'); g.fillStyle = '#808080'; g.fillRect(99, 40, 1, 1); return 'aa'; })()"}),
        &b,
    )
    .await;
    let aa_on = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "aa_probe"}),
        &b,
    )
    .await;
    let aa_on_sc = sc_of(&aa_on);
    let ignored = aa_on_sc
        .get("ignoredPixels")
        .and_then(|v| v.as_i64())
        .unwrap_or(-1);
    check(
        "aa-1px-edge-passes",
        aa_on_sc.get("pass").and_then(|v| v.as_bool()) == Some(true)
            && aa_on_sc.get("diffPixels").and_then(|v| v.as_i64()) == Some(0)
            && ignored == 1,
        &format!("diff={:?} ignored={ignored}", aa_on_sc.get("diffPixels")),
    );
    let aa_off = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "aa_probe", "includeAA": true}),
        &b,
    )
    .await;
    check(
        "aa-1px-edge-fails-when-included",
        sc_of(&aa_off).get("pass").and_then(|v| v.as_bool()) == Some(false)
            && sc_of(&aa_off).get("diffPixels").and_then(|v| v.as_i64()) == Some(1),
        &truncate(&aa_off.to_string(), 160),
    );

    // The cluster budget is a knob, not a constant: with minDiffCluster 0 the
    // same AA-eligible pixel is larger than the budget and counts.
    let aa_knob = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "aa_probe", "minDiffCluster": 0}),
        &b,
    )
    .await;
    check(
        "aa-min-cluster-knob",
        sc_of(&aa_knob).get("pass").and_then(|v| v.as_bool()) == Some(false)
            && sc_of(&aa_knob).get("diffPixels").and_then(|v| v.as_i64()) == Some(1),
        &truncate(&aa_knob.to_string(), 160),
    );

    // A real 1 px colour change in a flat fill: not AA-eligible (the baseline
    // neighbourhood has no contrast), so it counts even with AA on.
    let _ = crate::tools::call("navigate", &serde_json::json!({"url": curl.clone()}), &b).await;
    let _ = crate::tools::call(
        "evaluate",
        &serde_json::json!({"expression": "(() => { const g = document.getElementById('c').getContext('2d'); g.fillStyle = '#ffffff'; g.fillRect(30, 85, 1, 1); return 'one'; })()"}),
        &b,
    )
    .await;
    let one_px = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "aa_probe"}),
        &b,
    )
    .await;
    check(
        "aa-flat-1px-fails",
        sc_of(&one_px).get("pass").and_then(|v| v.as_bool()) == Some(false)
            && sc_of(&one_px).get("diffPixels").and_then(|v| v.as_i64()) == Some(1),
        &truncate(&one_px.to_string(), 160),
    );

    // A 2x2 block is a 4-pixel component: larger than minDiffCluster, so it
    // counts even where it borders an edge.
    let _ = crate::tools::call("navigate", &serde_json::json!({"url": curl.clone()}), &b).await;
    let _ = crate::tools::call(
        "evaluate",
        &serde_json::json!({"expression": "(() => { const g = document.getElementById('c').getContext('2d'); g.fillStyle = '#ffffff'; g.fillRect(60, 85, 2, 2); return 'block'; })()"}),
        &b,
    )
    .await;
    let block = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "aa_probe"}),
        &b,
    )
    .await;
    check(
        "aa-2x2-block-fails",
        sc_of(&block).get("pass").and_then(|v| v.as_bool()) == Some(false)
            && sc_of(&block).get("diffPixels").and_then(|v| v.as_i64()) == Some(4),
        &truncate(&block.to_string(), 160),
    );

    // ---------------------------------------------- baseline environment meta
    // The sidecar records what rendered the baseline; a mismatch is a warning
    // by default and a failure only when the caller requires a match.
    let _ = crate::tools::call("navigate", &serde_json::json!({"url": curl.clone()}), &b).await;
    let esave = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "save", "name": "env_probe"}),
        &b,
    )
    .await;
    let esave_sc = sc_of(&esave);
    let meta_path = vdir.join("env_probe.meta.json");
    let meta_text = std::fs::read_to_string(&meta_path).unwrap_or_default();
    let meta: serde_json::Value =
        serde_json::from_str(&meta_text).unwrap_or(serde_json::Value::Null);
    let meta_browser = meta.get("browser").and_then(|v| v.as_str()).unwrap_or("");
    let meta_major = meta
        .get("browserMajor")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let meta_w = meta.pointer("/viewport/width").and_then(|v| v.as_i64());
    let meta_h = meta.pointer("/viewport/height").and_then(|v| v.as_i64());
    let meta_dsf = meta.get("deviceScaleFactor").and_then(|v| v.as_f64());
    let meta_url = meta.get("url").and_then(|v| v.as_str()).unwrap_or("");
    let meta_created = meta.get("createdAt").and_then(|v| v.as_str()).unwrap_or("");
    check(
        "visual-meta-sidecar",
        meta_browser.contains("Chrome")
            && meta_major > 0
            && meta_w == Some(800)
            && meta_h == Some(600)
            && meta_dsf == Some(1.0)
            && meta_url.contains("canvas.html")
            && meta.get("checksum").and_then(|v| v.as_str())
                == esave_sc.get("checksum").and_then(|v| v.as_str())
            && meta_created.len() == 20
            && meta_created.ends_with('Z'),
        &format!(
            "browser={meta_browser} major={meta_major} viewport={meta_w:?}x{meta_h:?} dsf={meta_dsf:?} createdAt={meta_created}"
        ),
    );
    let ematch = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "env_probe"}),
        &b,
    )
    .await;
    check(
        "visual-env-match",
        sc_of(&ematch)
            .pointer("/environment/match")
            .and_then(|v| v.as_bool())
            == Some(true)
            && sc_of(&ematch).get("warning").map(|w| w.is_null()) == Some(true)
            && sc_of(&ematch).get("pass").and_then(|v| v.as_bool()) == Some(true),
        &truncate(&ematch.to_string(), 200),
    );

    // Simulate browser-update rot by rewriting the recorded viewport. Pixels
    // are untouched, so this proves the environment check alone decides.
    let mut edited = meta.clone();
    edited["viewport"] = serde_json::json!({"width": 640, "height": 480});
    let _ = std::fs::write(
        &meta_path,
        serde_json::to_string_pretty(&edited).unwrap_or_default(),
    );
    let enomatch = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "env_probe"}),
        &b,
    )
    .await;
    let enomatch_sc = sc_of(&enomatch);
    let env_warning = enomatch_sc
        .get("warning")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    check(
        "visual-env-mismatch-warns",
        enomatch_sc
            .pointer("/environment/match")
            .and_then(|v| v.as_bool())
            == Some(false)
            && env_warning.contains("baseline_environment_mismatch")
            && enomatch_sc.get("pass").and_then(|v| v.as_bool()) == Some(true),
        &truncate(&enomatch.to_string(), 220),
    );
    let erequired = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "env_probe", "requireEnvironmentMatch": true}),
        &b,
    )
    .await;
    check(
        "visual-require-env-fails",
        sc_of(&erequired).get("pass").and_then(|v| v.as_bool()) == Some(false)
            && sc_of(&erequired)
                .pointer("/environment/match")
                .and_then(|v| v.as_bool())
                == Some(false),
        &truncate(&erequired.to_string(), 200),
    );
    let eupdate = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "env_probe", "update": true}),
        &b,
    )
    .await;
    let meta_after: serde_json::Value = std::fs::read_to_string(&meta_path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(serde_json::Value::Null);
    check(
        "visual-update-rewrites-sidecar",
        sc_of(&eupdate).get("updated").and_then(|v| v.as_bool()) == Some(true)
            && meta_after
                .pointer("/viewport/width")
                .and_then(|v| v.as_i64())
                == Some(800),
        &truncate(&format!("{eupdate} meta={meta_after}"), 220),
    );

    // ------------------------------------------ scratch target stays in back
    // The diff tab is created with background:true (never activates) and is
    // closed before the reply, so the page-target count is unchanged.
    let pages_before = page_targets(&b).await;
    let bgcmp = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "env_probe"}),
        &b,
    )
    .await;
    let pages_after = page_targets(&b).await;
    check(
        "visual-scratch-background",
        sc_of(&bgcmp)
            .get("scratchBackground")
            .and_then(|v| v.as_bool())
            == Some(true)
            && pages_before == pages_after
            && pages_before > 0,
        &format!(
            "scratchBackground={:?} pages {pages_before}->{pages_after}",
            sc_of(&bgcmp).get("scratchBackground")
        ),
    );

    // ------------------------------------------------------- unknown metadata
    // A sidecar that is missing, malformed or unreadable is NOT a match: the
    // strict gate must not pass without evidence.
    let meta_file = vdir.join("env_probe.meta.json");
    let _ = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "env_probe", "update": true}),
        &b,
    )
    .await;
    std::fs::remove_file(&meta_file).expect("remove sidecar");

    let unknown = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "env_probe"}),
        &b,
    )
    .await;
    let unknown_sc = sc_of(&unknown);
    let unknown_warning = unknown_sc
        .get("warning")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    check(
        "visual-meta-missing-warns",
        unknown.get("isError").is_none()
            && unknown_sc
                .pointer("/environment/known")
                .and_then(|v| v.as_bool())
                == Some(false)
            && unknown_sc
                .pointer("/environment/match")
                .and_then(|v| v.as_bool())
                == Some(false)
            && unknown_warning.contains("baseline_environment_unknown")
            && unknown_sc.get("pass").and_then(|v| v.as_bool()) == Some(true),
        &truncate(&unknown.to_string(), 220),
    );
    let missing_req = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "env_probe", "requireEnvironmentMatch": true}),
        &b,
    )
    .await;
    check(
        "visual-meta-missing-fails-when-required",
        missing_req.get("isError").is_some()
            && sc_of(&missing_req).get("code").and_then(|v| v.as_str())
                == Some("baseline_environment_unknown")
            && sc_of(&missing_req).get("reason").and_then(|v| v.as_str()) == Some("missing"),
        &truncate(&missing_req.to_string(), 220),
    );

    // Malformed JSON.
    std::fs::write(&meta_file, "{ this is not json").expect("write malformed sidecar");
    let malformed = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "env_probe", "requireEnvironmentMatch": true}),
        &b,
    )
    .await;
    check(
        "visual-meta-malformed-fails",
        malformed.get("isError").is_some()
            && sc_of(&malformed).get("code").and_then(|v| v.as_str())
                == Some("baseline_environment_unknown")
            && sc_of(&malformed).get("reason").and_then(|v| v.as_str()) == Some("malformed"),
        &truncate(&malformed.to_string(), 220),
    );

    // Unreadable: a directory where the sidecar file should be (the read
    // fails with a non-NotFound error).
    std::fs::remove_file(&meta_file).expect("remove malformed sidecar");
    let as_dir = std::fs::create_dir(&meta_file).is_ok();
    let unreadable = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "env_probe", "requireEnvironmentMatch": true}),
        &b,
    )
    .await;
    check(
        "visual-meta-unreadable-fails",
        as_dir
            && unreadable.get("isError").is_some()
            && sc_of(&unreadable).get("code").and_then(|v| v.as_str())
                == Some("baseline_environment_unknown")
            && sc_of(&unreadable).get("reason").and_then(|v| v.as_str()) == Some("unreadable"),
        &truncate(&unreadable.to_string(), 220),
    );
    let _ = std::fs::remove_dir_all(&meta_file);
    // Restore a healthy sidecar for the checks that follow.
    let _ = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "env_probe", "update": true}),
        &b,
    )
    .await;

    // ------------------------------------------------- scratch refusal (typed)
    // An engine that cannot create the diff tab in the background must be
    // refused, never retried in the foreground (that would activate a tab).
    std::env::set_var("GLADIUS_TEST_SCRATCH_REJECT_BACKGROUND", "1");
    let pages_before_reject = page_targets(&b).await;
    let rejected = crate::tools::call(
        "visual",
        &serde_json::json!({"action": "compare", "name": "env_probe"}),
        &b,
    )
    .await;
    let pages_after_reject = page_targets(&b).await;
    std::env::remove_var("GLADIUS_TEST_SCRATCH_REJECT_BACKGROUND");
    check(
        "scratch-background-unsupported",
        rejected.get("isError").is_some()
            && sc_of(&rejected).get("code").and_then(|v| v.as_str())
                == Some("scratch_background_unsupported")
            && pages_before_reject == pages_after_reject,
        &format!(
            "pages {pages_before_reject}->{pages_after_reject} {}",
            truncate(&rejected.to_string(), 180)
        ),
    );

    // ----------------------------------------------- cancel entry-window race
    // The dispatch/registry/cancel path only exists in a real server, so this
    // check drives one as a child process. The hooks park the call in the
    // entry window and skip the abort, so the liveness recheck under the
    // section gate is the only thing that can keep the capture out.
    let race = match McpChild::spawn(&[
        ("GLADIUS_TEST_ENTRY_MS", "300"),
        ("GLADIUS_TEST_ABORT_HOLD_MS", "1500"),
        ("GLADIUS_TEST_ABORT_SKIP", "1"),
        ("GLADIUS_TEST_CRITICAL_DELAY_MS", "3000"),
    ]) {
        Ok(mut srv) => {
            let mut run = || -> Result<String, String> {
                srv.request(
                    "initialize",
                    serde_json::json!({"protocolVersion": "2025-06-18"}),
                )?;
                srv.call("navigate", serde_json::json!({"url": curl.clone()}))?;
                srv.call(
                    "viewport",
                    serde_json::json!({"width": 900, "height": 700, "deviceScaleFactor": 2}),
                )?;
                srv.send_tool(10, "screenshot", serde_json::json!({"deterministic": true}))?;
                std::thread::sleep(std::time::Duration::from_millis(100));
                srv.notify(
                    "notifications/cancelled",
                    serde_json::json!({"requestId": 10}),
                )?;
                // Past the gate hand-over (entry 300 + hold 1500); a task that
                // entered would hold the page overridden for 3 s.
                std::thread::sleep(std::time::Duration::from_millis(2000));
                srv.send_tool(
                    20,
                    "wait_for",
                    serde_json::json!({"expression": "document.getElementById('__gladius_deterministic') === null && window.devicePixelRatio === 2", "ms": 1500}),
                )?;
                let w = srv.wait_id(20)?;
                if w.pointer("/result/isError").is_some() {
                    return Err(format!("cancelled capture entered the section: {w}"));
                }
                Ok("entry-window cancel never mutated".to_string())
            };
            run()
        }
        Err(e) => Err(e),
    };
    match race {
        Ok(detail) => check("cancel-entry-window", true, &detail),
        Err(detail) => check("cancel-entry-window", false, &detail),
    }

    // ------------------------------------------------- background rendering
    // A tab that is NOT the active page must still render, still screenshot
    // and still run its timers (the launch flags exist for exactly this).
    let back = crate::tools::call("navigate", &serde_json::json!({"url": url}), &b).await;
    check(
        "background-navigate",
        back.get("isError").is_none(),
        &truncate(&back.to_string(), 120),
    );
    let _ = crate::tools::call(
        "evaluate",
        &serde_json::json!({"expression": "window.__gladiusTicks = 0; setInterval(function () { window.__gladiusTicks++; }, 50); 'armed'"}),
        &b,
    )
    .await;
    let fg = crate::tools::call("screenshot", &serde_json::json!({}), &b).await;
    let fg_meta = sc_of(&fg);
    let fg_b64 = content_image(&fg);
    let _ = crate::tools::call(
        "tabs",
        &serde_json::json!({"action": "new", "url": "about:blank"}),
        &b,
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    let bg_sid = background_session(&b, "selftest.html").await;
    let mut bg_same = false;
    let mut bg_detail = String::from("no background session");
    if let Some(sid) = &bg_sid {
        let ticks = crate::tools::call(
            "cdp",
            &serde_json::json!({"method": "Runtime.evaluate",
                "params": {"expression": "window.__gladiusTicks", "returnByValue": true},
                "session": sid}),
            &b,
        )
        .await;
        let tick_n = sc_of(&ticks)
            .pointer("/result/result/value")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let shot = crate::tools::call(
            "cdp",
            &serde_json::json!({"method": "Page.captureScreenshot",
                "params": {"format": "png", "fromSurface": true},
                "session": sid}),
            &b,
        )
        .await;
        use base64::Engine as _;
        let bg_b64 = sc_of(&shot)
            .pointer("/result/data")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let bg_bytes = base64::engine::general_purpose::STANDARD
            .decode(bg_b64.as_bytes())
            .unwrap_or_default();
        let fg_bytes = base64::engine::general_purpose::STANDARD
            .decode(fg_b64.as_bytes())
            .unwrap_or_default();
        let bg_dims = png_dims(&bg_bytes);
        let fg_dims = png_dims(&fg_bytes);
        bg_same = !bg_bytes.is_empty() && bg_bytes == fg_bytes;
        bg_detail = format!(
            "ticks={tick_n} bgBytes={} fgBytes={} bgDims={bg_dims:?} fgDims={fg_dims:?}",
            bg_bytes.len(),
            fg_bytes.len()
        );
        check(
            "background-timers",
            tick_n >= 10,
            &format!("ticks in 1.2s = {tick_n} (throttled would be ~0-1)"),
        );
        check(
            "background-renders",
            bg_dims.is_some() && bg_dims == fg_dims && bg_bytes.len() > 1000,
            &bg_detail,
        );
        check(
            "background-identical-pixels",
            bg_same,
            &format!(
                "foreground {} bytes vs background, dims={fg_dims:?}",
                fg_meta.get("bytes").and_then(|v| v.as_i64()).unwrap_or(0)
            ),
        );
    } else {
        check("background-renders", false, &bg_detail);
        check("background-identical-pixels", bg_same, &bg_detail);
    }
    let _ = crate::tools::call(
        "tabs",
        &serde_json::json!({"action": "close", "index": 1}),
        &b,
    )
    .await;

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

/// The PNG base64 of the first `image` content block (screenshot replies).
fn content_image(v: &serde_json::Value) -> String {
    v.get("content")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|b| b.get("data"))
        .and_then(|d| d.as_str())
        .unwrap_or("")
        .to_string()
}

/// Minimal MCP server child used by the selftest checks that need the real
/// dispatch path (registry, cancellation, exec lock). Synchronous on purpose:
/// the selftest runtime is current-thread and nothing else is in flight.
struct McpChild {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    rx: std::sync::mpsc::Receiver<String>,
    next_id: i64,
}

impl McpChild {
    fn spawn(env_extra: &[(&str, &str)]) -> Result<Self, String> {
        use std::process::{Command, Stdio};
        let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
        let mut cmd = Command::new(exe);
        cmd.arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (k, v) in env_extra {
            cmd.env(k, v);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("spawn selftest child: {e}"))?;
        let stdin = child.stdin.take().ok_or("child stdin")?;
        let stdout = child.stdout.take().ok_or("child stdout")?;
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut r = std::io::BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match std::io::BufRead::read_line(&mut r, &mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if tx.send(line.trim_end().to_string()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(McpChild {
            child,
            stdin,
            rx,
            next_id: 1,
        })
    }

    fn send(&mut self, v: &serde_json::Value) -> Result<(), String> {
        use std::io::Write as _;
        writeln!(self.stdin, "{v}").map_err(|e| format!("write child stdin: {e}"))?;
        self.stdin
            .flush()
            .map_err(|e| format!("flush child stdin: {e}"))
    }

    fn notify(&mut self, method: &str, params: serde_json::Value) -> Result<(), String> {
        self.send(&serde_json::json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    fn send_tool(&mut self, id: i64, name: &str, args: serde_json::Value) -> Result<(), String> {
        self.send(
            &serde_json::json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": name, "arguments": args}}),
        )
    }

    fn wait_id(&mut self, id: i64) -> Result<serde_json::Value, String> {
        let line = self
            .rx
            .recv_timeout(std::time::Duration::from_secs(120))
            .map_err(|e| format!("child reply for {id}: {e}"))?;
        let v: serde_json::Value =
            serde_json::from_str(&line).map_err(|e| format!("child json: {e}"))?;
        if v.get("id").and_then(|i| i.as_i64()) == Some(id) {
            return Ok(v);
        }
        // Any other frame is a response nobody asked for (or one a cancelled
        // call must not have sent): fail loudly.
        Err(format!(
            "unexpected child frame while waiting for {id}: {v}"
        ))
    }

    fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(
            &serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        )?;
        self.wait_id(id)
    }

    fn call(&mut self, name: &str, args: serde_json::Value) -> Result<serde_json::Value, String> {
        self.request(
            "tools/call",
            serde_json::json!({"name": name, "arguments": args}),
        )
    }
}

impl Drop for McpChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Page targets currently open (the scratch diff tab would be one more).
async fn page_targets(b: &Browser) -> i64 {
    b.cdp_raw("Target.getTargets", serde_json::json!({}), None)
        .await
        .ok()
        .and_then(|v| {
            v.get("targetInfos").and_then(|t| t.as_array()).map(|a| {
                a.iter()
                    .filter(|t| t.get("type").and_then(|x| x.as_str()) == Some("page"))
                    .count() as i64
            })
        })
        .unwrap_or(-1)
}

fn sc_of(v: &serde_json::Value) -> serde_json::Value {
    v.get("structuredContent")
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

/// First per-check record with this check name.
fn find_check<'a>(sc: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
    sc.get("checks")?
        .as_array()?
        .iter()
        .find(|c| c.get("check").and_then(|v| v.as_str()) == Some(name))
}

/// PNG IHDR dimensions (bytes 16..24). Dependency free: the signature and
/// the header are all this needs, and it proves a capture is a real frame of
/// the expected size rather than an empty payload.
fn png_dims(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 24 || bytes[0..8] != SIG || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Some((w, h))
}

/// Attach to the page target whose URL contains `needle` and return its
/// session id (used to drive a NON-active, backgrounded page directly).
async fn background_session(b: &Browser, needle: &str) -> Option<String> {
    let targets = b
        .cdp_raw("Target.getTargets", serde_json::json!({}), None)
        .await
        .ok()?;
    let infos = targets.get("targetInfos")?.as_array()?;
    let tid = infos
        .iter()
        .find(|t| {
            t.get("type").and_then(|v| v.as_str()) == Some("page")
                && t.get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .contains(needle)
        })?
        .get("targetId")?
        .as_str()?
        .to_string();
    let attached = b
        .cdp_raw(
            "Target.attachToTarget",
            serde_json::json!({"targetId": tid, "flatten": true}),
            None,
        )
        .await
        .ok()?;
    Some(attached.get("sessionId")?.as_str()?.to_string())
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
