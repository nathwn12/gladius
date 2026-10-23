//! Deterministic pixel capture.
//!
//! One screenshot taken under conditions that are fully pinned by the engine
//! instead of by whatever the page happened to be doing:
//!
//! - `Emulation.setDeviceMetricsOverride` fixes the viewport and forces
//!   `deviceScaleFactor: 1` / `mobile: false` (restored afterwards: the prior
//!   override is replayed, or cleared when there was none);
//! - `Emulation.setEmulatedMedia` reports `prefers-reduced-motion: reduce`
//!   while the capture runs and is cleared afterwards. The clear returns the
//!   page to the ENGINE/HOST default, not to `false`: a host that asks for
//!   reduced motion (CI runners disable animations) reports `reduce` again
//!   after the clear. The invariant is therefore "unchanged from the
//!   pre-capture value", the reply reports whether the emulation was applied
//!   at all (`reducedMotionEmulated`), and the freeze stylesheet below is what
//!   stops CSS animation on every engine;
//! - a temporary style element freezes `animation`, `transition`,
//!   `caret-color`, smooth scrolling and scrollbar gutter/width;
//! - the page is settled before the shot: `document.fonts.ready`, every `img`
//!   `complete` (or errored), then two `requestAnimationFrame` ticks, each
//!   wait raced against a timer so a background tab that starves rAF still
//!   settles instead of hanging;
//! - `Page.captureScreenshot` with `format: "png"`, `fromSurface: true` and
//!   `captureBeyondViewport` for full-page shots.
//!
//! Restoration is CONFIRMED, not assumed. The overrides are reverted with
//! awaited CDP commands, but a reply only says the agent accepted them: the
//! page's media-query and viewport state are refreshed on the renderer's own
//! schedule, and a clear can be lost outright. The capture therefore polls the
//! page (bounded, 2 s cap) until it reports the pre-capture viewport/dsf, no
//! freeze stylesheet and no emulated reduced-motion, and re-applies the
//! restore once if it does not - so a caller that reads the page immediately
//! after this call can never observe the emulated state. That gap is what the
//! release gate hit with `matchMedia('(prefers-reduced-motion: reduce)')`.
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
    /// True when this engine actually emulated `prefers-reduced-motion`
    /// (`Emulation.setEmulatedMedia` implemented). False on the pinned
    /// `chrome-headless-shell`, where the preference stays the host's and only
    /// the freeze stylesheet stops motion.
    pub reduced_motion_emulated: bool,
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
            m.insert(
                "reducedMotionEmulated".to_string(),
                json!(self.reduced_motion_emulated),
            );
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

/// Does this engine implement `Emulation.setEmulatedMedia`? The pinned
/// `chrome-headless-shell` 155 DOES, on a PAGE session (on the browser-level
/// connection the same method answers `-32601`, which is why a capability
/// probe must go through a session - measured on the pin). This cache only
/// guards an engine that genuinely lacks it: the first definitive answer is
/// cached (0 = unknown, 1 = supported, 2 = unsupported).
static MEDIA_EMU: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn media_emu_known() -> Option<bool> {
    use std::sync::atomic::Ordering;
    match MEDIA_EMU.load(Ordering::SeqCst) {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

fn media_emu_remember(supported: bool) {
    use std::sync::atomic::Ordering;
    MEDIA_EMU.store(if supported { 1 } else { 2 }, Ordering::SeqCst);
}

/// "This engine has no such command" - the CDP error for a method that is not
/// part of the build's protocol surface.
fn method_missing(err: &str) -> bool {
    err.contains("-32601") || err.contains("wasn't found") || err.contains("was not found")
}

/// The explicit reset: empty media type plus an empty feature list. On an
/// engine that implements the command this removes the emulation entirely
/// (back to the engine/host default); it is never a "set no-preference"
/// override, which would *change* a host that prefers reduced motion.
fn clear_media_params() -> Value {
    json!({"media": "", "features": []})
}

/// How long the restore may take to become visible to the PAGE. Bounded: a
/// wedged renderer must not hang a capture (the pixels are already valid, so
/// the confirmation is warned about, never fatal).
const RESTORE_CONFIRM_CAP: std::time::Duration = std::time::Duration::from_secs(2);

/// Cadence of the confirmation probe. Small: normally the very first probe
/// already agrees (the clear is applied by the time the reply lands), so the
/// steady-state cost is one `Runtime.evaluate`.
const RESTORE_CONFIRM_STEP: std::time::Duration = std::time::Duration::from_millis(10);

/// Test-only hook (never set in production): drop the first N media clears,
/// simulating a lost clear (the symptom the release gate hit). The restore
/// confirmation is what repairs it, and the counter lets the retry succeed.
fn test_drop_media_clears() -> u32 {
    std::env::var("GLADIUS_TEST_DROP_MEDIA_CLEAR")
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(0)
}

/// Consume one "drop the clear" credit, if any are left. Re-armed from the
/// environment whenever it runs dry, so a harness may set the hook at any
/// point in the process (the selftest does) and each capture gets its own
/// drop.
fn take_dropped_clear() -> bool {
    use std::sync::atomic::{AtomicU32, Ordering};
    static REMAINING: AtomicU32 = AtomicU32::new(0);
    if REMAINING.load(Ordering::SeqCst) == 0 {
        let want = test_drop_media_clears();
        if want == 0 {
            return false;
        }
        REMAINING.store(want, Ordering::SeqCst);
    }
    REMAINING
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
        .is_ok()
}

/// Page-level view of the restored state: metrics, freeze stylesheet gone and
/// the motion preference back to what the page reported BEFORE the capture.
/// That last value is deliberately the baseline, not `false`: the clear
/// returns the page to the engine/host default, and a host that asks for
/// reduced motion (a CI runner with animations disabled reports `reduce`)
/// reports it again afterwards - so `false` would assert something the capture
/// neither set nor owns.
///
/// This is the ONLY proof that restoration is complete - a CDP reply says the
/// agent accepted the command, not that the page has applied it.
fn restore_probe_js() -> String {
    format!(
        "[window.innerWidth, window.innerHeight, window.devicePixelRatio, \
          document.getElementById('{STYLE_ID}') === null, \
          matchMedia('(prefers-reduced-motion: reduce)').matches]"
    )
}

/// Poll the page (bounded) until it reports the pre-capture state: the
/// viewport/dsf we read before overriding, no freeze stylesheet, and no
/// emulated reduced-motion. Returns true when the page agrees.
async fn confirm_restored(browser: &Browser, expect: (i64, i64, f64, bool)) -> bool {
    let (ew, eh, edpr, e_reduce) = expect;
    let expr = restore_probe_js();
    let deadline = std::time::Instant::now() + RESTORE_CONFIRM_CAP;
    loop {
        if let Ok(v) = browser.eval(&expr).await {
            let a = v.as_array().cloned().unwrap_or_default();
            let num = |i: usize| a.get(i).and_then(|x| x.as_f64());
            let style_gone = a.get(3).and_then(|x| x.as_bool()) == Some(true);
            // The preference must be back to the BASELINE (see the doc above).
            let reduced_motion_restored = a.get(4).and_then(|x| x.as_bool()) == Some(e_reduce);
            let size_ok = num(0) == Some(ew as f64) && num(1) == Some(eh as f64);
            let dpr_ok = num(2).map(|d| (d - edpr).abs() < 0.001).unwrap_or(false);
            if size_ok && dpr_ok && style_gone && reduced_motion_restored {
                return true;
            }
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(RESTORE_CONFIRM_STEP).await;
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
        reduced_motion_emulated: false,
    })
}

/// Replay the prior device-metrics override (or clear it when there was
/// none). Used by the restore and by its retry, so both paths are identical.
async fn restore_metrics(browser: &Browser, prior: Option<&Value>) {
    match prior {
        Some(p) => {
            if let Err(e) = browser
                .command("Emulation.setDeviceMetricsOverride", p.clone(), true)
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
    // The pre-capture read-back is also the EXPECTATION the restore must
    // confirm later: metrics, dsf AND the motion preference the page reports
    // before we touch it (which on an engine without media emulation is the
    // host's, not ours).
    let (vw, vh, vdpr) = browser
        .viewport_readback()
        .await
        .unwrap_or((1280, 720, 1.0));
    let base_reduce = browser
        .eval("matchMedia('(prefers-reduced-motion: reduce)').matches")
        .await
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let width = opts.width.unwrap_or(vw).max(1);
    let height = opts.height.unwrap_or(vh).max(1);
    browser
        .command(
            "Emulation.setDeviceMetricsOverride",
            json!({"width": width, "height": height, "deviceScaleFactor": 1, "mobile": false}),
            true,
        )
        .await?;
    // Emulate reduced motion ONLY where the engine implements the command.
    // chrome-headless-shell 155 has no `Emulation.setEmulatedMedia` at all, so
    // the preference stays the host's (reported as `reducedMotionEmulated:
    // false`); the freeze stylesheet below still stops CSS animation. The first
    // attempt doubles as the capability probe.
    let mut media_emulated = media_emu_known().unwrap_or(true);
    if media_emulated {
        match browser
            .command(
                "Emulation.setEmulatedMedia",
                json!({"features": [{"name": "prefers-reduced-motion", "value": "reduce"}]}),
                true,
            )
            .await
        {
            Ok(_) => media_emu_remember(true),
            Err(e) if method_missing(&e) => {
                media_emu_remember(false);
                media_emulated = false;
                warn(
                    "deterministic capture: this engine has no Emulation.setEmulatedMedia; \
                     prefers-reduced-motion is not emulated - the freeze stylesheet still \
                     stops CSS animation",
                );
            }
            Err(e) => warn(&format!(
                "deterministic capture: setEmulatedMedia failed: {e} (preference left as-is)"
            )),
        }
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
    if media_emulated {
        if take_dropped_clear() {
            warn("test hook: dropped the media clear (GLADIUS_TEST_DROP_MEDIA_CLEAR)");
        } else if let Err(e) = browser
            .command("Emulation.setEmulatedMedia", clear_media_params(), true)
            .await
        {
            warn(&format!(
                "deterministic capture: could not clear emulated media: {e}"
            ));
        }
    }
    restore_metrics(browser, prior.as_ref()).await;

    // A CDP reply only says the agent accepted the command. The page's media
    // query and viewport state are refreshed on the renderer's own schedule,
    // and a clear can be lost outright (session churn), so a caller that reads
    // the page immediately after this call can still see the emulated state -
    // exactly the release-gate failure. Restoration therefore is not complete
    // until the PAGE reports it: confirm (bounded), and repair once if the
    // clear did not land.
    let expectation = (vw, vh, vdpr, base_reduce);
    if !confirm_restored(browser, expectation).await {
        warn(
            "deterministic capture: page did not confirm the restored state within 2s; \
             re-applying the restore once",
        );
        if let Err(e) = browser.eval(&style_restore_js()).await {
            warn(&format!(
                "deterministic capture: retry could not remove the freeze stylesheet: {e}"
            ));
        }
        if media_emulated {
            if let Err(e) = browser
                .command("Emulation.setEmulatedMedia", clear_media_params(), true)
                .await
            {
                warn(&format!(
                    "deterministic capture: retry could not clear emulated media: {e}"
                ));
            }
        }
        restore_metrics(browser, prior.as_ref()).await;
        if !confirm_restored(browser, expectation).await {
            warn(
                "deterministic capture: page still reports the emulated state after a retry \
                 (the next read-back shows the truth)",
            );
        }
    }

    let mut cap = decode(&shot?, true)?;
    cap.reduced_motion_emulated = media_emulated;
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
