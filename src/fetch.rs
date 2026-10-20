use std::time::Duration;

/// Byte budget for the fast path: only the head of the body is stripped to
/// text. The old code read the whole body (`resp.text()`) and `html_to_text`
/// then held roughly 2x of it in `out`, so a huge page meant huge RSS while
/// the 8000-char cap applied only to the final string. 256 KB is plenty for
/// stripped text.
const MAX_FETCH_BYTES: usize = 256 * 1024;

/// Plain HTTP GET that uses NO browser (the fast path).
pub async fn fetch_text(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("gladius/0.1")
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("http get: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("http {status}"));
    }
    // Stream with a byte budget: pre-size from the declared length, then take
    // at most the first MAX_FETCH_BYTES. Dropping `resp` after the budget
    // closes the connection, so a gigabyte body never lands in memory.
    let hint = resp
        .content_length()
        .unwrap_or(0)
        .min(MAX_FETCH_BYTES as u64) as usize;
    let mut buf = Vec::with_capacity(hint);
    while buf.len() < MAX_FETCH_BYTES {
        match resp.chunk().await.map_err(|e| format!("http body: {e}"))? {
            Some(c) => {
                let room = MAX_FETCH_BYTES - buf.len();
                if c.len() <= room {
                    buf.extend_from_slice(&c);
                } else {
                    buf.extend_from_slice(&c[..room]);
                    break;
                }
            }
            None => break,
        }
    }
    Ok(html_to_text(&String::from_utf8_lossy(&buf)))
}

fn html_to_text(html: &str) -> String {
    // Strip tags; drop script/style contents; decode common entities.
    let mut out = String::with_capacity(html.len().min(32768));
    let mut in_tag = false;
    let mut tag_buf = String::new();
    let mut skip: Option<&str> = None; // "script" | "style"
    for c in html.chars() {
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
    // Entity order matters: `&amp;` is decoded LAST so each entity decodes
    // exactly once. Decoding it first double-decodes: `&amp;lt;` (which
    // renders as the literal text `&lt;`) would come back as `<`.
    let s = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > 8000 {
        collapsed.chars().take(8000).collect()
    } else {
        collapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// F3 proof: each entity decodes exactly once. `&amp;lt;` renders as the
    /// literal text `&lt;`, so it must come back as `&lt;` - not `<`.
    /// Pre-fix (`&amp;` replaced first) the first assert yields `<tag>`.
    #[test]
    fn entities_decode_exactly_once() {
        assert_eq!(html_to_text("<p>&amp;lt;tag&amp;gt;</p>"), "&lt;tag&gt;");
        assert_eq!(html_to_text("<p>&lt;tag&gt;</p>"), "<tag>");
        assert_eq!(html_to_text("<p>&amp;amp;</p>"), "&amp;");
        assert_eq!(html_to_text("<p>a&nbsp;b &quot;q&quot;</p>"), "a b \"q\"");
    }

    /// F2 proof: an 8 MB loopback body must NOT be pulled in full. The server
    /// counts body bytes actually written; the client stops at its byte
    /// budget and drops the connection, so `sent < len`. Pre-fix
    /// (`resp.text()`) the server sends everything and this fails.
    #[tokio::test]
    async fn fetch_streams_within_byte_budget() {
        let prefix = "<html><body><p>FETCH_HEAD_MARKER_8m</p>";
        let body_len: usize = 8 * 1024 * 1024;
        assert!(body_len > prefix.len(), "body dwarfs the prefix");
        let ln = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback bind");
        let port = ln.local_addr().expect("loopback addr").port();
        let srv = tokio::spawn(async move {
            let (mut s, _) = ln.accept().await.expect("loopback accept");
            // Consume the request head.
            let mut buf = vec![0u8; 4096];
            let mut req = Vec::new();
            loop {
                match s.read(&mut buf).await {
                    Ok(0) => return 0usize,
                    Ok(n) => {
                        req.extend_from_slice(&buf[..n]);
                        if req.windows(4).any(|w| w == b"\r\n\r\n") || req.len() > 8192 {
                            break;
                        }
                    }
                    Err(_) => return 0usize,
                }
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {body_len}\r\nConnection: close\r\nContent-Type: text/html\r\n\r\n{prefix}"
            );
            if s.write_all(head.as_bytes()).await.is_err() {
                return 0usize;
            }
            let mut sent = prefix.len(); // body bytes on the wire so far
            let chunk = vec![b'x'; 65536];
            while sent < body_len {
                let want = (body_len - sent).min(chunk.len());
                match s.write_all(&chunk[..want]).await {
                    Ok(()) => sent += want,
                    Err(_) => break, // client went away: budget proved
                }
            }
            sent
        });
        let url = format!("http://127.0.0.1:{port}/big");
        let t0 = std::time::Instant::now();
        let text = fetch_text(&url).await.expect("bounded fetch ok");
        let dt = t0.elapsed();
        assert!(
            text.contains("FETCH_HEAD_MARKER_8m"),
            "head of body is stripped"
        );
        assert!(
            text.chars().count() <= 8000,
            "final cap still holds: {} chars",
            text.chars().count()
        );
        let sent = srv.await.expect("server task");
        assert!(
            sent < body_len / 2,
            "client stopped early: server wrote {sent}/{body_len} body bytes"
        );
        assert!(
            dt < Duration::from_secs(15),
            "bounded fetch is fast: {dt:?}"
        );
    }
}
