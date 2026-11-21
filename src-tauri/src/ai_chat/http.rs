//! Provider HTTP client: request building, SSE parsing, event mapping, errors, redaction.
use super::attachments::{anthropic_parts, openai_parts};
use super::settings::{ApiKind, Header};
use super::store::Turn;
use regex::Regex;
use serde_json::{json, Value};
use std::{sync::OnceLock, time::Duration};
use zeroize::Zeroizing;

const LINE_LIMIT: usize = 1024 * 1024;
const EVENT_LIMIT: usize = 4 * 1024 * 1024;
const ERROR_BODY_LIMIT: usize = 64 * 1024;
const PROBE_BODY_LIMIT: usize = 4 * 1024 * 1024;
pub(crate) const ANTHROPIC_VERSION: &str = "2023-06-01";

pub(crate) fn client() -> Result<reqwest::Client, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(120))
        .user_agent(concat!("Lomi/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "Cannot create the HTTP client.".to_string())
}

// ---- redaction -------------------------------------------------------

fn secret_patterns() -> &'static [Regex; 5] {
    static P: OnceLock<[Regex; 5]> = OnceLock::new();
    P.get_or_init(|| {
        [
            Regex::new(r"sk-[A-Za-z0-9_-]{8,}").expect("static pattern"),
            Regex::new(r"Bearer\s+\S+").expect("static pattern"),
            // JWT-like tokens.
            Regex::new(r"eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+")
                .expect("static pattern"),
            Regex::new(r"(?:gh[po]_[A-Za-z0-9]{10,}|github_pat_[A-Za-z0-9_]{10,})")
                .expect("static pattern"),
            // Credentials in URL queries or log text; the name is dropped with the value.
            Regex::new(r#"(?i)(?:access_token|api_key|token|key)=[^\s&"']+"#)
                .expect("static pattern"),
        ]
    })
}

/// Makes provider-supplied text safe to show: removes the key and key-shaped
/// tokens, strips control characters and truncates to 2 KiB.
pub(crate) fn redact(text: &str, key: Option<&str>) -> String {
    let mut out = text.to_string();
    if let Some(key) = key.filter(|k| !k.is_empty()) {
        out = out.replace(key, "[redacted]");
    }
    for re in secret_patterns() {
        out = re.replace_all(&out, "[redacted]").into_owned();
    }
    let mut out: String = out
        .chars()
        .filter_map(|c| match c {
            '\n' | '\r' | '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect();
    if out.len() > 2048 {
        let mut end = 2048;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
    }
    out.trim().to_string()
}

// ---- SSE -------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Default)]
pub(crate) struct SseParser {
    buf: Vec<u8>,
    event: Option<String>,
    data: Option<String>,
    bom_checked: bool,
}

impl SseParser {
    /// Lines end with LF, CRLF or a bare CR. A CR at the end of the buffer
    /// waits for the next byte, which may be its LF.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseEvent>, String> {
        self.buf.extend_from_slice(bytes);
        if !self.bom_checked {
            const BOM: &[u8] = b"\xEF\xBB\xBF";
            if self.buf.len() < BOM.len() && BOM.starts_with(&self.buf) {
                return Ok(Vec::new());
            }
            if self.buf.starts_with(BOM) {
                self.buf.drain(..BOM.len());
            }
            self.bom_checked = true;
        }
        let mut out = Vec::new();
        let mut start = 0;
        while let Some(pos) = self.buf[start..]
            .iter()
            .position(|&b| b == b'\n' || b == b'\r')
        {
            let end = start + pos;
            let mut next = end + 1;
            if self.buf[end] == b'\r' {
                match self.buf.get(next) {
                    None => break,
                    Some(b'\n') => next += 1,
                    Some(_) => {}
                }
            }
            let line = &self.buf[start..end];
            if line.len() > LINE_LIMIT {
                return Err("Unexpected provider response.".into());
            }
            let line = String::from_utf8_lossy(line).into_owned();
            start = next;
            Self::line(&mut self.event, &mut self.data, &line, &mut out)?;
        }
        self.buf.drain(..start);
        if self.buf.len() > LINE_LIMIT {
            return Err("Unexpected provider response.".into());
        }
        Ok(out)
    }

    /// Dispatches a pending event whose blank line never arrived; a partial
    /// trailing line is discarded.
    pub(crate) fn finish(&mut self) -> Option<SseEvent> {
        if self.buf.last() == Some(&b'\r') {
            if let Some(event) = self.push(b"\n").ok().and_then(|e| e.into_iter().next()) {
                self.buf.clear();
                return Some(event);
            }
        }
        self.buf.clear();
        let data = self.data.take()?;
        Some(SseEvent {
            event: self.event.take(),
            data,
        })
    }

    fn line(
        event: &mut Option<String>,
        data: &mut Option<String>,
        line: &str,
        out: &mut Vec<SseEvent>,
    ) -> Result<(), String> {
        if line.is_empty() {
            if let Some(d) = data.take() {
                out.push(SseEvent {
                    event: event.take(),
                    data: d,
                });
            }
            *event = None;
            return Ok(());
        }
        if line.starts_with(':') {
            return Ok(());
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => *event = Some(value.to_string()),
            "data" => match data {
                Some(d) => {
                    if d.len() + value.len() + 1 > EVENT_LIMIT {
                        return Err("Unexpected provider response.".into());
                    }
                    d.push('\n');
                    d.push_str(value);
                }
                None => *data = Some(value.to_string()),
            },
            _ => {}
        }
        Ok(())
    }
}

// ---- event mapping ---------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Mapped {
    Text(String),
    Stop(String),
    Done,
    Error(String),
}

fn error_message(value: &Value) -> String {
    value
        .get("error")
        .and_then(|e| e.get("message").and_then(Value::as_str).or(e.as_str()))
        .unwrap_or("The provider reported an error.")
        .to_string()
}

pub(crate) fn map_event(kind: ApiKind, ev: &SseEvent) -> Vec<Mapped> {
    let mut out = Vec::new();
    if kind != ApiKind::Anthropic && ev.data.trim() == "[DONE]" {
        return vec![Mapped::Done];
    }
    let Ok(v) = serde_json::from_str::<Value>(&ev.data) else {
        return out;
    };
    if kind == ApiKind::Anthropic {
        match v.get("type").and_then(Value::as_str) {
            Some("content_block_delta") => {
                if v["delta"]["type"] == "text_delta" {
                    if let Some(t) = v["delta"]["text"].as_str().filter(|t| !t.is_empty()) {
                        out.push(Mapped::Text(t.into()));
                    }
                }
            }
            Some("message_delta") => {
                if let Some(r) = v["delta"]["stop_reason"].as_str() {
                    out.push(Mapped::Stop(r.into()));
                }
            }
            Some("message_stop") => out.push(Mapped::Done),
            Some("error") => out.push(Mapped::Error(error_message(&v))),
            _ => {}
        }
        return out;
    }
    if v.get("error").is_some_and(|e| !e.is_null()) {
        return vec![Mapped::Error(error_message(&v))];
    }
    let choice = &v["choices"][0];
    if let Some(t) = choice["delta"]["content"]
        .as_str()
        .filter(|t| !t.is_empty())
    {
        out.push(Mapped::Text(t.into()));
    }
    if let Some(r) = choice["finish_reason"].as_str() {
        out.push(Mapped::Stop(r.into()));
    }
    out
}

// ---- request building ------------------------------------------------

pub(crate) struct ChatRequest<'a> {
    pub kind: ApiKind,
    pub base_url: &'a str,
    pub headers: &'a [Header],
    pub key: Option<&'a str>,
    pub model: &'a str,
    pub system: &'a str,
    pub max_tokens: Option<u32>,
    /// (role, content, attachments) with role "user" or "assistant".
    pub messages: &'a [Turn],
    /// Folder holding the attachment files.
    pub attachments_dir: &'a std::path::Path,
}

/// Anthropic rejects consecutive same-role turns.
fn merge_turns(messages: &[Turn]) -> Vec<Turn> {
    let mut out: Vec<Turn> = Vec::new();
    for (role, text, files) in messages {
        match out.last_mut() {
            Some((r, t, f)) if r == role => {
                t.push_str("\n\n");
                t.push_str(text);
                f.extend(files.iter().cloned());
            }
            _ => out.push((role, text.clone(), files.clone())),
        }
    }
    out
}

/// Fails before any network access when an attachment cannot be sent.
pub(crate) fn build_body(r: &ChatRequest) -> Result<Value, String> {
    let dir = r.attachments_dir;
    if r.kind == ApiKind::Anthropic {
        let mut messages = Vec::new();
        for (role, content, files) in merge_turns(r.messages) {
            let content = if files.is_empty() {
                json!(content)
            } else {
                anthropic_parts(&content, &files, dir)?
            };
            messages.push(json!({"role": role, "content": content}));
        }
        let mut body = json!({
            "model": r.model,
            "max_tokens": r.max_tokens.unwrap_or(8192),
            "messages": messages,
            "stream": true,
        });
        if !r.system.is_empty() {
            body["system"] = json!(r.system);
        }
        return Ok(body);
    }
    let mut messages = Vec::new();
    if !r.system.is_empty() {
        messages.push(json!({"role": "system", "content": r.system}));
    }
    for (role, content, files) in r.messages {
        let content = if files.is_empty() {
            json!(content)
        } else {
            openai_parts(content, files, dir, r.kind == ApiKind::Openai)?
        };
        messages.push(json!({"role": role, "content": content}));
    }
    let mut body = json!({"model": r.model, "stream": true, "messages": messages});
    if let Some(n) = r.max_tokens {
        let field = if r.kind == ApiKind::Openai {
            "max_completion_tokens"
        } else {
            "max_tokens"
        };
        body[field] = json!(n);
    }
    Ok(body)
}

fn headers(
    kind: ApiKind,
    custom: &[Header],
    key: Option<&str>,
) -> Result<reqwest::header::HeaderMap, String> {
    use reqwest::header::{HeaderName, HeaderValue, AUTHORIZATION};
    let bad = || "Invalid request header.".to_string();
    let mut map = reqwest::header::HeaderMap::new();
    for h in custom {
        let name = HeaderName::from_bytes(h.name.as_bytes()).map_err(|_| bad())?;
        map.insert(name, HeaderValue::from_str(&h.value).map_err(|_| bad())?);
    }
    if let Some(key) = key {
        if kind == ApiKind::Anthropic {
            let mut v = HeaderValue::from_str(key).map_err(|_| bad())?;
            v.set_sensitive(true);
            map.insert("x-api-key", v);
        } else {
            let mut v = HeaderValue::from_str(&format!("Bearer {key}")).map_err(|_| bad())?;
            v.set_sensitive(true);
            map.insert(AUTHORIZATION, v);
        }
    }
    if kind == ApiKind::Anthropic {
        map.insert(
            "anthropic-version",
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );
    }
    Ok(map)
}

fn host_of(base_url: &str) -> String {
    reqwest::Url::parse(base_url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "the provider".into())
}

pub(crate) fn send_error(error: &reqwest::Error, base_url: &str) -> String {
    if error.is_timeout() && !error.is_connect() {
        "The provider did not respond for 120 seconds.".into()
    } else {
        format!("Cannot connect to {}.", redact(&host_of(base_url), None))
    }
}

pub(crate) async fn status_error(mut response: reqwest::Response, key: Option<&str>) -> String {
    let status = response.status().as_u16();
    match status {
        401 | 403 => {
            return format!("Authentication failed (HTTP {status}). Check the API key.");
        }
        404 => return "Endpoint or model not found (HTTP 404).".into(),
        500..=599 => return format!("Provider error (HTTP {status})."),
        _ => {}
    }
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        body.extend_from_slice(&chunk);
        if body.len() >= ERROR_BODY_LIMIT {
            body.truncate(ERROR_BODY_LIMIT);
            break;
        }
    }
    let message = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
        .map(|m| redact(&m, key))
        .filter(|m| !m.is_empty());
    let head = match status {
        429 => format!("Rate limited (HTTP {status})."),
        400..=499 => format!("The provider rejected the request (HTTP {status})."),
        _ => format!("Unexpected provider response (HTTP {status})."),
    };
    match message {
        Some(m) => format!("{head} {m}"),
        None => head,
    }
}

/// Sends the streaming request. Returns the response only for a 200 event stream.
pub(crate) async fn open_stream(
    client: &reqwest::Client,
    r: &ChatRequest<'_>,
) -> Result<reqwest::Response, String> {
    let path = if r.kind == ApiKind::Anthropic {
        "messages"
    } else {
        "chat/completions"
    };
    let body = serde_json::to_vec(&build_body(r)?).map_err(|_| "Cannot encode the request.")?;
    let mut map = headers(r.kind, r.headers, r.key)?;
    map.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    map.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("text/event-stream"),
    );
    let response = client
        .post(format!("{}/{path}", r.base_url))
        .headers(map)
        .body(body)
        .send()
        .await
        .map_err(|e| send_error(&e, r.base_url))?;
    if response.status().as_u16() != 200 {
        return Err(status_error(response, r.key).await);
    }
    let is_sse = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.trim()
                .to_ascii_lowercase()
                .starts_with("text/event-stream")
        });
    if !is_sse {
        return Err("Unexpected provider response.".into());
    }
    Ok(response)
}

pub(crate) fn parse_models(body: &[u8]) -> Result<Vec<String>, String> {
    let v: Value =
        serde_json::from_slice(body).map_err(|_| "Unexpected provider response.".to_string())?;
    let data = v["data"]
        .as_array()
        .ok_or("Unexpected provider response.")?;
    let mut ids: Vec<String> = Vec::new();
    for id in data.iter().filter_map(|m| m["id"].as_str()) {
        if super::settings::valid_model_id(id).is_ok() && !ids.iter().any(|i| i == id) {
            ids.push(id.to_string());
        }
    }
    ids.sort();
    ids.truncate(500);
    Ok(ids)
}

pub(crate) async fn probe(
    client: &reqwest::Client,
    kind: ApiKind,
    base_url: &str,
    custom: &[Header],
    key: Option<&Zeroizing<String>>,
) -> Result<Vec<String>, String> {
    let key = key.map(|k| k.as_str());
    let url = if kind == ApiKind::Anthropic {
        format!("{base_url}/models?limit=1000")
    } else {
        format!("{base_url}/models")
    };
    let mut response = client
        .get(url)
        .headers(headers(kind, custom, key)?)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| send_error(&e, base_url))?;
    if !response.status().is_success() {
        return Err(status_error(response, key).await);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| send_error(&e, base_url))?
    {
        if body.len() + chunk.len() > PROBE_BODY_LIMIT {
            return Err("Unexpected provider response.".into());
        }
        body.extend_from_slice(&chunk);
    }
    let ids = parse_models(&body)?;
    Ok(if kind == ApiKind::Google {
        google_models(ids)
    } else {
        ids
    })
}

/// AI Studio lists `models/<id>` and includes non-chat models; chat requests take the bare id.
fn google_models(ids: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = ids
        .into_iter()
        .map(|id| id.strip_prefix("models/").unwrap_or(&id).to_string())
        .filter(|id| id.starts_with("gemini") && !id.contains("embedding"))
        .collect();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_model_ids_are_bare_chat_models() {
        let ids = vec![
            "models/embedding-001".to_string(),
            "models/gemini-2.5-flash".to_string(),
            "models/gemini-embedding-001".to_string(),
            "models/imagen-4".to_string(),
        ];
        assert_eq!(google_models(ids), vec!["gemini-2.5-flash".to_string()]);
    }

    fn ev(data: &str) -> SseEvent {
        SseEvent {
            event: None,
            data: data.into(),
        }
    }

    #[test]
    fn redaction() {
        let key = "my-secret-key-123";
        let out = redact(
            &format!("bad {key} sk-abcdefgh12 sk-ant-api03-xyz_abc-123 Bearer tok.en\x07\nend"),
            Some(key),
        );
        assert_eq!(out, "bad [redacted] [redacted] [redacted] [redacted] end");
        assert!(!redact("sk-short", None).contains("[redacted]"));
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0.c2lnbmF0dXJl";
        let pat = format!("github_pat_{}", "A1b2".repeat(6));
        let text = format!(
            "{jwt} ghp_abcdefghij1234 gho_abcdefghij1234 {pat} https://x.test/a?Token=abc123&API_KEY=zzz&k=1 access_token=q"
        );
        assert_eq!(
            redact(&text, None),
            "[redacted] [redacted] [redacted] [redacted] https://x.test/a?[redacted]&[redacted]&k=1 [redacted]"
        );
        let long = redact(&"é".repeat(3000), None);
        assert!(long.len() <= 2048 && long.chars().all(|c| c == 'é'));
    }

    #[test]
    fn sse_split_and_crlf() {
        let mut p = SseParser::default();
        let text = "data: h\u{e9}llo\r\n\r\n";
        let bytes = text.as_bytes();
        // Split inside the two-byte "é".
        let cut = text.find('\u{e9}').unwrap() + 1;
        assert!(p.push(&bytes[..cut]).unwrap().is_empty());
        let events = p.push(&bytes[cut..]).unwrap();
        assert_eq!(events, vec![ev("h\u{e9}llo")]);
        let mut p = SseParser::default();
        let mut got = Vec::new();
        for b in b"event: x\ndata: a\ndata: b\n\n: comment\ndata:c\n\n" {
            got.extend(p.push(&[*b]).unwrap());
        }
        assert_eq!(
            got,
            vec![
                SseEvent {
                    event: Some("x".into()),
                    data: "a\nb".into()
                },
                ev("c")
            ]
        );
    }

    #[test]
    fn sse_finish_and_limits() {
        let mut p = SseParser::default();
        assert!(p.push(b"data: [DONE]\ndata: par").unwrap().is_empty());
        assert_eq!(p.finish(), Some(ev("[DONE]")));
        let mut p = SseParser::default();
        assert!(p.push(&vec![b'a'; LINE_LIMIT + 1]).is_err());
    }

    #[test]
    fn sse_bom_bare_cr_and_event_cap() {
        let mut p = SseParser::default();
        let mut got = p.push(b"\xEF\xBB").unwrap();
        got.extend(p.push(b"\xBFdata: a\r\r").unwrap());
        got.extend(p.push(b"data: b\r").unwrap());
        got.extend(p.push(b"\n\r\n").unwrap());
        assert_eq!(got, vec![ev("a"), ev("b")]);
        let mut p = SseParser::default();
        assert!(p.push(b"data: c\r").unwrap().is_empty());
        assert_eq!(p.finish(), Some(ev("c")));
        let mut p = SseParser::default();
        let line = format!("data: {}\n", "x".repeat(LINE_LIMIT - 10));
        let mut result = Ok(Vec::new());
        for _ in 0..5 {
            result = p.push(line.as_bytes());
            if result.is_err() {
                break;
            }
        }
        assert!(result.is_err());
    }

    #[test]
    fn openai_mapping() {
        let k = ApiKind::Openai;
        assert_eq!(map_event(k, &ev("[DONE]")), vec![Mapped::Done]);
        assert_eq!(
            map_event(
                k,
                &ev(r#"{"choices":[{"delta":{"content":"hi"},"finish_reason":null}]}"#)
            ),
            vec![Mapped::Text("hi".into())]
        );
        assert_eq!(
            map_event(
                k,
                &ev(r#"{"choices":[{"delta":{"content":"x"},"finish_reason":"stop"}]}"#)
            ),
            vec![Mapped::Text("x".into()), Mapped::Stop("stop".into())]
        );
        assert_eq!(
            map_event(k, &ev(r#"{"error":{"message":"boom"}}"#)),
            vec![Mapped::Error("boom".into())]
        );
        assert!(map_event(k, &ev(r#"{"choices":[]}"#)).is_empty());
        assert!(map_event(k, &ev("not json")).is_empty());
    }

    #[test]
    fn anthropic_mapping() {
        let k = ApiKind::Anthropic;
        assert_eq!(
            map_event(
                k,
                &ev(r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"yo"}}"#)
            ),
            vec![Mapped::Text("yo".into())]
        );
        assert!(map_event(
            k,
            &ev(
                r#"{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"z"}}"#
            )
        )
        .is_empty());
        assert_eq!(
            map_event(
                k,
                &ev(r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#)
            ),
            vec![Mapped::Stop("end_turn".into())]
        );
        assert_eq!(
            map_event(k, &ev(r#"{"type":"message_stop"}"#)),
            vec![Mapped::Done]
        );
        assert_eq!(
            map_event(
                k,
                &ev(r#"{"type":"error","error":{"type":"overloaded","message":"busy"}}"#)
            ),
            vec![Mapped::Error("busy".into())]
        );
        assert!(map_event(k, &ev(r#"{"type":"ping"}"#)).is_empty());
    }

    #[test]
    fn bodies() {
        let t = |role, text: &str| (role, text.to_string(), Vec::new());
        let msgs = [
            t("user", "a"),
            t("user", "b"),
            t("assistant", "c"),
            t("user", "d"),
        ];
        let dir = std::path::Path::new("/nonexistent");
        let mk = |kind, max| ChatRequest {
            kind,
            base_url: "https://x",
            headers: &[],
            key: None,
            model: "m",
            system: "sys",
            max_tokens: max,
            messages: &msgs,
            attachments_dir: dir,
        };
        let a = build_body(&mk(ApiKind::Anthropic, None)).unwrap();
        assert_eq!(a["max_tokens"], 8192);
        assert_eq!(a["system"], "sys");
        assert_eq!(a["messages"][0]["content"], "a\n\nb");
        assert_eq!(a["messages"].as_array().unwrap().len(), 3);
        let o = build_body(&mk(ApiKind::Openai, Some(7))).unwrap();
        assert_eq!(o["messages"][0]["role"], "system");
        assert_eq!(o["messages"][1]["content"], "a");
        assert_eq!(o["max_completion_tokens"], 7);
        assert!(o.get("max_tokens").is_none());
        let c = build_body(&mk(ApiKind::OpenaiCompatible, Some(7))).unwrap();
        assert_eq!(c["max_tokens"], 7);
        assert!(build_body(&mk(ApiKind::Openai, None))
            .unwrap()
            .get("max_completion_tokens")
            .is_none());
    }

    #[test]
    fn bodies_with_attachments() {
        use super::super::attachments::{Attachment, Kind};
        let dir = tempfile::tempdir().unwrap();
        let mut files = Vec::new();
        for (id, name, mime, kind, data) in [
            ("i", "p.png", "image/png", Kind::Image, &b"\x89PNG"[..]),
            ("t", "n.txt", "text/plain", Kind::Text, &b"hello"[..]),
            ("d", "r.pdf", "application/pdf", Kind::Pdf, &b"%PDF"[..]),
        ] {
            std::fs::write(dir.path().join(id), data).unwrap();
            files.push(Attachment {
                id: id.into(),
                name: name.into(),
                mime: mime.into(),
                kind,
                size: data.len() as u64,
            });
        }
        let msgs = [
            ("user", "look".to_string(), files.clone()),
            ("assistant", "ok".to_string(), Vec::new()),
            ("user", String::new(), files[..1].to_vec()),
        ];
        let mk = |kind| ChatRequest {
            kind,
            base_url: "https://x",
            headers: &[],
            key: None,
            model: "m",
            system: "",
            max_tokens: None,
            messages: &msgs,
            attachments_dir: dir.path(),
        };
        let o = build_body(&mk(ApiKind::Openai)).unwrap();
        let c = &o["messages"][0]["content"];
        assert_eq!(c[0], json!({"type": "text", "text": "look"}));
        assert_eq!(c[1]["image_url"]["url"], "data:image/png;base64,iVBORw==");
        assert_eq!(c[2]["text"], "<file name=\"n.txt\">\nhello\n</file>");
        assert_eq!(c[3]["file"]["filename"], "r.pdf");
        assert_eq!(
            c[3]["file"]["file_data"],
            "data:application/pdf;base64,JVBERg=="
        );
        assert_eq!(o["messages"][1]["content"], "ok");
        // An attachment-only turn has no empty text part.
        assert_eq!(o["messages"][2]["content"].as_array().unwrap().len(), 1);
        for kind in [ApiKind::Google, ApiKind::OpenaiCompatible] {
            assert_eq!(
                build_body(&mk(kind)).unwrap_err(),
                "This provider can't read PDF files."
            );
        }
        let a = build_body(&mk(ApiKind::Anthropic)).unwrap();
        let c = &a["messages"][0]["content"];
        assert_eq!(c[1]["type"], "image");
        assert_eq!(c[1]["source"]["media_type"], "image/png");
        assert_eq!(c[1]["source"]["data"], "iVBORw==");
        assert_eq!(c[2]["text"], "<file name=\"n.txt\">\nhello\n</file>");
        assert_eq!(c[3]["type"], "document");
        assert_eq!(c[3]["source"]["media_type"], "application/pdf");
    }

    #[test]
    fn models_parse() {
        let ids = parse_models(br#"{"data":[{"id":"b"},{"id":"a"},{"id":"a"},{"id":""},{"x":1}]}"#)
            .unwrap();
        assert_eq!(ids, vec!["a", "b"]);
        assert!(parse_models(b"{}").is_err());
    }
}
