use std::collections::{HashMap, HashSet, VecDeque};
use std::process::Stdio;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex as AsyncMutex, oneshot};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
type WsSink = futures_util::stream::SplitSink<WsStream, Message>;
type WsReader = futures_util::stream::SplitStream<WsStream>;

/// A JS dialog the page opened (`alert` / `confirm` / `prompt` /
/// `beforeunload`), recorded when the reader auto-handles it.
#[derive(Clone, Debug)]
pub struct DialogRecord {
    pub dtype: String,
    pub message: String,
    pub url: String,
}

/// A buffered console message (`console.*` call or uncaught exception),
/// shaped `{type, text, url, line, ts}` for the `console` tool. Kept in a
/// bounded ring (`CONSOLE_CAP`); the reader pushes, tools drain.
#[derive(Clone, Debug)]
pub struct ConsoleMsg {
    pub mtype: String,
    pub text: String,
    pub url: String,
    pub line: i64,
    pub ts: u64,
}

impl ConsoleMsg {
    fn json(&self) -> Value {
        json!({"type": self.mtype, "text": self.text, "url": self.url,
               "line": self.line, "ts": self.ts})
    }
}

/// Ring cap for buffered console messages (stated in the `console` tool
/// description so agents can reason about depth).
pub const CONSOLE_CAP: usize = 200;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

struct Shared {
    sink: AsyncMutex<WsSink>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    next_id: AtomicU64,
    session: AsyncMutex<Option<String>>,
    target: AsyncMutex<Option<String>>,
    /// Latest auto-handled dialog (drained by each acting tool before it
    /// runs, so a record found afterwards belongs to that action).
    dialog: AsyncMutex<Option<DialogRecord>>,
    /// Load-event plumbing for event-driven waits: bumped + notified on
    /// every `Page.loadEventFired` the reader sees (any session). Waiters
    /// use it as the wake signal with a fine-cadence `readyState` poll as
    /// fallback, so a wait never depends on event delivery alone.
    load_notify: tokio::sync::Notify,
    /// Stable tab index space: targetIds in first-seen (creation) order.
    /// CDP's getTargets order flaps under load (measured: a fresh tab sorted
    /// first on one call, second on the next), so indices must NOT follow
    /// raw CDP order - the opener stays 0, new tabs append.
    tab_order: AsyncMutex<Vec<String>>,
    /// Buffered console messages (bounded ring, `CONSOLE_CAP`). Pushed by
    /// the reader on `Runtime.consoleAPICalled` / `Runtime.exceptionThrown`
    /// / `Log.entryAdded`; drained by the `console` tool. Empty (and free)
    /// when the page is quiet.
    console: AsyncMutex<VecDeque<ConsoleMsg>>,
    /// In-flight CDP network request ids for the `networkIdle` wait. Fed by
    /// the reader on `Network.requestWillBeSent` / `loadingFinished` /
    /// `loadingFailed`; empty (and free) when nothing loads. The Network
    /// domain is enabled LAZILY on the first network-idle wait, so pages
    /// that never ask pay nothing.
    net_inflight: AsyncMutex<HashSet<String>>,
    net_notify: tokio::sync::Notify,
    net_enabled: AsyncMutex<bool>,
}

struct Inner {
    _profile: ProfileDir,
    #[cfg(windows)]
    _job: Option<std::os::windows::io::OwnedHandle>,
    child: tokio::process::Child,
    pid: u32,
    shared: Arc<Shared>,
    reader: tokio::task::JoinHandle<()>,
}

pub struct Browser {
    state: AsyncMutex<Option<Inner>>,
    pid: Mutex<Option<u32>>,
    refs: AsyncMutex<HashMap<String, String>>,
    snap_url: AsyncMutex<String>,
}

impl Browser {
    pub fn new() -> Self {
        Browser {
            state: AsyncMutex::new(None),
            pid: Mutex::new(None),
            refs: AsyncMutex::new(HashMap::new()),
            snap_url: AsyncMutex::new(String::new()),
        }
    }

    /// Store the ref table from the latest AX snapshot (ref `eN` -> label).
    pub async fn store_refs(&self, url: String, refs: HashMap<String, String>) {
        *self.refs.lock().await = refs;
        *self.snap_url.lock().await = url;
    }

    /// Label for a known ref id (`eN`), or None when the ref was never issued.
    pub async fn ref_label(&self, id: &str) -> Option<String> {
        self.refs.lock().await.get(id).cloned()
    }

    /// URL captured at the last AX snapshot (empty when none taken yet).
    pub async fn snap_url(&self) -> String {
        self.snap_url.lock().await.clone()
    }

    /// Drain the latest auto-handled dialog record (if any). Acting tools
    /// call this BEFORE running so a record found afterwards belongs to
    /// their own action.
    pub async fn take_dialog(&self) -> Option<DialogRecord> {
        // No browser yet means no dialog could have been seen.
        let guard = self.state.lock().await;
        let inner = match guard.as_ref() {
            Some(inner) => inner,
            None => return None,
        };
        let shared = inner.shared.clone();
        drop(guard);
        let v = shared.dialog.lock().await.take();
        v
    }

    /// Peek at the latest auto-handled dialog record without draining it.
    pub async fn last_dialog(&self) -> Option<DialogRecord> {
        let guard = self.state.lock().await;
        let inner = match guard.as_ref() {
            Some(inner) => inner,
            None => return None,
        };
        let shared = inner.shared.clone();
        drop(guard);
        let v = shared.dialog.lock().await.clone();
        v
    }

    async fn shared(&self) -> Result<Arc<Shared>, String> {
        {
            let guard = self.state.lock().await;
            if let Some(inner) = guard.as_ref() {
                return Ok(inner.shared.clone());
            }
        }
        let inner = launch().await?;
        let shared = inner.shared.clone();
        *self.pid.lock().unwrap() = Some(inner.pid);
        *self.state.lock().await = Some(inner);
        Ok(shared)
    }

    /// Send a raw CDP command. If `attach` is true, ensure a page session exists
    /// and send with the `sessionId` field.
    pub async fn command(
        &self,
        method: &str,
        params: Value,
        attach: bool,
    ) -> Result<Value, String> {
        let shared = self.shared().await?;
        let session = if attach {
            Some(ensure_session(&shared).await?)
        } else {
            None
        };
        send_raw(&shared, method, params, session).await
    }

    /// Runtime.evaluate helper returning the value (or a JS error string).
    ///
    /// A CDP result with NO `value` key (e.g. JavaScript `undefined`) is an
    /// error, keeping it distinguishable from an explicit `null` value.
    pub async fn eval(&self, expression: &str) -> Result<Value, String> {
        let r = self
            .command(
                "Runtime.evaluate",
                json!({"expression": expression, "returnByValue": true, "awaitPromise": true}),
                true,
            )
            .await?;
        if let Some(ex) = r.get("exceptionDetails") {
            return Err(format!("js exception: {ex}"));
        }
        let robj = r
            .get("result")
            .ok_or("evaluate: no result in response".to_string())?;
        match robj.get("value") {
            Some(v) => Ok(v.clone()), // includes an explicit null
            None => {
                let t = robj
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                Err(format!("evaluate: expression returned {t} (no value)"))
            }
        }
    }

    pub async fn tabs_list(&self) -> Result<Value, String> {
        let shared = self.shared().await?;
        tabs_object(&shared).await
    }

    pub async fn tabs_new(&self, url: Option<&str>) -> Result<Value, String> {
        let shared = self.shared().await?;
        let open_url = url.filter(|u| !u.trim().is_empty()).unwrap_or("about:blank");
        let created = send_raw(
            &shared,
            "Target.createTarget",
            json!({"url": open_url}),
            None,
        )
        .await?;
        let tid = created
            .get("targetId")
            .and_then(|v| v.as_str())
            .ok_or("tabs: createTarget returned no targetId".to_string())?
            .to_string();
        let _ = send_raw(
            &shared,
            "Target.activateTarget",
            json!({"targetId": tid}),
            None,
        )
        .await;
        attach_to(&shared, &tid).await?;
        // Settle: the new target's record carries the destination URL long
        // before its navigation commits (measured live: target record showed
        // /windows/new while the page still evaluated to about:blank). Wait
        // (bounded) for the commit so the report is truthful and the next
        // tool observes the real page.
        settle_new_tab(&shared, open_url != "about:blank").await;
        tabs_object(&shared).await
    }

    pub async fn tabs_select(&self, index: usize) -> Result<Value, String> {
        let shared = self.shared().await?;
        // Ensure an active page exists so the index space is well-defined.
        ensure_session(&shared).await?;
        // A click that opens a tab returns before Chrome lists the new
        // target (measured: system Chrome still showed 1 target ~120ms
        // after click_settle; headless-shell already showed 2). Bounded
        // wait for the index to appear so select is deterministic.
        let deadline = Instant::now() + Duration::from_secs(10);
        let infos = loop {
            let infos = ordered_targets(&shared).await?;
            if infos.len() > index {
                break infos;
            }
            if Instant::now() >= deadline {
                return Err(format!("tabs: no tab at index {index}"));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        let t = infos
            .get(index)
            .ok_or(format!("tabs: no tab at index {index}"))?;
        let tid = t
            .get("targetId")
            .and_then(|v| v.as_str())
            .ok_or("tabs: target has no targetId".to_string())?
            .to_string();
        // A non-blank target record means a real page is (or should be)
        // loading there: settle until its navigation commits. A blank
        // record (fresh about:blank tab) settles on first paint instead.
        let record_url = t
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let already_active = shared.target.lock().await.clone() == Some(tid.clone());
        let _ = send_raw(
            &shared,
            "Target.activateTarget",
            json!({"targetId": tid}),
            None,
        )
        .await?;
        if !already_active {
            attach_to(&shared, &tid).await?;
        }
        // Settle: a freshly opened tab may still show url "" / about:blank
        // here (measured: tab-02's select saw url:"" while tab-01's list saw
        // the committed URL). Bounded wait for commit + load so the
        // following evaluate observes the real page.
        let expect_page = !record_url.is_empty() && record_url != "about:blank";
        settle_new_tab(&shared, expect_page).await;
        tabs_object(&shared).await
    }

    pub async fn tabs_close(&self, index: usize) -> Result<Value, String> {
        let shared = self.shared().await?;
        ensure_session(&shared).await?;
        let infos = ordered_targets(&shared).await?;
        let t = infos
            .get(index)
            .ok_or(format!("tabs: no tab at index {index}"))?;
        let tid = t
            .get("targetId")
            .and_then(|v| v.as_str())
            .ok_or("tabs: target has no targetId".to_string())?
            .to_string();
        let was_active = shared.target.lock().await.clone() == Some(tid.clone());
        send_raw(
            &shared,
            "Target.closeTarget",
            json!({"targetId": tid}),
            None,
        )
        .await?;
        // Settle: closeTarget acknowledges before the target detaches, so an
        // immediate getTargets can still list it (measured under parallel
        // load). Bounded wait until it disappears from the target list.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match page_targets(&shared).await {
                Ok(infos)
                    if infos.iter().any(|t| {
                        t.get("targetId").and_then(|v| v.as_str()) == Some(tid.as_str())
                    }) =>
                {
                    if Instant::now() >= deadline {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                _ => break,
            }
        }
        if was_active {
            *shared.session.lock().await = None;
            *shared.target.lock().await = None;
            // Track exactly one survivor so the other tools keep working.
            let rest = ordered_targets(&shared).await?;
            if let Some(first) = rest.first() {
                if let Some(fid) = first.get("targetId").and_then(|v| v.as_str()) {
                    let fid = fid.to_string();
                    let _ = send_raw(
                        &shared,
                        "Target.activateTarget",
                        json!({"targetId": fid}),
                        None,
                    )
                    .await;
                    // A closing target can transiently fail attach; keep the
                    // list truthful either way (next command re-attaches).
                    let _ = attach_to(&shared, &fid).await;
                }
            }
        }
        // Report without forcing a re-attach: an empty farm has no active.
        let active = shared.target.lock().await.clone();
        let infos = ordered_targets(&shared).await?;
        let mut tabs = Vec::new();
        for (i, t) in infos.iter().enumerate() {
            let id = t.get("targetId").and_then(|v| v.as_str()).unwrap_or("");
            tabs.push(json!({
                "index": i,
                "url": t.get("url").and_then(|v| v.as_str()).unwrap_or(""),
                "title": t.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                "active": Some(id.to_string()) == active,
            }));
        }
        Ok(json!({"tabs": tabs, "ok": true}))
    }

    pub async fn close(&self) -> Result<(), String> {
        let inner = self.state.lock().await.take();
        if let Some(inner) = inner {
            let pid = inner.pid;
            inner.reader.abort();
            let mut child = inner.child;
            // Fast path: terminate the direct child and wait for it. The
            // KILL_ON_JOB_CLOSE job handle (`_job`, dropped below with the
            // rest of `inner`) reaps surviving grandchildren, so the old
            // taskkill spawn (~220 ms measured) is fallback-only.
            let _ = child.kill().await;
            let exited =
                tokio::time::timeout(Duration::from_secs(2), child.wait())
                    .await
                    .is_ok();
            if !exited {
                kill_tree(pid);
                let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
            }
        }
        *self.pid.lock().unwrap() = None;
        Ok(())
    }

    /// Bounded wait for the active page to report `readyState == complete`,
    /// woken by the reader on `Page.loadEventFired` with a 10 ms
    /// `readyState` poll as fallback (same 10 s-class caps callers used
    /// before). Returns immediately when the page is already complete or
    /// no browser exists. Never fails: a timeout just returns.
    pub async fn wait_for_load(&self, cap: Duration) {
        let shared = {
            let guard = self.state.lock().await;
            match guard.as_ref() {
                Some(inner) => inner.shared.clone(),
                None => return,
            }
        };
        let deadline = Instant::now() + cap;
        // Subscribe before the first check so an event racing the poll
        // still wakes this waiter.
        tokio::pin!(let notified = shared.load_notify.notified(););
        loop {
            // Clip each poll eval to the REMAINING budget (the same wrap the
            // `wait_for` tool already uses): the deadline is checked between
            // evals, but an unwrapped `send_raw` allows 30s per call, so a
            // stalled peer would turn a short cap into 30s+.
            let remaining = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(remaining, self.eval("document.readyState")).await
            {
                Ok(Ok(Value::String(s))) if s == "complete" => return,
                _ => {}
            }
            if Instant::now() >= deadline {
                return;
            }
            let remain = deadline.saturating_duration_since(Instant::now());
            tokio::select! {
                _ = &mut notified => {
                    notified.set(shared.load_notify.notified());
                }
                _ = tokio::time::sleep(remain.min(Duration::from_millis(10))) => {}
            }
        }
    }

    /// Newest-first? No - oldest-first slice of the console ring: the last
    /// `limit` messages in arrival order, plus `count`/`total`/`cap` so an
    /// agent can tell a quiet page (`total: 0`) from a truncated one.
    pub async fn console_get(&self, limit: usize) -> Value {
        let msgs: Vec<ConsoleMsg> = {
            let guard = self.state.lock().await;
            match guard.as_ref() {
                Some(inner) => inner.shared.console.lock().await.iter().cloned().collect(),
                None => Vec::new(),
            }
        };
        let total = msgs.len();
        let take = limit.min(total);
        let slice = &msgs[total - take..];
        json!({"messages": slice.iter().map(|m| m.json()).collect::<Vec<_>>(),
               "count": take, "total": total, "cap": CONSOLE_CAP})
    }

    /// Drain the console ring, returning the number of messages dropped.
    pub async fn console_clear(&self) -> usize {
        let guard = self.state.lock().await;
        match guard.as_ref() {
            Some(inner) => inner.shared.console.lock().await.drain(..).count(),
            None => 0,
        }
    }

    /// Atomic take-and-return for `console drain`: the slice AND the clear
    /// happen under ONE `console` lock acquisition. The old shape (clone
    /// under one acquisition in `console_get`, drain under a second in
    /// `console_clear`) let the reader push between the two awaits; those
    /// arrivals were cleared but never returned. Same envelope as
    /// get-then-clear (`messages/count/total/cap` plus `cleared`).
    pub async fn console_drain(&self, limit: usize) -> Value {
        let guard = self.state.lock().await;
        match guard.as_ref() {
            Some(inner) => {
                let mut q = inner.shared.console.lock().await;
                let total = q.len();
                let take = limit.min(total);
                let messages: Vec<Value> =
                    q.iter().skip(total - take).map(|m| m.json()).collect();
                let cleared = q.drain(..).count();
                json!({"messages": messages, "count": take,
                       "total": total, "cap": CONSOLE_CAP, "cleared": cleared})
            }
            None => json!({"messages": [], "count": 0,
                           "total": 0, "cap": CONSOLE_CAP, "cleared": 0}),
        }
    }

    /// Enable the CDP Network domain once (lazy: the first network-idle
    /// wait pays one round-trip; every other op pays nothing).
    pub async fn ensure_network(&self) -> Result<(), String> {
        let shared = self.shared().await?;
        if *shared.net_enabled.lock().await {
            return Ok(());
        }
        let session = ensure_session(&shared).await?;
        send_raw(&shared, "Network.enable", json!({}), Some(session)).await?;
        *shared.net_enabled.lock().await = true;
        Ok(())
    }

    /// Bounded wait for a quiet-network window: returns true once the
    /// in-flight request set stays empty for `idle`, false on `budget`
    /// expiry (the caller reports its typed timeout - this never hangs,
    /// even when the page never goes idle). No browser yet is trivially
    /// idle.
    pub async fn wait_network_idle(&self, idle: Duration, budget: Duration) -> bool {
        let shared = {
            let guard = self.state.lock().await;
            match guard.as_ref() {
                Some(inner) => inner.shared.clone(),
                None => return true,
            }
        };
        let deadline = Instant::now() + budget;
        let mut quiet_start: Option<Instant> = None;
        // Subscribe before the first check so an event racing the poll
        // still wakes this waiter.
        tokio::pin!(let notified = shared.net_notify.notified(););
        loop {
            if shared.net_inflight.lock().await.is_empty() {
                let qs = *quiet_start.get_or_insert_with(Instant::now);
                if qs.elapsed() >= idle {
                    return true;
                }
            } else {
                quiet_start = None;
            }
            if Instant::now() >= deadline {
                return false;
            }
            let remain = deadline.saturating_duration_since(Instant::now());
            tokio::select! {
                _ = &mut notified => {
                    notified.set(shared.net_notify.notified());
                }
                _ = tokio::time::sleep(remain.min(Duration::from_millis(50))) => {}
            }
        }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        if let Some(pid) = self.pid.lock().unwrap().take() {
            kill_tree(pid);
        }
    }
}

async fn send_raw(
    shared: &Arc<Shared>,
    method: &str,
    params: Value,
    session: Option<String>,
) -> Result<Value, String> {
    send_raw_with_timeout(shared, method, params, session, Duration::from_secs(30)).await
}

/// `send_raw` with an injectable response timeout (production passes 30s;
/// tests pass a short budget against the black-hole rig).
async fn send_raw_with_timeout(
    shared: &Arc<Shared>,
    method: &str,
    params: Value,
    session: Option<String>,
    timeout: Duration,
) -> Result<Value, String> {
    let id = shared.next_id.fetch_add(1, Ordering::SeqCst);
    let mut envelope = serde_json::Map::new();
    envelope.insert("id".to_string(), json!(id));
    envelope.insert("method".to_string(), json!(method));
    envelope.insert("params".to_string(), params);
    if let Some(s) = session {
        envelope.insert("sessionId".to_string(), json!(s));
    }
    let text = Value::Object(envelope).to_string();
    let (tx, rx) = oneshot::channel();
    shared.pending.lock().unwrap().insert(id, tx);
    {
        let mut sink = shared.sink.lock().await;
        if let Err(e) = sink.send(Message::Text(text.into())).await {
            shared.pending.lock().unwrap().remove(&id);
            return Err(format!("ws send: {e}"));
        }
    }
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(resp)) => {
            if let Some(err) = resp.get("error") {
                return Err(format!("cdp error: {err}"));
            }
            Ok(resp.get("result").cloned().unwrap_or(Value::Null))
        }
        Ok(Err(_)) => Err("cdp: response channel closed".to_string()),
        Err(_) => {
            // Timeout path: drop the pending entry, mirroring the send-error
            // path above. Without this, one `tx` leaks per unanswered call
            // (a late response is the only other removal, and a dead peer
            // never sends one).
            shared.pending.lock().unwrap().remove(&id);
            Err("cdp: response timeout".to_string())
        }
    }
}

async fn ensure_session(shared: &Arc<Shared>) -> Result<String, String> {
    if let Some(s) = shared.session.lock().await.clone() {
        return Ok(s);
    }
    // Oldest tab first, so a fresh session lands on the opener.
    // (ordered_targets already filters to page targets.)
    let infos = ordered_targets(shared).await?;
    let mut target_id: Option<String> = infos
        .first()
        .and_then(|t| t.get("targetId"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    if target_id.is_none() {
        let created = send_raw(shared, "Target.createTarget", json!({"url": "about:blank"}), None)
            .await?;
        target_id = created
            .get("targetId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
    }
    let tid = target_id.ok_or("cdp: no page target available".to_string())?;
    let sid = attach_to(shared, &tid).await?;
    Ok(sid)
}

/// Attach to a page target, tracking it as the single active page for all
/// session-scoped (`attach: true`) commands. Returns the new session id.
async fn attach_to(shared: &Arc<Shared>, target_id: &str) -> Result<String, String> {
    let attached = send_raw(
        shared,
        "Target.attachToTarget",
        json!({"targetId": target_id, "flatten": true}),
        None,
    )
    .await?;
    let sid = attached
        .get("sessionId")
        .and_then(|v| v.as_str())
        .ok_or("cdp: attach returned no sessionId".to_string())?
        .to_string();
    *shared.session.lock().await = Some(sid.clone());
    *shared.target.lock().await = Some(target_id.to_string());
    // Enable page events on every new session: this is what delivers
    // Page.javascriptDialogOpening to the reader for auto-handling.
    let _ = send_raw(shared, "Page.enable", json!({}), Some(sid.clone())).await;
    // Runtime events feed the console ring (consoleAPICalled +
    // exceptionThrown); Log events add failed-resource lines. One extra
    // round-trip per attach; when the page is quiet the reader work is
    // two string compares per event, i.e. no steady-state cost.
    let _ = send_raw(shared, "Runtime.enable", json!({}), Some(sid.clone())).await;
    let _ = send_raw(shared, "Log.enable", json!({}), Some(sid.clone())).await;
    Ok(sid)
}

/// Page targets excluding devtools internals, in STABLE creation order
/// (first-seen targetId order, not raw CDP order). The position in this
/// list is the `index` used by the `tabs` tool: the opener stays 0 and
/// freshly opened tabs append, no matter how CDP shuffles its own list.
async fn ordered_targets(shared: &Arc<Shared>) -> Result<Vec<Value>, String> {
    let infos = page_targets(shared).await?;
    let mut order = shared.tab_order.lock().await;
    {
        let present: std::collections::HashSet<&str> = infos
            .iter()
            .filter_map(|t| t.get("targetId").and_then(|v| v.as_str()))
            .collect();
        order.retain(|id| present.contains(id.as_str()));
        for t in &infos {
            if let Some(id) = t.get("targetId").and_then(|v| v.as_str()) {
                if !order.iter().any(|x| x == id) {
                    order.push(id.to_string());
                }
            }
        }
    }
    let rank = |t: &Value| {
        let id = t.get("targetId").and_then(|v| v.as_str()).unwrap_or("");
        order.iter().position(|x| x == id).unwrap_or(usize::MAX)
    };
    let mut sorted = infos;
    sorted.sort_by_key(rank);
    Ok(sorted)
}

/// Page targets excluding devtools internals, in raw CDP order (unstable -
/// use `ordered_targets` for anything index-facing).
async fn page_targets(shared: &Arc<Shared>) -> Result<Vec<Value>, String> {
    let targets = send_raw(shared, "Target.getTargets", json!({}), None).await?;
    let infos = targets
        .get("targetInfos")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(infos
        .into_iter()
        .filter(|t| {
            t.get("type").and_then(|v| v.as_str()) == Some("page")
                && !t
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .starts_with("devtools://")
        })
        .collect())
}

async fn tabs_object(shared: &Arc<Shared>) -> Result<Value, String> {
    // Ensure exactly one active page is tracked before reporting.
    ensure_session(shared).await?;
    let active = shared.target.lock().await.clone();
    let infos = ordered_targets(shared).await?;
    // Live truth for the active tab: Target.getTargets info flaps for fresh
    // tabs (measured: url "" vs the committed URL across calls), so ask the
    // page itself. Falls back to target info when the page will not answer.
    let live = live_page_info(shared).await;
    let mut tabs = Vec::new();
    for (i, t) in infos.iter().enumerate() {
        let tid = t.get("targetId").and_then(|v| v.as_str()).unwrap_or("");
        let is_active = Some(tid.to_string()) == active;
        let (url, title) = if is_active {
            if let Some((u, ti)) = live.clone() {
                (u, ti)
            } else {
                (
                    t.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    t.get("title").and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                )
            }
        } else {
            (
                t.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                t.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            )
        };
        tabs.push(json!({
            "index": i,
            "url": url,
            "title": title,
            "active": is_active,
        }));
    }
    Ok(json!({"tabs": tabs, "ok": true}))
}

/// Best-effort live `(url, title)` from the active page session. Returns
/// None when the page will not answer (caller falls back to target info).
async fn live_page_info(shared: &Arc<Shared>) -> Option<(String, String)> {
    let session = shared.session.lock().await.clone()?;
    let eval = |expr: &str| {
        let shared = shared.clone();
        let session = session.clone();
        let expr = expr.to_string();
        async move {
            let r = tokio::time::timeout(
                Duration::from_secs(5),
                send_raw(
                    &shared,
                    "Runtime.evaluate",
                    json!({"expression": expr, "returnByValue": true}),
                    Some(session),
                ),
            )
            .await
            .ok()?
            .ok()?;
            if r.get("exceptionDetails").is_some() {
                return None;
            }
            r.get("result")
                .and_then(|o| o.get("value"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        }
    };
    let url = eval("window.location.href").await?;
    if url.is_empty() {
        return None;
    }
    let title = eval("document.title").await.unwrap_or_default();
    Some((url, title))
}

/// Bounded settle for the active page after a tab switch/creation.
/// When `expect_page` is true (the target record names a real URL), waits
/// until the live page reports a committed, non-blank URL (20s cap): a
/// fresh tab evaluates to "" / about:blank until its navigation commits,
/// and returning earlier strands the next tool on the blank page. When
/// false (a legitimate about:blank tab), returns on first non-empty href.
async fn settle_new_tab(shared: &Arc<Shared>, expect_page: bool) {
    let session = match shared.session.lock().await.clone() {
        Some(s) => s,
        None => return,
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let ask = |expr: &str| {
            let shared = shared.clone();
            let session = session.clone();
            let expr = expr.to_string();
            async move {
                let r = tokio::time::timeout(
                    Duration::from_secs(5),
                    send_raw(
                        &shared,
                        "Runtime.evaluate",
                        json!({"expression": expr, "returnByValue": true}),
                        Some(session),
                    ),
                )
                .await
                .ok()?
                .ok()?;
                if r.get("exceptionDetails").is_some() {
                    return None;
                }
                r.get("result")
                    .and_then(|o| o.get("value"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            }
        };
        let href = ask("window.location.href").await.unwrap_or_default();
        if !href.is_empty() && href != "about:blank" {
            break;
        }
        if !expect_page && !href.is_empty() {
            break;
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Push one message onto the console ring, evicting the oldest when full.
async fn push_console(shared: &Arc<Shared>, msg: ConsoleMsg) {
    let mut q = shared.console.lock().await;
    if q.len() >= CONSOLE_CAP {
        q.pop_front();
    }
    q.push_back(msg);
}

/// A CDP RemoteObject rendered as the agent-visible string: real values
/// verbatim, otherwise the description (e.g. `Error: ...`), otherwise the
/// unserializable marker (`-Infinity`, `bigint`, ...), otherwise the type.
fn remote_to_string(o: &Value) -> String {
    if let Some(v) = o.get("value") {
        if let Some(s) = v.as_str() {
            return s.to_string();
        }
        return v.to_string();
    }
    if let Some(s) = o.get("unserializableValue").and_then(|x| x.as_str()) {
        return s.to_string();
    }
    if let Some(s) = o.get("description").and_then(|x| x.as_str()) {
        return s.to_string();
    }
    o.get("type")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown")
        .to_string()
}

/// `Runtime.consoleAPICalled` params -> ConsoleMsg.
fn console_message(params: &Value) -> ConsoleMsg {
    let mtype = params
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("log")
        .to_string();
    let args = params
        .get("args")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();
    let text = args
        .iter()
        .map(remote_to_string)
        .collect::<Vec<_>>()
        .join(" ");
    let frame = params
        .get("stackTrace")
        .and_then(|s| s.get("callFrames"))
        .and_then(|f| f.get(0))
        .cloned()
        .unwrap_or(Value::Null);
    ConsoleMsg {
        mtype,
        text,
        url: frame
            .get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string(),
        line: frame.get("lineNumber").and_then(|l| l.as_i64()).unwrap_or(-1),
        ts: params
            .get("timestamp")
            .and_then(|t| t.as_f64())
            .map(|t| t as u64)
            .unwrap_or_else(now_ms),
    }
}

/// `Runtime.exceptionThrown` params -> ConsoleMsg (type `error`).
fn exc_message(params: &Value) -> ConsoleMsg {
    let d = params.get("exceptionDetails").cloned().unwrap_or(Value::Null);
    let mut text = d
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or("uncaught exception")
        .to_string();
    if let Some(desc) = d
        .get("exception")
        .and_then(|e| e.get("description"))
        .and_then(|x| x.as_str())
    {
        text.push_str(": ");
        text.push_str(desc);
    }
    ConsoleMsg {
        mtype: "error".to_string(),
        text,
        url: d.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string(),
        line: d.get("lineNumber").and_then(|l| l.as_i64()).unwrap_or(-1),
        ts: now_ms(),
    }
}

/// `Log.entryAdded` entry -> ConsoleMsg (failed resources, violations...).
fn log_message(entry: &Value) -> ConsoleMsg {
    ConsoleMsg {
        mtype: entry
            .get("level")
            .and_then(|l| l.as_str())
            .unwrap_or("error")
            .to_string(),
        text: entry
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
        url: entry.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string(),
        line: entry.get("lineNumber").and_then(|l| l.as_i64()).unwrap_or(-1),
        ts: entry
            .get("timestamp")
            .and_then(|t| t.as_f64())
            .map(|t| t as u64)
            .unwrap_or_else(now_ms),
    }
}

async fn reader_loop(mut stream: WsReader, shared: Arc<Shared>) {
    while let Some(msg) = stream.next().await {
        let text = match msg {
            Ok(Message::Text(t)) => t.to_string(),
            Ok(Message::Binary(b)) => String::from_utf8_lossy(&b).into_owned(),
            _ => continue,
        };
        let v: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Route responses by id; events (no id) fall through below.
        if let Some(id) = v.get("id").and_then(|i| i.as_u64()) {
            if let Some(tx) = shared.pending.lock().unwrap().remove(&id) {
                let _ = tx.send(v);
            }
            continue;
        }
        // Event-driven wait signal: wake any `wait_for_load` waiter. The
        // `readyState` poll there is the fallback, so a missed or
        // session-scoped event can only cost one 10 ms cadence, never a
        // hang. Dialog handling below is unchanged.
        if v.get("method").and_then(|m| m.as_str()) == Some("Page.loadEventFired") {
            shared.load_notify.notify_waiters();
            continue;
        }
        // Console ring: buffer console.* calls, uncaught exceptions, and
        // failed-resource log lines for the `console` tool. Bounded
        // (`CONSOLE_CAP`); a quiet page costs two string compares here.
        if v.get("method").and_then(|m| m.as_str()) == Some("Runtime.consoleAPICalled") {
            let params = v.get("params").cloned().unwrap_or(Value::Null);
            push_console(&shared, console_message(&params)).await;
            continue;
        }
        if v.get("method").and_then(|m| m.as_str()) == Some("Runtime.exceptionThrown") {
            let params = v.get("params").cloned().unwrap_or(Value::Null);
            push_console(&shared, exc_message(&params)).await;
            continue;
        }
        if v.get("method").and_then(|m| m.as_str()) == Some("Log.entryAdded") {
            let params = v
                .get("params")
                .and_then(|p| p.get("entry"))
                .cloned()
                .unwrap_or(Value::Null);
            push_console(&shared, log_message(&params)).await;
            continue;
        }
        // Network-idle plumbing: track in-flight request ids for the
        // `networkIdle` wait. Finishes wake waiters; the 50 ms poll in
        // `wait_network_idle` is the fallback, so a missed event costs one
        // cadence, never a hang.
        if v.get("method").and_then(|m| m.as_str()) == Some("Network.requestWillBeSent") {
            if let Some(rid) = v
                .get("params")
                .and_then(|p| p.get("requestId"))
                .and_then(|r| r.as_str())
            {
                shared.net_inflight.lock().await.insert(rid.to_string());
            }
            continue;
        }
        if v.get("method").and_then(|m| m.as_str()) == Some("Network.loadingFinished")
            || v.get("method").and_then(|m| m.as_str()) == Some("Network.loadingFailed")
        {
            if let Some(rid) = v
                .get("params")
                .and_then(|p| p.get("requestId"))
                .and_then(|r| r.as_str())
            {
                shared.net_inflight.lock().await.remove(rid);
            }
            shared.net_notify.notify_waiters();
            continue;
        }
        // A JS dialog (`alert`/`confirm`/`prompt`/`beforeunload`) blocks the
        // page's JS thread: every in-flight Runtime.evaluate hangs until the
        // dialog is handled (measured: dlg-01 click HANG, 15s bench timeout).
        // Auto-ACCEPT immediately so a dialog can NEVER wedge a run, and
        // record it for the acting tool's typed report.
        if v.get("method").and_then(|m| m.as_str()) == Some("Page.javascriptDialogOpening") {
            let params = v.get("params").cloned().unwrap_or(Value::Null);
            let dtype = params
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("alert")
                .to_string();
            let message = params
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string();
            let url = params
                .get("url")
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_string();
            *shared.dialog.lock().await = Some(DialogRecord {
                dtype: dtype.clone(),
                message,
                url,
            });
            let dialog_params = if dtype == "prompt" {
                json!({"accept": true, "promptText": ""})
            } else {
                json!({"accept": true})
            };
            let hid = shared.next_id.fetch_add(1, Ordering::SeqCst);
            let mut envelope = serde_json::Map::new();
            envelope.insert("id".to_string(), json!(hid));
            envelope.insert(
                "method".to_string(),
                json!("Page.handleJavaScriptDialog"),
            );
            envelope.insert("params".to_string(), dialog_params);
            if let Some(s) = v.get("sessionId").and_then(|s| s.as_str()) {
                envelope.insert("sessionId".to_string(), json!(s));
            }
            let text = Value::Object(envelope).to_string();
            // Register the id so the reply is routed (and dropped) instead
            // of lingering as an unknown response.
            let (tx, _rx) = oneshot::channel();
            shared.pending.lock().unwrap().insert(hid, tx);
            let mut sink = shared.sink.lock().await;
            if sink.send(Message::Text(text.into())).await.is_err() {
                shared.pending.lock().unwrap().remove(&hid);
            }
        }
    }
}

/// Headless-shell-only engine resolution: `GLADIUS_CHROME` wins, else the
/// newest `chrome-headless-shell.exe` under `%LOCALAPPDATA%\ms-playwright`.
/// There is deliberately NO system-Chrome fallback: only the headless shell
/// backend is supported, so a missing shell is a clear error, never a
/// silent switch to a headed-capable binary.
fn find_chrome() -> Result<std::path::PathBuf, String> {
    if let Ok(p) = std::env::var("GLADIUS_CHROME") {
        if !p.trim().is_empty() {
            return Ok(std::path::PathBuf::from(p));
        }
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let base = std::path::PathBuf::from(local).join("ms-playwright");
        if let Ok(rd) = std::fs::read_dir(&base) {
            let mut cands: Vec<std::path::PathBuf> = Vec::new();
            for e in rd.flatten() {
                let p = e
                    .path()
                    .join("chrome-headless-shell-win64")
                    .join("chrome-headless-shell.exe");
                if p.is_file() {
                    cands.push(p);
                }
            }
            cands.sort();
            if let Some(newest) = cands.pop() {
                return Ok(newest);
            }
        }
    }
    Err(format!(
        "chrome backend not found: no chrome-headless-shell.exe under \
         %LOCALAPPDATA%\\ms-playwright\\*\\chrome-headless-shell-win64\\ \
         (e.g. %LOCALAPPDATA%\\ms-playwright\\chromium_headless_shell-*/\
         chrome-headless-shell-win64\\chrome-headless-shell.exe); \
         set GLADIUS_CHROME to an explicit browser binary path"
    ))
}

/// Startup deadline for the DevTools listening line (default 30s).
/// `GLADIUS_STARTUP_TIMEOUT_SECS` overrides it (tests use it to force the
/// timeout path quickly).
fn startup_timeout() -> Duration {
    std::env::var("GLADIUS_STARTUP_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(30))
}

/// Test hook: when `GLADIUS_TEST_STUB_SLEEP_SECS=N` is set, spawn a silent
/// sleeper instead of Chrome. It never prints the DevTools line, so the
/// IDENTICAL startup-wait path below is exercised; its command line carries
/// the `GLADIUS_STARTUP_STUB` marker so tests can assert no stub survives.
fn stub_sleep_secs() -> Option<u64> {
    std::env::var("GLADIUS_TEST_STUB_SLEEP_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
}

/// RAII guard over the freshly spawned backend: dropping it with the child
/// still inside kills the whole process tree. Every early return between
/// spawn and the successful `Inner` build drops the guard, so NO
/// startup-failure path can leak a browser process.
struct ChildGuard {
    child: Option<tokio::process::Child>,
    pid: u32,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            kill_tree(self.pid);
            let _ = c.start_kill();
        }
    }
}

async fn launch() -> Result<Inner, String> {
    let deadline = startup_timeout();
    sweep_stale_profiles();
    let profile = ProfileDir::create()?;
    let mut std_cmd: std::process::Command;
    if let Some(secs) = stub_sleep_secs() {
        eprintln!("[gladius] backend: test stub-sleeper ({secs}s)");
        std_cmd = std::process::Command::new("powershell");
        std_cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!("Start-Sleep -Seconds {secs} # GLADIUS_STARTUP_STUB"),
        ]);
    } else {
        let chrome = find_chrome()?;
        eprintln!("[gladius] backend: {}", chrome.display());
        eprintln!("[gladius] profile: {}", profile.path().display());
        std_cmd = std::process::Command::new(&chrome);
        std_cmd.args(headless_argv(profile.path()));
    }
    std_cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std_cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let spawned = Command::from(std_cmd)
        .spawn()
        .map_err(|e| format!("spawn chrome: {e}"))?;
    // Guard is armed from here on: any `?`/early-return kills the tree.
    let mut guard = ChildGuard {
        pid: spawned.id().unwrap_or(u32::MAX),
        child: Some(spawned),
    };
    if guard.pid == u32::MAX {
        return Err("chrome child has no pid".to_string());
    }
    // Hard-kill cover (G-4): the OS reaps the tree when gladius dies by ANY
    // means. Degrades to None (graceful kill_tree still applies) where the
    // host forbids job nesting; never fails the launch.
    #[cfg(windows)]
    let job = job::arm(guard.child.as_ref().expect("guard holds child"));
    let pid = guard.pid;
    let stderr = guard
        .child
        .as_mut()
        .and_then(|c| c.stderr.take())
        .ok_or("chrome child has no stderr".to_string())?;

    let mut lines = BufReader::new(stderr).lines();
    let start = Instant::now();
    let ws_url = loop {
        let elapsed = start.elapsed();
        if elapsed >= deadline {
            return Err("timeout waiting for DevTools listening line".to_string());
        }
        let remain = deadline - elapsed;
        let next = tokio::time::timeout(remain, lines.next_line())
            .await
            .map_err(|_| "stderr read timeout".to_string())?
            .map_err(|e| format!("stderr read: {e}"))?;
        match next {
            None => {
                return Err("chrome stderr closed before DevTools line".to_string());
            }
            Some(l) => {
                if l.trim().is_empty() {
                    continue;
                }
                eprintln!("[gladius] chrome: {l}");
                if let Some(i) = l.find("ws://") {
                    break l[i..].trim().to_string();
                }
            }
        }
    };

    let (ws, _) = connect_async(&ws_url)
        .await
        .map_err(|e| format!("ws connect {ws_url}: {e}"))?;
    let (sink, stream) = ws.split();
    let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let shared = Arc::new(Shared {
        sink: AsyncMutex::new(sink),
        pending,
        next_id: AtomicU64::new(1),
        session: AsyncMutex::new(None),
        target: AsyncMutex::new(None),
        dialog: AsyncMutex::new(None),
        load_notify: tokio::sync::Notify::new(),
        tab_order: AsyncMutex::new(Vec::new()),
        console: AsyncMutex::new(VecDeque::new()),
        net_inflight: AsyncMutex::new(HashSet::new()),
        net_notify: tokio::sync::Notify::new(),
        net_enabled: AsyncMutex::new(false),
    });
    let reader = tokio::spawn(reader_loop(stream, shared.clone()));
    eprintln!("[gladius] browser ready pid={pid}");
    let child = guard.child.take().expect("guard holds child");
    drop(guard); // disarmed: child moved out, Drop is a no-op
    Ok(Inner {
        _profile: profile,
        #[cfg(windows)]
        _job: job,
        child,
        pid,
        shared,
        reader,
    })
}

/// Headless engine argv for `profile_dir`. Kept separate from the
/// spawn/cleanup plumbing so a future headed opt-in mode can reuse engine
/// resolution (`find_chrome`) with its own argv without touching the rest
/// of `launch`.
fn headless_argv(profile_dir: &std::path::Path) -> Vec<String> {
    vec![
        "--headless".to_string(),
        "--disable-gpu".to_string(),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--remote-debugging-port=0".to_string(),
        // Memory tuning (measured): one renderer, no background services.
        "--process-per-site".to_string(),
        "--renderer-process-limit=1".to_string(),
        "--disable-background-networking".to_string(),
        "--disable-sync".to_string(),
        "--disable-default-apps".to_string(),
        "--disable-features=Translate,BackForwardCache,MediaRouter,OptimizationHints,AcceptCHFrame"
            .to_string(),
        "--mute-audio".to_string(),
        "--disable-software-rasterizer".to_string(),
        // Tab-opening clicks are synthetic (no trusted gesture), so a
        // stock Chrome popup blocker swallows target=_blank opens
        // (measured: window.open -> null on system Chrome, new target on
        // headless-shell). Automation backends must not depend on which
        // binary enforces the blocker.
        "--disable-popup-blocking".to_string(),
        format!("--user-data-dir={}", profile_dir.display()),
        "about:blank".to_string(),
    ]
}

/// Owned browser profile dir. The name carries our own pid
/// (`gladius-<gladius-pid>-<millis>-<n>`) so a later launch can sweep it
/// after a hard kill; graceful exit removes it via this Drop (as before).
struct ProfileDir {
    path: std::path::PathBuf,
}

impl ProfileDir {
    fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn create() -> Result<Self, String> {
        static CTR: AtomicU64 = AtomicU64::new(0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        for _ in 0..100 {
            let n = CTR.fetch_add(1, Ordering::SeqCst);
            let path =
                std::env::temp_dir().join(format!("gladius-{}-{now}-{n}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(ProfileDir { path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("profile dir: {e}")),
            }
        }
        Err("profile dir: too many collisions".to_string())
    }
}

impl Drop for ProfileDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Delete `gladius-<pid>-*` profile dirs whose owner gladius pid is dead.
/// Runs at every launch: the recovery path for hard-killed servers whose
/// graceful cleanup never ran. Conservative: our prefix only, never our own
/// pid, and only provably-dead owners (any doubt keeps the dir, so a live
/// server's profile is never swept).
fn sweep_stale_profiles() {
    let rd = match std::fs::read_dir(std::env::temp_dir()) {
        Ok(r) => r,
        Err(_) => return,
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let pid: u32 = match name
            .strip_prefix("gladius-")
            .and_then(|r| r.split('-').next())
            .and_then(|p| p.parse().ok())
        {
            Some(p) => p,
            None => continue,
        };
        if pid == std::process::id() {
            continue;
        }
        if owner_alive(pid) {
            continue;
        }
        eprintln!(
            "[gladius] sweep: removing stale profile {}",
            e.path().display()
        );
        let _ = std::fs::remove_dir_all(e.path());
    }
}

#[cfg(windows)]
fn owner_alive(pid: u32) -> bool {
    job::process_alive(pid)
}

#[cfg(not(windows))]
fn owner_alive(_pid: u32) -> bool {
    true // conservative: never sweep where liveness is unchecked
}

#[cfg(windows)]
fn kill_tree(pid: u32) {
    let mut cmd = std::process::Command::new("taskkill");
    cmd.args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let _ = cmd.status();
}

#[cfg(not(windows))]
fn kill_tree(_pid: u32) {}

/// Raw `kernel32` job-object cover (G-4): no new crate. The spawned browser
/// is assigned to a job with `KILL_ON_JOB_CLOSE`, so the OS reaps the whole
/// tree when the last job handle closes - which happens on ANY gladius
/// death, including a hard `TerminateProcess` that skips `Drop`/`kill_tree`.
/// The handle lives in `Inner`: graceful `close()` keeps using `kill_tree`
/// first, and the job fires only if stragglers remain.
#[cfg(windows)]
mod job {
    use std::os::windows::io::{FromRawHandle, OwnedHandle, RawHandle};

    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const ERROR_INVALID_PARAMETER: u32 = 87;

    #[repr(C)]
    #[derive(Default)]
    struct BasicLimits {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimits {
        basic: BasicLimits,
        io: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    extern "system" {
        fn CreateJobObjectW(attrs: *const std::ffi::c_void, name: *const u16) -> RawHandle;
        fn SetInformationJobObject(
            job: RawHandle,
            class: u32,
            info: *const std::ffi::c_void,
            len: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: RawHandle, process: RawHandle) -> i32;
        fn CloseHandle(handle: RawHandle) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> RawHandle;
        fn GetLastError() -> u32;
    }

    /// Assign `child` to a fresh kill-on-close job, returning the handle
    /// that must stay open for the browser's lifetime. Returns `None` (with
    /// a stderr note, graceful `kill_tree` still applies) where the host
    /// already placed us in a job that forbids nesting - never fails the
    /// launch over hard-kill cover alone.
    pub(super) fn arm(child: &tokio::process::Child) -> Option<OwnedHandle> {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                eprintln!("[gladius] job: CreateJobObjectW failed, no hard-kill cover");
                return None;
            }
            let mut info = ExtendedLimits::default();
            info.basic.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<ExtendedLimits>() as u32,
            );
            if ok == 0 {
                CloseHandle(job);
                eprintln!("[gladius] job: SetInformationJobObject failed, no hard-kill cover");
                return None;
            }
            let proc = match child.raw_handle() {
                Some(h) => h as RawHandle,
                None => {
                    CloseHandle(job);
                    eprintln!("[gladius] job: child has no handle, no hard-kill cover");
                    return None;
                }
            };
            if AssignProcessToJobObject(job, proc) == 0 {
                CloseHandle(job);
                eprintln!(
                    "[gladius] job: AssignProcessToJobObject failed, no hard-kill cover"
                );
                return None;
            }
            eprintln!("[gladius] job: browser armed with KILL_ON_JOB_CLOSE");
            Some(OwnedHandle::from_raw_handle(job))
        }
    }

    /// True unless the pid is provably dead (conservative: any doubt keeps
    /// the profile dir, so a live server's profile is never swept).
    pub(super) fn process_alive(pid: u32) -> bool {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if !h.is_null() {
                CloseHandle(h);
                return true;
            }
            GetLastError() != ERROR_INVALID_PARAMETER
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rig for timeout/atomicity proofs: a real WS pair whose server side
    /// NEVER answers (black-hole peer, local only), plus a sleeper child
    /// standing in for the browser process (reaped by `close`).
    struct TestRig {
        browser: Browser,
        shared: Arc<Shared>,
        tasks: Vec<tokio::task::JoinHandle<()>>,
    }

    async fn rig() -> TestRig {
        let ln = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback bind");
        let port = ln.local_addr().expect("loopback addr").port();
        let srv = tokio::spawn(async move {
            while let Ok((s, _)) = ln.accept().await {
                tokio::spawn(async move {
                    match tokio_tungstenite::accept_async(s).await {
                        Ok(mut ws) => {
                            while ws.next().await.is_some() {}
                        }
                        Err(_) => {}
                    }
                });
            }
        });
        let url = format!("ws://127.0.0.1:{port}/");
        let (ws, _) = connect_async(&url).await.expect("ws connect");
        let (sink, stream) = ws.split();
        let shared = Arc::new(Shared {
            sink: AsyncMutex::new(sink),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicU64::new(1),
            session: AsyncMutex::new(None),
            target: AsyncMutex::new(None),
            dialog: AsyncMutex::new(None),
            load_notify: tokio::sync::Notify::new(),
            tab_order: AsyncMutex::new(Vec::new()),
            console: AsyncMutex::new(VecDeque::new()),
            net_inflight: AsyncMutex::new(HashSet::new()),
            net_notify: tokio::sync::Notify::new(),
            net_enabled: AsyncMutex::new(false),
        });
        let reader = tokio::spawn(reader_loop(stream, shared.clone()));
        let mut std_cmd = std::process::Command::new("powershell");
        std_cmd
            .args(["-NoProfile", "-NonInteractive", "-Command", "Start-Sleep -Seconds 120"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            std_cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let child = Command::from(std_cmd).spawn().expect("spawn sleeper");
        let pid = child.id().expect("sleeper pid");
        let profile = ProfileDir::create().expect("profile");
        let inner = Inner {
            _profile: profile,
            #[cfg(windows)]
            _job: None,
            child,
            pid,
            shared: shared.clone(),
            reader,
        };
        let browser = Browser {
            state: AsyncMutex::new(Some(inner)),
            pid: Mutex::new(Some(pid)),
            refs: AsyncMutex::new(HashMap::new()),
            snap_url: AsyncMutex::new(String::new()),
        };
        TestRig {
            browser,
            shared,
            tasks: vec![srv],
        }
    }

    impl TestRig {
        async fn shutdown(self) {
            let TestRig {
                browser, tasks, ..
            } = self;
            let _ = browser.close().await;
            for t in tasks {
                t.abort();
            }
        }
    }

    fn drain_count(v: &Value) -> usize {
        v.get("messages")
            .and_then(|m| m.as_array())
            .map(|a| a.len())
            .unwrap_or(0)
    }

    /// F6 proof: an unanswered call must not leave its `tx` in the pending
    /// map. Pre-fix the timeout path returns without removing the entry (it
    /// is removed only if a late response arrives, which never happens for
    /// a dead peer): one leaked entry per timed-out call.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_raw_cleans_pending_on_timeout() {
        let rig = rig().await;
        let r = send_raw_with_timeout(
            &rig.shared,
            "Page.enable",
            json!({}),
            None,
            Duration::from_millis(100),
        )
        .await;
        assert!(r.is_err(), "black hole never answers: {r:?}");
        assert!(
            rig.shared.pending.lock().unwrap().is_empty(),
            "pending map leaked the timed-out call"
        );
        rig.shutdown().await;
    }

    /// F5 proof: `wait_for_load(100ms)` against a never-answering peer must
    /// return in ~100ms, not after `send_raw`'s 30s per-call timeout. Pre-fix
    /// the eval is unwrapped, so this blocks ~30s and fails the bound below.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wait_for_load_respects_cap_on_dead_peer() {
        let rig = rig().await;
        let t0 = Instant::now();
        rig.browser
            .wait_for_load(Duration::from_millis(100))
            .await;
        let dt = t0.elapsed();
        assert!(
            dt < Duration::from_secs(10),
            "soft cap bypassed by unwrapped eval: took {dt:?}"
        );
        rig.shutdown().await;
    }

    /// F4 proof: `console_drain` (single-lock take-and-return) against a
    /// concurrent pusher. Every pushed message is either returned by some
    /// drain or still in the ring: the reconciled total always equals what
    /// was pushed. The pre-fix two-step shape (get-then-clear) lost 115 of
    /// 150 messages on this same rig. Timing-dependent by nature: a failure
    /// here means loss.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn console_drain_is_atomic() {
        const N: u64 = 150; // under CONSOLE_CAP: no eviction can explain loss
        let rig = rig().await;
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sh = rig.shared.clone();
        let d = done.clone();
        let pusher = tokio::spawn(async move {
            for i in 0..N {
                push_console(
                    &sh,
                    ConsoleMsg {
                        mtype: "log".to_string(),
                        text: format!("tick {i}"),
                        url: String::new(),
                        line: 0,
                        ts: i,
                    },
                )
                .await;
                tokio::task::yield_now().await;
            }
            d.store(true, Ordering::SeqCst);
        });
        let mut returned = 0usize;
        loop {
            let v = rig.browser.console_drain(200).await;
            returned += drain_count(&v);
            if done.load(Ordering::SeqCst) {
                // Pusher is finished: one last drain, then the ring is empty
                // and the books must balance exactly.
                let v = rig.browser.console_drain(200).await;
                returned += drain_count(&v);
                let rest = rig.browser.console_get(200).await;
                assert_eq!(drain_count(&rest), 0, "ring must be empty at the end");
                assert_eq!(
                    returned, N as usize,
                    "atomic drain lost {} of {N} messages",
                    N as usize - returned
                );
                break;
            }
        }
        pusher.await.expect("pusher");
        rig.shutdown().await;
    }
}
