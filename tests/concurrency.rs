//! G2a concurrency + cancellation: ping during a long call, cancel
//! suppression, pre-start cancel, read-only overlap, EOF with a call in
//! flight, and resultType preservation after the dispatch rewrite.
//! Spawns `gladius mcp` over stdio. No browser needed: every `wait_for`
//! here is a bare `ms` delay (no selector/text/network/expression), which
//! never launches the engine.

use std::io::{BufRead, BufReader, Write};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

struct Client {
    stdin: Option<ChildStdin>,
    rx: Receiver<String>,
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
                    Ok(0) => break, // EOF: server closed stdout, clean end.
                    Ok(_) => {
                        let _ = tx.send(line.trim_end().to_string());
                    }
                    Err(e) => {
                        // Read errors must fail loudly downstream, never
                        // vanish: forward a sentinel the parser rejects
                        // with the raw error attached.
                        let _ = tx.send(format!("READ_ERROR: {e}"));
                        break;
                    }
                }
            }
        });
        Client {
            stdin: Some(stdin),
            rx,
            child,
        }
    }

    fn send(&mut self, v: &Value) {
        let line = v.to_string();
        writeln!(self.stdin.as_mut().expect("stdin open"), "{line}").expect("write stdin");
        self.stdin
            .as_mut()
            .expect("stdin open")
            .flush()
            .expect("flush");
    }

    fn call(&mut self, id: Value, name: &str, args: Value) {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": name, "arguments": args}}));
    }

    fn wait_for(&mut self, id: Value, ms: i64) {
        self.call(id, "wait_for", json!({"ms": ms}));
    }

    /// Next raw stdout line, up to `dur`. `None` only on timeout. A
    /// disconnected channel means the reader thread ended early (server
    /// gone or stdout broken) — that is a loud failure, never a quiet
    /// timeout, so a disconnect can never be mistaken for one.
    fn next_raw(&self, dur: Duration) -> Option<String> {
        match self.rx.recv_timeout(dur) {
            Ok(frame) => Some(frame),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                panic!("stdout channel disconnected (server stdout reader ended)")
            }
        }
    }

    /// Parse one stdout line losslessly: a blank, non-JSON, non-object, or
    /// non-2.0 line is a test failure carrying the raw line — malformed
    /// server output (including the reader's `READ_ERROR` sentinel) must
    /// never escape the exact-frame-count assertions by being silently
    /// skipped.
    fn parse_frame(line: &str) -> Value {
        assert!(
            !line.trim().is_empty(),
            "blank stdout line from server: {line:?}"
        );
        let v: Value = serde_json::from_str(line)
            .unwrap_or_else(|_| panic!("non-JSON stdout line from server: {line:?}"));
        assert!(v.is_object(), "stdout frame is not a JSON object: {line:?}");
        assert_eq!(
            v.get("jsonrpc"),
            Some(&json!("2.0")),
            "stdout frame without jsonrpc 2.0: {line:?}"
        );
        // This server sends only response frames on stdout: every frame
        // must carry an id (string/number/null) and exactly one of
        // result/error. Bare or response-less frames fail with the raw
        // line instead of slipping through the count assertions.
        let id_ok = v
            .get("id")
            .map(|id| id.is_string() || id.is_number() || id.is_null())
            .unwrap_or(false);
        assert!(id_ok, "stdout frame without id: {line:?}");
        let obj = v.as_object().expect("object checked above");
        let has_result = obj.contains_key("result");
        let has_error = obj.contains_key("error");
        assert!(
            has_result != has_error,
            "stdout frame must carry exactly one of result/error: {line:?}"
        );
        v
    }

    /// Next parsed frame, up to `dur`. `None` only on timeout.
    fn next(&self, dur: Duration) -> Option<Value> {
        self.next_raw(dur).map(|line| Self::parse_frame(&line))
    }

    /// Drain lines for `dur`, returning all parsed responses. Nothing is
    /// ever discarded: callers assert on the complete vector.
    fn drain(&self, dur: Duration) -> Vec<Value> {
        self.drain_stamped(Instant::now() + dur)
            .into_iter()
            .map(|(_, v)| v)
            .collect()
    }

    /// Collect every stdout frame until `end`, timestamped at receipt.
    /// Nothing is ever discarded: callers assert on the complete vector.
    fn drain_stamped(&self, end: Instant) -> Vec<(Instant, Value)> {
        let mut out = Vec::new();
        loop {
            let now = Instant::now();
            if now >= end {
                break;
            }
            match self.next(end - now) {
                Some(v) => out.push((Instant::now(), v)),
                None => continue,
            }
        }
        out
    }

    /// Wait until a response with `id` arrives (nothing ignored), up to
    /// `dur`. Any other valid frame seen meanwhile fails the test.
    /// NOTE: discards nothing; unexpected frames are collected and
    /// asserted empty.
    fn wait_id(&self, id: &Value, dur: Duration, what: &str) -> Value {
        self.wait_ids(std::slice::from_ref(id), dur, what).remove(0)
    }

    /// Wait until responses for ALL of `ids` have arrived (nothing
    /// ignored), up to `dur`. Returns them in `ids` order. Unexpected or
    /// duplicate frames fail the test instead of being discarded: there is
    /// no allowed set — `wait_ids_extra` plus the `drain_stamped` windows
    /// already account for every frame, so any extra here is a defect.
    fn wait_ids(&self, ids: &[Value], dur: Duration, what: &str) -> Vec<Value> {
        let (matched, extra) = self.wait_ids_extra(ids, dur, what);
        assert!(
            extra.is_empty(),
            "unexpected frames while waiting for ids {ids:?} ({what}): {extra:?}"
        );
        matched
    }

    /// Collect frames until every id in `ids` has arrived (in order) or
    /// `dur` elapses. Returns `(matched, extra)`: `matched` parallels
    /// `ids`; `extra` holds every other valid frame seen meanwhile —
    /// unmatched ids and duplicates of already-satisfied ids — recorded,
    /// never discarded.
    fn wait_ids_extra(&self, ids: &[Value], dur: Duration, what: &str) -> (Vec<Value>, Vec<Value>) {
        let end = Instant::now() + dur;
        let mut seen: Vec<Option<Value>> = ids.iter().map(|_| None).collect();
        let mut missing = ids.len();
        let mut extra = Vec::new();
        while missing > 0 {
            let now = Instant::now();
            assert!(
                now < end,
                "timeout waiting for ids {ids:?} ({what}); extras so far: {extra:?}"
            );
            match self.next(end - now) {
                Some(v) => {
                    let slot = ids
                        .iter()
                        .enumerate()
                        .find(|(i, id)| seen[*i].is_none() && v.get("id") == Some(id))
                        .map(|(i, _)| i);
                    match slot {
                        Some(i) => {
                            seen[i] = Some(v);
                            missing -= 1;
                        }
                        None => extra.push(v),
                    }
                }
                None => continue,
            }
        }
        (seen.into_iter().map(|o| o.expect("seen")).collect(), extra)
    }

    fn close_stdin(&mut self) {
        self.stdin.take();
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.stdin.take();
        let _ = self.child.kill();
    }
}

/// Lossless observation-window assertion: every frame in `resps` is
/// accounted for — no frame may carry the forbidden (cancelled) id, and
/// every other frame's id must be in the caller-passed `allowed` set
/// (empty when nothing else should arrive). Malformed/blank frames never
/// reach here: `parse_frame` already rejects them. A valid unexpected id
/// (e.g. 999) fails instead of passing silently.
fn assert_window(resps: &[Value], forbidden: &Value, allowed: &[Value], what: &str) {
    for r in resps {
        let id = r
            .get("id")
            .unwrap_or_else(|| panic!("{what}: id-less frame in window: {r:?}"));
        assert!(
            id != forbidden,
            "{what}: unexpected response for forbidden id {forbidden}: {r}"
        );
        assert!(
            allowed.iter().any(|a| id == a),
            "{what}: unexpected id outside allowed set: {r}"
        );
    }
}

#[test]
fn ping_during_long_call() {
    let mut c = Client::spawn();
    c.wait_for(json!(1), 5000);
    std::thread::sleep(Duration::from_millis(300));
    let t0 = Instant::now();
    c.send(&json!({"jsonrpc": "2.0", "id": 2, "method": "ping", "params": {}}));
    // The ping reply must arrive while the 5s wait is still in flight:
    // strictly less than 1s, and no id-1 response may precede it.
    let pong = c.wait_id(&json!(2), Duration::from_secs(1), "ping during wait");
    let ping_dt = t0.elapsed();
    assert!(pong.get("error").is_none(), "ping must not error: {pong}");
    assert_eq!(pong.get("result"), Some(&json!({})), "ping result: {pong}");
    assert!(
        ping_dt < Duration::from_secs(1),
        "ping answered in {ping_dt:?} while wait in flight"
    );
    // The long call still completes afterwards with its own id.
    let done = c.wait_id(&json!(1), Duration::from_secs(10), "wait completion");
    assert!(
        done.get("error").is_none(),
        "wait completes after ping: {done}"
    );
}

#[test]
fn cancelled_call_sends_no_response() {
    let mut c = Client::spawn();
    let t_start = Instant::now();
    c.wait_for(json!(1), 5000);
    std::thread::sleep(Duration::from_millis(300));
    c.send(
        &json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
        "params": {"requestId": 1}}),
    );
    // No response for the cancelled id within 2s; nothing else either.
    let got = c.drain(Duration::from_secs(2));
    assert_window(&got, &json!(1), &[], "cancelled wait");
    // Ping still answers on a live server.
    let t0 = Instant::now();
    c.send(&json!({"jsonrpc": "2.0", "id": 2, "method": "ping", "params": {}}));
    let pong = c.wait_id(&json!(2), Duration::from_secs(2), "ping after cancel");
    assert_eq!(pong.get("result"), Some(&json!({})), "ping ok: {pong}");
    assert!(
        t0.elapsed() < Duration::from_secs(2),
        "ping fast after cancel"
    );
    // No late response for the cancelled id after another 1s; nothing else.
    let late = c.drain(Duration::from_secs(1));
    assert_window(&late, &json!(1), &[], "no late response");
    // Keep observing PAST the original 5s deadline: the cancelled wait
    // would have completed by then, and still nothing may arrive.
    let deadline = t_start + Duration::from_millis(6500);
    let rest = c.drain(deadline.saturating_duration_since(Instant::now()));
    assert_window(&rest, &json!(1), &[], "no response past 5s deadline");
}

#[test]
fn duplicate_id_newest_generation_wins_single_response() {
    let mut c = Client::spawn();
    let t0 = Instant::now();
    c.wait_for(json!(7), 2000); // older generation: superseded, must stay silent
    c.wait_for(json!(7), 100); // newer generation: answers fast
                               // The whole 3s window (past the older 2s deadline) must contain
                               // exactly one frame: the newest generation's fast answer.
    let got = c.drain_stamped(t0 + Duration::from_secs(3));
    assert_eq!(
        got.len(),
        1,
        "exactly one response for reused id 7: {got:?}"
    );
    let (at, only) = &got[0];
    assert_eq!(only.get("id"), Some(&json!(7)), "id 7: {got:?}");
    assert!(
        only.get("error").is_none(),
        "newest generation answers ok: {got:?}"
    );
    assert!(
        *at - t0 < Duration::from_secs(1),
        "single response arrives with the short wait"
    );
}

#[test]
fn duplicate_id_at_capacity_single_response() {
    // Repro: 4 active long calls fill the semaphore; reusing id 1 must
    // yield exactly one response for id 1 — the replacement result (if its
    // permit freed) or a single busy error — never both, and never the
    // superseded old result after its deadline. The whole window is
    // collected: exactly four frames, nothing else.
    let mut c = Client::spawn();
    let t0 = Instant::now();
    for id in 1..=4 {
        c.wait_for(json!(id), 1200);
    }
    std::thread::sleep(Duration::from_millis(200));
    c.wait_for(json!(1), 100); // supersedes the in-flight id 1
                               // Observe past the old 1200ms deadline (plus margin).
    let got = c.drain_stamped(t0 + Duration::from_secs(3));
    assert_eq!(
        got.len(),
        4,
        "exactly four responses, nothing else: {got:?}"
    );
    let ones: Vec<&(Instant, Value)> = got
        .iter()
        .filter(|p| p.1.get("id") == Some(&json!(1)))
        .collect();
    assert_eq!(
        ones.len(),
        1,
        "exactly one response for id 1 (replacement or busy, not both): {got:?}"
    );
    let (at1, only) = ones[0];
    assert!(
        *at1 - t0 < Duration::from_secs(1),
        "id-1 frame arrives fast (replacement ~100ms or immediate busy)"
    );
    if only.get("error").is_some() {
        assert_eq!(
            only.get("error").and_then(|e| e.get("code")),
            Some(&json!(-32603)),
            "non-result for id 1 must be busy: {only}"
        );
    } else {
        assert!(
            only.get("result").is_some(),
            "id 1 replacement answers ok: {only}"
        );
    }
    // The other three long calls were never superseded: one response each.
    for id in 2..=4 {
        let n = got
            .iter()
            .filter(|p| p.1.get("id") == Some(&json!(id)))
            .count();
        assert_eq!(n, 1, "id {id} answers exactly once: {got:?}");
    }
}

#[test]
fn duplicate_id_invalid_tool_name_single_error() {
    // id 77 runs a long wait; a second frame with id 77 but no tool name
    // supersedes it before validation. The whole window (past the old
    // 400ms deadline) must contain exactly one frame: the -32602.
    let mut c = Client::spawn();
    let t0 = Instant::now();
    c.wait_for(json!(77), 400);
    std::thread::sleep(Duration::from_millis(50));
    c.send(&json!({"jsonrpc": "2.0", "id": 77, "method": "tools/call",
        "params": {}}));
    std::thread::sleep(Duration::from_millis(600));
    let got = c.drain_stamped(t0 + Duration::from_secs(3));
    assert_eq!(got.len(), 1, "exactly one frame for reused id 77: {got:?}");
    let (_, only) = &got[0];
    assert_eq!(only.get("id"), Some(&json!(77)), "id 77: {got:?}");
    assert_eq!(
        only.get("error").and_then(|e| e.get("code")),
        Some(&json!(-32602)),
        "duplicate invalid call is -32602: {got:?}"
    );
}

#[test]
fn duplicate_id_fast_method_single_response() {
    // id 77 runs a long wait; a ping reusing id 77 supersedes it. The whole
    // window (past the old 400ms deadline) must contain exactly one frame:
    // the fast ping response.
    let mut c = Client::spawn();
    let t0 = Instant::now();
    c.wait_for(json!(77), 400);
    std::thread::sleep(Duration::from_millis(50));
    c.send(&json!({"jsonrpc": "2.0", "id": 77, "method": "ping", "params": {}}));
    std::thread::sleep(Duration::from_millis(600));
    let got = c.drain_stamped(t0 + Duration::from_secs(3));
    assert_eq!(got.len(), 1, "exactly one frame for reused id 77: {got:?}");
    let (at, only) = &got[0];
    assert_eq!(only.get("id"), Some(&json!(77)), "id 77: {got:?}");
    assert_eq!(only.get("result"), Some(&json!({})), "ping ok: {got:?}");
    assert!(
        *at - t0 < Duration::from_secs(1),
        "single response arrives with the fast ping"
    );
}

#[test]
fn invalid_id_cancel_frame_rejected_call_survives() {
    // A frame with an invalid id type (bool) naming notifications/cancelled
    // must be rejected -32600 with id null: no supersession, no dispatch,
    // so the live call 77 is NOT cancelled and still completes. Ping 88
    // then works on the healthy server. Every stdout frame in the window
    // is collected: exactly three responses and nothing else.
    let mut c = Client::spawn();
    let t0 = Instant::now();
    c.wait_for(json!(77), 400);
    std::thread::sleep(Duration::from_millis(50));
    c.send(
        &json!({"jsonrpc": "2.0", "id": true, "method": "notifications/cancelled",
        "params": {"requestId": 77}}),
    );
    // Let the old 400ms wait run past its deadline, then ping on the same
    // connection. All three responses fall inside one collected window.
    std::thread::sleep(Duration::from_millis(600));
    c.send(&json!({"jsonrpc": "2.0", "id": 88, "method": "ping", "params": {}}));
    let got = c.drain(t0 + Duration::from_secs(3) - Instant::now());
    assert_eq!(
        got.len(),
        3,
        "exactly three responses and nothing else: {got:?}"
    );
    let nulls: Vec<&Value> = got
        .iter()
        .filter(|v| v.get("id") == Some(&json!(null)))
        .collect();
    assert_eq!(nulls.len(), 1, "one null-id frame: {got:?}");
    assert_eq!(
        nulls[0].get("error").and_then(|e| e.get("code")),
        Some(&json!(-32600)),
        "bool id is -32600: {got:?}"
    );
    let w77: Vec<&Value> = got
        .iter()
        .filter(|v| v.get("id") == Some(&json!(77)))
        .collect();
    assert_eq!(w77.len(), 1, "one id-77 frame: {got:?}");
    assert!(
        w77[0].get("error").is_none(),
        "live call 77 not cancelled by bad frame: {w77:?}"
    );
    assert!(w77[0].get("result").is_some(), "wait 77 succeeds: {w77:?}");
    let p88: Vec<&Value> = got
        .iter()
        .filter(|v| v.get("id") == Some(&json!(88)))
        .collect();
    assert_eq!(p88.len(), 1, "one ping frame: {got:?}");
    assert_eq!(p88[0].get("result"), Some(&json!({})), "ping ok: {got:?}");
}

#[test]
fn invalid_version_cancel_frame_rejected_call_survives() {
    // A frame with an invalid `jsonrpc` version ("1.0") carrying
    // notifications/cancelled must be rejected -32600 with its own id:
    // no supersession, no cancellation dispatch, so the live call 1 is
    // NOT cancelled and still completes. Ping 3 then works on the healthy
    // server. Every stdout frame in the window is collected: exactly three
    // responses and nothing else.
    let mut c = Client::spawn();
    let t0 = Instant::now();
    c.wait_for(json!(1), 1200);
    c.send(
        &json!({"jsonrpc": "1.0", "id": 2, "method": "notifications/cancelled",
        "params": {"requestId": 1}}),
    );
    c.send(&json!({"jsonrpc": "2.0", "id": 3, "method": "ping", "params": {}}));
    let got = c.drain(Duration::from_secs(5));
    let _ = t0;
    assert_eq!(
        got.len(),
        3,
        "exactly three responses and nothing else: {got:?}"
    );
    let bad: Vec<&Value> = got
        .iter()
        .filter(|v| v.get("id") == Some(&json!(2)))
        .collect();
    assert_eq!(bad.len(), 1, "one id-2 frame: {got:?}");
    assert_eq!(
        bad[0].get("error").and_then(|e| e.get("code")),
        Some(&json!(-32600)),
        "invalid jsonrpc is -32600: {got:?}"
    );
    let w1: Vec<&Value> = got
        .iter()
        .filter(|v| v.get("id") == Some(&json!(1)))
        .collect();
    assert_eq!(w1.len(), 1, "one id-1 frame: {got:?}");
    assert!(
        w1[0].get("error").is_none(),
        "live call 1 not cancelled by bad-version frame: {w1:?}"
    );
    assert!(w1[0].get("result").is_some(), "wait 1 succeeds: {w1:?}");
    let p3: Vec<&Value> = got
        .iter()
        .filter(|v| v.get("id") == Some(&json!(3)))
        .collect();
    assert_eq!(p3.len(), 1, "one ping frame: {got:?}");
    assert_eq!(p3[0].get("result"), Some(&json!({})), "ping ok: {got:?}");
}

#[test]
fn many_cancels_then_healthy_and_id_reuse() {
    // 1100 wait+cancel pairs: past the 1024 tombstone bound, so eviction
    // runs. Then reuse a cancelled id and check server health.
    let mut c = Client::spawn();
    const N: i64 = 1100;
    for i in 0..N {
        let id = json!(2000 + i);
        c.wait_for(id.clone(), 2000);
        c.send(
            &json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
            "params": {"requestId": id}}),
        );
    }
    // Observe past the 2000ms wait (3s): map every response by id. A
    // cancelled call may only ever surface as a single busy refusal
    // (permit not yet reaped at dispatch); never a result, never twice.
    let got = c.drain(Duration::from_secs(3));
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for v in &got {
        // No id-less frames: `parse_frame` already rejects response-less
        // shapes, and this loop must not skip anything either — every
        // server frame is counted.
        let id = v
            .get("id")
            .unwrap_or_else(|| panic!("id-less stdout frame: {v:?}"));
        let key = id.to_string();
        let n = counts.entry(key.clone()).or_insert(0);
        *n += 1;
        assert_eq!(*n, 1, "duplicate response for id {id}: {got:?}");
        let num = id.as_i64().unwrap_or(-1);
        assert!(
            (2000..2000 + N).contains(&num),
            "unexpected id outside batch window: {v}"
        );
        if v.get("error").is_none() {
            panic!("cancelled id {num} emitted a result: {v}");
        }
        assert_eq!(
            v.get("error").and_then(|e| e.get("code")),
            Some(&json!(-32603)),
            "non-result must be busy: {v}"
        );
    }
    // Reusing a cancelled id must answer (no stale suppression).
    c.wait_for(json!(2000), 100);
    let r = c.wait_id(&json!(2000), Duration::from_secs(5), "reused id answers");
    assert!(r.get("error").is_none(), "id reuse ok: {r}");
    // Server still healthy.
    c.send(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping", "params": {}}));
    let pong = c.wait_id(&json!(1), Duration::from_secs(2), "ping");
    assert_eq!(pong.get("result"), Some(&json!({})), "ping ok: {pong}");
}

#[test]
fn writer_death_exits_cleanly_no_panic() {
    // Close the stdout read end, then ping: the writer's first flush fails,
    // and the server must exit 0 without polling its writer handle twice.
    let bin = env!("CARGO_BIN_EXE_gladius");
    let mut cmd = Command::new(bin);
    cmd.arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = cmd.spawn().expect("spawn gladius mcp");
    let mut stdin = child.stdin.take().expect("child stdin");
    let stdout = child.stdout.take().expect("child stdout");
    let stderr = child.stderr.take().expect("child stderr");
    drop(stdout);
    let (etx, erx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut r = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match r.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let _ = etx.send(line.trim_end().to_string());
                }
                Err(_) => break,
            }
        }
    });
    let ping = json!({"jsonrpc": "2.0", "id": 1, "method": "ping", "params": {}}).to_string();
    writeln!(stdin, "{ping}").expect("write ping");
    stdin.flush().expect("flush ping");
    let end = Instant::now() + Duration::from_secs(8);
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(s) => break s,
            None => {
                assert!(
                    Instant::now() < end,
                    "server did not exit after stdout closed"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    };
    drop(stdin);
    assert!(
        status.success(),
        "writer-dead shutdown exits 0, got {status}"
    );
    let mut errs = Vec::new();
    while let Ok(l) = erx.recv_timeout(Duration::from_millis(200)) {
        errs.push(l);
    }
    for l in &errs {
        assert!(!l.contains("panicked"), "no panic on writer death: {l}");
        assert!(
            !l.contains("JoinHandle"),
            "no JoinHandle error on writer death: {l}"
        );
    }
}

#[test]
fn pre_start_cancel_sends_no_response() {
    let mut c = Client::spawn();
    c.wait_for(json!(1), 5000);
    // Cancel immediately: the call must never produce a response.
    c.send(
        &json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
        "params": {"requestId": 1}}),
    );
    let got = c.drain(Duration::from_secs(2));
    assert_window(&got, &json!(1), &[], "pre-start cancel");
    // Server still alive.
    c.send(&json!({"jsonrpc": "2.0", "id": 2, "method": "ping", "params": {}}));
    let pong = c.wait_id(&json!(2), Duration::from_secs(2), "ping");
    assert_eq!(pong.get("result"), Some(&json!({})), "ping ok: {pong}");
}

#[test]
fn read_only_calls_overlap() {
    let mut c = Client::spawn();
    let t0 = Instant::now();
    c.wait_for(json!(1), 1500);
    c.wait_for(json!(2), 1500);
    let both = c.wait_ids(&[json!(1), json!(2)], Duration::from_secs(10), "both waits");
    let (r1, r2) = (&both[0], &both[1]);
    let dt = t0.elapsed();
    assert!(r1.get("error").is_none(), "wait 1 ok: {r1}");
    assert!(r2.get("error").is_none(), "wait 2 ok: {r2}");
    // Serial execution would take ~3s; overlap lands near max(1.5s).
    assert!(
        dt < Duration::from_millis(2800),
        "two 1.5s waits overlapped, took {dt:?}"
    );
}

#[test]
fn eof_with_call_in_flight_exits_clean() {
    let mut c = Client::spawn();
    c.wait_for(json!(1), 300_000);
    std::thread::sleep(Duration::from_millis(300));
    c.close_stdin();
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        match c.child.try_wait().expect("try_wait") {
            Some(status) => {
                assert!(status.success(), "server exit 0 on EOF, got {status}");
                return;
            }
            None => {
                assert!(Instant::now() < end, "server did not exit within 3s of EOF");
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

#[test]
fn result_type_preserved_after_dispatch_rewrite() {
    let mut c = Client::spawn();
    // Modern tools/list carries resultType complete + 22 tools.
    c.send(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list",
        "params": {"_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28"}}}));
    let modern = c.wait_id(&json!(1), Duration::from_secs(10), "modern tools/list");
    let result = modern.get("result").expect("modern result");
    assert_eq!(
        result.get("resultType"),
        Some(&json!("complete")),
        "modern tools/list resultType: {modern}"
    );
    assert_eq!(
        result
            .get("tools")
            .and_then(|t| t.as_array())
            .map(|a| a.len()),
        Some(22),
        "22 tools: {modern}"
    );
    // Legacy tools/list carries no resultType.
    c.send(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}));
    let legacy = c.wait_id(&json!(2), Duration::from_secs(10), "legacy tools/list");
    let result = legacy.get("result").expect("legacy result");
    assert!(
        result.get("resultType").is_none(),
        "legacy tools/list no resultType: {legacy}"
    );
    // Ping answers {} on both eras.
    c.send(&json!({"jsonrpc": "2.0", "id": 3, "method": "ping", "params": {}}));
    let pong = c.wait_id(&json!(3), Duration::from_secs(10), "ping");
    assert_eq!(pong.get("result"), Some(&json!({})), "ping: {pong}");
}
