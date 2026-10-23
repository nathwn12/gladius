//! Deterministic pixel capture.
//!
//! One screenshot taken under conditions that are fully pinned by the engine
//! instead of by whatever the page happened to be doing:
//!
//! - `Emulation.setDeviceMetricsOverride` fixes the viewport and forces
//!   `deviceScaleFactor: 1` / `mobile: false` (restored afterwards: the prior
//!   override is replayed, or cleared when there was none);
//! - `Emulation.setEmulatedMedia` reports `prefers-reduced-motion: reduce`;
//! - a temporary style element freezes `animation`, `transition`,
//!   `caret-color`, smooth scrolling and scrollbar gutter/width;
//! - the page is settled before the shot: `document.fonts.ready`, every `img`
//!   `complete` (or errored), then two `requestAnimationFrame` ticks, each
//!   wait raced against a timer so a background tab that starves rAF still
//!   settles instead of hanging;
//! - `Page.captureScreenshot` with `format: "png"`, `fromSurface: true` and
//!   `captureBeyondViewport` for full-page shots.
//!
//! Everything rides CDP: no OS input, no window, no foreground tab.
//! The checksum is FNV-1a 64 over the decoded PNG bytes - cheap, dependency
//! free, and enough to prove two captures are byte-identical.

use base64::Engine as _;
use serde_json::{json, Value};

use crate::browser::Browser;

/// Backstop budget for the settle evaluate. The page races its own waits
/// against timers, so this only fires for a genuinely stuck renderer.
const SETTLE_CAP: std::time::Duration = std::time::Duration::from_secs(5);

/// Temporary freeze style. Injected with a stable id so removal is exact.
const STYLE_ID: &str = "__gladius_deterministic";

fn style_js() -> String {
    format!(
        "(() => {{ const id = '{STYLE_ID}'; let el = document.getElementById(id); \
         if (!el) {{ el = document.createElement('style'); el.id = id; \
         (document.head || document.documentElement).appendChild(el); }} \
         el.textContent = '{CSS}'; \
         return true; }})()"
    )
}

fn style_restore_js() -> String {
    format!(
        "(() => {{ const el = document.getElementById('{STYLE_ID}'); if (el) el.remove(); \
         return document.getElementById('{STYLE_ID}') === null; }})()"
    )
}

/// The freeze rules: animation/transition stopped (delay and duration
/// pinned to 0 so a running one cannot resume), caret hidden, smooth
/// scrolling and overflow anchoring off, scrollbar gutter forced to auto and
/// the scrollbar itself collapsed to zero so its presence cannot change the
/// content box between runs.
const CSS: &str = "*,*::before,*::after{animation:none!important;animation-delay:0s!important;animation-duration:0s!important;animation-iteration-count:1!important;transition:none!important;transition-delay:0s!important;transition-duration:0s!important;caret-color:transparent!important;scroll-behavior:auto!important;overflow-anchor:none!important;}html,body{scrollbar-gutter:auto!important;}::-webkit-scrollbar{width:0!important;height:0!important;display:none!important;}";

/// Fonts + images + two animation frames, every wait bounded.
const SETTLE_JS: &str = r#"(() => {
  const done = (fn) => new Promise((res) => { fn(() => res(true)); });
  return (async () => {
    try { await Promise.race([document.fonts.ready, done((r) => setTimeout(r, 3000))]); } catch (e) {}
    const imgs = Array.prototype.slice.call(document.images || []);
    await Promise.all(imgs.map((i) => {
      if (i.complete) return Promise.resolve(true);
      return Promise.race([
        done((r) => { i.addEventListener('load', r, {once: true}); i.addEventListener('error', r, {once: true}); }),
        done((r) => setTimeout(r, 3000))
      ]);
    }));
    const tick = () => Promise.race([done((r) => requestAnimationFrame(() => r(true))), done((r) => setTimeout(r, 250))]);
    await tick();
    await tick();
    return true;
  })();
})()"#;

/// Live `(innerWidth, innerHeight, devicePixelRatio, href)` for the capture
/// metadata. Every value is read back from the page, never assumed.
async fn live_metrics(browser: &Browser) -> Value {
    browser
        .eval("[window.innerWidth, window.innerHeight, window.devicePixelRatio, location.href]")
        .await
        .unwrap_or(Value::Null)
}

/// One capture, either plain (the historic behaviour) or deterministic.
pub struct Options {
    pub full_page: bool,
    pub deterministic: bool,
    pub width: Option<i64>,
    pub height: Option<i64>,
}

/// A decoded capture plus the metadata that makes it auditable.
pub struct Capture {
    pub base64: String,
    pub bytes: Vec<u8>,
    pub checksum: String,
    pub width: i64,
    pub height: i64,
    pub dsf: f64,
    pub url: String,
    pub deterministic: bool,
}

impl Capture {
    /// Deterministic metadata block (view n/a when `deterministic` is false).
    pub fn meta(&self) -> Value {
        let mut m = serde_json::Map::new();
        m.insert("mimeType".to_string(), json!("image/png"));
        m.insert("bytes".to_string(), json!(self.bytes.len() as i64));
        if self.deterministic {
            m.insert("deterministic".to_string(), json!(true));
            m.insert(
                "viewport".to_string(),
                json!({"width": self.width, "height": self.height}),
            );
            m.insert("deviceScaleFactor".to_string(), json!(self.dsf));
            m.insert("url".to_string(), json!(self.url));
            m.insert("checksum".to_string(), json!(self.checksum));
        }
        Value::Object(m)
    }
}

/// One stderr line for a non-fatal capture problem. Restoration problems are
/// never swallowed: the capture still succeeds (the pixels are real), but the
/// operator can see that a page may be left with a stale override.
pub(crate) fn warn(msg: &str) {
    eprintln!("[gladius] warn: {msg}");
}

/// Test-only hook: hold the critical section open for this many milliseconds
/// after the overrides are applied (and, in `visual::compare`, after the
/// scratch target exists). It exists so the cancellation tests can cancel
/// deterministically in the middle of the mutation window instead of racing a
/// fast capture. Never set in production; an unparseable value is ignored.
pub(crate) fn test_delay_ms() -> Option<u64> {
    std::env::var("GLADIUS_TEST_CRITICAL_DELAY_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|ms| *ms > 0)
}

/// Apply the test-only critical-section delay when the hook is set.
pub(crate) async fn maybe_test_delay() {
    if let Some(ms) = test_delay_ms() {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
    }
}

/// FNV-1a 64 over `bytes`, 16 hex digits. Cheap content fingerprint: two
/// captures of the same pixels produce the same PNG bytes and the same
/// checksum, and any pixel change breaks it.
pub fn checksum(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn plain_params(full_page: bool) -> Value {
    if full_page {
        json!({"format": "png", "captureBeyondViewport": true, "fromSurface": true})
    } else {
        json!({"format": "png", "fromSurface": true})
    }
}

fn decode(r: &Value, deterministic: bool) -> Result<Capture, String> {
    let data = r
        .get("data")
        .and_then(|v| v.as_str())
        .ok_or("screenshot: no data returned".to_string())?;
    // Decoded with the `base64` crate (never hand-rolled).
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(|e| format!("screenshot: capture payload is not valid base64: {e}"))?;
    let sum = checksum(&bytes);
    Ok(Capture {
        base64: data.to_string(),
        bytes,
        checksum: sum,
        width: 0,
        height: 0,
        dsf: 1.0,
        url: String::new(),
        deterministic,
    })
}

/// Capture the active page. `deterministic: true` pins the conditions above
/// and restores every override before returning (best effort: a restore
/// failure is never reported as a capture failure - it is LOGGED, because a
/// silent swallow would hide a page left with a stale override, and the next
/// read-back shows the real state rather than a claim).
///
/// The whole capture runs under the connection's capture lock: the
/// deterministic path mutates page-visible state (metrics override, emulated
/// media, an injected stylesheet) and restoration must not interleave with
/// another capture's mutations.
pub async fn capture(browser: &Browser, opts: &Options) -> Result<Capture, String> {
    let _serial = browser
        .capture_lock()
        .await
        .map_err(|e| format!("screenshot: {e}"))?;
    if !opts.deterministic {
        let r = browser
            .command("Page.captureScreenshot", plain_params(opts.full_page), true)
            .await?;
        return decode(&r, false);
    }

    // Prior override we must put back. Read BEFORE we overwrite anything.
    let prior = browser.viewport_override().await;
    let (vw, vh, _) = browser
        .viewport_readback()
        .await
        .unwrap_or((1280, 720, 1.0));
    let width = opts.width.unwrap_or(vw).max(1);
    let height = opts.height.unwrap_or(vh).max(1);
    browser
        .command(
            "Emulation.setDeviceMetricsOverride",
            json!({"width": width, "height": height, "deviceScaleFactor": 1, "mobile": false}),
            true,
        )
        .await?;
    if let Err(e) = browser
        .command(
            "Emulation.setEmulatedMedia",
            json!({"features": [{"name": "prefers-reduced-motion", "value": "reduce"}]}),
            true,
        )
        .await
    {
        warn(&format!(
            "deterministic capture: setEmulatedMedia failed: {e}"
        ));
    }
    if let Err(e) = browser.eval(&style_js()).await {
        warn(&format!("deterministic capture: freeze style failed: {e}"));
    }
    // Test-only: widen the mutation window so a cancellation test can land
    // inside it deterministically.
    maybe_test_delay().await;
    if tokio::time::timeout(SETTLE_CAP, browser.eval(SETTLE_JS))
        .await
        .is_err()
    {
        warn("deterministic capture: settle exceeded its budget (proceeding with the shot)");
    }
    let live = live_metrics(browser).await;
    let shot = browser
        .command("Page.captureScreenshot", plain_params(opts.full_page), true)
        .await;

    // Restore on every path, capture success or not: drop the freeze style,
    // clear the media emulation, then replay the prior metrics override (or
    // clear it when there was none). Each failure is logged, never swallowed.
    if let Err(e) = browser.eval(&style_restore_js()).await {
        warn(&format!(
            "deterministic capture: could not remove the freeze stylesheet: {e}"
        ));
    }
    if let Err(e) = browser
        .command("Emulation.setEmulatedMedia", json!({"features": []}), true)
        .await
    {
        warn(&format!(
            "deterministic capture: could not clear emulated media: {e}"
        ));
    }
    match prior {
        Some(p) => {
            if let Err(e) = browser
                .command("Emulation.setDeviceMetricsOverride", p, true)
                .await
            {
                warn(&format!(
                    "deterministic capture: could not restore the prior viewport override: {e}"
                ));
            }
        }
        None => {
            if let Err(e) = browser
                .command("Emulation.clearDeviceMetricsOverride", json!({}), true)
                .await
            {
                warn(&format!(
                    "deterministic capture: could not clear the metrics override: {e}"
                ));
            }
        }
    }

    let mut cap = decode(&shot?, true)?;
    let arr = live.as_array().cloned().unwrap_or_default();
    let num = |i: usize| arr.get(i).and_then(|v| v.as_f64());
    cap.width = num(0).map(|v| v as i64).unwrap_or(width);
    cap.height = num(1).map(|v| v as i64).unwrap_or(height);
    cap.dsf = num(2).unwrap_or(1.0);
    cap.url = arr
        .get(3)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(cap)
}
