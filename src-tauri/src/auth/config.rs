use reqwest::Url;

const DEFAULT_ORIGIN: &str = "https://auth.lomi.dev";
const DEFAULT_CLIENT_ID: &str = "lomi-desktop";

#[derive(Clone)]
pub(super) struct AuthConfig {
    pub origin: String,
    pub client_id: String,
    pub environment: String,
}

impl AuthConfig {
    pub fn from_build() -> Result<Self, String> {
        let host_identity_origin = option_env!("LOMI_HOST_IDENTITY_ORIGIN");
        if cfg!(debug_assertions) {
            Self::for_build(
                true,
                host_identity_origin.or(option_env!("LOMI_AUTH_ORIGIN")),
                option_env!("LOMI_AUTH_CLIENT_ID"),
            )
        } else {
            // The release auth issuer is the same public identity pin used by
            // the shared host. The OAuth client remains the desktop consumer
            // client; LOMI_HOST_CLIENT_ID belongs to host enrollment only.
            Self::new(
                host_identity_origin.unwrap_or(DEFAULT_ORIGIN),
                DEFAULT_CLIENT_ID,
                false,
            )
        }
    }

    pub fn is_host_identity_origin(&self) -> bool {
        option_env!("LOMI_HOST_IDENTITY_ORIGIN") == Some(self.origin.as_str())
    }

    fn for_build(
        is_debug: bool,
        origin_override: Option<&str>,
        client_id_override: Option<&str>,
    ) -> Result<Self, String> {
        let (origin, client_id) = if is_debug {
            (
                origin_override.unwrap_or(DEFAULT_ORIGIN),
                client_id_override.unwrap_or(DEFAULT_CLIENT_ID),
            )
        } else {
            (DEFAULT_ORIGIN, DEFAULT_CLIENT_ID)
        };
        Self::new(origin, client_id, is_debug)
    }

    fn new(origin: &str, client_id: &str, allow_local_http: bool) -> Result<Self, String> {
        if client_id.is_empty()
            || client_id.len() > 100
            || !client_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        {
            return Err("The account sign-in configuration is invalid.".into());
        }
        let parsed = parse_origin(origin, allow_local_http)?;
        let host = parsed
            .host_str()
            .ok_or_else(|| "The account sign-in configuration is invalid.".to_string())?;
        let environment = if is_loopback(host) {
            "development".to_string()
        } else {
            host.to_ascii_lowercase().replace('.', "-")
        };
        let mut canonical = parsed.to_string();
        if canonical.ends_with('/') {
            canonical.pop();
        }
        Ok(Self {
            origin: canonical,
            client_id: client_id.to_string(),
            environment,
        })
    }

    pub fn endpoint(&self, path: &str) -> Result<Url, String> {
        Url::parse(&format!("{}{}", self.origin, path))
            .map_err(|_| "The account service address is invalid.".to_string())
    }

    pub fn storage_namespace(&self) -> String {
        self.storage_namespace_for_build(cfg!(debug_assertions))
    }

    fn storage_namespace_for_build(&self, is_debug: bool) -> String {
        if is_debug {
            format!("{}-debug", self.environment)
        } else {
            self.environment.clone()
        }
    }

    pub fn remote_origin(&self) -> Result<String, String> {
        let auth = Url::parse(&self.origin).map_err(|_| "Invalid account origin.")?;
        let expected = match auth.host_str() {
            Some("auth.lomi.dev") => "https://remote.lomi.dev",
            Some("auth-staging.lomi.dev") => "https://remote-staging.lomi.dev",
            Some(host) if cfg!(debug_assertions) && is_loopback(host) => "http://127.0.0.1:3002",
            _ => return Err("Remote is not configured for this account environment.".into()),
        };
        let configured = if cfg!(debug_assertions) {
            option_env!("LOMI_REMOTE_ORIGIN").unwrap_or(expected)
        } else {
            expected
        };
        let parsed = Url::parse(configured).map_err(|_| "Invalid Remote origin.")?;
        if parsed.as_str().trim_end_matches('/') != expected
            && !(cfg!(debug_assertions)
                && expected == "http://127.0.0.1:3002"
                && parsed.as_str().trim_end_matches('/') == "http://127.0.0.1:4322")
        {
            return Err("Remote origin does not match the account environment.".into());
        }
        Ok(parsed.as_str().trim_end_matches('/').into())
    }

    pub fn desktop_authorization_url(&self, raw: &str) -> Result<String, String> {
        let parsed = Url::parse(raw)
            .map_err(|_| "The account service returned an invalid sign-in address.".to_string())?;
        let base = Url::parse(&self.origin)
            .map_err(|_| "The account service address is invalid.".to_string())?;
        if parsed.origin() != base.origin()
            || parsed.path() != "/desktop"
            || parsed.fragment().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err("The account service returned an untrusted sign-in address.".into());
        }
        let request = parsed
            .query()
            .and_then(|query| query.strip_prefix("request="))
            .filter(|request| valid_request_id(request))
            .ok_or_else(|| {
                "The account service returned an untrusted sign-in address.".to_string()
            })?;
        if parsed.as_str() != format!("{}/desktop?request={request}", self.origin) {
            return Err("The account service returned an untrusted sign-in address.".into());
        }
        Ok(parsed.to_string())
    }

    pub fn account_portal_uri(&self) -> Result<String, String> {
        self.endpoint("/account").map(|url| url.to_string())
    }
}

fn valid_request_id(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn parse_origin(origin: &str, allow_local_http: bool) -> Result<Url, String> {
    let parsed = Url::parse(origin)
        .map_err(|_| "The account sign-in configuration is invalid.".to_string())?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "The account sign-in configuration is invalid.".to_string())?;
    let local = is_loopback(host);
    let secure_owned_host = host == "lomi.dev" || host.ends_with(".lomi.dev");
    let valid_scheme = if parsed.scheme() == "https" {
        parsed.port().is_none_or(|port| port == 443) && secure_owned_host
    } else {
        allow_local_http
            && parsed.scheme() == "http"
            && local
            && parsed.port_or_known_default() == Some(4321)
    };
    if !valid_scheme
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("The account sign-in origin is not trusted.".into());
    }
    Ok(parsed)
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1")
}
