//! Layout geometry without pixels.
//!
//! - `assert_layout`: one call proves a set of geometric invariants for one
//!   or many targets (`exists`, `visible`, `stableFrames`, `box`, `hitTarget`,
//!   `computed`, `noOverflow`, `inViewport`) and returns a per-check record.
//! - `layout`: the "where is it actually" primitive - a box, scroll size,
//!   computed display/visibility/opacity, a z-order hint and every detected
//!   overlapping pair for each match of a selector or ref list.
//!
//! Every measured value comes from the engine: `DOM.getBoxModel` /
//! `DOM.getContentQuads` for boxes (page coordinates, converted to viewport
//! coordinates by the live scroll offset) and `Runtime.evaluate` for
//! `getBoundingClientRect`, scroll sizes, computed styles, paint order and
//! hit-testing. Nothing is estimated, and the two geometry sources are
//! reported side by side (`box` vs `rect`) so a disagreement is visible
//! rather than hidden.
//!
//! Main frame only: a ref (`@eN`) or a CSS selector resolves in the active
//! page's document.

use serde_json::{json, Value};

use crate::browser::Browser;
use crate::tools::ToolErr;

/// One requested target: an AX ref id (`eN`) or a CSS selector.
#[derive(Clone)]
pub enum Spec {
    Ref(String),
    Selector(String),
}

impl Spec {
    fn selector(&self) -> String {
        match self {
            // The ref attribute the snapshot stamps on every node it names.
            Spec::Ref(id) => format!("[data-gladius-ref=\"{id}\"]"),
            Spec::Selector(s) => s.clone(),
        }
    }

    fn label(&self) -> String {
        match self {
            Spec::Ref(id) => format!("@{id}"),
            Spec::Selector(s) => s.clone(),
        }
    }
}

/// A resolved DOM node: which target label it answers to, the selector that
/// found it, its position in that selector's match list, and its node id.
struct Node {
    label: String,
    selector: String,
    index: usize,
    node_id: i64,
}

/// A box in CSS px. `doc` uses the engine's page coordinates,
/// `viewport` has the live scroll offset removed (what is on screen and what
/// `Input.dispatchMouseEvent` / a screenshot clip uses).
#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn to_json(self) -> Value {
        json!({"x": self.x, "y": self.y, "w": self.w, "h": self.h})
    }

    fn intersect(self, o: Rect) -> Option<(Rect, f64)> {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let y1 = (self.y + self.h).min(o.y + o.h);
        let w = x1 - x0;
        let h = y1 - y0;
        if w > 0.0 && h > 0.0 {
            Some((Rect { x: x0, y: y0, w, h }, w * h))
        } else {
            None
        }
    }
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|n| n.as_f64())
}

fn rect_of(v: &Value) -> Option<Rect> {
    Some(Rect {
        x: num(v, "x")?,
        y: num(v, "y")?,
        w: num(v, "w")?,
        h: num(v, "h")?,
    })
}

/// Parse a quad (`[x1,y1,...]`, 8 numbers) into its axis-aligned bounds.
fn quad_rect(q: &Value) -> Option<Rect> {
    let a = q.as_array()?;
    if a.len() < 8 {
        return None;
    }
    let mut xs: Vec<f64> = Vec::with_capacity(4);
    let mut ys: Vec<f64> = Vec::with_capacity(4);
    for i in 0..4 {
        xs.push(a.get(i * 2)?.as_f64()?);
        ys.push(a.get(i * 2 + 1)?.as_f64()?);
    }
    let minx = xs.iter().cloned().fold(f64::INFINITY, f64::min);
    let maxx = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let miny = ys.iter().cloned().fold(f64::INFINITY, f64::min);
    let maxy = ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    Some(Rect {
        x: minx,
        y: miny,
        w: maxx - minx,
        h: maxy - miny,
    })
}

async fn root_node(browser: &Browser) -> Result<i64, ToolErr> {
    let r = browser
        .command(
            "DOM.getDocument",
            json!({"depth": 0, "pierce": false}),
            true,
        )
        .await
        .map_err(ToolErr::from)?;
    r.get("root")
        .and_then(|r| r.get("nodeId"))
        .and_then(|v| v.as_i64())
        .ok_or_else(|| {
            ToolErr::fallback("layout: DOM.getDocument returned no root nodeId".to_string())
        })
}

/// Resolve every spec to its matching nodes (document order). An invalid
/// selector is a typed `invalid_args`/`bad_value`, never a silent miss.
async fn resolve(browser: &Browser, tool: &str, specs: &[Spec]) -> Result<Vec<Node>, ToolErr> {
    let root = root_node(browser).await?;
    let mut out: Vec<Node> = Vec::new();
    for spec in specs {
        let selector = spec.selector();
        let r = browser
            .command(
                "DOM.querySelectorAll",
                json!({"nodeId": root, "selector": selector}),
                true,
            )
            .await
            .map_err(|e| {
                ToolErr::typed(
                    format!("{tool}: invalid selector '{}' ({e})", spec.label()),
                    json!({"code": "invalid_args", "reason": "bad_value", "tool": tool,
                           "arg": "selector", "selector": spec.label()}),
                )
            })?;
        let ids: Vec<i64> = r
            .get("nodeIds")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_i64()).collect())
            .unwrap_or_default();
        let many = ids.len() > 1;
        for (i, id) in ids.into_iter().enumerate() {
            out.push(Node {
                label: if many {
                    format!("{}[{}]", spec.label(), i)
                } else {
                    spec.label()
                },
                selector: selector.clone(),
                index: i,
                node_id: id,
            });
        }
    }
    Ok(out)
}

/// Border box from `DOM.getBoxModel` (page coordinates) plus the content quad
/// count from `DOM.getContentQuads`. `None` means the node has no layout box
/// (display:none, detached, zero-size) - a measurement, not an error.
async fn box_geom(browser: &Browser, node_id: i64) -> Result<Option<(Rect, i64)>, ToolErr> {
    let bm = match browser
        .command("DOM.getBoxModel", json!({"nodeId": node_id}), true)
        .await
    {
        Ok(v) => v,
        // The engine's own words for "this node has no box".
        Err(e)
            if e.contains("Could not compute box model")
                || e.contains("no layout object")
                || e.contains("No node") =>
        {
            return Ok(None)
        }
        Err(e) => return Err(ToolErr::fallback(e)),
    };
    let quad = bm
        .get("model")
        .and_then(|m| m.get("border"))
        .cloned()
        .unwrap_or(Value::Null);
    let Some(rect) = quad_rect(&quad) else {
        return Ok(None);
    };
    let quads = browser
        .command("DOM.getContentQuads", json!({"nodeId": node_id}), true)
        .await
        .ok()
        .and_then(|v| {
            v.get("quads")
                .and_then(|q| q.as_array())
                .map(|a| a.len() as i64)
        })
        .unwrap_or(0);
    Ok(Some((rect, quads)))
}

/// Live element facts from `Runtime.evaluate`: rect, scroll size, computed
/// display/visibility/opacity (plus the requested extra properties), paint
/// order, hit test at the centre and the viewport box.
async fn element_info(
    browser: &Browser,
    node: &Node,
    computed_keys: &[String],
) -> Result<Option<Value>, ToolErr> {
    let sel =
        serde_json::to_string(&node.selector).map_err(|e| ToolErr::fallback(e.to_string()))?;
    let keys =
        serde_json::to_string(computed_keys).map_err(|e| ToolErr::fallback(e.to_string()))?;
    let idx = node.index;
    let expr = format!(
        r#"(() => {{
  const el = document.querySelectorAll({sel})[{idx}];
  if (!el) return null;
  const cs = getComputedStyle(el);
  const r = el.getBoundingClientRect();
  const extra = {{}};
  for (const k of {keys}) {{
    const kebab = String(k).replace(/[A-Z]/g, (m) => '-' + m.toLowerCase());
    extra[k] = cs.getPropertyValue(kebab) || '';
  }}
  let hit = null;
  try {{ hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2); }} catch (e) {{ hit = null; }}
  return {{
    rect: {{x: r.x, y: r.y, w: r.width, h: r.height}},
    scroll: {{width: el.scrollWidth, height: el.scrollHeight,
              clientWidth: el.clientWidth, clientHeight: el.clientHeight,
              top: el.scrollTop, left: el.scrollLeft}},
    computed: {{display: cs.display, visibility: cs.visibility, opacity: cs.opacity,
                position: cs.position, zIndex: cs.zIndex}},
    extra: extra,
    hit: hit ? {{self: hit === el, contains: el.contains(hit),
                 tag: (hit.tagName || '').toUpperCase(), id: hit.id || ''}} : null,
    paintIndex: Array.prototype.indexOf.call(document.querySelectorAll('*'), el),
    viewport: {{width: window.innerWidth, height: window.innerHeight,
                scrollX: window.scrollX, scrollY: window.scrollY}}
  }};
}})()"#
    );
    let v = browser.eval(&expr).await.map_err(ToolErr::from)?;
    if v.is_null() {
        return Ok(None);
    }
    Ok(Some(v))
}

/// Sample the element's viewport rect once per animation frame, `frames`
/// times. Each wait is raced against a timer so a background/shadowed page
/// that starves rAF still returns a sample set instead of hanging.
async fn stable_sample(browser: &Browser, node: &Node, frames: i64) -> Result<Vec<Rect>, ToolErr> {
    let sel =
        serde_json::to_string(&node.selector).map_err(|e| ToolErr::fallback(e.to_string()))?;
    let idx = node.index;
    let n = frames.clamp(1, 60);
    let expr = format!(
        r#"(() => {{
  const el = document.querySelectorAll({sel})[{idx}];
  if (!el) return null;
  const out = [];
  const tick = () => Promise.race([
    new Promise((res) => requestAnimationFrame(() => res(true))),
    new Promise((res) => setTimeout(() => res(true), 200))
  ]);
  return (async () => {{
    for (let k = 0; k < {n}; k++) {{
      const r = el.getBoundingClientRect();
      out.push([r.x, r.y, r.width, r.height]);
      await tick();
    }}
    return out;
  }})();
}})()"#
    );
    let v = browser.eval(&expr).await.map_err(ToolErr::from)?;
    let arr = v.as_array().cloned().unwrap_or_default();
    let mut out = Vec::with_capacity(arr.len());
    for q in arr {
        let a = q.as_array().cloned().unwrap_or_default();
        let g = |i: usize| a.get(i).and_then(|x| x.as_f64()).unwrap_or(0.0);
        out.push(Rect {
            x: g(0),
            y: g(1),
            w: g(2),
            h: g(3),
        });
    }
    Ok(out)
}

fn visible_of(info: &Value, has_box: bool) -> bool {
    let c = info.get("computed").cloned().unwrap_or(Value::Null);
    let display = c.get("display").and_then(|v| v.as_str()).unwrap_or("");
    let vis = c.get("visibility").and_then(|v| v.as_str()).unwrap_or("");
    let opacity = c
        .get("opacity")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(1.0);
    has_box && display != "none" && vis != "hidden" && vis != "collapse" && opacity > 0.0
}

// ------------------------------------------------------------ assert_layout

/// Parsed `checks` input.
struct Checks {
    expect_exists: bool,
    visible: bool,
    stable_frames: Option<i64>,
    expected_box: Option<Value>,
    hit_target: bool,
    computed: Option<serde_json::Map<String, Value>>,
    no_overflow: Option<f64>,
    in_viewport: bool,
}

fn parse_checks(tool: &str, checks: Option<&Value>) -> Result<Checks, ToolErr> {
    let mut c = Checks {
        expect_exists: true,
        visible: true,
        stable_frames: None,
        expected_box: None,
        hit_target: false,
        computed: None,
        no_overflow: None,
        in_viewport: false,
    };
    let Some(v) = checks else { return Ok(c) };
    let Some(obj) = v.as_object() else {
        return Err(ToolErr::typed(
            format!("{tool}: 'checks' must be an object"),
            json!({"code": "invalid_args", "reason": "bad_value", "tool": tool, "arg": "checks"}),
        ));
    };
    for (k, val) in obj {
        match k.as_str() {
            "exists" | "visible" | "hitTarget" | "inViewport" => {
                let Some(b) = val.as_bool() else {
                    return Err(bad_check(tool, k, "boolean"));
                };
                match k.as_str() {
                    "exists" => c.expect_exists = b,
                    "visible" => c.visible = b,
                    "hitTarget" => c.hit_target = b,
                    _ => c.in_viewport = b,
                }
            }
            "stableFrames" => {
                let Some(n) = val.as_i64() else {
                    return Err(bad_check(tool, k, "integer"));
                };
                if n < 1 {
                    return Err(bad_check(tool, k, "integer >= 1"));
                }
                c.stable_frames = Some(n);
            }
            "box" => {
                if !val.is_object() {
                    return Err(bad_check(tool, k, "object {x,y,w,h,tolerance}"));
                }
                c.expected_box = Some(val.clone());
            }
            "computed" => {
                let Some(m) = val.as_object() else {
                    return Err(bad_check(tool, k, "object of property -> value"));
                };
                c.computed = Some(m.clone());
            }
            "noOverflow" => match val {
                Value::Bool(true) => c.no_overflow = Some(0.0),
                Value::Bool(false) => c.no_overflow = None,
                Value::Number(n) => c.no_overflow = Some(n.as_f64().unwrap_or(0.0)),
                Value::Object(o) => {
                    c.no_overflow = Some(o.get("tolerance").and_then(|t| t.as_f64()).unwrap_or(0.0))
                }
                _ => return Err(bad_check(tool, k, "boolean, number or {tolerance}")),
            },
            other => {
                return Err(ToolErr::typed(
                    format!("{tool}: unknown check '{other}'"),
                    json!({"code": "invalid_args", "reason": "unknown_arg", "tool": tool, "arg": other}),
                ))
            }
        }
    }
    Ok(c)
}

fn bad_check(tool: &str, key: &str, want: &str) -> ToolErr {
    ToolErr::typed(
        format!("{tool}: check '{key}' must be {want}"),
        json!({"code": "invalid_args", "reason": "bad_value", "tool": tool, "arg": key}),
    )
}

fn push(
    out: &mut Vec<Value>,
    target: &str,
    check: &str,
    pass: bool,
    expected: Value,
    actual: Value,
    delta: Option<f64>,
) {
    out.push(json!({"target": target, "check": check, "pass": pass,
                    "expected": expected, "actual": actual, "delta": delta}));
}

/// Compare one measured component, returning `(pass, delta)`.
fn cmp(actual: f64, expected: f64, tol: f64) -> (bool, f64) {
    let d = actual - expected;
    ((d.abs() <= tol), d)
}

/// `assert_layout`: prove geometry without pixels.
pub async fn assert_layout(
    browser: &Browser,
    specs: &[Spec],
    checks: Option<&Value>,
    screenshot_on_fail: bool,
    screenshot_path: Option<&str>,
) -> Result<Value, ToolErr> {
    let wanted = parse_checks("assert_layout", checks)?;
    let nodes = resolve(browser, "assert_layout", specs).await?;
    let computed_keys: Vec<String> = wanted
        .computed
        .as_ref()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();

    // Requested targets in request order. A spec that resolved to nothing
    // becomes its own slot, so a missing element is a FAILING CHECK (with
    // expected/actual) rather than an error.
    enum Slot {
        Missing(String),
        Node(usize),
    }
    let mut order: Vec<Slot> = Vec::new();
    for spec in specs {
        let sel = spec.selector();
        let found: Vec<usize> = nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.selector == sel)
            .map(|(i, _)| i)
            .collect();
        if found.is_empty() {
            order.push(Slot::Missing(spec.label()));
        } else {
            for i in found {
                order.push(Slot::Node(i));
            }
        }
    }

    let mut results: Vec<Value> = Vec::new();
    let mut targets: Vec<Value> = Vec::new();
    for slot in &order {
        let node = match slot {
            Slot::Node(i) => Some(&nodes[*i]),
            Slot::Missing(label) => {
                // Absent: the exists check answers for it, and nothing else
                // can be measured. `exists:false` turns this into a pass.
                push(
                    &mut results,
                    label,
                    "exists",
                    !wanted.expect_exists,
                    json!(wanted.expect_exists),
                    json!(false),
                    None,
                );
                targets.push(
                    json!({"target": label, "found": false, "nodeId": Value::Null,
                                    "box": Value::Null, "documentBox": Value::Null, "quads": 0}),
                );
                None
            }
        };
        let Some(node) = node else { continue };
        let label = node.label.clone();
        let geom = box_geom(browser, node.node_id).await?;
        let info = element_info(browser, node, &computed_keys).await?;
        let Some(info) = info else {
            push(
                &mut results,
                &label,
                "exists",
                !wanted.expect_exists,
                json!(wanted.expect_exists),
                json!(false),
                None,
            );
            targets.push(
                json!({"target": label.clone(), "found": false, "nodeId": node.node_id,
                                "box": Value::Null, "documentBox": Value::Null, "quads": 0}),
            );
            continue;
        };
        let viewport = info.get("viewport").cloned().unwrap_or(Value::Null);
        let scroll_x = num(&viewport, "scrollX").unwrap_or(0.0);
        let scroll_y = num(&viewport, "scrollY").unwrap_or(0.0);
        let js_rect = info.get("rect").and_then(rect_of).unwrap_or(Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        });
        let doc_rect = geom.map(|(r, _)| r);
        let quads = geom.map(|(_, q)| q).unwrap_or(0);
        // Viewport coordinates from the engine's own box model.
        let box_rect = doc_rect.map(|r| Rect {
            x: r.x - scroll_x,
            y: r.y - scroll_y,
            w: r.w,
            h: r.h,
        });
        let has_box = box_rect.is_some() && js_rect.w > 0.0 && js_rect.h > 0.0;
        targets.push(json!({
            "target": label.clone(), "found": true, "nodeId": node.node_id,
            "box": box_rect.map(|r| r.to_json()).unwrap_or(Value::Null),
            "documentBox": doc_rect.map(|r| r.to_json()).unwrap_or(Value::Null),
            "rect": js_rect.to_json(),
            "quads": quads,
            "viewport": viewport,
        }));

        // exists: found at all (a node id and a JS handle both resolved).
        push(
            &mut results,
            &label,
            "exists",
            wanted.expect_exists,
            json!(wanted.expect_exists),
            json!(true),
            None,
        );

        // visible: a real box, not display:none/visibility:hidden, opacity>0.
        if wanted.visible {
            let vis = visible_of(&info, has_box);
            push(
                &mut results,
                &label,
                "visible",
                vis,
                json!(true),
                json!({"hasBox": has_box,
                       "display": info.pointer("/computed/display").cloned().unwrap_or(Value::Null),
                       "visibility": info.pointer("/computed/visibility").cloned().unwrap_or(Value::Null),
                       "opacity": info.pointer("/computed/opacity").cloned().unwrap_or(Value::Null)}),
                None,
            );
        }

        // box: viewport-relative CSS px, per-component tolerance.
        if let Some(spec) = &wanted.expected_box {
            let tol = spec
                .get("tolerance")
                .and_then(|t| t.as_f64())
                .unwrap_or(0.5);
            let measured = box_rect.unwrap_or(Rect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            });
            let mut pass = box_rect.is_some();
            let mut delta = 0.0f64;
            for (k, got) in [
                ("x", measured.x),
                ("y", measured.y),
                ("w", measured.w),
                ("h", measured.h),
            ] {
                if let Some(want) = spec.get(k).and_then(|v| v.as_f64()) {
                    let (ok, d) = cmp(got, want, tol);
                    pass &= ok;
                    if d.abs() > delta.abs() {
                        delta = d;
                    }
                }
            }
            push(
                &mut results,
                &label,
                "box",
                pass,
                spec.clone(),
                measured.to_json(),
                Some(delta),
            );
        }

        // stableFrames: identical bbox across N animation frames.
        if let Some(n) = wanted.stable_frames {
            let samples = stable_sample(browser, node, n).await?;
            let mut delta = 0.0f64;
            let mut pass = samples.len() as i64 == n;
            if let Some(first) = samples.first() {
                for s in &samples {
                    for (a, b) in [
                        (s.x, first.x),
                        (s.y, first.y),
                        (s.w, first.w),
                        (s.h, first.h),
                    ] {
                        let d = (a - b).abs();
                        if d > delta {
                            delta = d;
                        }
                    }
                }
                pass &= delta == 0.0;
            }
            push(
                &mut results,
                &label,
                "stableFrames",
                pass,
                json!({"frames": n, "identical": true}),
                json!({"frames": samples.len(), "delta": delta,
                       "rects": samples.iter().map(|r| r.to_json()).collect::<Vec<_>>()}),
                Some(delta),
            );
        }

        // hitTarget: the topmost element at the centre is the target or a
        // descendant of it (the same guard `click` uses).
        if wanted.hit_target {
            let hit = info.get("hit").cloned().unwrap_or(Value::Null);
            let same = hit.get("self").and_then(|b| b.as_bool()).unwrap_or(false);
            let contains = hit
                .get("contains")
                .and_then(|b| b.as_bool())
                .unwrap_or(false);
            push(
                &mut results,
                &label,
                "hitTarget",
                has_box && (same || contains),
                json!(true),
                json!({"point": {"x": js_rect.x + js_rect.w / 2.0, "y": js_rect.y + js_rect.h / 2.0},
                       "self": same, "contains": contains,
                       "hit": hit}),
                None,
            );
        }

        // computed: requested subset of getComputedStyle, string-exact.
        if let Some(want) = &wanted.computed {
            let extra = info.get("extra").cloned().unwrap_or(Value::Null);
            let mut actual = serde_json::Map::new();
            let mut pass = true;
            for (k, expect) in want {
                let got = extra.get(k).cloned().unwrap_or(Value::Null);
                let expect_s = match expect {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                let got_s = got.as_str().unwrap_or("").to_string();
                if got_s != expect_s {
                    pass = false;
                }
                actual.insert(k.clone(), got);
            }
            push(
                &mut results,
                &label,
                "computed",
                pass,
                Value::Object(want.clone()),
                Value::Object(actual),
                None,
            );
        }

        // noOverflow: scroll size within the client size (+ tolerance).
        if let Some(tol) = wanted.no_overflow {
            let s = info.get("scroll").cloned().unwrap_or(Value::Null);
            let sw = num(&s, "width").unwrap_or(0.0);
            let sh = num(&s, "height").unwrap_or(0.0);
            let cw = num(&s, "clientWidth").unwrap_or(0.0);
            let ch = num(&s, "clientHeight").unwrap_or(0.0);
            let dx = (sw - cw - tol).max(0.0);
            let dy = (sh - ch - tol).max(0.0);
            push(
                &mut results,
                &label,
                "noOverflow",
                has_box && dx <= 0.0 && dy <= 0.0,
                json!({"tolerance": tol}),
                json!({"scrollWidth": sw, "clientWidth": cw,
                       "scrollHeight": sh, "clientHeight": ch}),
                Some(dx.max(dy)),
            );
        }

        // inViewport: the whole box lies inside the visual viewport.
        if wanted.in_viewport {
            let vw = num(&viewport, "width").unwrap_or(0.0);
            let vh = num(&viewport, "height").unwrap_or(0.0);
            let over = [
                -js_rect.x,
                -js_rect.y,
                js_rect.x + js_rect.w - vw,
                js_rect.y + js_rect.h - vh,
            ]
            .iter()
            .cloned()
            .fold(f64::NEG_INFINITY, f64::max);
            push(
                &mut results,
                &label,
                "inViewport",
                has_box && over <= 0.5,
                json!({"viewport": {"width": vw, "height": vh}}),
                json!({"rect": js_rect.to_json(), "overshoot": over.max(0.0)}),
                Some(over.max(0.0)),
            );
        }
    }

    let pass = !results.is_empty()
        && results
            .iter()
            .all(|c| c.get("pass").and_then(|b| b.as_bool()).unwrap_or(false));
    let mut out = json!({"pass": pass, "checks": results, "targets": targets});
    if !pass && screenshot_on_fail {
        let cap = crate::capture::capture(
            browser,
            &crate::capture::Options {
                full_page: false,
                deterministic: true,
                width: None,
                height: None,
            },
        )
        .await?;
        let path = match screenshot_path {
            Some(p) => std::path::PathBuf::from(p),
            None => crate::visual::visual_dir()
                .join("failures")
                .join(format!("assert-layout-{}.png", cap.checksum)),
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                ToolErr::fallback(format!("assert_layout: create {}: {e}", parent.display()))
            })?;
        }
        std::fs::write(&path, &cap.bytes).map_err(|e| {
            ToolErr::fallback(format!("assert_layout: write {}: {e}", path.display()))
        })?;
        out["screenshot"] = json!({
            "path": path.to_string_lossy(),
            "bytes": cap.bytes.len() as i64,
            "checksum": cap.checksum,
            "viewport": {"width": cap.width, "height": cap.height},
            "url": cap.url,
        });
    }
    Ok(out)
}

// ------------------------------------------------------------------- layout

/// `layout`: boxes, scroll sizes, computed visibility and overlapping pairs
/// for every match of the requested selectors/refs.
pub async fn layout(
    browser: &Browser,
    specs: &[Spec],
    include_overlaps: bool,
) -> Result<Value, ToolErr> {
    let nodes = resolve(browser, "layout", specs).await?;
    let mut elements: Vec<Value> = Vec::new();
    let mut rects: Vec<(String, Rect, bool)> = Vec::new();
    // The viewport is reported even when nothing matched: the schema promises
    // an object, and "no elements" still has a viewport. Read once, up front,
    // and use it for the scroll offsets.
    let viewport = match browser
        .eval("[window.innerWidth, window.innerHeight, window.scrollX, window.scrollY]")
        .await
    {
        Ok(v) => {
            let a = v.as_array().cloned().unwrap_or_default();
            let g = |i: usize| a.get(i).and_then(|x| x.as_f64()).unwrap_or(0.0);
            json!({"width": g(0), "height": g(1), "scrollX": g(2), "scrollY": g(3)})
        }
        Err(e) => return Err(ToolErr::from(e)),
    };
    for node in &nodes {
        let info = element_info(browser, node, &[]).await?;
        let geom = box_geom(browser, node.node_id).await?;
        let Some(info) = info else { continue };
        let scroll_x = num(&viewport, "scrollX").unwrap_or(0.0);
        let scroll_y = num(&viewport, "scrollY").unwrap_or(0.0);
        let js_rect = info.get("rect").and_then(rect_of).unwrap_or(Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        });
        let doc_rect = geom.map(|(r, _)| r);
        let quads = geom.map(|(_, q)| q).unwrap_or(0);
        let box_rect = doc_rect.map(|r| Rect {
            x: r.x - scroll_x,
            y: r.y - scroll_y,
            w: r.w,
            h: r.h,
        });
        let has_box = box_rect.is_some() && js_rect.w > 0.0 && js_rect.h > 0.0;
        let vis = visible_of(&info, has_box);
        let computed = info.get("computed").cloned().unwrap_or(Value::Null);
        let stacking = {
            let pos = computed
                .get("position")
                .and_then(|v| v.as_str())
                .unwrap_or("static");
            let z = computed
                .get("zIndex")
                .and_then(|v| v.as_str())
                .unwrap_or("auto");
            let opacity = computed
                .get("opacity")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(1.0);
            (pos != "static" && z != "auto") || opacity < 1.0 || pos == "fixed" || pos == "sticky"
        };
        let paint = info
            .get("paintIndex")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        elements.push(json!({
            "target": node.label,
            "nodeId": node.node_id,
            "found": true,
            "rect": js_rect.to_json(),
            "box": box_rect.map(|r| r.to_json()).unwrap_or(Value::Null),
            "documentBox": doc_rect.map(|r| r.to_json()).unwrap_or(Value::Null),
            "quads": quads,
            "scroll": info.get("scroll").cloned().unwrap_or(Value::Null),
            "computed": computed,
            "zOrder": {"paintIndex": paint,
                       "zIndex": info.pointer("/computed/zIndex").cloned().unwrap_or(Value::Null),
                       "position": info.pointer("/computed/position").cloned().unwrap_or(Value::Null),
                       "stackingContext": stacking},
            "visible": vis,
        }));
        rects.push((node.label.clone(), js_rect, vis));
    }

    let mut overlaps: Vec<Value> = Vec::new();
    if include_overlaps {
        for i in 0..rects.len() {
            for j in (i + 1)..rects.len() {
                let (a, ra, va) = &rects[i];
                let (b, rb, vb) = &rects[j];
                if !*va || !*vb {
                    continue;
                }
                if let Some((r, area)) = ra.intersect(*rb) {
                    overlaps.push(json!({"a": a, "b": b, "rect": r.to_json(), "area": area}));
                }
            }
        }
    }

    let found = elements.len();
    let requested: usize = specs.len();
    Ok(json!({
        "elements": elements,
        "overlaps": overlaps,
        "viewport": viewport,
        "count": found,
        "requested": requested,
    }))
}
