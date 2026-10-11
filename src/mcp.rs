use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::browser::Browser;
use crate::tools;

/// Serve MCP over stdio: newline-delimited JSON-RPC 2.0.
/// STDOUT carries only protocol frames; ALL logs go to STDERR.
pub async fn serve() -> Result<(), String> {
    let browser = Arc::new(Browser::new());
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut out = tokio::io::stdout();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|e| format!("stdin: {e}"))?
    {
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                write_frame(
                    &mut out,
                    json!({"jsonrpc": "2.0", "id": null,
                           "error": {"code": -32700, "message": "parse error"}}),
                )
                .await?;
                continue;
            }
        };
        if let Some(resp) = handle(&req, &browser).await {
            write_frame(&mut out, resp).await?;
        }
    }
    // EOF: make sure no browser process is left behind.
    let _ = browser.close().await;
    Ok(())
}

async fn handle(req: &Value, browser: &Arc<Browser>) -> Option<Value> {
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    // JSON-RPC notifications (no `id`) get NO response, whatever the method.
    if req.get("id").is_none() {
        return None;
    }
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => {
            let ver = req
                .get("params")
                .and_then(|p| p.get("protocolVersion"))
                .and_then(|v| v.as_str())
                .unwrap_or("2025-06-18")
                .to_string();
            let info = crate::browser::backend_info();
            let mut server_info = serde_json::Map::new();
            server_info.insert("name".to_string(), json!("gladius"));
            server_info.insert("version".to_string(), json!(env!("CARGO_PKG_VERSION")));
            if let Some(b) = info.get("backend") {
                server_info.insert("backend".to_string(), b.clone());
            }
            if let Some(caps) = info.get("caps") {
                server_info.insert("caps".to_string(), caps.clone());
            }
            if let Some(url) = info.get("cdpUrl") {
                server_info.insert("cdpUrl".to_string(), url.clone());
            }
            Some(json!({"jsonrpc": "2.0", "id": id, "result": {
                "protocolVersion": ver,
                "capabilities": {"tools": {}},
                "serverInfo": server_info
            }}))
        }
        "notifications/initialized" => None, // notification: no reply
        "ping" => Some(json!({"jsonrpc": "2.0", "id": id, "result": {}})),
        "tools/list" => Some(json!({"jsonrpc": "2.0", "id": id, "result": crate::tools::list()})),
        "tools/call" => {
            let params = req.get("params").cloned().unwrap_or(json!({}));
            let name = params
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            if name.is_empty() {
                return Some(json!({"jsonrpc": "2.0", "id": id,
                    "error": {"code": -32602, "message": "missing tool name"}}));
            }
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let result = tools::call(&name, &args, browser).await;
            Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
        }
        _ => Some(json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": "method not found"}})),
    }
}

async fn write_frame(out: &mut tokio::io::Stdout, v: Value) -> Result<(), String> {
    let mut s = v.to_string();
    s.push('\n');
    out.write_all(s.as_bytes())
        .await
        .map_err(|e| format!("stdout: {e}"))?;
    out.flush().await.map_err(|e| format!("stdout: {e}"))?;
    Ok(())
}
