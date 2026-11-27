//! Debug-only extra trust anchor for a local development stack (for example a
//! Caddy `tls internal` CA). This module is compiled out of release builds.
//! Certificate and hostname verification stay enabled; the CA is only added to
//! the root store. Any misconfiguration fails closed. Mirrors the Host's
//! `lomi-host-http` dev CA; the variable differs so that the desktop and the
//! Host opt in separately.
use rustls::pki_types::{pem::PemObject, CertificateDer};
use std::path::Path;

const ENV: &str = "LOMI_DESKTOP_DEV_EXTRA_CA";

/// `None` when the variable is unset; an error when it is set but unusable.
pub(crate) fn extra_roots() -> Result<Option<Vec<CertificateDer<'static>>>, &'static str> {
    let Some(path) = std::env::var_os(ENV) else {
        return Ok(None);
    };
    load(Path::new(&path)).map(Some)
}

fn load(path: &Path) -> Result<Vec<CertificateDer<'static>>, &'static str> {
    const INVALID: &str = "invalid dev CA";
    if !path.is_absolute() {
        return Err(INVALID);
    }
    let certs = CertificateDer::pem_file_iter(path)
        .and_then(|iter| iter.collect::<Result<Vec<_>, _>>())
        .map_err(|_| INVALID)?;
    if certs.is_empty() {
        return Err(INVALID);
    }
    Ok(certs)
}

/// System trust plus the extra CA for account/Remote HTTP.
pub(crate) fn reqwest_builder(
    builder: reqwest::ClientBuilder,
) -> Result<reqwest::ClientBuilder, &'static str> {
    let Some(roots) = extra_roots()? else {
        return Ok(builder);
    };
    roots.iter().try_fold(builder, |builder, der| {
        let cert = reqwest::Certificate::from_der(der).map_err(|_| "invalid dev CA")?;
        Ok(builder.add_root_certificate(cert))
    })
}

/// Bundled web roots plus the extra CA for the relay link; `None` keeps the default connector.
#[cfg(feature = "remote-host")]
pub(crate) fn relay_connector() -> Result<Option<tokio_tungstenite::Connector>, &'static str> {
    use std::sync::Arc;
    let Some(extra) = extra_roots()? else {
        return Ok(None);
    };
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for der in extra {
        roots.add(der).map_err(|_| "invalid dev CA")?;
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| "invalid dev CA")?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(Some(tokio_tungstenite::Connector::Rustls(Arc::new(config))))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CERT: &str = "-----BEGIN CERTIFICATE-----
MIIB+zCCAaICCQDwyhXUT4UXEDAKBggqhkjOPQQDAjAMMQowCAYDVQQDDAF0MB4XDTI2MTEyNjIxMDgzMFoXDTM2MTEyMzIxMDgzMFowDDEKMAgGA1UEAwwBdDCCAUswggEDBgcqhkjOPQIBMIH3AgEBMCwGByqGSM49AQECIQD/////AAAAAQAAAAAAAAAAAAAAAP///////////////zBbBCD/////AAAAAQAAAAAAAAAAAAAAAP///////////////AQgWsY12Ko6k+ez671VdpiGvGUdBrDMU7D2O848PifSYEsDFQDEnTYIhucEk2pmeOETnSa3gZ9+kARBBGsX0fLhLEJH+Lzm5WOkQPJ3A32BLeszoPShOUXYmMKWT+NC4v4af5uO5+tKfA+eFivOM1drMV7Oy7ZAaDe/UfUCIQD/////AAAAAP//////////vOb6racXnoTzucrC/GMlUQIBAQNCAAQ/RbKuxumzeqLLBYY4za+ICBscrjv3zubP1TAXTfny/GlszQJPmesPQeR+vCWH6EEZDmTHkv9VNhTmC3RB8JLcMAoGCCqGSM49BAMCA0cAMEQCIHe0K6IHYsA+2gjXCtQd+g/1/SZ951ydfG4/22a+d9ZyAiAqN7glXvzbnj4mN5IiTyOE8XlzdHX6GDdZ2StGgC+OdA==
-----END CERTIFICATE-----
";

    fn temp(name: &str, body: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("lomi-dev-ca-{}-{name}", std::process::id()));
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn loads_every_certificate_in_a_pem_file() {
        let path = temp("ok.pem", &format!("{CERT}{CERT}"));
        assert_eq!(load(&path).unwrap().len(), 2);
    }

    #[test]
    fn invalid_missing_empty_or_relative_files_fail_closed() {
        let garbage = temp("bad.pem", "not a pem");
        let empty = temp("empty.pem", "");
        let bad_base64 = temp(
            "b64.pem",
            "-----BEGIN CERTIFICATE-----\n!!!\n-----END CERTIFICATE-----\n",
        );
        for path in [garbage, empty, bad_base64, "/nonexistent/ca.pem".into()] {
            assert!(load(&path).is_err());
        }
        assert!(load(Path::new("relative.pem")).is_err());
    }

    #[test]
    fn loaded_certificate_is_accepted_by_reqwest() {
        let path = temp("reqwest.pem", CERT);
        for der in load(&path).unwrap() {
            reqwest::Certificate::from_der(&der).unwrap();
        }
    }

    // Release builds must not reference the module: the nearest cfg above every use is the debug one.
    #[test]
    fn every_use_is_gated_on_debug_assertions() {
        for (name, source) in [
            ("auth/http.rs", include_str!("auth/http.rs")),
            ("remote_core/mod.rs", include_str!("remote_core/mod.rs")),
            ("lib.rs", include_str!("lib.rs")),
        ] {
            let lines: Vec<&str> = source.lines().collect();
            let mut uses = 0;
            for (i, line) in lines.iter().enumerate() {
                if line.contains("dev_ca::") || line.trim() == "mod dev_ca;" {
                    uses += 1;
                    let cfg = lines[..i].iter().rev().find(|l| l.contains("cfg("));
                    assert_eq!(
                        cfg.map(|l| l.trim()),
                        Some("#[cfg(debug_assertions)]"),
                        "{name}:{}",
                        i + 1
                    );
                }
            }
            assert!(uses > 0, "{name} no longer uses the dev CA");
        }
    }
}
