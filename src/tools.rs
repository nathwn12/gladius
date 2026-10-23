use serde_json::{json, Value};

use crate::browser::Browser;
use crate::fetch::fetch_text;

pub fn list() -> Value {
    let mut listed = json!({"tools": [
        {"name": "navigate", "title": "Navigate",
         "description": "Navigate the active page to a URL",
         "inputSchema": {"type": "object", "properties": {"url": {"type": "string"}}, "required": ["url"], "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {"url": {"type": "string"}, "title": {"type": "string"}, "warning": {"type": "string"}},
            "required": ["url", "title"], "additionalProperties": false},
         "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": true}},
        {"name": "snapshot", "title": "Snapshot",
         "description": "Snapshot: one line per node with a stable ref (@eN, unique per snapshot), role/name/value, plus page text. Default is the real Accessibility tree (shadow + same-process frames; hidden/ignored subtrees excluded). mode:\"legacy\" keeps the old DOM heuristic; mode:\"text\" is text-only. frame=<CDP frameId> from targets scopes the snapshot to that iframe (same-process first; OOPIF is a follow-up). Shadow refs resolve in the snapshot but acting on a shadow ref through the main-document resolver is not yet supported (typed stale_ref, never a faked click).",
         "inputSchema": {"type": "object", "properties": {"mode": {"type": "string", "enum": ["ax", "legacy", "text"]}, "frame": {"type": "string"}}, "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {
                "url": {"type": "string"}, "title": {"type": "string"},
                "truncated": {"type": "boolean"},
                "nodes": {"type": "array", "items": {"type": "object",
                    "properties": {"ref": {"type": "string"}, "role": {"type": "string"},
                        "name": {"type": "string"}, "tag": {"type": "string"},
                        "disabled": {"type": "boolean"}, "value": {"type": "string"}},
                    "required": ["ref", "role", "name", "tag", "disabled", "value"],
                    "additionalProperties": false}},
                "mode": {"type": "string"}, "text": {"type": "string"}},
            "required": [], "additionalProperties": false,
            "anyOf": [{"required": ["url", "title", "truncated", "nodes"]}, {"required": ["mode", "text"]}]},
         "annotations": {"readOnlyHint": true, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false}},
        {"name": "screenshot", "title": "Screenshot",
         "description": "PNG screenshot of the active page (unavailable with GLADIUS_BACKEND_CAPS=dom-only: no rendering pipeline). Optional path: an ABSOLUTE filesystem path the decoded PNG is also written to (missing parent directories are created; a failed write is a typed error, never a silent success). There is no allowlisted screenshot root: only absolute paths are accepted, and a relative path is rejected. When path is absent the reply shape is unchanged. deterministic:true pins the capture: device metrics fixed (current viewport, or width/height; deviceScaleFactor 1, mobile false) and restored afterwards, prefers-reduced-motion:reduce, a temporary style element freezing animation/transition/caret-color/scrollbars, then document.fonts.ready + every img complete + two animation-frame ticks before Page.captureScreenshot({format:png, fromSurface:true, captureBeyondViewport for full page}). The reply then carries viewport, deviceScaleFactor, url, byte length and a cheap content checksum, so two captures of the same page can be proven byte-identical. Everything rides CDP: no OS cursor, no OS keyboard, no window, no foreground tab.",
         "inputSchema": {"type": "object", "properties": {
             "fullPage": {"type": "boolean"}, "path": {"type": "string"},
             "deterministic": {"type": "boolean"},
             "width": {"type": "integer"}, "height": {"type": "integer"}},
             "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {"mimeType": {"type": "string"}, "bytes": {"type": "integer", "minimum": 0},
                "path": {"type": "string"}, "deterministic": {"type": "boolean"},
                "viewport": {"type": "object"}, "deviceScaleFactor": {"type": "number"},
                "url": {"type": "string"}, "checksum": {"type": "string"}},
            "required": ["mimeType", "bytes"], "additionalProperties": false},
         "annotations": {"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}},
        {"name": "click", "title": "Click",
         "description": "Click an element by CSS selector (first match) or AX ref (@eN from snapshot; exact). The click is exactly two CDP events at the element's exact float box centre: mousePressed then mouseReleased - no mouseMoved, no hover, no movement loop, no interpolation. hoverFirst:true sends ONE mouseMoved at that same point first, for pages whose handlers read hover state; the default is false. frame=<CDP frameId> from targets scopes to that iframe (same-process CSS selectors; refs are main-frame only - with frame= use a CSS selector; OOPIF without a mapped target is a typed error, never a main-frame click)",
         "inputSchema": {"type": "object", "properties": {"element": {"type": "string"}, "frame": {"type": "string"}, "hoverFirst": {"type": "boolean"}}, "required": ["element"], "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {"ok": {"type": "boolean"}, "dialog": {"type": "object"}, "note": {"type": "string"}},
            "required": ["ok"], "additionalProperties": false},
         "annotations": {"readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true}},
        {"name": "type", "title": "Type",
         "description": "Type text into an element by CSS selector (first match) or AX ref (@eN from snapshot; exact). frame=<CDP frameId> from targets scopes to that iframe (same-process CSS selectors; refs are main-frame only - with frame= use a CSS selector; OOPIF without a mapped target is a typed error, never a main-frame type)",
         "inputSchema": {"type": "object", "properties": {
             "element": {"type": "string"}, "text": {"type": "string"}, "submit": {"type": "boolean"}, "frame": {"type": "string"}},
             "required": ["element", "text"], "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {"ok": {"type": "boolean"}}, "required": ["ok"], "additionalProperties": false},
         "annotations": {"readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true}},
        {"name": "evaluate", "title": "Evaluate",
         "description": "Evaluate a JS expression in the page, await promises. Optional realm routing: target=<CDP targetId> evaluates in that target's realm (page, dedicated/shared worker, service worker); frame=<CDP frameId> evaluates in that iframe's realm (isolated world, works cross-origin). Without either, the active page's main frame. target and frame are mutually exclusive.",
         "inputSchema": {"type": "object", "properties": {"expression": {"type": "string"}, "target": {"type": "string"}, "frame": {"type": "string"}}, "required": ["expression"], "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {"value": {}, "dialog": {"type": "object"}, "note": {"type": "string"}},
            "required": ["value"], "additionalProperties": false},
         "annotations": {"readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true}},
        {"name": "wait_for", "title": "Wait For",
         "description": "Wait for a selector/text, a JS expression condition, a delay, or network idle (max 15s). networkIdle:true waits for a quiet-network window of idleMs (default 500); a page that never goes idle returns the typed timeout. expression: polls the active page until the JS evaluates truthy (backoff cadence inside ms); expiry is the typed timeout.",
         "inputSchema": {"type": "object", "properties": {
             "selector": {"type": "string"}, "text": {"type": "string"}, "ms": {"type": "integer"},
             "networkIdle": {"type": "boolean"}, "idleMs": {"type": "integer"}, "expression": {"type": "string"}},
             "minProperties": 1, "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {"ok": {"type": "boolean"}}, "required": ["ok"], "additionalProperties": false},
         "annotations": {"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}},
        {"name": "targets", "title": "Targets",
         "description": "List every CDP target with its stable id (pages, workers of all kinds) plus the active page's frame tree (main frame + nested iframes, each with its frameId), or make a page target current. action list (default)|select; select needs target=<targetId> (page targets only; worker realms are reached via evaluate target=).",
         "inputSchema": {"type": "object", "properties": {
             "action": {"type": "string", "enum": ["list", "select"]}, "target": {"type": "string"}},
             "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {"targets": {"type": "array"}, "frames": {"type": "array"},
                "tabs": {"type": "array"}, "ok": {"type": "boolean"}},
            "required": [], "additionalProperties": false,
            "anyOf": [{"required": ["targets"]}, {"required": ["tabs"]}]},
         "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}},
        {"name": "dialog", "title": "Dialog",
         "description": "JS-dialog policy and record. The reader auto-handles every alert/confirm/prompt/beforeunload with the policy (default accept) so a session never deadlocks; the dialog's own text is recorded. action get (default: last dialog + current policy)|policy (set default: needs accept bool, optional promptText for prompt dialogs).",
         "inputSchema": {"type": "object", "properties": {
             "action": {"type": "string", "enum": ["get", "policy"]}, "accept": {"type": "boolean"}, "promptText": {"type": "string"}},
             "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {"dialog": {}, "policy": {"type": "object",
                "properties": {"accept": {"type": "boolean"}, "promptText": {"type": "string"}},
                "required": ["accept"], "additionalProperties": false}},
            "required": ["policy"], "additionalProperties": false},
         "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}},
        {"name": "history", "title": "History",
         "description": "Navigation history on the active page: back|forward step one entry, reload replays the current one. Returns the live url+title after the load settle; a step past the boundary is a typed error, never a silent no-op.",
         "inputSchema": {"type": "object", "properties": {
             "action": {"type": "string"}}, "required": ["action"], "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {"ok": {"type": "boolean"}, "url": {"type": "string"}, "title": {"type": "string"}},
            "required": ["ok", "url", "title"], "additionalProperties": false},
         "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": true}},
        {"name": "network", "title": "Network",
         "description": "Observable network: captured requests/responses (method, url, status, resource type, timing) since the Network domain was first enabled, cap 500. action get (default)|drain|clear; filter is a plain substring matched against the url; limit caps returned (default 100).",
         "inputSchema": {"type": "object", "properties": {
             "action": {"type": "string", "enum": ["get", "drain", "clear"]}, "filter": {"type": "string"}, "limit": {"type": "integer"}},
             "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {"entries": {"type": "array"}, "count": {"type": "integer"},
                "total": {"type": "integer"}, "cap": {"type": "integer"}, "cleared": {"type": "integer"}},
            "required": ["cap"], "additionalProperties": false},
         "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false}},
        {"name": "cdp", "title": "CDP",
         "description": "Raw CDP escape hatch: send any protocol method with params on the browser-level connection or on one named session. Returns the raw CDP result; a CDP error reply is a tool error (isError), never a protocol error.",
         "inputSchema": {"type": "object", "properties": {
             "method": {"type": "string"}, "params": {"type": "object"}, "session": {"type": "string"}},
             "required": ["method"], "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {"result": {}}, "required": ["result"], "additionalProperties": false},
         "annotations": {"readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true}},
        {"name": "console", "title": "Console",
         "description": "Console capture: buffered console messages + uncaught exceptions, each {type,text,url,line,ts}. Ring buffer, cap 200. action get (default)|drain|clear; limit caps returned (default 100).",
         "inputSchema": {"type": "object", "properties": {
             "action": {"type": "string", "enum": ["get", "drain", "clear"]}, "limit": {"type": "integer"}},
             "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {"messages": {"type": "array"}, "count": {"type": "integer"},
                "total": {"type": "integer"}, "cap": {"type": "integer"}, "cleared": {"type": "integer"}},
            "required": ["cap"], "additionalProperties": false},
         "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false}},
        {"name": "upload", "title": "Upload",
         "description": "Set files on an <input type=file> by CSS selector (first match) or AX ref (@eN from snapshot; exact). paths[] must exist locally. frame=<CDP frameId> from targets scopes to that iframe (same-process CSS selectors; refs are main-frame only - with frame= use a CSS selector; OOPIF without a mapped target is a typed error, never a main-frame upload)",
         "inputSchema": {"type": "object", "properties": {
             "element": {"type": "string"}, "paths": {"type": "array", "items": {"type": "string"}, "minItems": 1}, "frame": {"type": "string"}},
             "required": ["element", "paths"], "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {"ok": {"type": "boolean"}, "files": {"type": "integer"}},
            "required": ["ok", "files"], "additionalProperties": false},
         "annotations": {"readOnlyHint": false, "destructiveHint": true, "idempotentHint": true, "openWorldHint": false}},
        {"name": "fetch_text", "title": "Fetch Text",
         "description": "Fast plain-HTTP GET with no browser; HTML stripped to text",
         "inputSchema": {"type": "object", "properties": {"url": {"type": "string"}}, "required": ["url"], "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {"text": {"type": "string"}, "url": {"type": "string"}, "chars": {"type": "integer"}},
            "required": ["text", "url", "chars"], "additionalProperties": false},
         "annotations": {"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": true}},
        {"name": "hover", "title": "Hover",
         "description": "Hover over an element by CSS selector (first match) or AX ref (@eN from snapshot; exact). frame=<CDP frameId> from targets scopes to that iframe (same-process CSS selectors; refs are main-frame only - with frame= use a CSS selector; OOPIF without a mapped target is a typed error, never a main-frame hover)",
         "inputSchema": {"type": "object", "properties": {"element": {"type": "string"}, "frame": {"type": "string"}}, "required": ["element"], "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {"ok": {"type": "boolean"}}, "required": ["ok"], "additionalProperties": false},
         "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": true}},
        {"name": "select", "title": "Select",
         "description": "Choose a <select> option by value (preferred) or visible text; element is a CSS selector or AX ref (@eN). frame=<CDP frameId> from targets scopes to that iframe (same-process CSS selectors; refs are main-frame only - with frame= use a CSS selector; OOPIF without a mapped target is a typed error, never a main-frame select)",
         "inputSchema": {"type": "object", "properties": {
             "element": {"type": "string"}, "value": {"type": "string"}, "text": {"type": "string"}, "frame": {"type": "string"}},
             "required": ["element"], "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {"ok": {"type": "boolean"}}, "required": ["ok"], "additionalProperties": false},
         "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}},
        {"name": "key", "title": "Key",
         "description": "Press a named key (Enter, Tab, Escape, Backspace, ArrowDown, ...) on the focused element. Combos with '+' (Ctrl+A, Shift+Tab, Alt+F4, Meta+C; case-insensitive modifiers, last token is the base key) hold modifiers (Alt=1/Ctrl=2/Meta=4/Shift=8); with Ctrl/Meta/Alt held the base dispatches as rawKeyDown with no text (Ctrl+A never inserts text)",
         "inputSchema": {"type": "object", "properties": {"key": {"type": "string", "minLength": 1}}, "required": ["key"], "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {"ok": {"type": "boolean"}}, "required": ["ok"], "additionalProperties": false},
         "annotations": {"readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true}},
        {"name": "viewport", "title": "Viewport",
         "description": "Set the page viewport in CSS pixels (Emulation.setDeviceMetricsOverride) and return the applied metrics plus a live read-back of window.innerWidth/innerHeight/devicePixelRatio, so an override the engine ignored cannot read as success. reset:true clears the override and restores the engine default (width/height are then ignored). The override is replayed on every new tab.",
         "inputSchema": {"type": "object", "properties": {
             "width": {"type": "integer"}, "height": {"type": "integer"},
             "deviceScaleFactor": {"type": "number"}, "mobile": {"type": "boolean"},
             "reset": {"type": "boolean"}},
             "oneOf": [
                 {"properties": {"reset": {"const": true}}, "required": ["reset"]},
                 {"properties": {"reset": {"const": false}}, "required": ["reset", "width", "height"]},
                 {"required": ["width", "height"], "not": {"required": ["reset"]}}],
             "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {"applied": {}, "readback": {"type": "object"},
                "matched": {"type": "boolean"}, "reset": {"type": "boolean"}, "note": {"type": "string"}},
            "required": ["readback"], "additionalProperties": false},
         "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}},
        {"name": "tabs", "title": "Tabs",
         "description": "List, open, close, or select browser tabs (pages)",
         "inputSchema": {"type": "object", "properties": {
             "action": {"type": "string", "enum": ["list", "new", "select", "close"]}, "url": {"type": "string"}, "index": {"type": "integer"}},
             "required": ["action"], "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {"tabs": {"type": "array"}, "ok": {"type": "boolean"}},
            "required": ["tabs", "ok"], "additionalProperties": false},
         "annotations": {"readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true}},
        {"name": "close", "title": "Close",
         "description": "Shut down the browser (owned process tree) or detach from an attached external engine (GLADIUS_CDP_URL; engine left running)",
         "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
         "outputSchema": {"type": "object",
            "properties": {"ok": {"type": "boolean"}}, "required": ["ok"], "additionalProperties": false},
         "annotations": {"readOnlyHint": false, "destructiveHint": true, "idempotentHint": true, "openWorldHint": false}}
    ]});
    // G5: inserted after tabs (before close) programmatically so the
    // single literal above stays under the macro recursion limit; the count
    // in serverInfo derives from this array (now 25 tools).
    {
        let tools = listed["tools"].as_array_mut().expect("tools array");
        let pos = tools
            .iter()
            .position(|t| t.get("name").and_then(|v| v.as_str()) == Some("tabs"))
            .expect("tabs tool")
            + 1;
        tools.insert(pos, json!({"name": "storage", "title": "Storage",
          "description": "Cookies + Web Storage in one call. action get|set|clear|save|load (default get); scope cookies|localStorage|sessionStorage|all (default all). save/load persist a versioned JSON file (replies carry counts/bytes/origin only, never values); load enforces an origin match unless force:true. allSites:true widens cookie get to every site and cookie clear to the whole profile (clear with allSites on an attached engine needs confirm:true). redact:true masks get values. sessionStorage is tab-scoped (clears with the tab) while localStorage persists.",
          "inputSchema": {"type": "object", "properties": {
              "action": {"type": "string"}, "scope": {"type": "string"},
              "cookies": {"type": "array", "items": {"type": "object"}}, "items": {"type": "object"},
              "path": {"type": "string"}, "allSites": {"type": "boolean"},
              "redact": {"type": "boolean"}, "overwrite": {"type": "boolean"},
              "force": {"type": "boolean"}, "confirm": {"type": "boolean"}},
              "additionalProperties": false},
          "outputSchema": {"type": "object",
             "properties": {"ok": {"type": "boolean"}, "action": {"type": "string"},
                 "scope": {"type": "string"}, "url": {"type": "string"}, "origin": {"type": "string"},
                 "cookies": {"type": "array"}, "localStorage": {"type": "object"}, "sessionStorage": {"type": "object"},
                 "counts": {"type": "object"}, "set": {"type": "object"}, "cleared": {"type": "object"},
                 "failed": {"type": "array"}, "applied": {"type": "object"},
                 "path": {"type": "string"}, "bytes": {"type": "integer"},
                 "fileOrigin": {"type": "string"}, "redacted": {"type": "boolean"}, "note": {"type": "string"}},
             "required": ["ok", "action"], "additionalProperties": false},
          "annotations": {"readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": false}}));
        // Geometry + visual-regression primitives: geometry needs no pixels,
        // appearance does - both are engine measurements, never guesses.
        // Inserted before `close` so the shutdown tool stays last.
        let at = tools
            .iter()
            .position(|t| t.get("name").and_then(|v| v.as_str()) == Some("close"))
            .unwrap_or(tools.len());
        tools.insert(at, json!({"name": "assert_layout", "title": "Assert Layout",
          "description": "Prove geometry without pixels: one call, one or many targets (ref @eN and/or CSS selector, string or array), several checks. Checks: exists (default true; false asserts absence), visible (a real box, not display:none/visibility:hidden/collapse, opacity>0), stableFrames (N animation frames, identical bbox), box {x,y,w,h,tolerance} in VIEWPORT CSS px from DOM.getBoxModel, hitTarget (elementFromPoint at the exact centre is the target or a descendant - the same guard click uses), computed (subset of getComputedStyle, string-exact), noOverflow (scrollWidth/Height <= clientWidth/Height + tolerance), inViewport (whole box inside the visual viewport). Returns per-check {target, check, pass, expected, actual, delta} plus an overall pass. screenshotOnFail:true adds one deterministic capture's path/bytes/checksum. Values are engine measurements (DOM.getBoxModel + Runtime.evaluate), never guesses. Headless + CDP only: no OS cursor, no OS keyboard, no window, no foreground tab.",
          "inputSchema": {"type": "object", "properties": {
              "ref": {"oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}}]},
              "selector": {"oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}}]},
              "checks": {"type": "object", "properties": {
                  "exists": {"type": "boolean"}, "visible": {"type": "boolean"},
                  "stableFrames": {"type": "integer", "minimum": 1},
                  "box": {"type": "object", "properties": {
                      "x": {"type": "number"}, "y": {"type": "number"},
                      "w": {"type": "number"}, "h": {"type": "number"},
                      "tolerance": {"type": "number"}}, "additionalProperties": false},
                  "hitTarget": {"type": "boolean"},
                  "computed": {"type": "object"},
                  "noOverflow": {"oneOf": [{"type": "boolean"}, {"type": "number"},
                      {"type": "object", "properties": {"tolerance": {"type": "number"}}, "additionalProperties": false}]},
                  "inViewport": {"type": "boolean"}},
                  "additionalProperties": false},
              "screenshotOnFail": {"type": "boolean"},
              "screenshotPath": {"type": "string"}},
              "anyOf": [{"required": ["ref"]}, {"required": ["selector"]}],
              "additionalProperties": false},
          "outputSchema": {"type": "object",
             "properties": {"pass": {"type": "boolean"}, "checks": {"type": "array"},
                 "targets": {"type": "array"}, "screenshot": {"type": "object"}},
             "required": ["pass", "checks", "targets"], "additionalProperties": false},
          "annotations": {"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}}));
        tools.insert(at + 1, json!({"name": "layout", "title": "Layout",
          "description": "The where-is-it-actually primitive: every match of a selector or ref list with its viewport rect, page-coordinate box (DOM.getBoxModel), content quad count, scroll size (scrollWidth/Height vs clientWidth/Height), computed display/visibility/opacity, a z-order hint (paint order, z-index, position, stacking-context flag) and every detected overlapping pair with its intersection rect and area (includeOverlaps default true). Main frame; read-only; engine measurements only. Headless + CDP only: no OS input, no window.",
          "inputSchema": {"type": "object", "properties": {
              "ref": {"oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}}]},
              "selector": {"oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}}]},
              "includeOverlaps": {"type": "boolean"}},
              "anyOf": [{"required": ["ref"]}, {"required": ["selector"]}],
              "additionalProperties": false},
          "outputSchema": {"type": "object",
             "properties": {"elements": {"type": "array"}, "overlaps": {"type": "array"},
                 "viewport": {"type": "object"}, "count": {"type": "integer"},
                 "requested": {"type": "integer"}},
             "required": ["elements", "overlaps", "viewport"], "additionalProperties": false},
          "annotations": {"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}}));
        tools.insert(at + 2, json!({"name": "visual", "title": "Visual Baselines",
          "description": "Deterministic visual-regression baselines under GLADIUS_VISUAL_DIR (default <cwd>/.gladius/visual), names sanitised to [A-Za-z0-9._-]. action save captures a deterministic PNG named <name> plus <name>.meta.json (browser, viewport, deviceScaleFactor, url, checksum, createdAt). compare captures the same deterministic shot and diffs it against the baseline in a scratch headless target created with background:true (the engine decodes both PNGs onto a canvas - no new runtime dependency), returning pass, diffPixels, totalPixels, ratio, bbox, maxChannelDelta, baselinePath, diffPath and environment {match, baseline, current}; a browser-major, viewport or deviceScaleFactor mismatch adds a baseline_environment_mismatch warning (not a failure unless requireEnvironmentMatch:true). Anti-aliasing is ignored by default with a deterministic rule: a candidate whose 3x3 luminance neighbourhood has >=32 levels of contrast in BOTH images is suppressed when its 4-connected component is no larger than minDiffCluster (default 2); the threshold is never raised for tolerance. antialias (default true) and includeAA (default false) are inverses. threshold is a per-channel fraction of the 0-255 range (default 0.1), maxDiffPixels/maxDiffRatio set the pass bar (default 0/0), maskRects skip regions; update:true rewrites the baseline and its sidecar; list and delete manage them. compare opens and closes one scratch target on the connected engine, always created with background:true so it never activates (on an external attach that is a real extra tab for a moment); an engine that refuses a background target is refused with a typed scratch_background_unsupported error, never retried in the foreground. Headless + CDP only: no OS input, no window.",
          "inputSchema": {"type": "object", "properties": {
              "action": {"type": "string", "enum": ["save", "compare", "list", "delete"]},
              "name": {"type": "string"}, "fullPage": {"type": "boolean"},
              "threshold": {"type": "number"}, "maxDiffPixels": {"type": "integer"},
              "maxDiffRatio": {"type": "number"},
              "antialias": {"type": "boolean"},
              "includeAA": {"type": "boolean"},
              "minDiffCluster": {"type": "integer", "minimum": 0},
              "requireEnvironmentMatch": {"type": "boolean"},
              "maskRects": {"type": "array", "items": {"type": "object", "properties": {
                  "x": {"type": "number"}, "y": {"type": "number"},
                  "w": {"type": "number"}, "h": {"type": "number"}},
                  "required": ["x", "y", "w", "h"], "additionalProperties": false}},
              "update": {"type": "boolean"}},
              "required": ["action"], "additionalProperties": false},
          "outputSchema": {"type": "object",
             "properties": {"ok": {"type": "boolean"}, "action": {"type": "string"},
                 "name": {"type": "string"}, "pass": {"type": "boolean"}, "updated": {"type": "boolean"},
                 "diffPixels": {"type": "integer"}, "totalPixels": {"type": "integer"},
                 "ratio": {"type": "number"},
                 // `null` in the documented cases: bbox only exists when
                 // something differed, diffPath only once a diff image was
                 // written, and compare{update:true} returns neither.
                 "bbox": {"type": ["object", "null"]},
                 "maxChannelDelta": {"type": "integer"}, "baselinePath": {"type": "string"},
                 "diffPath": {"type": ["string", "null"]}, "bytes": {"type": "integer"},
                 "path": {"type": "string"}, "metaPath": {"type": "string"},
                 "antialias": {"type": "boolean"}, "minDiffCluster": {"type": "integer"},
                 "ignoredPixels": {"type": "integer"}, "ignoredClusters": {"type": "integer"},
                 "scratchBackground": {"type": "boolean"},
                 "environment": {"type": "object"},
                 "warning": {"type": ["string", "null"]},
                 "checksum": {"type": "string"}, "baselineChecksum": {"type": "string"},
                 "viewport": {"type": "object"}, "url": {"type": "string"},
                 "baselines": {"type": "array"}, "dir": {"type": "string"},
                 "count": {"type": "integer"}, "deleted": {"type": "array"},
                 "sizeMismatch": {"type": "boolean"}, "size": {"type": "object"},
                 "threshold": {"type": "number"}, "maxDiffPixels": {"type": "integer"},
                 "maxDiffRatio": {"type": "number"}, "fullPage": {"type": "boolean"}},
             "required": ["ok", "action"], "additionalProperties": false},
          "annotations": {"readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": false}}));
    }
    listed
}

/// Success envelope (G2b): the text block stays byte-identical JSON while
/// `structuredContent` carries the parsed Value for machine readers.
fn ok_text(s: String) -> Value {
    let v: Value = serde_json::from_str(&s).unwrap_or(Value::Null);
    json!({"content": [{"type": "text", "text": s}], "structuredContent": v})
}

/// Typed tool error: the unchanged prose string plus the machine-readable
/// code/reason/context minted at the error SITE (where the failure is
/// known), never derived by re-reading the prose. The prose is partly
/// caller-controlled (selectors, paths, option names), so classifying from
/// it is unsound; each site passes its own code/reason and the real
/// argument value through instead.
///
/// Envelope contract (G1): the prose in `content[0].text` is byte-for-byte
/// what it always was - clients that pattern-match English keep working -
/// and `isError` is unchanged. `structuredContent` rides alongside; a
/// client that ignores the new field sees the old envelope exactly.
pub(crate) struct ToolErr {
    prose: String,
    sc: Value,
}

impl ToolErr {
    /// A failure the site types explicitly: stable code/reason plus the
    /// real offending value (`ref` for an AX ref, `element` for a selector,
    /// `path` for a local file, `want` for a wanted option, `name` for an
    /// unknown tool).
    pub(crate) fn typed(prose: String, sc: Value) -> Self {
        Self { prose, sc }
    }

    /// Anything that genuinely knows nothing (CDP/js-engine failures and
    /// other untyped strings): code only, never an invented reason.
    pub(crate) fn fallback(prose: String) -> Self {
        Self {
            prose,
            sc: json!({"code": "error"}),
        }
    }

    pub(crate) fn envelope(self) -> Value {
        json!({"content": [{"type": "text", "text": self.prose}], "isError": true, "structuredContent": self.sc})
    }
}

/// `?` on engine calls (`browser.command` / `browser.eval` / serde): those
/// failures carry no site-known type, so they keep the generic fallback —
/// except the `stale session: ` prefix minted by `send_session`, which is
/// the typed invalid-target the caller retries explicitly (the session was
/// dropped without proof of non-execution, so no silent resend happened).
impl From<String> for ToolErr {
    fn from(prose: String) -> Self {
        if prose.starts_with("stale session: ") {
            return Self::typed(
                prose,
                json!({"code": "invalid_target", "reason": "stale_session"}),
            );
        }
        Self::fallback(prose)
    }
}

/// Closed machine-readable code set for tool errors, minted at the site:
///
///   stale_ref        reason unknown|navigated|node_gone      (+ tool, ref)
///   not_actionable   reason hidden|disabled|readonly|occluded|not_editable (+ tool, ref|element)
///   invalid_target   reason not_select|not_found|not_file   (+ tool, ref|element)
///   option_not_found reason no_match                         (+ tool, want)
///   capability_gated reason dom_only                         (+ tool)
///   timeout          reason budget_exceeded                   (+ tool)
///   invalid_args     reason missing_arg|bad_value|bad_index|unknown_action|unknown_tool|need_option|unknown_arg (+ tool, arg?)
///   error            fallback, code only (cdp/js-engine failures and anything untyped)
///
/// Fail loud: every tool rejects arguments outside its own schema with
/// `invalid_args`/`unknown_arg` (never silent acceptance) via
/// `check_unknown` at the top of each dispatch arm.
///
/// Dispatch a tool call. Always returns the MCP result object
/// (`{"content": [...]}` with optional `isError`); tool failures are NOT
/// protocol errors.
/// Fail-loud argument gate: any key outside the tool's own schema is a
/// typed `invalid_args`/`unknown_arg` error, never a silent ignore. (A
/// silently ignored `target`/`frame`/`contextId` once routed evaluations to
/// the wrong realm with no error; that shape must never recur.)
fn check_unknown(tool: &str, args: &Value, allowed: &[&str]) -> Result<(), ToolErr> {
    if let Some(obj) = args.as_object() {
        for k in obj.keys() {
            if !allowed.contains(&k.as_str()) {
                return Err(ToolErr::typed(
                    format!("{tool}: unknown argument '{k}' (not in this tool's schema; rejected, not ignored)"),
                    json!({"code": "invalid_args", "reason": "unknown_arg", "tool": tool, "arg": k}),
                ));
            }
        }
    }
    Ok(())
}

pub async fn call(name: &str, args: &Value, browser: &Browser) -> Value {
    let r: Result<Value, ToolErr> = match name {
        "navigate" => {
            if let Err(e) = check_unknown("navigate", args, &["url"]) {
                return e.envelope();
            }
            let url = match args.get("url").and_then(|v| v.as_str()) {
                Some(u) => u.to_string(),
                None => return ToolErr::typed(
                    "navigate: missing required arg 'url'".to_string(),
                    json!({"code": "invalid_args", "reason": "missing_arg", "tool": "navigate"}),
                )
                .envelope(),
            };
            tool_navigate(browser, &url).await
        }
        "snapshot" => {
            if let Err(e) = check_unknown("snapshot", args, &["mode", "frame"]) {
                return e.envelope();
            }
            tool_snapshot(
                browser,
                args.get("mode").and_then(|v| v.as_str()),
                args.get("frame").and_then(|v| v.as_str()),
            )
            .await
        }
        "screenshot" => {
            if let Err(e) = check_unknown(
                "screenshot",
                args,
                &["fullPage", "path", "deterministic", "width", "height"],
            ) {
                return e.envelope();
            }
            let full = args
                .get("fullPage")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let path = args.get("path").and_then(|v| v.as_str());
            let deterministic = args
                .get("deterministic")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let width = args.get("width").and_then(|v| v.as_i64());
            let height = args.get("height").and_then(|v| v.as_i64());
            // Metric overrides only mean something under a pinned
            // capture; accept them otherwise and they would silently do
            // nothing, so refuse instead.
            if !deterministic && (width.is_some() || height.is_some()) {
                return ToolErr::typed(
                        "screenshot: 'width'/'height' require deterministic:true (a plain capture uses the live viewport)".to_string(),
                        json!({"code": "invalid_args", "reason": "bad_value", "tool": "screenshot", "arg": "width"}),
                    )
                    .envelope();
            }
            tool_screenshot(browser, full, path, deterministic, width, height).await
        }
        "assert_layout" => {
            // Boxed on purpose: the geometry body would otherwise be
            // inlined into this already-large dispatch future, and the
            // debug-profile poll frame is the tightest stack budget on
            // the launch path (see main.rs on stack depth).
            Box::pin(tool_assert_layout(browser, args)).await
        }
        "layout" => Box::pin(tool_layout(browser, args)).await,
        "visual" => Box::pin(tool_visual(browser, args)).await,
        "click" => {
            if let Err(e) = check_unknown("click", args, &["element", "frame", "hoverFirst"]) {
                return e.envelope();
            }
            let el =
                match args.get("element").and_then(|v| v.as_str()) {
                    Some(e) => e.to_string(),
                    None => return ToolErr::typed(
                        "click: missing required arg 'element'".to_string(),
                        json!({"code": "invalid_args", "reason": "missing_arg", "tool": "click"}),
                    )
                    .envelope(),
                };
            let frame = args
                .get("frame")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            // Hover is opt-in: a click sends press+release only unless the
            // caller explicitly asks for the hover state first.
            let hover_first = args
                .get("hoverFirst")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            tool_click(browser, &el, frame.as_deref(), hover_first).await
        }
        "type" => {
            if let Err(e) = check_unknown("type", args, &["element", "text", "submit", "frame"]) {
                return e.envelope();
            }
            let (el, text) =
                match (
                    args.get("element").and_then(|v| v.as_str()),
                    args.get("text").and_then(|v| v.as_str()),
                ) {
                    (Some(e), Some(t)) => (e.to_string(), t.to_string()),
                    _ => return ToolErr::typed(
                        "type: missing required args 'element'/'text'".to_string(),
                        json!({"code": "invalid_args", "reason": "missing_arg", "tool": "type"}),
                    )
                    .envelope(),
                };
            let submit = args
                .get("submit")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let frame = args
                .get("frame")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            tool_type(browser, &el, &text, submit, frame.as_deref()).await
        }
        "evaluate" => {
            if let Err(e) = check_unknown("evaluate", args, &["expression", "target", "frame"]) {
                return e.envelope();
            }
            let expr = match args.get("expression").and_then(|v| v.as_str()) {
                Some(e) => e.to_string(),
                None => return ToolErr::typed(
                    "evaluate: missing required arg 'expression'".to_string(),
                    json!({"code": "invalid_args", "reason": "missing_arg", "tool": "evaluate"}),
                )
                .envelope(),
            };
            let target = args
                .get("target")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let frame = args
                .get("frame")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            tool_evaluate(browser, &expr, target.as_deref(), frame.as_deref()).await
        }
        "wait_for" => {
            if let Err(e) = check_unknown(
                "wait_for",
                args,
                &[
                    "selector",
                    "text",
                    "ms",
                    "networkIdle",
                    "idleMs",
                    "expression",
                ],
            ) {
                return e.envelope();
            }
            let sel = args
                .get("selector")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let ms = args.get("ms").and_then(|v| v.as_i64());
            let net_idle = args
                .get("networkIdle")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let idle_ms = args.get("idleMs").and_then(|v| v.as_i64());
            let expr = args
                .get("expression")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            tool_wait_for(
                browser,
                sel.as_deref(),
                text.as_deref(),
                ms,
                net_idle,
                idle_ms,
                expr.as_deref(),
            )
            .await
        }
        "console" => {
            if let Err(e) = check_unknown("console", args, &["action", "limit"]) {
                return e.envelope();
            }
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("get")
                .to_string();
            let limit = args.get("limit").and_then(|v| v.as_i64());
            tool_console(browser, &action, limit).await
        }
        "upload" => {
            if let Err(e) = check_unknown("upload", args, &["element", "paths", "frame"]) {
                return e.envelope();
            }
            let el =
                match args.get("element").and_then(|v| v.as_str()) {
                    Some(e) => e.to_string(),
                    None => return ToolErr::typed(
                        "upload: missing required arg 'element'".to_string(),
                        json!({"code": "invalid_args", "reason": "missing_arg", "tool": "upload"}),
                    )
                    .envelope(),
                };
            let paths: Option<Vec<String>> =
                args.get("paths").and_then(|v| v.as_array()).and_then(|a| {
                    let mut out = Vec::with_capacity(a.len());
                    for x in a {
                        out.push(x.as_str()?.to_string());
                    }
                    Some(out)
                });
            let paths =
                match paths {
                    Some(p) => p,
                    None => return ToolErr::typed(
                        "upload: missing required arg 'paths' (array of strings)".to_string(),
                        json!({"code": "invalid_args", "reason": "missing_arg", "tool": "upload"}),
                    )
                    .envelope(),
                };
            tool_upload(
                browser,
                &el,
                &paths,
                args.get("frame").and_then(|v| v.as_str()),
            )
            .await
        }
        "fetch_text" => {
            if let Err(e) = check_unknown("fetch_text", args, &["url"]) {
                return e.envelope();
            }
            let url = match args.get("url").and_then(|v| v.as_str()) {
                Some(u) => u.to_string(),
                None => return ToolErr::typed(
                    "fetch_text: missing required arg 'url'".to_string(),
                    json!({"code": "invalid_args", "reason": "missing_arg", "tool": "fetch_text"}),
                )
                .envelope(),
            };
            match fetch_text(&url).await {
                Ok(t) => {
                    let chars = t.chars().count() as i64;
                    Ok(json!({"content": [{"type": "text", "text": t.clone()}],
                              "structuredContent": {"text": t, "url": url, "chars": chars}}))
                }
                Err(e) => Err(ToolErr::fallback(e)),
            }
        }
        "hover" => {
            if let Err(e) = check_unknown("hover", args, &["element", "frame"]) {
                return e.envelope();
            }
            let el =
                match args.get("element").and_then(|v| v.as_str()) {
                    Some(e) => e.to_string(),
                    None => return ToolErr::typed(
                        "hover: missing required arg 'element'".to_string(),
                        json!({"code": "invalid_args", "reason": "missing_arg", "tool": "hover"}),
                    )
                    .envelope(),
                };
            tool_hover(browser, &el, args.get("frame").and_then(|v| v.as_str())).await
        }
        "select" => {
            if let Err(e) = check_unknown("select", args, &["element", "value", "text", "frame"]) {
                return e.envelope();
            }
            let el =
                match args.get("element").and_then(|v| v.as_str()) {
                    Some(e) => e.to_string(),
                    None => return ToolErr::typed(
                        "select: missing required arg 'element'".to_string(),
                        json!({"code": "invalid_args", "reason": "missing_arg", "tool": "select"}),
                    )
                    .envelope(),
                };
            let value = args
                .get("value")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            tool_select(
                browser,
                &el,
                value.as_deref(),
                text.as_deref(),
                args.get("frame").and_then(|v| v.as_str()),
            )
            .await
        }
        "key" => {
            if let Err(e) = check_unknown("key", args, &["key"]) {
                return e.envelope();
            }
            let key = match args.get("key").and_then(|v| v.as_str()) {
                Some(k) => k.to_string(),
                None => {
                    return ToolErr::typed(
                        "key: missing required arg 'key'".to_string(),
                        json!({"code": "invalid_args", "reason": "missing_arg", "tool": "key"}),
                    )
                    .envelope()
                }
            };
            tool_key(browser, &key).await
        }
        "viewport" => tool_viewport(browser, args).await,
        "tabs" => {
            if let Err(e) = check_unknown("tabs", args, &["action", "url", "index"]) {
                return e.envelope();
            }
            let action =
                match args.get("action").and_then(|v| v.as_str()) {
                    Some(a) => a.to_string(),
                    None => return ToolErr::typed(
                        "tabs: missing required arg 'action'".to_string(),
                        json!({"code": "invalid_args", "reason": "missing_arg", "tool": "tabs"}),
                    )
                    .envelope(),
                };
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let index = args.get("index").and_then(|v| v.as_i64());
            tool_tabs(browser, &action, url.as_deref(), index).await
        }
        "storage" => {
            if let Err(e) = check_unknown(
                "storage",
                args,
                &[
                    "action",
                    "scope",
                    "cookies",
                    "items",
                    "path",
                    "allSites",
                    "redact",
                    "overwrite",
                    "force",
                    "confirm",
                ],
            ) {
                return e.envelope();
            }
            tool_storage(browser, args).await
        }
        "close" => {
            if let Err(e) = check_unknown("close", args, &[]) {
                return e.envelope();
            }
            match browser.close().await {
                Ok(()) => Ok(ok_text(json!({"ok": true}).to_string())),
                Err(e) => Err(ToolErr::fallback(e)),
            }
        }
        "targets" => {
            if let Err(e) = check_unknown("targets", args, &["action", "target"]) {
                return e.envelope();
            }
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("list")
                .to_string();
            let target = args
                .get("target")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            tool_targets(browser, &action, target.as_deref()).await
        }
        "dialog" => {
            if let Err(e) = check_unknown("dialog", args, &["action", "accept", "promptText"]) {
                return e.envelope();
            }
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("get")
                .to_string();
            tool_dialog(
                browser,
                &action,
                args.get("accept").and_then(|v| v.as_bool()),
                args.get("promptText").and_then(|v| v.as_str()),
            )
            .await
        }
        "history" => {
            if let Err(e) = check_unknown("history", args, &["action"]) {
                return e.envelope();
            }
            let action =
                match args.get("action").and_then(|v| v.as_str()) {
                    Some(a) => a.to_string(),
                    None => return ToolErr::typed(
                        "history: missing required arg 'action'".to_string(),
                        json!({"code": "invalid_args", "reason": "missing_arg", "tool": "history"}),
                    )
                    .envelope(),
                };
            tool_history(browser, &action).await
        }
        "network" => {
            if let Err(e) = check_unknown("network", args, &["action", "filter", "limit"]) {
                return e.envelope();
            }
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("get")
                .to_string();
            let filter = args
                .get("filter")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let limit = args.get("limit").and_then(|v| v.as_i64());
            tool_network(browser, &action, filter.as_deref(), limit).await
        }
        "cdp" => {
            if let Err(e) = check_unknown("cdp", args, &["method", "params", "session"]) {
                return e.envelope();
            }
            let method = match args.get("method").and_then(|v| v.as_str()) {
                Some(m) => m.to_string(),
                None => {
                    return ToolErr::typed(
                        "cdp: missing required arg 'method'".to_string(),
                        json!({"code": "invalid_args", "reason": "missing_arg", "tool": "cdp"}),
                    )
                    .envelope()
                }
            };
            let params = args.get("params").cloned().unwrap_or(json!({}));
            if !params.is_object() {
                return ToolErr::typed(
                    "cdp: 'params' must be an object".to_string(),
                    json!({"code": "invalid_args", "reason": "bad_value", "tool": "cdp"}),
                )
                .envelope();
            }
            let session = args
                .get("session")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            tool_cdp(browser, &method, params, session).await
        }
        other => {
            return ToolErr::typed(
                format!("unknown tool: {other}"),
                json!({"code": "invalid_args", "reason": "unknown_tool", "name": other}),
            )
            .envelope()
        }
    };
    match r {
        Ok(v) => v,
        Err(e) => e.envelope(),
    }
}

async fn tool_navigate(browser: &Browser, url: &str) -> Result<Value, ToolErr> {
    browser.command("Page.enable", json!({}), true).await?;
    let nav = browser
        .command("Page.navigate", json!({"url": url}), true)
        .await?;
    if let Some(err) = nav.get("errorText").and_then(|v| v.as_str()) {
        if !err.is_empty() {
            return Err(ToolErr::fallback(format!("navigate: {err}")));
        }
    }
    // Wait for load (event-driven: wakes on Page.loadEventFired, 10s cap).
    browser
        .wait_for_load(std::time::Duration::from_secs(10))
        .await;
    let title = browser
        .eval("document.title")
        .await?
        .as_str()
        .unwrap_or("")
        .to_string();
    let cur = browser
        .eval("window.location.href")
        .await?
        .as_str()
        .unwrap_or(url)
        .to_string();
    // NOTE: no HTTP status is reported. The tool speaks CDP, not HTTP: it
    // never sees the main-frame response status (there is no Network-domain
    // capture here), and a hardcoded 200 lies on every 404. A wrong field is
    // worse than no field, so the envelope is just url + title.
    //
    // Transient blank-load race (a commit of an EMPTY DOM - empty title,
    // empty body - at the right URL, historically observed as a double-fetch
    // race where the losing request won the commit):
    // when the committed page is non-blank yet title AND body text are both
    // empty, say so in a `warning` field so the caller can see it (and
    // re-navigate - the flake is retryable). One extra eval, only on
    // suspicious pages; no retry magic here.
    if !title.is_empty() || cur.is_empty() || cur == "about:blank" {
        return Ok(ok_text(json!({"url": cur, "title": title}).to_string()));
    }
    // NOTE: `.length` arrives as an int on some engines but as a float on
    // others (`42` vs `42.0` number shape), so compare as f64 - `as_i64`
    // would miss the empty case on float-shaping engines entirely.
    let body_len = browser
        .eval("(((document.body && document.body.innerText) || '') + '').length")
        .await
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(-1.0);
    if body_len == 0.0 {
        Ok(ok_text(
            json!({"url": cur, "title": title,
                   "warning": "possible blank-load race: committed URL with empty document (empty title and empty body text); re-navigate to confirm"})
            .to_string(),
        ))
    } else {
        Ok(ok_text(json!({"url": cur, "title": title}).to_string()))
    }
}

/// Parse an AX ref arg (`@e3`) to its table id (`e3`). Selectors pass through
/// as None so existing selector/text targeting keeps working.
fn parse_ref(s: &str) -> Option<String> {
    let t = s.trim();
    let rest = t.strip_prefix('@')?;
    let id = rest.trim();
    let num = id.strip_prefix('e')?;
    if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
        Some(id.to_string())
    } else {
        None
    }
}

/// Guard a ref-targeted action: unknown refs and post-navigation refs fail
/// with a TYPED stale-ref error naming the reason - never a silent
/// wrong-element click. Returns the ref id (`eN`) when it may proceed.
async fn guard_ref(
    browser: &Browser,
    tool: &str,
    _element: &str,
    id: &str,
) -> Result<String, ToolErr> {
    if browser.ref_label(id).await.is_none() {
        return Err(ToolErr::typed(
            format!("stale ref: {tool}: unknown ref @{id} (no such ref; take a fresh snapshot)"),
            json!({"code": "stale_ref", "reason": "unknown", "tool": tool, "ref": format!("@{id}")}),
        ));
    }
    let snap = browser.snap_url().await;
    if !snap.is_empty() {
        let cur = browser
            .eval("window.location.href")
            .await
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        if !cur.is_empty() && cur != snap {
            return Err(ToolErr::typed(
                format!("stale ref: {tool}: ref @{id} is stale (page navigated from {snap} to {cur}; take a fresh snapshot)"),
                json!({"code": "stale_ref", "reason": "navigated", "tool": tool, "ref": format!("@{id}")}),
            ));
        }
    }
    Ok(format!("@{id}"))
}

fn stale_gone(tool: &str, element: &str, id: &str) -> ToolErr {
    let _ = element;
    ToolErr::typed(
        format!("stale ref: {tool}: ref @{id} is stale (node gone; take a fresh snapshot)"),
        json!({"code": "stale_ref", "reason": "node_gone", "tool": tool, "ref": format!("@{id}")}),
    )
}

/// G4b frame policy: AX refs (`@eN`) are stamped against the MAIN document
/// resolver, so a ref scoped to a frame realm cannot resolve safely. Reject it
/// with a typed `invalid_args` (never a silent main-frame click); frame
/// actions take CSS selectors. Snapshot refs stay main-frame: a frame-scoped
/// snapshot stamps its own table, but acting through the main-document
/// resolver is unsupported, so the typed rejection stands either way.
fn ref_in_frame(tool: &str, element: &str, frame: &str) -> ToolErr {
    ToolErr::typed(
        format!("{tool}: refs (@eN) are main-frame only; with frame= use a CSS selector (got ref '{element}' with frame '{frame}'; take a frame snapshot for names, then act with the selector)"),
        json!({"code": "invalid_args", "reason": "bad_value", "tool": tool, "arg": "frame", "frame": frame, "element": element}),
    )
}

/// G4b: normalize the optional `frame` arg. Empty/blank is a typed
/// `invalid_args` (never the main frame by accident).
fn frame_id_ok(tool: &str, frame: Option<&str>) -> Result<Option<String>, ToolErr> {
    match frame {
        None => Ok(None),
        Some(f) if f.trim().is_empty() => Err(ToolErr::typed(
            format!("{tool}: 'frame' must be a non-empty CDP frameId (from targets)"),
            json!({"code": "invalid_args", "reason": "bad_value", "tool": tool, "arg": "frame"}),
        )),
        Some(f) => Ok(Some(f.to_string())),
    }
}

/// G4b: map a frame-realm engine failure to the site-known type. Unknown
/// frame ids (isolated-world creation refuses them) are typed
/// `invalid_target`/`not_found` with the frame; OOPIF/detached shapes keep
/// their own typed prose from the site; everything else is the generic
/// fallback.
fn frame_lookup_err(tool: &str, fid: &str, e: String) -> ToolErr {
    let l = e.to_lowercase();
    if l.contains("no frame with id") || l.contains("no such frame") || l.contains("cannot find") {
        ToolErr::typed(
            format!("{tool}: no frame with id '{fid}'"),
            json!({"code": "invalid_target", "reason": "not_found", "tool": tool, "frame": fid}),
        )
    } else {
        ToolErr::fallback(e)
    }
}

/// G4b verifier: leaf-realm evaluation with OOPIF routing. When the frame
/// has a mapped OOPIF target session, the leaf JS (preflight, focus,
/// selection, rect reads) runs THERE (`eval_in_session`); otherwise it runs
/// in the frame's isolated world on the page session (`eval_in_frame`,
/// same-process). The owner chain, parent-side hit-test and dispatch always
/// stay on the page session, whose visual surface owns the coordinates.
/// Unknown frames are typed `invalid_target`/`not_found` - never a
/// main-frame evaluation.
async fn frame_realm_eval(
    browser: &Browser,
    tool: &str,
    fid: &str,
    expr: &str,
) -> Result<Value, ToolErr> {
    if let Some((_, sid)) = browser.oop_session_for_frame(fid).await {
        return browser
            .eval_in_session(&sid, expr)
            .await
            .map_err(ToolErr::from);
    }
    browser
        .eval_in_frame(fid, expr)
        .await
        .map_err(|e| frame_lookup_err(tool, fid, e))
}

/// G4b verifier: parent-side hit-test against the TRUE frame owners (not
/// just "any iframe"). Stamps every owner node in the chain with a temporary
/// attribute, then checks `document.elementFromPoint(x, y)` in the MAIN
/// realm: the hit itself or one of its ancestors must carry the stamp. A
/// covering parent overlay or a DIFFERENT iframe has no stamp and is typed
/// `not_actionable`/`occluded` - no dispatch happens. The stamp is removed
/// before returning on every path (snapshot refs already stamp nodes the
/// same way; a `data-*` attribute is page-inert).
/// G4b verifier: per-level parent-side hit-test against the TRUE owners.
/// A main-document-only check passes a stamped outer iframe even when an
/// overlay INSIDE it covers the requested inner iframe, so every ancestor
/// level is checked in its own realm: the point is mapped into that
/// parent's viewport (`frame_chain_points`), `elementFromPoint` runs there,
/// and the hit (or one of its ancestors) must carry the owner stamp for
/// that level. Each parent document holds exactly one stamped node (the
/// on-path child), so a covering overlay or a DIFFERENT iframe at ANY level
/// is typed `not_actionable`/`occluded` and nothing dispatches. Returns the
/// main-viewport point for dispatch once every level passes.
async fn frame_owner_hit_ok(
    browser: &Browser,
    tool: &str,
    element: &str,
    fid: &str,
    lx: f64,
    ly: f64,
) -> Result<(f64, f64), ToolErr> {
    fn occluded(tool: &str, element: &str, fid: &str) -> ToolErr {
        ToolErr::typed(
            format!(
                "{tool}: element is occluded by another element (no {tool} dispatched): {element}"
            ),
            json!({"code": "not_actionable", "reason": "occluded", "tool": tool, "element": element, "frame": fid}),
        )
    }
    const ATTR: &str = "data-gladius-frame-owner";
    let attr_lit =
        serde_json::to_string(ATTR).unwrap_or_else(|_| "\"data-gladius-frame-owner\"".to_string());
    let levels = browser
        .frame_chain_points(fid, lx, ly)
        .await
        .map_err(|e| frame_lookup_err(tool, fid, e))?;
    if levels.is_empty() {
        // `frame` names the main frame itself: no owners exist, the point is
        // already main-viewport, and the frame-local preflight hit-tested.
        return Ok((lx, ly));
    }
    let nodes = browser
        .frame_owner_nodes(fid)
        .await
        .map_err(|e| frame_lookup_err(tool, fid, e))?;
    if nodes.len() != levels.len() {
        return Err(ToolErr::fallback(format!(
            "{tool}: frame owner chain mismatch for frame '{fid}'"
        )));
    }
    // Stamp every owner; a stamping error still runs the cleanup below.
    let mut verdict: Result<(), ToolErr> = Ok(());
    for nid in &nodes {
        if let Err(e) = browser
            .command(
                "DOM.setAttributeValue",
                json!({"nodeId": nid, "name": ATTR, "value": "1"}),
                true,
            )
            .await
        {
            verdict = Err(ToolErr::from(e));
            break;
        }
    }
    // Per-level verdicts, immediate parent first, main last. Skipped when
    // stamping failed (a partial stamp would false-negative the identity
    // check into a misleading occluded).
    if verdict.is_ok() {
        for (realm, x, y) in &levels {
            let expr = format!("((x, y, attr) => {{ const h = document.elementFromPoint(x, y); if (!h) return 'NONE'; let n = h; while (n) {{ try {{ if (n.hasAttribute && n.hasAttribute(attr)) return 'OWNER'; }} catch (e) {{}} n = n.parentElement; }} return (h.tagName || 'NONE'); }})({x}, {y}, {attr_lit})");
            let v: Result<Value, ToolErr> = match realm {
                None => browser.eval(&expr).await.map_err(ToolErr::from),
                Some(p) => frame_realm_eval(browser, tool, p, &expr).await,
            };
            match v {
                Ok(v) if v.as_str() == Some("OWNER") => {}
                Ok(_) => {
                    verdict = Err(occluded(tool, element, fid));
                    break;
                }
                Err(e) => {
                    verdict = Err(e);
                    break;
                }
            }
        }
    }
    // Cleanup guard: the temp stamp is removed from EVERY ancestor document
    // (each level's own realm, main last) whatever happened above - a
    // stamping error, a level failure, or an eval failure must never leave
    // stamps behind. Best-effort: cleanup errors never replace the verdict.
    for realm in levels.iter().map(|(r, _, _)| r) {
        let cleanup = format!("((attr) => {{ try {{ document.querySelectorAll('[' + attr + ']').forEach((n) => n.removeAttribute(attr)); }} catch (e) {{}} return 'ok'; }})({attr_lit})");
        match realm {
            None => {
                let _ = browser.eval(&cleanup).await;
            }
            Some(p) => {
                let _ = frame_realm_eval(browser, tool, p, &cleanup).await;
            }
        }
    }
    let (mx, my) = levels.last().map(|(_, x, y)| (*x, *y)).unwrap_or((lx, ly));
    verdict.map(|()| (mx, my))
}

/// Structured `snapshot` payload (G2): the node table the text block was
/// built from (`items`), trimmed to exactly the nodes whose lines survived
/// the 8000-char cap, so `nodes` matches the text block's refs one-for-one
/// (including on a truncated page). Zero new page evaluation - `items` is
/// what the snapshot already computed.
fn snapshot_nodes(
    url: &str,
    title: &str,
    items: &[Value],
    shown_lines: usize,
    truncated: bool,
) -> Value {
    // lines[0] is the header; lines[1..] are one node each.
    let n = shown_lines.saturating_sub(1).min(items.len());
    let nodes: Vec<Value> = items
        .iter()
        .take(n)
        .map(|it| {
            json!({
                "ref": it.get("ref").and_then(|x| x.as_str()).unwrap_or(""),
                "role": it.get("role").and_then(|x| x.as_str()).unwrap_or(""),
                "name": it.get("name").and_then(|x| x.as_str()).unwrap_or(""),
                "tag": it.get("tag").and_then(|x| x.as_str()).unwrap_or(""),
                "disabled": it.get("disabled").and_then(|x| x.as_bool()).unwrap_or(false),
                "value": it.get("value").and_then(|x| x.as_str()).unwrap_or(""),
            })
        })
        .collect();
    json!({"url": url, "title": title, "truncated": truncated, "nodes": nodes})
}

/// AX snapshot result: the human text block stays FIRST and byte-identical
/// (cap, footer, and truncation behavior unchanged); the machine node table
/// rides as the second block plus `structuredContent`. `snapshot
/// {"mode":"text"}` never gains one.
fn ok_snapshot(text: String, structured: Value) -> Value {
    json!({"content": [{"type": "text", "text": text},
                       {"type": "text", "text": structured.to_string()}],
           "structuredContent": structured})
}

async fn tool_snapshot(
    browser: &Browser,
    mode: Option<&str>,
    frame: Option<&str>,
) -> Result<Value, ToolErr> {
    match mode {
        Some("text") => return tool_snapshot_text_frame(browser, frame).await,
        Some("legacy") => return tool_snapshot_legacy(browser, frame).await,
        Some("ax") | None => return tool_snapshot_ax(browser, frame).await,
        Some(other) => Err(ToolErr::typed(
            format!("snapshot: unknown mode '{other}' (want 'ax'|'legacy'|'text')"),
            json!({"code": "invalid_args", "reason": "bad_value", "tool": "snapshot", "arg": "mode", "want": other}),
        )),
    }
}

async fn tool_snapshot_legacy(browser: &Browser, frame: Option<&str>) -> Result<Value, ToolErr> {
    // Legacy DOM-heuristic mode (explicit `mode:"legacy"` only): enumerate actionable nodes in
    // document order, stamp each with an exact data-gladius-ref id, and
    // report role/name/value. Refs are unique across snapshots: the lock
    // serializes concurrent snapshots and the base offset keeps @eN from
    // ever being reassigned. With frame= the same heuristic runs inside
    // that iframe's realm (isolated world, works cross-origin).
    let _snap_guard = browser.lock_snapshot().await;
    let start = browser.ref_start();
    let raw_js = r#"(() => {
  const base = __REF_BASE__;
  // Secret hygiene: password inputs and inputs carrying a credential-ish
  // autocomplete token never expose their value (see isSensitive). Their
  // displayed/structured value is the literal mask "••••", and the raw
  // value is NOT used as the AX name fallback (aria-label, placeholder,
  // id, name, type instead, in that order).
  const SENS_AUTOCOMPLETE = new Set(["current-password", "new-password", "cc-number", "cc-exp", "cc-csc", "one-time-code"]);
  const isSensitive = (el) => {
    if ((el.tagName || "").toUpperCase() !== "INPUT") return false;
    const ty = String((el.getAttribute && el.getAttribute("type")) || el.type || "text").toLowerCase();
    if (ty === "password") return true;
    const ac = String((el.getAttribute && el.getAttribute("autocomplete")) || el.autocomplete || "");
    return ac.toLowerCase().split(/\s+/).filter(Boolean).some((tok) => SENS_AUTOCOMPLETE.has(tok));
  };
  const cands = Array.from(document.querySelectorAll("a,button,input,select,textarea,[role],[tabindex],[onclick],[onmouseover]"));
  const seen = new Set(); const items = []; let n = 0;
  for (const el of cands) {
    if (!el || seen.has(el)) continue; seen.add(el);
    if (el.tagName === "OPTION") continue;
    let role = (el.getAttribute && el.getAttribute("role")) || "";
    if (!role) {
      const t = (el.tagName || "").toUpperCase();
      if (t === "BUTTON") role = "button";
      else if (t === "A") role = "link";
      else if (t === "SELECT") role = "combobox";
      else if (t === "TEXTAREA") role = "textbox";
      else if (t === "INPUT") {
        const ty = (el.type || "text").toLowerCase();
        if (ty === "checkbox" || ty === "radio") role = "checkbox";
        else if (ty === "submit" || ty === "button" || ty === "image") role = "button";
        else role = "textbox";
      } else role = t.toLowerCase() || "generic";
    }
    let name = (el.getAttribute && (el.getAttribute("aria-label") || "")) || "";
    if (!name) {
      const t = (el.tagName || "").toUpperCase();
      if (t === "INPUT" || t === "TEXTAREA") { const rawForName = (t === "INPUT" && isSensitive(el)) ? "" : (el.value || ""); name = el.placeholder || rawForName || el.id || el.name || el.type || ""; }
      else if (t === "SELECT") { try { const o = el.options[el.selectedIndex]; name = (o && o.text) || el.id || el.name || ""; } catch (e) { name = el.id || ""; } }
      else name = el.innerText || el.textContent || el.id || "";
    }
    name = (name + "").replace(/\s+/g, " ").trim().slice(0, 80);
    let value = "";
    try {
      const t = (el.tagName || "").toUpperCase();
      if (t === "INPUT" || t === "TEXTAREA" || t === "SELECT") { if (t === "INPUT" && isSensitive(el)) value = "••••"; else value = String(el.value).slice(0, 60); }
    } catch (e) {}
    n++;
    const ref = "e" + (base + n);
    try { el.dataset.gladiusRef = ref; } catch (e) {}
    items.push({ref: ref, role: role, name: name, value: value,
                tag: (el.tagName || "").toLowerCase(), disabled: !!el.disabled});
  }
  return {url: location.href, title: document.title, items: items};
})()"#;
    let expr = raw_js.replace("__REF_BASE__", &start.to_string());
    let v = match frame {
        Some(fid) => browser.eval_in_frame(fid, &expr).await?,
        None => browser.eval(&expr).await?,
    };
    let empty = Vec::new();
    let items = v.get("items").and_then(|i| i.as_array()).unwrap_or(&empty);
    let url = v
        .get("url")
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();
    let title = v
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    // Store the ref table for exact ref resolution + stale detection.
    let mut table = std::collections::HashMap::new();
    for it in items.iter() {
        if let (Some(r), role, name) = (
            it.get("ref").and_then(|x| x.as_str()),
            it.get("role").and_then(|x| x.as_str()).unwrap_or(""),
            it.get("name").and_then(|x| x.as_str()).unwrap_or(""),
        ) {
            table.insert(r.to_string(), format!("{role} {name}"));
        }
    }
    browser.store_refs(url.clone(), table).await;
    let total = items.len();
    browser.ref_advance(total);
    // Page text keeps old callers (marker asserts) working on AX output.
    // With frame= the frame's own text is reported.
    let frame_owned;
    let tv = match frame {
        Some(fid) => {
            frame_owned = fid.to_string();
            browser
                .eval_in_frame(
                    &frame_owned,
                    "((((document.body && document.body.innerText) || (document.documentElement && document.documentElement.innerText) || '')) + '').slice(0, 12000)",
                )
                .await
        }
        None => {
            browser
                .eval("((((document.body && document.body.innerText) || (document.documentElement && document.documentElement.innerText) || '')) + '').slice(0, 12000)")
                .await
        }
    };
    let raw = tv
        .ok()
        .and_then(|x| x.as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    let text = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut lines: Vec<String> = Vec::with_capacity(total + 3);
    lines.push(format!("AX snapshot: {title} <{url}>"));
    for it in items.iter() {
        let r = it.get("ref").and_then(|x| x.as_str()).unwrap_or("?");
        let role = it.get("role").and_then(|x| x.as_str()).unwrap_or("");
        let name = it.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let tag = it.get("tag").and_then(|x| x.as_str()).unwrap_or("");
        let val = it.get("value").and_then(|x| x.as_str()).unwrap_or("");
        let dis = it
            .get("disabled")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        let mut ln = format!("- {role} \"{name}\" [@{r}] ({tag})");
        if !val.is_empty() {
            ln.push_str(&format!(" value=\"{val}\""));
        }
        if dis {
            ln.push_str(" [disabled]");
        }
        lines.push(ln);
    }
    // Bounded output with EXPLICIT truncation (announced + by how much).
    // The footer is never cut: lines and the expendable text tail shrink
    // first, so a truncation notice is always present when content is lost.
    let cap = 8000usize;
    let total_lines = lines.len();
    let mut shown = total_lines;
    let mut out = lines.join("\n");
    let mut tail = if text.is_empty() {
        String::new()
    } else {
        format!("\ntext: {text}")
    };
    let mut tail_cut = false;
    loop {
        let mut footer = if total == 0 {
            format!("\n[snapshot: {total} nodes, refs none]")
        } else {
            let first = start + 1;
            let last = start + total;
            format!("\n[snapshot: {total} nodes, refs @e{first}..@e{last}]")
        };
        if shown < total_lines {
            let rest_chars: usize = lines
                .iter()
                .skip(shown)
                .map(|l| l.chars().count() + 1)
                .sum::<usize>()
                + text.chars().count();
            footer.push_str(&format!(
                "\n[truncated: showing {shown} of {total_lines} lines, {rest_chars} chars omitted]"
            ));
        }
        let full = format!("{out}{tail}{footer}");
        if full.chars().count() <= cap {
            let truncated = shown < total_lines || tail_cut;
            return Ok(ok_snapshot(
                full,
                snapshot_nodes(&url, &title, items, shown, truncated),
            ));
        }
        // Shrink order: text tail first, then whole lines off the end.
        if !tail.is_empty() {
            tail_cut = true;
            let excess = full.chars().count() - cap;
            let keep = tail.chars().count().saturating_sub(excess);
            tail = if keep == 0 {
                String::new()
            } else {
                tail.chars().take(keep).collect()
            };
        } else if shown > 1 {
            shown -= 1;
            out = lines[..shown].join("\n");
        } else {
            // Degenerate: header alone plus footer still exceed the cap;
            // hard-cut (footer stays truthful about the node count).
            let cut: String = full.chars().take(cap).collect();
            return Ok(ok_snapshot(
                cut,
                snapshot_nodes(&url, &title, items, shown, true),
            ));
        }
    }
}

/// Real Accessibility-tree snapshot (the DEFAULT when `mode` is absent or
/// null; explicit `mode:"ax"` is identical; optional `frame=`).
/// Nodes come from `Accessibility.getFullAXTree`: non-ignored AX nodes with
/// a `backendDOMNodeId`. Backend ids map to frontend node ids
/// (`DOM.pushNodesByBackendIdsToFrontend`); each is stamped with
/// `data-gladius-ref` via `DOM.setAttributeValue` so the existing ref
/// guards keep resolving. Role/name come from the AX node; tag and the
/// live disabled/value refine from the element in one eval
/// (`DOM.describeNode` fallback for shadow nodes `querySelector` cannot
/// reach). Unlabeled form controls (empty AX name on input/textarea/select)
/// fall back the same way the legacy heuristic names them
/// (placeholder, live value unless sensitive, selected option, id, name) so
/// an unlabeled control keeps a stable human name on both paths. Sensitive
/// inputs keep the G0 mask. AX-ignored subtrees (display:none, aria-hidden,
/// and other ignored nodes) are NOT stamped and never appear in the lines:
/// an old ref to such a node resolves through the guard to the typed
/// `stale_ref` (unknown/node-gone), never a silent main-frame click.
/// Text format, block2 shape, cap, and footer match the legacy path.
/// Same-process iframes are covered; OOPIF subtrees needing a separate
/// target session are a documented follow-up (the engine error surfaces
/// instead of faked content). Acting on a shadow ref through the
/// main-document `querySelector` resolver is NOT yet supported (G4): the
/// resolver cannot reach into open shadow roots, so a shadow ref action
/// today returns the typed `stale_ref`/`node-gone` - documented, never
/// faked.
async fn tool_snapshot_ax(browser: &Browser, frame: Option<&str>) -> Result<Value, ToolErr> {
    let _snap_guard = browser.lock_snapshot().await;
    let start = browser.ref_start();
    let tree = browser.ax_tree(frame).await.map_err(|e| {
        let l = e.to_lowercase();
        if frame.is_some() && (l.contains("frame") || l.contains("no frame") || l.contains("no such")) {
            ToolErr::typed(
                format!("snapshot: no frame with id '{}'", frame.unwrap_or("")),
                json!({"code": "invalid_target", "reason": "not_found", "tool": "snapshot", "frame": frame.unwrap_or("")}),
            )
        } else {
            ToolErr::from(e)
        }
    })?;
    let empty_nodes = Vec::new();
    let ax_nodes = tree
        .get("nodes")
        .and_then(|n| n.as_array())
        .unwrap_or(&empty_nodes);
    struct Cand {
        backend: i64,
        role: String,
        name: String,
        ax_value: String,
        ax_disabled: bool,
    }
    let mut cands: Vec<Cand> = Vec::new();
    for n in ax_nodes.iter() {
        if n.get("ignored").and_then(|v| v.as_bool()) == Some(true) {
            continue;
        }
        let backend = match n.get("backendDOMNodeId").and_then(|v| v.as_i64()) {
            Some(b) => b,
            None => continue,
        };
        let role_raw = n
            .get("role")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if role_raw.is_empty() {
            continue;
        }
        let role = role_raw.to_lowercase();
        // Text leaves point at text nodes (no element to stamp, no ref to
        // act on); their text already surfaces in the page-text tail and in
        // their element ancestors' accessible names.
        if role == "statictext" || role == "inlinetextbox" {
            continue;
        }
        let raw_name = n
            .get("name")
            .and_then(|x| x.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let name: String = raw_name
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(80)
            .collect();
        let ax_value_raw = match n.get("value").and_then(|x| x.get("value")) {
            Some(Value::String(s)) => s.clone(),
            Some(other) if !other.is_null() => {
                let s = other.to_string();
                s.trim_matches('"').to_string()
            }
            _ => String::new(),
        };
        let ax_value: String = ax_value_raw.chars().take(60).collect();
        let mut disabled = n.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false);
        if let Some(props) = n.get("properties").and_then(|v| v.as_array()) {
            for p in props {
                if p.get("name").and_then(|v| v.as_str()) != Some("disabled") {
                    continue;
                }
                if let Some(v) = p.get("value").and_then(|v| v.get("value")) {
                    if v.as_bool() == Some(true) || v.as_i64().is_some_and(|i| i != 0) {
                        disabled = true;
                    }
                }
            }
        }
        cands.push(Cand {
            backend,
            role,
            name,
            ax_value,
            ax_disabled: disabled,
        });
    }
    // Resolve + stamp. `DOM.enable` first (the upload path does the same),
    // then a document handle: pushNodesByBackendIdsToFrontend needs the
    // document requested first (pierce so shadow/iframe nodes resolve).
    browser.command("DOM.enable", json!({}), true).await?;
    let _ = browser
        .command("DOM.getDocument", json!({"depth": 0, "pierce": true}), true)
        .await?;
    let backend_ids: Vec<Value> = cands.iter().map(|c| json!(c.backend)).collect();
    let pushed = browser
        .command(
            "DOM.pushNodesByBackendIdsToFrontend",
            json!({"backendNodeIds": backend_ids}),
            true,
        )
        .await?;
    let node_ids = pushed
        .get("nodeIds")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    // Placeholder items in AX order; enrichment fills tag/disabled/value.
    let mut items: Vec<Value> = Vec::new();
    let mut live_nids: Vec<i64> = Vec::new();
    let mut n_ok: usize = 0;
    for (cand, nid_v) in cands.iter().zip(node_ids.iter()) {
        let nid = nid_v.as_i64().unwrap_or(0);
        if nid == 0 {
            continue;
        }
        let r = format!("e{}", start + n_ok + 1);
        let set = browser
            .command(
                "DOM.setAttributeValue",
                json!({"nodeId": nid, "name": "data-gladius-ref", "value": r}),
                true,
            )
            .await;
        if set.is_err() {
            continue;
        }
        n_ok += 1;
        items.push(json!({
            "ref": r, "role": cand.role, "name": cand.name,
            "value": cand.ax_value, "tag": "", "disabled": cand.ax_disabled,
        }));
        live_nids.push(nid);
    }
    // Live enrichment: one eval maps ref -> {tag, disabled, value (60-char
    // display cut), fullVal (uncapped raw value for name fallback), sens,
    // ph, eid, enm, selText, ty}.
    // Shadow nodes (querySelector cannot reach into open shadow roots from
    // the document) come back null and fall to DOM.describeNode below.
    let refs: Vec<String> = items
        .iter()
        .filter_map(|it| {
            it.get("ref")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        })
        .collect();
    let refs_lit = serde_json::to_string(&refs).unwrap_or_else(|_| "[]".to_string());
    let enrich_js = format!(
        r#"((refs) => {{
  const SENS = new Set(["current-password", "new-password", "cc-number", "cc-exp", "cc-csc", "one-time-code"]);
  const out = {{}};
  for (const ref of refs) {{
    const el = document.querySelector('[data-gladius-ref="' + ref + '"]');
    if (!el) {{ out[ref] = null; continue; }}
    const tag = ((el.tagName || "").toLowerCase());
    let rawVal = "";
    try {{ if ("value" in el) rawVal = String(el.value); }} catch (e) {{}}
    const ty = String((el.getAttribute && el.getAttribute("type")) || el.type || "").toLowerCase();
    const ac = String((el.getAttribute && el.getAttribute("autocomplete")) || el.autocomplete || "");
    const sens = (tag === "input") && (ty === "password" || ac.toLowerCase().split(/\s+/).filter(Boolean).some((t) => SENS.has(t)));
    let ph = ""; try {{ ph = el.placeholder || ""; }} catch (e) {{}}
    let eid = ""; try {{ eid = el.id || ""; }} catch (e) {{}}
    let enm = ""; try {{ enm = el.name || ""; }} catch (e) {{}}
    let selText = "";
    try {{ if (tag === "select" && el.options && el.selectedIndex >= 0) {{ const o = el.options[el.selectedIndex]; selText = (o && (o.text || o.textContent)) || ""; }} }} catch (e) {{}}
    // Fallback candidates ship FULL and RAW: nothing is truncated or
    // normalized here. Truncating (or normalizing) before the legacy-order
    // pick would corrupt it - e.g. 79 spaces + "ID_END" sliced to 80 chars
    // normalizes to "I" instead of "ID_END". The 60-char cut applies only
    // to the display `value`, never to names.
    out[ref] = {{tag: tag, disabled: !!el.disabled, value: rawVal.slice(0, 60), fullVal: rawVal, sens: sens, ph: String(ph || ""), eid: String(eid || ""), enm: String(enm || ""), selText: String(selText || ""), ty: String(ty || "").slice(0, 20)}};
  }}
  return out;
}})({refs_lit})"#
    );
    let enrich: Value = match frame {
        Some(fid) => browser
            .eval_in_frame(fid, &enrich_js)
            .await
            .unwrap_or(Value::Null),
        None => browser.eval(&enrich_js).await.unwrap_or(Value::Null),
    };
    let enrich_map = enrich.as_object().cloned().unwrap_or_default();
    for (idx, it) in items.iter_mut().enumerate() {
        let r = it
            .get("ref")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let ax_value = it
            .get("value")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let ax_disabled = it
            .get("disabled")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        match enrich_map.get(&r) {
            Some(Value::Null) | None => {
                // Shadow (or otherwise unreachable) node: describe for tag
                // + credential attributes; value/disabled stay AX-side with
                // the G0 mask applied.
                let nid = live_nids.get(idx).cloned().unwrap_or(0);
                let mut tag = String::new();
                let mut sens = false;
                if nid != 0 {
                    if let Ok(desc) = browser
                        .command("DOM.describeNode", json!({"nodeId": nid}), true)
                        .await
                    {
                        if let Some(node) = desc.get("node") {
                            tag = node
                                .get("nodeName")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_lowercase();
                            if let Some(attrs) = node.get("attributes").and_then(|v| v.as_array()) {
                                let mut ty = String::new();
                                let mut ac = String::new();
                                let mut i = 0;
                                while i + 1 < attrs.len() {
                                    let k = attrs[i].as_str().unwrap_or("");
                                    let v = attrs[i + 1].as_str().unwrap_or("");
                                    if k.eq_ignore_ascii_case("type") {
                                        ty = v.to_lowercase();
                                    } else if k.eq_ignore_ascii_case("autocomplete") {
                                        ac = v.to_string();
                                    }
                                    i += 2;
                                }
                                const SENS_TOKS: [&str; 6] = [
                                    "current-password",
                                    "new-password",
                                    "cc-number",
                                    "cc-exp",
                                    "cc-csc",
                                    "one-time-code",
                                ];
                                sens = tag == "input"
                                    && (ty == "password"
                                        || ac
                                            .to_lowercase()
                                            .split_whitespace()
                                            .any(|t| SENS_TOKS.contains(&t)));
                            }
                        }
                    }
                }
                it["tag"] = json!(tag);
                it["value"] = json!(if sens {
                    "••••".to_string()
                } else {
                    ax_value
                });
                it["disabled"] = json!(ax_disabled);
            }
            Some(info) => {
                let tag = info
                    .get("tag")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let live_dis = info
                    .get("disabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let live_val = info
                    .get("value")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let sens = info.get("sens").and_then(|v| v.as_bool()).unwrap_or(false);
                let value = if sens {
                    "••••".to_string()
                } else if tag == "input" || tag == "textarea" || tag == "select" {
                    live_val.clone()
                } else {
                    ax_value
                };
                // Unlabeled-control fallback (mirrors the legacy heuristic):
                // an empty AX name on input/textarea/select falls back in
                // legacy order - placeholder, full raw value unless
                // sensitive, selected option text, id, name, type for
                // input/textarea; option text, id, name for select. Ordering
                // runs on the RAW candidates (a whitespace-only string is
                // truthy in the legacy `||` chain, so it blocks fallthrough
                // and then normalizes to ""); the SELECTED fallback is
                // whitespace-normalized and capped at 80 exactly like the
                // legacy `replace(/\s+/g, " ").trim().slice(0, 80)`. The
                // 60-char display cut applies to the shown value only, never
                // to the fallback name; sensitive values stay masked and are
                // never used as names; an all-whitespace pick stays empty.
                let ax_name = it
                    .get("name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                if ax_name.is_empty() && (tag == "input" || tag == "textarea" || tag == "select") {
                    let s = |k: &str| {
                        info.get(k)
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string()
                    };
                    let (full_val, ph, eid, enm, sel_text, ty) = (
                        s("fullVal"),
                        s("ph"),
                        s("eid"),
                        s("enm"),
                        s("selText"),
                        s("ty"),
                    );
                    let mut fb = String::new();
                    if tag == "select" {
                        if !sel_text.is_empty() {
                            fb = sel_text;
                        } else if !eid.is_empty() {
                            fb = eid;
                        } else if !enm.is_empty() {
                            fb = enm;
                        }
                    } else if !ph.is_empty() {
                        fb = ph;
                    } else if !sens && !full_val.is_empty() {
                        fb = full_val;
                    } else if !eid.is_empty() {
                        fb = eid;
                    } else if !enm.is_empty() {
                        fb = enm;
                    } else if tag == "input" && !ty.is_empty() {
                        fb = ty;
                    }
                    if !fb.is_empty() {
                        // Legacy normalization: collapse every whitespace
                        // run to one space, trim, cap at 80 chars.
                        let norm: String = fb.split_whitespace().collect::<Vec<_>>().join(" ");
                        let norm: String = norm.chars().take(80).collect();
                        if !norm.is_empty() {
                            it["name"] = json!(norm);
                        }
                    }
                }
                it["tag"] = json!(tag);
                it["value"] = json!(value);
                it["disabled"] = json!(ax_disabled || live_dis);
            }
        }
    }
    // url/title/text: the frame's own when scoped, else the main page.
    let (url, title, text) = match frame {
        Some(fid) => {
            let pair = browser
                .eval_in_frame(fid, "[window.location.href, document.title]")
                .await
                .ok();
            let u = pair
                .as_ref()
                .and_then(|v| v.get(0))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let t = pair
                .as_ref()
                .and_then(|v| v.get(1))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tv = browser
                .eval_in_frame(
                    fid,
                    "((((document.body && document.body.innerText) || (document.documentElement && document.documentElement.innerText) || '')) + '').slice(0, 12000)",
                )
                .await
                .ok()
                .and_then(|x| x.as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            let tx = tv.split_whitespace().collect::<Vec<_>>().join(" ");
            (u, t, tx)
        }
        None => {
            let uv = browser.eval("window.location.href").await.ok();
            let u = uv
                .and_then(|x| x.as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            let tv2 = browser.eval("document.title").await.ok();
            let t = tv2
                .and_then(|x| x.as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            let tv = browser
                .eval("((((document.body && document.body.innerText) || (document.documentElement && document.documentElement.innerText) || '')) + '').slice(0, 12000)")
                .await;
            let raw = tv
                .ok()
                .and_then(|x| x.as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            let tx = raw.split_whitespace().collect::<Vec<_>>().join(" ");
            (u, t, tx)
        }
    };
    // Store the ref table for exact ref resolution + stale detection.
    let mut table = std::collections::HashMap::new();
    for it in items.iter() {
        if let (Some(r), role, name) = (
            it.get("ref").and_then(|x| x.as_str()),
            it.get("role").and_then(|x| x.as_str()).unwrap_or(""),
            it.get("name").and_then(|x| x.as_str()).unwrap_or(""),
        ) {
            table.insert(r.to_string(), format!("{role} {name}"));
        }
    }
    browser.store_refs(url.clone(), table).await;
    let total = items.len();
    browser.ref_advance(total);
    let mut lines: Vec<String> = Vec::with_capacity(total + 3);
    lines.push(format!("AX snapshot: {title} <{url}>"));
    for it in items.iter() {
        let r = it.get("ref").and_then(|x| x.as_str()).unwrap_or("?");
        let role = it.get("role").and_then(|x| x.as_str()).unwrap_or("");
        let name = it.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let tag = it.get("tag").and_then(|x| x.as_str()).unwrap_or("");
        let val = it.get("value").and_then(|x| x.as_str()).unwrap_or("");
        let dis = it
            .get("disabled")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        let mut ln = format!("- {role} \"{name}\" [@{r}] ({tag})");
        if !val.is_empty() {
            ln.push_str(&format!(" value=\"{val}\""));
        }
        if dis {
            ln.push_str(" [disabled]");
        }
        lines.push(ln);
    }
    // Bounded output with EXPLICIT truncation (announced + by how much).
    // The footer is never cut: lines and the expendable text tail shrink
    // first, so a truncation notice is always present when content is lost.
    let cap = 8000usize;
    let total_lines = lines.len();
    let mut shown = total_lines;
    let mut out = lines.join("\n");
    let mut tail = if text.is_empty() {
        String::new()
    } else {
        format!("\ntext: {text}")
    };
    let mut tail_cut = false;
    loop {
        let mut footer = if total == 0 {
            format!("\n[snapshot: {total} nodes, refs none]")
        } else {
            let first = start + 1;
            let last = start + total;
            format!("\n[snapshot: {total} nodes, refs @e{first}..@e{last}]")
        };
        if shown < total_lines {
            let rest_chars: usize = lines
                .iter()
                .skip(shown)
                .map(|l| l.chars().count() + 1)
                .sum::<usize>()
                + text.chars().count();
            footer.push_str(&format!(
                "\n[truncated: showing {shown} of {total_lines} lines, {rest_chars} chars omitted]"
            ));
        }
        let full = format!("{out}{tail}{footer}");
        if full.chars().count() <= cap {
            let truncated = shown < total_lines || tail_cut;
            return Ok(ok_snapshot(
                full,
                snapshot_nodes(&url, &title, &items, shown, truncated),
            ));
        }
        // Shrink order: text tail first, then whole lines off the end.
        if !tail.is_empty() {
            tail_cut = true;
            let excess = full.chars().count() - cap;
            let keep = tail.chars().count().saturating_sub(excess);
            tail = if keep == 0 {
                String::new()
            } else {
                tail.chars().take(keep).collect()
            };
        } else if shown > 1 {
            shown -= 1;
            out = lines[..shown].join("\n");
        } else {
            // Degenerate: header alone plus footer still exceed the cap;
            // hard-cut (footer stays truthful about the node count).
            let cut: String = full.chars().take(cap).collect();
            return Ok(ok_snapshot(
                cut,
                snapshot_nodes(&url, &title, &items, shown, true),
            ));
        }
    }
}

async fn tool_snapshot_text_frame(
    browser: &Browser,
    frame: Option<&str>,
) -> Result<Value, ToolErr> {
    // Cap the transfer at the source: the page returns an already-sliced
    // string so huge pages never balloon memory on this side; the 8000-char
    // cap below still applies after whitespace collapsing. With frame= the
    // frame's own text is reported (isolated world, works cross-origin).
    let v = match frame {
        Some(fid) => {
            browser
                .eval_in_frame(
                    fid,
                    "((((document.body && document.body.innerText) || (document.documentElement && document.documentElement.innerText) || '')) + '').slice(0, 12000)",
                )
                .await?
        }
        None => {
            browser
                .eval("((((document.body && document.body.innerText) || (document.documentElement && document.documentElement.innerText) || '')) + '').slice(0, 12000)")
                .await?
        }
    };
    let raw = v.as_str().unwrap_or("").to_string();
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    // Text-only note (no AX refs exist): reserve its room so the total
    // never exceeds the 8000-char cap the suite asserts.
    let lines = raw.lines().count().max(1);
    let suffix = format!("\n[text-only snapshot: {lines} lines, no element refs]");
    let budget = 8000usize.saturating_sub(suffix.len());
    let capped: String = if collapsed.chars().count() > budget {
        collapsed.chars().take(budget).collect()
    } else {
        collapsed
    };
    let prose = format!("{capped}{suffix}");
    Ok(json!({"content": [{"type": "text", "text": prose.clone()}],
              "structuredContent": {"mode": "text", "text": prose}}))
}

/// Write an already-decoded screenshot PNG to `path`. There is no
/// allowlisted screenshot root in this crate, so only an ABSOLUTE path is
/// accepted (a relative path is a typed `invalid_args`/`bad_value` refusal,
/// never resolved against a working directory). Missing parent directories
/// are created; any I/O failure is a typed error, never a silent success.
/// Split out from `tool_screenshot` so the file path is unit-testable
/// without a browser.
fn write_screenshot_file(path: &str, png: &[u8]) -> Result<(), ToolErr> {
    let dest = std::path::Path::new(path);
    if !dest.is_absolute() {
        return Err(ToolErr::typed(
            format!("screenshot: refusing relative path (pass an absolute path): {path}"),
            json!({"code": "invalid_args", "reason": "bad_value", "tool": "screenshot", "arg": "path"}),
        ));
    }
    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                ToolErr::fallback(format!("screenshot: cannot create parent directory: {e}"))
            })?;
        }
    }
    std::fs::write(dest, png)
        .map_err(|e| ToolErr::fallback(format!("screenshot: save failed: {e}")))?;
    Ok(())
}

async fn tool_screenshot(
    browser: &Browser,
    full_page: bool,
    path: Option<&str>,
    deterministic: bool,
    width: Option<i64>,
    height: Option<i64>,
) -> Result<Value, ToolErr> {
    // Capability gate: a dom-only backend has no rendering pipeline, so
    // there are no pixels to capture. Refuse with the backend and the
    // reason, never a fake or blank image. Checked before any browser
    // launch, so a dom-only refusal never spawns an engine.
    // (`upload` needs no gate: it speaks DOM CDP only -
    // DOM.enable/getDocument/querySelector/setFileInputFiles - with no
    // compositor/pixel path. Verified, not assumed.)
    match crate::browser::backend_caps() {
        Ok(crate::browser::BackendCaps::DomOnly) => {
            return Err(ToolErr::typed(
                "screenshot: unavailable with GLADIUS_BACKEND_CAPS=dom-only backend (no rendering pipeline; refusing instead of returning a fake image)".to_string(),
                json!({"code": "capability_gated", "reason": "dom_only", "tool": "screenshot"}),
            ));
        }
        Ok(crate::browser::BackendCaps::Render) => {}
        Err(e) => return Err(ToolErr::fallback(format!("screenshot: {e}"))),
    }
    let cap = crate::capture::capture(
        browser,
        &crate::capture::Options {
            full_page,
            deterministic,
            width,
            height,
        },
    )
    .await?;
    if let Some(p) = path {
        write_screenshot_file(p, &cap.bytes)?;
    }
    let mut meta = match cap.meta() {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    if let Some(p) = path {
        meta.insert("path".to_string(), json!(p));
    }
    let meta = Value::Object(meta);
    Ok(
        json!({"content": [{"type": "image", "data": cap.base64, "mimeType": "image/png"},
                          {"type": "text", "text": meta.to_string()}],
              "structuredContent": meta}),
    )
}

/// Parse the `ref` / `selector` targeting of the geometry tools (string or
/// array of strings; refs must be `@eN` and known to the last snapshot; an
/// empty target set is refused rather than silently measuring nothing).
async fn layout_specs(
    browser: &Browser,
    tool: &str,
    args: &Value,
) -> Result<Vec<crate::layout::Spec>, ToolErr> {
    let mut out: Vec<crate::layout::Spec> = Vec::new();
    for key in ["ref", "selector"] {
        let Some(v) = args.get(key) else { continue };
        if v.is_null() {
            continue;
        }
        let items: Vec<String> = match v {
            Value::String(s) => vec![s.clone()],
            Value::Array(a) => {
                let mut acc = Vec::with_capacity(a.len());
                for x in a {
                    match x.as_str() {
                        Some(s) => acc.push(s.to_string()),
                        None => {
                            return Err(ToolErr::typed(
                                format!("{tool}: '{key}' array must contain strings"),
                                json!({"code": "invalid_args", "reason": "bad_value", "tool": tool, "arg": key}),
                            ))
                        }
                    }
                }
                acc
            }
            _ => {
                return Err(ToolErr::typed(
                    format!("{tool}: '{key}' must be a string or an array of strings"),
                    json!({"code": "invalid_args", "reason": "bad_value", "tool": tool, "arg": key}),
                ))
            }
        };
        for s in items {
            if key == "ref" {
                let Some(id) = parse_ref(&s) else {
                    return Err(ToolErr::typed(
                        format!("{tool}: ref '{s}' must look like @eN (from snapshot)"),
                        json!({"code": "invalid_args", "reason": "bad_value", "tool": tool, "arg": "ref"}),
                    ));
                };
                guard_ref(browser, tool, &s, &id).await?;
                out.push(crate::layout::Spec::Ref(id));
            } else {
                out.push(crate::layout::Spec::Selector(s));
            }
        }
    }
    if out.is_empty() {
        return Err(ToolErr::typed(
            format!("{tool}: need 'ref' (@eN) or 'selector'"),
            json!({"code": "invalid_args", "reason": "missing_arg", "tool": tool}),
        ));
    }
    Ok(out)
}

fn not_found_baseline(name: &str) -> ToolErr {
    ToolErr::typed(
        format!(
            "visual: no baseline named '{name}' in {} (save it first)",
            crate::visual::visual_dir().display()
        ),
        json!({"code": "invalid_target", "reason": "not_found", "tool": "visual", "name": name}),
    )
}

/// `assert_layout` body (kept out of the dispatch match: see the boxed arms).
async fn tool_assert_layout(browser: &Browser, args: &Value) -> Result<Value, ToolErr> {
    check_unknown(
        "assert_layout",
        args,
        &[
            "ref",
            "selector",
            "checks",
            "screenshotOnFail",
            "screenshotPath",
        ],
    )?;
    let specs = layout_specs(browser, "assert_layout", args).await?;
    let on_fail = args
        .get("screenshotOnFail")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let shot_path = args.get("screenshotPath").and_then(|v| v.as_str());
    if shot_path.is_some() && !on_fail {
        return Err(ToolErr::typed(
            "assert_layout: 'screenshotPath' requires screenshotOnFail:true".to_string(),
            json!({"code": "invalid_args", "reason": "need_option", "tool": "assert_layout", "arg": "screenshotPath"}),
        ));
    }
    crate::layout::assert_layout(browser, &specs, args.get("checks"), on_fail, shot_path)
        .await
        .map(|v| ok_text(v.to_string()))
}

/// `layout` body.
async fn tool_layout(browser: &Browser, args: &Value) -> Result<Value, ToolErr> {
    check_unknown("layout", args, &["ref", "selector", "includeOverlaps"])?;
    let specs = layout_specs(browser, "layout", args).await?;
    let overlaps = args
        .get("includeOverlaps")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    crate::layout::layout(browser, &specs, overlaps)
        .await
        .map(|v| ok_text(v.to_string()))
}

/// `visual` body: save / compare / list / delete.
async fn tool_visual(browser: &Browser, args: &Value) -> Result<Value, ToolErr> {
    check_unknown(
        "visual",
        args,
        &[
            "action",
            "name",
            "fullPage",
            "threshold",
            "maxDiffPixels",
            "maxDiffRatio",
            "maskRects",
            "update",
            "antialias",
            "includeAA",
            "minDiffCluster",
            "requireEnvironmentMatch",
        ],
    )?;
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
    // save/compare need a rendering pipeline; list/delete touch files only.
    if matches!(action, "save" | "compare") {
        match crate::browser::backend_caps() {
            Ok(crate::browser::BackendCaps::DomOnly) => {
                return Err(ToolErr::typed(
                    format!("visual: action '{action}' is unavailable with GLADIUS_BACKEND_CAPS=dom-only (no rendering pipeline; refusing instead of comparing nothing)"),
                    json!({"code": "capability_gated", "reason": "dom_only", "tool": "visual"}),
                ));
            }
            Ok(crate::browser::BackendCaps::Render) => {}
            Err(e) => return Err(ToolErr::fallback(format!("visual: {e}"))),
        }
    }
    let name = match action {
        "list" => None,
        _ => match args.get("name").and_then(|v| v.as_str()) {
            Some(n) => Some(n.to_string()),
            None => {
                return Err(ToolErr::typed(
                    format!("visual: action '{action}' needs 'name'"),
                    json!({"code": "invalid_args", "reason": "missing_arg", "tool": "visual", "arg": "name"}),
                ))
            }
        },
    };
    let safe = match &name {
        Some(n) => Some(crate::visual::sanitise_name(n).map_err(|e| {
            ToolErr::typed(
                e,
                json!({"code": "invalid_args", "reason": "bad_value", "tool": "visual", "arg": "name"}),
            )
        })?),
        None => None,
    };
    let safe = safe.as_deref().unwrap_or_default();
    match action {
        "save" => {
            let full = args
                .get("fullPage")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            crate::visual::save(browser, safe, full)
                .await
                .map(|v| ok_text(v.to_string()))
                .map_err(ToolErr::from)
        }
        "list" => crate::visual::list()
            .map(|v| ok_text(v.to_string()))
            .map_err(ToolErr::from),
        "delete" => {
            if !crate::visual::exists(safe) {
                return Err(not_found_baseline(safe));
            }
            crate::visual::delete(safe)
                .map(|v| ok_text(v.to_string()))
                .map_err(ToolErr::from)
        }
        "compare" => {
            if !crate::visual::exists(safe) {
                return Err(not_found_baseline(safe));
            }
            // `antialias` (default true) and `includeAA` (default false) are
            // inverses. Both may be sent only when they agree; a contradiction
            // is refused rather than silently resolved.
            let antialias_arg = args.get("antialias").and_then(|v| v.as_bool());
            let include_aa = args.get("includeAA").and_then(|v| v.as_bool());
            if let (Some(a), Some(i)) = (antialias_arg, include_aa) {
                if a == i {
                    return Err(ToolErr::typed(
                        "visual: 'antialias' and 'includeAA' are inverses; send one, or send both with opposite values (antialias = !includeAA)".to_string(),
                        json!({"code": "invalid_args", "reason": "bad_value", "tool": "visual", "arg": "antialias"}),
                    ));
                }
            }
            let antialias = match (antialias_arg, include_aa) {
                (Some(a), _) => a,
                (None, Some(i)) => !i,
                (None, None) => true,
            };
            let min_cluster = args
                .get("minDiffCluster")
                .and_then(|v| v.as_i64())
                .unwrap_or(2);
            if !(0..=1000).contains(&min_cluster) {
                return Err(ToolErr::typed(
                    "visual: 'minDiffCluster' must be an integer in 0..=1000 (0 counts every pixel)".to_string(),
                    json!({"code": "invalid_args", "reason": "bad_value", "tool": "visual", "arg": "minDiffCluster"}),
                ));
            }
            let opt = crate::visual::DiffOpts {
                threshold: args
                    .get("threshold")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.1),
                max_diff_pixels: args
                    .get("maxDiffPixels")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
                max_diff_ratio: args
                    .get("maxDiffRatio")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
                mask_rects: args
                    .get("maskRects")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default(),
                full_page: args
                    .get("fullPage")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                update: args
                    .get("update")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                antialias,
                min_diff_cluster: min_cluster,
                require_environment_match: args
                    .get("requireEnvironmentMatch")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            };
            // A per-channel threshold is a 0..1 fraction; a value outside
            // that range would silently compare nothing (or everything), so
            // refuse it instead.
            if !(0.0..=1.0).contains(&opt.threshold) {
                return Err(ToolErr::typed(
                    "visual: 'threshold' must be a fraction in 0..=1 (per-channel share of the 0-255 range)".to_string(),
                    json!({"code": "invalid_args", "reason": "bad_value", "tool": "visual", "arg": "threshold"}),
                ));
            }
            // `compare` carries its own typed errors (scratch refusal, unknown
            // baseline environment), so they pass through unchanged.
            crate::visual::compare(browser, safe, &opt)
                .await
                .map(|v| ok_text(v.to_string()))
        }
        other => Err(ToolErr::typed(
            format!("visual: unknown action '{other}' (want save|compare|list|delete)"),
            json!({"code": "invalid_args", "reason": "unknown_action", "tool": "visual", "arg": "action"}),
        )),
    }
}

/// Main-frame click preflight (shared by the ref and selector paths): resolve,
/// visibility/disabled checks (as before), `scrollIntoView`, rect center, then
/// a `document.elementFromPoint(cx, cy)` hit-test. The hit element must be the
/// target itself or a descendant of it; anything else (an opaque overlay, a
/// covering sibling) is a typed `not_actionable`/`occluded` and no click is
/// dispatched.
///
/// `<option>` fallback (documented, the ONLY synthetic-click path): an
/// `<option>` has no layout box, so a pointer sequence cannot hit-test or land
/// on it. That single case keeps the old in-page `el.click()`; every rendered
/// element goes through the trusted CDP pointer sequence in `click_dispatch`.
fn click_preflight_ref(id: &str) -> (String, String) {
    let rid = serde_json::to_string(id).unwrap_or_else(|_| "\"\"".to_string());
    let expr = format!(
        "((ref) => {{ const el = document.querySelector('[data-gladius-ref=\"' + ref + '\"]'); if (!el) return 'GLADIUS_NOTFOUND'; const cs = getComputedStyle(el); if (cs.display === 'none' || cs.visibility === 'hidden' || cs.visibility === 'collapse') return 'GLADIUS_HIDDEN'; if (el.disabled) return 'GLADIUS_DISABLED'; const tag = (el.tagName || '').toUpperCase(); if (tag === 'OPTION') {{ const h0 = location.href; try {{ el.scrollIntoView({{block: 'center'}}); }} catch (e) {{}} try {{ el.click(); }} catch (e) {{ return 'GLADIUS_HIDDEN'; }} try {{ return 'GLADIUS_FALLBACK:' + JSON.stringify({{h0: h0, h1: location.href, rs: document.readyState}}); }} catch (e) {{ return 'GLADIUS_FALLBACK'; }} }} try {{ el.scrollIntoView({{block: 'center'}}); }} catch (e) {{}} const r = el.getBoundingClientRect(); if (!r || r.width === 0 || r.height === 0) return 'GLADIUS_HIDDEN'; const cx = r.left + r.width / 2, cy = r.top + r.height / 2; let hit = null; try {{ hit = document.elementFromPoint(cx, cy); }} catch (e) {{ return 'GLADIUS_OCCLUDED'; }} if (!hit || (hit !== el && !el.contains(hit))) return 'GLADIUS_OCCLUDED'; return 'GLADIUS_POINT:' + JSON.stringify({{x: cx, y: cy, h0: location.href}}); }})({rid})"
    );
    (expr, rid)
}

fn click_preflight_sel(sel: &str) -> String {
    format!(
        "(() => {{ const el = document.querySelector({sel}); if (!el) return 'GLADIUS_NOTFOUND'; const cs = getComputedStyle(el); if (cs.display === 'none' || cs.visibility === 'hidden' || cs.visibility === 'collapse') return 'GLADIUS_HIDDEN'; if (el.disabled) return 'GLADIUS_DISABLED'; const tag = (el.tagName || '').toUpperCase(); if (tag === 'OPTION') {{ const h0 = location.href; try {{ el.scrollIntoView({{block: 'center'}}); }} catch (e) {{}} try {{ el.click(); }} catch (e) {{ return 'GLADIUS_HIDDEN'; }} try {{ return 'GLADIUS_FALLBACK:' + JSON.stringify({{h0: h0, h1: location.href, rs: document.readyState}}); }} catch (e) {{ return 'GLADIUS_FALLBACK'; }} }} try {{ el.scrollIntoView({{block: 'center'}}); }} catch (e) {{}} const r = el.getBoundingClientRect(); if (!r || r.width === 0 || r.height === 0) return 'GLADIUS_HIDDEN'; const cx = r.left + r.width / 2, cy = r.top + r.height / 2; let hit = null; try {{ hit = document.elementFromPoint(cx, cy); }} catch (e) {{ return 'GLADIUS_OCCLUDED'; }} if (!hit || (hit !== el && !el.contains(hit))) return 'GLADIUS_OCCLUDED'; return 'GLADIUS_POINT:' + JSON.stringify({{x: cx, y: cy, h0: location.href}}); }})()"
    )
}

/// Dispatch a trusted main-frame pointer click at `(x, y)` (viewport CSS px)
/// via `Browser::mouse_click`, then build the `{h0, h1, rs}` settle state the
/// same shape the old in-page click reported (h0 from preflight, h1/rs read
/// live after dispatch). A CDP dispatch failure is a typed error at the call
/// site (never a silent no-op); the settle/dialog/`click_ok` path is unchanged.
async fn click_dispatch(
    browser: &Browser,
    x: f64,
    y: f64,
    h0: &str,
    hover_first: bool,
) -> Result<String, ToolErr> {
    browser
        .mouse_click(x, y, hover_first)
        .await
        .map_err(ToolErr::from)?;
    let live = browser.eval("[location.href, document.readyState]").await?;
    let h1 = live.get(0).and_then(|v| v.as_str()).unwrap_or(h0);
    let rs = live.get(1).and_then(|v| v.as_str()).unwrap_or("complete");
    let st = serde_json::json!({"h0": h0, "h1": h1, "rs": rs}).to_string();
    Ok(st)
}

async fn tool_click(
    browser: &Browser,
    element: &str,
    frame: Option<&str>,
    hover_first: bool,
) -> Result<Value, ToolErr> {
    // Drain any earlier dialog record so one found below belongs to THIS click.
    browser.take_dialog().await;
    let fid = frame_id_ok("click", frame)?;
    if let Some(ref f) = fid {
        // Frame policy: refs cannot resolve safely in a frame realm.
        if parse_ref(element).is_some() {
            return Err(ref_in_frame("click", element, f));
        }
        return tool_click_frame(browser, element, f, hover_first).await;
    }
    if let Some(id) = parse_ref(element) {
        let at_id = guard_ref(browser, "click", element, &id).await?;
        let (expr, _) = click_preflight_ref(&id);
        let v = browser.eval(&expr).await?;
        return match v.as_str() {
            Some(s) if s.starts_with("GLADIUS_FALLBACK:") => {
                // Documented <option> fallback only (no layout box for a
                // pointer sequence); settle + dialog reporting as before.
                click_settle(browser, &s["GLADIUS_FALLBACK:".len()..]).await;
                Ok(click_ok(browser).await)
            }
            Some("GLADIUS_FALLBACK") => {
                click_settle(browser, "").await;
                Ok(click_ok(browser).await)
            }
            Some(s) if s.starts_with("GLADIUS_POINT:") => {
                let pt: Value = serde_json::from_str(&s["GLADIUS_POINT:".len()..])
                    .map_err(|_| "click: no layout point".to_string())?;
                let x = pt.get("x").and_then(|n| n.as_f64()).unwrap_or(0.0);
                let y = pt.get("y").and_then(|n| n.as_f64()).unwrap_or(0.0);
                let h0 = pt
                    .get("h0")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let st = click_dispatch(browser, x, y, &h0, hover_first).await?;
                click_settle(browser, &st).await;
                Ok(click_ok(browser).await)
            }
            Some("GLADIUS_NOTFOUND") => Err(stale_gone("click", element, &id)),
            Some("GLADIUS_DISABLED") => Err(ToolErr::typed(
                format!("click: {at_id} is disabled"),
                json!({"code": "not_actionable", "reason": "disabled", "tool": "click", "ref": at_id}),
            )),
            Some("GLADIUS_OCCLUDED") => Err(ToolErr::typed(
                format!("click: {at_id} is occluded by another element (no click dispatched)"),
                json!({"code": "not_actionable", "reason": "occluded", "tool": "click", "ref": at_id}),
            )),
            _ => Err(ToolErr::typed(
                format!("click: {at_id} not visible or not clickable"),
                json!({"code": "not_actionable", "reason": "hidden", "tool": "click", "ref": at_id}),
            )),
        };
    }
    let sel = serde_json::to_string(element).map_err(|e| e.to_string())?;
    // Bounded presence wait (3s cap): page load is navigate's job (10s load
    // poll); once the page reports complete, a static element is either
    // present or never will be. True misses (err-02, click-03's removed
    // link) return the IDENTICAL error text at 3s as at 10s (measured H2H:
    // both burned the full 10s for nothing); slow-page arrivals stay covered
    // by navigate's poll, not here.
    {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            // Clip each probe to the REMAINING budget (the same wrap the
            // `wait_for` tool already uses): a stalled peer must not turn
            // this 3s presence cap into a 30s `send_raw` timeout.
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let probe = format!("(!!document.querySelector({sel}))");
            if let Ok(Ok(Value::Bool(true))) =
                tokio::time::timeout(remaining, browser.eval(&probe)).await
            {
                break;
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    let expr = click_preflight_sel(&sel);
    let v = browser.eval(&expr).await?;
    match v.as_str() {
        Some(s) if s.starts_with("GLADIUS_FALLBACK:") => {
            click_settle(browser, &s["GLADIUS_FALLBACK:".len()..]).await;
            Ok(click_ok(browser).await)
        }
        Some("GLADIUS_FALLBACK") => {
            click_settle(browser, "").await;
            Ok(click_ok(browser).await)
        }
        Some(s) if s.starts_with("GLADIUS_POINT:") => {
            let pt: Value = serde_json::from_str(&s["GLADIUS_POINT:".len()..])
                .map_err(|_| "click: no layout point".to_string())?;
            let x = pt.get("x").and_then(|n| n.as_f64()).unwrap_or(0.0);
            let y = pt.get("y").and_then(|n| n.as_f64()).unwrap_or(0.0);
            let h0 = pt
                .get("h0")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let st = click_dispatch(browser, x, y, &h0, hover_first).await?;
            click_settle(browser, &st).await;
            Ok(click_ok(browser).await)
        }
        Some("GLADIUS_NOTFOUND") => Err(ToolErr::typed(
            format!("click: element not found: {element}"),
            json!({"code": "invalid_target", "reason": "not_found", "tool": "click", "element": element}),
        )),
        Some("GLADIUS_DISABLED") => Err(ToolErr::typed(
            format!("click: element is disabled: {element}"),
            json!({"code": "not_actionable", "reason": "disabled", "tool": "click", "element": element}),
        )),
        Some("GLADIUS_OCCLUDED") => Err(ToolErr::typed(
            format!(
                "click: element is occluded by another element (no click dispatched): {element}"
            ),
            json!({"code": "not_actionable", "reason": "occluded", "tool": "click", "element": element}),
        )),
        _ => Err(ToolErr::typed(
            format!("click: element not visible or not clickable: {element}"),
            json!({"code": "not_actionable", "reason": "hidden", "tool": "click", "element": element}),
        )),
    }
}

/// G4b frame-scoped click: the selector preflight (resolve, visibility,
/// disabled, scrollIntoView, `<option>` fallback, elementFromPoint hit-test)
/// runs in the FRAME realm via `eval_in_frame` (same trusted logic as the
/// main frame, but inside the iframe). The leaf center (frame-viewport CSS px)
/// is mapped to main-frame viewport CSS px through the owner rect chain
/// (`frame_chain_points`: owner left/top + padding + border per ancestor, JS rect
/// chain, never getContentQuads), a parent-side overlay guard hit-tests the
/// final point, and the trusted CDP pointer sequence dispatches on the page
/// session at main-frame viewport px. OOPIF/unmapped frames are typed errors.
async fn tool_click_frame(
    browser: &Browser,
    element: &str,
    fid: &str,
    hover_first: bool,
) -> Result<Value, ToolErr> {
    let sel = serde_json::to_string(element).map_err(|e| e.to_string())?;
    // Bounded presence wait inside the frame realm (same 3s cap shape as main).
    {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let probe = format!("(!!document.querySelector({sel}))");
            if let Ok(Ok(Value::Bool(true))) =
                tokio::time::timeout(remaining, frame_realm_eval(browser, "click", fid, &probe))
                    .await
            {
                break;
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    let expr = click_preflight_sel(&sel);
    let v = frame_realm_eval(browser, "click", fid, &expr).await?;
    match v.as_str() {
        Some(s) if s.starts_with("GLADIUS_FALLBACK:") => {
            // Frame `<option>` fallback: no pointer sequence, no navigation;
            // settle empty (the frame-href tail would mismatch the main href
            // and burn the load wait).
            click_settle(browser, "").await;
            Ok(click_ok(browser).await)
        }
        Some("GLADIUS_FALLBACK") => {
            click_settle(browser, "").await;
            Ok(click_ok(browser).await)
        }
        Some(s) if s.starts_with("GLADIUS_POINT:") => {
            let pt: Value = serde_json::from_str(&s["GLADIUS_POINT:".len()..])
                .map_err(|_| "click: no layout point".to_string())?;
            let lx = pt.get("x").and_then(|n| n.as_f64()).unwrap_or(0.0);
            let ly = pt.get("y").and_then(|n| n.as_f64()).unwrap_or(0.0);
            // h0 must be the MAIN href (click_settle compares top-level
            // flight): the frame realm's own href would always mismatch and
            // burn the 10s load wait on every frame click.
            let h0 = browser
                .eval("window.location.href")
                .await
                .ok()
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            // Per-level parent-side hit-test against the TRUE owners plus the
            // main-viewport point: a covering overlay or a different iframe
            // at ANY level is typed occluded, never a click into the wrong
            // surface.
            let (mx, my) = frame_owner_hit_ok(browser, "click", element, fid, lx, ly).await?;
            let st = click_dispatch(browser, mx, my, &h0, hover_first).await?;
            click_settle(browser, &st).await;
            Ok(click_ok(browser).await)
        }
        Some("GLADIUS_NOTFOUND") => Err(ToolErr::typed(
            format!("click: element not found: {element}"),
            json!({"code": "invalid_target", "reason": "not_found", "tool": "click", "element": element, "frame": fid}),
        )),
        Some("GLADIUS_DISABLED") => Err(ToolErr::typed(
            format!("click: element is disabled: {element}"),
            json!({"code": "not_actionable", "reason": "disabled", "tool": "click", "element": element, "frame": fid}),
        )),
        Some("GLADIUS_OCCLUDED") => Err(ToolErr::typed(
            format!(
                "click: element is occluded by another element (no click dispatched): {element}"
            ),
            json!({"code": "not_actionable", "reason": "occluded", "tool": "click", "element": element, "frame": fid}),
        )),
        _ => Err(ToolErr::typed(
            format!("click: element not visible or not clickable: {element}"),
            json!({"code": "not_actionable", "reason": "hidden", "tool": "click", "element": element, "frame": fid}),
        )),
    }
}

/// Settle a click that navigates (measured H2H form-02: submit click
/// returned before the GET committed, evaluate read the OLD page while both
/// refs settled on the navigation). `state` is the JSON tail of the click
/// eval (`{h0, h1, rs}` observed synchronously in-page right after dispatch;
/// "" when the eval predates this reporting).
///
/// Cost: zero extra round-trips when the click already shows flight
/// (h1 != h0 or not complete -> straight to the event-driven load wait);
/// otherwise a SHORT bounded quiet watch (120 ms at 10 ms cadence) for a
/// late commit (measured: the selenium GET commits ~100 ms after dispatch).
/// Clicks that never navigate (tab-01's new-tab, dlg-01's alert, plain
/// buttons) pay only the quiet window instead of the old 1 s watch, and
/// return on the old page, unchanged.
async fn click_settle(browser: &Browser, state: &str) {
    let st: Value = serde_json::from_str(state).unwrap_or(Value::Null);
    let h0 = st.get("h0").and_then(|v| v.as_str()).unwrap_or("");
    let h1 = st.get("h1").and_then(|v| v.as_str()).unwrap_or(h0);
    let rs = st.get("rs").and_then(|v| v.as_str()).unwrap_or("complete");
    if h1 != h0 || rs != "complete" {
        browser
            .wait_for_load(std::time::Duration::from_secs(10))
            .await;
        return;
    }
    if h0.is_empty() {
        return;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(120);
    loop {
        // A dialog record means the page's JS thread was blocked by THIS
        // click (drained before dispatch, so it is ours); the reader already
        // accepted it, so hand off to the load wait - one eval when nothing
        // further navigates - and let click_ok report it as before.
        if browser.last_dialog().await.is_some() {
            browser
                .wait_for_load(std::time::Duration::from_secs(10))
                .await;
            return;
        }
        if let Ok(v) = browser.eval("[location.href, document.readyState]").await {
            let href = v.get(0).and_then(|x| x.as_str()).unwrap_or(h0);
            let cur = v.get(1).and_then(|x| x.as_str()).unwrap_or("complete");
            if href != h0 || cur != "complete" {
                browser
                    .wait_for_load(std::time::Duration::from_secs(10))
                    .await;
                return;
            }
        }
        if std::time::Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Success envelope for a click. When the click opened a JS dialog, the
/// reader auto-accepted it (the page is unblocked) and the outcome carries
/// a typed dialog report with the dialog's own message text - never a
/// silent ok, never a hang.
async fn click_ok(browser: &Browser) -> Value {
    match browser.last_dialog().await {
        Some(d) => {
            // Truthful verb: the record keeps the policy it ran under, so a
            // dismissed dialog reports dismissed, an accepted one accepted.
            let verb = if d.accepted { "accepted" } else { "dismissed" };
            ok_text(
            json!({"ok": true, "dialog": {"type": d.dtype, "message": d.message, "url": d.url},
                   "note": format!("dialog auto-handled: {} {:?} ({verb}, page unblocked)", d.dtype, d.message)})
                .to_string(),
            )
        }
        None => ok_text(json!({"ok": true}).to_string()),
    }
}

/// Main-frame `type` preflight: resolve, visibility/disabled/readOnly checks,
/// editable-kind gate, then focus plus select-all so `Input.insertText`
/// REPLACES. Text inputs use `el.focus()` plus `el.select()`; contenteditable
/// uses `el.focus()` plus select-all of the element's OWN contents
/// (`Range.selectNodeContents`), so insertText replaces text WITHIN the
/// editable element while siblings outside it stay untouched.
///
/// Non-editable targets (file inputs, date/time-like and other non-text input
/// types, non-editable elements) return `GLADIUS_NOTEDIT:<kind>` and the tool
/// reports typed `not_actionable`/`not_editable` (never a silent no-op).
/// There is NO synthetic value-setter path: if `Input.insertText` fails, the
/// tool returns a typed error and does not fall back.
fn type_preflight_ref(id: &str) -> String {
    let rid = serde_json::to_string(id).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        "((ref) => {{ const el = document.querySelector('[data-gladius-ref=\"' + ref + '\"]'); if (!el) return 'GLADIUS_NOTFOUND'; if (el.disabled) return 'GLADIUS_DISABLED'; if (el.readOnly) return 'GLADIUS_READONLY'; const cs = getComputedStyle(el); if (cs.display === 'none' || cs.visibility === 'hidden' || cs.visibility === 'collapse') return 'GLADIUS_HIDDEN'; const r = el.getBoundingClientRect(); if (!r || (r.width === 0 && r.height === 0)) return 'GLADIUS_HIDDEN'; const tag = (el.tagName || '').toUpperCase(); let kind = ''; if (tag === 'INPUT') {{ const ty = String(el.type || 'text').toLowerCase(); if (['text','search','url','tel','password','email','number'].includes(ty)) kind = 'TEXT'; else kind = 'NOTEDIT:' + ty; }} else if (tag === 'TEXTAREA') kind = 'TEXT'; else if (el.isContentEditable) kind = 'CE'; else kind = 'NOTEDIT:tag:' + tag; if (kind.startsWith('NOTEDIT:')) return 'GLADIUS_NOTEDIT:' + kind.slice(8); try {{ el.focus(); }} catch (e) {{ return 'GLADIUS_HIDDEN'; }} try {{ if (kind === 'TEXT') {{ if (typeof el.select === 'function') el.select(); else if (typeof el.setSelectionRange === 'function') el.setSelectionRange(0, el.value.length); }} else {{ const range = document.createRange(); range.selectNodeContents(el); const sel = getSelection(); sel.removeAllRanges(); sel.addRange(range); }} }} catch (e) {{ return 'GLADIUS_HIDDEN'; }} return 'GLADIUS_READY'; }})({rid})"
    )
}

fn type_preflight_sel(sel: &str) -> String {
    format!(
        "(() => {{ const el = document.querySelector({sel}); if (!el) return 'GLADIUS_NOTFOUND'; if (el.disabled) return 'GLADIUS_DISABLED'; if (el.readOnly) return 'GLADIUS_READONLY'; const cs = getComputedStyle(el); if (cs.display === 'none' || cs.visibility === 'hidden' || cs.visibility === 'collapse') return 'GLADIUS_HIDDEN'; const r = el.getBoundingClientRect(); if (!r || (r.width === 0 && r.height === 0)) return 'GLADIUS_HIDDEN'; const tag = (el.tagName || '').toUpperCase(); let kind = ''; if (tag === 'INPUT') {{ const ty = String(el.type || 'text').toLowerCase(); if (['text','search','url','tel','password','email','number'].includes(ty)) kind = 'TEXT'; else kind = 'NOTEDIT:' + ty; }} else if (tag === 'TEXTAREA') kind = 'TEXT'; else if (el.isContentEditable) kind = 'CE'; else kind = 'NOTEDIT:tag:' + tag; if (kind.startsWith('NOTEDIT:')) return 'GLADIUS_NOTEDIT:' + kind.slice(8); try {{ el.focus(); }} catch (e) {{ return 'GLADIUS_HIDDEN'; }} try {{ if (kind === 'TEXT') {{ if (typeof el.select === 'function') el.select(); else if (typeof el.setSelectionRange === 'function') el.setSelectionRange(0, el.value.length); }} else {{ const range = document.createRange(); range.selectNodeContents(el); const sel = getSelection(); sel.removeAllRanges(); sel.addRange(range); }} }} catch (e) {{ return 'GLADIUS_HIDDEN'; }} return 'GLADIUS_READY'; }})()"
    )
}

fn type_not_editable(tool: &str, detail: &str, ctx: &Value) -> ToolErr {
    let mut sc = serde_json::Map::new();
    sc.insert("code".to_string(), json!("not_actionable"));
    sc.insert("reason".to_string(), json!("not_editable"));
    sc.insert("tool".to_string(), json!(tool));
    if let Some(r) = ctx.get("ref").and_then(|v| v.as_str()) {
        sc.insert("ref".to_string(), json!(r));
        ToolErr::typed(
            format!("type: {r} is not editable for typing (input type '{detail}'; use the matching tool)"),
            Value::Object(sc),
        )
    } else if let Some(e) = ctx.get("element").and_then(|v| v.as_str()) {
        sc.insert("element".to_string(), json!(e));
        ToolErr::typed(
            format!("type: element is not editable for typing (input type '{detail}'): {e}"),
            Value::Object(sc),
        )
    } else {
        ToolErr::typed(
            format!("type: target is not editable for typing (input type '{detail}')"),
            Value::Object(sc),
        )
    }
}

async fn tool_type(
    browser: &Browser,
    element: &str,
    text: &str,
    submit: bool,
    frame: Option<&str>,
) -> Result<Value, ToolErr> {
    let fid = frame_id_ok("type", frame)?;
    if let Some(ref f) = fid {
        if parse_ref(element).is_some() {
            return Err(ref_in_frame("type", element, f));
        }
        let sel = serde_json::to_string(element).map_err(|e| e.to_string())?;
        let expr = type_preflight_sel(&sel);
        let v = frame_realm_eval(browser, "type", f, &expr).await?;
        match v.as_str() {
            Some("GLADIUS_READY") => {}
            Some("GLADIUS_DISABLED") => {
                return Err(ToolErr::typed(
                    format!("type: element is disabled: {element}"),
                    json!({"code": "not_actionable", "reason": "disabled", "tool": "type", "element": element, "frame": f}),
                ));
            }
            Some("GLADIUS_READONLY") => {
                return Err(ToolErr::typed(
                    format!("type: element is read-only: {element}"),
                    json!({"code": "not_actionable", "reason": "readonly", "tool": "type", "element": element, "frame": f}),
                ));
            }
            Some("GLADIUS_HIDDEN") => {
                return Err(ToolErr::typed(
                    format!("type: element not visible or not editable: {element}"),
                    json!({"code": "not_actionable", "reason": "hidden", "tool": "type", "element": element, "frame": f}),
                ));
            }
            Some(s) if s.starts_with("GLADIUS_NOTEDIT:") => {
                let kind = &s["GLADIUS_NOTEDIT:".len()..];
                return Err(type_not_editable(
                    "type",
                    kind,
                    &json!({"element": element, "frame": f}),
                ));
            }
            _ => {
                return Err(ToolErr::typed(
                    format!("type: element not found: {element}"),
                    json!({"code": "invalid_target", "reason": "not_found", "tool": "type", "element": element, "frame": f}),
                ));
            }
        }
        // Focus + select-all ran in the frame realm; `Input.insertText`
        // dispatches to the frame's focused editable on the page session.
        browser.insert_text(text).await.map_err(|e| {
            type_not_editable(
                "type",
                &format!("insertText failed: {e}"),
                &json!({"element": element, "frame": f}),
            )
        })?;
        if submit {
            submit_enter(browser).await?;
        }
        return Ok(ok_text(json!({"ok": true}).to_string()));
    }
    if let Some(id) = parse_ref(element) {
        let at_id = guard_ref(browser, "type", element, &id).await?;
        let expr = type_preflight_ref(&id);
        let v = browser.eval(&expr).await?;
        match v.as_str() {
            Some("GLADIUS_READY") => {}
            Some("GLADIUS_NOTFOUND") => return Err(stale_gone("type", element, &id)),
            Some("GLADIUS_DISABLED") => {
                return Err(ToolErr::typed(
                    format!("type: {at_id} is disabled"),
                    json!({"code": "not_actionable", "reason": "disabled", "tool": "type", "ref": at_id}),
                ))
            }
            Some("GLADIUS_READONLY") => {
                return Err(ToolErr::typed(
                    format!("type: {at_id} is read-only"),
                    json!({"code": "not_actionable", "reason": "readonly", "tool": "type", "ref": at_id}),
                ))
            }
            Some("GLADIUS_HIDDEN") => {
                return Err(ToolErr::typed(
                    format!("type: {at_id} not visible or not editable"),
                    json!({"code": "not_actionable", "reason": "hidden", "tool": "type", "ref": at_id}),
                ))
            }
            Some(s) if s.starts_with("GLADIUS_NOTEDIT:") => {
                let kind = &s["GLADIUS_NOTEDIT:".len()..];
                return Err(type_not_editable("type", kind, &json!({"ref": at_id})));
            }
            _ => return Err(stale_gone("type", element, &id)),
        }
        browser.insert_text(text).await.map_err(|e| {
            type_not_editable(
                "type",
                &format!("insertText failed: {e}"),
                &json!({"ref": at_id}),
            )
        })?;
        if submit {
            submit_enter(browser).await?;
        }
        return Ok(ok_text(json!({"ok": true}).to_string()));
    }
    let sel = serde_json::to_string(element).map_err(|e| e.to_string())?;
    let expr = type_preflight_sel(&sel);
    let v = browser.eval(&expr).await?;
    match v.as_str() {
        Some("GLADIUS_READY") => {}
        Some("GLADIUS_DISABLED") => {
            return Err(ToolErr::typed(
                format!("type: element is disabled: {element}"),
                json!({"code": "not_actionable", "reason": "disabled", "tool": "type", "element": element}),
            ));
        }
        Some("GLADIUS_READONLY") => {
            return Err(ToolErr::typed(
                format!("type: element is read-only: {element}"),
                json!({"code": "not_actionable", "reason": "readonly", "tool": "type", "element": element}),
            ));
        }
        Some("GLADIUS_HIDDEN") => {
            return Err(ToolErr::typed(
                format!("type: element not visible or not editable: {element}"),
                json!({"code": "not_actionable", "reason": "hidden", "tool": "type", "element": element}),
            ));
        }
        Some(s) if s.starts_with("GLADIUS_NOTEDIT:") => {
            let kind = &s["GLADIUS_NOTEDIT:".len()..];
            return Err(type_not_editable(
                "type",
                kind,
                &json!({"element": element}),
            ));
        }
        _ => {
            return Err(ToolErr::typed(
                format!("type: element not found: {element}"),
                json!({"code": "invalid_target", "reason": "not_found", "tool": "type", "element": element}),
            ))
        }
    }
    browser.insert_text(text).await.map_err(|e| {
        type_not_editable(
            "type",
            &format!("insertText failed: {e}"),
            &json!({"element": element}),
        )
    })?;
    if submit {
        submit_enter(browser).await?;
    }
    Ok(ok_text(json!({"ok": true}).to_string()))
}

async fn submit_enter(browser: &Browser) -> Result<(), String> {
    // Full keyDown (with text) so Enter triggers default actions like
    // form submission; rawKeyDown alone does not.
    browser
        .command(
            "Input.dispatchKeyEvent",
            json!({"type": "keyDown", "key": "Enter", "code": "Enter", "text": "\r",
                   "windowsVirtualKeyCode": 13, "nativeVirtualKeyCode": 13}),
            true,
        )
        .await?;
    browser
        .command(
            "Input.dispatchKeyEvent",
            json!({"type": "keyUp", "key": "Enter", "code": "Enter",
                   "windowsVirtualKeyCode": 13, "nativeVirtualKeyCode": 13}),
            true,
        )
        .await?;
    Ok(())
}

async fn tool_evaluate(
    browser: &Browser,
    expression: &str,
    target: Option<&str>,
    frame: Option<&str>,
) -> Result<Value, ToolErr> {
    if target.is_some() && frame.is_some() {
        return Err(ToolErr::typed(
            "evaluate: 'target' and 'frame' are mutually exclusive (one realm per call)"
                .to_string(),
            json!({"code": "invalid_args", "reason": "bad_value", "tool": "evaluate"}),
        ));
    }
    browser.take_dialog().await;
    let v = match (target, frame) {
        (Some(tid), None) => browser.eval_in_target(tid, expression).await?,
        (None, Some(fid)) => browser.eval_in_frame(fid, expression).await?,
        (None, None) => browser.eval(expression).await?,
        _ => unreachable!(),
    };
    let mut obj = serde_json::Map::new();
    obj.insert("value".to_string(), v);
    // An evaluate that opened a dialog (e.g. `alert(1)`) was auto-accepted
    // by the reader; report it instead of returning a bare value.
    if let Some(d) = browser.last_dialog().await {
        obj.insert(
            "dialog".to_string(),
            json!({"type": d.dtype, "message": d.message, "url": d.url}),
        );
        let verb = if d.accepted { "accepted" } else { "dismissed" };
        obj.insert(
            "note".to_string(),
            json!(format!(
                "dialog auto-handled: {} {:?} ({verb}, page unblocked)",
                d.dtype, d.message
            )),
        );
    }
    Ok(ok_text(Value::Object(obj).to_string()))
}

async fn tool_console(
    browser: &Browser,
    action: &str,
    limit: Option<i64>,
) -> Result<Value, ToolErr> {
    // Cap the RETURNED slice; the ring itself is CONSOLE_CAP (browser.rs).
    let lim = limit.unwrap_or(100).clamp(1, 200) as usize;
    match action {
        "get" => Ok(ok_text(browser.console_get(lim).await.to_string())),
        "drain" => {
            // Atomic take-and-return (single lock inside `console_drain`):
            // arrivals during a drain are either returned or kept, never lost.
            Ok(ok_text(browser.console_drain(lim).await.to_string()))
        }
        "clear" => {
            let n = browser.console_clear().await;
            Ok(ok_text(
                json!({"cleared": n, "cap": crate::browser::CONSOLE_CAP}).to_string(),
            ))
        }
        other => Err(ToolErr::typed(
            format!("console: unknown action '{other}' (get|drain|clear)"),
            json!({"code": "invalid_args", "reason": "unknown_action", "tool": "console"}),
        )),
    }
}

async fn tool_upload(
    browser: &Browser,
    element: &str,
    paths: &[String],
    frame: Option<&str>,
) -> Result<Value, ToolErr> {
    if paths.is_empty() {
        return Err(ToolErr::typed(
            "upload: 'paths' is empty".to_string(),
            json!({"code": "invalid_args", "reason": "missing_arg", "tool": "upload"}),
        ));
    }
    for p in paths {
        if !std::path::Path::new(p).is_file() {
            return Err(ToolErr::typed(
                format!("upload: file not found: {p}"),
                json!({"code": "file_not_found", "reason": "missing_path", "tool": "upload", "path": p}),
            ));
        }
    }
    // AX refs resolve to their stamped selector so both paths share the
    // DOM lookup below; the guard gives the typed stale-ref error.
    // Frame policy first: refs cannot resolve safely in a frame realm.
    let fid = frame_id_ok("upload", frame)?;
    if let Some(ref f) = fid {
        if parse_ref(element).is_some() {
            return Err(ref_in_frame("upload", element, f));
        }
        let lit = serde_json::to_string(element).map_err(|e| e.to_string())?;
        let probe = format!(
            "(() => {{ const el = document.querySelector({lit}); if (!el) return 'GLADIUS_NOTFOUND'; if (el.disabled) return 'GLADIUS_DISABLED'; if (el.tagName !== 'INPUT' || (el.type || '').toLowerCase() !== 'file') return 'GLADIUS_NOTFILE'; return 'GLADIUS_OK'; }})()"
        );
        match frame_realm_eval(browser, "upload", f, &probe)
            .await?
            .as_str()
        {
            Some("GLADIUS_OK") => {}
            Some("GLADIUS_NOTFOUND") => {
                return Err(ToolErr::typed(
                    format!("upload: element not found: {element}"),
                    json!({"code": "invalid_target", "reason": "not_found", "tool": "upload", "element": element, "frame": f}),
                ));
            }
            Some("GLADIUS_DISABLED") => {
                return Err(ToolErr::typed(
                    format!("upload: element is disabled: {element}"),
                    json!({"code": "not_actionable", "reason": "disabled", "tool": "upload", "element": element, "frame": f}),
                ));
            }
            _ => {
                return Err(ToolErr::typed(
                    format!("upload: element is not a file input: {element}"),
                    json!({"code": "invalid_target", "reason": "not_file", "tool": "upload", "element": element, "frame": f}),
                ));
            }
        }
        // CDP file path scoped to the frame realm. When the frame is an OOPIF
        // with a mapped session, the file input resolves through that
        // session's own DOM document (it is unreachable from the page
        // session); otherwise the frame's own querySelector object resolves
        // to a page-session node (no file-chooser dialog). The probe above
        // already validated the element is a file input in THIS frame, so the
        // resolved node is exact - never a main-frame upload.
        if let Some((_, sid)) = browser.oop_session_for_frame(f).await {
            return match browser.upload_in_session(&sid, element, paths).await {
                Ok(n) => Ok(ok_text(json!({"ok": true, "files": n as i64}).to_string())),
                Err(e) if e.contains("element not found") => Err(ToolErr::typed(
                    format!("upload: element not found: {element}"),
                    json!({"code": "invalid_target", "reason": "not_found", "tool": "upload", "element": element, "frame": f}),
                )),
                Err(e) => Err(ToolErr::fallback(e)),
            };
        }
        let node = match browser.frame_query_node(f, element).await {
            Ok(n) => n,
            Err(e) => {
                let l = e.to_lowercase();
                if l.contains("no frame with id") {
                    return Err(frame_lookup_err("upload", f, e));
                }
                if l.contains("not reachable") {
                    // OOPIF probe for a sharper typed error (mapped target vs
                    // unmapped), but typed either way - never a main-frame
                    // upload.
                    let mapped = browser.oop_target_for_frame(f).await;
                    let prose = match mapped {
                        Some(tid) => format!("upload: frame '{f}' is out-of-process (target {tid}); frame upload needs a mapped same-process document (no upload dispatched)"),
                        None => format!("upload: frame '{f}' has no accessible document (out-of-process-unmapped or detached; no upload dispatched)"),
                    };
                    return Err(ToolErr::typed(
                        prose,
                        json!({"code": "invalid_target", "reason": "not_found", "tool": "upload", "element": element, "frame": f}),
                    ));
                }
                if l.contains("not found in frame") {
                    return Err(ToolErr::typed(
                        format!("upload: element not found: {element}"),
                        json!({"code": "invalid_target", "reason": "not_found", "tool": "upload", "element": element, "frame": f}),
                    ));
                }
                return Err(frame_lookup_err("upload", f, e));
            }
        };
        if node == 0 {
            return Err(ToolErr::typed(
                format!("upload: element not found: {element}"),
                json!({"code": "invalid_target", "reason": "not_found", "tool": "upload", "element": element, "frame": f}),
            ));
        }
        browser
            .command(
                "DOM.setFileInputFiles",
                json!({"nodeId": node, "files": paths}),
                true,
            )
            .await
            .map_err(|e| format!("upload: {e}"))?;
        return Ok(ok_text(
            json!({"ok": true, "files": paths.len()}).to_string(),
        ));
    }
    let sel_owned;
    let (sel, rid): (&str, Option<String>) = match parse_ref(element) {
        Some(id) => {
            guard_ref(browser, "upload", element, &id).await?;
            sel_owned = format!("[data-gladius-ref=\"{id}\"]");
            (&sel_owned, Some(id))
        }
        None => (element, None),
    };
    let lit = serde_json::to_string(sel).map_err(|e| e.to_string())?;
    let probe = format!(
        "(() => {{ const el = document.querySelector({lit}); if (!el) return 'GLADIUS_NOTFOUND'; if (el.disabled) return 'GLADIUS_DISABLED'; if (el.tagName !== 'INPUT' || (el.type || '').toLowerCase() !== 'file') return 'GLADIUS_NOTFILE'; return 'GLADIUS_OK'; }})()"
    );
    match browser.eval(&probe).await?.as_str() {
        Some("GLADIUS_OK") => {}
        Some("GLADIUS_NOTFOUND") => match rid {
            Some(id) => return Err(stale_gone("upload", element, &id)),
            None => {
                return Err(ToolErr::typed(
                    format!("upload: element not found: {element}"),
                    json!({"code": "invalid_target", "reason": "not_found", "tool": "upload", "element": element}),
                ))
            }
        },
        Some("GLADIUS_DISABLED") => {
            return Err(match &rid {
                Some(id) => ToolErr::typed(
                    format!("upload: element is disabled: {element}"),
                    json!({"code": "not_actionable", "reason": "disabled", "tool": "upload", "ref": format!("@{id}")}),
                ),
                None => ToolErr::typed(
                    format!("upload: element is disabled: {element}"),
                    json!({"code": "not_actionable", "reason": "disabled", "tool": "upload", "element": element}),
                ),
            });
        }
        _ => {
            return Err(match &rid {
                Some(id) => ToolErr::typed(
                    format!("upload: element is not a file input: {element}"),
                    json!({"code": "invalid_target", "reason": "not_file", "tool": "upload", "ref": format!("@{id}")}),
                ),
                None => ToolErr::typed(
                    format!("upload: element is not a file input: {element}"),
                    json!({"code": "invalid_target", "reason": "not_file", "tool": "upload", "element": element}),
                ),
            });
        }
    }
    // CDP file path: document node -> querySelector -> setFileInputFiles.
    // Works headless (no file-chooser dialog involved).
    browser.command("DOM.enable", json!({}), true).await?;
    let doc = browser
        .command("DOM.getDocument", json!({"depth": 0}), true)
        .await?;
    let root = doc
        .get("root")
        .and_then(|r| r.get("nodeId"))
        .and_then(|n| n.as_i64())
        .ok_or("upload: no document node".to_string())?;
    let found = browser
        .command(
            "DOM.querySelector",
            json!({"nodeId": root, "selector": sel}),
            true,
        )
        .await?;
    let node = found.get("nodeId").and_then(|n| n.as_i64()).unwrap_or(0);
    if node == 0 {
        return match rid {
            Some(id) => Err(stale_gone("upload", element, &id)),
            None => Err(ToolErr::typed(
                format!("upload: element not found: {element}"),
                json!({"code": "invalid_target", "reason": "not_found", "tool": "upload", "element": element}),
            )),
        };
    }
    browser
        .command(
            "DOM.setFileInputFiles",
            json!({"nodeId": node, "files": paths}),
            true,
        )
        .await
        .map_err(|e| format!("upload: {e}"))?;
    Ok(ok_text(
        json!({"ok": true, "files": paths.len()}).to_string(),
    ))
}

async fn tool_wait_for(
    browser: &Browser,
    selector: Option<&str>,
    text: Option<&str>,
    ms: Option<i64>,
    network_idle: bool,
    idle_ms: Option<i64>,
    expression: Option<&str>,
) -> Result<Value, ToolErr> {
    let budget_ms: i64 = ms.unwrap_or(5000).clamp(0, 15000);
    let budget = std::time::Duration::from_millis(budget_ms.max(0) as u64);
    // One condition per call: `expression` never combines with
    // `selector`/`text`. Validated before any waiting so a conflicting call
    // fails fast instead of burning the budget on the wrong leg.
    if expression.is_some() && (selector.is_some() || text.is_some()) {
        return Err(ToolErr::typed(
            "wait_for: 'expression' is mutually exclusive with 'selector'/'text' (one condition per call)".to_string(),
            json!({"code": "invalid_args", "reason": "bad_value", "tool": "wait_for"}),
        ));
    }
    // ms:0 is a no-op: return immediately without any CDP round-trip. The
    // network-idle path keeps its own zero-budget behavior below, where the
    // lazy enable is inside the deadline like every other await.
    if budget_ms == 0 && !network_idle {
        return Ok(ok_text(json!({"ok": true}).to_string()));
    }
    // HARD deadline shared by both phases below: checked before AND after
    // every await, so the total can never exceed `ms` no matter how slow
    // one step is. Phase 1 (selector/text) eats into phase 2's
    // (network-idle) budget by construction.
    let deadline = std::time::Instant::now() + budget;
    // Session-continuity pin: the wait belongs to the page it started on.
    // A vanished/changed session mid-wait is a dead page, never a reason
    // to migrate the wait onto a fresh page and burn the budget there.
    let mut pinned: Option<String> = None;
    if selector.is_some() || text.is_some() {
        let sel_expr = selector.map(|s| {
            let lit = serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string());
            // Visibility, not mere existence: a display:none node must NOT
            // satisfy the wait (same shape the click/hover guards use).
            format!("(() => {{ const el = document.querySelector({lit}); if (!el) return false; const cs = getComputedStyle(el); if (cs.display === 'none' || cs.visibility === 'hidden' || cs.visibility === 'collapse') return false; const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0; }})()")
        });
        let text_lit =
            text.map(|t| serde_json::to_string(t).unwrap_or_else(|_| "\"\"".to_string()));
        // Each per-await CDP evaluation is clipped to the remaining budget.
        loop {
            let mut done = true;
            if let Some(ref e) = sel_expr {
                if std::time::Instant::now() >= deadline {
                    return Err(wait_timeout());
                }
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                match tokio::time::timeout(remaining, browser.eval(e)).await {
                    Ok(Ok(v)) => {
                        if let Some(msg) = browser.check_session_pin(&mut pinned).await {
                            return Err(ToolErr::from(msg));
                        }
                        if v != Value::Bool(true) {
                            done = false;
                        }
                    }
                    Ok(Err(e)) => {
                        if let Some(se) = stale_session_err(&e) {
                            return Err(se);
                        }
                        done = false;
                    }
                    _ => done = false,
                }
                if std::time::Instant::now() >= deadline {
                    return Err(wait_timeout());
                }
            }
            if done {
                if let Some(ref lit) = text_lit {
                    if std::time::Instant::now() >= deadline {
                        return Err(wait_timeout());
                    }
                    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                    let e = format!(
                        "((document.body && document.body.innerText) || '').includes({lit})"
                    );
                    match tokio::time::timeout(remaining, browser.eval(&e)).await {
                        Ok(Ok(v)) => {
                            if let Some(msg) = browser.check_session_pin(&mut pinned).await {
                                return Err(ToolErr::from(msg));
                            }
                            if v != Value::Bool(true) {
                                done = false;
                            }
                        }
                        Ok(Err(e)) => {
                            if let Some(se) = stale_session_err(&e) {
                                return Err(se);
                            }
                            done = false;
                        }
                        _ => done = false,
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err(wait_timeout());
                    }
                }
            }
            if done {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err(wait_timeout());
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            tokio::time::sleep(remaining.min(std::time::Duration::from_millis(100))).await;
        }
    } else if !network_idle && expression.is_none() {
        // Legacy bare delay: no condition, just sleep the budget.
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        tokio::time::sleep(remaining).await;
        return Ok(ok_text(json!({"ok": true}).to_string()));
    }
    // Predicate form: poll a computed JS condition on the active page with a
    // backoff cadence inside the same hard deadline. Expiry is the typed
    // timeout; a throwing condition is a real error (not "not yet true").
    if let Some(expr) = expression {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match browser.wait_for_expression(expr, remaining).await {
            Ok(true) => {}
            Ok(false) => return Err(wait_timeout()),
            Err(e) => return Err(stale_session_err(&e).unwrap_or_else(|| ToolErr::fallback(e))),
        }
    }
    if network_idle {
        // Quiet-network window on the REMAINING budget (zero when phase 1
        // ate it all): drains in-flight requests, then requires `idle`
        // continuous quiet. Never hangs - expiry is the typed timeout.
        let idle_win =
            std::time::Duration::from_millis(idle_ms.unwrap_or(500).clamp(0, 5000).max(0) as u64);
        // The lazy `Network.enable` enable is INSIDE the
        // deadline as well. Its CDP call allows `send_raw`'s 30s
        // per-call timeout - double the largest budget this tool accepts
        // (15s) - so with an established session and a peer holding the
        // `Network.enable` response the unwrapped await answered a 100ms
        // wait ~300x late (as a raw CDP error) instead of the typed timeout.
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match tokio::time::timeout(remaining, browser.ensure_network()).await {
            Ok(Ok(())) => {
                if let Some(msg) = browser.check_session_pin(&mut pinned).await {
                    return Err(ToolErr::from(msg));
                }
            }
            // Budget spent while the enable was still outstanding: no quiet
            // window can be observed inside `ms`, so the budget IS the answer.
            Err(_) => return Err(wait_timeout()),
            // A real CDP failure keeps its own message (the pre-fix `?`).
            Ok(Err(e)) => return Err(e.into()),
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match browser
            .wait_network_idle(idle_win, remaining, &mut pinned)
            .await
        {
            Ok(true) => {}
            Ok(false) => return Err(wait_timeout()),
            Err(e) => return Err(stale_session_err(&e).unwrap_or_else(|| ToolErr::fallback(e))),
        }
        if let Some(msg) = browser.check_session_pin(&mut pinned).await {
            return Err(ToolErr::from(msg));
        }
    }
    Ok(ok_text(json!({"ok": true}).to_string()))
}

/// Budget-exhaustion failure for `wait_for`: the site knows the budget ran
/// out, so it mints the typed timeout (never a generic error).
fn wait_timeout() -> ToolErr {
    ToolErr::typed(
        "wait_for: timeout".to_string(),
        json!({"code": "timeout", "reason": "budget_exceeded", "tool": "wait_for"}),
    )
}

/// Session-death fast path for polling waits: an engine error minted by
/// `send_session` (`stale session: ...` — the session-death class, surfaced
/// either from a failed send or from a failed re-attach inside it) means
/// the page is gone. Return the typed `invalid_target`/`stale_session` at
/// once instead of polling the corpse until the budget burns. Anything
/// else keeps the existing handling (a false condition keeps polling; a
/// throwing expression is its own real error).
fn stale_session_err(e: &str) -> Option<ToolErr> {
    e.strip_prefix("stale session: ")
        .map(|_| ToolErr::from(e.to_string()))
}

async fn tool_hover(
    browser: &Browser,
    element: &str,
    frame: Option<&str>,
) -> Result<Value, ToolErr> {
    // G4a: same cheap hit-test/occlusion guard as click (elementFromPoint at
    // the hover point). An occluded hover target is a typed
    // not_actionable/occluded instead of a pointer move to the overlay. The
    // dispatch itself stays the trusted CDP mouseMoved it already was.
    // G4b: with frame= the guard runs in the frame realm and the leaf point
    // maps through the owner rect chain; dispatch stays on the page session.
    let fid = frame_id_ok("hover", frame)?;
    if let Some(ref f) = fid {
        if parse_ref(element).is_some() {
            return Err(ref_in_frame("hover", element, f));
        }
        let sel = serde_json::to_string(element).map_err(|e| e.to_string())?;
        let expr = format!(
            "(() => {{ const el = document.querySelector({sel}); if (!el) return 'GLADIUS_NOTFOUND'; const cs = getComputedStyle(el); if (cs.display === 'none' || cs.visibility === 'hidden' || cs.visibility === 'collapse') return 'GLADIUS_HIDDEN'; try {{ el.scrollIntoView({{block: 'center', inline: 'center'}}); }} catch (e) {{}} const r = el.getBoundingClientRect(); if (!r || r.width === 0 || r.height === 0) return 'GLADIUS_HIDDEN'; const cx = r.left + r.width / 2, cy = r.top + r.height / 2; let hit = null; try {{ hit = document.elementFromPoint(cx, cy); }} catch (e) {{ return 'GLADIUS_OCCLUDED'; }} if (!hit || (hit !== el && !el.contains(hit))) return 'GLADIUS_OCCLUDED'; return JSON.stringify({{x: cx, y: cy}}); }})()"
        );
        let v = frame_realm_eval(browser, "hover", f, &expr).await?;
        if v.as_str() == Some("GLADIUS_NOTFOUND") {
            return Err(ToolErr::typed(
                format!("hover: element not found: {element}"),
                json!({"code": "invalid_target", "reason": "not_found", "tool": "hover", "element": element, "frame": f}),
            ));
        }
        if v.as_str() == Some("GLADIUS_HIDDEN") {
            return Err(ToolErr::typed(
                format!("hover: element not visible or not hoverable: {element}"),
                json!({"code": "not_actionable", "reason": "hidden", "tool": "hover", "element": element, "frame": f}),
            ));
        }
        if v.as_str() == Some("GLADIUS_OCCLUDED") {
            return Err(ToolErr::typed(
                format!("hover: element is occluded by another element (no hover dispatched): {element}"),
                json!({"code": "not_actionable", "reason": "occluded", "tool": "hover", "element": element, "frame": f}),
            ));
        }
        let pt: Value = serde_json::from_str(v.as_str().unwrap_or(""))
            .map_err(|_| "hover: no layout box".to_string())?;
        let lx = pt.get("x").and_then(|n| n.as_f64()).unwrap_or(0.0);
        let ly = pt.get("y").and_then(|n| n.as_f64()).unwrap_or(0.0);
        // Same per-level parent-side owner check as click: no hover into a
        // surface that is not the requested frame.
        let (mx, my) = frame_owner_hit_ok(browser, "hover", element, f, lx, ly).await?;
        browser.mouse_move(mx, my).await?;
        return Ok(ok_text(json!({"ok": true}).to_string()));
    }
    if let Some(id) = parse_ref(element) {
        let at_id = guard_ref(browser, "hover", element, &id).await?;
        let rid = serde_json::to_string(&id).map_err(|e| e.to_string())?;
        let expr = format!(
            "((ref) => {{ const el = document.querySelector('[data-gladius-ref=\"' + ref + '\"]'); if (!el) return 'GLADIUS_NOTFOUND'; const cs = getComputedStyle(el); if (cs.display === 'none' || cs.visibility === 'hidden' || cs.visibility === 'collapse') return 'GLADIUS_HIDDEN'; try {{ el.scrollIntoView({{block: 'center', inline: 'center'}}); }} catch (e) {{}} const r = el.getBoundingClientRect(); if (!r || r.width === 0 || r.height === 0) return 'GLADIUS_HIDDEN'; const cx = r.left + r.width / 2, cy = r.top + r.height / 2; let hit = null; try {{ hit = document.elementFromPoint(cx, cy); }} catch (e) {{ return 'GLADIUS_OCCLUDED'; }} if (!hit || (hit !== el && !el.contains(hit))) return 'GLADIUS_OCCLUDED'; return JSON.stringify({{x: cx, y: cy}}); }})({rid})"
        );
        let v = browser.eval(&expr).await?;
        if v.as_str() == Some("GLADIUS_NOTFOUND") {
            return Err(stale_gone("hover", element, &id));
        }
        if v.as_str() == Some("GLADIUS_HIDDEN") {
            return Err(ToolErr::typed(
                format!("hover: {at_id} not visible or not hoverable"),
                json!({"code": "not_actionable", "reason": "hidden", "tool": "hover", "ref": at_id}),
            ));
        }
        if v.as_str() == Some("GLADIUS_OCCLUDED") {
            return Err(ToolErr::typed(
                format!("hover: {at_id} is occluded by another element (no hover dispatched)"),
                json!({"code": "not_actionable", "reason": "occluded", "tool": "hover", "ref": at_id}),
            ));
        }
        return hover_move(browser, &v).await;
    }
    let sel = serde_json::to_string(element).map_err(|e| e.to_string())?;
    let expr = format!(
        "(() => {{ const el = document.querySelector({sel}); if (!el) return 'GLADIUS_NOTFOUND'; const cs = getComputedStyle(el); if (cs.display === 'none' || cs.visibility === 'hidden' || cs.visibility === 'collapse') return 'GLADIUS_HIDDEN'; try {{ el.scrollIntoView({{block: 'center', inline: 'center'}}); }} catch (e) {{}} const r = el.getBoundingClientRect(); if (!r || r.width === 0 || r.height === 0) return 'GLADIUS_HIDDEN'; const cx = r.left + r.width / 2, cy = r.top + r.height / 2; let hit = null; try {{ hit = document.elementFromPoint(cx, cy); }} catch (e) {{ return 'GLADIUS_OCCLUDED'; }} if (!hit || (hit !== el && !el.contains(hit))) return 'GLADIUS_OCCLUDED'; return JSON.stringify({{x: cx, y: cy}}); }})()"
    );
    let v = browser.eval(&expr).await?;
    if v.as_str() == Some("GLADIUS_NOTFOUND") {
        return Err(ToolErr::typed(
            format!("hover: element not found: {element}"),
            json!({"code": "invalid_target", "reason": "not_found", "tool": "hover", "element": element}),
        ));
    }
    if v.as_str() == Some("GLADIUS_HIDDEN") {
        return Err(ToolErr::typed(
            format!("hover: element not visible or not hoverable: {element}"),
            json!({"code": "not_actionable", "reason": "hidden", "tool": "hover", "element": element}),
        ));
    }
    if v.as_str() == Some("GLADIUS_OCCLUDED") {
        return Err(ToolErr::typed(
            format!(
                "hover: element is occluded by another element (no hover dispatched): {element}"
            ),
            json!({"code": "not_actionable", "reason": "occluded", "tool": "hover", "element": element}),
        ));
    }
    hover_move(browser, &v).await
}

async fn hover_move(browser: &Browser, v: &Value) -> Result<Value, ToolErr> {
    let s = v.as_str().unwrap_or("").to_string();
    let pt: Value = serde_json::from_str(&s).map_err(|_| "hover: no layout box".to_string())?;
    let x = pt.get("x").and_then(|n| n.as_f64()).unwrap_or(0.0);
    let y = pt.get("y").and_then(|n| n.as_f64()).unwrap_or(0.0);
    browser.mouse_move(x, y).await?;
    Ok(ok_text(json!({"ok": true}).to_string()))
}

async fn tool_select(
    browser: &Browser,
    element: &str,
    value: Option<&str>,
    text: Option<&str>,
    frame: Option<&str>,
) -> Result<Value, ToolErr> {
    if value.is_none() && text.is_none() {
        return Err(ToolErr::typed(
            "select: need 'value' or 'text'".to_string(),
            json!({"code": "invalid_args", "reason": "need_option", "tool": "select"}),
        ));
    }
    // G4b: with frame= the same JS selection runs in the frame realm via
    // eval_in_frame (no coordinates involved). Refs are main-frame only.
    let fid = frame_id_ok("select", frame)?;
    if let Some(ref f) = fid {
        if parse_ref(element).is_some() {
            return Err(ref_in_frame("select", element, f));
        }
        let sel = serde_json::to_string(element).map_err(|e| e.to_string())?;
        // `value` wins when both are given.
        let val_lit = match value {
            Some(s) => serde_json::to_string(s).map_err(|e| e.to_string())?,
            None => "null".to_string(),
        };
        let txt_lit = match text {
            Some(s) => serde_json::to_string(s).map_err(|e| e.to_string())?,
            None => "null".to_string(),
        };
        let expr = format!(
            "(() => {{ const el = document.querySelector({sel}); if (!el) return 'GLADIUS_NOTFOUND'; if (!el || el.tagName !== 'SELECT') return 'GLADIUS_NOTSELECT'; const wantValue = {val_lit}; const wantText = {txt_lit}; const opts = Array.from(el.options); let opt = null; if (wantValue !== null) {{ opt = opts.find(o => o.value === wantValue); }} else {{ opt = opts.find(o => o.text === wantText || o.textContent === wantText); }} if (!opt) return 'GLADIUS_NOOPT'; el.value = opt.value; el.selectedIndex = opts.indexOf(opt); el.dispatchEvent(new Event('input', {{bubbles: true}})); el.dispatchEvent(new Event('change', {{bubbles: true}})); return 'GLADIUS_OK'; }})()"
        );
        let v = frame_realm_eval(browser, "select", f, &expr).await?;
        return match v.as_str() {
            Some("GLADIUS_OK") => Ok(ok_text(json!({"ok": true}).to_string())),
            Some("GLADIUS_NOTFOUND") => Err(ToolErr::typed(
                format!("select: element not found: {element}"),
                json!({"code": "invalid_target", "reason": "not_found", "tool": "select", "element": element, "frame": f}),
            )),
            Some("GLADIUS_NOTSELECT") => Err(ToolErr::typed(
                format!("select: element is not a <select>: {element}"),
                json!({"code": "invalid_target", "reason": "not_select", "tool": "select", "element": element, "frame": f}),
            )),
            _ => {
                let want = value.or(text).unwrap_or("");
                Err(ToolErr::typed(
                    format!("select: option not found: {want}"),
                    json!({"code": "option_not_found", "reason": "no_match", "tool": "select", "want": want, "frame": f}),
                ))
            }
        };
    }
    if let Some(id) = parse_ref(element) {
        guard_ref(browser, "select", element, &id).await?;
        let rid = serde_json::to_string(&id).map_err(|e| e.to_string())?;
        let val_lit = match value {
            Some(s) => serde_json::to_string(s).map_err(|e| e.to_string())?,
            None => "null".to_string(),
        };
        let txt_lit = match text {
            Some(s) => serde_json::to_string(s).map_err(|e| e.to_string())?,
            None => "null".to_string(),
        };
        let expr = format!(
            "((ref) => {{ const el = document.querySelector('[data-gladius-ref=\"' + ref + '\"]'); if (!el) return 'GLADIUS_NOTFOUND'; if (!el || el.tagName !== 'SELECT') return 'GLADIUS_NOTSELECT'; const wantValue = {val_lit}; const wantText = {txt_lit}; const opts = Array.from(el.options); let opt = null; if (wantValue !== null) {{ opt = opts.find(o => o.value === wantValue); }} else {{ opt = opts.find(o => o.text === wantText || o.textContent === wantText); }} if (!opt) return 'GLADIUS_NOOPT'; el.value = opt.value; el.selectedIndex = opts.indexOf(opt); el.dispatchEvent(new Event('input', {{bubbles: true}})); el.dispatchEvent(new Event('change', {{bubbles: true}})); return 'GLADIUS_OK'; }})({rid})"
        );
        let v = browser.eval(&expr).await?;
        return match v.as_str() {
            Some("GLADIUS_OK") => Ok(ok_text(json!({"ok": true}).to_string())),
            Some("GLADIUS_NOTFOUND") => Err(stale_gone("select", element, &id)),
            Some("GLADIUS_NOTSELECT") => Err(ToolErr::typed(
                format!("select: @{id} is not a <select>"),
                json!({"code": "invalid_target", "reason": "not_select", "tool": "select", "ref": format!("@{id}")}),
            )),
            _ => {
                let want = value.or(text).unwrap_or("");
                Err(ToolErr::typed(
                    format!("select: option not found: {want}"),
                    json!({"code": "option_not_found", "reason": "no_match", "tool": "select", "want": want}),
                ))
            }
        };
    }
    let sel = serde_json::to_string(element).map_err(|e| e.to_string())?;
    // `value` wins when both are given.
    let val_lit = match value {
        Some(s) => serde_json::to_string(s).map_err(|e| e.to_string())?,
        None => "null".to_string(),
    };
    let txt_lit = match text {
        Some(s) => serde_json::to_string(s).map_err(|e| e.to_string())?,
        None => "null".to_string(),
    };
    let expr = format!(
        "(() => {{ const el = document.querySelector({sel}); if (!el) return 'GLADIUS_NOTFOUND'; if (!el || el.tagName !== 'SELECT') return 'GLADIUS_NOTSELECT'; const wantValue = {val_lit}; const wantText = {txt_lit}; const opts = Array.from(el.options); let opt = null; if (wantValue !== null) {{ opt = opts.find(o => o.value === wantValue); }} else {{ opt = opts.find(o => o.text === wantText || o.textContent === wantText); }} if (!opt) return 'GLADIUS_NOOPT'; el.value = opt.value; el.selectedIndex = opts.indexOf(opt); el.dispatchEvent(new Event('input', {{bubbles: true}})); el.dispatchEvent(new Event('change', {{bubbles: true}})); return 'GLADIUS_OK'; }})()"
    );
    let v = browser.eval(&expr).await?;
    match v.as_str() {
        Some("GLADIUS_OK") => Ok(ok_text(json!({"ok": true}).to_string())),
        Some("GLADIUS_NOTFOUND") => Err(ToolErr::typed(
            format!("select: element not found: {element}"),
            json!({"code": "invalid_target", "reason": "not_found", "tool": "select", "element": element}),
        )),
        Some("GLADIUS_NOTSELECT") => Err(ToolErr::typed(
            format!("select: element is not a <select>: {element}"),
            json!({"code": "invalid_target", "reason": "not_select", "tool": "select", "element": element}),
        )),
        _ => {
            let want = value.or(text).unwrap_or("");
            Err(ToolErr::typed(
                format!("select: option not found: {want}"),
                json!({"code": "option_not_found", "reason": "no_match", "tool": "select", "want": want}),
            ))
        }
    }
}

/// Map a human key name to (key, code, vk, text-for-keyDown). Unknown
/// multi-char names are None: the caller fails loud instead of dispatching
/// a silent no-op.
fn key_map(key: &str) -> Option<(String, String, u32, Option<String>)> {
    let lower = key.to_lowercase();
    Some(match lower.as_str() {
        "enter" => ("Enter".into(), "Enter".into(), 13, Some("\r".into())),
        "tab" => ("Tab".into(), "Tab".into(), 9, None),
        "escape" | "esc" => ("Escape".into(), "Escape".into(), 27, None),
        "backspace" => ("Backspace".into(), "Backspace".into(), 8, None),
        "delete" | "del" => ("Delete".into(), "Delete".into(), 46, None),
        "arrowup" | "up" => ("ArrowUp".into(), "ArrowUp".into(), 38, None),
        "arrowdown" | "down" => ("ArrowDown".into(), "ArrowDown".into(), 40, None),
        "arrowleft" | "left" => ("ArrowLeft".into(), "ArrowLeft".into(), 37, None),
        "arrowright" | "right" => ("ArrowRight".into(), "ArrowRight".into(), 39, None),
        "home" => ("Home".into(), "Home".into(), 36, None),
        "end" => ("End".into(), "End".into(), 35, None),
        "pageup" => ("PageUp".into(), "PageUp".into(), 33, None),
        "pagedown" => ("PageDown".into(), "PageDown".into(), 34, None),
        "space" | "spacebar" => (" ".into(), "Space".into(), 32, Some(" ".into())),
        "shift" => ("Shift".into(), "ShiftLeft".into(), 16, None),
        "control" | "ctrl" => ("Control".into(), "ControlLeft".into(), 17, None),
        "alt" => ("Alt".into(), "AltLeft".into(), 18, None),
        // G4b: the fourth modifier (mask bit Meta=4). Aliases cover the common
        // spellings agents reach for; vk 91 is VK_LWIN.
        "meta" | "cmd" | "command" => ("Meta".into(), "MetaLeft".into(), 91, None),
        _ => {
            // F1..F12.
            if lower.len() > 1
                && lower.len() <= 3
                && lower.as_bytes()[0] == b'f'
                && lower[1..].chars().all(|c| c.is_ascii_digit())
            {
                if let Ok(n) = lower[1..].parse::<u32>() {
                    if (1..=12).contains(&n) {
                        let name = format!("F{n}");
                        return Some((name.clone(), name, 111 + n, None));
                    }
                }
            }
            // Single printable char: real text entry path.
            if key.chars().count() == 1 {
                let c = key.chars().next().unwrap();
                let up = c.to_ascii_uppercase();
                let code = if c.is_ascii_alphabetic() {
                    format!("Key{up}")
                } else if c.is_ascii_digit() {
                    format!("Digit{c}")
                } else if c == ' ' {
                    "Space".to_string()
                } else {
                    String::new()
                };
                return Some((c.to_string(), code, up as u32, Some(c.to_string())));
            }
            // Unknown multi-char name: reject (fail loud, never a silent no-op).
            return None;
        }
    })
}

async fn tool_key(browser: &Browser, key: &str) -> Result<Value, ToolErr> {
    if key.is_empty() {
        return Err(ToolErr::typed(
            "key: missing required arg 'key'".to_string(),
            json!({"code": "invalid_args", "reason": "missing_arg", "tool": "key"}),
        ));
    }
    // G4b key combos: `Ctrl+A`, `Shift+Tab`, `Alt+...`, `Meta+...`
    // (case-insensitive modifiers, '+'-joined, last token is the base key
    // through the existing key_map). A key WITHOUT '+' keeps the exact
    // single-key behavior below (no modifiers field, identical params).
    if key.contains('+') {
        return tool_key_combo(browser, key).await;
    }
    let Some((k, code, vk, text)) = key_map(key) else {
        return Err(ToolErr::typed(
            format!("key: unknown key '{key}' (not a recognized key name or single character)"),
            json!({"code": "invalid_args", "reason": "bad_value", "tool": "key", "arg": "key", "want": key}),
        ));
    };
    let mut down = serde_json::Map::new();
    down.insert("type".to_string(), json!("keyDown"));
    down.insert("key".to_string(), json!(k));
    if !code.is_empty() {
        down.insert("code".to_string(), json!(code));
    }
    if vk != 0 {
        down.insert("windowsVirtualKeyCode".to_string(), json!(vk));
        down.insert("nativeVirtualKeyCode".to_string(), json!(vk));
    }
    if let Some(t) = text {
        down.insert("text".to_string(), json!(t));
    }
    browser
        .command("Input.dispatchKeyEvent", Value::Object(down), true)
        .await?;
    let mut up = serde_json::Map::new();
    up.insert("type".to_string(), json!("keyUp"));
    up.insert("key".to_string(), json!(k));
    if !code.is_empty() {
        up.insert("code".to_string(), json!(code));
    }
    if vk != 0 {
        up.insert("windowsVirtualKeyCode".to_string(), json!(vk));
        up.insert("nativeVirtualKeyCode".to_string(), json!(vk));
    }
    browser
        .command("Input.dispatchKeyEvent", Value::Object(up), true)
        .await?;
    Ok(ok_text(json!({"ok": true}).to_string()))
}

/// G4b key-combo dispatch: `Ctrl+A`, `Shift+Tab`, `Alt+...`, `Meta+...`.
///
/// Parse: split on '+', trim; every token but the last must be a modifier
/// (case-insensitive: ctrl/control, shift, alt, meta/cmd/command); the last
/// token is the base key through the existing `key_map` (so `F5`, `Delete`,
/// single chars all keep working as combo bases). Empty tokens (a bare `+`
/// base excepted: a trailing '+' means the base key IS '+') and unknown
/// names are typed `invalid_args`/`bad_value`, never a silent no-op.
///
/// Modifier mask: Alt=1, Ctrl=2, Meta=4, Shift=8. Every event carries the full
/// mask. Order: modifier keyDowns in written order, base keyDown + keyUp,
/// modifier keyUps in reverse. When Ctrl/Meta/Alt is held the base dispatches
/// as `type:"rawKeyDown"` with NO `text` (Ctrl+A must select-all, never insert
/// "a"); Shift-only combos keep the normal `keyDown` with text. Shift+Tab is
/// `{key:"Tab",code:"Tab",vk:9,modifiers:8}` with no text per the same rule
/// (Tab has no text in the map).
async fn tool_key_combo(browser: &Browser, key: &str) -> Result<Value, ToolErr> {
    fn unknown(k: &str) -> ToolErr {
        ToolErr::typed(
            format!(
                "key: unknown key '{k}' (not a recognized key name, modifier, or single character)"
            ),
            json!({"code": "invalid_args", "reason": "bad_value", "tool": "key", "arg": "key", "want": k}),
        )
    }
    // Split, keeping a trailing '+' as the base key `+` (e.g. `Ctrl++`).
    let raw_parts: Vec<String> = key.split('+').map(|s| s.trim().to_string()).collect();
    let (mod_parts, base_str): (Vec<String>, String) =
        if raw_parts.last().map(|s| s.is_empty()).unwrap_or(false) {
            let mods: Vec<String> = raw_parts[..raw_parts.len() - 1]
                .iter()
                .filter(|s| !s.is_empty())
                .cloned()
                .collect();
            // A bare `+` (or `++`) has no modifiers: single-key `+` dispatch
            // with the historical params (no `modifiers` field).
            if mods.is_empty() {
                let Some(mapped) = key_map("+") else {
                    return Err(unknown(key));
                };
                return tool_key_single(browser, mapped).await;
            }
            (mods, "+".to_string())
        } else {
            let b = raw_parts.last().cloned().unwrap_or_default();
            let m = raw_parts[..raw_parts.len().saturating_sub(1)].to_vec();
            (m, b)
        };
    // Any empty token in the middle (`Ctrl++A`) is a typo, not a key.
    if mod_parts.iter().any(|s| s.is_empty()) || base_str.is_empty() {
        return Err(unknown(key));
    }
    // No modifiers after normalization (e.g. base `+` handled above): refuse
    // rather than guess - the only no-modifier '+' shape already returned.
    if mod_parts.is_empty() {
        return Err(unknown(key));
    }
    // Modifier set (case-insensitive) with mask bits.
    let mut mods: Vec<(String, String, u32)> = Vec::new();
    let mut mask: u32 = 0;
    for m in &mod_parts {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => {
                mask |= 2;
                mods.push(("Control".to_string(), "ControlLeft".to_string(), 17));
            }
            "shift" => {
                mask |= 8;
                mods.push(("Shift".to_string(), "ShiftLeft".to_string(), 16));
            }
            "alt" => {
                mask |= 1;
                mods.push(("Alt".to_string(), "AltLeft".to_string(), 18));
            }
            "meta" | "cmd" | "command" => {
                mask |= 4;
                mods.push(("Meta".to_string(), "MetaLeft".to_string(), 91));
            }
            _ => return Err(unknown(key)),
        }
    }
    let Some((bk, bcode, bvk, btext)) = key_map(&base_str) else {
        return Err(unknown(key));
    };
    let raw_base = mask & 0x7 != 0; // Ctrl/Meta/Alt held: rawKeyDown, no text.
    let mut seq: Vec<Value> = Vec::new();
    for (k, code, vk) in &mods {
        let mut p = serde_json::Map::new();
        p.insert("type".to_string(), json!("keyDown"));
        p.insert("key".to_string(), json!(k));
        if !code.is_empty() {
            p.insert("code".to_string(), json!(code));
        }
        if *vk != 0 {
            p.insert("windowsVirtualKeyCode".to_string(), json!(vk));
            p.insert("nativeVirtualKeyCode".to_string(), json!(vk));
        }
        p.insert("modifiers".to_string(), json!(mask));
        seq.push(Value::Object(p));
    }
    {
        let mut p = serde_json::Map::new();
        p.insert(
            "type".to_string(),
            json!(if raw_base { "rawKeyDown" } else { "keyDown" }),
        );
        p.insert("key".to_string(), json!(bk));
        if !bcode.is_empty() {
            p.insert("code".to_string(), json!(bcode));
        }
        if bvk != 0 {
            p.insert("windowsVirtualKeyCode".to_string(), json!(bvk));
            p.insert("nativeVirtualKeyCode".to_string(), json!(bvk));
        }
        if !raw_base {
            if let Some(t) = btext {
                p.insert("text".to_string(), json!(t));
            }
        }
        p.insert("modifiers".to_string(), json!(mask));
        seq.push(Value::Object(p));
    }
    {
        let mut p = serde_json::Map::new();
        p.insert("type".to_string(), json!("keyUp"));
        p.insert("key".to_string(), json!(bk));
        if !bcode.is_empty() {
            p.insert("code".to_string(), json!(bcode));
        }
        if bvk != 0 {
            p.insert("windowsVirtualKeyCode".to_string(), json!(bvk));
            p.insert("nativeVirtualKeyCode".to_string(), json!(bvk));
        }
        p.insert("modifiers".to_string(), json!(mask));
        seq.push(Value::Object(p));
    }
    for (k, code, vk) in mods.iter().rev() {
        let mut p = serde_json::Map::new();
        p.insert("type".to_string(), json!("keyUp"));
        p.insert("key".to_string(), json!(k));
        if !code.is_empty() {
            p.insert("code".to_string(), json!(code));
        }
        if *vk != 0 {
            p.insert("windowsVirtualKeyCode".to_string(), json!(vk));
            p.insert("nativeVirtualKeyCode".to_string(), json!(vk));
        }
        p.insert("modifiers".to_string(), json!(mask));
        seq.push(Value::Object(p));
    }
    for params in seq {
        browser
            .command("Input.dispatchKeyEvent", params, true)
            .await
            .map_err(ToolErr::from)?;
    }
    Ok(ok_text(json!({"ok": true}).to_string()))
}

/// Single-key dispatch helper shared by the bare-`+` combo normalization.
/// Params are byte-identical to the historical single-key path (no
/// `modifiers` field).
async fn tool_key_single(
    browser: &Browser,
    mapped: (String, String, u32, Option<String>),
) -> Result<Value, ToolErr> {
    let (k, code, vk, text) = mapped;
    let mut down = serde_json::Map::new();
    down.insert("type".to_string(), json!("keyDown"));
    down.insert("key".to_string(), json!(k));
    if !code.is_empty() {
        down.insert("code".to_string(), json!(code));
    }
    if vk != 0 {
        down.insert("windowsVirtualKeyCode".to_string(), json!(vk));
        down.insert("nativeVirtualKeyCode".to_string(), json!(vk));
    }
    if let Some(t) = text {
        down.insert("text".to_string(), json!(t));
    }
    browser
        .command("Input.dispatchKeyEvent", Value::Object(down), true)
        .await?;
    let mut up = serde_json::Map::new();
    up.insert("type".to_string(), json!("keyUp"));
    up.insert("key".to_string(), json!(k));
    if !code.is_empty() {
        up.insert("code".to_string(), json!(code));
    }
    if vk != 0 {
        up.insert("windowsVirtualKeyCode".to_string(), json!(vk));
        up.insert("nativeVirtualKeyCode".to_string(), json!(vk));
    }
    browser
        .command("Input.dispatchKeyEvent", Value::Object(up), true)
        .await?;
    Ok(ok_text(json!({"ok": true}).to_string()))
}

/// `storage`: cookies + Web Storage (localStorage/sessionStorage) in one
/// call, plus a versioned file save/load.
///
/// Cookie reads are page-scoped (`Network.getCookies` with the active url);
/// `allSites:true` widens the read to every site (`Storage.getCookies`,
/// browser level). Cookie writes go per-cookie through `Network.setCookie`
/// and are verified by a read-back diff against the page jar: anything the
/// engine drops (`success:false` or a read-back miss) is a typed
/// `cookie_rejected` naming the cookie and domain - never a silent partial
/// write. Cookie clear is page-scoped (`Network.deleteCookies`); `allSites`
/// wipes the whole profile (`Storage.clearCookies`, owned engine only - on
/// an attached engine that is a browser-wide wipe of somebody else's
/// profile, so it needs explicit `confirm:true`).
///
/// Web Storage scopes run in the page realm as JSON round-trips. Opaque
/// origins (no origin to scope storage to) throw SecurityError in-page, so
/// every storage leg probes first and fails typed (`opaque_origin`) instead
/// of surfacing a thrown exception.
///
/// `save` writes `{gladiusStorage:1,savedAt,url,origin,cookies,
/// localStorage,sessionStorage}`: the parent directory must exist, an
/// existing file is never clobbered without `overwrite:true`, and the reply
/// carries counts/bytes/origin only - never values. `load` validates the
/// file (JSON shape + version) and enforces an origin match unless
/// `force:true`, then reports per-item applied/failed. `redact:true` masks
/// get values with the G0 mask and sets `redacted:true`. Values are never
/// logged (no stderr carries caller data anywhere on this path).
async fn tool_storage(browser: &Browser, args: &Value) -> Result<Value, ToolErr> {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("get");
    if !["get", "set", "clear", "save", "load"].contains(&action) {
        return Err(ToolErr::typed(
            format!("storage: unknown action '{action}' (get|set|clear|save|load)"),
            json!({"code": "invalid_args", "reason": "unknown_action", "tool": "storage"}),
        ));
    }
    let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("all");
    let want_cookies = matches!(scope, "cookies" | "all");
    let want_local = matches!(scope, "localStorage" | "all");
    let want_session = matches!(scope, "sessionStorage" | "all");
    if !(want_cookies || want_local || want_session) {
        return Err(ToolErr::typed(
            format!("storage: unknown scope '{scope}' (cookies|localStorage|sessionStorage|all)"),
            json!({"code": "invalid_args", "reason": "bad_value", "tool": "storage", "arg": "scope", "want": scope}),
        ));
    }
    // Live page identity for every page-scoped leg (and the reply).
    let pair = browser
        .eval("JSON.stringify([window.location.href, window.location.origin])")
        .await
        .map_err(ToolErr::from)?;
    let (url, origin) = match pair
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
    {
        Some(v) => (
            v.get(0).and_then(|x| x.as_str()).unwrap_or("").to_string(),
            v.get(1).and_then(|x| x.as_str()).unwrap_or("").to_string(),
        ),
        None => (String::new(), String::new()),
    };
    let req = StorageReq {
        browser,
        scope,
        want_cookies,
        want_local,
        want_session,
        url: &url,
        origin: &origin,
    };
    match action {
        "get" => storage_get(&req, flag(args, "allSites"), flag(args, "redact")).await,
        "set" => storage_set(&req, args).await,
        "clear" => storage_clear(&req, args, flag(args, "allSites")).await,
        "save" => storage_save(&req, args, flag(args, "allSites")).await,
        _ => storage_load(&req, args).await,
    }
}

/// One page-scoped storage request: the shared legs (realm, scope, page
/// identity) bundled so the action fns stay under the argument limit.
struct StorageReq<'a> {
    browser: &'a Browser,
    scope: &'a str,
    want_cookies: bool,
    want_local: bool,
    want_session: bool,
    url: &'a str,
    origin: &'a str,
}

fn flag(args: &Value, key: &str) -> bool {
    args.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

/// Typed opaque-origin refusal: Web Storage has no origin to scope to here,
/// so every storage read/write is refused instead of surfacing a thrown
/// SecurityError (or, worse, an empty success).
fn opaque_err() -> ToolErr {
    ToolErr::typed(
        "storage: web storage is blocked on this page (opaque origin: no origin to scope it to)"
            .to_string(),
        json!({"code": "invalid_target", "reason": "opaque_origin", "tool": "storage"}),
    )
}

/// Probe that the requested Web Storage scopes are usable in the page realm
/// (a thrown SecurityError means an opaque origin).
async fn storage_realm_ok(
    browser: &Browser,
    want_local: bool,
    want_session: bool,
) -> Result<(), ToolErr> {
    let expr = match (want_local, want_session) {
        (true, true) => "(() => { try { void localStorage.length; void sessionStorage.length; return 'ok'; } catch (e) { return 'GLADIUS_OPAQUE'; } })()",
        (true, false) => "(() => { try { void localStorage.length; return 'ok'; } catch (e) { return 'GLADIUS_OPAQUE'; } })()",
        _ => "(() => { try { void sessionStorage.length; return 'ok'; } catch (e) { return 'GLADIUS_OPAQUE'; } })()",
    };
    match browser.eval(expr).await.map_err(ToolErr::from)?.as_str() {
        Some("GLADIUS_OPAQUE") => Err(opaque_err()),
        _ => Ok(()),
    }
}

/// Read both Web Storage maps in one round-trip (opaque origins fail typed
/// via the `GLADIUS_OPAQUE` marker, never as a thrown exception).
async fn read_storage_maps(
    browser: &Browser,
) -> Result<
    (
        std::collections::BTreeMap<String, String>,
        std::collections::BTreeMap<String, String>,
    ),
    ToolErr,
> {
    let v = browser.eval("(() => { try { const dump = (s) => { const o = Object.create(null); for (let i = 0; i < s.length; i++) { const k = s.key(i); o[k] = s.getItem(k); } return o; }; return JSON.stringify({local: dump(localStorage), session: dump(sessionStorage)}); } catch (e) { return 'GLADIUS_OPAQUE'; } })()").await.map_err(ToolErr::from)?;
    if v.as_str() == Some("GLADIUS_OPAQUE") {
        return Err(opaque_err());
    }
    let parsed: Value = serde_json::from_str(v.as_str().unwrap_or("{}"))
        .map_err(|_| "storage: unreadable web storage".to_string())?;
    let conv = |key: &str| {
        parsed
            .get(key)
            .and_then(|x| x.as_object())
            .map(|o| {
                o.iter()
                    .filter_map(|(k, val)| val.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok((conv("local"), conv("session")))
}

/// Shape one CDP cookie for the reply (固定的 key set; `redact` masks).
fn shape_cookie(c: &Value, redact: bool) -> Value {
    json!({
        "name": c.get("name").cloned().unwrap_or(Value::Null),
        "value": if redact { json!("••••") } else { c.get("value").cloned().unwrap_or(Value::Null) },
        "domain": c.get("domain").cloned().unwrap_or(Value::Null),
        "path": c.get("path").cloned().unwrap_or(Value::Null),
        "secure": c.get("secure").and_then(|v| v.as_bool()).unwrap_or(false),
        "httpOnly": c.get("httpOnly").and_then(|v| v.as_bool()).unwrap_or(false),
        "sameSite": c.get("sameSite").cloned().unwrap_or(Value::Null),
        "expires": c.get("expires").cloned().unwrap_or(Value::Null),
    })
}

/// Lifetime note attached whenever sessionStorage is in scope: tab-scoped
/// storage clears with the tab while localStorage persists across reloads.
fn lifetime_note(want_session: bool) -> Value {
    if want_session {
        json!("sessionStorage is tab-scoped and clears with the tab; localStorage persists across reloads")
    } else {
        Value::Null
    }
}

async fn storage_get(r: &StorageReq<'_>, all_sites: bool, redact: bool) -> Result<Value, ToolErr> {
    let StorageReq {
        browser,
        scope,
        want_cookies,
        want_local,
        want_session,
        url,
        origin,
    } = *r;
    if want_local || want_session {
        storage_realm_ok(browser, want_local, want_session).await?;
    }
    let raw: Vec<Value> = if want_cookies {
        let r = if all_sites {
            browser
                .cdp_raw("Storage.getCookies", json!({}), None)
                .await
                .map_err(ToolErr::from)?
        } else {
            browser
                .command("Network.getCookies", json!({"urls": [url]}), true)
                .await
                .map_err(ToolErr::from)?
        };
        r.get("cookies")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let (local, session) = if want_local || want_session {
        read_storage_maps(browser).await?
    } else {
        (Default::default(), Default::default())
    };
    let mask = |m: &std::collections::BTreeMap<String, String>| -> Value {
        if redact {
            json!(m
                .keys()
                .map(|k| (k.clone(), json!("••••")))
                .collect::<serde_json::Map<String, Value>>())
        } else {
            json!(m)
        }
    };
    let mut out = serde_json::Map::new();
    out.insert("ok".to_string(), json!(true));
    out.insert("action".to_string(), json!("get"));
    out.insert("scope".to_string(), json!(scope));
    out.insert("url".to_string(), json!(url));
    out.insert("origin".to_string(), json!(origin));
    if want_cookies {
        out.insert(
            "cookies".to_string(),
            json!(raw
                .iter()
                .map(|c| shape_cookie(c, redact))
                .collect::<Vec<_>>()),
        );
    }
    if want_local {
        out.insert("localStorage".to_string(), mask(&local));
    }
    if want_session {
        out.insert("sessionStorage".to_string(), mask(&session));
    }
    out.insert(
        "counts".to_string(),
        json!({
            "cookies": raw.len(),
            "localStorage": local.len(),
            "sessionStorage": session.len(),
        }),
    );
    out.insert("redacted".to_string(), json!(redact));
    let note = lifetime_note(want_session);
    if !note.is_null() {
        out.insert("note".to_string(), note);
    }
    Ok(ok_text(Value::Object(out).to_string()))
}

/// A validated `Network.setCookie` row. Validation runs fully before
/// anything applies, so a bad entry changes nothing.
struct SetCookie {
    name: String,
    value: String,
    url: Option<String>,
    domain: Option<String>,
    path: Option<String>,
    secure: bool,
    http_only: bool,
    same_site: Option<String>,
    expires: Option<f64>,
}

fn bad_cookie(what: &str) -> ToolErr {
    ToolErr::typed(
        format!("storage: bad cookie entry ({what}; need name+value strings)"),
        json!({"code": "invalid_args", "reason": "bad_value", "tool": "storage", "arg": "cookies"}),
    )
}

fn parse_set_cookie(c: &Value) -> Result<SetCookie, ToolErr> {
    let obj = c.as_object().ok_or_else(|| bad_cookie("not an object"))?;
    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| bad_cookie("missing name"))?
        .to_string();
    let value = obj
        .get("value")
        .and_then(|v| v.as_str())
        .ok_or_else(|| bad_cookie("missing value"))?
        .to_string();
    let opt_str = |key: &str| -> Result<Option<String>, ToolErr> {
        match obj.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            _ => Err(bad_cookie(&format!("bad {key}"))),
        }
    };
    let opt_bool = |key: &str| -> Result<bool, ToolErr> {
        match obj.get(key) {
            None | Some(Value::Null) => Ok(false),
            Some(Value::Bool(b)) => Ok(*b),
            _ => Err(bad_cookie(&format!("bad {key}"))),
        }
    };
    let same_site = match opt_str("sameSite")? {
        None => None,
        Some(s) => Some(match s.to_ascii_lowercase().as_str() {
            "strict" => "Strict".to_string(),
            "lax" => "Lax".to_string(),
            "none" => "None".to_string(),
            _ => {
                return Err(ToolErr::typed(
                    format!("storage: bad cookie sameSite '{s}' (want Strict|Lax|None)"),
                    json!({"code": "invalid_args", "reason": "bad_value", "tool": "storage", "arg": "cookies"}),
                ));
            }
        }),
    };
    let secure = opt_bool("secure")?;
    if same_site.as_deref() == Some("None") && !secure {
        return Err(ToolErr::typed(
            "storage: Secure is required with SameSite=None".to_string(),
            json!({"code": "invalid_args", "reason": "bad_value", "tool": "storage", "arg": "cookies"}),
        ));
    }
    let expires = match obj.get("expires") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_f64() {
            Some(f) if f.is_finite() => Some(f),
            _ => return Err(bad_cookie("bad expires")),
        },
    };
    Ok(SetCookie {
        name,
        value,
        url: opt_str("url")?,
        domain: opt_str("domain")?,
        path: opt_str("path")?,
        secure,
        http_only: opt_bool("httpOnly")?,
        same_site,
        expires,
    })
}

fn cookie_matches(stored: &Value, name: &str, value: &str, domain: &str, path: &str) -> bool {
    if stored.get("name").and_then(|v| v.as_str()) != Some(name) {
        return false;
    }
    if stored.get("value").and_then(|v| v.as_str()) != Some(value) {
        return false;
    }
    if !domain.is_empty() {
        let sd = stored
            .get("domain")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim_start_matches('.');
        if sd != domain.trim_start_matches('.') {
            return false;
        }
    }
    if !path.is_empty() {
        let sp = stored.get("path").and_then(|v| v.as_str()).unwrap_or("");
        if sp != path {
            return false;
        }
    }
    true
}

/// Apply validated cookies: per-cookie `Network.setCookie`, then a
/// read-back diff against the page jar. Returns (applied, failed); a
/// `success:false` reply and a read-back miss both land in `failed` - the
/// caller decides between the typed `cookie_rejected` (set) and the
/// applied/failed report (load).
async fn apply_cookies(
    browser: &Browser,
    default_url: &str,
    cookies: &[SetCookie],
) -> Result<(usize, Vec<Value>), ToolErr> {
    let mut failed = Vec::new();
    for c in cookies {
        let mut params = serde_json::Map::new();
        params.insert("name".to_string(), json!(c.name));
        params.insert("value".to_string(), json!(c.value));
        params.insert(
            "url".to_string(),
            json!(c.url.clone().unwrap_or_else(|| default_url.to_string())),
        );
        if let Some(d) = &c.domain {
            params.insert("domain".to_string(), json!(d));
        }
        if let Some(p) = &c.path {
            params.insert("path".to_string(), json!(p));
        }
        if c.secure {
            params.insert("secure".to_string(), json!(true));
        }
        if c.http_only {
            params.insert("httpOnly".to_string(), json!(true));
        }
        if let Some(s) = &c.same_site {
            params.insert("sameSite".to_string(), json!(s));
        }
        if let Some(e) = c.expires {
            params.insert("expires".to_string(), json!(e));
        }
        match browser
            .command("Network.setCookie", Value::Object(params), true)
            .await
        {
            Ok(r) if r.get("success").and_then(|v| v.as_bool()) == Some(true) => {}
            _ => failed.push(json!({
                "cookie": c.name,
                "domain": c.domain.clone().unwrap_or_default(),
                "reason": "set_failed",
            })),
        }
    }
    // Read-back diff against the WHOLE jar (`Storage.getCookies`): a
    // cookie stored under a non-current path (or another readable scope)
    // is stored, not rejected - the page-URL jar cannot see it, so it must
    // not judge it. Comparison is by name/domain/path, not current URL.
    if !cookies.is_empty() {
        let live = browser
            .cdp_raw("Storage.getCookies", json!({}), None)
            .await
            .map_err(ToolErr::from)?;
        let jar = live
            .get("cookies")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        for c in cookies {
            if failed
                .iter()
                .any(|f| f.get("cookie").and_then(|v| v.as_str()) == Some(c.name.as_str()))
            {
                continue;
            }
            let dom = c.domain.clone().unwrap_or_default();
            let pth = c.path.clone().unwrap_or_default();
            if !jar
                .iter()
                .any(|s| cookie_matches(s, &c.name, &c.value, &dom, &pth))
            {
                failed.push(json!({
                    "cookie": c.name,
                    "domain": dom,
                    "reason": "not_stored",
                }));
            }
        }
    }
    let applied = cookies.len().saturating_sub(failed.len());
    Ok((applied, failed))
}

fn cookie_rejected(first: &Value, failed: &[Value]) -> ToolErr {
    ToolErr::typed(
        format!(
            "storage: cookie rejected (not stored): {}",
            first.get("cookie").and_then(|v| v.as_str()).unwrap_or("?")
        ),
        json!({
            "code": "cookie_rejected",
            "reason": "rejected",
            "tool": "storage",
            "cookie": first.get("cookie").cloned().unwrap_or(Value::Null),
            "domain": first.get("domain").cloned().unwrap_or(Value::Null),
            "failed": failed,
        }),
    )
}

/// Parse `items` (key->string map). `None` means absent (empty map); a
/// non-object or non-string value is a typed `bad_value` before anything
/// applies.
fn parse_items(v: Option<&Value>) -> Result<std::collections::BTreeMap<String, String>, ToolErr> {
    let Some(obj) = v else {
        return Ok(Default::default());
    };
    let map = obj.as_object().ok_or_else(|| {
        ToolErr::typed(
            "storage: 'items' must be an object (key->value strings)".to_string(),
            json!({"code": "invalid_args", "reason": "bad_value", "tool": "storage", "arg": "items"}),
        )
    })?;
    let mut out = std::collections::BTreeMap::new();
    for (k, val) in map {
        match val.as_str() {
            Some(s) => {
                out.insert(k.clone(), s.to_string());
            }
            None => {
                return Err(ToolErr::typed(
                    format!("storage: bad item value for key '{k}' (want a string)"),
                    json!({"code": "invalid_args", "reason": "bad_value", "tool": "storage", "arg": "items"}),
                ));
            }
        }
    }
    Ok(out)
}

/// Write validated items into one Web Storage scope. Returns
/// (applied, failed-keys); quota/opaque write failures land in `failed`.
async fn apply_items(
    browser: &Browser,
    local: bool,
    items: &std::collections::BTreeMap<String, String>,
) -> Result<(usize, Vec<Value>), ToolErr> {
    if items.is_empty() {
        return Ok((0, Vec::new()));
    }
    let scope = if local {
        "localStorage"
    } else {
        "sessionStorage"
    };
    let store = if local {
        "localStorage"
    } else {
        "sessionStorage"
    };
    // Array-of-pairs, never an object literal: `{"__proto__": v}` would set
    // the prototype instead of an own property and silently drop the key.
    let pairs: Vec<(&String, &String)> = items.iter().collect();
    let lit = serde_json::to_string(&pairs).map_err(|e| e.to_string())?;
    let expr = format!("((pairs) => {{ const s = {store}; const failed = []; for (const [k, v] of pairs) {{ try {{ s.setItem(k, v); }} catch (e) {{ failed.push(k); }} }} return JSON.stringify({{failed}}); }})({lit})");
    let v = browser.eval(&expr).await.map_err(ToolErr::from)?;
    let keys: Vec<String> = v
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .and_then(|p| {
            p.get("failed").and_then(|f| f.as_array()).map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
        })
        .unwrap_or_default();
    let failed: Vec<Value> = keys
        .into_iter()
        .map(|k| json!({"scope": scope, "key": k, "reason": "not_stored"}))
        .collect();
    Ok((items.len().saturating_sub(failed.len()), failed))
}

async fn storage_set(r: &StorageReq<'_>, args: &Value) -> Result<Value, ToolErr> {
    let StorageReq {
        browser,
        scope,
        want_cookies,
        want_local,
        want_session,
        url,
        origin,
    } = *r;
    // Validate everything before anything applies (a bad entry changes
    // nothing).
    let mut cookies = Vec::new();
    if want_cookies {
        match args.get("cookies") {
            None => {}
            Some(Value::Array(a)) => {
                for c in a {
                    cookies.push(parse_set_cookie(c)?);
                }
            }
            _ => {
                return Err(ToolErr::typed(
                    "storage: 'cookies' must be an array of {name, value, ...}".to_string(),
                    json!({"code": "invalid_args", "reason": "bad_value", "tool": "storage", "arg": "cookies"}),
                ));
            }
        }
    }
    let items = parse_items(args.get("items"))?;
    let has_cookies = want_cookies && !cookies.is_empty();
    let has_items = (want_local || want_session) && !items.is_empty();
    if !has_cookies && !has_items {
        return Err(ToolErr::typed(
            "storage: set needs 'cookies' and/or 'items'".to_string(),
            json!({"code": "invalid_args", "reason": "need_option", "tool": "storage"}),
        ));
    }
    if want_local || want_session {
        storage_realm_ok(browser, want_local, want_session).await?;
    }
    let (applied_cookies, mut failed) = if has_cookies {
        apply_cookies(browser, url, &cookies).await?
    } else {
        (0, Vec::new())
    };
    let (applied_local, failed_local) = if want_local {
        apply_items(browser, true, &items).await?
    } else {
        (0, Vec::new())
    };
    let (applied_session, failed_session) = if want_session {
        apply_items(browser, false, &items).await?
    } else {
        (0, Vec::new())
    };
    failed.extend(failed_local);
    failed.extend(failed_session);
    if !failed.is_empty() {
        // Cookie failures carry the typed rejection; storage-only failures
        // (quota) keep the engine-shaped fallback.
        if let Some(first) = failed.iter().find(|f| f.get("cookie").is_some()) {
            return Err(cookie_rejected(first, &failed));
        }
        return Err(ToolErr::fallback(format!(
            "storage: {} web-storage item(s) not stored",
            failed.len()
        )));
    }
    Ok(ok_text(
        json!({
            "ok": true, "action": "set", "scope": scope, "url": url, "origin": origin,
            "set": {"cookies": applied_cookies, "localStorage": applied_local, "sessionStorage": applied_session},
            "failed": [],
        })
        .to_string(),
    ))
}

async fn storage_clear(
    r: &StorageReq<'_>,
    args: &Value,
    all_sites: bool,
) -> Result<Value, ToolErr> {
    let StorageReq {
        browser,
        scope,
        want_cookies,
        want_local,
        want_session,
        url,
        origin,
    } = *r;
    let mut cleared = (0usize, 0usize, 0usize);
    if want_cookies {
        if all_sites {
            // Browser-wide wipe: on an attached engine that is somebody
            // else's profile, so it needs explicit confirmation.
            if crate::browser::cdp_url_override().is_some() && !flag(args, "confirm") {
                return Err(ToolErr::typed(
                    "storage: clear with allSites:true on an attached engine needs confirm:true (browser-wide cookie wipe)".to_string(),
                    json!({"code": "invalid_args", "reason": "missing_arg", "tool": "storage", "arg": "confirm"}),
                ));
            }
            let before = browser
                .cdp_raw("Storage.getCookies", json!({}), None)
                .await
                .map_err(ToolErr::from)?;
            cleared.0 = before
                .get("cookies")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            browser
                .cdp_raw("Storage.clearCookies", json!({}), None)
                .await
                .map_err(ToolErr::from)?;
        } else {
            let live = browser
                .command("Network.getCookies", json!({"urls": [url]}), true)
                .await
                .map_err(ToolErr::from)?;
            let jar = live
                .get("cookies")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for c in &jar {
                if let Some(name) = c.get("name").and_then(|v| v.as_str()) {
                    browser
                        .command(
                            "Network.deleteCookies",
                            json!({"name": name, "url": url}),
                            true,
                        )
                        .await
                        .map_err(ToolErr::from)?;
                    cleared.0 += 1;
                }
            }
        }
    }
    if want_local || want_session {
        storage_realm_ok(browser, want_local, want_session).await?;
        if want_local {
            let v = browser.eval("(() => { try { const n = localStorage.length; localStorage.clear(); return n; } catch (e) { return 'GLADIUS_OPAQUE'; } })()").await.map_err(ToolErr::from)?;
            if v.as_str() == Some("GLADIUS_OPAQUE") {
                return Err(opaque_err());
            }
            cleared.1 = v.as_i64().unwrap_or(0).max(0) as usize;
        }
        if want_session {
            let v = browser.eval("(() => { try { const n = sessionStorage.length; sessionStorage.clear(); return n; } catch (e) { return 'GLADIUS_OPAQUE'; } })()").await.map_err(ToolErr::from)?;
            if v.as_str() == Some("GLADIUS_OPAQUE") {
                return Err(opaque_err());
            }
            cleared.2 = v.as_i64().unwrap_or(0).max(0) as usize;
        }
    }
    let mut out = serde_json::Map::new();
    out.insert("ok".to_string(), json!(true));
    out.insert("action".to_string(), json!("clear"));
    out.insert("scope".to_string(), json!(scope));
    out.insert("url".to_string(), json!(url));
    out.insert("origin".to_string(), json!(origin));
    out.insert(
        "cleared".to_string(),
        json!({"cookies": cleared.0, "localStorage": cleared.1, "sessionStorage": cleared.2}),
    );
    let note = lifetime_note(want_session);
    if !note.is_null() {
        out.insert("note".to_string(), note);
    }
    Ok(ok_text(Value::Object(out).to_string()))
}

/// Current time as unix millis for `savedAt` (no new dependency).
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

async fn storage_save(r: &StorageReq<'_>, args: &Value, all_sites: bool) -> Result<Value, ToolErr> {
    let StorageReq {
        browser,
        scope,
        want_cookies,
        want_local,
        want_session,
        url,
        origin,
    } = *r;
    let path = match args.get("path").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => {
            return Err(ToolErr::typed(
                "storage: save needs required arg 'path'".to_string(),
                json!({"code": "invalid_args", "reason": "missing_arg", "tool": "storage"}),
            ));
        }
    };
    let dest = std::path::Path::new(&path);
    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() && !parent.is_dir() {
            return Err(ToolErr::typed(
                format!(
                    "storage: save parent directory does not exist: {}",
                    parent.display()
                ),
                json!({"code": "invalid_args", "reason": "bad_value", "tool": "storage", "arg": "path"}),
            ));
        }
    }
    if dest.exists() && !flag(args, "overwrite") {
        return Err(ToolErr::typed(
            format!("storage: refusing to overwrite existing file (pass overwrite:true): {path}"),
            json!({"code": "invalid_args", "reason": "bad_value", "tool": "storage", "arg": "path"}),
        ));
    }
    // Gather (storage reads gate opaque origins themselves).
    let cookies: Vec<Value> = if want_cookies {
        let r = if all_sites {
            browser
                .cdp_raw("Storage.getCookies", json!({}), None)
                .await
                .map_err(ToolErr::from)?
        } else {
            browser
                .command("Network.getCookies", json!({"urls": [url]}), true)
                .await
                .map_err(ToolErr::from)?
        };
        r.get("cookies")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let (local, session) = if want_local || want_session {
        read_storage_maps(browser).await?
    } else {
        (Default::default(), Default::default())
    };
    let doc = json!({
        "gladiusStorage": 1,
        "savedAt": now_ms(),
        "url": url,
        "origin": origin,
        "cookies": if want_cookies { cookies.clone() } else { Vec::new() },
        "localStorage": if want_local { json!(local) } else { json!({}) },
        "sessionStorage": if want_session { json!(session) } else { json!({}) },
    });
    let bytes = serde_json::to_string(&doc).map_err(|e| e.to_string())?;
    std::fs::write(dest, &bytes).map_err(|e| format!("storage: save failed: {e}"))?;
    let counts = json!({
        "cookies": cookies.len(),
        "localStorage": local.len(),
        "sessionStorage": session.len(),
    });
    // The reply carries counts/bytes/origin only - never values.
    Ok(ok_text(
        json!({
            "ok": true, "action": "save", "scope": scope,
            "url": url, "origin": origin,
            "counts": counts, "path": path, "bytes": bytes.len() as i64,
        })
        .to_string(),
    ))
}

async fn storage_load(r: &StorageReq<'_>, args: &Value) -> Result<Value, ToolErr> {
    let StorageReq {
        browser,
        scope,
        want_cookies,
        want_local,
        want_session,
        url,
        origin,
    } = *r;
    let path = match args.get("path").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => {
            return Err(ToolErr::typed(
                "storage: load needs required arg 'path'".to_string(),
                json!({"code": "invalid_args", "reason": "missing_arg", "tool": "storage"}),
            ));
        }
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ToolErr::typed(
                format!("storage: file not found: {path}"),
                json!({"code": "file_not_found", "reason": "missing_path", "tool": "storage", "path": path}),
            ));
        }
        Err(e) => return Err(ToolErr::fallback(format!("storage: load failed: {e}"))),
    };
    let doc: Value = serde_json::from_str(&text).map_err(|_| {
        ToolErr::typed(
            "storage: not valid gladius storage JSON (want {gladiusStorage:1,...})".to_string(),
            json!({"code": "invalid_args", "reason": "bad_value", "tool": "storage", "arg": "path"}),
        )
    })?;
    if doc.get("gladiusStorage").and_then(|v| v.as_i64()) != Some(1) {
        return Err(ToolErr::typed(
            "storage: unsupported storage version (want gladiusStorage:1)".to_string(),
            json!({"code": "invalid_args", "reason": "bad_value", "tool": "storage", "arg": "path"}),
        ));
    }
    let file_origin = doc
        .get("origin")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if file_origin != origin && !flag(args, "force") {
        return Err(ToolErr::typed(
            format!("storage: origin mismatch (file {file_origin} vs current {origin}; pass force:true to apply anyway)"),
            json!({"code": "origin_mismatch", "reason": "mismatch", "tool": "storage", "fileOrigin": file_origin, "origin": origin, "path": path}),
        ));
    }
    // Validate everything before anything applies. Malformed rows fail
    // open into the failed list (the file is caller data, not the schema),
    // so the reply accounts for every row.
    let mut cookies = Vec::new();
    let mut failed: Vec<Value> = Vec::new();
    if want_cookies {
        match doc.get("cookies") {
            None | Some(Value::Null) => {}
            Some(Value::Array(a)) => {
                for c in a {
                    match parse_set_cookie(c) {
                        Ok(sc) => cookies.push(sc),
                        Err(_) => failed.push(json!({
                            "cookie": c.get("name").and_then(|v| v.as_str()).unwrap_or("?"),
                            "domain": c.get("domain").and_then(|v| v.as_str()).unwrap_or(""),
                            "reason": "bad_entry",
                        })),
                    }
                }
            }
            _ => {
                return Err(ToolErr::typed(
                    "storage: bad file cookies (want an array)".to_string(),
                    json!({"code": "invalid_args", "reason": "bad_value", "tool": "storage", "arg": "path"}),
                ));
            }
        }
    }
    let file_items = |key: &str| -> Result<std::collections::BTreeMap<String, String>, ToolErr> {
        match doc.get(key) {
            None | Some(Value::Null) => Ok(Default::default()),
            Some(v) => parse_items(Some(v)).map_err(|_| {
                ToolErr::typed(
                    format!("storage: bad file {key} (want an object)"),
                    json!({"code": "invalid_args", "reason": "bad_value", "tool": "storage", "arg": "path"}),
                )
            }),
        }
    };
    let local_items = if want_local {
        file_items("localStorage")?
    } else {
        Default::default()
    };
    let session_items = if want_session {
        file_items("sessionStorage")?
    } else {
        Default::default()
    };
    if (want_local && !local_items.is_empty()) || (want_session && !session_items.is_empty()) {
        storage_realm_ok(browser, want_local, want_session).await?;
    }
    let (applied_cookies, failed_cookies) = if want_cookies && !cookies.is_empty() {
        apply_cookies(browser, url, &cookies).await?
    } else {
        (0, Vec::new())
    };
    failed.extend(failed_cookies);
    let (applied_local, failed_local) = if want_local {
        apply_items(browser, true, &local_items).await?
    } else {
        (0, Vec::new())
    };
    let (applied_session, failed_session) = if want_session {
        apply_items(browser, false, &session_items).await?
    } else {
        (0, Vec::new())
    };
    failed.extend(failed_local);
    failed.extend(failed_session);
    Ok(ok_text(
        json!({
            "ok": true, "action": "load", "scope": scope,
            "url": url, "origin": origin,
            "path": path, "fileOrigin": file_origin,
            "applied": {"cookies": applied_cookies, "localStorage": applied_local, "sessionStorage": applied_session},
            "failed": failed,
        })
        .to_string(),
    ))
}

/// `viewport`: set (or clear) the CDP device-metrics override on the active
/// `viewport`: set (or clear) the CDP device-metrics override on the active
/// page. The reply always carries the LIVE read-back of
/// `window.innerWidth` / `window.innerHeight` / `window.devicePixelRatio`
/// next to what was requested, so an override the engine accepted but
/// ignored shows up as `"matched": false` instead of a false success.
async fn tool_viewport(browser: &Browser, args: &Value) -> Result<Value, ToolErr> {
    check_unknown(
        "viewport",
        args,
        &["width", "height", "deviceScaleFactor", "mobile", "reset"],
    )?;
    if args.get("reset").and_then(|v| v.as_bool()).unwrap_or(false) {
        browser.viewport_reset().await?;
        let (w, h, dpr) = browser.viewport_readback().await?;
        return Ok(ok_text(
            json!({
                "reset": true,
                "applied": Value::Null,
                "readback": {"innerWidth": w, "innerHeight": h, "devicePixelRatio": dpr},
                "note": "override cleared; the read-back above is the engine default for this page",
            })
            .to_string(),
        ));
    }
    // Positive integers only: a zero/negative viewport is not a smaller
    // page, it is a silent no-op (or an engine error) - refuse it instead
    // of reporting a size that will never be honoured.
    let dim = |name: &str| -> Result<i64, ToolErr> {
        let v = args.get(name).ok_or_else(|| {
            ToolErr::typed(
                format!("viewport: missing required arg '{name}'"),
                json!({"code": "invalid_args", "reason": "missing_arg", "tool": "viewport"}),
            )
        })?;
        // JSON numbers arrive as int or float depending on the client
        // (1440 vs 1440.0); accept an integral float, reject the rest.
        let n = match v.as_i64() {
            Some(n) => n,
            None => match v.as_f64() {
                Some(f) if f.fract() == 0.0 => f as i64,
                _ => {
                    return Err(ToolErr::typed(
                        format!("viewport: '{name}' must be an integer, got {v}"),
                        json!({"code": "invalid_args", "reason": "bad_value", "tool": "viewport"}),
                    ));
                }
            },
        };
        if n < 1 {
            return Err(ToolErr::typed(
                format!("viewport: '{name}' must be >= 1, got {n}"),
                json!({"code": "invalid_args", "reason": "bad_value", "tool": "viewport"}),
            ));
        }
        Ok(n)
    };
    let width = dim("width")?;
    let height = dim("height")?;
    let dsf = match args.get("deviceScaleFactor") {
        None | Some(Value::Null) => 1.0,
        Some(v) => match v.as_f64() {
            Some(f) if f > 0.0 => f,
            _ => {
                return Err(ToolErr::typed(
                    format!("viewport: 'deviceScaleFactor' must be a number > 0, got {v}"),
                    json!({"code": "invalid_args", "reason": "bad_value", "tool": "viewport"}),
                ));
            }
        },
    };
    let mobile = args
        .get("mobile")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let params = json!({
        "width": width,
        "height": height,
        "deviceScaleFactor": dsf,
        "mobile": mobile,
    });
    browser.viewport_set(params).await?;
    let (w, h, dpr) = browser.viewport_readback().await?;
    Ok(ok_text(
        json!({
            "applied": {
                "width": width,
                "height": height,
                "deviceScaleFactor": dsf,
                "mobile": mobile,
            },
            "readback": {"innerWidth": w, "innerHeight": h, "devicePixelRatio": dpr},
            "matched": w == width && h == height,
        })
        .to_string(),
    ))
}

async fn tool_tabs(
    browser: &Browser,
    action: &str,
    url: Option<&str>,
    index: Option<i64>,
) -> Result<Value, ToolErr> {
    let to_usize = |i: Option<i64>| -> Result<usize, ToolErr> {
        match i {
            Some(n) if n >= 0 => Ok(n as usize),
            Some(n) => Err(ToolErr::typed(
                format!("tabs: bad index {n}"),
                json!({"code": "invalid_args", "reason": "bad_index", "tool": "tabs"}),
            )),
            None => Err(ToolErr::typed(
                "tabs: missing required arg 'index'".to_string(),
                json!({"code": "invalid_args", "reason": "missing_arg", "tool": "tabs"}),
            )),
        }
    };
    let inner = match action {
        "list" => browser.tabs_list().await.map_err(tabs_err)?,
        "new" => browser.tabs_new(url).await.map_err(tabs_err)?,
        "select" => browser
            .tabs_select(to_usize(index)?)
            .await
            .map_err(tabs_err)?,
        "close" => browser
            .tabs_close(to_usize(index)?)
            .await
            .map_err(tabs_err)?,
        other => {
            return Err(ToolErr::typed(
                format!("tabs: unknown action '{other}'"),
                json!({"code": "invalid_args", "reason": "unknown_action", "tool": "tabs"}),
            ))
        }
    };
    Ok(ok_text(inner.to_string()))
}

/// Engine-originated `tabs: no tab at index N` (raised in browser.rs, where
/// the tab index space lives and this file must not reach): the suffix is a
/// plain integer, never caller prose, so re-attaching the site-known type
/// here is exact. Anything else keeps the generic fallback.
fn tabs_err(e: String) -> ToolErr {
    if e.starts_with("tabs: no tab at index ") {
        ToolErr::typed(
            e,
            json!({"code": "invalid_target", "reason": "not_found", "tool": "tabs"}),
        )
    } else {
        ToolErr::fallback(e)
    }
}

/// `targets`: list every CDP target with its stable id plus the active
/// page's frame tree, or make a page target current by id.
async fn tool_targets(
    browser: &Browser,
    action: &str,
    target: Option<&str>,
) -> Result<Value, ToolErr> {
    match action {
        "list" => {
            // Frames first: it ensures a page session exists (attach), so
            // the `active` flag below names the tracked page even on a cold
            // start. Either leg failing is the tool's error.
            let frames = browser
                .frame_tree()
                .await
                .map_err(targets_err)?
                .get("frames")
                .cloned()
                .unwrap_or(json!([]));
            let mut merged = browser.targets_all().await.map_err(targets_err)?;
            if let Some(obj) = merged.as_object_mut() {
                obj.insert("frames".to_string(), frames);
            }
            Ok(ok_text(merged.to_string()))
        }
        "select" => {
            let tid = match target {
                Some(t) => t,
                None => {
                    return Err(ToolErr::typed(
                        "targets: missing required arg 'target' (CDP targetId)".to_string(),
                        json!({"code": "invalid_args", "reason": "missing_arg", "tool": "targets"}),
                    ));
                }
            };
            let inner = browser.select_target(tid).await.map_err(targets_err)?;
            Ok(ok_text(inner.to_string()))
        }
        other => Err(ToolErr::typed(
            format!("targets: unknown action '{other}' (list|select)"),
            json!({"code": "invalid_args", "reason": "unknown_action", "tool": "targets"}),
        )),
    }
}

/// Engine-originated `targets:` failures (raised in browser.rs, where the
/// target/session space lives): the prefix before the first detail is
/// caller-free prose, so re-type exact known shapes here. Select of a
/// non-page target and unknown ids carry `invalid_target`; attach/frame
/// failures keep the generic fallback (CDP/js-engine territory).
fn targets_err(e: String) -> ToolErr {
    if e.starts_with("targets: no target with id ") {
        ToolErr::typed(
            e,
            json!({"code": "invalid_target", "reason": "not_found", "tool": "targets"}),
        )
    } else if e.contains("not a page (select takes page targets") {
        ToolErr::typed(
            e,
            json!({"code": "invalid_target", "reason": "not_select", "tool": "targets"}),
        )
    } else {
        ToolErr::fallback(e)
    }
}

/// `dialog`: report the last auto-handled dialog plus the live policy
/// (`get`), or set the default policy for the next dialog (`policy`).
async fn tool_dialog(
    browser: &Browser,
    action: &str,
    accept: Option<bool>,
    prompt_text: Option<&str>,
) -> Result<Value, ToolErr> {
    match action {
        "get" => {
            let last = match browser.last_dialog().await {
                Some(d) => json!({"type": d.dtype, "message": d.message, "url": d.url}),
                None => Value::Null,
            };
            let p = browser.dialog_policy_get().await;
            Ok(ok_text(
                json!({"dialog": last,
                       "policy": {"accept": p.accept, "promptText": p.prompt_text}})
                .to_string(),
            ))
        }
        "policy" => {
            let a = match accept {
                Some(v) => v,
                None => {
                    return Err(ToolErr::typed(
                        "dialog: 'policy' needs required arg 'accept' (bool)".to_string(),
                        json!({"code": "invalid_args", "reason": "missing_arg", "tool": "dialog"}),
                    ));
                }
            };
            let p = browser
                .dialog_policy_set(a, prompt_text.map(|s| s.to_string()))
                .await
                .map_err(ToolErr::fallback)?;
            Ok(ok_text(
                json!({"policy": {"accept": p.accept, "promptText": p.prompt_text}}).to_string(),
            ))
        }
        other => Err(ToolErr::typed(
            format!("dialog: unknown action '{other}' (get|policy)"),
            json!({"code": "invalid_args", "reason": "unknown_action", "tool": "dialog"}),
        )),
    }
}

/// `history`: back / forward / reload on the active page.
async fn tool_history(browser: &Browser, action: &str) -> Result<Value, ToolErr> {
    match browser.history(action).await {
        Ok(v) => Ok(ok_text(v.to_string())),
        Err(e) => Err(history_err(e)),
    }
}

/// Engine-originated `history:` failures: unknown actions and boundary
/// steps are site-known types; CDP failures keep the fallback.
fn history_err(e: String) -> ToolErr {
    if e.starts_with("history: unknown action ") {
        ToolErr::typed(
            e,
            json!({"code": "invalid_args", "reason": "unknown_action", "tool": "history"}),
        )
    } else if e.contains("(at boundary)") {
        ToolErr::typed(
            e,
            json!({"code": "invalid_target", "reason": "not_found", "tool": "history"}),
        )
    } else {
        ToolErr::fallback(e)
    }
}

/// `network`: get / drain / clear the captured request/response ring, with
/// an optional plain-substring url filter and a return cap.
async fn tool_network(
    browser: &Browser,
    action: &str,
    filter: Option<&str>,
    limit: Option<i64>,
) -> Result<Value, ToolErr> {
    // Cap the RETURNED slice; the ring itself is NET_CAP (browser.rs).
    let lim = limit.unwrap_or(100).clamp(1, 500) as usize;
    match action {
        "get" => Ok(ok_text(browser.network_get(filter, lim).await.to_string())),
        "drain" => Ok(ok_text(
            browser.network_drain(filter, lim).await.to_string(),
        )),
        "clear" => {
            let n = browser.network_clear().await;
            Ok(ok_text(
                json!({"cleared": n, "cap": crate::browser::NET_CAP}).to_string(),
            ))
        }
        other => Err(ToolErr::typed(
            format!("network: unknown action '{other}' (get|drain|clear)"),
            json!({"code": "invalid_args", "reason": "unknown_action", "tool": "network"}),
        )),
    }
}

/// `cdp`: raw protocol escape hatch. `params` defaults to `{}`; `session`
/// names one attached session (browser-level when absent). The raw CDP
/// `result` rides back verbatim; a CDP error reply is the tool's `isError`,
/// never a protocol error.
async fn tool_cdp(
    browser: &Browser,
    method: &str,
    params: Value,
    session: Option<String>,
) -> Result<Value, ToolErr> {
    if method.trim().is_empty() {
        return Err(ToolErr::typed(
            "cdp: missing required arg 'method'".to_string(),
            json!({"code": "invalid_args", "reason": "missing_arg", "tool": "cdp"}),
        ));
    }
    match browser.cdp_raw(method, params, session).await {
        Ok(v) => Ok(ok_text(json!({"result": v}).to_string())),
        Err(e) => Err(ToolErr::fallback(e)),
    }
}

#[cfg(test)]
mod screenshot_file_tests {
    use super::write_screenshot_file;

    // 1x1 transparent PNG, decoded with the same `base64` crate the tool uses.
    const TINY_PNG_B64: &str =
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

    fn decode_fixture() -> Vec<u8> {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(TINY_PNG_B64)
            .expect("fixture must decode")
    }

    #[test]
    fn path_writes_valid_png_and_creates_missing_parents() {
        let dir =
            std::env::temp_dir().join(format!("gladius-screenshot-test-{}", std::process::id()));
        let target = dir.join("nested").join("shot.png");
        let _ = std::fs::remove_dir_all(&dir);
        if let Err(e) = write_screenshot_file(target.to_str().unwrap(), &decode_fixture()) {
            panic!("write must succeed: {}", e.prose);
        }
        let back = std::fs::read(&target).expect("file must exist");
        assert!(
            back.len() >= 8 && back[..8] == PNG_MAGIC,
            "file must start with the PNG magic bytes"
        );
        assert_eq!(back, decode_fixture(), "round trip must be exact");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relative_path_is_rejected() {
        let r = write_screenshot_file("relative/shot.png", &decode_fixture());
        assert!(r.is_err(), "relative path must fail closed");
    }
}
