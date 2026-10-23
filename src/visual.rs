//! Visual baselines: `save` / `compare` / `list` / `delete` over deterministic
//! PNG captures stored under `GLADIUS_VISUAL_DIR` (default
//! `<cwd>/.gladius/visual`).
//!
//! ## Why the diff runs inside a headless target
//!
//! Comparing two PNGs needs a decoder, and this crate deliberately adds no
//! runtime dependency for it. Instead the two images are embedded as `data:`
//! URLs in a throwaway HTML page, which is opened in a scratch target of the
//! ALREADY RUNNING headless engine: the browser decodes both PNGs, draws them
//! into a 2D canvas, diffs `ImageData` pixel by pixel, and hands back the
//! counts, the union bounding box and (when something differs) a diff PNG as
//! a data URL. The target is closed immediately afterwards. Same engine, same
//! sandbox, zero new crates, no OS input and no window on the owned headless
//! engine (on an external attach it is one scratch tab, created in the
//! BACKGROUND, closed immediately - see below).
//!
//! `data:` URLs are same-origin for canvas purposes, so `getImageData` never
//! taints - which is why the harness does not read the files directly
//! (`file://` images would taint the canvas and need a launch flag we do not
//! want).
//!
//! ## Anti-aliasing tolerance (the rule, so it can be argued with)
//!
//! Counting every channel delta above a threshold makes a baseline fail on
//! sub-pixel rasterisation noise, so the diff follows the prior art
//! (the pixelmatch-family `includeAA:false` default, Chromatic, odiff) and
//! ignores anti-aliasing by default. The rule is deliberately mechanical:
//!
//! 1. A pixel is a *candidate* when its largest channel delta exceeds
//!    `threshold` (unchanged) and it is not inside a `maskRects` region.
//! 2. Candidate pixels are grouped into 4-connected components.
//! 3. A candidate pixel is AA-eligible when its 3x3 luminance neighbourhood
//!    has a range of at least 32 levels (of 255) in **both** images - i.e. it
//!    sits on a high-contrast edge in the baseline AND in the capture.
//! 4. A component is ignored **only** when `antialias` is on, every pixel in
//!    it is AA-eligible, and it is no larger than `minDiffCluster` (default
//!    2). Everything else counts.
//!
//! That is why a 1 px edge artefact passes while a 1 px colour change in a
//! flat area fails (the baseline neighbourhood there has no contrast, so the
//! pixel is not AA-eligible) and a 2x2 change fails (a 4-pixel component is
//! larger than the minimum). The threshold is never raised to buy tolerance.
//!
//! ## Baseline environment metadata (browser-update rot)
//!
//! Every `save` writes `<name>.meta.json` beside the PNG: browser product and
//! major version, viewport, deviceScaleFactor, url, checksum and `createdAt`.
//! `compare` reads it and reports `environment: {match, baseline, current}`,
//! adding a `baseline_environment_mismatch` warning when the browser major,
//! the viewport or the DSF differ. A mismatch does not fail the compare by
//! default (the pixels still decide); `requireEnvironmentMatch:true` makes it
//! fail. `update:true` rewrites the PNG and the sidecar together.
//!
//! ## Scratch target activation
//!
//! `Target.createTarget` is called with `background: true`, so the scratch tab
//! never becomes the active tab. There is **no foreground fallback**: an
//! engine that rejects the parameter gets a typed
//! `scratch_background_unsupported` error and no target is created, because an
//! activating tab on an engine we do not own is exactly what the owner
//! forbids. The target is closed on every path (success, error, cancel).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{json, Value};

use crate::browser::Browser;
use crate::capture::{self, Options};
use crate::tools::ToolErr;

/// Budget for the scratch diff target: page load + decode + pixel loop.
const DIFF_BUDGET: Duration = Duration::from_secs(20);

/// Luminance range (0..255) across a 3x3 window that makes a pixel "on a
/// high-contrast edge" for the AA rule. Mirrors what the prior art treats as
/// a local gradient rather than a flat fill.
const AA_EDGE_CONTRAST: i64 = 32;

/// Baseline root: `GLADIUS_VISUAL_DIR` when set, else `<cwd>/.gladius/visual`.
pub fn visual_dir() -> PathBuf {
    match std::env::var("GLADIUS_VISUAL_DIR") {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v.trim()),
        _ => std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".gladius")
            .join("visual"),
    }
}

/// Name sanitisation: only `[A-Za-z0-9._-]` survives; everything else
/// becomes `_`. Path separators are therefore impossible, and the base names
/// `.` / `..` / all-dots are refused.
pub fn sanitise_name(raw: &str) -> Result<String, String> {
    let t = raw.trim();
    if t.is_empty() {
        return Err("visual: 'name' must be a non-empty baseline name".to_string());
    }
    let mut out = String::with_capacity(t.len());
    for c in t.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.chars().all(|c| c == '.') {
        return Err(format!(
            "visual: name '{raw}' sanitises to dots only; refused"
        ));
    }
    if out.len() > 120 {
        out.truncate(120);
    }
    Ok(out)
}

pub fn baseline_path(name: &str) -> PathBuf {
    visual_dir().join(format!("{name}.png"))
}

fn diff_path(name: &str) -> PathBuf {
    visual_dir().join(format!("{name}.diff.png"))
}

/// Sidecar path for the baseline's recorded environment.
pub fn meta_path(name: &str) -> PathBuf {
    visual_dir().join(format!("{name}.meta.json"))
}

/// Whether a baseline (or its diff by-product) exists for `name`. The name
/// is expected to be already sanitised.
pub fn exists(name: &str) -> bool {
    baseline_path(name).is_file() || diff_path(name).is_file()
}

fn write_png(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("visual: cannot create {}: {e}", parent.display()))?;
        }
    }
    std::fs::write(path, bytes).map_err(|e| format!("visual: write {}: {e}", path.display()))
}

fn write_json(path: &std::path::Path, v: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("visual: cannot create {}: {e}", parent.display()))?;
        }
    }
    let text = serde_json::to_string_pretty(v)
        .map_err(|e| format!("visual: encode {}: {e}", path.display()))?;
    std::fs::write(path, text).map_err(|e| format!("visual: write {}: {e}", path.display()))
}

// -------------------------------------------------------------- environment

/// The recorded environment of a baseline. This is what catches
/// browser-update rot: the same DOM rendered by Chrome 155 and Chrome 156 is
/// not the same baseline, and a silently accepted drift hides a real change
/// behind "the engine updated".
#[derive(Clone)]
pub struct EnvMeta {
    pub browser: String,
    pub browser_major: i64,
    pub width: i64,
    pub height: i64,
    pub dsf: f64,
    pub url: String,
    pub checksum: String,
    pub created_at: String,
}

impl EnvMeta {
    fn to_json(&self) -> Value {
        json!({
            "browser": self.browser,
            "browserMajor": self.browser_major,
            "viewport": {"width": self.width, "height": self.height},
            "deviceScaleFactor": self.dsf,
            "url": self.url,
            "checksum": self.checksum,
            "createdAt": self.created_at,
        })
    }

    fn from_json(v: &Value) -> Option<EnvMeta> {
        Some(EnvMeta {
            browser: v.get("browser")?.as_str()?.to_string(),
            browser_major: v.get("browserMajor")?.as_i64()?,
            width: v.pointer("/viewport/width")?.as_i64()?,
            height: v.pointer("/viewport/height")?.as_i64()?,
            dsf: v.get("deviceScaleFactor")?.as_f64()?,
            url: v.get("url")?.as_str()?.to_string(),
            checksum: v.get("checksum")?.as_str()?.to_string(),
            created_at: v.get("createdAt")?.as_str()?.to_string(),
        })
    }

    /// The three axes that decide `environment.match`: browser major,
    /// viewport, deviceScaleFactor. The url is recorded (and returned) but a
    /// baseline is allowed to be compared from a different path.
    fn differences(&self, current: &EnvMeta) -> Vec<String> {
        let mut out = Vec::new();
        if self.browser_major != current.browser_major {
            out.push(format!(
                "browser major {} ({}) -> {} ({})",
                self.browser_major, self.browser, current.browser_major, current.browser
            ));
        }
        if self.width != current.width || self.height != current.height {
            out.push(format!(
                "viewport {}x{} -> {}x{}",
                self.width, self.height, current.width, current.height
            ));
        }
        if (self.dsf - current.dsf).abs() > f64::EPSILON {
            out.push(format!("deviceScaleFactor {} -> {}", self.dsf, current.dsf));
        }
        out
    }
}

/// What the sidecar actually told us. `Missing`, `Malformed` and
/// `Unreadable` are deliberately NOT collapsed into "no metadata": treating
/// them as a match would let `requireEnvironmentMatch:true` pass without
/// evidence.
pub enum MetaState {
    Recorded(EnvMeta),
    Missing,
    Malformed(String),
    Unreadable(String),
}

impl MetaState {
    /// `missing` / `malformed` / `unreadable` / `recorded`.
    fn reason(&self) -> &'static str {
        match self {
            MetaState::Recorded(_) => "recorded",
            MetaState::Missing => "missing",
            MetaState::Malformed(_) => "malformed",
            MetaState::Unreadable(_) => "unreadable",
        }
    }

    fn recorded(&self) -> Option<&EnvMeta> {
        match self {
            MetaState::Recorded(m) => Some(m),
            _ => None,
        }
    }

    fn detail(&self) -> String {
        match self {
            MetaState::Recorded(m) => format!("recorded in {} / {}", m.browser, m.created_at),
            MetaState::Missing => "no sidecar".to_string(),
            MetaState::Malformed(why) => format!("malformed sidecar ({why})"),
            MetaState::Unreadable(why) => format!("unreadable sidecar ({why})"),
        }
    }
}

fn read_meta(name: &str) -> MetaState {
    let p = meta_path(name);
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return MetaState::Missing,
        Err(e) => {
            return MetaState::Unreadable(format!("cannot read {}: {e}", p.display()));
        }
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(v) => match EnvMeta::from_json(&v) {
            Some(m) => MetaState::Recorded(m),
            None => MetaState::Malformed(format!(
                "{} is not a complete environment record",
                p.display()
            )),
        },
        Err(e) => MetaState::Malformed(format!("{} is not valid JSON: {e}", p.display())),
    }
}

/// `(browser product, major version)` from the engine itself. Never fails the
/// caller: an engine that will not answer `Browser.getVersion` is recorded as
/// `unknown`/0 and the mismatch logic simply never keys on it.
async fn engine_identity(browser: &Browser) -> (String, i64) {
    let v = match browser.cdp_raw("Browser.getVersion", json!({}), None).await {
        Ok(v) => v,
        Err(e) => {
            capture::warn(&format!("visual: Browser.getVersion failed: {e}"));
            return ("unknown".to_string(), 0);
        }
    };
    let product = v
        .get("product")
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string();
    let version = product.rsplit('/').next().unwrap_or("");
    let major = version
        .split('.')
        .next()
        .and_then(|m| m.parse::<i64>().ok())
        .unwrap_or(0);
    let browser = if product.is_empty() {
        "unknown".to_string()
    } else {
        product
    };
    (browser, major)
}

async fn current_environment(browser: &Browser, cap: &capture::Capture) -> EnvMeta {
    let (product, major) = engine_identity(browser).await;
    EnvMeta {
        browser: product,
        browser_major: major,
        width: cap.width,
        height: cap.height,
        dsf: cap.dsf,
        url: cap.url.clone(),
        checksum: cap.checksum.clone(),
        created_at: iso8601_now(),
    }
}

/// UTC timestamp, `1970-01-01T00:00:00Z` shape. Dependency free: seconds from
/// the Unix epoch through Hinnant's civil-from-days.
fn iso8601_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant's `civil_from_days` (proleptic Gregorian), exact for all
/// dates we can produce.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// --------------------------------------------------------------- save/list

/// Capture deterministically and store the PNG plus its environment sidecar.
pub async fn save(browser: &Browser, name: &str, full_page: bool) -> Result<Value, String> {
    let safe = sanitise_name(name)?;
    let cap = capture::capture(
        browser,
        &Options {
            full_page,
            deterministic: true,
            width: None,
            height: None,
        },
    )
    .await?;
    let path = baseline_path(&safe);
    write_png(&path, &cap.bytes)?;
    let env = current_environment(browser, &cap).await;
    let meta = meta_path(&safe);
    write_json(&meta, &env.to_json())?;
    Ok(json!({
        "ok": true, "action": "save", "name": safe,
        "path": path.to_string_lossy(),
        "metaPath": meta.to_string_lossy(),
        "bytes": cap.bytes.len() as i64,
        "checksum": cap.checksum,
        "viewport": {"width": cap.width, "height": cap.height},
        "url": cap.url,
        "fullPage": full_page,
        "environment": env.to_json(),
    }))
}

/// Every stored baseline (diff images are excluded: they are by-products).
pub fn list() -> Result<Value, String> {
    let dir = visual_dir();
    let mut entries: Vec<Value> = Vec::new();
    match std::fs::read_dir(&dir) {
        Ok(rd) => {
            for e in rd.flatten() {
                let p = e.path();
                let fname = match p.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                if !fname.ends_with(".png") || fname.ends_with(".diff.png") {
                    continue;
                }
                let bytes = e.metadata().map(|m| m.len()).unwrap_or(0);
                let name = fname.trim_end_matches(".png").to_string();
                entries.push(json!({
                    "name": name,
                    "path": p.to_string_lossy(),
                    "bytes": bytes as i64,
                    "meta": meta_path(&name).is_file(),
                }));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("visual: cannot read {}: {e}", dir.display())),
    }
    entries.sort_by(|a, b| {
        a.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .cmp(b.get("name").and_then(|v| v.as_str()).unwrap_or(""))
    });
    let count = entries.len() as i64;
    Ok(
        json!({"ok": true, "action": "list", "dir": dir.to_string_lossy(), "baselines": entries, "count": count}),
    )
}

/// Remove the baseline, its sidecar and its diff by-product for `name`.
pub fn delete(name: &str) -> Result<Value, String> {
    let safe = sanitise_name(name)?;
    let base = baseline_path(&safe);
    let diff = diff_path(&safe);
    let meta = meta_path(&safe);
    let had_base = base.is_file();
    let had_diff = diff.is_file();
    let had_meta = meta.is_file();
    if !had_base && !had_diff && !had_meta {
        return Err(format!(
            "visual: no baseline named '{}' in {}",
            safe,
            visual_dir().display()
        ));
    }
    let mut deleted: Vec<String> = Vec::new();
    for (path, had) in [(&base, had_base), (&diff, had_diff), (&meta, had_meta)] {
        if had {
            std::fs::remove_file(path)
                .map_err(|e| format!("visual: delete {}: {e}", path.display()))?;
            deleted.push(path.to_string_lossy().to_string());
        }
    }
    Ok(json!({"ok": true, "action": "delete", "name": safe, "deleted": deleted}))
}

// ----------------------------------------------------------------- compare

/// Compare tuning. `threshold` is a per-channel fraction of the 0..255 range
/// (0.1 means a channel must move by more than 25.5 levels to count).
/// `antialias` enables the documented AA rule; `min_diff_cluster` is the
/// largest AA-eligible component that is ignored (default 2).
pub struct DiffOpts {
    pub threshold: f64,
    pub max_diff_pixels: i64,
    pub max_diff_ratio: f64,
    pub mask_rects: Vec<Value>,
    pub full_page: bool,
    pub update: bool,
    pub antialias: bool,
    pub min_diff_cluster: i64,
    pub require_environment_match: bool,
}

impl Default for DiffOpts {
    fn default() -> Self {
        DiffOpts {
            threshold: 0.1,
            max_diff_pixels: 0,
            max_diff_ratio: 0.0,
            mask_rects: Vec::new(),
            full_page: false,
            update: false,
            antialias: true,
            min_diff_cluster: 2,
            require_environment_match: false,
        }
    }
}

/// Result of the in-engine pixel diff.
pub struct PixelDiff {
    pub width: i64,
    pub height: i64,
    pub diff_pixels: i64,
    pub total_pixels: i64,
    pub max_channel_delta: i64,
    pub bbox: Option<Value>,
    pub diff_png_base64: Option<String>,
    pub size_mismatch: bool,
    pub ignored_pixels: i64,
    pub ignored_clusters: i64,
    /// True when the engine accepted `Target.createTarget {background:true}`.
    pub scratch_background: bool,
}

fn file_url(path: &std::path::Path) -> String {
    format!(
        "file:///{}",
        path.to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches('/')
    )
}

/// `Target.createTarget` params for the scratch diff tab: ALWAYS
/// `background: true`. The compare must never activate a tab, so there is no
/// foreground variant to fall back to (`background` is the only spelling).
fn create_target_params() -> Value {
    json!({"url": "about:blank", "background": true})
}

/// Does this CDP error mean "this engine does not know/accept the
/// `background` parameter"? Such an engine cannot host the diff tab without
/// taking the foreground, so the compare refuses instead of falling back.
fn background_param_unsupported(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("background")
        || e.contains("invalid parameters")
        || e.contains("failed to deserialize")
}

/// The refusal for an engine that cannot host the diff tab in the
/// background. Typed, so a caller can tell "this engine cannot do it" from a
/// generic failure - and so nobody is tempted to fall back to a foreground
/// tab.
fn scratch_background_unsupported(detail: &str) -> ToolErr {
    ToolErr::typed(
        format!(
            "visual: scratch_background_unsupported: the engine refused to create the diff \
             target in the background ({detail}); refusing to open a foreground tab"
        ),
        serde_json::json!({"code": "scratch_background_unsupported",
                           "reason": "background_param_rejected", "tool": "visual"}),
    )
}

/// Test-only hook (never set in production): pretend the engine rejected
/// `background:true`, so the refusal path can be exercised without an engine
/// that actually rejects it. The target is NOT created in that case.
fn test_reject_background() -> bool {
    std::env::var("GLADIUS_TEST_SCRATCH_REJECT_BACKGROUND")
        .map(|v| !v.trim().is_empty() && v.trim() != "0")
        .unwrap_or(false)
}

/// The harness page. Both images ride in as `data:` URLs, so decoding happens
/// in the browser and no PNG bytes cross the CDP boundary. The AA rule is
/// implemented here (see the module docs for the exact definition).
fn harness_html(a: &str, b: &str, opts: &DiffOpts) -> String {
    let opt = json!({
        "threshold": opts.threshold,
        "maskRects": opts.mask_rects,
        "antialias": opts.antialias,
        "minDiffCluster": opts.min_diff_cluster,
        "edgeContrast": AA_EDGE_CONTRAST,
    });
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><title>gladius-diff</title></head><body>
<script>
window.__gladiusDiffResult = null;
const A = 'data:image/png;base64,{a}';
const B = 'data:image/png;base64,{b}';
const OPT = {opt};
function load(src) {{
  return new Promise(function (res, rej) {{
    const i = new Image();
    i.onload = function () {{ res(i); }};
    i.onerror = function () {{ rej(new Error('decode_failed')); }};
    i.src = src;
  }});
}}
(async function () {{
  try {{
    const a = await load(A), b = await load(B);
    const aw = a.naturalWidth, ah = a.naturalHeight, bw = b.naturalWidth, bh = b.naturalHeight;
    if (aw !== bw || ah !== bh) {{
      window.__gladiusDiffResult = {{sizeMismatch: true, aw: aw, ah: ah, bw: bw, bh: bh}};
      return;
    }}
    const w = aw, h = ah, n = w * h;
    const c = document.createElement('canvas');
    c.width = w; c.height = h;
    const g = c.getContext('2d', {{willReadFrequently: true}});
    g.drawImage(a, 0, 0);
    const da = g.getImageData(0, 0, w, h).data;
    g.clearRect(0, 0, w, h);
    g.drawImage(b, 0, 0);
    const db = g.getImageData(0, 0, w, h).data;

    // Luminance per pixel (integer 0..255) for the edge test.
    const lumA = new Uint8Array(n), lumB = new Uint8Array(n);
    for (let p = 0, i = 0; p < n; p++, i += 4) {{
      lumA[p] = (0.2126 * da[i] + 0.7152 * da[i + 1] + 0.0722 * da[i + 2]) | 0;
      lumB[p] = (0.2126 * db[i] + 0.7152 * db[i + 1] + 0.0722 * db[i + 2]) | 0;
    }}

    const thr = Math.max(0, Math.min(1, OPT.threshold || 0)) * 255;
    const rects = OPT.maskRects || [];
    function masked(x, y) {{
      for (let k = 0; k < rects.length; k++) {{
        const m = rects[k];
        if (x >= m.x && x < m.x + m.w && y >= m.y && y < m.y + m.h) return true;
      }}
      return false;
    }}

    // 1. Candidates: channel delta above threshold, outside every mask rect.
    const mask = new Uint8Array(n);
    const delta = new Uint8Array(n);
    for (let y = 0; y < h; y++) {{
      for (let x = 0; x < w; x++) {{
        const p = y * w + x, i = p * 4;
        let d = 0;
        for (let ch = 0; ch < 4; ch++) {{
          const dd = Math.abs(da[i + ch] - db[i + ch]);
          if (dd > d) d = dd;
        }}
        delta[p] = d;
        if (d > thr && !masked(x, y)) mask[p] = 1;
      }}
    }}

    // 2. AA eligibility: a high-contrast 3x3 luminance neighbourhood in BOTH
    //    images (edge in the baseline AND in the capture).
    const edgeContrast = OPT.edgeContrast == null ? 32 : OPT.edgeContrast;
    function onEdge(lum, x, y) {{
      let mn = 255, mx = 0;
      for (let yy = y - 1; yy <= y + 1; yy++) {{
        if (yy < 0 || yy >= h) continue;
        for (let xx = x - 1; xx <= x + 1; xx++) {{
          if (xx < 0 || xx >= w) continue;
          const l = lum[yy * w + xx];
          if (l < mn) mn = l;
          if (l > mx) mx = l;
        }}
      }}
      return (mx - mn) >= edgeContrast;
    }}

    // 3. 4-connected components of candidates; ignore a component only when
    //    antialias is on, every pixel is AA-eligible, and it is small.
    const aaOn = OPT.antialias !== false;
    const minCluster = Math.max(0, OPT.minDiffCluster == null ? 2 : OPT.minDiffCluster | 0);
    const counted = new Uint8Array(n);
    const seen = new Uint8Array(n);
    const stack = [];
    const comp = [];
    let diff = 0, ignored = 0, ignoredClusters = 0, maxDelta = 0;
    let x0 = w, y0 = h, x1 = -1, y1 = -1;
    for (let y = 0; y < h; y++) {{
      for (let x = 0; x < w; x++) {{
        const start = y * w + x;
        if (!mask[start] || seen[start]) continue;
        comp.length = 0;
        stack.length = 0;
        stack.push(start);
        seen[start] = 1;
        let allEdge = true;
        while (stack.length) {{
          const q = stack.pop();
          comp.push(q);
          const qx = q % w, qy = (q - qx) / w;
          if (allEdge && !(onEdge(lumA, qx, qy) && onEdge(lumB, qx, qy))) allEdge = false;
          if (qx > 0 && mask[q - 1] && !seen[q - 1]) {{ seen[q - 1] = 1; stack.push(q - 1); }}
          if (qx + 1 < w && mask[q + 1] && !seen[q + 1]) {{ seen[q + 1] = 1; stack.push(q + 1); }}
          if (qy > 0 && mask[q - w] && !seen[q - w]) {{ seen[q - w] = 1; stack.push(q - w); }}
          if (qy + 1 < h && mask[q + w] && !seen[q + w]) {{ seen[q + w] = 1; stack.push(q + w); }}
        }}
        if (aaOn && allEdge && comp.length <= minCluster) {{
          ignored += comp.length;
          ignoredClusters++;
          continue;
        }}
        for (let k = 0; k < comp.length; k++) {{
          const q = comp[k];
          counted[q] = 1;
          const qx = q % w, qy = (q - qx) / w;
          diff++;
          if (qx < x0) x0 = qx;
          if (qy < y0) y0 = qy;
          if (qx > x1) x1 = qx;
          if (qy > y1) y1 = qy;
          if (delta[q] > maxDelta) maxDelta = delta[q];
        }}
      }}
    }}

    // 4. Diff image: counted pixels red, ignored AA pixels amber.
    let diffPng = null;
    const out = g.createImageData(w, h);
    if (diff > 0) {{
      for (let p = 0; p < n; p++) {{
        const i = p * 4;
        if (counted[p]) {{
          out.data[i] = 255; out.data[i + 1] = 0; out.data[i + 2] = 0; out.data[i + 3] = 255;
        }} else if (mask[p]) {{
          out.data[i] = 255; out.data[i + 1] = 191; out.data[i + 2] = 0; out.data[i + 3] = 200;
        }}
      }}
      g.clearRect(0, 0, w, h);
      g.putImageData(out, 0, 0);
      diffPng = c.toDataURL('image/png');
    }}
    window.__gladiusDiffResult = {{
      sizeMismatch: false, w: w, h: h,
      diffPixels: diff, totalPixels: n,
      ignoredPixels: ignored, ignoredClusters: ignoredClusters,
      maxChannelDelta: maxDelta,
      bbox: diff > 0 ? {{x: x0, y: y0, w: x1 - x0 + 1, h: y1 - y0 + 1}} : null,
      diffPng: diffPng
    }};
  }} catch (e) {{
    window.__gladiusDiffResult = {{error: String(e && e.message ? e.message : e)}};
  }}
}})();
</script></body></html>"#
    )
}

/// Decode + diff two PNGs inside a scratch headless target, then close it.
pub async fn png_diff(
    browser: &Browser,
    baseline: &[u8],
    current: &[u8],
    opts: &DiffOpts,
) -> Result<PixelDiff, ToolErr> {
    let dir =
        tempfile::tempdir().map_err(|e| ToolErr::fallback(format!("visual: scratch dir: {e}")))?;
    let page = dir.path().join("diff.html");
    let a_b64 = base64::engine::general_purpose::STANDARD.encode(baseline);
    let b_b64 = base64::engine::general_purpose::STANDARD.encode(current);
    std::fs::write(&page, harness_html(&a_b64, &b_b64, opts))
        .map_err(|e| ToolErr::fallback(format!("visual: scratch page: {e}")))?;
    // Background target ONLY: the scratch tab must never become the active
    // tab. An engine that cannot honour `background:true` is refused with a
    // typed error - never retried in the foreground, which would activate a
    // tab on an engine we do not own.
    if test_reject_background() {
        return Err(scratch_background_unsupported(
            "engine rejected Target.createTarget background:true (injected by GLADIUS_TEST_SCRATCH_REJECT_BACKGROUND)",
        ));
    }
    let created = match browser
        .cdp_raw("Target.createTarget", create_target_params(), None)
        .await
    {
        Ok(v) => v,
        Err(e) if background_param_unsupported(&e) => {
            return Err(scratch_background_unsupported(&e));
        }
        Err(e) => {
            return Err(ToolErr::fallback(format!(
                "visual: scratch target was not created: {e}"
            )))
        }
    };
    let tid = created
        .get("targetId")
        .and_then(|v| v.as_str())
        .ok_or("visual: scratch target was not created".to_string())?
        .to_string();
    let attached = browser
        .cdp_raw(
            "Target.attachToTarget",
            json!({"targetId": tid, "flatten": true}),
            None,
        )
        .await;
    let sid = match attached {
        Ok(v) => v
            .get("sessionId")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string()),
        Err(_) => None,
    };
    let mut value: Option<Value> = None;
    let mut last: Option<String> = None;
    // Test-only: widen the window in which the scratch target is open, so a
    // cancellation test can cancel while it exists and assert it is closed.
    crate::capture::maybe_test_delay().await;
    if let Some(sid) = sid.clone() {
        // Navigate explicitly on the scratch session: the created target can
        // race its own initial load, and an explicit navigate reports the
        // engine's errorText instead of leaving us staring at about:blank.
        match browser
            .cdp_raw(
                "Page.navigate",
                json!({"url": file_url(&page)}),
                Some(sid.clone()),
            )
            .await
        {
            Ok(v) => {
                if let Some(e) = v.get("errorText").and_then(|e| e.as_str()) {
                    if !e.is_empty() {
                        last = Some(format!("navigate failed: {e}"));
                    }
                }
            }
            Err(e) => last = Some(format!("navigate failed: {e}")),
        }
        let deadline = Instant::now() + DIFF_BUDGET;
        loop {
            let probe = browser
                .cdp_raw(
                    "Runtime.evaluate",
                    json!({"expression": "window.__gladiusDiffResult", "returnByValue": true}),
                    Some(sid.clone()),
                )
                .await;
            match probe {
                Ok(v) => {
                    if let Some(ex) = v.get("exceptionDetails") {
                        last = Some(format!("js exception: {ex}"));
                    } else if let Some(val) = v.get("result").and_then(|r| r.get("value")) {
                        if !val.is_null() {
                            value = Some(val.clone());
                            break;
                        }
                        last = Some("result is null (page still loading)".to_string());
                    } else {
                        // `undefined`: the harness script has not run yet.
                        last = Some(format!(
                            "undefined (page url now {})",
                            v.get("result")
                                .and_then(|r| r.get("type"))
                                .and_then(|t| t.as_str())
                                .unwrap_or("?")
                        ));
                    }
                }
                Err(e) => last = Some(format!("probe failed: {e}")),
            }
            if Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    } else {
        last = Some("could not attach to the scratch target".to_string());
    }
    if value.is_none() {
        // One diagnostic read on the scratch page itself: what the target
        // actually shows when the harness result never appeared.
        if let Ok(v) = browser
            .cdp_raw(
                "Runtime.evaluate",
                json!({"expression": "[document.location.href, document.readyState, (document.scripts || []).length, document.contentType]", "returnByValue": true}),
                Some(sid.clone().unwrap_or_default()),
            )
            .await
        {
            let page = v
                .get("result")
                .and_then(|r| r.get("value"))
                .cloned()
                .unwrap_or(Value::Null);
            last = Some(format!(
                "{} | page={page}",
                last.unwrap_or_else(|| "no probe".to_string())
            ));
        }
    }
    // Close the scratch target on every path; a failure here must not mask
    // the diff result (the target is a by-product, never the answer) but it
    // must not vanish either: a target we cannot close leaks a tab, so it is
    // reported on stderr.
    if let Err(e) = browser
        .cdp_raw("Target.closeTarget", json!({"targetId": tid}), None)
        .await
    {
        eprintln!("[gladius] warn: visual: could not close the scratch diff target {tid}: {e}");
    }
    let v = value.ok_or_else(|| {
        ToolErr::fallback(format!(
            "visual: scratch diff target never produced a result ({})",
            last.unwrap_or_else(|| "no probe".to_string())
        ))
    })?;
    if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
        return Err(ToolErr::fallback(format!(
            "visual: diff failed inside the engine: {err}"
        )));
    }
    if v.get("sizeMismatch").and_then(|b| b.as_bool()) == Some(true) {
        let bw = v.get("bw").and_then(|n| n.as_i64()).unwrap_or(0);
        let bh = v.get("bh").and_then(|n| n.as_i64()).unwrap_or(0);
        return Ok(PixelDiff {
            width: bw,
            height: bh,
            diff_pixels: bw * bh,
            total_pixels: bw * bh,
            max_channel_delta: 255,
            bbox: None,
            diff_png_base64: None,
            size_mismatch: true,
            ignored_pixels: 0,
            ignored_clusters: 0,
            scratch_background: true,
        });
    }
    let num = |k: &str| v.get(k).and_then(|n| n.as_i64()).unwrap_or(0);
    Ok(PixelDiff {
        width: num("w"),
        height: num("h"),
        diff_pixels: num("diffPixels"),
        total_pixels: num("totalPixels"),
        max_channel_delta: num("maxChannelDelta"),
        bbox: v.get("bbox").cloned().filter(|b| !b.is_null()),
        diff_png_base64: v
            .get("diffPng")
            .and_then(|s| s.as_str())
            .map(|s| s.trim_start_matches("data:image/png;base64,").to_string()),
        size_mismatch: false,
        ignored_pixels: num("ignoredPixels"),
        ignored_clusters: num("ignoredClusters"),
        scratch_background: true,
    })
}

/// Diff the current deterministic capture against the stored baseline, and
/// check the recorded environment.
pub async fn compare(browser: &Browser, name: &str, opts: &DiffOpts) -> Result<Value, ToolErr> {
    let safe = sanitise_name(name).map_err(ToolErr::fallback)?;
    let base = baseline_path(&safe);
    if !base.is_file() {
        return Err(ToolErr::fallback(format!(
            "visual: no baseline named '{}' in {} (save it first)",
            safe,
            visual_dir().display()
        )));
    }
    let baseline = std::fs::read(&base)
        .map_err(|e| ToolErr::fallback(format!("visual: read {}: {e}", base.display())))?;
    let meta_state = read_meta(&safe);
    // Unknown metadata (missing/malformed/unreadable sidecar) is NOT a match.
    // With `requireEnvironmentMatch:true` it is a typed failure, because a
    // strict gate must not pass without evidence; otherwise it warns and the
    // pixels still decide.
    if opts.require_environment_match && meta_state.recorded().is_none() {
        return Err(ToolErr::typed(
            format!(
                "visual: baseline_environment_unknown: {} (requireEnvironmentMatch:true needs a recorded environment; re-save the baseline with update:true)",
                meta_state.detail()
            ),
            json!({"code": "baseline_environment_unknown",
                   "reason": meta_state.reason(), "tool": "visual", "name": safe,
                   "metaPath": meta_path(&safe).to_string_lossy()}),
        ));
    }
    let baseline_checksum = capture::checksum(&baseline);
    let cap = capture::capture(
        browser,
        &Options {
            full_page: opts.full_page,
            deterministic: true,
            width: None,
            height: None,
        },
    )
    .await?;
    let current_env = current_environment(browser, &cap).await;

    if opts.update {
        write_png(&base, &cap.bytes)?;
        write_json(&meta_path(&safe), &current_env.to_json())?;
        return Ok(json!({
            "ok": true, "action": "compare", "name": safe, "updated": true, "pass": true,
            "baselinePath": base.to_string_lossy(),
            "metaPath": meta_path(&safe).to_string_lossy(),
            "diffPath": Value::Null,
            "diffPixels": 0, "totalPixels": cap.width * cap.height, "ratio": 0.0,
            "bbox": Value::Null, "maxChannelDelta": 0,
            "threshold": opts.threshold, "maxDiffPixels": opts.max_diff_pixels,
            "maxDiffRatio": opts.max_diff_ratio,
            "antialias": opts.antialias, "minDiffCluster": opts.min_diff_cluster,
            "ignoredPixels": 0, "ignoredClusters": 0,
            "checksum": cap.checksum, "baselineChecksum": baseline_checksum,
            "viewport": {"width": cap.width, "height": cap.height}, "url": cap.url,
            "sizeMismatch": false,
            "environment": {"match": true, "baseline": current_env.to_json(),
                            "current": current_env.to_json()},
            "warning": Value::Null,
            "scratchBackground": true,
        }));
    }

    let diff = png_diff(browser, &baseline, &cap.bytes, opts).await?;
    let ratio = if diff.total_pixels > 0 {
        diff.diff_pixels as f64 / diff.total_pixels as f64
    } else {
        0.0
    };
    let mut wrote_diff: Option<String> = None;
    if let Some(b64) = &diff.diff_png_base64 {
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64.as_bytes()) {
            let p = diff_path(&safe);
            if write_png(&p, &bytes).is_ok() {
                wrote_diff = Some(p.to_string_lossy().to_string());
            }
        }
    }

    // Environment check: a mismatch (or an unknown baseline environment) is a
    // WARNING by default - the pixels still decide - and a failure only when
    // the caller requires a match.
    let (env_known, env_match, warning) = match meta_state.recorded() {
        Some(b) => {
            let diffs = b.differences(&current_env);
            if diffs.is_empty() {
                (true, true, Value::Null)
            } else {
                (
                    true,
                    false,
                    json!(format!(
                        "baseline_environment_mismatch: {} (the baseline was recorded in a \
                         different environment; re-save it with update:true after confirming \
                         the change is expected)",
                        diffs.join("; ")
                    )),
                )
            }
        }
        None => (
            false,
            false,
            json!(format!(
                "baseline_environment_unknown: {} (the baseline carries no usable environment \
                 record, so a match cannot be established; re-save it with update:true)",
                meta_state.detail()
            )),
        ),
    };
    let pixel_pass = !diff.size_mismatch
        && diff.diff_pixels <= opts.max_diff_pixels
        && ratio <= opts.max_diff_ratio;
    let pass = pixel_pass && (env_match || !opts.require_environment_match);
    Ok(json!({
        "ok": true, "action": "compare", "name": safe, "updated": false, "pass": pass,
        "baselinePath": base.to_string_lossy(),
        "metaPath": meta_path(&safe).to_string_lossy(),
        "diffPath": match &wrote_diff { Some(p) => json!(p), None => Value::Null },
        "diffPixels": diff.diff_pixels,
        "totalPixels": diff.total_pixels,
        "ratio": ratio,
        "bbox": match &diff.bbox { Some(b) => b.clone(), None => Value::Null },
        "maxChannelDelta": diff.max_channel_delta,
        "threshold": opts.threshold,
        "maxDiffPixels": opts.max_diff_pixels,
        "maxDiffRatio": opts.max_diff_ratio,
        "antialias": opts.antialias,
        "minDiffCluster": opts.min_diff_cluster,
        "ignoredPixels": diff.ignored_pixels,
        "ignoredClusters": diff.ignored_clusters,
        "sizeMismatch": diff.size_mismatch,
        "size": {"width": diff.width, "height": diff.height},
        "baselineChecksum": baseline_checksum,
        "checksum": cap.checksum,
        "viewport": {"width": cap.width, "height": cap.height},
        "url": cap.url,
        "environment": {
            "match": env_match,
            "known": env_known,
            "baseline": match meta_state.recorded() { Some(b) => b.to_json(), None => Value::Null },
            "current": current_env.to_json(),
        },
        "warning": warning,
        "scratchBackground": diff.scratch_background,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The civil-date conversion behind `createdAt` (no chrono dependency).
    #[test]
    fn civil_from_days_is_exact() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(1), (1970, 1, 2));
        // 1700000000 s = 2023-11-14T22:13:20Z, a widely published value.
        assert_eq!(civil_from_days(1_700_000_000 / 86_400), (2023, 11, 14));
        // Leap day.
        assert_eq!(civil_from_days(59), (1970, 3, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    /// The scratch target is ALWAYS requested in the background: there is no
    /// foreground variant, because an activating tab is exactly what the
    /// owner forbids. A rejected parameter is recognised so the compare can
    /// refuse with a typed error instead of falling back.
    #[test]
    fn create_target_only_ever_asks_for_a_background_tab() {
        assert_eq!(
            create_target_params().get("background"),
            Some(&json!(true)),
            "background:true is the only spelling we send"
        );
        // A rejecting engine and a real error are told apart.
        assert!(background_param_unsupported(
            "Invalid parameters: Failed to deserialize params.background - BINDINGS: boolean value expected"
        ));
        assert!(background_param_unsupported(
            "Unknown parameter 'background'"
        ));
        assert!(!background_param_unsupported(
            "Target with given id does not exist"
        ));
    }

    /// The AA rule's tuning is carried into the harness page; both directions
    /// of the `antialias`/`includeAA` naming resolve to one flag at the tool
    /// layer (`tools::tool_visual`), and the default here is "ignore AA".
    #[test]
    fn harness_receives_the_antialias_rule() {
        let opts = DiffOpts::default();
        assert!(opts.antialias, "AA is ignored by default");
        assert_eq!(opts.min_diff_cluster, 2, "default cluster budget");
        let html = harness_html("AAA", "BBB", &opts);
        assert!(html.contains("\"antialias\":true"), "flag reaches the page");
        assert!(
            html.contains("\"minDiffCluster\":2"),
            "budget reaches the page"
        );
        assert!(
            html.contains("ignoredClusters"),
            "the page reports what it ignored"
        );
        let off = DiffOpts {
            antialias: false,
            ..DiffOpts::default()
        };
        assert!(harness_html("AAA", "BBB", &off).contains("\"antialias\":false"));
    }
}
