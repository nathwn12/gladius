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
//! knows how to ignore anti-aliasing. The rule is deliberately mechanical:
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
//! That is why a 1 px edge artefact can be subtracted while a 1 px colour
//! change in a flat area is not (the baseline neighbourhood there has no
//! contrast, so the pixel is not AA-eligible) and a 2x2 change fails (a
//! 4-pixel component is larger than the minimum). The threshold is never
//! raised to buy tolerance.
//!
//! Since 0.8.0 the subtraction does not buy a PASS by default: `passWithinTolerance`
//! is false while `ignoredPixels > 0` unless the caller passes
//! `allowIgnoredAA:true`. Strictness is the default; the escape hatch is
//! explicit and always announced in `warning`.
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
use crate::tools::{ToolCode, ToolErr, ToolReason};

/// Budget for the scratch diff target: page load + decode + pixel loop.
const DIFF_BUDGET: Duration = Duration::from_secs(20);

/// Luminance range (0..255) across a 3x3 window that makes a pixel "on a
/// high-contrast edge" for the AA rule. Mirrors what the prior art treats as
/// a local gradient rather than a flat fill.
const AA_EDGE_CONTRAST: i64 = 32;

/// Baseline root: `GLADIUS_VISUAL_DIR` when set, else `%LOCALAPPDATA%/gladius/
/// visual` on Windows, else `<cwd>/.gladius/visual` when `LOCALAPPDATA` is
/// unset (the same fallback discipline the Chromium cache uses).
///
/// The default used to be the caller's own repo, which is the bug S2.4 closes:
/// a capture tool must not write into the tree it is testing.
pub fn visual_dir() -> PathBuf {
    match std::env::var("GLADIUS_VISUAL_DIR") {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v.trim()),
        _ => match std::env::var("LOCALAPPDATA") {
            Ok(local) if !local.trim().is_empty() => {
                PathBuf::from(local.trim()).join("gladius").join("visual")
            }
            _ => std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(".gladius")
                .join("visual"),
        },
    }
}

/// Baseline cap (S2.4): 500 stored PNGs, and 500 `.diff.png` by-products,
/// counted separately. `GLADIUS_VISUAL_CAP` overrides both for tests and for
/// a caller with a smaller disk budget; it never raises the refusal's
/// meaning, it only moves the number.
pub fn visual_cap() -> i64 {
    std::env::var("GLADIUS_VISUAL_CAP")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(500)
}

/// How one file in the store is classified. `save` writes `<name>.png` **and**
/// its `<name>.meta.json` sidecar; `compare` writes `<name>.diff.png` and no
/// sidecar. The sidecar is therefore the identity record: a `.diff.png` that
/// carries one is a **baseline** whose name happens to end in `.diff` (a name
/// `save` accepted before S2.4 closed that door), never a by-product. A suffix
/// alone is not evidence, which is why the count and the age-out both ask here.
enum StoreFile {
    Baseline,
    DiffByProduct,
    Other,
}

fn classify(dir: &std::path::Path, fname: &str) -> StoreFile {
    let Some(base) = fname.strip_suffix(".png") else {
        return StoreFile::Other;
    };
    if base.ends_with(".diff") && !dir.join(format!("{base}.meta.json")).is_file() {
        return StoreFile::DiffByProduct;
    }
    StoreFile::Baseline
}

/// Is this exact path a stored baseline? True when the sidecar that `save`
/// writes beside the PNG is present.
fn is_baseline_png(path: &std::path::Path) -> bool {
    match (path.parent(), path.file_name().and_then(|n| n.to_str())) {
        (Some(dir), Some(fname)) => matches!(classify(dir, fname), StoreFile::Baseline),
        _ => false,
    }
}

/// `(baselines, diff by-products)` stored under `dir`. Counted from the file
/// names plus the sidecar that identifies a baseline, so no sidecar is required
/// to be *readable* - only to exist.
fn store_counts(dir: &std::path::Path) -> (i64, i64) {
    let mut pngs = 0;
    let mut diffs = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            match classify(dir, &name) {
                StoreFile::Baseline => pngs += 1,
                StoreFile::DiffByProduct => diffs += 1,
                StoreFile::Other => {}
            }
        }
    }
    (pngs, diffs)
}

/// A baseline name whose PNG path is also the `.diff.png` by-product path of
/// another baseline is refused. One path cannot hold two artifacts: a baseline
/// that shared it would be excluded from the cap and eligible for the automatic
/// age-out, which the spec forbids. `<name>.diff` is the whole colliding family
/// (case-insensitively, because the store lives on a case-insensitive
/// filesystem).
fn byproduct_namespace_refusal(name: &str) -> Option<ToolErr> {
    let lower = name.to_ascii_lowercase();
    let stem = lower.strip_suffix(".diff")?;
    Some(ToolErr::typed(
        format!(
            "visual: the baseline name '{name}' would be stored as '{name}.png', which is \
             exactly the .diff.png by-product that `compare` writes for the baseline \
             '{stem}'; a baseline and a by-product cannot share a path, so this name is \
             refused - pick a name that does not end in '.diff'"
        ),
        json!({"code": "invalid_args", "reason": "bad_value", "tool": "visual", "arg": "name"}),
    ))
}

/// Refuse a save that would grow the store past its cap.
///
/// Never a silent eviction: the caller is told the number and both levers
/// (`visual delete`, `GLADIUS_VISUAL_CAP`). Overwriting a baseline that is
/// already stored is not growth and is always allowed.
fn cap_refusal(name: &str) -> Option<ToolErr> {
    let dir = visual_dir();
    let cap = visual_cap();
    let (pngs, _) = store_counts(&dir);
    if pngs < cap || baseline_path(name).is_file() {
        return None;
    }
    Some(ToolErr::typed(
        format!(
            "visual: the store at {} already holds {pngs} baselines and its cap is {cap}; \
             delete one with `visual delete` (or point GLADIUS_VISUAL_DIR somewhere else, or \
             raise GLADIUS_VISUAL_CAP) - a save is refused rather than evicting a baseline \
             silently",
            dir.display()
        ),
        json!({"code": ToolCode::InvalidArgs.as_str(), "reason": ToolReason::BadValue.as_str(), "tool": "visual", "arg": "name"}),
    ))
}

/// Age out `.diff.png` by-products (oldest first) so the next write keeps the
/// store under its diff cap. Diff images are the only files eligible: they are
/// by-products of a compare, never a baseline - and a `.diff.png` whose own
/// sidecar exists is a baseline, so it is not in this set at all.
fn age_out_diffs() {
    let dir = visual_dir();
    let cap = visual_cap();
    let mut diffs: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !matches!(classify(&dir, &name), StoreFile::DiffByProduct) {
            continue;
        }
        let modified = e
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        diffs.push((modified, e.path()));
    }
    if (diffs.len() as i64) < cap {
        return;
    }
    diffs.sort_by(|a, b| a.0.cmp(&b.0));
    // Leave room for the write that is about to happen.
    let excess = diffs.len() as i64 - cap + 1;
    for (_, path) in diffs.into_iter().take(excess.max(0) as usize) {
        eprintln!(
            "[gladius] visual: aging out diff by-product {}",
            path.display()
        );
        let _ = std::fs::remove_file(&path);
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
    /// Did the capture that produced this baseline settle? `None` means the
    /// sidecar is from 0.7.x and records nothing - which is `unknown`, never
    /// `true`.
    pub settled: Option<bool>,
    /// Which settle waits lost, when the sidecar records it.
    pub settle_timeouts: Option<Vec<String>>,
    // The 0.8.0 fingerprint (S1.8). Every one is `Option` because a 0.7.x
    // sidecar has none of them: a missing axis is UNKNOWN, and the compare says
    // so instead of letting the baseline match silently.
    /// Full engine version string (`155.0.8059.84`), not just the major.
    pub full_version: Option<String>,
    /// `navigator.platform`.
    pub platform: Option<String>,
    /// `navigator.userAgent`.
    pub user_agent: Option<String>,
    /// `prefers-color-scheme`: `dark` or `light`.
    pub color_scheme: Option<String>,
    /// `forced-colors: active`.
    pub forced_colors: Option<bool>,
    /// `prefers-contrast`: `more`, `less` or `no-preference`.
    pub contrast: Option<String>,
    /// `prefers-reduced-motion`: `reduce` or `no-preference`.
    pub reduced_motion: Option<String>,
    /// FNV-1a over the sorted `family|weight|style` of every `FontFace`.
    pub font_count_hash: Option<String>,
    /// `innerWidth - documentElement.clientWidth`.
    pub scrollbar_width: Option<i64>,
}

impl EnvMeta {
    fn to_json(&self) -> Value {
        let mut m = serde_json::Map::new();
        m.insert("browser".to_string(), json!(self.browser));
        m.insert("browserMajor".to_string(), json!(self.browser_major));
        m.insert(
            "viewport".to_string(),
            json!({"width": self.width, "height": self.height}),
        );
        m.insert("deviceScaleFactor".to_string(), json!(self.dsf));
        m.insert("url".to_string(), json!(self.url));
        m.insert("checksum".to_string(), json!(self.checksum));
        m.insert("createdAt".to_string(), json!(self.created_at));
        // A baseline cannot lie about its own capture: the settle state rides
        // in the sidecar, so `compare` can report an unsettled baseline even
        // when the current capture settled perfectly.
        if let Some(s) = self.settled {
            m.insert("settled".to_string(), json!(s));
        }
        if let Some(t) = &self.settle_timeouts {
            m.insert("settleTimeouts".to_string(), json!(t));
        }
        for (key, v) in [
            ("browserFullVersion", json!(self.full_version)),
            ("platform", json!(self.platform)),
            ("userAgent", json!(self.user_agent)),
            ("colorScheme", json!(self.color_scheme)),
            ("forcedColors", json!(self.forced_colors)),
            ("contrast", json!(self.contrast)),
            ("reducedMotion", json!(self.reduced_motion)),
            ("fontCountHash", json!(self.font_count_hash)),
            ("scrollbarWidth", json!(self.scrollbar_width)),
        ] {
            if !v.is_null() {
                m.insert(key.to_string(), v);
            }
        }
        Value::Object(m)
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
            // Optional by design: a 0.7.x sidecar has neither key, and that is
            // recorded as unknown rather than assumed settled.
            settled: v.get("settled").and_then(|s| s.as_bool()),
            settle_timeouts: v.get("settleTimeouts").and_then(|t| t.as_array()).map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            }),
            full_version: v
                .get("browserFullVersion")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string()),
            platform: v
                .get("platform")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string()),
            user_agent: v
                .get("userAgent")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string()),
            color_scheme: v
                .get("colorScheme")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string()),
            forced_colors: v.get("forcedColors").and_then(|b| b.as_bool()),
            contrast: v
                .get("contrast")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string()),
            reduced_motion: v
                .get("reducedMotion")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string()),
            font_count_hash: v
                .get("fontCountHash")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string()),
            scrollbar_width: v.get("scrollbarWidth").and_then(|n| n.as_i64()),
        })
    }

    /// Every axis that decides `environment.match`: browser major, viewport,
    /// deviceScaleFactor, and (S1.8) the full version, platform, user agent,
    /// colour scheme, forced colours, contrast preference, reduced-motion
    /// preference, the installed-font fingerprint and the scrollbar width. The
    /// url is recorded (and returned) but a baseline is allowed to be compared
    /// from a different path.
    ///
    /// Returns `(differences, unknown_axes)`. An axis the BASELINE does not
    /// record (a 0.7.x sidecar) is unknown, never a match: the caller reports
    /// it so the compare cannot look clean by omission.
    fn differences(&self, current: &EnvMeta) -> (Vec<String>, Vec<String>) {
        fn axis<T: PartialEq + std::fmt::Debug>(
            name: &str,
            base: &Option<T>,
            cur: &Option<T>,
            out: &mut Vec<String>,
            unknown: &mut Vec<String>,
        ) {
            match base {
                None => unknown.push(name.to_string()),
                Some(b) => {
                    if cur.as_ref() != Some(b) {
                        out.push(format!("{name}: {b:?} -> {cur:?}"));
                    }
                }
            }
        }
        let mut out = Vec::new();
        let mut unknown = Vec::new();
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
        axis(
            "browserFullVersion",
            &self.full_version,
            &current.full_version,
            &mut out,
            &mut unknown,
        );
        axis(
            "platform",
            &self.platform,
            &current.platform,
            &mut out,
            &mut unknown,
        );
        axis(
            "userAgent",
            &self.user_agent,
            &current.user_agent,
            &mut out,
            &mut unknown,
        );
        axis(
            "colorScheme",
            &self.color_scheme,
            &current.color_scheme,
            &mut out,
            &mut unknown,
        );
        axis(
            "forcedColors",
            &self.forced_colors,
            &current.forced_colors,
            &mut out,
            &mut unknown,
        );
        axis(
            "contrast",
            &self.contrast,
            &current.contrast,
            &mut out,
            &mut unknown,
        );
        axis(
            "reducedMotion",
            &self.reduced_motion,
            &current.reduced_motion,
            &mut out,
            &mut unknown,
        );
        axis(
            "fontCountHash",
            &self.font_count_hash,
            &current.font_count_hash,
            &mut out,
            &mut unknown,
        );
        axis(
            "scrollbarWidth",
            &self.scrollbar_width,
            &current.scrollbar_width,
            &mut out,
            &mut unknown,
        );
        (out, unknown)
    }
}

/// What the sidecar actually told us. `Missing`, `Malformed` and
/// `Unreadable` are deliberately NOT collapsed into "no metadata": treating
/// them as a match would let `requireEnvironmentMatch:true` pass without
/// evidence.
pub enum MetaState {
    Recorded(Box<EnvMeta>),
    Missing,
    Malformed(String),
    Unreadable(String),
}

impl MetaState {
    /// `missing` / `malformed` / `unreadable` / `recorded`.
    fn reason(&self) -> ToolReason {
        match self {
            MetaState::Recorded(_) => ToolReason::Recorded,
            MetaState::Missing => ToolReason::Missing,
            MetaState::Malformed(_) => ToolReason::Malformed,
            MetaState::Unreadable(_) => ToolReason::Unreadable,
        }
    }

    fn recorded(&self) -> Option<&EnvMeta> {
        match self {
            MetaState::Recorded(m) => Some(m.as_ref()),
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
            Some(m) => MetaState::Recorded(Box::new(m)),
            None => MetaState::Malformed(format!(
                "{} is not a complete environment record",
                p.display()
            )),
        },
        Err(e) => MetaState::Malformed(format!("{} is not valid JSON: {e}", p.display())),
    }
}

/// `(browser product, major version, full version)` from the engine itself.
/// Never fails the caller: an engine that will not answer `Browser.getVersion`
/// is recorded as `unknown`/0 and the mismatch logic simply never keys on it.
async fn engine_identity(browser: &Browser) -> (String, i64, String) {
    let v = match browser.cdp_raw("Browser.getVersion", json!({}), None).await {
        Ok(v) => v,
        Err(e) => {
            capture::warn(&format!("visual: Browser.getVersion failed: {e}"));
            return ("unknown".to_string(), 0, "unknown".to_string());
        }
    };
    let product = v
        .get("product")
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string();
    let version = product.rsplit('/').next().unwrap_or("").to_string();
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
    let full = if version.is_empty() {
        "unknown".to_string()
    } else {
        version
    };
    (browser, major, full)
}

/// The environment axes the PAGE can answer (S1.8): platform, user agent,
/// media preferences, the installed-font fingerprint and the scrollbar width.
/// Read live in one evaluate - never assumed, never cached across calls.
async fn live_axes(browser: &Browser) -> Value {
    let expr = "(() => { \
        const mq = (q) => { try { return matchMedia(q).matches; } catch (e) { return null; } }; \
        const faces = []; \
        try { for (const f of document.fonts) faces.push(f.family + '|' + f.weight + '|' + f.style); } catch (e) {} \
        faces.sort(); \
        const dark = mq('(prefers-color-scheme: dark)'); \
        const more = mq('(prefers-contrast: more)'); \
        const less = mq('(prefers-contrast: less)'); \
        const reduce = mq('(prefers-reduced-motion: reduce)'); \
        return { \
          platform: navigator.platform || '', \
          userAgent: navigator.userAgent || '', \
          colorScheme: dark === true ? 'dark' : 'light', \
          forcedColors: mq('(forced-colors: active)') === true, \
          contrast: more === true ? 'more' : (less === true ? 'less' : 'no-preference'), \
          reducedMotion: reduce === true ? 'reduce' : 'no-preference', \
          scrollbarWidth: window.innerWidth - document.documentElement.clientWidth, \
          fontKeys: faces.join('\\n') \
        }; })()";
    browser.eval(expr).await.unwrap_or(Value::Null)
}

async fn current_environment(browser: &Browser, cap: &capture::Capture) -> EnvMeta {
    let (product, major, full) = engine_identity(browser).await;
    let axes = live_axes(browser).await;
    let text = |k: &str| axes.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
    let font_keys = text("fontKeys").unwrap_or_default();
    EnvMeta {
        browser: product,
        browser_major: major,
        width: cap.width,
        height: cap.height,
        dsf: cap.dsf,
        url: cap.url.clone(),
        checksum: cap.checksum.clone(),
        created_at: iso8601_now(),
        settled: Some(cap.settled),
        settle_timeouts: Some(cap.settle_timeouts.clone()),
        full_version: Some(full),
        platform: text("platform"),
        user_agent: text("userAgent"),
        color_scheme: text("colorScheme"),
        forced_colors: axes.get("forcedColors").and_then(|b| b.as_bool()),
        contrast: text("contrast"),
        reduced_motion: text("reducedMotion"),
        // The same FNV-1a the capture checksum uses: cheap, dependency free,
        // and enough to catch a font set that changed under a baseline.
        font_count_hash: Some(capture::checksum(font_keys.as_bytes())),
        scrollbar_width: axes.get("scrollbarWidth").and_then(|n| n.as_i64()),
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
///
/// The cap is checked BEFORE the capture: a refused save costs no capture and
/// writes nothing (S2.4). A name whose PNG would land in the by-product
/// namespace is refused for the same reason - it could never be counted or
/// protected like the baseline it is.
pub async fn save(browser: &Browser, name: &str, full_page: bool) -> Result<Value, ToolErr> {
    let safe = sanitise_name(name).map_err(ToolErr::from)?;
    if let Some(refusal) = byproduct_namespace_refusal(&safe) {
        return Err(refusal);
    }
    if let Some(refusal) = cap_refusal(&safe) {
        return Err(refusal);
    }
    let cap = capture::capture(
        browser,
        &Options {
            full_page,
            deterministic: true,
            width: None,
            height: None,
        },
    )
    .await
    .map_err(ToolErr::from)?;
    let path = baseline_path(&safe);
    write_png(&path, &cap.bytes).map_err(ToolErr::from)?;
    let env = current_environment(browser, &cap).await;
    let meta = meta_path(&safe);
    write_json(&meta, &env.to_json()).map_err(ToolErr::from)?;
    Ok(json!({
        "ok": true, "action": "save", "name": safe,
        "path": path.to_string_lossy(),
        "metaPath": meta.to_string_lossy(),
        "bytes": cap.bytes.len() as i64,
        "checksum": cap.checksum,
        "viewport": {"width": cap.width, "height": cap.height},
        "url": cap.url,
        "settled": cap.settled,
        "settleTimeouts": cap.settle_timeouts,
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
                if !matches!(classify(&dir, &fname), StoreFile::Baseline) {
                    continue;
                }
                let bytes = e.metadata().map(|m| m.len()).unwrap_or(0);
                let name = fname.trim_end_matches(".png").to_string();
                // The settle state travels with the baseline, so `list` can
                // report it too; a 0.7.x sidecar has none and stays null.
                let settled = match read_meta(&name).recorded().and_then(|m| m.settled) {
                    Some(s) => json!(s),
                    None => Value::Null,
                };
                entries.push(json!({
                    "name": name,
                    "path": p.to_string_lossy(),
                    "bytes": bytes as i64,
                    "meta": meta_path(&name).is_file(),
                    "settled": settled,
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
    let (_, diff_count) = store_counts(&dir);
    Ok(
        json!({"ok": true, "action": "list", "dir": dir.to_string_lossy(),
        "baselines": entries, "count": count, "cap": visual_cap(),
        "diffCount": diff_count}),
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
    /// When false (the default since 0.8.0), a compare passes only if NO
    /// anti-aliased component was subtracted (`ignored_pixels == 0`).
    /// `allowIgnoredAA:true` re-admits the AA rule explicitly.
    pub allow_ignored_aa: bool,
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
            allow_ignored_aa: false,
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
    /// Pixels covered by at least one applied mask rect (union, not sum).
    pub masked_pixels: i64,
    /// The rects actually used, after clipping to the image and rounding.
    pub mask_rects_applied: Vec<Value>,
    /// Requested rects that clipped away to nothing and were not applied.
    pub mask_rects_dropped: i64,
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
        serde_json::json!({"code": ToolCode::ScratchBackgroundUnsupported.as_str(),
                           "reason": ToolReason::BackgroundParamRejected.as_str(), "tool": "visual"}),
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
    // Mask accounting (S1.3): every requested rect is clipped to the image
    // bounds and rounded to whole device pixels; a rect that is degenerate
    // after clipping is dropped and counted, never silently treated as a mask.
    // The covered pixels are a UNION: overlapping rects are not double counted.
    const applied = [];
    let dropped = 0;
    const maskedPx = new Uint8Array(n);
    const requested = OPT.maskRects || [];
    for (let k = 0; k < requested.length; k++) {{
      const m = requested[k] || {{}};
      const mx = Number(m.x) || 0, my = Number(m.y) || 0;
      const mw = Number(m.w) || 0, mh = Number(m.h) || 0;
      let x0 = Math.round(mx), y0 = Math.round(my);
      let x1 = Math.round(mx + mw), y1 = Math.round(my + mh);
      if (x0 < 0) x0 = 0;
      if (y0 < 0) y0 = 0;
      if (x1 > w) x1 = w;
      if (y1 > h) y1 = h;
      if (x1 <= x0 || y1 <= y0) {{ dropped++; continue; }}
      applied.push({{x: x0, y: y0, w: x1 - x0, h: y1 - y0}});
      for (let y = y0; y < y1; y++) {{
        const row = y * w;
        for (let x = x0; x < x1; x++) maskedPx[row + x] = 1;
      }}
    }}
    let maskedCount = 0;
    for (let p = 0; p < n; p++) if (maskedPx[p]) maskedCount++;

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
        if (d > thr && !maskedPx[p]) mask[p] = 1;
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
      maskedPixels: maskedCount, maskRectsApplied: applied, maskRectsDropped: dropped,
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
            masked_pixels: 0,
            mask_rects_applied: Vec::new(),
            mask_rects_dropped: 0,
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
        masked_pixels: num("maskedPixels"),
        mask_rects_applied: v
            .get("maskRectsApplied")
            .and_then(|a| a.as_array())
            .cloned()
            .unwrap_or_default(),
        mask_rects_dropped: num("maskRectsDropped"),
        scratch_background: true,
    })
}

/// Diff the current deterministic capture against the stored baseline, and
/// check the recorded environment.
/// True when the AA subtraction may still count as a pass. `allow_ignored_aa`
/// is FALSE by default since 0.8.0, so strictness is the default and
/// `allowIgnoredAA:true` is the explicit opt-in that re-admits the rule.
fn aa_tolerated(ignored_pixels: i64, allow_ignored_aa: bool) -> bool {
    allow_ignored_aa || ignored_pixels == 0
}

/// Warning text whenever the AA rule subtracted pixels: a pass is never
/// silently explained away, and the flag that decides whether those pixels
/// count is named as it currently stands.
fn aa_warning(ignored_pixels: i64, ignored_clusters: i64) -> Option<String> {
    if ignored_pixels <= 0 {
        return None;
    }
    Some(format!(
        "ignored_antialiasing: {ignored_pixels} pixel(s) in {ignored_clusters} cluster(s) were subtracted as anti-aliasing; a 1-2 px edge change can hide there (allowIgnoredAA:false is the default, so they fail the compare; pass allowIgnoredAA:true to re-admit the AA rule)"
    ))
}

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
            json!({"code": ToolCode::BaselineEnvironmentUnknown.as_str(),
                   "reason": meta_state.reason().as_str(), "tool": "visual", "name": safe,
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
        // The baseline on disk IS this capture now, so the identity pair is
        // stated about the file that was just written: same checksum, same
        // bytes, within any tolerance by construction.
        return Ok(json!({
            "ok": true, "action": "compare", "name": safe, "updated": true,
            "pixelsIdentical": true, "passWithinTolerance": true, "pass": true,
            "baselinePath": base.to_string_lossy(),
            "metaPath": meta_path(&safe).to_string_lossy(),
            "diffPath": Value::Null,
            "diffPixels": 0, "totalPixels": cap.width * cap.height, "ratio": 0.0,
            "bbox": Value::Null, "maxChannelDelta": 0,
            "threshold": opts.threshold, "maxDiffPixels": opts.max_diff_pixels,
            "maxDiffRatio": opts.max_diff_ratio,
            "antialias": opts.antialias, "minDiffCluster": opts.min_diff_cluster,
            "ignoredPixels": 0, "ignoredClusters": 0,
            "checksum": cap.checksum, "baselineChecksum": cap.checksum,
            "viewport": {"width": cap.width, "height": cap.height}, "url": cap.url,
            "settled": cap.settled, "settleTimeouts": cap.settle_timeouts,
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
    let mut diff_write_note: Option<String> = None;
    if let Some(b64) = &diff.diff_png_base64 {
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64.as_bytes()) {
            let p = diff_path(&safe);
            // `<stem>.diff.png` is also the path of a baseline literally named
            // `<stem>.diff` (a name an older build accepted). A by-product
            // never overwrites a stored baseline: that would be the silent
            // eviction S2.4 forbids, and the note says why no diff was written.
            if is_baseline_png(&p) {
                diff_write_note = Some(format!(
                    "diff_path_reserved: the by-product path {} is a stored baseline, so the \
                     diff image was not written over it",
                    p.display()
                ));
            } else {
                // Diff images are the only by-products: age the oldest out so
                // the store stays under its own cap (S2.4).
                age_out_diffs();
                if write_png(&p, &bytes).is_ok() {
                    wrote_diff = Some(p.to_string_lossy().to_string());
                }
            }
        }
    }

    // Environment check: a mismatch (or an unknown baseline environment) is a
    // WARNING by default - the pixels still decide - and a failure only when
    // the caller requires a match.
    let (env_known, env_match, mut notes, unknown_axes) = match meta_state.recorded() {
        Some(b) => {
            let (diffs, unknown) = b.differences(&current_env);
            // An unsettled baseline is not a match: the artifact carries its
            // own honesty, so a strict gate cannot pass on pixels that were
            // captured mid-flight. `None` is a 0.7.x sidecar: unknown, never
            // true.
            if b.settled == Some(false) && opts.require_environment_match {
                return Err(ToolErr::typed(
                    format!(
                        "visual: baseline_environment_unsettled: the baseline was captured \
                         unsettled (settleTimeouts: {}) and requireEnvironmentMatch:true needs a \
                         settled baseline; re-save it with update:true",
                        b.settle_timeouts
                            .as_ref()
                            .map(|t| t.join(", "))
                            .unwrap_or_else(|| "unknown".to_string())
                    ),
                    json!({"code": ToolCode::BaselineEnvironmentUnsettled.as_str(), "reason": ToolReason::Unsettled.as_str(),
                           "tool": "visual", "name": safe,
                           "settleTimeouts": b.settle_timeouts}),
                ));
            }
            let mut out: Vec<String> = Vec::new();
            if !diffs.is_empty() {
                out.push(format!(
                    "baseline_environment_mismatch: {} (the baseline was recorded in a \
                     different environment; re-save it with update:true after confirming \
                     the change is expected)",
                    diffs.join("; ")
                ));
            }
            // An axis the sidecar does not record is UNKNOWN, and unknown is
            // never a match: a 0.7.x baseline must not look clean by omission.
            if !unknown.is_empty() {
                out.push(format!(
                    "baseline_environment_unknown_axes: the baseline records no {} (a 0.7.x \
                     sidecar), so a match cannot be established for them; re-save it with \
                     update:true",
                    unknown.join(", ")
                ));
            }
            match b.settled {
                Some(false) => out.push(format!(
                    "baseline_unsettled: the baseline was captured before it settled \
                     (settleTimeouts: {}), so it records a moment that may not repeat",
                    b.settle_timeouts
                        .as_ref()
                        .map(|t| t.join(", "))
                        .unwrap_or_else(|| "unknown".to_string())
                )),
                None => out.push(
                    "baseline_settle_unknown: the sidecar records no settle state (0.7.x \
                     baseline), so whether it settled cannot be established; re-save it with \
                     update:true"
                        .to_string(),
                ),
                Some(true) => {}
            }
            (true, diffs.is_empty(), out, unknown)
        }
        None => (
            false,
            false,
            vec![format!(
                "baseline_environment_unknown: {} (the baseline carries no usable environment \
                 record, so a match cannot be established; re-save it with update:true)",
                meta_state.detail()
            )],
            Vec::new(),
        ),
    };
    if let Some(note) = diff_write_note {
        notes.push(note);
    }
    if let Some(aa) = aa_warning(diff.ignored_pixels, diff.ignored_clusters) {
        notes.push(aa);
    }
    // Mask accounting: a mask is a legitimate tool, an unmetered one is not.
    // None of these is a failure - the counts are true either way - but the
    // reply never lets a masked regression pass as an unexplained silence.
    if diff.mask_rects_dropped > 0 {
        notes.push(format!(
            "mask_rect_dropped: {} requested mask rect(s) were degenerate after clipping to \
             the image and were not applied",
            diff.mask_rects_dropped
        ));
    }
    let mask_coverage = if diff.total_pixels > 0 {
        diff.masked_pixels as f64 / diff.total_pixels as f64
    } else {
        0.0
    };
    if mask_coverage > 0.10 {
        notes.push(format!(
            "mask_coverage: {:.1}% of the diff area was masked; a regression inside a mask \
             is invisible",
            mask_coverage * 100.0
        ));
    }
    if !opts.mask_rects.is_empty() && (cap.dsf - 1.0).abs() > 0.001 {
        notes.push(format!(
            "mask_scale_mismatch: deviceScaleFactor {} - mask rects are device pixels of the \
             captured PNG, so they do not line up with CSS pixels",
            cap.dsf
        ));
    }
    let warning = if notes.is_empty() {
        Value::Null
    } else {
        json!(notes.join("; "))
    };
    let pixel_pass = aa_tolerated(diff.ignored_pixels, opts.allow_ignored_aa)
        && !diff.size_mismatch
        && diff.diff_pixels <= opts.max_diff_pixels
        && ratio <= opts.max_diff_ratio;
    // Two questions, two booleans, no summary that can disagree with either:
    // `pixels_identical` is a fact about the bytes, `pass_within_tolerance` is
    // the judgement. `pass` is the 0.7.x name and is emitted as the alias, so
    // it can never say something the judgement does not.
    let pixels_identical = baseline_checksum == cap.checksum;
    let pass_within_tolerance = pixel_pass && (env_match || !opts.require_environment_match);
    Ok(json!({
        "ok": true, "action": "compare", "name": safe, "updated": false,
        "pixelsIdentical": pixels_identical,
        "passWithinTolerance": pass_within_tolerance,
        "pass": pass_within_tolerance,
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
        "maskedPixels": diff.masked_pixels,
        "maskCoverageRatio": mask_coverage,
        "maskRectsApplied": diff.mask_rects_applied,
        "sizeMismatch": diff.size_mismatch,
        "size": {"width": diff.width, "height": diff.height},
        "baselineChecksum": baseline_checksum,
        "checksum": cap.checksum,
        "viewport": {"width": cap.width, "height": cap.height},
        "url": cap.url,
        "settled": cap.settled,
        "settleTimeouts": cap.settle_timeouts,
        "environment": {
            "match": env_match,
            "known": env_known,
            "unknownAxes": unknown_axes,
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

    /// A 1-2 px anti-aliased component can hide a real edge change, so since
    /// 0.8.0 the DEFAULT requires a zero-ignored diff and
    /// `allowIgnoredAA:true` is the explicit opt-in; a warning always names
    /// what was ignored.
    #[test]
    fn ignored_aa_can_be_made_a_pass_condition() {
        assert!(aa_tolerated(0, false), "zero ignored is a clean pass");
        assert!(!aa_tolerated(2, false), "allowIgnoredAA:false rejects 2 px");
        assert!(aa_tolerated(2, true), "allowIgnoredAA:true admits AA");
        assert!(
            !DiffOpts::default().allow_ignored_aa,
            "strictness is the default, not the opt-in"
        );
        assert!(aa_warning(0, 0).is_none(), "no ignored pixels, no warning");
        let w = aa_warning(2, 1).expect("warning names the subtraction");
        assert!(w.contains("2 pixel(s)") && w.contains("allowIgnoredAA:false"));
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
