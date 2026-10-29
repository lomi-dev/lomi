use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::{net::SocketAddr, time::Instant};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
    time::{timeout_at, Duration, Instant as TokioInstant},
};

const HEADER_LIMIT: usize = 8 * 1024;
const CALLBACK_PATH: &str = "/auth/callback";
const BROWSER_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) struct PendingCallback {
    pub(super) code: String,
    stream: TcpStream,
}

impl PendingCallback {
    pub(super) async fn complete(mut self, authenticated: bool) -> bool {
        let response = completion_response(authenticated);
        matches!(
            timeout_at(
                TokioInstant::now() + BROWSER_TIMEOUT,
                self.stream.write_all(response.as_bytes())
            )
            .await,
            Ok(Ok(()))
        )
    }
}

#[derive(Debug)]
pub(super) enum CallbackError {
    Cancelled,
    Expired,
    Invalid,
}

pub(super) async fn bind_loopback() -> Result<TcpListener, String> {
    TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| "Cannot open a local sign-in callback.".to_string())
}

pub(super) fn redirect_uri(listener: &TcpListener) -> Result<String, String> {
    let address = listener
        .local_addr()
        .map_err(|_| "Cannot open a local sign-in callback.".to_string())?;
    if !address.ip().is_loopback() || address.port() < 1024 {
        return Err("Cannot open a local sign-in callback.".into());
    }
    Ok(format!("http://127.0.0.1:{}/auth/callback", address.port()))
}

pub(super) async fn wait_for_callback(
    listener: TcpListener,
    expected_state: &str,
    expires_at: Instant,
    mut cancel: watch::Receiver<bool>,
) -> Result<PendingCallback, CallbackError> {
    let deadline = TokioInstant::from_std(expires_at);
    let local_address = listener.local_addr().ok();
    let mut listener = Some(listener);
    loop {
        let Some(active_listener) = listener.as_ref() else {
            return Err(CallbackError::Invalid);
        };
        let accepted = tokio::select! {
            changed = cancel.changed() => {
                let _ = changed;
                return Err(CallbackError::Cancelled);
            }
            accepted = timeout_at(deadline, active_listener.accept()) => match accepted {
                Ok(Ok((stream, peer))) => (stream, peer),
                Ok(Err(_)) => return Err(CallbackError::Invalid),
                Err(_) => return Err(CallbackError::Expired),
            }
        };
        let (mut stream, peer) = accepted;
        let request_deadline = deadline.min(TokioInstant::now() + BROWSER_TIMEOUT);
        let request = tokio::select! {
            changed = cancel.changed() => {
                let _ = changed;
                return Err(CallbackError::Cancelled);
            }
            request = read_request(&mut stream, request_deadline) => request,
        };
        let Some(request) = request else {
            if write_invalid_response(&mut stream, &mut cancel).await {
                return Err(CallbackError::Cancelled);
            }
            if TokioInstant::now() >= deadline {
                return Err(CallbackError::Expired);
            }
            continue;
        };
        match parse_callback_request(&request, peer, local_address, expected_state) {
            Some(code) => {
                drop(listener.take());
                return Ok(PendingCallback { code, stream });
            }
            None => {
                if write_invalid_response(&mut stream, &mut cancel).await {
                    return Err(CallbackError::Cancelled);
                }
            }
        }
        if TokioInstant::now() >= deadline {
            return Err(CallbackError::Expired);
        }
    }
}

async fn read_request(stream: &mut TcpStream, deadline: TokioInstant) -> Option<Vec<u8>> {
    let mut request = Vec::with_capacity(1024);
    let mut buffer = [0_u8; 1024];
    loop {
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return Some(request);
        }
        if request.len() >= HEADER_LIMIT {
            return None;
        }
        let limit = buffer.len().min(HEADER_LIMIT - request.len());
        let count = match timeout_at(deadline, stream.read(&mut buffer[..limit])).await {
            Ok(Ok(count)) if count > 0 => count,
            _ => return None,
        };
        request.extend_from_slice(&buffer[..count]);
    }
}

fn parse_callback_request(
    request: &[u8],
    peer: SocketAddr,
    local: Option<SocketAddr>,
    expected_state: &str,
) -> Option<String> {
    if !peer.ip().is_loopback() || request.len() > HEADER_LIMIT {
        return None;
    }
    let header_end = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")?
        + 4;
    if request[header_end..]
        .iter()
        .any(|byte| !byte.is_ascii_whitespace())
    {
        return None;
    }
    let text = std::str::from_utf8(&request[..header_end]).ok()?;
    let mut lines = text[..header_end - 4].split("\r\n");
    let request_line = lines.next()?;
    let mut request_parts = request_line.split(' ');
    if request_parts.next()? != "GET" {
        return None;
    }
    let target = request_parts.next()?;
    if request_parts.next()? != "HTTP/1.1" || request_parts.next().is_some() {
        return None;
    }

    let mut host = None;
    let mut content_length_seen = false;
    for line in lines {
        let (name, value) = line.split_once(':')?;
        if name.is_empty()
            || name
                .bytes()
                .any(|byte| !byte.is_ascii_alphanumeric() && byte != b'-')
        {
            return None;
        }
        if name.eq_ignore_ascii_case("host") {
            if host.replace(value.trim()).is_some() {
                return None;
            }
        } else if name.eq_ignore_ascii_case("content-length") {
            if content_length_seen || value.trim() != "0" {
                return None;
            }
            content_length_seen = true;
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            return None;
        }
    }
    let port = local?.port();
    if host? != format!("127.0.0.1:{port}") {
        return None;
    }

    let (path, query) = target.split_once('?')?;
    if path != CALLBACK_PATH || target.contains('#') {
        return None;
    }
    let mut code = None;
    let mut state = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        match key {
            "code" if code.is_none() && valid_nonce(value) => code = Some(value),
            "state" if state.is_none() && valid_nonce(value) => state = Some(value),
            _ => return None,
        }
    }
    if query.matches('&').count() != 1 {
        return None;
    }
    let state = state?;
    if !constant_time_eq(state.as_bytes(), expected_state.as_bytes()) {
        return None;
    }
    code.map(str::to_string)
}

fn valid_nonce(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

async fn write_invalid_response(
    stream: &mut TcpStream,
    cancel: &mut watch::Receiver<bool>,
) -> bool {
    let response = http_response(
        "400 Bad Request",
        "error",
        "Unable to return to Lomi",
        "This sign-in link is invalid. Return to Lomi and start sign-in again.",
        "role=\"alert\"",
    );
    tokio::select! {
        _ = timeout_at(TokioInstant::now() + BROWSER_TIMEOUT, stream.write_all(response.as_bytes())) => false,
        changed = cancel.changed() => {
            let _ = changed;
            true
        }
    }
}

fn completion_response(authenticated: bool) -> String {
    if authenticated {
        http_response(
            "200 OK",
            "success",
            "Authentication successful",
            "You have successfully signed in to Lomi. You can close this tab and return to the app.",
            "role=\"status\"",
        )
    } else {
        http_response(
            "200 OK",
            "error",
            "Sign-in was not completed",
            "Lomi could not activate your session. Return to Lomi and try signing in again.",
            "role=\"alert\"",
        )
    }
}

fn http_response(status: &str, state: &str, title: &str, message: &str, role: &str) -> String {
    let cloud = STANDARD.encode(include_bytes!("assets/clouds.webp"));
    let geist_latin = STANDARD.encode(include_bytes!("assets/Geist-latin-wght-normal.woff2"));
    let geist_latin_ext =
        STANDARD.encode(include_bytes!("assets/Geist-latin-ext-wght-normal.woff2"));
    let wordmark = STANDARD.encode(include_bytes!("assets/lomi-wordmark.svg"));
    let body = include_str!("callback.html")
        .replace("{{state}}", state)
        .replace("{{title}}", title)
        .replace("{{message}}", message)
        .replace("{{role}}", role)
        .replace("{{cloud}}", &cloud)
        .replace("{{geist_latin}}", &geist_latin)
        .replace("{{geist_latin_ext}}", &geist_latin_ext)
        .replace("{{wordmark}}", &wordmark);
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:; base-uri 'none'; frame-ancestors 'none'; form-action 'none'\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
