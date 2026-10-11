use std::time::Duration;

/// Plain HTTP GET that uses NO browser (the fast path).
pub async fn fetch_text(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("gladius/0.1")
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("http get: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("http {status}"));
    }
    let html = resp.text().await.map_err(|e| format!("http body: {e}"))?;
    Ok(html_to_text(&html))
}

fn html_to_text(html: &str) -> String {
    // Strip tags; drop script/style contents; decode common entities.
    let mut out = String::with_capacity(html.len().min(32768));
    let mut in_tag = false;
    let mut tag_buf = String::new();
    let mut skip: Option<&str> = None; // "script" | "style"
    let mut chars = html.chars();
    while let Some(c) = chars.next() {
        if in_tag {
            if c == '>' {
                in_tag = false;
                let t = tag_buf.trim();
                let closing = t.starts_with('/');
                let name = t
                    .trim_start_matches('/')
                    .split(|ch: char| ch.is_whitespace() || ch == '/')
                    .next()
                    .unwrap_or("")
                    .to_lowercase();
                if !closing && (name == "script" || name == "style") {
                    skip = Some(if name == "script" { "script" } else { "style" });
                } else if closing && (name == "script" || name == "style") {
                    skip = None;
                } else if skip.is_none() {
                    out.push(' ');
                }
                tag_buf.clear();
            } else {
                tag_buf.push(c);
            }
            continue;
        }
        if c == '<' {
            in_tag = true;
            tag_buf.clear();
            continue;
        }
        if skip.is_some() {
            continue;
        }
        out.push(c);
    }
    let s = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'");
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > 8000 {
        collapsed.chars().take(8000).collect()
    } else {
        collapsed
    }
}
