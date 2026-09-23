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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_normalization_preserves_scheme_host_and_port_boundaries() {
        let origin = Origin::parse("https://EXAMPLE.com:443/").unwrap();
        assert_eq!(origin.as_str(), "https://example.com");
        for allowed in ["https://example.com/a?b=c#d", "https://example.com:443/"] {
            assert!(origin.permits(&Url::parse(allowed).unwrap()));
        }
        for denied in [
            "http://example.com/",
            "https://example.com:444/",
            "https://example.com.evil/",
            "https://sub.example.com/",
            "https://example.com./",
            "https://user@example.com/",
            "about:blank",
            "file:///example.com",
        ] {
            assert!(!origin.permits(&Url::parse(denied).unwrap()), "{denied}");
        }
        let local = Origin::parse("http://localhost:3000").unwrap();
        for denied in [
            "http://127.0.0.1:3000",
            "http://localhost:3001",
            "http://192.168.1.2:3000",
        ] {
            assert!(!local.permits(&Url::parse(denied).unwrap()));
        }
        assert_eq!(
            Origin::parse("http://[::1]:3000/").unwrap().as_str(),
            "http://[::1]:3000"
        );
        assert_eq!(
            Origin::parse("https://żółć.example").unwrap().as_str(),
            "https://xn--kda4b0koi.example"
        );
    }

    #[test]
    fn internal_hosts_and_ambiguous_approvals_are_rejected() {
        for host in INTERNAL_HOSTS {
            for value in [
                format!("http://{host}"),
                format!("https://nested.{host}.:443/"),
            ] {
                assert!(address(&value).is_err(), "{value}");
            }
        }
        for value in [
            "https://example.com/path",
            "https://example.com/?token=secret",
            "https://example.com/#x",
            "https://user:pass@example.com",
            "https://*.example.com",
            "about:blank",
            "javascript:alert(1)",
            "https://exam\nple.com",
        ] {
            assert!(Origin::parse(value).is_err(), "{value}");
        }
        assert!(address("about:blank").is_ok());
        assert!(address("https://plugin.localhost.example/page").is_ok());
    }
}
