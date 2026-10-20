use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::FutureExt;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, Mutex, Semaphore};
use tokio::task::JoinHandle;

use crate::browser::Browser;
use crate::tools;

/// G1 version matrix.
const LEGACY_1: &str = "2025-06-18";
const LEGACY_2: &str = "2025-11-25";
const LATEST_LEGACY: &str = "2025-11-25";
const MODERN: &str = "2026-07-28";

/// Cap on concurrent in-flight `tools/call` tasks. Past this the server
/// answers `-32603 Server busy` instead of queueing unboundedly.
const MAX_IN_FLIGHT_CALLS: usize = 4;

/// Bound for lingering cancelled entries (an aborted task never runs its
/// epilogue, so its entry stays until evicted). Live entries are never
/// evicted: with a 4-slot semaphore only a handful can exist at once.
const MAX_REGISTRY_ENTRIES: usize = 1024;

fn supported_versions() -> Value {
    json!([LEGACY_1, LEGACY_2, MODERN])
}

fn is_supported(v: &str) -> bool {
    v == LEGACY_1 || v == LEGACY_2 || v == MODERN
}

fn is_legacy(v: &str) -> bool {
    v == LEGACY_1 || v == LEGACY_2
}

fn negotiate(client: Option<&str>) -> String {
    match client {
        Some(v) if is_legacy(v) => v.to_string(),
        _ => LATEST_LEGACY.to_string(),
    }
}

fn capabilities() -> Value {
    json!({"tools": {"listChanged": false}})
}

fn instructions() -> &'static str {
    "Gladius is headless-only (no headed mode). Always start with snapshot to get stable @eN refs, then act with @eN refs or CSS selectors. All 22 tools report typed errors (structuredContent code/reason alongside the prose); tool failures are results, never protocol errors."
}

fn server_info() -> Value {
    let info = crate::browser::backend_info();
    let mut m = serde_json::Map::new();
    m.insert("name".to_string(), json!("gladius"));
    m.insert("version".to_string(), json!(env!("CARGO_PKG_VERSION")));
    if let Some(b) = info.get("backend") {
        m.insert("backend".to_string(), b.clone());
    }
    if let Some(caps) = info.get("caps") {
        m.insert("caps".to_string(), caps.clone());
    }
    if let Some(url) = info.get("cdpUrl") {
        m.insert("cdpUrl".to_string(), url.clone());
    }
    // Preserved extras (moved out of capabilities): tool count derived
    // from the real tool list (never hardcoded), and `gated` names the
    // render-dependent tools refused on the CURRENT backend.
    let tool_count = crate::tools::list()
        .get("tools")
        .and_then(|t| t.as_array())
        .map(|a| a.len())
        .unwrap_or(21);
    m.insert("count".to_string(), json!(tool_count));
    let mut gated = serde_json::Map::new();
    if info.get("caps").and_then(|c| c.as_str()) != Some("render") {
        gated.insert("screenshot".to_string(), json!("requires caps: render"));
    }
    m.insert("gated".to_string(), Value::Object(gated));
    Value::Object(m)
}

/// Per-request modern detection: params._meta["io.modelcontextprotocol/protocolVersion"].
fn requested_version(req: &Value) -> Option<String> {
    req.get("params")?
        .as_object()?
        .get("_meta")?
        .as_object()?
        .get("io.modelcontextprotocol/protocolVersion")?
        .as_str()
        .map(|s| s.to_string())
}

/// Canonical request-id key shared by dispatch and cancellation:
/// `serde_json::to_string` of the raw id, so `1` and `"1"` never collide.
fn id_key(id: &Value) -> String {
    serde_json::to_string(id).unwrap_or_default()
}

/// True when the named tool mutates browser/page state and must hold the
/// execution lock. Read-only tools (snapshot, screenshot, wait_for,
/// fetch_text, targets list, console/network get) run lock-free so they
/// overlap freely. `dialog` locks only for the mutating `policy` action;
/// `network`/`console` lock only for `drain`/`clear`; `targets` locks only
/// for `select`.
fn needs_exec_lock(name: &str, args: &Value) -> bool {
    match name {
        "navigate" | "click" | "type" | "select" | "upload" | "key" | "hover" | "viewport"
        | "tabs" | "history" | "cdp" | "evaluate" | "close" => true,
        "targets" => {
            args.get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("list")
                != "list"
        }
        "dialog" => args.get("action").and_then(|v| v.as_str()).unwrap_or("get") != "get",
        "network" | "console" => {
            matches!(
                args.get("action").and_then(|v| v.as_str()).unwrap_or("get"),
                "drain" | "clear"
            )
        }
        "storage" => args.get("action").and_then(|v| v.as_str()).unwrap_or("get") != "get",
        _ => false,
    }
}

/// One in-flight (or recently cancelled) call. Every transition happens
/// under the single registry lock, so cancellation and response emission
/// are totally ordered: either cancel wins (no send) or the send commits
/// first (cancel becomes a no-op). The lock is never held across tool
/// execution — only for check+send sized critical sections.
struct CallState {
    handle: Option<JoinHandle<()>>,
    /// Dispatch generation: a duplicate id supersedes the older entry, and
    /// only the newest generation may emit for the id (the older task goes
    /// silent without touching the new entry).
    gen: u64,
    cancelled: bool,
    emitted: bool,
}

type Registry = Arc<Mutex<HashMap<String, CallState>>>;

/// Drop lingering cancelled entries (aborted tasks never run their
/// epilogue) once the registry grows past its bound. Live entries are
/// never touched.
fn evict_stale_locked(map: &mut HashMap<String, CallState>) {
    if map.len() <= MAX_REGISTRY_ENTRIES {
        return;
    }
    map.retain(|_, s| !(s.cancelled && s.handle.is_none()));
}

/// True when (`key`, `gen`) still owns its entry and has not been
/// cancelled. Brief registry probe; never held across tool execution.
async fn is_live(registry: &Registry, key: &str, gen: u64) -> bool {
    matches!(
        registry.lock().await.get(key),
        Some(st) if st.gen == gen && !st.cancelled
    )
}

/// Remove the entry only when it is still ours; a superseded or evicted
/// entry belongs to someone else (or nobody) and is left alone.
async fn remove_if_mine(registry: &Registry, key: &str, gen: u64) {
    let mut map = registry.lock().await;
    if matches!(map.get(key), Some(st) if st.gen == gen) {
        map.remove(key);
    }
}

/// Try to commit `resp` for (`key`, `gen`): under the single registry lock,
/// send at most once and only when live. Check, mark, and send happen in
/// one critical section (the unbounded send never blocks, so no await
/// under lock), which closes the cancel-after-check race: a cancel either
/// lands before the commit (no send) or after it (harmless no-op).
/// Always removes OUR entry; never another generation's. Returns true when
/// the response was sent.
async fn commit_emit(
    registry: &Registry,
    key: &str,
    gen: u64,
    resp: Value,
    tx: &mpsc::UnboundedSender<Value>,
) -> bool {
    let mut map = registry.lock().await;
    let live = matches!(
        map.get(key),
        Some(st) if st.gen == gen && !st.cancelled && !st.emitted
    );
    if !live {
        if matches!(map.get(key), Some(st) if st.gen == gen) {
            map.remove(key);
        }
        return false;
    }
    if let Some(st) = map.get_mut(key) {
        st.emitted = true;
    }
    // Unbounded send only fails when the writer is gone (shutdown); report
    // unsent so callers never assume delivery. Cleanup still happens.
    let sent = tx.send(resp).is_ok();
    map.remove(key);
    sent
}

/// Response for a tool task that panicked: a protocol-level `-32603` for
/// the id (emitted only when the call was not cancelled), then cleanup via
/// the normal commit path.
fn panic_response(id: &Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id,
        "error": {"code": -32603, "message": "internal error"}})
}

/// Serve MCP over stdio: newline-delimited JSON-RPC 2.0.
/// STDOUT carries only protocol frames; ALL logs go to STDERR.
///
/// Concurrency shape (G2a):
/// - fast methods (initialize, server/discover, tools/list, ping,
///   notifications) run inline on the read loop: they never block behind
///   a running tool.
/// - `tools/call` runs on a spawned task per request, capped by a
///   semaphore (4): past the cap the call is refused `-32603 Server busy`.
/// - one writer task owns stdout; every response travels through an
///   unbounded mpsc channel, so frames are whole lines, never interleaved.
///   A oneshot signals writer exit; the writer `JoinHandle` is awaited
///   exactly once on the shutdown path, so a dead stdout can never panic
///   the server.
/// - `notifications/cancelled` aborts the matching in-flight call task;
///   the aborted call emits no response (single-lock commit).
///
/// Shutdown (EOF or dead stdout) always closes the browser and exits 0.
pub async fn serve() -> Result<(), String> {
    let browser = Arc::new(Browser::new());
    let exec = Arc::new(Mutex::new(()));
    let sem = Arc::new(Semaphore::new(MAX_IN_FLIGHT_CALLS));
    let registry: Registry = Arc::new(Mutex::new(HashMap::new()));
    let gen_ctr = Arc::new(AtomicU64::new(0));

    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    let (writer_done_tx, mut writer_done_rx) = oneshot::channel::<()>();
    // Awaited exactly once, on the shutdown path below. The read loop
    // watches `writer_done_rx`, never the handle itself, so a completed
    // handle is never polled twice (`JoinHandle polled after completion`).
    let writer_handle: JoinHandle<()> = tokio::spawn(async move {
        let mut out = tokio::io::stdout();
        while let Some(v) = rx.recv().await {
            let mut s = v.to_string();
            s.push('\n');
            if out.write_all(s.as_bytes()).await.is_err() {
                break;
            }
            if out.flush().await.is_err() {
                break;
            }
        }
        let _ = writer_done_tx.send(());
    });

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        let line: Option<String> = tokio::select! {
            biased;
            _ = &mut writer_done_rx => {
                // Writer exited (stdout gone or broken): stop accepting.
                // Shared shutdown below closes the browser and exits 0.
                break;
            }
            next = lines.next_line() => {
                next.map_err(|e| format!("stdin: {e}"))?
            }
        };
        let Some(line) = line else {
            break; // EOF: cleanup below.
        };
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                if send_resp(
                    &tx,
                    json!({"jsonrpc": "2.0", "id": null,
                           "error": {"code": -32700, "message": "parse error"}}),
                )
                .is_err()
                {
                    break;
                }
                continue;
            }
        };
        // Strict framing: batch arrays and non-object scalars are one
        // -32600 with id null (never silent).
        if req.is_array() || !req.is_object() {
            if send_resp(
                &tx,
                json!({"jsonrpc": "2.0", "id": null,
                       "error": {"code": -32600, "message": "invalid request"}}),
            )
            .is_err()
            {
                break;
            }
            continue;
        }
        let has_id = req
            .as_object()
            .map(|o| o.contains_key("id"))
            .unwrap_or(false);
        // Strict JSON-RPC version first: `jsonrpc` must be exactly "2.0" on
        // EVERY frame (requests and notifications alike), BEFORE id handling,
        // supersession, method extraction, or cancellation dispatch. A
        // wrong-version frame is -32600 (echoing a valid string/number id,
        // else null) and never disturbs a live call: without this, an
        // invalid-version `notifications/cancelled` would abort the task it
        // names, and an invalid-version frame carrying a live id would
        // supersede (cancel) it.
        if req.get("jsonrpc").and_then(|v| v.as_str()) != Some("2.0") {
            let echo = match req.get("id") {
                Some(v) if v.is_string() || v.is_number() => v.clone(),
                _ => Value::Null,
            };
            if send_resp(
                &tx,
                json!({"jsonrpc": "2.0", "id": echo,
                       "error": {"code": -32600, "message": "invalid request"}}),
            )
            .is_err()
            {
                break;
            }
            continue;
        }
        // Id presence/type next. A frame carrying an `id`
        // key with bool/object/array type is immediately rejected -32600
        // with id null: no supersession, no method dispatch, so a malformed
        // frame can never cancel or disturb a live call.
        // A valid id (string/number/null) supersedes any in-flight call
        // with the same canonical id first — flagged cancelled, aborted,
        // entry removed — so at most one response is ever emitted per
        // IN-FLIGHT id (completed ids are free for sequential reuse).
        // Completed ids are free: sequential reuse is normal JSON-RPC, not
        // a duplicate. The superseded task's epilogue sees a missing entry
        // and stays silent. This covers fast methods too: a duplicate
        // ping/tools/list with an in-flight call id kills the call; only
        // the fast response emits.
        if has_id {
            let raw = req.get("id").cloned().unwrap_or(Value::Null);
            if !(raw.is_string() || raw.is_number() || raw.is_null()) {
                if send_resp(
                    &tx,
                    json!({"jsonrpc": "2.0", "id": null,
                           "error": {"code": -32600, "message": "invalid request"}}),
                )
                .is_err()
                {
                    break;
                }
                continue;
            }
            let key = id_key(&raw);
            let mut map = registry.lock().await;
            if let Some(old) = map.get_mut(&key) {
                old.cancelled = true;
                if let Some(h) = old.handle.take() {
                    h.abort();
                }
                map.remove(&key);
            }
        }
        let method = req
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        // Cancellation is a notification: never a response, even when it
        // carries an `id` of its own.
        if method == "notifications/cancelled" {
            handle_cancel(&req, &registry).await;
            if tx.is_closed() {
                break;
            }
            continue;
        }
        if method == "tools/call" {
            dispatch_call(req, &browser, &exec, &sem, &registry, &gen_ctr, &tx).await;
            if tx.is_closed() {
                break;
            }
            continue;
        }
        if let Some(resp) = handle_fast(&req, &browser).await {
            if send_resp(&tx, resp).is_err() {
                break;
            }
        }
    }

    // Shutdown (EOF or dead stdout): stop accepting, abort every in-flight
    // call (aborted calls emit no response), give the writer up to 2s to
    // flush what is already queued, close the browser, exit 0. Bounded
    // throughout: never hangs, never panics.
    {
        let mut map = registry.lock().await;
        for (_, mut st) in map.drain() {
            if let Some(h) = st.handle.take() {
                h.abort();
            }
        }
    }
    drop(tx);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), writer_handle).await;
    let _ = browser.close().await;
    Ok(())
}

/// Best-effort send through the writer channel: `Err` when the writer is
/// gone, in which case the read loop must stop.
fn send_resp(tx: &mpsc::UnboundedSender<Value>, v: Value) -> Result<(), ()> {
    tx.send(v).map_err(|_| ())
}

/// `notifications/cancelled`: under the single registry lock, abort the
/// in-flight call whose canonical id matches `params.requestId`. Unknown,
/// completed, evicted, and already-emitted ids are no-ops. The cancelled
/// call sends no response: the flag suppresses even a task momentarily in
/// a non-preemptible section, and the lingering entry is bounded by
/// eviction.
async fn handle_cancel(req: &Value, registry: &Registry) {
    let Some(rid) = req.get("params").and_then(|p| p.get("requestId")) else {
        return;
    };
    let key = id_key(rid);
    let mut map = registry.lock().await;
    let Some(st) = map.get_mut(&key) else {
        return;
    };
    if st.emitted {
        return;
    }
    st.cancelled = true;
    if let Some(h) = st.handle.take() {
        h.abort();
    }
    evict_stale_locked(&mut map);
}

/// Route one `tools/call` request: inline validation errors go straight to
/// the writer; a duplicate in-flight id supersedes the older generation
/// first (idempotent with the read-loop supersede); then valid calls spawn
/// a task (or are refused `-32603` past the semaphore cap — the superseded
/// old call stays silent, so the busy error is the single in-flight
/// response for the id; completed ids remain free for sequential reuse).
#[allow(clippy::too_many_arguments)]
async fn dispatch_call(
    req: Value,
    browser: &Arc<Browser>,
    exec: &Arc<Mutex<()>>,
    sem: &Arc<Semaphore>,
    registry: &Registry,
    gen_ctr: &Arc<AtomicU64>,
    tx: &mpsc::UnboundedSender<Value>,
) {
    // JSON-RPC notifications (no `id`) get NO response, whatever the method:
    // a tools/call notification never executes.
    if !req
        .as_object()
        .map(|o| o.contains_key("id"))
        .unwrap_or(false)
    {
        return;
    }
    let raw_id = req.get("id").cloned().unwrap_or(Value::Null);
    if !(raw_id.is_string() || raw_id.is_number() || raw_id.is_null()) {
        let _ = send_resp(
            tx,
            json!({"jsonrpc": "2.0", "id": null,
                "error": {"code": -32600, "message": "invalid request"}}),
        );
        return;
    }
    let id = raw_id;
    let jsonrpc_ok = req.get("jsonrpc").and_then(|v| v.as_str()) == Some("2.0");
    if !jsonrpc_ok {
        let _ = send_resp(
            tx,
            json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32600, "message": "invalid request"}}),
        );
        return;
    }
    let requested = requested_version(&req);
    if let Some(ref v) = requested {
        if !is_supported(v) {
            let _ = send_resp(
                tx,
                json!({"jsonrpc": "2.0", "id": id,
                    "error": {"code": -32022, "message": "unsupported protocol version",
                        "data": {"supported": supported_versions(), "requested": v}}}),
            );
            return;
        }
    }
    let modern = requested.as_deref() == Some(MODERN);
    let params = req.get("params").cloned().unwrap_or(json!({}));
    let name = params
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();
    if name.is_empty() {
        let _ = send_resp(
            tx,
            json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32602, "message": "missing tool name"}}),
        );
        return;
    }
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let key = id_key(&id);
    let gen = gen_ctr.fetch_add(1, Ordering::Relaxed);
    // Duplicate-id supersession BEFORE the semaphore decision (idempotent
    // with the top-level supersede in the read loop, which already removed
    // any live entry for this id): an in-flight entry for this id is cancelled, aborted, and removed, so at most the
    // newest generation can ever emit for the id. Completed ids are free:
    // sequential reuse is normal JSON-RPC and always proceeds as a fresh
    // call. Its epilogue then sees a
    // missing/stale entry and stays silent. The lock is released before the
    // semaphore acquisition below: never held across it or tool execution.
    // (Invalid-id frames never reach dispatch: the read loop rejects them
    // -32600 before supersession. Valid-but-malformed duplicates — bad
    // method/tool/params — still supersede here, and their error is the
    // single in-flight response for the id.)
    {
        let mut map = registry.lock().await;
        if let Some(old) = map.get_mut(&key) {
            old.cancelled = true;
            if let Some(h) = old.handle.take() {
                h.abort();
            }
            map.remove(&key);
        }
    }
    let permit = match sem.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            // The superseded old call is already silent; the busy error is
            // the single response for this id.
            let _ = send_resp(
                tx,
                json!({"jsonrpc": "2.0", "id": id,
                    "error": {"code": -32603, "message": "Server busy"}}),
            );
            return;
        }
    };
    // Placeholder BEFORE spawn: the task can never finish before its entry
    // exists. No duplicate can be present here (superseded above, and
    // dispatch runs sequentially on the read loop); a present entry would
    // mean a task finished between the blocks, which is impossible without
    // an entry — defensively abort it rather than leak its handle.
    {
        let mut map = registry.lock().await;
        evict_stale_locked(&mut map);
        if let Some(old) = map.get_mut(&key) {
            old.cancelled = true;
            if let Some(h) = old.handle.take() {
                h.abort();
            }
        }
        map.insert(
            key.clone(),
            CallState {
                handle: None,
                gen,
                cancelled: false,
                emitted: false,
            },
        );
    }
    let browser = browser.clone();
    let exec = exec.clone();
    let tx_task = tx.clone();
    let registry_task = registry.clone();
    let lock_needed = needs_exec_lock(&name, &args);
    let key_task = key.clone();
    let handle: JoinHandle<()> = tokio::spawn(async move {
        // The permit releases when the task ends (normal, abort, panic).
        let _permit = permit;
        if !is_live(&registry_task, &key_task, gen).await {
            remove_if_mine(&registry_task, &key_task, gen).await;
            return;
        }
        // A tool panic is caught and converted to a `-32603` response for
        // the id (when not cancelled) instead of a silent entry leak.
        let outcome = std::panic::AssertUnwindSafe(async {
            if lock_needed {
                // A cancelled task waiting here aborts cleanly: the lock
                // future is dropped, the tool never runs, nothing is sent.
                let _guard = exec.lock().await;
                if !is_live(&registry_task, &key_task, gen).await {
                    return None;
                }
                Some(tools::call(&name, &args, &browser).await)
            } else {
                if !is_live(&registry_task, &key_task, gen).await {
                    return None;
                }
                Some(tools::call(&name, &args, &browser).await)
            }
        })
        .catch_unwind()
        .await;
        let resp_opt = match outcome {
            Ok(Some(mut result)) => {
                if modern {
                    if let Some(obj) = result.as_object_mut() {
                        obj.insert("resultType".to_string(), json!("complete"));
                    }
                }
                Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
            }
            Ok(None) => None, // superseded/cancelled while queued
            Err(_) => Some(panic_response(&id)),
        };
        if let Some(resp) = resp_opt {
            commit_emit(&registry_task, &key_task, gen, resp, &tx_task).await;
        } else {
            remove_if_mine(&registry_task, &key_task, gen).await;
        }
    });
    {
        let mut map = registry.lock().await;
        match map.get_mut(&key) {
            // Our placeholder still current: record the handle for cancel.
            Some(st) if st.gen == gen && !st.emitted => {
                st.handle = Some(handle);
            }
            // A fast task already finished (its epilogue committed and
            // removed the entry) or the entry was superseded/evicted: the
            // handle is done or will exit silently on its own; aborting is
            // a harmless no-op.
            _ => {
                handle.abort();
            }
        }
    }
}

async fn handle_fast(req: &Value, _browser: &Arc<Browser>) -> Option<Value> {
    // JSON-RPC notifications (no `id`) get NO response, whatever the method.
    let has_id = req
        .as_object()
        .map(|o| o.contains_key("id"))
        .unwrap_or(false);
    if !has_id {
        return None;
    }
    // JSON-RPC request IDs: string, number, or null only. Boolean, object,
    // and array IDs are Invalid Request: -32600 with id null (never echo
    // the invalid value, never execute the method).
    let raw_id = req.get("id").cloned().unwrap_or(Value::Null);
    if !(raw_id.is_string() || raw_id.is_number() || raw_id.is_null()) {
        return Some(json!({"jsonrpc": "2.0", "id": null,
            "error": {"code": -32600, "message": "invalid request"}}));
    }
    let id = raw_id;
    // Strict JSON-RPC: require jsonrpc=="2.0" and a (non-empty) string method.
    let jsonrpc_ok = req.get("jsonrpc").and_then(|v| v.as_str()) == Some("2.0");
    let method = match req.get("method").and_then(|m| m.as_str()) {
        Some(m) if !m.is_empty() => m.to_string(),
        _ => {
            return Some(json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32600, "message": "invalid request"}}));
        }
    };
    if !jsonrpc_ok {
        return Some(json!({"jsonrpc": "2.0", "id": id,
            "error": {"code": -32600, "message": "invalid request"}}));
    }
    // Per-request modern detection.
    let requested = requested_version(req);
    if let Some(ref v) = requested {
        if !is_supported(v) {
            return Some(json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32022, "message": "unsupported protocol version",
                    "data": {"supported": supported_versions(), "requested": v}}}));
        }
    }
    let modern = requested.as_deref() == Some(MODERN);
    match method.as_str() {
        "initialize" => {
            let client_ver = req
                .get("params")
                .and_then(|p| p.get("protocolVersion"))
                .and_then(|v| v.as_str());
            let ver = negotiate(client_ver);
            Some(json!({"jsonrpc": "2.0", "id": id, "result": {
                "protocolVersion": ver,
                "capabilities": capabilities(),
                "serverInfo": server_info(),
                "instructions": instructions()
            }}))
        }
        "server/discover" => Some(json!({"jsonrpc": "2.0", "id": id, "result": {
            "supportedVersions": supported_versions(),
            "capabilities": capabilities(),
            "serverInfo": server_info(),
            "instructions": instructions(),
            "resultType": "complete"
        }})),
        "notifications/initialized" => None, // notification: no reply
        "notifications/cancelled" => None,   // handled in the read loop; never a reply
        "ping" => Some(json!({"jsonrpc": "2.0", "id": id, "result": {}})),
        "tools/list" => {
            let mut result = crate::tools::list();
            if modern {
                if let Some(obj) = result.as_object_mut() {
                    obj.insert("resultType".to_string(), json!("complete"));
                }
            }
            Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
        }
        "tools/call" => Some(json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": "method not found"}})),
        _ => Some(json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": "method not found"}})),
    }
}

#[cfg(test)]
mod concurrency_tests {
    use super::*;

    fn test_registry() -> Registry {
        Arc::new(Mutex::new(HashMap::new()))
    }

    fn live_state(gen: u64) -> CallState {
        CallState {
            handle: None,
            gen,
            cancelled: false,
            emitted: false,
        }
    }

    #[tokio::test]
    async fn commit_sends_once_for_live_call() {
        let reg = test_registry();
        let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
        reg.lock().await.insert("1".to_string(), live_state(9));
        let resp = json!({"jsonrpc": "2.0", "id": 1, "result": {}});
        assert!(commit_emit(&reg, "1", 9, resp.clone(), &tx).await);
        assert_eq!(rx.recv().await, Some(resp));
        assert!(!reg.lock().await.contains_key("1"));
    }

    #[tokio::test]
    async fn commit_silent_when_cancelled() {
        let reg = test_registry();
        let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
        reg.lock().await.insert(
            "1".to_string(),
            CallState {
                handle: None,
                gen: 9,
                cancelled: true,
                emitted: false,
            },
        );
        let resp = json!({"jsonrpc": "2.0", "id": 1, "result": {}});
        assert!(!commit_emit(&reg, "1", 9, resp, &tx).await);
        assert!(rx.try_recv().is_err(), "cancelled call sends nothing");
        assert!(
            !reg.lock().await.contains_key("1"),
            "cancelled entry cleaned up"
        );
    }

    #[tokio::test]
    async fn commit_silent_on_stale_generation() {
        let reg = test_registry();
        let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
        // Newest generation owns the entry; the old task must not touch it.
        reg.lock().await.insert("1".to_string(), live_state(2));
        let resp = json!({"jsonrpc": "2.0", "id": 1, "result": {}});
        assert!(!commit_emit(&reg, "1", 1, resp, &tx).await);
        assert!(rx.try_recv().is_err(), "stale generation sends nothing");
        assert!(
            reg.lock().await.contains_key("1"),
            "newest entry left for its own task"
        );
    }

    #[test]
    fn panic_response_shape() {
        let r = panic_response(&json!(42));
        assert_eq!(r.get("id"), Some(&json!(42)), "id echoed: {r}");
        assert_eq!(
            r.get("error").and_then(|e| e.get("code")),
            Some(&json!(-32603)),
            "panic maps to -32603: {r}"
        );
    }

    #[test]
    fn evict_bounds_tombstones_keeps_live() {
        let mut map = HashMap::new();
        for i in 0..1100 {
            map.insert(
                format!("c{i}"),
                CallState {
                    handle: None,
                    gen: i as u64,
                    cancelled: true,
                    emitted: false,
                },
            );
        }
        map.insert("live".to_string(), live_state(9999));
        evict_stale_locked(&mut map);
        assert!(
            map.len() <= MAX_REGISTRY_ENTRIES,
            "tombstones bounded: {}",
            map.len()
        );
        assert!(map.contains_key("live"), "live entry kept");
        // Under the bound nothing is touched.
        evict_stale_locked(&mut map);
        assert!(map.contains_key("live"), "no-op under bound");
    }
}
