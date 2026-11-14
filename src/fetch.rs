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
    // A comment is its own lexical unit: nothing inside it is a tag, and the
    // `>` rule below must never fire there. Its terminator is a state
    // machine, not a dash count: `-->` needs both dashes, `--!>` needs the
    // bang and only one of them, and only an empty comment closes abruptly
    // (`<!-->`, `<!--->`). The states are the HTML tokenizer's
    // comment-start, comment-start-dash, comment, comment-end-dash,
    // comment-end and comment-end-bang states with everything that is not a
    // terminator collapsed into `Data` (a comment appends its text anyway,
    // and appended text is never emitted). `Done` is written once, then the
    // comment is left on the next line, so it is never read back.
    #[derive(Clone, Copy, PartialEq)]
    enum Comment {
        Start,
        StartDash,
        Data,
        EndDash,
        End,
        EndBang,
        Done,
    }
    let mut comment = Comment::Start;
    let mut in_comment = false;
    let mut skip: Option<&str> = None; // "script" | "style"
    for (pos, c) in html.char_indices() {
        if let Some(raw_tag) = skip {
            if !in_tag {
                // In raw text, '<' is ordinary data unless it starts the
                // matching end tag. In particular, comparisons must not
                // consume the real closing tag as part of a fake tag.
                let rest = &html[pos..];
                let end = 2 + raw_tag.len();
                let matches_end = rest.starts_with("</")
                    && rest
                        .get(2..end)
                        .is_some_and(|name| name.eq_ignore_ascii_case(raw_tag))
                    && rest
                        .get(end..)
                        .and_then(|tail| tail.chars().next())
                        .is_some_and(|ch| ch.is_ascii_whitespace() || ch == '/' || ch == '>');
                if !matches_end {
                    continue;
                }
            }
        }
        if in_comment {
            comment = match (comment, c) {
                (Comment::Start, '>') => Comment::Done, // abrupt close, `<!-->`
                (Comment::Start, '-') => Comment::StartDash,
                (Comment::Start, _) => Comment::Data,
                (Comment::StartDash, '>') => Comment::Done, // abrupt close, `<!--->`
                (Comment::StartDash, '-') => Comment::End,
                (Comment::StartDash, _) => Comment::Data,
                (Comment::Data, '-') => Comment::EndDash,
                (Comment::Data, _) => Comment::Data,
                (Comment::EndDash, '-') => Comment::End,
                // A single `-` is data: reconsume it in the comment state.
                (Comment::EndDash, _) => Comment::Data,
                (Comment::End, '>') => Comment::Done, // `-->`
                (Comment::End, '!') => Comment::EndBang,
                (Comment::End, '-') => Comment::End,
                (Comment::End, _) => Comment::Data,
                (Comment::EndBang, '>') => Comment::Done, // `--!>`
                (Comment::EndBang, '-') => Comment::EndDash,
                (Comment::EndBang, _) => Comment::Data,
                (Comment::Done, _) => Comment::Done,
            };
            if comment == Comment::Done {
                in_comment = false;
            }
            continue;
        }
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
                if skip.is_some() {
                    if closing && skip == Some(name.as_str()) {
                        skip = None;
                    }
                } else if !closing && (name == "script" || name == "style") {
                    skip = Some(if name == "script" { "script" } else { "style" });
                } else {
                    out.push(' ');
                }
                tag_buf.clear();
            } else {
                tag_buf.push(c);
                // `<!--` is a comment, not a tag that a bare `>` closes.
                // Only reached outside raw text: in script/style, `<` is data
                // and never sets `in_tag`, so `<!--` there stays script data.
                if tag_buf == "!--" {
                    in_tag = false;
                    tag_buf.clear();
                    in_comment = true;
                    comment = Comment::Start;
                }
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

    #[test]
    fn script_and_style_end_only_at_their_matching_closing_tag() {
        assert_eq!(
            html_to_text("<script>const x = '</style>'; secret</script><p>visible</p>"),
            "visible"
        );
        assert_eq!(
            html_to_text("<style>/* </script> secret */</style><p>visible</p>"),
            "visible"
        );
    }

    #[test]
    fn less_than_in_raw_text_does_not_consume_the_closing_tag() {
        assert_eq!(
            html_to_text("<script>if (a < b) x++;</script><p>visible</p>"),
            "visible"
        );
        assert_eq!(
            html_to_text("<style>/* a < b */</STYLE><p>visible</p>"),
            "visible"
        );
        assert_eq!(
            html_to_text("<script>ignore </scripture> secret</ScRiPt ><p>visible</p>"),
            "visible"
        );
    }

    /// A comment is one lexical unit: nothing inside it is a tag, and the
    /// `-->` that ends it is not text. The bare `>` rule must never fire
    /// inside one. Pre-fix the scanner re-entered data mode at the first
    /// `>` of the body and emitted the rest of the comment.
    #[test]
    fn html_comments_are_dropped_whole() {
        assert_eq!(html_to_text("<!-- secret --><p>visible</p>"), "visible");
        assert_eq!(
            html_to_text("<!-- secret > still hidden --><p>visible</p>"),
            "visible"
        );
        assert_eq!(
            html_to_text("<!-- <p>hidden</p> --><p>visible</p>"),
            "visible"
        );
        assert_eq!(html_to_text("<div>a<!-- <p>b</p> -->c</div>"), "ac");
    }

    /// An unterminated comment swallows the rest of the input: there is no
    /// text after an open comment, so there is nothing to emit.
    #[test]
    fn unclosed_html_comment_runs_to_end_of_input() {
        assert_eq!(html_to_text("<!-- secret > still hidden"), "");
        assert_eq!(html_to_text("<p>visible</p><!-- secret > still"), "visible");
    }

    /// `-->` and `--!>` both end a comment (HTML comment-end and
    /// comment-end-bang states), and `<!-->` and `<!--->` are empty comments
    /// that the `>` closes at once - neither may swallow the document.
    #[test]
    fn comment_terminators_match_the_html_tokenizer() {
        assert_eq!(html_to_text("<!-- secret > --!><p>visible</p>"), "visible");
        assert_eq!(html_to_text("<!--><p>visible</p>"), "visible");
        assert_eq!(html_to_text("<!---><p>visible</p>"), "visible");
        assert_eq!(html_to_text("<!----><p>visible</p>"), "visible");
        // A dash run that is not followed by `>` keeps the comment open.
        assert_eq!(html_to_text("<!-- a -- b --><p>visible</p>"), "visible");
    }

    /// Only an *empty* comment closes abruptly: `<!-- x ->` is comment data
    /// (comment end dash state re-enters the comment state), so what looks
    /// like a terminator must not end the comment early.
    #[test]
    fn a_single_dash_is_not_a_comment_terminator() {
        assert_eq!(html_to_text("<!-- x -><p>visible</p> --> tail"), "tail");
        assert_eq!(html_to_text("<!-- x - --><p>visible</p>"), "visible");
    }

    /// The bang is part of the terminator, and only one is: `--!>` closes,
    /// `--!!>` puts the second `!` back into comment data and runs on to the
    /// real `-->`, so the body may not leak.
    #[test]
    fn comment_end_bang_takes_exactly_one_bang() {
        assert_eq!(
            html_to_text("<!-- x --!!>hidden --><p>visible</p>"),
            "visible"
        );
        assert_eq!(
            html_to_text("<!-- x --!->hidden --><p>visible</p>"),
            "visible"
        );
    }

    /// Raw text wins over comment syntax: `<!--` inside script or style is
    /// script data, not the start of a comment, and the raw text still ends
    /// only at its matching end tag.
    #[test]
    fn comment_markers_in_raw_text_are_not_comments() {
        assert_eq!(
            html_to_text("<script>var a = \"<!--\";</script><p>visible</p>"),
            "visible"
        );
        assert_eq!(
            html_to_text("<style>/* <!-- */</style><p>visible</p>"),
            "visible"
        );
        assert_eq!(
            html_to_text("<script>if (a-->b) c();</script><p>visible</p>"),
            "visible"
        );
    }

    /// The converse: a script or style tag inside a comment is comment text,
    /// so it may not put the scanner into raw-text mode and eat what follows
    /// the comment.
    #[test]
    fn tags_inside_comments_are_comment_text() {
        assert_eq!(html_to_text("<!-- <script> --><p>visible</p>"), "visible");
        assert_eq!(html_to_text("<!-- <style> --><p>visible</p>"), "visible");
    }

    /// Regression pair: `>` as ordinary data text is not a tag end, and a
    /// comment with no `>` in it was already dropped.
    #[test]
    fn comment_fix_leaves_data_text_alone() {
        assert_eq!(html_to_text("<p>a > b</p>"), "a > b");
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
