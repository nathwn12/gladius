use std::collections::{HashMap, HashSet, VecDeque};
use std::process::Stdio;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::sync::{Mutex as AsyncMutex, oneshot};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{
        Message,
        client::IntoClientRequest,
        handshake::client::Request as WsRequest,
        http::Uri as TungsteniteHttpUri,
    },
};

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
    /// Active device-metrics override (`Emulation.setDeviceMetricsOverride`
    /// params), or None when the engine default applies. The override is a
    /// property of the CDP SESSION, not of the connection, so it is
    /// replayed by `attach_to` on every new session (new tab, re-attach):
    /// without the replay a fresh tab would silently revert to the engine
    /// default while the caller still believed the viewport was set.
    viewport: AsyncMutex<Option<Value>>,
}

/// Ownership of the engine behind an `Inner` connection.
///
/// - `Owned`: gladius launched the owned Chromium (`chrome-headless-shell`,
///   pinned build resolved by `find_chromium`) itself: child handle, job
///   cover, and one fresh private profile dir created before launch and
///   swept after the tree is reaped. `close()` reaps the whole tree.
/// - `Attached`: gladius connected to an externally started engine via
///   `GLADIUS_CDP_URL` (the external-CDP seam: any compatible engine plugs in
///   here). The engine is NOT ours: `close()` only detaches
///   the CDP session we opened and never kills, never sweeps a profile.
enum Engine {
    Owned {
        /// Private profile dir for this instance (created fresh before
        /// launch; `Drop` sweeps it after the tree is reaped).
        _profile: ProfileDir,
        #[cfg(windows)]
        _job: Option<std::os::windows::io::OwnedHandle>,
        child: Box<tokio::process::Child>,
        pid: u32,
    },
    Attached {
        cdp_url: String,
    },
}

struct Inner {
    engine: Engine,
    shared: Arc<Shared>,
    reader: tokio::task::JoinHandle<()>,
}

/// Declared backend capability (`GLADIUS_BACKEND_CAPS`).
///
/// - `Render` (default): a real rendering pipeline exists; `screenshot`
///   returns pixels.
/// - `DomOnly`: no rendering pipeline (DOM/CDP only). Pixel tools must refuse,
///   never fake an image.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BackendCaps {
    Render,
    DomOnly,
}

/// Parse one `GLADIUS_BACKEND_CAPS` value. Only `render` and `dom-only`
/// (case-insensitive, surrounding whitespace ignored) are accepted.
pub fn parse_backend_caps(s: &str) -> Result<BackendCaps, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "render" => Ok(BackendCaps::Render),
        "dom-only" => Ok(BackendCaps::DomOnly),
        other => Err(format!(
            "GLADIUS_BACKEND_CAPS: unknown value '{other}' (want 'render' or 'dom-only')"
        )),
    }
}

/// Effective backend capability. Unset/blank means `render` (the current
/// default path is unchanged).
pub fn backend_caps() -> Result<BackendCaps, String> {
    match std::env::var("GLADIUS_BACKEND_CAPS") {
        Ok(v) if !v.trim().is_empty() => parse_backend_caps(&v),
        _ => Ok(BackendCaps::Render),
    }
}

/// Raw `GLADIUS_CDP_URL` override (trimmed, empty means unset).
pub fn cdp_url_override() -> Option<String> {
    std::env::var("GLADIUS_CDP_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Engine name for the initialize response: `external-cdp` when an explicit
/// `GLADIUS_CDP_URL` attaches to an external engine, otherwise the owned
/// `chromium` launch (the default and the only launchable engine).
pub fn backend_name() -> String {
    if cdp_url_override().is_some() {
        "external-cdp".to_string()
    } else {
        "chromium".to_string()
    }
}

/// `(backend, caps)` labels for the initialize response. Never connects:
/// safe on the cold handshake path. An invalid caps value reports
/// `invalid` here; the tool path surfaces the typed error.
pub fn backend_info() -> Value {
    let caps = match backend_caps() {
        Ok(BackendCaps::DomOnly) => "dom-only",
        Ok(BackendCaps::Render) => "render",
        Err(_) => "invalid",
    };
    match cdp_url_override() {
        Some(url) => json!({"backend": backend_name(), "caps": caps, "cdpUrl": url}),
        None => json!({"backend": backend_name(), "caps": caps}),
    }
}

/// Build the WS `Shared` half from a connected sink. One constructor for
/// both the owned launch path and the attach path, so the session/target
/// flow after the connect (`ensure_session` / `attach_to` / tools) is the
/// SAME code, never a fork.
fn new_shared(sink: WsSink) -> Arc<Shared> {
    Arc::new(Shared {
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
        viewport: AsyncMutex::new(None),
    })
}

/// Resolve a `GLADIUS_CDP_URL` value to a ws:// endpoint.
///
/// - `ws://` / `wss://` connect directly.
/// - `http://` / `https://` resolve via the documented readiness endpoint
///   `GET <base>/json/version` (`webSocketDebuggerUrl`), so the http path
///   never depends on engine stderr wording.
async fn resolve_ws_url(raw: &str) -> Result<String, String> {
    let s = raw.trim();
    if s.starts_with("ws://") || s.starts_with("wss://") {
        return Ok(s.to_string());
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        let base = s.trim_end_matches('/');
        let ver_url = if base.ends_with("/json/version") {
            base.to_string()
        } else {
            format!("{base}/json/version")
        };
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| format!("cdp http client: {e}"))?;
        let resp = client
            .get(&ver_url)
            .send()
            .await
            .map_err(|e| format!("cdp GET {ver_url}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("cdp GET {ver_url}: http {}", resp.status()));
        }
        let text = resp
            .text()
            .await
            .map_err(|e| format!("cdp GET {ver_url} body: {e}"))?;
        let v: Value =
            serde_json::from_str(&text).map_err(|e| format!("cdp {ver_url}: bad json: {e}"))?;
        let ws = v
            .get("webSocketDebuggerUrl")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if ws.is_empty() {
            return Err(format!("cdp {ver_url}: no webSocketDebuggerUrl in response"));
        }
        return Ok(ws);
    }
    Err("GLADIUS_CDP_URL must be a ws://, wss://, http:// or https:// endpoint".to_string())
}

/// Attach to an externally started engine.
/// Connects to the resolved ws endpoint, then reuses the EXISTING
/// session/target flow. Creates no profile, sweeps nothing, owns no pid.
async fn attach(raw: &str) -> Result<Inner, String> {
    let ws_url = resolve_ws_url(raw).await?;
    eprintln!("[gladius] backend: external-cdp {ws_url} (attach; engine not owned)");
    let (ws, _) = connect_async(&ws_url)
        .await
        .map_err(|e| format!("ws connect {ws_url}: {e}"))?;
    let (sink, stream) = ws.split();
    let shared = new_shared(sink);
    let reader = tokio::spawn(reader_loop(stream, shared.clone()));
    eprintln!("[gladius] browser ready attached external-cdp");
    Ok(Inner {
        engine: Engine::Attached { cdp_url: ws_url },
        shared,
        reader,
    })
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
        let owned_pid: Option<u32> = match &inner.engine {
            Engine::Owned { pid, .. } => Some(*pid),
            Engine::Attached { .. } => None,
        };
        *self.pid.lock().unwrap() = owned_pid;
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

    /// Apply a device-metrics override to the ACTIVE page
    /// (`Emulation.setDeviceMetricsOverride`) and remember it, so every
    /// later session on this connection (new tab, re-attach) replays it.
    /// Returns the raw CDP result (kept so a caller can see what the engine
    /// acknowledged, though the tool reports the live read-back instead).
    pub async fn viewport_set(&self, params: Value) -> Result<Value, String> {
        let shared = self.shared().await?;
        let session = ensure_session(&shared).await?;
        let r = send_raw(
            &shared,
            "Emulation.setDeviceMetricsOverride",
            params.clone(),
            Some(session),
        )
        .await?;
        *shared.viewport.lock().await = Some(params);
        Ok(r)
    }

    /// Clear the override on the active page
    /// (`Emulation.clearDeviceMetricsOverride`) and forget it, so a later
    /// new tab also gets the engine default.
    pub async fn viewport_reset(&self) -> Result<Value, String> {
        let shared = self.shared().await?;
        let session = ensure_session(&shared).await?;
        // Pre-clear size: the settle target below.
        let before = self.viewport_readback().await.ok();
        *shared.viewport.lock().await = None;
        let r = send_raw(
            &shared,
            "Emulation.clearDeviceMetricsOverride",
            json!({}),
            Some(session),
        )
        .await;
        // Settle: on later-created targets the size update can lag the
        // acknowledgement (measured on Chromium: stale size but fresh DPR
        // immediately after clear, correct size within 1s). Poll briefly
        // for a size that differs from pre-clear; expiry just falls through
        // to the tool's live read-back, never an error.
        if let Some((bw, bh, _)) = before {
            let deadline = Instant::now() + Duration::from_secs(1);
            loop {
                match self.viewport_readback().await {
                    Ok((w, h, _)) if w != bw || h != bh => break,
                    _ => {}
                }
                if Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        r
    }

    /// Live `(innerWidth, innerHeight, devicePixelRatio)` from the active
    /// page. Every viewport call returns this read-back, so an override the
    /// engine silently ignored can never read as success.
    pub async fn viewport_readback(&self) -> Result<(i64, i64, f64), String> {
        let v = self
            .eval("[window.innerWidth, window.innerHeight, window.devicePixelRatio]")
            .await?;
        let arr = v
            .as_array()
            .ok_or("viewport: read-back was not an array".to_string())?;
        let num = |i: usize| arr.get(i).and_then(|x| x.as_f64());
        let w = num(0).ok_or("viewport: no window.innerWidth in read-back".to_string())?;
        let h = num(1).ok_or("viewport: no window.innerHeight in read-back".to_string())?;
        // devicePixelRatio is reported, never asserted: an engine that does
        // not emulate the scale still reports a usable size.
        let dpr = num(2).unwrap_or(1.0);
        Ok((w as i64, h as i64, dpr))
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
        // A click that opens a tab returns before the engine lists the new
        // target (measured: one target still listed ~120ms after
        // click_settle on a slow engine, two already on a fast one). Bounded
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
            let Inner {
                engine,
                shared,
                reader,
            } = inner;
            match engine {
                Engine::Owned { mut child, pid, .. } => {
                    reader.abort();
                    // Fast path: terminate the direct child and wait for it. The
                    // KILL_ON_JOB_CLOSE job handle (dropped with the rest of the
                    // owned state) reaps surviving grandchildren, so the old
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
                Engine::Attached { cdp_url } => {
                    // Ownership rule: the attached engine is NOT ours. Detach
                    // the CDP session we opened (best-effort, before the
                    // reader goes away so the reply can route), then drop the
                    // connection. Never kill_tree, never kill a pid, never
                    // sweep a profile we did not create. Pages/targets in the
                    // external engine stay open by design.
                    if let Some(sid) = shared.session.lock().await.clone() {
                        let _ = send_raw(
                            &shared,
                            "Target.detachFromTarget",
                            json!({"sessionId": sid}),
                            None,
                        )
                        .await;
                    }
                    *shared.session.lock().await = None;
                    *shared.target.lock().await = None;
                    reader.abort();
                    eprintln!("[gladius] detach: external engine left running (not owned) {cdp_url}");
                }
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
    /// wait pays one round-trip; every other op pays nothing). On Chromium
    /// `Network.enable` alone is sufficient to observe request starts via
    /// `Network.requestWillBeSent`; no `Fetch.enable` interception is used.
    pub async fn ensure_network(&self) -> Result<(), String> {
        let shared = self.shared().await?;
        if *shared.net_enabled.lock().await {
            return Ok(());
        }
        let session = ensure_session(&shared).await?;
        send_raw(&shared, "Network.enable", json!({}), Some(session.clone())).await?;
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

/// Owns one pending-map slot for the lifetime of the call that inserted it.
/// A dropped call future (a caller's `tokio::time::timeout` clipping it, a
/// cancelled task, an aborted wait) removes its own entry here, so a `tx`
/// can never outlive the call that created it: `send_raw_with_timeout`'s
/// timeout path is NOT the only exit. A late response finds no entry - the
/// reader removed it first and a second removal is a no-op.
struct PendingGuard {
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    id: u64,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.pending.lock().unwrap().remove(&self.id);
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
    // Every exit from here on - including a dropped future - clears the entry
    // (the old explicit removals below covered only two of the exits).
    let _guard = PendingGuard {
        pending: shared.pending.clone(),
        id,
    };
    {
        let mut sink = shared.sink.lock().await;
        if let Err(e) = sink.send(Message::Text(text.into())).await {
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
        // Timeout path: the entry is removed by `_guard`'s drop, so one
        // unanswered call never leaves a `tx` behind (a late response is the
        // only other exit, and a dead peer never sends one).
        Err(_) => Err("cdp: response timeout".to_string()),
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
    // Replay an active device-metrics override onto THIS session. The
    // override is session-scoped, so a new tab / re-attach would otherwise
    // observe (and report) the engine default while the caller still holds
    // the numbers it set. Best-effort like the enables above: a replay
    // failure is NOT silent - the next read-back reports the real size.
    let pending_viewport = shared.viewport.lock().await.clone();
    if let Some(params) = pending_viewport {
        let _ = send_raw(
            shared,
            "Emulation.setDeviceMetricsOverride",
            params,
            Some(sid.clone()),
        )
        .await;
    }
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
        // Chromium emits `Log.entryAdded` live (failed resources,
        // violations), so the Log leg reports alongside
        // `Runtime.consoleAPICalled`.
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
        // Chromium fires `Page.javascriptDialogOpening` with
        // stall-until-handled semantics, so this path acts on every dialog.
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

/// Startup deadline for the owned-engine readiness poll (default 30s).
/// `GLADIUS_STARTUP_TIMEOUT_SECS` overrides it (tests use it to force the
/// timeout path quickly).
fn startup_timeout() -> Duration {
    std::env::var("GLADIUS_STARTUP_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(30))
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

/// Headed-mode request: headless-only BY OWNER DECISION (2026-10-17).
/// There is no headed code path at all: the `chrome-headless-shell` artifact
/// cannot go headed and that is a property we rely on. `GLADIUS_HEADED` set
/// to a truthy non-blank value (`1`/`true`/`yes`, case-insensitive) is an
/// explicit request for a visible browser and fails closed here; blank/unset
/// (and `0`/`false`/`no`) means headless. A headed CLI flag is rejected in
/// `main.rs` with the same error.
fn headed_requested() -> bool {
    match std::env::var("GLADIUS_HEADED") {
        Ok(v) => {
            let t = v.trim().to_ascii_lowercase();
            !t.is_empty() && !["0", "false", "no", "off"].contains(&t.as_str())
        }
        Err(_) => false,
    }
}

/// Headed-mode refusal text (single source: `launch` and `main.rs` share it).
fn headed_unsupported_error() -> String {
    "headed mode is not supported: gladius is headless-only by owner decision \
     (2026-10-17); the owned chrome-headless-shell artifact cannot go headed. \
     Attach to an externally started engine with GLADIUS_CDP_URL instead"
        .to_string()
}

/// Launch dispatcher: a headed request fails closed (no headed path exists);
/// `GLADIUS_CDP_URL` set means attach to the external engine instead of
/// launching; anything else launches the owned Chromium
/// (`launch_owned_chromium`, the default and only launch path).
async fn launch() -> Result<Inner, String> {
    if headed_requested() {
        return Err(headed_unsupported_error());
    }
    // A bad caps value is a config error: fail fast on either path.
    backend_caps()?;
    if let Some(raw) = cdp_url_override() {
        return attach(&raw).await;
    }
    launch_owned_chromium().await
}

/// Pinned Chromium identity (keep in lockstep with
/// `scripts/fetch-chromium.ps1` and `scripts/chromium-manifest.json`).
const PINNED_CHROMIUM_VERSION: &str = "155.0.8059.84";
/// First 8 hex chars of the pinned zip sha256 (the versioned cache dir tag).
const PINNED_ZIP_SHORT8: &str = "5ca590a4";
/// Env override for the explicit browser binary path.
const CHROMIUM_ENV: &str = "GLADIUS_CHROMIUM";
/// Env override for the versioned cache root.
const CHROMIUM_CACHE_ENV: &str = "GLADIUS_CHROMIUM_CACHE";
/// Zip-relative path of the shell binary inside the versioned cache dir.
const CHROMIUM_EXE_REL: &str = "chrome-headless-shell-win64/chrome-headless-shell.exe";

/// Owned Chromium resolution: an explicit `GLADIUS_CHROMIUM` path wins when
/// set and non-blank (must be a file), else the pinned build under the owned
/// cache root (`GLADIUS_CHROMIUM_CACHE` when set and non-blank, otherwise
/// `%LOCALAPPDATA%\gladius\chromium\<version>-<zip-sha8>\`). A missing
/// browser is a CLEAR error naming `scripts/fetch-chromium.ps1`: never a
/// silent fallback to any ambient, shared, or system browser.
fn find_chromium() -> Result<std::path::PathBuf, String> {
    if let Ok(p) = std::env::var(CHROMIUM_ENV) {
        if !p.trim().is_empty() {
            let cand = std::path::PathBuf::from(p.trim());
            if cand.is_file() {
                return Ok(cand);
            }
            return Err(format!(
                "chromium browser not found: GLADIUS_CHROMIUM='{}' is not a file; \
                 run scripts/fetch-chromium.ps1 to provision the pinned browser",
                cand.display()
            ));
        }
    }
    let root = match std::env::var(CHROMIUM_CACHE_ENV) {
        Ok(v) if !v.trim().is_empty() => std::path::PathBuf::from(v.trim()),
        _ => match std::env::var("LOCALAPPDATA") {
            Ok(local) if !local.trim().is_empty() => {
                std::path::PathBuf::from(local.trim()).join("gladius/chromium")
            }
            _ => {
                return Err(
                    "chromium browser not found: LOCALAPPDATA is unset and \
                     GLADIUS_CHROMIUM_CACHE is unset; set one of them or run \
                     scripts/fetch-chromium.ps1 to provision the pinned browser"
                        .to_string(),
                );
            }
        },
    };
    let cand = root
        .join(format!("{PINNED_CHROMIUM_VERSION}-{PINNED_ZIP_SHORT8}"))
        .join(CHROMIUM_EXE_REL.replace('/', std::path::MAIN_SEPARATOR_STR));
    if cand.is_file() {
        return Ok(cand);
    }
    Err(format!(
        "chromium browser not found: expected {} (pinned \
         {PINNED_CHROMIUM_VERSION}); run scripts/fetch-chromium.ps1 to provision \
         the pinned browser",
        cand.display()
    ))
}

/// Owned Chromium argv for `port` with private `profile_dir`. Each flag
/// appears EXACTLY ONCE (kept with the repo's exactly-once discipline).
/// Bare `--headless` is the shell's headless selector (the full browser's
/// new-mode spelling is not ours, and the removed old-mode spelling must
/// never reappear). The profile dir is created fresh per instance before
/// launch: since Chrome 136 the remote-debugging switches are ignored on
/// the default data directory, so without a private profile no port binds
/// and readiness never goes ready. `about:blank` is the last positional
/// token. Never passed: sandbox-off switches (unsupported on Windows),
/// dev-shm switches (Linux containers only), keychain/password-store
/// flags, or the piped debug transport (it has no HTTP surface, so the
/// readiness poll cannot work).
fn chromium_argv(port: u16, profile_dir: &std::path::Path) -> Vec<String> {
    vec![
        "--headless".to_string(),
        format!("--remote-debugging-port={port}"),
        format!("--user-data-dir={}", profile_dir.display()),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        format!("--remote-allow-origins=http://127.0.0.1:{port}"),
        "about:blank".to_string(),
    ]
}

/// Confine the OWNED Chromium ws endpoint to our loopback port.
///
/// The owned path frees its chosen port before the browser binds it, so a
/// local squatter can claim the port and advertise `ws://<somewhere-else>/`
/// via `/json/version`. The attach path (`resolve_ws_url`) must keep
/// accepting whatever the user pointed at, so confinement lives here and is
/// applied ONLY to the owned resolve result: the scheme must be `ws`/`wss`,
/// the host (exactly as the connector's parser reports it, brackets
/// stripped) must be loopback (`127.0.0.1`, `::1`, `localhost`), and the
/// explicit port must equal the port we chose (a missing port is rejected).
/// Pure: no I/O, unit-tested below.
fn confine_owned_ws_uri(port: u16, uri: &TungsteniteHttpUri) -> Result<(), String> {
    let scheme_ok = uri
        .scheme_str()
        .is_some_and(|s| s.eq_ignore_ascii_case("ws") || s.eq_ignore_ascii_case("wss"));
    if !scheme_ok {
        return Err(format!(
            "owned ws endpoint rejected: non-ws scheme in {uri:?}"
        ));
    }
    // Host exactly as the connector's parser (`http::Uri`, the same parse
    // `connect_async` dials off) reports it. `http` keeps the brackets on a
    // bracketed IPv6 literal (`Authority::host` returns `[::1]`), so strip
    // one surrounding pair before comparing; compare case-insensitively.
    // `Authority::host` already drops any userinfo, so a `user@host` shape
    // cannot smuggle a host past this check.
    let host = uri.host().unwrap_or("");
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if !(bare.eq_ignore_ascii_case("127.0.0.1")
        || bare.eq_ignore_ascii_case("::1")
        || bare.eq_ignore_ascii_case("localhost"))
    {
        return Err(format!(
            "owned ws endpoint rejected: non-loopback host {host:?} in {uri:?}"
        ));
    }
    match uri.port_u16() {
        Some(got) if got == port => Ok(()),
        Some(got) => Err(format!(
            "owned ws endpoint rejected: port {got} != owned port {port} in {uri:?}"
        )),
        None => Err(format!("owned ws endpoint rejected: no port in {uri:?}")),
    }
}

/// Parse an owned-path ws URL ONCE with the connector's own parser
/// (`IntoClientRequest` - the same parse `connect_async` performs), confine
/// the resulting URI to our loopback port, and return the SAME request for
/// the connector to dial. Production (`launch_owned_chromium`) and the
/// confinement tests both call this, so validation and dialing can never
/// parse differently: the connector reads host/port off `request.uri()`,
/// which is the exact value confined here. Never hand-roll URI parsing.
fn owned_ws_request(port: u16, url: &str) -> Result<WsRequest, String> {
    let request = url
        .into_client_request()
        .map_err(|e| format!("owned ws endpoint rejected: bad ws url {url:?}: {e}"))?;
    confine_owned_ws_uri(port, request.uri())?;
    Ok(request)
}

/// Remaining startup budget from a SINGLE clock sample. Returns `None` once
/// the deadline has passed; callers map `None` to their phase's timeout
/// error. The single sample is the point: deriving the expiry check and the
/// subtraction from two separate `elapsed()` calls lets the subtraction
/// underflow and panic instead of timing out.
fn startup_remaining(start: Instant, deadline: Duration) -> Option<Duration> {
    let elapsed = start.elapsed();
    if elapsed >= deadline {
        None
    } else {
        Some(deadline - elapsed)
    }
}

/// Fail-closed child poll for the owned startup loop: an exited child is an
/// error (the port may now belong to someone else - never keep polling it),
/// and a `try_wait` failure is an error too (an unknown child state is never
/// treated as alive).
fn check_chromium_child(guard: &mut ChildGuard) -> Result<(), String> {
    match guard.child.as_mut().expect("guard holds child").try_wait() {
        Ok(Some(status)) => Err(format!("chromium child exited during startup: {status}")),
        Ok(None) => Ok(()),
        Err(e) => Err(format!("chromium child wait failed during startup: {e}")),
    }
}

/// Owned-launch attempts: the pick-port → spawn → readiness sequence runs up
/// to this many times, with a FRESH `:0` pick per attempt (the pick lives
/// inside the single-attempt function below, so a stale pick is never
/// reused). Bounded so a permanently broken engine still fails fast with
/// today's error instead of looping.
const OWNED_LAUNCH_ATTEMPTS: usize = 3;

/// Retryable owned-launch failure: ONLY the early-child-exit signal from
/// `check_chromium_child` (`chromium child exited during startup: ...`) — the
/// stale-port race where a local squatter claimed our freed pick between the
/// `:0` probe and the browser's own bind, so the browser died on its bind and
/// exited before `/json/version` ever went ready. Everything else is
/// fail-closed and never retried: the squatter-confinement rejections
/// (`owned ws endpoint rejected: ...`) stay hard errors (re-rolling the port
/// until a squatter's advertisement accidentally matched would defeat the
/// confinement), a `try_wait` failure is unknown child state (never treated
/// as a dead-port signal), and timeouts keep their existing budgets. Pure:
/// no I/O, unit-tested below via the retry helper.
fn launch_startup_exit_is_retryable(err: &str) -> bool {
    err.starts_with("chromium child exited during startup:")
}

/// Bounded retry over fallible attempts: runs `attempt` until it succeeds,
/// surfaces a non-retryable error immediately, and surfaces the LAST error
/// unchanged once `max_attempts` retryable failures exhaust the bound (an
/// exhausted retry reports exactly what the final attempt reported — nothing
/// is hidden). `max_attempts` is clamped to at least 1. Pure control flow:
/// the owned launch passes its single-attempt future plus the
/// early-child-exit predicate; the unit tests pass scripted closures.
async fn retry_attempts<T, E, F, Fut>(
    max_attempts: usize,
    is_retryable: impl Fn(&E) -> bool,
    mut attempt: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let bound = max_attempts.max(1);
    let mut last: Option<E> = None;
    for _ in 0..bound {
        match attempt().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                if !is_retryable(&e) {
                    return Err(e);
                }
                last = Some(e);
            }
        }
    }
    Err(last.expect("retry bound >= 1 runs at least one attempt"))
}

/// Owned launch with stale-port retry: the pick-port → spawn → readiness
/// sequence frees its chosen port before the browser binds it, so under
/// parallel load a local squatter can claim the pick and the browser exits
/// on its bind before `/json/version` ever goes ready. An early child exit
/// during startup is therefore retried with a FRESH port pick, bounded by
/// `OWNED_LAUNCH_ATTEMPTS`. Only that signal retries (see
/// `launch_startup_exit_is_retryable`); every attempt otherwise behaves
/// exactly like today's single launch — same argv, same readiness poll,
/// same budgets, same fail-closed confinement — and exhaustion surfaces the
/// last attempt's error unchanged.
async fn launch_owned_chromium() -> Result<Inner, String> {
    retry_attempts(
        OWNED_LAUNCH_ATTEMPTS,
        |e: &String| launch_startup_exit_is_retryable(e),
        launch_owned_chromium_once,
    )
    .await
}

/// Single owned-launch attempt: the owned Chromium (`chrome-headless-shell`
/// on a free loopback port with a fresh private profile), modelled as
/// `Engine::Owned` so `close()` kills the tree we started and sweeps the
/// profile. Picks a FRESH free port AND a fresh profile on every call (both
/// live inside the single-attempt function below, so a stale pick is never
/// reused). Readiness polls `GET http://127.0.0.1:<port>/json/version` for
/// `webSocketDebuggerUrl` inside `startup_timeout()`. The ws URL then flows
/// through the SAME shared machinery the attach path uses (`resolve_ws_url`
/// handles `http://...` -> `/json/version` -> `webSocketDebuggerUrl`,
/// bounded here by the remaining startup budget so one poll iteration can
/// never exceed it), then through the SAME `owned_ws_request` the tests
/// exercise (connector-parser parse + loopback-port confine, and the
/// returned request is what `connect_async` dials - no re-parse), so the
/// post-connect session/target flow is never forked. Every wait here is
/// bounded (the browser also drops idle browser WS connections at ~60-70 s,
/// so clients connect lazily on first tool use and bound every wait).
async fn launch_owned_chromium_once() -> Result<Inner, String> {
    let deadline = startup_timeout();
    sweep_stale_profiles();
    // Fresh private profile BEFORE launch (functional, not hygiene): since
    // Chrome 136 the remote-debugging switches are ignored on the default
    // data directory, so without this no port binds and readiness never goes
    // ready. The dir is created here so the argv path is private to this
    // instance; `Drop` sweeps it after the tree is reaped.
    let profile = ProfileDir::create()?;
    let chromium = find_chromium()?;
    // Free loopback port: bind :0, read the port, drop it.
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("chromium free port: {e}"))?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("chromium free port: {e}"))?
            .port();
        drop(listener);
        port
    };
    eprintln!(
        "[gladius] backend: chromium {} (port {port})",
        chromium.display()
    );
    eprintln!("[gladius] profile: {}", profile.path().display());
    let mut std_cmd = std::process::Command::new(&chromium);
    std_cmd.args(chromium_argv(port, profile.path()));
    // Readiness is the HTTP poll below, so browser stdio is discarded (never
    // a piped-but-undrained buffer the browser could block on).
    std_cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std_cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let spawned = Command::from(std_cmd)
        .spawn()
        .map_err(|e| format!("spawn chromium: {e}"))?;
    // Guard is armed from here on: any `?`/early-return kills the tree.
    let mut guard = ChildGuard {
        pid: spawned.id().unwrap_or(u32::MAX),
        child: Some(spawned),
    };
    if guard.pid == u32::MAX {
        return Err("chromium child has no pid".to_string());
    }
    // Hard-kill cover (G-4): the OS reaps the tree
    // when gladius dies by ANY means.
    #[cfg(windows)]
    let job = job::arm(guard.child.as_ref().expect("guard holds child"));
    let pid = guard.pid;
    let base = format!("http://127.0.0.1:{port}");
    let start = Instant::now();
    let ws_request = loop {
        // Child-exit detection: if the browser died, the port may now belong
        // to someone else - never keep polling it, fail on the exit status.
        // Fail-closed: a `try_wait` error is never treated as "still alive".
        check_chromium_child(&mut guard)?;
        // Single-sample budget: this one sample feeds BOTH the expiry check
        // and the per-iteration bound below, so the subtraction cannot race
        // and underflow-panic.
        let remaining = startup_remaining(start, deadline).ok_or_else(|| {
            format!(
                "timeout waiting for chromium /json/version on {base}: \
                 deadline passed with no ready endpoint"
            )
        })?;
        // Bound ONE poll iteration by the remaining budget: `resolve_ws_url`
        // carries its own 10 s client timeout, so without this wrap a 1 s
        // startup budget could take ~10 s. `resolve_ws_url` itself stays
        // unrestricted for the attach path - the bound lives here, on the
        // owned path only.
        match tokio::time::timeout(remaining, resolve_ws_url(&base)).await {
            Err(_) => {
                return Err(format!(
                    "timeout waiting for chromium /json/version on {base}: \
                     deadline passed while awaiting resolve"
                ));
            }
            Ok(Ok(u)) => {
                // A success that arrives after the deadline is still a
                // timeout: the budget is enforced, not just tested on error.
                if start.elapsed() >= deadline {
                    return Err(format!(
                        "timeout waiting for chromium /json/version on {base}: \
                         deadline passed before accepting a ready endpoint"
                    ));
                }
                // Re-check the child: it may have died while `resolve_ws_url`
                // awaited - a late resolve from a dead browser's port must
                // never be accepted as a live owned browser.
                check_chromium_child(&mut guard)?;
                // Owned path only: parse with the CONNECTOR's parser and
                // confine the advertised endpoint to our loopback port before
                // connecting anywhere. The loop yields the SAME request the
                // connector will dial (no re-parse).
                break owned_ws_request(port, &u)?;
            }
            Ok(Err(e)) => {
                if start.elapsed() >= deadline {
                    return Err(format!(
                        "timeout waiting for chromium /json/version on {base}: last error: {e}"
                    ));
                }
                // Browser still starting: re-poll at 100 ms cadence inside
                // the same deadline.
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    };

    // The connect also runs inside the startup budget: a stalled handshake
    // must error on elapse, never overshoot the deadline unbounded.
    let ws_url = ws_request.uri().to_string();
    let remaining = startup_remaining(start, deadline).ok_or_else(|| {
        format!("timeout waiting for chromium ws connect on {ws_url}: startup budget exhausted")
    })?;
    let (ws, _) = tokio::time::timeout(remaining, connect_async(ws_request))
        .await
        .map_err(|_| {
            format!(
                "ws connect {ws_url} timed out after {}ms of remaining startup budget",
                remaining.as_millis()
            )
        })?
        .map_err(|e| format!("ws connect {ws_url}: {e}"))?;
    // Post-connect child re-check: confinement only checks the advertised
    // host and port, so a squatter on OUR OWN port advertising a same-port
    // loopback URL passes it - the child's liveness is the only remaining
    // discriminator. If the child died (e.g. its own bind failure) while
    // the handshake awaited, this socket may belong to an impostor: fail
    // here so the guard kills the tree and the socket is dropped.
    // Review-enforced, not test-enforced: no test drives
    // `launch_owned_chromium`, so removing this call (or the
    // `owned_ws_request` confine above) stays green.
    check_chromium_child(&mut guard)?;
    // Tokio's `timeout` polls the inner future first, so a handshake that
    // completes at/after expiry still returns `Ok`: re-enforce the deadline
    // after a successful connect, before building `Inner` or announcing
    // readiness.
    if start.elapsed() >= deadline {
        return Err(format!(
            "timeout waiting for chromium ws connect on {ws_url}: \
             handshake completed after the startup budget expired"
        ));
    }
    let (sink, stream) = ws.split();
    let shared = new_shared(sink);
    let reader = tokio::spawn(reader_loop(stream, shared.clone()));
    eprintln!("[gladius] browser ready pid={pid} (chromium)");
    let child = guard.child.take().expect("guard holds child");
    drop(guard); // disarmed: child moved out, Drop is a no-op
    Ok(Inner {
        engine: Engine::Owned {
            _profile: profile,
            #[cfg(windows)]
            _job: job,
            child: Box::new(child),
            pid,
        },
        shared,
        reader,
    })
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
/// Runs at every owned launch: the recovery path for hard-killed servers
/// whose graceful cleanup never ran. Conservative: our prefix only, never
/// our own pid, and only provably-dead owners (any doubt keeps the dir, so
/// a live server's profile is never swept).
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
    use std::sync::atomic::AtomicUsize;

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
                    if let Ok(mut ws) = tokio_tungstenite::accept_async(s).await {
                        while ws.next().await.is_some() {}
                    }
                });
            }
        });
        let url = format!("ws://127.0.0.1:{port}/");
        let (ws, _) = connect_async(&url).await.expect("ws connect");
        let (sink, stream) = ws.split();
        let shared = new_shared(sink);
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
            engine: Engine::Owned {
                _profile: profile,
                #[cfg(windows)]
                _job: None,
                child: Box::new(child),
                pid,
            },
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

    /// Clipping a call from OUTSIDE (`tokio::time::timeout` around it) drops
    /// the call future mid-await, which the call's own timeout path never
    /// sees: the pending slot must still be cleared, or every clipped call
    /// leaks one `tx` that a peer holding the response never answers.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn clipped_call_leaves_no_pending_entry() {
        let rig = rig().await;
        let clipped = tokio::time::timeout(
            Duration::from_millis(50),
            send_raw_with_timeout(
                &rig.shared,
                "Page.enable",
                json!({}),
                None,
                Duration::from_secs(30),
            ),
        )
        .await;
        assert!(clipped.is_err(), "the outer clip must fire: {clipped:?}");
        assert!(
            rig.shared.pending.lock().unwrap().is_empty(),
            "a dropped call leaked its pending entry"
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

    /// P2 regression: `wait_for` honors its OWN budget while the lazy
    /// `Network.enable` enable is still outstanding. The peer answers the
    /// attach flow (so the CDP session is established) and then holds the
    /// `Network.enable` response forever: pre-fix the unwrapped await paid
    /// `send_raw`'s 30s per-call timeout, so a 100ms wait answered ~300x
    /// late, with a raw CDP error instead of the typed timeout.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wait_for_network_idle_honors_budget_when_enable_is_held() {
        let fake = fake_cdp_withholding(&["Network.enable"]).await;
        let inner = attach(&fake.ws_url).await.expect("attach over ws");
        let b = wrap_attached(inner);
        assert!(
            b.eval("1+1").await.is_ok(),
            "session established before the wait"
        );
        let t0 = Instant::now();
        let r = crate::tools::call("wait_for", &json!({"networkIdle": true, "ms": 100}), &b).await;
        let dt = t0.elapsed();
        assert!(r.get("isError").is_some(), "a held enable is not idle: {r}");
        assert_eq!(
            r.get("structuredContent")
                .and_then(|s| s.get("code"))
                .and_then(|c| c.as_str()),
            Some("timeout"),
            "budget exhaustion must be the typed timeout: {r}"
        );
        assert!(
            dt >= Duration::from_millis(90),
            "budget honored, took {dt:?}"
        );
        assert!(dt < Duration::from_secs(3), "budget honored, took {dt:?}");
        // The clip DROPS the enable future: its pending slot must not outlive
        // it (a peer that never answers would otherwise grow the map per call).
        assert!(
            b.shared()
                .await
                .expect("shared")
                .pending
                .lock()
                .unwrap()
                .is_empty(),
            "clipped enable left a pending entry behind"
        );
        b.close().await.expect("detach close");
        for t in fake.tasks {
            t.abort();
        }
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

    /// Serialize env-touching tests: `GLADIUS_BACKEND_CAPS` / `GLADIUS_CDP_URL`
    /// are process-global, so caps/url tests never run against each other.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct EnvGuard {
        saved: Vec<(String, Option<String>)>,
    }

    impl EnvGuard {
        fn hold(keys: &[&str]) -> (std::sync::MutexGuard<'static, ()>, EnvGuard) {
            let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let mut saved = Vec::new();
            for k in keys {
                saved.push((k.to_string(), std::env::var(k).ok()));
                std::env::remove_var(k);
            }
            (guard, EnvGuard { saved })
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in self.saved.drain(..) {
                match v {
                    Some(val) => std::env::set_var(&k, val),
                    None => std::env::remove_var(&k),
                }
            }
        }
    }

    /// Minimal fake external CDP engine (loopback WS only): answers the
    /// browser-domain calls the attach flow needs, records detach.
    struct FakeCdp {
        ws_url: String,
        detach_seen: Arc<std::sync::atomic::AtomicBool>,
        tasks: Vec<tokio::task::JoinHandle<()>>,
    }

    async fn fake_cdp() -> FakeCdp {
        fake_cdp_withholding(&[]).await
    }

    /// Same fake engine, except every method in `withheld` is answered with
    /// silence (no response frame at all): the caller's per-call timeout is
    /// the only thing that can end that call. Used to hold one specific CDP
    /// call open without breaking the rest of the flow.
    async fn fake_cdp_withholding(withheld: &[&str]) -> FakeCdp {
        let withheld: Vec<String> = withheld.iter().map(|m| m.to_string()).collect();
        let detach_seen = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ln = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback bind");
        let port = ln.local_addr().expect("loopback addr").port();
        let flag = detach_seen.clone();
        let srv = tokio::spawn(async move {
            while let Ok((s, _)) = ln.accept().await {
                let flag = flag.clone();
                let withheld = withheld.clone();
                tokio::spawn(async move {
                    let mut ws = match tokio_tungstenite::accept_async(s).await {
                        Ok(w) => w,
                        Err(_) => return,
                    };
                    while let Some(msg) = ws.next().await {
                        let text = match msg {
                            Ok(Message::Text(t)) => t.to_string(),
                            Ok(Message::Binary(b)) => String::from_utf8_lossy(&b).into_owned(),
                            _ => continue,
                        };
                        let v: Value = match serde_json::from_str(&text) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        let id = match v.get("id").and_then(|i| i.as_u64()) {
                            Some(i) => i,
                            None => continue,
                        };
                        let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("");
                        if withheld.iter().any(|w| w == method) {
                            continue; // held open: never answer
                        }
                        if method == "Target.detachFromTarget" {
                            flag.store(true, Ordering::SeqCst);
                        }
                        let result = match method {
                            "Target.getTargets" => json!({"targetInfos": [
                                {"targetId": "T1", "type": "page",
                                 "url": "about:blank", "title": "fake"}
                            ]}),
                            "Target.attachToTarget" => json!({"sessionId": "S1"}),
                            "Runtime.evaluate" => json!({"result": {"type": "string", "value": "fake"}}),
                            _ => json!({}),
                        };
                        let _ = ws
                            .send(Message::Text(
                                json!({"id": id, "result": result}).to_string().into(),
                            ))
                            .await;
                    }
                });
            }
        });
        FakeCdp {
            ws_url: format!("ws://127.0.0.1:{port}/"),
            detach_seen,
            tasks: vec![srv],
        }
    }

    /// Fake `/json/version` HTTP endpoint pointing at `ws_url` (loopback).
    async fn fake_version_http(ws_url: String) -> (String, tokio::task::JoinHandle<()>) {
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
                let ws_url = ws_url.clone();
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
                    let body = serde_json::json!({
                        "webSocketDebuggerUrl": ws_url,
                        "Browser": "Fake/1.0",
                    })
                    .to_string();
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = s.write_all(resp.as_bytes()).await;
                });
            }
        });
        (format!("http://127.0.0.1:{port}"), h)
    }

    fn wrap_attached(inner: Inner) -> Browser {
        Browser {
            state: AsyncMutex::new(Some(inner)),
            pid: Mutex::new(None),
            refs: AsyncMutex::new(HashMap::new()),
            snap_url: AsyncMutex::new(String::new()),
        }
    }

    /// Spawn a sleeper standing in for the EXTERNAL engine process. Returns
    /// (pid, child): the child is owned by the test, never by gladius.
    async fn external_sleeper() -> (u32, tokio::process::Child) {
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
        let child = Command::from(std_cmd).spawn().expect("spawn external sleeper");
        let pid = child.id().expect("external sleeper pid");
        (pid, child)
    }

    /// Attach over ws:// connects and runs the shared CDP flow.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn attach_over_ws_runs_shared_flow() {
        let fake = fake_cdp().await;
        let inner = attach(&fake.ws_url).await.expect("attach over ws");
        assert!(
            matches!(inner.engine, Engine::Attached { .. }),
            "ws attach must build the attached engine"
        );
        let b = wrap_attached(inner);
        // Shared flow: a session-scoped command attaches (T1/S1) and runs.
        let v = b.eval("1+1").await.expect("eval over attach");
        assert_eq!(v, json!("fake"), "fake engine answers: {v:?}");
        b.close().await.expect("detach close");
        for t in fake.tasks {
            t.abort();
        }
    }

    /// Attach over http:// resolves the ws endpoint via /json/version.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn attach_over_http_resolves_via_json_version() {
        let fake = fake_cdp().await;
        let (base, http) = fake_version_http(fake.ws_url.clone()).await;
        let inner = attach(&base).await.expect("attach over http");
        match &inner.engine {
            Engine::Attached { cdp_url } => assert_eq!(
                cdp_url, &fake.ws_url,
                "http attach must resolve to the version endpoint url"
            ),
            Engine::Owned { .. } => panic!("http attach built the owned engine"),
        }
        let b = wrap_attached(inner);
        let v = b.eval("1+1").await.expect("eval over http attach");
        assert_eq!(v, json!("fake"), "fake engine answers: {v:?}");
        b.close().await.expect("detach close");
        http.abort();
        for t in fake.tasks {
            t.abort();
        }
    }

    /// Ownership on close: an attached close detaches (server sees
    /// detachFromTarget) and leaves the external process alive; it never
    /// deletes anything it did not create (a canary dir planted before
    /// attach must survive the close), and the attached variant owns no pid
    /// by construction.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn attached_close_leaves_external_process_alive() {
        let fake = fake_cdp().await;
        let (ext_pid, mut ext_child) = external_sleeper().await;
        // Canary: a dir only a sweep would remove. Nothing in the
        // attach+close path deletes anything, so survival proves the
        // ownership rule.
        let canary =
            std::env::temp_dir().join(format!("gladius-2147483646-canary-{}", ext_pid));
        std::fs::create_dir_all(&canary).expect("plant canary");
        let inner = attach(&fake.ws_url).await.expect("attach");
        assert!(
            matches!(inner.engine, Engine::Attached { .. }),
            "attach must build the attached engine (which owns no profile by construction)"
        );
        let b = wrap_attached(inner);
        let _ = b.eval("1+1").await.expect("session up");
        b.close().await.expect("detach close");
        assert!(
            fake.detach_seen.load(Ordering::SeqCst),
            "attached close must detach the session we opened"
        );
        // External process still alive: no wait exit.
        match ext_child.try_wait().expect("try_wait external") {
            None => {}
            Some(status) => panic!("external process died on detach: {status}"),
        }
        assert!(
            canary.is_dir(),
            "attached close must never delete anything it did not create"
        );
        std::fs::remove_dir_all(&canary).ok();
        // Test cleanup owns the sleeper: kill it here, never via gladius.
        #[cfg(windows)]
        {
            let _ = ext_child.kill().await;
            let _ = tokio::time::timeout(Duration::from_secs(5), ext_child.wait()).await;
        }
        #[cfg(not(windows))]
        {
            let _ = ext_child.kill().await;
        }
        for t in fake.tasks {
            t.abort();
        }
    }

    /// Capability parsing: only `render` and `dom-only` are accepted.
    #[test]
    fn backend_caps_parse_accepts_only_known_values() {
        assert_eq!(parse_backend_caps("render").unwrap(), BackendCaps::Render);
        assert_eq!(parse_backend_caps("dom-only").unwrap(), BackendCaps::DomOnly);
        assert_eq!(parse_backend_caps("  DOM-ONLY ").unwrap(), BackendCaps::DomOnly);
        assert!(parse_backend_caps("").is_err());
        assert!(parse_backend_caps("bogus").is_err());
    }

    /// Default path selection: with neither env var set the backend is the
    /// owned Chromium launch with render caps.
    #[test]
    fn default_path_is_chromium_render_when_no_env() {
        let (_lock, _env) = EnvGuard::hold(&[
            "GLADIUS_CDP_URL",
            "GLADIUS_BACKEND_CAPS",
            "GLADIUS_CHROMIUM",
            "GLADIUS_CHROMIUM_CACHE",
            "GLADIUS_HEADED",
        ]);
        assert_eq!(cdp_url_override(), None);
        assert_eq!(backend_caps().unwrap(), BackendCaps::Render);
        assert_eq!(backend_name(), "chromium");
        assert!(!headed_requested());
        let info = backend_info();
        assert_eq!(info.get("backend"), Some(&json!("chromium")));
        assert_eq!(info.get("caps"), Some(&json!("render")));
        assert!(info.get("cdpUrl").is_none());
    }

    /// Backend selection: unset -> chromium (the default and only launchable
    /// engine); `GLADIUS_CDP_URL` set -> attach. There is no other knob.
    #[test]
    fn backend_selection_prefers_attach_then_chromium() {
        let (_lock, _env) = EnvGuard::hold(&["GLADIUS_CDP_URL", "GLADIUS_HEADED"]);
        assert_eq!(backend_name(), "chromium");
        std::env::set_var("GLADIUS_CDP_URL", "ws://127.0.0.1:9/");
        assert_eq!(backend_name(), "external-cdp");
        let info = backend_info();
        assert_eq!(info.get("backend"), Some(&json!("external-cdp")));
        assert_eq!(info.get("cdpUrl"), Some(&json!("ws://127.0.0.1:9/")));
    }

    /// Headed requests fail closed: `GLADIUS_HEADED` truthy means the launch
    /// dispatcher refuses with the headless-only error instead of launching
    /// anything. Blank/unset/`0`/`false`/`no` mean headless (no refusal).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[allow(clippy::await_holding_lock)]
    async fn headed_request_fails_closed() {
        let (_lock, _env) = EnvGuard::hold(&["GLADIUS_HEADED", "GLADIUS_CDP_URL"]);
        assert!(!headed_requested());
        for off in ["", "   ", "0", "false", "FALSE", "no", "off"] {
            std::env::set_var("GLADIUS_HEADED", off);
            assert!(!headed_requested(), "must mean headless: {off:?}");
        }
        for on in ["1", "true", "TRUE", "yes"] {
            std::env::set_var("GLADIUS_HEADED", on);
            assert!(headed_requested(), "must request headed: {on:?}");
            match launch().await {
                Ok(_) => panic!("headed launch must fail for {on:?}"),
                Err(e) => assert!(
                    e.contains("headless-only") && e.contains("not supported"),
                    "headed refusal must be explicit, got: {e}"
                ),
            }
        }
    }

    /// Chromium argv: the researched launch contract, each flag exactly
    /// once, `about:blank` last. Bare `--headless` (the shell's selector);
    /// the exact-set match below is the enforcement: any added flag
    /// (a wrong headless selector, a sandbox-off switch, the piped debug
    /// transport, keychain switches) fails it.
    #[test]
    fn chromium_argv_matches_launch_contract_once_and_no_more() {
        let dir = std::env::temp_dir().join("gladius-argv-probe");
        let argv = chromium_argv(19431, &dir);
        assert!(
            argv.contains(&"--headless".to_string()),
            "bare --headless: {argv:?}"
        );
        assert_eq!(
            argv.last().map(|s| s.as_str()),
            Some("about:blank"),
            "about:blank last: {argv:?}"
        );
        for flag in [
            "--headless",
            "--no-first-run",
            "--no-default-browser-check",
        ] {
            let n = argv.iter().filter(|a| a.as_str() == flag).count();
            assert_eq!(n, 1, "flag {flag} must appear exactly once: {argv:?}");
        }
        let port_flag = "--remote-debugging-port=19431".to_string();
        assert_eq!(
            argv.iter().filter(|a| *a == &port_flag).count(),
            1,
            "explicit debug port once: {argv:?}"
        );
        let profile_flag = format!("--user-data-dir={}", dir.display());
        assert_eq!(
            argv.iter().filter(|a| *a == &profile_flag).count(),
            1,
            "private profile once: {argv:?}"
        );
        let origin_flag = "--remote-allow-origins=http://127.0.0.1:19431".to_string();
        assert_eq!(
            argv.iter().filter(|a| *a == &origin_flag).count(),
            1,
            "explicit loopback origin once: {argv:?}"
        );
        // No other flag-like token: exactly the known set.
        let mut flags: Vec<String> = argv
            .iter()
            .filter(|a| a.starts_with("--"))
            .cloned()
            .collect();
        flags.sort_unstable();
        let mut want = vec![
            "--headless".to_string(),
            "--no-default-browser-check".to_string(),
            "--no-first-run".to_string(),
            "--remote-allow-origins=http://127.0.0.1:19431".to_string(),
            "--remote-debugging-port=19431".to_string(),
            profile_flag,
        ];
        want.sort_unstable();
        assert_eq!(flags, want, "no extra or duplicated flags: {argv:?}");
        // Full-assertion companion: the shell's headless selector is bare,
        // never a suffixed spelling.
        for a in &argv {
            if a.starts_with("--headless") {
                assert_eq!(a, "--headless", "bare headless selector only: {argv:?}");
            }
        }
    }

    /// Chromium resolution order: explicit `GLADIUS_CHROMIUM` wins when it
    /// is a file; else the pinned cache path (honouring `GLADIUS_CHROMIUM_CACHE`
    /// as the root); else a clear error naming `scripts/fetch-chromium.ps1`
    /// that never falls back to any ambient, shared, or system browser.
    #[test]
    fn chromium_resolution_order_and_missing_error() {
        let (_lock, _env) = EnvGuard::hold(&[
            "GLADIUS_CHROMIUM",
            "GLADIUS_CHROMIUM_CACHE",
            "LOCALAPPDATA",
        ]);
        // Explicit override pointing nowhere: names the script.
        std::env::set_var(
            "GLADIUS_CHROMIUM",
            r"C:\no-such-gladius-dir-xyz\chrome-headless-shell.exe",
        );
        let e = find_chromium().expect_err("missing engine must fail");
        assert!(
            e.contains("scripts/fetch-chromium.ps1"),
            "names the provision script, got: {e}"
        );
        assert!(
            e.contains("GLADIUS_CHROMIUM"),
            "names the override, got: {e}"
        );
        // Explicit override pointing at a real file wins (this test file).
        let here = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        std::env::set_var("GLADIUS_CHROMIUM", &here);
        assert_eq!(find_chromium().expect("explicit file must win"), here);
        // Blank override + empty cache root + missing cache entry: the error
        // names the script and never resolves anything ambient.
        std::env::set_var("GLADIUS_CHROMIUM", "   ");
        let empty_root = std::env::temp_dir().join("gladius-no-such-cache-root-xyz");
        std::env::set_var("GLADIUS_CHROMIUM_CACHE", &empty_root);
        let e2 = find_chromium().expect_err("missing cache entry must fail");
        assert!(
            e2.contains("scripts/fetch-chromium.ps1"),
            "cache miss names the script, got: {e2}"
        );
        assert!(
            e2.contains(PINNED_CHROMIUM_VERSION),
            "cache miss names the pin, got: {e2}"
        );
        // Cache-root override is honoured: plant the exact pinned layout
        // under a scratch root and it resolves.
        let scratch = std::env::temp_dir().join(format!(
            "gladius-chromium-cache-probe-{}",
            std::process::id()
        ));
        let exe = scratch
            .join(format!("{PINNED_CHROMIUM_VERSION}-{PINNED_ZIP_SHORT8}"))
            .join(CHROMIUM_EXE_REL.replace('/', std::path::MAIN_SEPARATOR_STR));
        std::fs::create_dir_all(exe.parent().expect("exe parent")).expect("plant cache");
        std::fs::write(&exe, b"probe").expect("plant exe");
        std::env::set_var("GLADIUS_CHROMIUM_CACHE", &scratch);
        assert_eq!(find_chromium().expect("planted cache must win"), exe);
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// dom-only refusal: screenshot errors naming the backend and the
    /// reason, before any engine launch (no browser needed to prove it).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[allow(clippy::await_holding_lock)]
    async fn dom_only_screenshot_refuses_without_image() {
        let (_lock, _env) = EnvGuard::hold(&["GLADIUS_BACKEND_CAPS"]);
        std::env::set_var("GLADIUS_BACKEND_CAPS", "dom-only");
        let b = Browser::new();
        let r = crate::tools::call("screenshot", &json!({}), &b).await;
        assert!(
            r.get("isError").is_some(),
            "dom-only screenshot must be a tool error: {r}"
        );
        let msg = r
            .get("content")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("");
        assert!(
            msg.contains("dom-only"),
            "refusal names the backend, got: {msg}"
        );
        assert!(
            msg.contains("no rendering pipeline"),
            "refusal names the reason, got: {msg}"
        );
        assert!(
            !msg.contains("iVBORw0KGgo"),
            "refusal must never carry image bytes"
        );
        // No engine was launched proving it: state stays empty.
        assert!(
            b.snap_url().await.is_empty(),
            "refusal must not touch the browser"
        );
    }

    /// Owned-endpoint confinement table, through the SAME `owned_ws_request`
    /// production dials (parse with the connector's parser, confine
    /// `request.uri()`, hand the request to `connect_async`). The attach
    /// path (`resolve_ws_url`) is deliberately NOT confined - it must keep
    /// accepting whatever the user pointed at.
    #[test]
    fn owned_ws_request_accepts_only_our_loopback_port() {
        for url in [
            "ws://127.0.0.1:5555/devtools/browser/x",
            "ws://127.0.0.1:5555/",
            "ws://localhost:5555/devtools/browser/x",
            "ws://LOCALHOST:5555/devtools/browser/x",
            "ws://[::1]:5555/devtools/browser/x",
            "wss://127.0.0.1:5555/devtools/browser/x",
        ] {
            owned_ws_request(5555, url).unwrap_or_else(|e| panic!("must accept {url:?}: {e}"));
        }
        for url in [
            "ws://127.0.0.1:5556/devtools/browser/x",    // wrong port
            "ws://192.168.1.10:5555/devtools/browser/x", // foreign IP
            "ws://192.0.2.1:9000/",                      // TEST-NET-1 squatter shape
            "http://127.0.0.1:5555/devtools/browser/x",  // non-ws scheme
            "https://127.0.0.1:5555/x",
            "ws://127.0.0.1.evil.com:5555/x", // suffix smuggle
            "ws://localhost.evil.com:5555/x",
            "ws://localhost.:5555/x", // trailing dot
            "ws://127.0.0.1.:5555/x",
            "ws://127.0.0.1/x", // missing port
            "ws://localhost/x",
        ] {
            assert!(owned_ws_request(5555, url).is_err(), "must reject {url:?}");
        }
    }

    /// Bypass regressions: the old hand-rolled parser ended the authority at
    /// `/` only and took text after the last `@` as the host, so BOTH of
    /// these were accepted for owned port 5555 while the CONNECTOR (which
    /// ends the authority at the first `/`, `?`, or `#`) dials
    /// `192.0.2.1:9000`. Validated through the same `owned_ws_request`
    /// production dials: the connector parses identically because it reads
    /// the same `request.uri()`.
    #[test]
    fn owned_ws_request_rejects_query_fragment_host_smuggle() {
        let q = "ws://192.0.2.1:9000?@127.0.0.1:5555/";
        let q_err = owned_ws_request(5555, q).expect_err("query smuggle must be rejected");
        eprintln!("owned confinement rejects query-smuggle url {q}: {q_err}");
        let f = "ws://192.0.2.1:9000#@127.0.0.1:5555/";
        let f_err = owned_ws_request(5555, f).expect_err("fragment smuggle must be rejected");
        eprintln!("owned confinement rejects fragment-smuggle url {f}: {f_err}");
    }

    /// Budget helper through the SAME `startup_remaining` production uses: a
    /// fresh start yields the remaining budget, an expired start yields
    /// `None` (the timeout error), and the boundary is already expired - a
    /// handshake completing AT expiry is late (Tokio's `timeout` polls the
    /// inner future first, so completion at expiry would otherwise read as
    /// success).
    #[test]
    fn startup_remaining_enforces_the_deadline() {
        let deadline = Duration::from_millis(200);
        let got = startup_remaining(Instant::now(), deadline).expect("fresh start has budget");
        assert!(
            !got.is_zero() && got <= deadline,
            "fresh start must yield a live budget, got {got:?}"
        );
        assert!(
            startup_remaining(Instant::now() - Duration::from_secs(60), deadline).is_none(),
            "a long-past start is expired"
        );
        assert!(
            startup_remaining(Instant::now(), Duration::ZERO).is_none(),
            "a zero budget is immediately expired"
        );
        // Boundary: started exactly one deadline ago means elapsed >=
        // deadline (elapsed only grows from here), so this is expired.
        assert!(
            startup_remaining(Instant::now() - deadline, deadline).is_none(),
            "completion AT expiry is late"
        );
    }

    /// Composition: squatter `/json/version` endpoints resolve over a REAL
    /// local listener and are then rejected by the SAME `owned_ws_request`
    /// production dials. Covers the plain foreign endpoint AND the crafted
    /// query/fragment smuggle shapes (which carry the owned port after the
    /// `@`, exactly what the old parser accepted). Bounded (the fake
    /// answers immediately on loopback) and local only.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn owned_resolve_confine_rejects_squatter_endpoint() {
        let (base, http) = fake_version_http("ws://192.0.2.1:9000/".to_string()).await;
        let got = resolve_ws_url(&base).await.expect("squatter resolves");
        assert_eq!(got, "ws://192.0.2.1:9000/");
        let port: u16 = base
            .rsplit(':')
            .next()
            .expect("base has a port")
            .parse()
            .expect("base port parses");
        let r = owned_ws_request(port, &got);
        assert!(r.is_err(), "squatter endpoint must be confined, got: {got}");
        http.abort();
        // Crafted shapes: the smuggled port IS the owned port under test
        // (any fixed port works - the check compares against it).
        let owned: u16 = 5555;
        for crafted in [
            format!("ws://192.0.2.1:9000?@127.0.0.1:{owned}/"),
            format!("ws://192.0.2.1:9000#@127.0.0.1:{owned}/"),
        ] {
            let (base_c, http_c) = fake_version_http(crafted.clone()).await;
            let got_c = resolve_ws_url(&base_c)
                .await
                .expect("smuggle shape resolves");
            assert_eq!(got_c, crafted, "resolve must pass the URL through");
            let err = owned_ws_request(owned, &got_c).expect_err("smuggle shape must be confined");
            eprintln!("owned resolve+confine rejects squatter url {got_c}: {err}");
            http_c.abort();
        }
    }

    /// Owned-launch retry (a): two early-child exits then success → the
    /// launch succeeds on attempt 3 and the attempt closure ran 3 times.
    /// Scripted closures through the SAME `retry_attempts` production uses.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn launch_retry_succeeds_on_third_attempt() {
        let calls = Arc::new(AtomicUsize::new(0));
        let r: Result<&str, String> = retry_attempts(
            OWNED_LAUNCH_ATTEMPTS,
            |e: &String| launch_startup_exit_is_retryable(e),
            || {
                let calls = calls.clone();
                async move {
                    let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
                    if n < OWNED_LAUNCH_ATTEMPTS {
                        Err(format!(
                            "chromium child exited during startup: exit code: {n}"
                        ))
                    } else {
                        Ok("ready")
                    }
                }
            },
        )
        .await;
        assert_eq!(r, Ok("ready"));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            OWNED_LAUNCH_ATTEMPTS,
            "two failures then success must run exactly 3 attempts"
        );
    }

    /// Owned-launch retry (b): a NON-retryable error (the squatter
    /// confinement rejection) surfaces immediately — the closure runs once,
    /// no port re-roll. The predicate table asserts the boundary directly:
    /// only the early-child-exit signal retries; confinement rejections,
    /// unknown child state (`try_wait` failures), and timeouts never do.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn launch_retry_non_retryable_surfaces_immediately() {
        assert!(launch_startup_exit_is_retryable(
            "chromium child exited during startup: exit code: 1"
        ));
        for fatal in [
            "owned ws endpoint rejected: port 2 != owned port 1 in ws://127.0.0.1:2/",
            "chromium child wait failed during startup: unknown state",
            "timeout waiting for chromium /json/version on http://127.0.0.1:1: \
             deadline passed with no ready endpoint",
        ] {
            assert!(
                !launch_startup_exit_is_retryable(fatal),
                "must never retry: {fatal}"
            );
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let r: Result<&str, String> = retry_attempts(
            OWNED_LAUNCH_ATTEMPTS,
            |e: &String| launch_startup_exit_is_retryable(e),
            || {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err("owned ws endpoint rejected: port 2 != owned port 1 in \
                         ws://127.0.0.1:2/"
                        .to_string())
                }
            },
        )
        .await;
        assert!(r.is_err(), "confinement rejection must surface");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "a non-retryable error must not re-run the attempt"
        );
    }

    /// Owned-launch retry (c): exhausting the bound surfaces the LAST error
    /// unchanged — exactly what the final attempt reported, so nothing is
    /// hidden.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn launch_retry_exhaustion_surfaces_last_error() {
        let calls = Arc::new(AtomicUsize::new(0));
        let r: Result<&str, String> = retry_attempts(
            OWNED_LAUNCH_ATTEMPTS,
            |e: &String| launch_startup_exit_is_retryable(e),
            || {
                let calls = calls.clone();
                async move {
                    let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
                    Err(format!(
                        "chromium child exited during startup: exit code: {n}"
                    ))
                }
            },
        )
        .await;
        assert_eq!(
            r,
            Err(format!(
                "chromium child exited during startup: exit code: {OWNED_LAUNCH_ATTEMPTS}"
            ))
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            OWNED_LAUNCH_ATTEMPTS,
            "an always-retryable failure must run exactly the bound"
        );
    }
}
