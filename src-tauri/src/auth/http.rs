use super::{config::AuthConfig, safe_text, AuthSession, AuthUser};
use reqwest::{header, Client, Response, StatusCode};
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;

const RESPONSE_LIMIT: usize = 64 * 1024;

pub(super) struct ApiClient {
    client: Client,
    config: AuthConfig,
    remote_origin: Option<String>,
    host_identity_origin: Option<String>,
    host_remote_api_origin: Option<String>,
}

pub(super) struct DesktopStart {
    pub authorization_url: String,
}

pub(super) struct SessionCheck {
    pub user: AuthUser,
    pub session: AuthSession,
    pub renewed_token: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HttpProblem {
    Transport,
    Temporary,
    RateLimited(Option<u64>),
    Unauthorized,
    Forbidden,
    DesktopUpdateRequired,
    InvalidResponse,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteHttpError {
    pub status: Option<u16>,
    pub code: String,
    #[serde(skip)]
    problem: HttpProblem,
}

impl RemoteHttpError {
    fn local(problem: HttpProblem, code: &str) -> Self {
        Self {
            status: None,
            code: code.to_owned(),
            problem,
        }
    }

    pub(crate) fn local_code(code: &str) -> Self {
        Self::local(HttpProblem::Rejected, code)
    }

    fn from_http(status: StatusCode, body: &[u8], retry_after: Option<u64>) -> Self {
        let problem = classify_http_error(status, body, retry_after);
        let code = response_error_code(body).unwrap_or_else(|| {
            match status.as_u16() {
                401 => "UNAUTHORIZED",
                403 => "FORBIDDEN",
                404 => "NOT_FOUND",
                409 => "CONFLICT",
                426 => "DESKTOP_UPDATE_REQUIRED",
                429 => "RATE_LIMITED",
                500..=599 => "SERVICE_UNAVAILABLE",
                _ => "REQUEST_REJECTED",
            }
            .to_owned()
        });
        Self {
            status: Some(status.as_u16()),
            code,
            problem,
        }
    }

    pub(crate) fn message(&self) -> &'static str {
        match self.problem {
            HttpProblem::DesktopUpdateRequired => "Update Lomi to share workspaces remotely.",
            HttpProblem::Transport => {
                "Cannot connect to the Remote service. Check your connection and retry."
            }
            HttpProblem::Temporary => {
                "The Remote service is temporarily unavailable. Try again shortly."
            }
            HttpProblem::RateLimited(_) => "Too many Remote requests. Wait a moment and retry.",
            HttpProblem::Unauthorized => {
                "The Remote service could not verify your account session. Check Account settings and retry."
            }
            HttpProblem::Forbidden => "The Remote service denied access to this account.",
            HttpProblem::InvalidResponse => {
                "The Remote service returned an invalid response. Try again later."
            }
            HttpProblem::Rejected => "The Remote service rejected the request. Try again later.",
        }
    }
}

impl From<String> for RemoteHttpError {
    fn from(_: String) -> Self {
        Self::local(HttpProblem::Rejected, "REQUEST_FAILED")
    }
}

impl From<&'static str> for RemoteHttpError {
    fn from(_: &'static str) -> Self {
        Self::local(HttpProblem::Rejected, "REQUEST_FAILED")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PushRequestError {
    pub status: Option<u16>,
    pub code: String,
    pub retry_after_seconds: Option<u64>,
}

impl PushRequestError {
    pub(crate) fn local(code: &str) -> Self {
        Self {
            status: None,
            code: code.into(),
            retry_after_seconds: None,
        }
    }
}

impl std::fmt::Display for PushRequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.code)
    }
}

impl std::error::Error for PushRequestError {}

fn push_resource_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn push_route_allowed(method: &reqwest::Method, path: &str) -> bool {
    if path.len() > 256 {
        return false;
    }
    let Some(route) = path.strip_prefix("/v1/remote/native/push/") else {
        return false;
    };
    match (method, route) {
        (&reqwest::Method::GET, "preferences" | "sources" | "installations") => true,
        (&reqwest::Method::POST, "preferences" | "sources/register" | "events" | "test") => true,
        (&reqwest::Method::POST, _) => {
            let pieces: Vec<_> = route.split('/').collect();
            pieces.len() == 3
                && push_resource_id(pieces[1])
                && matches!(
                    (pieces[0], pieces[2]),
                    ("sources", "settings") | ("installations", "disable")
                )
        }
        _ => false,
    }
}

fn push_http_error(status: StatusCode, body: &[u8], retry_after: Option<u64>) -> PushRequestError {
    let supplied = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    let fallback = match status.as_u16() {
        400 => "INVALID_REQUEST",
        401 => "SESSION_REQUIRED",
        403 => "ACCOUNT_UNAVAILABLE",
        404 => "NOT_FOUND",
        409 => "REVISION_CONFLICT",
        426 => "UPDATE_REQUIRED",
        429 => "RATE_LIMITED",
        500..=599 => "SERVICE_UNAVAILABLE",
        _ => "REQUEST_REJECTED",
    };
    let code = supplied
        .filter(|code| {
            matches!(
                code.as_str(),
                "INVALID_REQUEST"
                    | "ACCESS_EXPIRED"
                    | "SESSION_REQUIRED"
                    | "FEATURE_DISABLED"
                    | "ACCOUNT_UNAVAILABLE"
                    | "NOT_FOUND"
                    | "REVISION_CONFLICT"
                    | "BINDING_CHANGED"
                    | "TOKEN_BINDING_CONFLICT"
                    | "UPDATE_REQUIRED"
                    | "RATE_LIMITED"
                    | "SERVICE_UNAVAILABLE"
            )
        })
        .unwrap_or_else(|| fallback.into());
    PushRequestError {
        status: Some(status.as_u16()),
        code,
        retry_after_seconds: retry_after.map(|seconds| seconds.clamp(1, 600)),
    }
}

#[derive(Clone, Copy)]
enum RemoteV2Destination {
    Identity,
    RemoteApi,
}

fn remote_v2_destination(method: &reqwest::Method, path: &str) -> Option<RemoteV2Destination> {
    use lomi_remote_protocol::HttpSurface;

    let route = remote_v2_route(method, path)?;
    if route.route == lomi_remote_protocol::HttpRoute::ConsumerAuthorizationIssue {
        return Some(RemoteV2Destination::Identity);
    }
    match route.surface {
        HttpSurface::ConsumerIdentity | HttpSurface::ConsumerManagement => {
            Some(RemoteV2Destination::Identity)
        }
        HttpSurface::ConsumerSession | HttpSurface::EitherOwnPeer => {
            Some(RemoteV2Destination::RemoteApi)
        }
        _ => None,
    }
}

pub(crate) fn remote_v2_route(
    method: &reqwest::Method,
    path: &str,
) -> Option<&'static lomi_remote_protocol::HttpRouteContract> {
    use lomi_remote_protocol::{HttpMethod, HTTP_ROUTES, REMOTE_HTTP_PREFIX};

    if path.contains('?') || path.contains('#') || path.trim() != path {
        return None;
    }
    let route_path = path.strip_prefix(REMOTE_HTTP_PREFIX)?;
    let method = match method.as_str() {
        "GET" => HttpMethod::Get,
        "POST" => HttpMethod::Post,
        "PUT" => HttpMethod::Put,
        _ => return None,
    };
    HTTP_ROUTES.iter().find(|contract| {
        contract.method == method && route_template_matches(contract.path, route_path)
    })
}

fn route_template_matches(template: &str, actual: &str) -> bool {
    let expected: Vec<_> = template.split('/').collect();
    let actual: Vec<_> = actual.split('/').collect();
    if expected.len() != actual.len() {
        return false;
    }
    expected.iter().zip(actual).all(|(expected, actual)| {
        if expected.starts_with('{') && expected.ends_with('}') {
            lomi_remote_protocol::WorkspaceId::try_from(actual.to_owned()).is_ok()
        } else {
            *expected == actual
        }
    })
}

impl ApiClient {
    #[cfg(test)]
    pub(super) fn set_test_remote_origin(&mut self, origin: String) {
        assert!(origin.starts_with("http://127.0.0.1:"));
        self.remote_origin = Some(origin);
    }

    pub fn new(config: AuthConfig) -> Result<Self, String> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(12))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "Cannot initialize the secure account connection.".to_string())?;
        let remote_origin = config.remote_origin().ok();
        Ok(Self {
            client,
            config,
            remote_origin,
            host_identity_origin: option_env!("LOMI_HOST_IDENTITY_ORIGIN").map(str::to_owned),
            host_remote_api_origin: option_env!("LOMI_HOST_REMOTE_API_ORIGIN").map(str::to_owned),
        })
    }

    pub async fn remote_request(
        &self,
        token: &str,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, HttpProblem> {
        // Native callers provide a route, never an origin or arbitrary headers.
        if !path.starts_with("/v1/remote/native/")
            || path.contains("..")
            || !path
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/-".contains(&b))
            || !matches!(method, reqwest::Method::GET | reqwest::Method::POST)
        {
            return Err(HttpProblem::Rejected);
        }
        let origin = self.remote_origin.as_ref().ok_or(HttpProblem::Rejected)?;
        let url = reqwest::Url::parse(&format!("{origin}{path}"))
            .map_err(|_| HttpProblem::InvalidResponse)?;
        let request = self
            .client
            .request(method, url)
            .bearer_auth(token)
            .header("X-Lomi-Remote-Protocol", "live-atomic-renew-v1")
            .timeout(Duration::from_secs(5));
        let request = if let Some(body) = body {
            request
                .header(header::CONTENT_TYPE, "application/json")
                .body(serde_json::to_vec(body).map_err(|_| HttpProblem::InvalidResponse)?)
        } else {
            request
        };
        let (response, bytes, retry_after) = read_response_limit(
            request.send().await.map_err(|_| HttpProblem::Transport)?,
            2 * 1024 * 1024,
        )
        .await?;
        if !response.status().is_success() {
            return Err(classify_status(response, &bytes, retry_after));
        }
        serde_json::from_slice(&bytes).map_err(|_| HttpProblem::InvalidResponse)
    }

    /// Native Remote v2 transport. The destination is derived from the
    /// canonical route surface, never from a caller-supplied URL or header.
    pub async fn remote_v2_request(
        &self,
        token: &str,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, RemoteHttpError> {
        let destination = remote_v2_destination(&method, path)
            .ok_or_else(|| RemoteHttpError::local(HttpProblem::Rejected, "REQUEST_REJECTED"))?;
        let origin = match destination {
            RemoteV2Destination::Identity => self.host_identity_origin.as_deref(),
            RemoteV2Destination::RemoteApi => self.host_remote_api_origin.as_deref(),
        }
        .ok_or_else(|| RemoteHttpError::local(HttpProblem::Rejected, "SERVICE_UNAVAILABLE"))?;
        let url = reqwest::Url::parse(&format!("{origin}{path}"))
            .map_err(|_| RemoteHttpError::local(HttpProblem::InvalidResponse, "INVALID_REQUEST"))?;
        let request = self
            .client
            .request(method, url)
            .bearer_auth(token)
            .timeout(Duration::from_secs(12));
        let request = if let Some(body) = body {
            let bytes = serde_json::to_vec(body).map_err(|_| {
                RemoteHttpError::local(HttpProblem::InvalidResponse, "INVALID_REQUEST")
            })?;
            if bytes.len() > lomi_remote_protocol::CONTROL_MESSAGE_BYTES as usize {
                return Err(RemoteHttpError::local(
                    HttpProblem::Rejected,
                    "REQUEST_REJECTED",
                ));
            }
            request
                .header(header::CONTENT_TYPE, "application/json")
                .body(bytes)
        } else {
            request
        };
        let (response, bytes, retry_after) = read_response_limit(
            request
                .send()
                .await
                .map_err(|_| RemoteHttpError::local(HttpProblem::Transport, "TRANSPORT_UNKNOWN"))?,
            lomi_remote_protocol::CONTROL_MESSAGE_BYTES as usize,
        )
        .await
        .map_err(|problem| RemoteHttpError::local(problem, problem_code(problem)))?;
        if !response.status().is_success() {
            return Err(RemoteHttpError::from_http(
                response.status(),
                &bytes,
                retry_after,
            ));
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| RemoteHttpError::local(HttpProblem::InvalidResponse, "INVALID_RESPONSE"))
    }

    pub async fn push_request(
        &self,
        token: &str,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, PushRequestError> {
        if !push_route_allowed(&method, path) {
            return Err(PushRequestError::local("INVALID_REQUEST"));
        }
        let origin = self
            .remote_origin
            .as_ref()
            .ok_or_else(|| PushRequestError::local("SERVICE_UNAVAILABLE"))?;
        let url = reqwest::Url::parse(&format!("{origin}{path}"))
            .map_err(|_| PushRequestError::local("INVALID_REQUEST"))?;
        let request = self
            .client
            .request(method, url)
            .bearer_auth(token)
            .header("X-Lomi-Remote-Protocol", "live-atomic-renew-v1")
            .timeout(Duration::from_secs(5));
        let request = if let Some(body) = body {
            let bytes =
                serde_json::to_vec(body).map_err(|_| PushRequestError::local("INVALID_REQUEST"))?;
            if bytes.len() > 16 * 1024 {
                return Err(PushRequestError::local("INVALID_REQUEST"));
            }
            request
                .header(header::CONTENT_TYPE, "application/json")
                .body(bytes)
        } else {
            request
        };
        let response = request
            .send()
            .await
            .map_err(|_| PushRequestError::local("TRANSPORT_UNKNOWN"))?;
        let status = response.status();
        let (response, bytes, retry_after) = read_response_limit(response, RESPONSE_LIMIT)
            .await
            .map_err(|problem| PushRequestError {
                status: Some(status.as_u16()),
                code: if problem == HttpProblem::Transport {
                    "TRANSPORT_UNKNOWN"
                } else {
                    "INVALID_RESPONSE"
                }
                .into(),
                retry_after_seconds: None,
            })?;
        if !response.status().is_success() {
            return Err(push_http_error(status, &bytes, retry_after));
        }
        serde_json::from_slice(&bytes).map_err(|_| PushRequestError {
            status: Some(status.as_u16()),
            code: "INVALID_RESPONSE".into(),
            retry_after_seconds: None,
        })
    }

    pub async fn start_desktop(
        &self,
        redirect_uri: &str,
        state: &str,
        code_challenge: &str,
    ) -> Result<DesktopStart, HttpProblem> {
        let url = self
            .config
            .endpoint("/v1/desktop/start")
            .map_err(|_| HttpProblem::InvalidResponse)?;
        let body = serde_json::json!({
            "clientId": self.config.client_id,
            "redirectUri": redirect_uri,
            "state": state,
            "codeChallenge": code_challenge,
        });
        let (response, bytes, retry_after) = self.send_json(self.client.post(url), &body).await?;
        if !response.status().is_success() {
            return Err(classify_status(response, &bytes, retry_after));
        }
        let wire: DesktopStartWire =
            serde_json::from_slice(&bytes).map_err(|_| HttpProblem::InvalidResponse)?;
        if wire.authorization_url.is_empty() || wire.authorization_url.len() > 2048 {
            return Err(HttpProblem::InvalidResponse);
        }
        if wire.expires_in != 300 {
            return Err(HttpProblem::InvalidResponse);
        }
        Ok(DesktopStart {
            authorization_url: wire.authorization_url,
        })
    }

    pub async fn exchange_desktop_code(
        &self,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<String, HttpProblem> {
        let url = self
            .config
            .endpoint("/v1/desktop/exchange")
            .map_err(|_| HttpProblem::InvalidResponse)?;
        let body = serde_json::json!({
            "clientId": self.config.client_id,
            "code": code,
            "codeVerifier": code_verifier,
            "redirectUri": redirect_uri,
        });
        let (response, bytes, retry_after) = self.send_json(self.client.post(url), &body).await?;
        if !response.status().is_success() {
            return Err(classify_status(response, &bytes, retry_after));
        }
        let wire: DesktopExchangeWire =
            serde_json::from_slice(&bytes).map_err(|_| HttpProblem::InvalidResponse)?;
        if wire.token_type != "Bearer"
            || !(1..=31_536_000).contains(&wire.expires_in)
            || !valid_token(&wire.access_token)
        {
            return Err(HttpProblem::InvalidResponse);
        }
        Ok(wire.access_token)
    }

    pub async fn check_session(&self, token: &str) -> Result<SessionCheck, HttpProblem> {
        let url = self
            .config
            .endpoint("/v1/me")
            .map_err(|_| HttpProblem::InvalidResponse)?;
        let response = self
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| HttpProblem::Transport)?;
        let status = response.status();
        let renewed_token = response
            .headers()
            .get("set-auth-token")
            .and_then(|value| value.to_str().ok())
            .filter(|value| valid_token(value))
            .map(str::to_string);
        let (response, bytes, retry_after) = read_response(response).await?;
        if !status.is_success() {
            return Err(classify_status(response, &bytes, retry_after));
        }
        let wire: MeWire =
            serde_json::from_slice(&bytes).map_err(|_| HttpProblem::InvalidResponse)?;
        let user_status = safe_text(&wire.user.status, 32);
        let user = AuthUser {
            id: safe_text(&wire.user.id, 128),
            display_name: safe_text(&wire.user.display_name, 160),
            email: safe_text(wire.user.email.as_deref().unwrap_or_default(), 254),
            github_login: safe_text(wire.user.github_login.as_deref().unwrap_or_default(), 64),
            status: user_status.clone(),
        };
        let session = AuthSession {
            id: safe_text(&wire.session.id, 128),
            expires_at: safe_text(&wire.session.expires_at, 64),
        };
        if user.id.is_empty()
            || user.display_name.is_empty()
            || session.id.is_empty()
            || session.expires_at.is_empty()
        {
            return Err(HttpProblem::InvalidResponse);
        }
        if user_status != "active" {
            return Err(HttpProblem::Forbidden);
        }
        Ok(SessionCheck {
            user,
            session,
            renewed_token,
        })
    }

    pub async fn sign_out(&self, token: &str) -> bool {
        let Ok(url) = self.config.endpoint("/api/auth/sign-out") else {
            return false;
        };
        let body = serde_json::json!({});
        let Ok((response, _bytes, _retry_after)) = self
            .send_json(self.client.post(url).bearer_auth(token), &body)
            .await
        else {
            return false;
        };
        response.status().is_success()
    }

    pub async fn probe(&self) -> Result<(), HttpProblem> {
        let url = self
            .config
            .endpoint("/health/live")
            .map_err(|_| HttpProblem::InvalidResponse)?;
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| HttpProblem::Transport)?;
        let status = response.status();
        let (response, _bytes, retry_after) = read_response(response).await?;
        if status.is_success() {
            Ok(())
        } else {
            Err(classify_status(response, &[], retry_after))
        }
    }

    async fn send_json(
        &self,
        request: reqwest::RequestBuilder,
        body: &Value,
    ) -> Result<(Response, Vec<u8>, Option<u64>), HttpProblem> {
        let bytes = serde_json::to_vec(body).map_err(|_| HttpProblem::InvalidResponse)?;
        let response = request
            .header(header::CONTENT_TYPE, "application/json")
            .body(bytes)
            .send()
            .await
            .map_err(|_| HttpProblem::Transport)?;
        read_response(response).await
    }
}

async fn read_response(
    response: Response,
) -> Result<(Response, Vec<u8>, Option<u64>), HttpProblem> {
    read_response_limit(response, RESPONSE_LIMIT).await
}

async fn read_response_limit(
    response: Response,
    limit: usize,
) -> Result<(Response, Vec<u8>, Option<u64>), HttpProblem> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(HttpProblem::InvalidResponse);
    }
    let retry_after = response
        .headers()
        .get(header::RETRY_AFTER)
        .or_else(|| response.headers().get("x-retry-after"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|seconds| seconds.clamp(1, 600));
    let mut response = response;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| HttpProblem::Transport)? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(HttpProblem::InvalidResponse);
        }
        body.extend_from_slice(&chunk);
    }
    Ok((response, body, retry_after))
}

fn classify_status(response: Response, body: &[u8], retry_after: Option<u64>) -> HttpProblem {
    classify_http_error(response.status(), body, retry_after)
}

fn problem_code(problem: HttpProblem) -> &'static str {
    match problem {
        HttpProblem::Transport => "TRANSPORT_UNKNOWN",
        HttpProblem::Temporary => "SERVICE_UNAVAILABLE",
        HttpProblem::RateLimited(_) => "RATE_LIMITED",
        HttpProblem::Unauthorized => "UNAUTHORIZED",
        HttpProblem::Forbidden => "FORBIDDEN",
        HttpProblem::DesktopUpdateRequired => "DESKTOP_UPDATE_REQUIRED",
        HttpProblem::InvalidResponse => "INVALID_RESPONSE",
        HttpProblem::Rejected => "REQUEST_REJECTED",
    }
}

fn response_error_code(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let code = value
        .get("code")
        .and_then(Value::as_str)
        .or_else(|| value.get("error").and_then(Value::as_str))?;
    if code.is_empty()
        || code.len() > 80
        || !code
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return None;
    }
    Some(code.to_owned())
}

fn classify_http_error(status: StatusCode, body: &[u8], retry_after: Option<u64>) -> HttpProblem {
    if status == StatusCode::UPGRADE_REQUIRED
        && serde_json::from_slice::<Value>(body)
            .ok()
            .is_some_and(|value| {
                value.get("error").and_then(Value::as_str) == Some("DESKTOP_UPDATE_REQUIRED")
            })
    {
        return HttpProblem::DesktopUpdateRequired;
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return HttpProblem::RateLimited(retry_after);
    }
    if status == StatusCode::UNAUTHORIZED {
        return HttpProblem::Unauthorized;
    }
    if status == StatusCode::FORBIDDEN {
        return HttpProblem::Forbidden;
    }
    if status.is_server_error() {
        return HttpProblem::Temporary;
    }
    HttpProblem::Rejected
}

fn valid_token(token: &str) -> bool {
    !token.is_empty() && token.len() <= 4096 && token.bytes().all(|byte| !byte.is_ascii_control())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DesktopStartWire {
    authorization_url: String,
    expires_in: u64,
}

#[derive(Deserialize)]
struct DesktopExchangeWire {
    access_token: String,
    token_type: String,
    expires_in: u64,
}

#[derive(Deserialize)]
struct MeWire {
    user: UserWire,
    session: SessionWire,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserWire {
    id: String,
    display_name: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    github_login: Option<String>,
    status: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionWire {
    id: String,
    expires_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        thread,
        time::Duration,
    };

    struct Reply {
        status: String,
        headers: String,
        body: Vec<u8>,
    }

    fn reply(status: &str, headers: &str, body: &str) -> Reply {
        Reply {
            status: status.to_string(),
            headers: headers.to_string(),
            body: body.as_bytes().to_vec(),
        }
    }

    fn loopback_server(replies: Vec<Reply>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            replies
                .into_iter()
                .map(|reply| {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                let request = read_http_request(&mut stream);
                let response = format!(
                        "HTTP/1.1 {}\r\nContent-Type: application/json\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n",
                        reply.status,
                        reply.headers,
                        reply.body.len()
                    );
                    stream.write_all(response.as_bytes()).unwrap();
                    if stream.write_all(&reply.body).is_ok() { let _=stream.flush(); }
                    request
                })
                .collect()
        });
        (format!("http://{address}"), worker)
    }

    fn read_http_request(stream: &mut TcpStream) -> String {
        let mut request = Vec::new();
        let mut buffer = [0_u8; 2048];
        let mut header_end = None;
        let mut content_length = 0;
        loop {
            let count = stream.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..count]);
            if header_end.is_none() {
                if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    let end = position + 4;
                    let headers = String::from_utf8_lossy(&request[..end]);
                    content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    header_end = Some(end);
                }
            }
            if header_end.is_some_and(|end| request.len() >= end + content_length) {
                break;
            }
        }
        String::from_utf8(request).unwrap()
    }

    fn test_api(origin: String) -> ApiClient {
        ApiClient::new(AuthConfig {
            origin,
            client_id: "lomi-desktop-test".into(),
            environment: "test".into(),
        })
        .unwrap()
    }

    #[tokio::test]
    async fn fixed_origin_transport_uses_desktop_handshake_renewal_header_and_signout_json() {
        let (origin, server) = loopback_server(vec![
            reply(
                "200 OK",
                "",
                &format!(
                    r#"{{"authorizationUrl":"https://auth.lomi.dev/desktop?request={}","expiresIn":300}}"#,
                    "A".repeat(43)
                ),
            ),
            reply(
                "200 OK",
                "",
                r#"{"access_token":"session-token","token_type":"Bearer","expires_in":900}"#,
            ),
            reply(
                "200 OK",
                "set-auth-token: rotated-token\r\n",
                r#"{"user":{"id":"user-1","displayName":"Ada","email":"ada@example.com","githubLogin":"ada","status":"active"},"session":{"id":"session-1","expiresAt":"2026-10-01T00:00:00Z"}}"#,
            ),
            reply("200 OK", "", "{}"),
        ]);
        let api = test_api(origin);

        let redirect_uri = "http://127.0.0.1:43210/auth/callback";
        let state = "B".repeat(43);
        let challenge = "C".repeat(43);
        let start = api
            .start_desktop(redirect_uri, &state, &challenge)
            .await
            .unwrap();
        assert_eq!(
            start.authorization_url,
            format!("https://auth.lomi.dev/desktop?request={}", "A".repeat(43))
        );
        assert_eq!(
            api.exchange_desktop_code("D".repeat(43).as_str(), &"E".repeat(43), redirect_uri)
                .await
                .unwrap(),
            "session-token"
        );
        let check = api.check_session("session-token").await.unwrap();
        assert_eq!(check.user.id, "user-1");
        assert_eq!(check.renewed_token.as_deref(), Some("rotated-token"));
        assert!(api.sign_out("rotated-token").await);

        let requests = server.join().unwrap();
        for request in &requests {
            assert!(!request
                .to_ascii_lowercase()
                .contains("x-lomi-remote-protocol:"));
        }
        assert!(requests[0].starts_with("POST /v1/desktop/start "));
        assert!(requests[0]
            .to_ascii_lowercase()
            .contains("content-type: application/json\r\n"));
        assert!(requests[0].contains(r#""clientId":"lomi-desktop-test""#));
        assert!(requests[0].contains(&format!(r#""redirectUri":"{redirect_uri}""#)));
        assert!(requests[0].contains(&format!(r#""state":"{state}""#)));
        assert!(requests[0].contains(&format!(r#""codeChallenge":"{challenge}""#)));
        assert!(requests[1].starts_with("POST /v1/desktop/exchange "));
        assert!(requests[1].contains(r#""code":"DDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDD""#));
        assert!(requests[1].contains(&format!(r#""codeVerifier":"{}""#, "E".repeat(43))));
        assert!(requests[1].contains(&format!(r#""redirectUri":"{redirect_uri}""#)));
        for request in [&requests[0], &requests[1]] {
            let headers = request
                .split("\r\n\r\n")
                .next()
                .unwrap()
                .to_ascii_lowercase();
            for forbidden in ["cookie:", "authorization:", "origin:", "sec-fetch-site:"] {
                assert!(!headers.contains(forbidden), "unexpected {forbidden}");
            }
        }
        assert!(requests[2].starts_with("GET /v1/me "));
        assert!(requests[2]
            .to_ascii_lowercase()
            .contains("authorization: bearer session-token"));
        assert!(requests[3].starts_with("POST /api/auth/sign-out "));
        assert!(requests[3]
            .to_ascii_lowercase()
            .contains("authorization: bearer rotated-token"));
        assert!(requests[3]
            .to_ascii_lowercase()
            .contains("content-type: application/json"));
        assert!(requests[3].ends_with("{}"));
    }

    #[tokio::test]
    async fn native_remote_uses_fixed_bearer_origin_without_browser_headers() {
        let (origin, server) = loopback_server(vec![
            reply(
                "200 OK",
                "",
                r#"{"authorizationExpiresAt":"2026-10-01T00:00:00Z","pairings":[],"grants":[],"channels":[]}"#,
            ),
            reply("200 OK", "", "{}"),
        ]);
        let mut api = test_api("https://auth.lomi.dev".into());
        api.remote_origin = Some(origin);
        let state = api
            .remote_request(
                "test-native-token",
                reqwest::Method::GET,
                "/v1/remote/native/session",
                None,
            )
            .await
            .unwrap();
        assert!(state["authorizationExpiresAt"].is_string());
        api.remote_request(
            "test-native-token",
            reqwest::Method::POST,
            "/v1/remote/native/hosts/host-1/heartbeat",
            Some(&serde_json::json!({"shares":[]})),
        )
        .await
        .unwrap();
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("GET /v1/remote/native/session "));
        assert!(requests[1].starts_with("POST /v1/remote/native/hosts/host-1/heartbeat "));
        for request in requests {
            let headers = request
                .split("\r\n\r\n")
                .next()
                .unwrap()
                .to_ascii_lowercase();
            assert!(headers.contains("authorization: bearer test-native-token"));
            assert!(headers.contains("x-lomi-remote-protocol: live-atomic-renew-v1\r\n"));
            for forbidden in ["cookie:", "origin:", "sec-fetch-site:"] {
                assert!(!headers.contains(forbidden));
            }
        }
        for path in [
            "/v1/me",
            "/v1/remote/native/../me",
            "/v1/remote/native/session?secret=x",
            "https://evil.test",
        ] {
            assert!(api
                .remote_request("token", reqwest::Method::GET, path, None)
                .await
                .is_err());
        }
    }

    #[tokio::test]
    async fn renewed_signed_bearer_is_forwarded_unchanged_to_remote() {
        for renewed_token in [
            "native-token.signature+with/padding=",
            "native-token.signature%2Bwith%2Fpadding%3D",
        ] {
            let (origin, server) = loopback_server(vec![
                reply(
                    "200 OK",
                    &format!("set-auth-token: {renewed_token}\r\n"),
                    r#"{"user":{"id":"user-1","displayName":"Ada","status":"active"},"session":{"id":"session-1","expiresAt":"2026-10-01T00:00:00Z"}}"#,
                ),
                reply("200 OK", "", r#"{"accountId":"account-1"}"#),
            ]);
            let mut api = test_api(origin.clone());
            api.remote_origin = Some(origin);
            let check = api.check_session("native-token").await.unwrap();
            assert_eq!(check.renewed_token.as_deref(), Some(renewed_token));
            api.remote_request(
                check.renewed_token.as_deref().unwrap(),
                reqwest::Method::GET,
                "/v1/remote/native/session",
                None,
            )
            .await
            .unwrap();

            let requests = server.join().unwrap();
            assert!(requests[0].starts_with("GET /v1/me "));
            assert!(requests[1].starts_with("GET /v1/remote/native/session "));
            assert!(requests[1].contains(&format!("authorization: Bearer {renewed_token}\r\n")));
            let headers = requests[1]
                .split("\r\n\r\n")
                .next()
                .unwrap()
                .to_ascii_lowercase();
            for forbidden in ["cookie:", "origin:", "sec-fetch-site:"] {
                assert!(!headers.contains(forbidden));
            }
        }
    }

    #[tokio::test]
    async fn native_remote_classifies_exact_update_requirement_without_server_text() {
        let (origin, server) = loopback_server(vec![reply(
            "426 Upgrade Required",
            "",
            r#"{"error":"DESKTOP_UPDATE_REQUIRED","message":"untrusted token or URL"}"#,
        )]);
        let mut api = test_api("https://auth.lomi.dev".into());
        api.remote_origin = Some(origin);
        assert_eq!(
            api.remote_request(
                "token",
                reqwest::Method::GET,
                "/v1/remote/native/session",
                None
            )
            .await
            .unwrap_err(),
            HttpProblem::DesktopUpdateRequired
        );
        server.join().unwrap();
        for (status, body) in [
            (
                StatusCode::UPGRADE_REQUIRED,
                br#"{"error":"OTHER_ERROR"}"#.as_slice(),
            ),
            (
                StatusCode::UPGRADE_REQUIRED,
                br#"{"message":"DESKTOP_UPDATE_REQUIRED"}"#.as_slice(),
            ),
            (StatusCode::UPGRADE_REQUIRED, b"not json".as_slice()),
            (
                StatusCode::BAD_REQUEST,
                br#"{"error":"DESKTOP_UPDATE_REQUIRED"}"#.as_slice(),
            ),
        ] {
            assert_eq!(
                classify_http_error(status, body, None),
                HttpProblem::Rejected
            );
        }
    }

    #[cfg(feature = "remote-probe")]
    #[tokio::test]
    async fn native_remote_state_capacity_has_a_separate_streaming_limit() {
        use lomi_remote_crypto::{Identity, PeerApproval, Permissions, Role};
        let host = Identity::generate([1; 16], [2; 16], Role::Host, 1).unwrap();
        let device = Identity::generate([1; 16], [3; 16], Role::Device, 1).unwrap();
        let host_bundle = host.public_bundle();
        let device_bundle = device.public_bundle();
        let approval = host
            .sign_peer_approval(PeerApproval {
                version: 1,
                account_id: ([1; 16]),
                host_id: ([2; 16]),
                device_id: ([3; 16]),
                host_fingerprint: host_bundle.bundle.fingerprint().unwrap(),
                device_fingerprint: device_bundle.bundle.fingerprint().unwrap(),
                pairing_nonce: [5; 32],
                grant_id: ([4; 16]),
                session_ids: (0..32).map(|n| [n; 16]).collect(),
                workspace_id: None,
                workspace_epoch: None,
                session_epochs: vec![],
                permissions: Permissions::Observe,
                access_epoch: 1,
                revision: 1,
                expires_at: 1_800_000_000,
            })
            .unwrap();
        let channels:Vec<_>=(0..64).map(|n| serde_json::json!({"id":format!("channel-{n}"),"sessionId":"session","hostId":"host","deviceId":"device","grantId":"grant","context":{"version":1,"account_id":([1;16]),"host_id":([2;16]),"device_id":([3;16]),"initiator_role":"device","responder_role":"host","channel_id":([n;16]),"grant_id":([4;16]),"session_id":([0;16]),"access_epoch":1,"revision":1},"hostBundle":host_bundle,"deviceBundle":device_bundle,"signedApproval":approval,"expiresAt":"2026-10-01T00:00:00Z"})).collect();
        let body=serde_json::json!({"pairings":[],"grants":[],"channels":channels,"serverTime":"2026-09-30T12:00:00Z","authorizationExpiresAt":"2026-09-30T12:00:10Z"}).to_string();
        assert!(body.len() > RESPONSE_LIMIT);
        let (origin, server) = loopback_server(vec![
            reply("200 OK", "", &body),
            reply("200 OK", "", &"x".repeat(2 * 1024 * 1024 + 1)),
        ]);
        let mut api = test_api("https://auth.lomi.dev".into());
        api.remote_origin = Some(origin);
        let state = api
            .remote_request(
                "native-test",
                reqwest::Method::GET,
                "/v1/remote/native/hosts/host/state",
                None,
            )
            .await
            .unwrap();
        assert_eq!(state["channels"].as_array().unwrap().len(), 64);
        assert!(matches!(
            api.remote_request(
                "native-test",
                reqwest::Method::GET,
                "/v1/remote/native/hosts/host/state",
                None
            )
            .await,
            Err(HttpProblem::InvalidResponse)
        ));
        tokio::task::spawn_blocking(move || server.join())
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn oversized_responses_are_rejected_before_json_parsing() {
        let (origin, server) =
            loopback_server(vec![reply("200 OK", "", &"x".repeat(RESPONSE_LIMIT + 1))]);
        let api = test_api(origin);
        assert!(matches!(
            api.start_desktop(
                "http://127.0.0.1:43210/auth/callback",
                &"B".repeat(43),
                &"C".repeat(43),
            )
            .await,
            Err(HttpProblem::InvalidResponse)
        ));
        let _ = server.join().unwrap();
    }

    #[tokio::test]
    async fn redirects_are_rejected_and_never_followed() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let target_address = target.local_addr().unwrap();
        let location = format!("Location: http://{target_address}/capture\r\n");
        let (origin, server) = loopback_server(vec![reply("302 Found", &location, "{}")]);
        let api = test_api(origin);
        assert!(matches!(
            api.start_desktop(
                "http://127.0.0.1:43210/auth/callback",
                &"B".repeat(43),
                &"C".repeat(43),
            )
            .await,
            Err(HttpProblem::Rejected)
        ));
        let _ = server.join().unwrap();
        assert!(
            matches!(target.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }

    #[test]
    fn maps_http_statuses_and_retry_after_safely() {
        assert_eq!(
            classify_http_error(StatusCode::TOO_MANY_REQUESTS, b"{}", Some(12)),
            HttpProblem::RateLimited(Some(12))
        );
        assert_eq!(
            classify_http_error(StatusCode::UNAUTHORIZED, b"{}", None),
            HttpProblem::Unauthorized
        );
        assert_eq!(
            classify_http_error(StatusCode::FORBIDDEN, b"{}", None),
            HttpProblem::Forbidden
        );
        assert_eq!(
            classify_http_error(
                StatusCode::BAD_REQUEST,
                br#"{"error":{"code":"VALIDATION_FAILED"}}"#,
                None
            ),
            HttpProblem::Rejected
        );
    }

    #[tokio::test]
    async fn push_preserves_bounded_errors_and_fixed_native_transport() {
        let (origin, server) = loopback_server(vec![
            reply(
                "409 Conflict",
                "",
                r#"{"error":"BINDING_CHANGED","secret":"never-forward"}"#,
            ),
            reply("426 Upgrade Required", "", r#"{"error":"UPDATE_REQUIRED"}"#),
            reply(
                "429 Too Many Requests",
                "Retry-After: 17\r\n",
                r#"{"error":"RATE_LIMITED"}"#,
            ),
            reply(
                "503 Service Unavailable",
                "",
                r#"{"error":"token-from-untrusted-server"}"#,
            ),
        ]);
        let mut api = test_api("https://auth.lomi.dev".into());
        api.remote_origin = Some(origin);
        for (status, code, retry) in [
            (409, "BINDING_CHANGED", None),
            (426, "UPDATE_REQUIRED", None),
            (429, "RATE_LIMITED", Some(17)),
            (503, "SERVICE_UNAVAILABLE", None),
        ] {
            let error = api
                .push_request(
                    "test-token",
                    reqwest::Method::POST,
                    "/v1/remote/native/push/preferences",
                    Some(&serde_json::json!({"enabled":false})),
                )
                .await
                .unwrap_err();
            assert_eq!(error.status, Some(status));
            assert_eq!(error.code, code);
            assert_eq!(error.retry_after_seconds, retry);
            let serialized = serde_json::to_string(&error).unwrap();
            assert!(!serialized.contains("never-forward"));
            assert!(!serialized.contains("token-from-untrusted-server"));
        }
        for request in server.join().unwrap() {
            assert!(request.starts_with("POST /v1/remote/native/push/preferences "));
            let headers = request
                .split("\r\n\r\n")
                .next()
                .unwrap()
                .to_ascii_lowercase();
            assert!(headers.contains("authorization: bearer test-token\r\n"));
            assert!(headers.contains("x-lomi-remote-protocol: live-atomic-renew-v1\r\n"));
            for forbidden in ["cookie:", "origin:", "sec-fetch-site:"] {
                assert!(!headers.contains(forbidden));
            }
        }
        for path in [
            "/v1/remote/native/session",
            "/v1/remote/native/push/events/extra",
            "/v1/remote/native/push/preferences?token=x",
            "/v1/remote/native/push/../session",
            "https://evil.test/v1/remote/native/push/preferences",
        ] {
            assert!(api
                .push_request("test-token", reqwest::Method::GET, path, None)
                .await
                .is_err());
        }
        assert!(!push_route_allowed(
            &reqwest::Method::DELETE,
            "/v1/remote/native/push/preferences"
        ));
        assert!(!push_route_allowed(
            &reqwest::Method::GET,
            "/v1/remote/native/push/events"
        ));
        assert_eq!(
            push_http_error(StatusCode::TOO_MANY_REQUESTS, b"{}", Some(u64::MAX))
                .retry_after_seconds,
            Some(600)
        );
    }

    #[tokio::test]
    async fn push_redirect_never_forwards_authorization() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let location = format!(
            "Location: http://{}/capture\r\n",
            target.local_addr().unwrap()
        );
        let (origin, server) = loopback_server(vec![reply("302 Found", &location, "{}")]);
        let mut api = test_api("https://auth.lomi.dev".into());
        api.remote_origin = Some(origin);
        assert_eq!(
            api.push_request(
                "test-token",
                reqwest::Method::GET,
                "/v1/remote/native/push/preferences",
                None
            )
            .await
            .unwrap_err()
            .status,
            Some(302)
        );
        server.join().unwrap();
        assert!(
            matches!(target.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }

    #[test]
    fn session_parser_never_serializes_untrusted_fields_directly() {
        let wire: MeWire = serde_json::from_slice(
            br#"{"user":{"id":"u\n1","displayName":"Name\u0000 X","email":"a@example.com","githubLogin":"dev","status":"active"},"session":{"id":"s-1","expiresAt":"2026-10-01T00:00:00Z"},"token":"must-not-escape"}"#,
        )
        .unwrap();
        let user = AuthUser {
            id: safe_text(&wire.user.id, 128),
            display_name: safe_text(&wire.user.display_name, 160),
            email: safe_text(wire.user.email.as_deref().unwrap_or_default(), 254),
            github_login: safe_text(wire.user.github_login.as_deref().unwrap_or_default(), 64),
            status: safe_text(&wire.user.status, 32),
        };
        let serialized = serde_json::to_string(&user).unwrap();
        assert!(!serialized.contains("must-not-escape"));
        assert!(!user.id.contains('\n'));
        assert!(!user.display_name.contains('\0'));
    }
}
