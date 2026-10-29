use super::catalog::Package;
use ring::digest::{Context, SHA1_FOR_LEGACY_USE_ONLY};
use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};
use tokio::{io::AsyncWriteExt, sync::watch};

const EXPANDED_LIMIT: u64 = 16 * 1024 * 1024 * 1024;
const ENTRY_LIMIT: usize = 100_000;

#[derive(Debug)]
pub struct ArchiveSummary {
    pub expanded_bytes: u64,
}

/// The caller creates an exclusive operation directory and has already obtained consent.
/// Nothing is published to the SDK; cancellation leaves only this operation's partial file.
pub async fn package(
    package: &Package,
    target: &Path,
    cancel: watch::Receiver<bool>,
    progress: impl Fn(u64, u64),
) -> Result<ArchiveSummary, String> {
    let url = reqwest::Url::parse(&package.url).map_err(|e| e.to_string())?;
    if url.scheme() != "https"
        || url.host_str() != Some("dl.google.com")
        || !url.path().starts_with("/android/repository/")
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || package.size == 0
        || package.size > 12 * 1024 * 1024 * 1024
        || package.sha1.len() != 40
        || !package.sha1.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("Invalid official Android archive descriptor".into());
    }
    verified(
        &package.url,
        package.size,
        &package.sha1,
        &SHA1_FOR_LEGACY_USE_ONLY,
        target,
        cancel.clone(),
        &progress,
    )
    .await?;
    let path = target.to_path_buf();
    let inspection_cancel = cancel.clone();
    let summary = tauri::async_runtime::spawn_blocking(move || {
        inspect_with_cancel(&path, || *inspection_cancel.borrow())
    })
    .await
    .map_err(|e| e.to_string())??;
    if *cancel.borrow() {
        return Err("Android download cancelled".into());
    }
    Ok(summary)
}

/// Only trusted provider descriptors reach this helper; redirects stay on provider CDNs.
pub(super) async fn verified(
    source: &str,
    size: u64,
    checksum: &str,
    algorithm: &'static ring::digest::Algorithm,
    target: &Path,
    mut cancel: watch::Receiver<bool>,
    progress: impl Fn(u64, u64),
) -> Result<(), String> {
    let url = reqwest::Url::parse(source).map_err(|e| e.to_string())?;
    if !official_url(&url) || size == 0 || size > 12 * 1024 * 1024 * 1024 {
        return Err("Invalid official Android tool download".into());
    }
    if *cancel.borrow() {
        return Err("Android download cancelled".into());
    }
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let url = attempt.url();
            if attempt.previous().len() < 5 && official_url(url) {
                attempt.follow()
            } else {
                attempt.error("Untrusted Android tool redirect")
            }
        }))
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(20 * 60))
        .user_agent("Lomi Android installer")
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = tokio::select! {
        result = client.get(url).send() => result.map_err(|e| e.to_string())?.error_for_status().map_err(|e| e.to_string())?,
        _ = cancel.changed() => return Err("Android download cancelled".into()),
    };
    if response.status() != reqwest::StatusCode::OK
        || response
            .content_length()
            .is_some_and(|actual| actual != size)
    {
        return Err("The Android provider returned an unexpected archive size or response. Refresh the catalog and retry.".into());
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target)
        .await
        .map_err(|e| e.to_string())?;
    let mut digest = Context::new(algorithm);
    let mut received = 0_u64;
    let mut last_progress = Instant::now();
    progress(0, size);
    loop {
        if *cancel.borrow() {
            return Err("Android download cancelled".into());
        }
        let chunk = tokio::select! {
            result = response.chunk() => result.map_err(|e| e.to_string())?,
            _ = cancel.changed() => return Err("Android download cancelled".into()),
        };
        let Some(bytes) = chunk else { break };
        received = received
            .checked_add(bytes.len() as u64)
            .ok_or("Android archive size overflow")?;
        if received > size {
            return Err("Android download exceeded the catalog size.".into());
        }
        digest.update(&bytes);
        file.write_all(&bytes).await.map_err(|e| e.to_string())?;
        if last_progress.elapsed() >= Duration::from_millis(100) {
            progress(received, size);
            last_progress = Instant::now();
        }
    }
    let actual = hex(digest.finish().as_ref());
    if received != size || actual != checksum.to_ascii_lowercase() {
        return Err(
            "Android archive integrity check failed. Discard this download and retry.".into(),
        );
    }
    file.sync_all().await.map_err(|e| e.to_string())?;
    drop(file);
    if *cancel.borrow() {
        return Err("Android download cancelled".into());
    }
    progress(received, size);
    Ok(())
}

fn official_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.fragment().is_none()
        && match url.host_str() {
            Some("dl.google.com") => url.query().is_none() && url.path().starts_with("/android/"),
            Some("github.com") => {
                url.query().is_none()
                    && url
                        .path()
                        .starts_with("/adoptium/temurin21-binaries/releases/download/")
            }
            Some("release-assets.githubusercontent.com" | "objects.githubusercontent.com") => true,
            _ => false,
        }
}

pub(super) fn inspect_with_cancel(
    path: &Path,
    cancelled: impl Fn() -> bool,
) -> Result<ArchiveSummary, String> {
    let started = Instant::now();
    let check = || {
        if cancelled() {
            Err("Android archive verification cancelled".to_string())
        } else if started.elapsed() >= Duration::from_secs(20 * 60) {
            Err("Android archive verification exceeded its deadline".to_string())
        } else {
            Ok(())
        }
    };
    check()?;
    let mut archive = zip::ZipArchive::new(File::open(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if archive.is_empty() || archive.len() > ENTRY_LIMIT {
        return Err("Android archive has an invalid entry count".into());
    }
    let mut entries = BTreeMap::<String, Entry>::new();
    let mut expanded_bytes = 0_u64;
    let mut buffer = [0; 64 * 1024];
    for index in 0..archive.len() {
        check()?;
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().trim_end_matches('/').to_string();
        let parts = safe_parts(&name)?;
        let key = parts.join("/").to_lowercase();
        if entries.contains_key(&key) {
            return Err("Android archive contains colliding paths".into());
        }
        expanded_bytes = expanded_bytes
            .checked_add(entry.size())
            .filter(|n| *n <= EXPANDED_LIMIT)
            .ok_or("Android archive exceeds the expanded size limit")?;
        let mode = entry.unix_mode().unwrap_or(0) & 0o170000;
        if !matches!(mode, 0 | 0o100000 | 0o040000 | 0o120000) {
            return Err("Android archive contains a special device or pipe".into());
        }
        let link = if mode == 0o120000 {
            if entry.size() == 0 || entry.size() > 4096 {
                return Err("Android archive link exceeds its limit".into());
            }
            let mut value = String::new();
            (&mut entry)
                .take(4097)
                .read_to_string(&mut value)
                .map_err(|e| e.to_string())?;
            if value.starts_with('/') || value.contains(['\\', '\0', ':']) {
                return Err("Android archive link escapes its directory".into());
            }
            let mut target = parts[..parts.len() - 1]
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>();
            for part in value.split('/') {
                match part {
                    "" | "." => {}
                    ".." => {
                        target
                            .pop()
                            .ok_or("Android archive link escapes its directory")?;
                    }
                    part => {
                        safe_parts(part)?;
                        target.push(part.into());
                    }
                }
            }
            if target.is_empty() {
                return Err("Android archive link points at the extraction root".into());
            }
            Some(target.join("/").to_lowercase())
        } else {
            let expected = entry.size();
            let mut bounded = (&mut entry).take(expected + 1);
            let mut actual = 0;
            loop {
                check()?;
                let count = bounded.read(&mut buffer).map_err(|e| e.to_string())?;
                if count == 0 {
                    break;
                }
                actual += count as u64;
            }
            if actual != expected {
                return Err("Android archive entry does not match its declared size".into());
            }
            None
        };
        entries.insert(
            key,
            Entry {
                directory: entry.is_dir(),
                link,
            },
        );
    }
    for (key, entry) in &entries {
        check()?;
        let parts: Vec<_> = key.split('/').collect();
        for length in 1..parts.len() {
            if entries
                .get(&parts[..length].join("/"))
                .is_some_and(|parent| !parent.directory || parent.link.is_some())
            {
                return Err(
                    "Android archive contains an entry below a file or symbolic link".into(),
                );
            }
        }
        if let Some(target) = &entry.link {
            let mut next = target;
            let mut depth = 0;
            loop {
                depth += 1;
                if depth > 32 || next == key {
                    return Err("Android archive contains a cyclic link".into());
                }
                let Some(target) = entries.get(next) else {
                    // Some archives omit explicit directory records.
                    if entries
                        .keys()
                        .any(|name| name.starts_with(&(next.clone() + "/")))
                    {
                        break;
                    }
                    return Err("Android archive contains a dangling link".into());
                };
                match &target.link {
                    Some(link) => next = link,
                    None => break,
                }
            }
        }
    }
    Ok(ArchiveSummary { expanded_bytes })
}

struct Entry {
    directory: bool,
    link: Option<String>,
}

pub(super) fn safe_parts(name: &str) -> Result<Vec<&str>, String> {
    if name.len() > 1024 || name.contains(['\\', ':', '\0']) {
        return Err("Unsafe Android archive path".into());
    }
    let parts: Vec<_> = name.split('/').collect();
    for part in &parts {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if part.is_empty()
            || matches!(*part, "." | "..")
            || part.ends_with(['.', ' '])
            || part.chars().any(char::is_control)
            || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.len() == 4
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err("Unsafe or nonportable Android archive path".into());
        }
    }
    Ok(parts)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
