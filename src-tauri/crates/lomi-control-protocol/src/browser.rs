//! Shared URL validation for human and agent browser navigation. An origin
//! grant restricts top-level navigation; it does not isolate subresource traffic.
use url::Url;

const INTERNAL_HOSTS: &[&str] = &[
    "tauri.localhost",
    "ipc.localhost",
    "theme.localhost",
    "plugin.localhost",
];

pub fn address(value: &str) -> Result<Url, &'static str> {
    if value.len() > 16_384 || value.chars().any(char::is_control) {
        return Err("Invalid or oversized web address.");
    }
    let url = Url::parse(value).map_err(|_| "Invalid web address.")?;
    if value == "about:blank" {
        return Ok(url);
    }
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none_or(|host| {
            let host = host.trim_end_matches('.');
            host.contains('*')
                || INTERNAL_HOSTS
                    .iter()
                    .any(|internal| host == *internal || host.ends_with(&format!(".{internal}")))
        })
    {
        return Err("Only HTTP and HTTPS pages can be opened.");
    }
    Ok(url)
}

/// Store only a normalized scheme/host/effective-port tuple. Deserialization of
/// a UI approval must call `parse`; a URL with a path is not an origin grant.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Origin(String);

impl Origin {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        let url = address(value)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("Enter an HTTP or HTTPS origin without a path, query or fragment.");
        }
        Ok(Self(url.origin().ascii_serialization()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn permits(&self, url: &Url) -> bool {
        address(url.as_str()).is_ok()
            && matches!(url.scheme(), "http" | "https")
            && url.origin().ascii_serialization() == self.0
    }
}
