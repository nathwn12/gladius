use serde_json::{Value, json};

use crate::browser::Browser;
use crate::fetch::fetch_text;

pub fn list() -> Value {
    json!({"tools": [
        {"name": "navigate", "description": "Navigate the active page to a URL",
         "inputSchema": {"type": "object", "properties": {"url": {"type": "string"}}, "required": ["url"]}},
        {"name": "snapshot", "description": "AX snapshot: one line per actionable node with a stable ref (@e1, @e2, ...), role/name/value, plus page text. Pass {\"mode\":\"text\"} for the legacy text-only output.",
         "inputSchema": {"type": "object", "properties": {"mode": {"type": "string"}}}},
        {"name": "screenshot", "description": "PNG screenshot of the active page (unavailable with GLADIUS_BACKEND_CAPS=dom-only: no rendering pipeline)",
         "inputSchema": {"type": "object", "properties": {"fullPage": {"type": "boolean"}}}},
        {"name": "click", "description": "Click an element by CSS selector or AX ref (@eN from snapshot; exact, never first-match)",
         "inputSchema": {"type": "object", "properties": {"element": {"type": "string"}}, "required": ["element"]}},
        {"name": "type", "description": "Type text into an element by CSS selector or AX ref (@eN from snapshot; exact)",
         "inputSchema": {"type": "object", "properties": {
             "element": {"type": "string"}, "text": {"type": "string"}, "submit": {"type": "boolean"}},
             "required": ["element", "text"]}},
        {"name": "evaluate", "description": "Evaluate a JS expression in the page, await promises",
         "inputSchema": {"type": "object", "properties": {"expression": {"type": "string"}}, "required": ["expression"]}},
        {"name": "wait_for", "description": "Wait for a selector/text, a delay, or network idle (max 15s). networkIdle:true waits for a quiet-network window of idleMs (default 500); a page that never goes idle returns the typed timeout.",
         "inputSchema": {"type": "object", "properties": {
             "selector": {"type": "string"}, "text": {"type": "string"}, "ms": {"type": "integer"},
             "networkIdle": {"type": "boolean"}, "idleMs": {"type": "integer"}}}},
        {"name": "console", "description": "Console capture: buffered console messages + uncaught exceptions, each {type,text,url,line,ts}. Ring buffer, cap 200. action get (default)|drain|clear; limit caps returned (default 100).",
         "inputSchema": {"type": "object", "properties": {
             "action": {"type": "string"}, "limit": {"type": "integer"}}}},
        {"name": "upload", "description": "Set files on an <input type=file> by CSS selector or AX ref (@eN from snapshot; exact). paths[] must exist locally.",
         "inputSchema": {"type": "object", "properties": {
             "element": {"type": "string"}, "paths": {"type": "array", "items": {"type": "string"}}},
             "required": ["element", "paths"]}},
        {"name": "fetch_text", "description": "Fast plain-HTTP GET with no browser; HTML stripped to text",
         "inputSchema": {"type": "object", "properties": {"url": {"type": "string"}}, "required": ["url"]}},
        {"name": "hover", "description": "Hover over an element by CSS selector or AX ref (@eN from snapshot; exact)",
         "inputSchema": {"type": "object", "properties": {"element": {"type": "string"}}, "required": ["element"]}},
        {"name": "select", "description": "Choose a <select> option by value (preferred) or visible text; element is a CSS selector or AX ref (@eN)",
         "inputSchema": {"type": "object", "properties": {
             "element": {"type": "string"}, "value": {"type": "string"}, "text": {"type": "string"}},
             "required": ["element"]}},
        {"name": "key", "description": "Press a named key (Enter, Tab, Escape, Backspace, ArrowDown, ...) on the focused element",
         "inputSchema": {"type": "object", "properties": {"key": {"type": "string"}}, "required": ["key"]}},
        {"name": "tabs", "description": "List, open, close, or select browser tabs (pages)",
         "inputSchema": {"type": "object", "properties": {
             "action": {"type": "string"}, "url": {"type": "string"}, "index": {"type": "integer"}},
             "required": ["action"]}},
        {"name": "close", "description": "Shut down the browser (owned process tree) or detach from an attached external engine (GLADIUS_CDP_URL; engine left running)",
         "inputSchema": {"type": "object", "properties": {}}}
    ]})
}

fn ok_text(s: String) -> Value {
    json!({"content": [{"type": "text", "text": s}]})
}

fn err_text(e: String) -> Value {
    json!({"content": [{"type": "text", "text": e}], "isError": true})
}

/// Dispatch a tool call. Always returns the MCP result object
/// (`{"content": [...]}` with optional `isError`); tool failures are NOT
/// protocol errors.
pub async fn call(name: &str, args: &Value, browser: &Browser) -> Value {
    let r: Result<Value, String> = match name {
        "navigate" => {
            let url = match args.get("url").and_then(|v| v.as_str()) {
                Some(u) => u.to_string(),
                None => return err_text("navigate: missing required arg 'url'".into()),
            };
            tool_navigate(browser, &url).await
        }
        "snapshot" => tool_snapshot(browser, args.get("mode").and_then(|v| v.as_str())).await,
        "screenshot" => {
            let full = args
                .get("fullPage")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            tool_screenshot(browser, full).await
        }
        "click" => {
            let el = match args.get("element").and_then(|v| v.as_str()) {
                Some(e) => e.to_string(),
                None => return err_text("click: missing required arg 'element'".into()),
            };
            tool_click(browser, &el).await
        }
        "type" => {
            let (el, text) = match (
                args.get("element").and_then(|v| v.as_str()),
                args.get("text").and_then(|v| v.as_str()),
            ) {
                (Some(e), Some(t)) => (e.to_string(), t.to_string()),
                _ => return err_text("type: missing required args 'element'/'text'".into()),
            };
            let submit = args
                .get("submit")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            tool_type(browser, &el, &text, submit).await
        }
        "evaluate" => {
            let expr = match args.get("expression").and_then(|v| v.as_str()) {
                Some(e) => e.to_string(),
                None => return err_text("evaluate: missing required arg 'expression'".into()),
            };
            tool_evaluate(browser, &expr).await
        }
        "wait_for" => {
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
            tool_wait_for(browser, sel.as_deref(), text.as_deref(), ms, net_idle, idle_ms).await
        }
        "console" => {
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("get")
                .to_string();
            let limit = args.get("limit").and_then(|v| v.as_i64());
            tool_console(browser, &action, limit).await
        }
        "upload" => {
            let el = match args.get("element").and_then(|v| v.as_str()) {
                Some(e) => e.to_string(),
                None => return err_text("upload: missing required arg 'element'".into()),
            };
            let paths: Option<Vec<String>> = args
                .get("paths")
                .and_then(|v| v.as_array())
                .and_then(|a| {
                    let mut out = Vec::with_capacity(a.len());
                    for x in a {
                        out.push(x.as_str()?.to_string());
                    }
                    Some(out)
                });
            let paths = match paths {
                Some(p) => p,
                None => return err_text("upload: missing required arg 'paths' (array of strings)".into()),
            };
            tool_upload(browser, &el, &paths).await
        }
        "fetch_text" => {
            let url = match args.get("url").and_then(|v| v.as_str()) {
                Some(u) => u.to_string(),
                None => return err_text("fetch_text: missing required arg 'url'".into()),
            };
            match fetch_text(&url).await {
                Ok(t) => Ok(ok_text(t)),
                Err(e) => Err(e),
            }
        }
        "hover" => {
            let el = match args.get("element").and_then(|v| v.as_str()) {
                Some(e) => e.to_string(),
                None => return err_text("hover: missing required arg 'element'".into()),
            };
            tool_hover(browser, &el).await
        }
        "select" => {
            let el = match args.get("element").and_then(|v| v.as_str()) {
                Some(e) => e.to_string(),
                None => return err_text("select: missing required arg 'element'".into()),
            };
            let value = args
                .get("value")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            tool_select(browser, &el, value.as_deref(), text.as_deref()).await
        }
        "key" => {
            let key = match args.get("key").and_then(|v| v.as_str()) {
                Some(k) => k.to_string(),
                None => return err_text("key: missing required arg 'key'".into()),
            };
            tool_key(browser, &key).await
        }
        "tabs" => {
            let action = match args.get("action").and_then(|v| v.as_str()) {
                Some(a) => a.to_string(),
                None => return err_text("tabs: missing required arg 'action'".into()),
            };
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let index = args.get("index").and_then(|v| v.as_i64());
            tool_tabs(browser, &action, url.as_deref(), index).await
        }
        "close" => match browser.close().await {
            Ok(()) => Ok(ok_text(json!({"ok": true}).to_string())),
            Err(e) => Err(e),
        },
        other => return err_text(format!("unknown tool: {other}")),
    };
    match r {
        Ok(v) => v,
        Err(e) => err_text(e),
    }
}

async fn tool_navigate(browser: &Browser, url: &str) -> Result<Value, String> {
    browser
        .command("Page.enable", json!({}), true)
        .await?;
    let nav = browser
        .command("Page.navigate", json!({"url": url}), true)
        .await?;
    if let Some(err) = nav.get("errorText").and_then(|v| v.as_str()) {
        if !err.is_empty() {
            return Err(format!("navigate: {err}"));
        }
    }
    // Wait for load (event-driven: wakes on Page.loadEventFired, 10s cap).
    browser.wait_for_load(std::time::Duration::from_secs(10)).await;
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
    // Transient blank-load race (measured on Obscura: the engine
    // double-fetches documents and the losing request can commit an EMPTY DOM
    // - empty title, empty body - at the right URL): when the committed page
    // is non-blank yet title AND body text are both empty, say so in a
    // `warning` field so the caller can see it (and re-navigate - the flake
    // is retryable). One extra eval, only on suspicious pages; no retry
    // magic here.
    if !title.is_empty() || cur.is_empty() || cur == "about:blank" {
        return Ok(ok_text(json!({"url": cur, "title": title}).to_string()));
    }
    let body_len = browser
        .eval("(((document.body && document.body.innerText) || '') + '').length")
        .await
        .ok()
        .and_then(|v| v.as_i64())
        .unwrap_or(-1);
    if body_len == 0 {
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
async fn guard_ref(browser: &Browser, tool: &str, _element: &str, id: &str) -> Result<String, String> {
    if browser.ref_label(id).await.is_none() {
        return Err(format!(
            "stale ref: {tool}: unknown ref @{id} (no such ref; take a fresh snapshot)"
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
            return Err(format!(
                "stale ref: {tool}: ref @{id} is stale (page navigated from {snap} to {cur}; take a fresh snapshot)"
            ));
        }
    }
    Ok(format!("@{id}"))
}

fn stale_gone(tool: &str, element: &str, id: &str) -> String {
    let _ = element;
    format!("stale ref: {tool}: ref @{id} is stale (node gone; take a fresh snapshot)")
}

async fn tool_snapshot(browser: &Browser, mode: Option<&str>) -> Result<Value, String> {
    if mode == Some("text") {
        return tool_snapshot_text(browser).await;
    }
    // AX-tree mode (default): enumerate actionable nodes in document order,
    // stamp each with an exact data-gladius-ref id, and report role/name/value.
    let v = browser
        .eval(
            r#"(() => {
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
      if (t === "INPUT" || t === "TEXTAREA") name = el.placeholder || el.value || el.id || el.name || el.type || "";
      else if (t === "SELECT") { try { const o = el.options[el.selectedIndex]; name = (o && o.text) || el.id || el.name || ""; } catch (e) { name = el.id || ""; } }
      else name = el.innerText || el.textContent || el.id || "";
    }
    name = (name + "").replace(/\s+/g, " ").trim().slice(0, 80);
    let value = "";
    try {
      const t = (el.tagName || "").toUpperCase();
      if (t === "INPUT" || t === "TEXTAREA" || t === "SELECT") value = String(el.value).slice(0, 60);
    } catch (e) {}
    n++;
    const ref = "e" + n;
    try { el.dataset.gladiusRef = ref; } catch (e) {}
    items.push({ref: ref, role: role, name: name, value: value,
                tag: (el.tagName || "").toLowerCase(), disabled: !!el.disabled});
  }
  return {url: location.href, title: document.title, items: items};
})()"#,
        )
        .await?;
    let empty = Vec::new();
    let items = v
        .get("items")
        .and_then(|i| i.as_array())
        .unwrap_or(&empty);
    let url = v.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string();
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
    // Page text keeps old callers (marker asserts) working on AX output.
    let tv = browser
        .eval("((((document.body && document.body.innerText) || (document.documentElement && document.documentElement.innerText) || '')) + '').slice(0, 12000)")
        .await;
    let raw = tv
        .ok()
        .and_then(|x| x.as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    let text = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let total = items.len();
    let mut lines: Vec<String> = Vec::with_capacity(total + 3);
    lines.push(format!("AX snapshot: {title} <{url}>"));
    for it in items.iter() {
        let r = it.get("ref").and_then(|x| x.as_str()).unwrap_or("?");
        let role = it.get("role").and_then(|x| x.as_str()).unwrap_or("");
        let name = it.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let tag = it.get("tag").and_then(|x| x.as_str()).unwrap_or("");
        let val = it.get("value").and_then(|x| x.as_str()).unwrap_or("");
        let dis = it.get("disabled").and_then(|x| x.as_bool()).unwrap_or(false);
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
    let mut tail = if text.is_empty() { String::new() } else { format!("\ntext: {text}") };
    loop {
        let mut footer = format!("\n[snapshot: {total} nodes, refs @e1..@e{total}]");
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
            return Ok(ok_text(full));
        }
        // Shrink order: text tail first, then whole lines off the end.
        if !tail.is_empty() {
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
            return Ok(ok_text(cut));
        }
    }
}

async fn tool_snapshot_text(browser: &Browser) -> Result<Value, String> {
    // Cap the transfer at the source: the page returns an already-sliced
    // string so huge pages never balloon memory on this side; the 8000-char
    // cap below still applies after whitespace collapsing.
    let v = browser
        .eval("((((document.body && document.body.innerText) || (document.documentElement && document.documentElement.innerText) || '')) + '').slice(0, 12000)")
        .await?;
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
    Ok(ok_text(format!("{capped}{suffix}")))
}

async fn tool_screenshot(browser: &Browser, full_page: bool) -> Result<Value, String> {
    // Capability gate: a dom-only backend has no rendering pipeline, so
    // there are no pixels to capture. Refuse with the backend and the
    // reason, never a fake or blank image. Checked before any browser
    // launch, so a dom-only refusal never spawns an engine.
    // (`upload` needs no gate: it speaks DOM CDP only -
    // DOM.enable/getDocument/querySelector/setFileInputFiles - with no
    // compositor/pixel path. Verified, not assumed.)
    match crate::browser::backend_caps() {
        Ok(crate::browser::BackendCaps::DomOnly) => {
            return Err("screenshot: unavailable with GLADIUS_BACKEND_CAPS=dom-only backend (no rendering pipeline; refusing instead of returning a fake image)".to_string());
        }
        Ok(crate::browser::BackendCaps::Render) => {}
        Err(e) => return Err(format!("screenshot: {e}")),
    }
    let params = if full_page {
        json!({"format": "png", "captureBeyondViewport": true, "fromSurface": true})
    } else {
        json!({"format": "png", "fromSurface": true})
    };
    let r = browser
        .command("Page.captureScreenshot", params, true)
        .await?;
    let data = r
        .get("data")
        .and_then(|v| v.as_str())
        .ok_or("screenshot: no data returned".to_string())?
        .to_string();
    Ok(json!({"content": [{"type": "image", "data": data, "mimeType": "image/png"}]}))
}

async fn tool_click(browser: &Browser, element: &str) -> Result<Value, String> {
    // Drain any earlier dialog record so one found below belongs to THIS click.
    browser.take_dialog().await;
    if let Some(id) = parse_ref(element) {
        let at_id = guard_ref(browser, "click", element, &id).await?;
        let rid = serde_json::to_string(&id).map_err(|e| e.to_string())?;
        let expr = format!(
            "((ref) => {{ const h0 = location.href; const el = document.querySelector('[data-gladius-ref=\"' + ref + '\"]'); if (!el) return 'GLADIUS_NOTFOUND'; const cs = getComputedStyle(el); if (cs.display === 'none' || cs.visibility === 'hidden' || cs.visibility === 'collapse') return 'GLADIUS_HIDDEN'; if (el.disabled) return 'GLADIUS_DISABLED'; try {{ el.scrollIntoView({{block: 'center'}}); }} catch (e) {{}} const r = el.getBoundingClientRect(); if (!r || r.width === 0 || r.height === 0) return 'GLADIUS_HIDDEN'; el.click(); try {{ return 'GLADIUS_OK:' + JSON.stringify({{h0: h0, h1: location.href, rs: document.readyState}}); }} catch (e) {{ return 'GLADIUS_OK'; }} }})({rid})"
        );
        let v = browser.eval(&expr).await?;
        return match v.as_str() {
            Some(s) if s.starts_with("GLADIUS_OK:") => {
                click_settle(browser, &s["GLADIUS_OK:".len()..]).await;
                Ok(click_ok(browser).await)
            }
            Some("GLADIUS_OK") => {
                click_settle(browser, "").await;
                Ok(click_ok(browser).await)
            }
            Some("GLADIUS_NOTFOUND") => Err(stale_gone("click", element, &id)),
            Some("GLADIUS_DISABLED") => Err(format!("click: {at_id} is disabled")),
            _ => Err(format!("click: {at_id} not visible or not clickable")),
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
            match tokio::time::timeout(remaining, browser.eval(&probe)).await {
                Ok(Ok(Value::Bool(true))) => break,
                _ => {}
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    let expr = format!(
        "(() => {{ const h0 = location.href; const el = document.querySelector({sel}); if (!el) return 'GLADIUS_NOTFOUND'; const cs = getComputedStyle(el); if (cs.display === 'none' || cs.visibility === 'hidden' || cs.visibility === 'collapse') return 'GLADIUS_HIDDEN'; if (el.disabled) return 'GLADIUS_DISABLED'; try {{ el.scrollIntoView({{block: 'center'}}); }} catch (e) {{}} const r = el.getBoundingClientRect(); if (!r || r.width === 0 || r.height === 0) return 'GLADIUS_HIDDEN'; el.click(); try {{ return 'GLADIUS_OK:' + JSON.stringify({{h0: h0, h1: location.href, rs: document.readyState}}); }} catch (e) {{ return 'GLADIUS_OK'; }} }})()"
    );
    let v = browser.eval(&expr).await?;
    match v.as_str() {
        Some(s) if s.starts_with("GLADIUS_OK:") => {
            click_settle(browser, &s["GLADIUS_OK:".len()..]).await;
            Ok(click_ok(browser).await)
        }
        Some("GLADIUS_OK") => {
            click_settle(browser, "").await;
            Ok(click_ok(browser).await)
        }
        Some("GLADIUS_NOTFOUND") => Err(format!("click: element not found: {element}")),
        Some("GLADIUS_DISABLED") => Err(format!("click: element is disabled: {element}")),
        _ => Err(format!(
            "click: element not visible or not clickable: {element}"
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
        browser.wait_for_load(std::time::Duration::from_secs(10)).await;
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
            browser.wait_for_load(std::time::Duration::from_secs(10)).await;
            return;
        }
        match browser.eval("[location.href, document.readyState]").await {
            Ok(v) => {
                let href = v.get(0).and_then(|x| x.as_str()).unwrap_or(h0);
                let cur = v.get(1).and_then(|x| x.as_str()).unwrap_or("complete");
                if href != h0 || cur != "complete" {
                    browser.wait_for_load(std::time::Duration::from_secs(10)).await;
                    return;
                }
            }
            _ => {}
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
        Some(d) => ok_text(
            json!({"ok": true, "dialog": {"type": d.dtype, "message": d.message, "url": d.url},
                   "note": format!("dialog auto-handled: {} {:?} (accepted, page unblocked)", d.dtype, d.message)})
                .to_string(),
        ),
        None => ok_text(json!({"ok": true}).to_string()),
    }
}

async fn tool_type(
    browser: &Browser,
    element: &str,
    text: &str,
    submit: bool,
) -> Result<Value, String> {
    if let Some(id) = parse_ref(element) {
        let at_id = guard_ref(browser, "type", element, &id).await?;
        let rid = serde_json::to_string(&id).map_err(|e| e.to_string())?;
        let txt = serde_json::to_string(text).map_err(|e| e.to_string())?;
        let expr = format!(
            "((ref, v) => {{ const el = document.querySelector('[data-gladius-ref=\"' + ref + '\"]'); if (!el) return 'GLADIUS_NOTFOUND'; if (el.disabled) return 'GLADIUS_DISABLED'; if (el.readOnly) return 'GLADIUS_READONLY'; el.focus(); if ('value' in el) {{ try {{ const p = (el instanceof HTMLTextAreaElement) ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype; const d = Object.getOwnPropertyDescriptor(p, 'value'); if (d && d.set) d.set.call(el, v); else el.value = v; }} catch (e) {{ el.value = v; }} }} else {{ el.textContent = v; }} el.dispatchEvent(new Event('input', {{bubbles: true}})); el.dispatchEvent(new Event('change', {{bubbles: true}})); return 'GLADIUS_OK'; }})({rid}, {txt})"
        );
        let v = browser.eval(&expr).await?;
        match v.as_str() {
            Some("GLADIUS_OK") => {}
            Some("GLADIUS_NOTFOUND") => return Err(stale_gone("type", element, &id)),
            Some("GLADIUS_DISABLED") => return Err(format!("type: {at_id} is disabled")),
            Some("GLADIUS_READONLY") => return Err(format!("type: {at_id} is read-only")),
            _ => return Err(stale_gone("type", element, &id)),
        }
        if submit {
            submit_enter(browser).await?;
        }
        return Ok(ok_text(json!({"ok": true}).to_string()));
    }
    let sel = serde_json::to_string(element).map_err(|e| e.to_string())?;
    let txt = serde_json::to_string(text).map_err(|e| e.to_string())?;
    let expr = format!(
        "(() => {{ const el = document.querySelector({sel}); if (!el) return 'GLADIUS_NOTFOUND'; if (el.disabled) return 'GLADIUS_DISABLED'; if (el.readOnly) return 'GLADIUS_READONLY'; el.focus(); const v = {txt}; if ('value' in el) {{ try {{ const p = (el instanceof HTMLTextAreaElement) ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype; const d = Object.getOwnPropertyDescriptor(p, 'value'); if (d && d.set) d.set.call(el, v); else el.value = v; }} catch (e) {{ el.value = v; }} }} else {{ el.textContent = v; }} el.dispatchEvent(new Event('input', {{bubbles: true}})); el.dispatchEvent(new Event('change', {{bubbles: true}})); return 'GLADIUS_OK'; }})()"
    );
    let v = browser.eval(&expr).await?;
    match v.as_str() {
        Some("GLADIUS_OK") => {}
        Some("GLADIUS_DISABLED") => {
            return Err(format!("type: element is disabled: {element}"));
        }
        Some("GLADIUS_READONLY") => {
            return Err(format!("type: element is read-only: {element}"));
        }
        _ => return Err(format!("type: element not found: {element}")),
    }
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

async fn tool_evaluate(browser: &Browser, expression: &str) -> Result<Value, String> {
    browser.take_dialog().await;
    let v = browser.eval(expression).await?;
    let mut obj = serde_json::Map::new();
    obj.insert("value".to_string(), v);
    // An evaluate that opened a dialog (e.g. `alert(1)`) was auto-accepted
    // by the reader; report it instead of returning a bare value.
    if let Some(d) = browser.last_dialog().await {
        obj.insert("dialog".to_string(), json!({"type": d.dtype, "message": d.message, "url": d.url}));
        obj.insert(
            "note".to_string(),
            json!(format!("dialog auto-handled: {} {:?} (accepted, page unblocked)", d.dtype, d.message)),
        );
    }
    Ok(ok_text(Value::Object(obj).to_string()))
}

async fn tool_console(
    browser: &Browser,
    action: &str,
    limit: Option<i64>,
) -> Result<Value, String> {
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
            Ok(ok_text(json!({"cleared": n, "cap": crate::browser::CONSOLE_CAP}).to_string()))
        }
        other => Err(format!("console: unknown action '{other}' (get|drain|clear)")),
    }
}

async fn tool_upload(
    browser: &Browser,
    element: &str,
    paths: &[String],
) -> Result<Value, String> {
    if paths.is_empty() {
        return Err("upload: 'paths' is empty".to_string());
    }
    for p in paths {
        if !std::path::Path::new(p).is_file() {
            return Err(format!("upload: file not found: {p}"));
        }
    }
    // AX refs resolve to their stamped selector so both paths share the
    // DOM lookup below; the guard gives the typed stale-ref error.
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
            None => return Err(format!("upload: element not found: {element}")),
        },
        Some("GLADIUS_DISABLED") => {
            return Err(format!("upload: element is disabled: {element}"));
        }
        _ => return Err(format!("upload: element is not a file input: {element}")),
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
        .command("DOM.querySelector", json!({"nodeId": root, "selector": sel}), true)
        .await?;
    let node = found.get("nodeId").and_then(|n| n.as_i64()).unwrap_or(0);
    if node == 0 {
        return match rid {
            Some(id) => Err(stale_gone("upload", element, &id)),
            None => Err(format!("upload: element not found: {element}")),
        };
    }
    browser
        .command("DOM.setFileInputFiles", json!({"nodeId": node, "files": paths}), true)
        .await
        .map_err(|e| format!("upload: {e}"))?;
    Ok(ok_text(json!({"ok": true, "files": paths.len()}).to_string()))
}

async fn tool_wait_for(
    browser: &Browser,
    selector: Option<&str>,
    text: Option<&str>,
    ms: Option<i64>,
    network_idle: bool,
    idle_ms: Option<i64>,
) -> Result<Value, String> {
    let budget_ms: i64 = ms.unwrap_or(5000).clamp(0, 15000);
    let budget = std::time::Duration::from_millis(budget_ms.max(0) as u64);
    // ms:0 is a no-op: return immediately without any CDP round-trip
    // (unchanged legacy path; a zero-budget network-idle wait below still
    // runs its one immediate quiet check).
    if budget_ms == 0 && !network_idle {
        return Ok(ok_text(json!({"ok": true}).to_string()));
    }
    // HARD deadline shared by both phases below: checked before AND after
    // every await, so the total can never exceed `ms` no matter how slow
    // one step is. Phase 1 (selector/text) eats into phase 2's
    // (network-idle) budget by construction.
    let deadline = std::time::Instant::now() + budget;
    if selector.is_some() || text.is_some() {
        let sel_expr = selector.map(|s| {
            let lit = serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string());
            format!("!!document.querySelector({lit})")
        });
        let text_lit = text.map(|t| serde_json::to_string(t).unwrap_or_else(|_| "\"\"".to_string()));
        // Each per-await CDP evaluation is clipped to the remaining budget.
        loop {
            let mut done = true;
            if let Some(ref e) = sel_expr {
                if std::time::Instant::now() >= deadline {
                    return Err("wait_for: timeout".to_string());
                }
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                match tokio::time::timeout(remaining, browser.eval(e)).await {
                    Ok(Ok(Value::Bool(true))) => {}
                    _ => done = false,
                }
                if std::time::Instant::now() >= deadline {
                    return Err("wait_for: timeout".to_string());
                }
            }
            if done {
                if let Some(ref lit) = text_lit {
                    if std::time::Instant::now() >= deadline {
                        return Err("wait_for: timeout".to_string());
                    }
                    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                    let e =
                        format!("((document.body && document.body.innerText) || '').includes({lit})");
                    match tokio::time::timeout(remaining, browser.eval(&e)).await {
                        Ok(Ok(Value::Bool(true))) => {}
                        _ => done = false,
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err("wait_for: timeout".to_string());
                    }
                }
            }
            if done {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err("wait_for: timeout".to_string());
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            tokio::time::sleep(remaining.min(std::time::Duration::from_millis(100))).await;
        }
    } else if !network_idle {
        // Legacy bare delay: no condition, just sleep the budget.
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        tokio::time::sleep(remaining).await;
        return Ok(ok_text(json!({"ok": true}).to_string()));
    }
    if network_idle {
        // Quiet-network window on the REMAINING budget (zero when phase 1
        // ate it all): drains in-flight requests, then requires `idle`
        // continuous quiet. Never hangs - expiry is the typed timeout.
        let idle_win =
            std::time::Duration::from_millis(idle_ms.unwrap_or(500).clamp(0, 5000).max(0) as u64);
        browser.ensure_network().await?;
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if !browser.wait_network_idle(idle_win, remaining).await {
            return Err("wait_for: timeout".to_string());
        }
    }
    Ok(ok_text(json!({"ok": true}).to_string()))
}

async fn tool_hover(browser: &Browser, element: &str) -> Result<Value, String> {
    if let Some(id) = parse_ref(element) {
        let at_id = guard_ref(browser, "hover", element, &id).await?;
        let rid = serde_json::to_string(&id).map_err(|e| e.to_string())?;
        let expr = format!(
            "((ref) => {{ const el = document.querySelector('[data-gladius-ref=\"' + ref + '\"]'); if (!el) return 'GLADIUS_NOTFOUND'; const cs = getComputedStyle(el); if (cs.display === 'none' || cs.visibility === 'hidden' || cs.visibility === 'collapse') return 'GLADIUS_HIDDEN'; try {{ el.scrollIntoView({{block: 'center', inline: 'center'}}); }} catch (e) {{}} const r = el.getBoundingClientRect(); if (!r || r.width === 0 || r.height === 0) return 'GLADIUS_HIDDEN'; return JSON.stringify({{x: r.left + r.width / 2, y: r.top + r.height / 2}}); }})({rid})"
        );
        let v = browser.eval(&expr).await?;
        if v.as_str() == Some("GLADIUS_NOTFOUND") {
            return Err(stale_gone("hover", element, &id));
        }
        if v.as_str() == Some("GLADIUS_HIDDEN") {
            return Err(format!("hover: {at_id} not visible or not hoverable"));
        }
        return hover_move(browser, &v).await;
    }
    let sel = serde_json::to_string(element).map_err(|e| e.to_string())?;
    let expr = format!(
        "(() => {{ const el = document.querySelector({sel}); if (!el) return 'GLADIUS_NOTFOUND'; const cs = getComputedStyle(el); if (cs.display === 'none' || cs.visibility === 'hidden' || cs.visibility === 'collapse') return 'GLADIUS_HIDDEN'; try {{ el.scrollIntoView({{block: 'center', inline: 'center'}}); }} catch (e) {{}} const r = el.getBoundingClientRect(); if (!r || r.width === 0 || r.height === 0) return 'GLADIUS_HIDDEN'; return JSON.stringify({{x: r.left + r.width / 2, y: r.top + r.height / 2}}); }})()"
    );
    let v = browser.eval(&expr).await?;
    if v.as_str() == Some("GLADIUS_NOTFOUND") {
        return Err(format!("hover: element not found: {element}"));
    }
    if v.as_str() == Some("GLADIUS_HIDDEN") {
        return Err(format!(
            "hover: element not visible or not hoverable: {element}"
        ));
    }
    hover_move(browser, &v).await
}

async fn hover_move(browser: &Browser, v: &Value) -> Result<Value, String> {
    let s = v.as_str().unwrap_or("").to_string();
    let pt: Value =
        serde_json::from_str(&s).map_err(|_| "hover: no layout box".to_string())?;
    let x = pt.get("x").and_then(|n| n.as_f64()).unwrap_or(0.0);
    let y = pt.get("y").and_then(|n| n.as_f64()).unwrap_or(0.0);
    browser
        .command(
            "Input.dispatchMouseEvent",
            json!({"type": "mouseMoved", "x": x, "y": y}),
            true,
        )
        .await?;
    Ok(ok_text(json!({"ok": true}).to_string()))
}

async fn tool_select(
    browser: &Browser,
    element: &str,
    value: Option<&str>,
    text: Option<&str>,
) -> Result<Value, String> {
    if value.is_none() && text.is_none() {
        return Err("select: need 'value' or 'text'".to_string());
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
            Some("GLADIUS_NOTSELECT") => Err(format!("select: @{id} is not a <select>")),
            _ => {
                let want = value.or(text).unwrap_or("");
                Err(format!("select: option not found: {want}"))
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
        Some("GLADIUS_NOTFOUND") => Err(format!("select: element not found: {element}")),
        Some("GLADIUS_NOTSELECT") => {
            Err(format!("select: element is not a <select>: {element}"))
        }
        _ => {
            let want = value.or(text).unwrap_or("");
            Err(format!("select: option not found: {want}"))
        }
    }
}

/// Map a human key name to (key, code, vk, text-for-keyDown).
fn key_map(key: &str) -> (String, String, u32, Option<String>) {
    let lower = key.to_lowercase();
    match lower.as_str() {
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
                        return (name.clone(), name, 111 + n, None);
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
                return (c.to_string(), code, up as u32, Some(c.to_string()));
            }
            // Unknown multi-char name: best-effort passthrough.
            (key.to_string(), key.to_string(), 0, None)
        }
    }
}

async fn tool_key(browser: &Browser, key: &str) -> Result<Value, String> {
    if key.is_empty() {
        return Err("key: missing required arg 'key'".to_string());
    }
    let (k, code, vk, text) = key_map(key);
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

async fn tool_tabs(
    browser: &Browser,
    action: &str,
    url: Option<&str>,
    index: Option<i64>,
) -> Result<Value, String> {
    let to_usize = |i: Option<i64>| -> Result<usize, String> {
        match i {
            Some(n) if n >= 0 => Ok(n as usize),
            Some(n) => Err(format!("tabs: bad index {n}")),
            None => Err("tabs: missing required arg 'index'".to_string()),
        }
    };
    let inner = match action {
        "list" => browser.tabs_list().await?,
        "new" => browser.tabs_new(url).await?,
        "select" => browser.tabs_select(to_usize(index)?).await?,
        "close" => browser.tabs_close(to_usize(index)?).await?,
        other => return Err(format!("tabs: unknown action '{other}'")),
    };
    Ok(ok_text(inner.to_string()))
}
