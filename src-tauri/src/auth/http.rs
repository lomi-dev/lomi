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
