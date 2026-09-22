use super::catalog::{self, Catalog, Host};
use sha2::{Digest, Sha256};
use std::time::Duration;

const SOURCES: [&str; 4] = [
    "https://dl.google.com/android/repository/repository2-3.xml",
    "https://dl.google.com/android/repository/sys-img/android/sys-img2-3.xml",
    "https://dl.google.com/android/repository/sys-img/google_apis/sys-img2-3.xml",
    "https://dl.google.com/android/repository/sys-img/google_apis_playstore/sys-img2-3.xml",
];

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub revision: String,
    pub packages: Vec<catalog::Package>,
    pub licenses: Vec<catalog::License>,
}

/// Called only by explicit Android setup/catalog actions. No CLI, JVM or ADB is
/// needed to display provider terms and download sizes before consent.
pub async fn fetch(host: Host) -> Result<Snapshot, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(60))
        .user_agent("Lomi Android catalog")
        .build()
        .map_err(|e| e.to_string())?;
    let (tools, aosp, google, play) = tokio::try_join!(
        source(&client, SOURCES[0], host),
        source(&client, SOURCES[1], host),
        source(&client, SOURCES[2], host),
        source(&client, SOURCES[3], host),
    )?;
    merge([tools, aosp, google, play])
}

async fn source(client: &reqwest::Client, source: &str, host: Host) -> Result<Catalog, String> {
    let mut response = client
        .get(source)
        .send()
        .await
        .map_err(|e| format!("Cannot load Android catalog: {e}. Check the connection and retry."))?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    if response.status() != reqwest::StatusCode::OK {
        return Err("Unexpected Android catalog response".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
            return Err("Android catalog exceeds 8 MiB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let xml = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
    let base = source
        .rsplit_once('/')
        .ok_or("Invalid catalog source")?
        .0
        .to_string()
        + "/";
    catalog::packages(xml, &base, host)
}

fn merge(catalogs: impl IntoIterator<Item = Catalog>) -> Result<Snapshot, String> {
    let mut snapshot = Snapshot {
        revision: String::new(),
        packages: Vec::new(),
        licenses: Vec::new(),
    };
    for catalog in catalogs {
        for license in catalog.licenses {
            if let Some(existing) = snapshot
                .licenses
                .iter()
                .find(|existing| existing.id == license.id)
            {
                if existing.digest != license.digest {
                    return Err("Android providers returned conflicting license text. Refresh the catalog before installing.".into());
                }
            } else {
                snapshot.licenses.push(license);
            }
        }
        for package in catalog.packages {
            if snapshot
                .packages
                .iter()
                .any(|existing| existing.id == package.id && existing.revision == package.revision)
            {
                return Err("Android catalog contains duplicate package revisions".into());
            }
            snapshot.packages.push(package);
        }
    }
    snapshot
        .packages
        .sort_by(|a, b| (&a.id, &a.revision).cmp(&(&b.id, &b.revision)));
    snapshot.licenses.sort_by(|a, b| a.id.cmp(&b.id));
    snapshot.revision = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?)
    );
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    #[ignore = "Reads current official Android SDK and image catalogs"]
    async fn actual_catalogs_are_compatible_and_share_exact_license_text() {
        let snapshot = super::fetch(super::Host::native().unwrap()).await.unwrap();
        assert!(snapshot.packages.iter().any(|p| p.id == "emulator"));
        for tag in ["default", "google_apis", "google_apis_playstore"] {
            assert!(snapshot
                .packages
                .iter()
                .any(|p| p.image.as_ref().is_some_and(|image| image.tag == tag)));
        }
        println!(
            "Catalog {}: {} packages, {} licenses",
            snapshot.revision,
            snapshot.packages.len(),
            snapshot.licenses.len()
        );
    }
}
