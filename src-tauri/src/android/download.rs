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
    #[cfg(test)]
    pub entries: usize,
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

/// Bound extraction before the selected SDK manager sees an archive. Link ancestors,
/// escaping targets and case-insensitive collisions are rejected on every host.
#[cfg(test)]
pub fn inspect(path: &Path) -> Result<ArchiveSummary, String> {
    inspect_with_cancel(path, || false)
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
    Ok(ArchiveSummary {
        #[cfg(test)]
        entries: entries.len(),
        expanded_bytes,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use zip::{write::SimpleFileOptions, ZipWriter};

    #[test]
    fn cancellation_interrupts_verification_inside_a_large_entry() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("large.zip");
        let mut writer = ZipWriter::new(File::create(&path).unwrap());
        writer
            .start_file("root/large", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&vec![0; 8 * 1024 * 1024]).unwrap();
        writer.finish().unwrap();
        let calls = AtomicUsize::new(0);
        let result = inspect_with_cancel(&path, || calls.fetch_add(1, Ordering::Relaxed) >= 4);
        assert!(result.unwrap_err().contains("cancelled"));
        assert_eq!(inspect(&path).unwrap().expanded_bytes, 8 * 1024 * 1024);
    }

    #[test]
    fn rejects_traversal_link_ancestors_and_cross_platform_collisions() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("archive.zip");
        for names in [
            vec!["../escape"],
            vec!["root/file", "root/FILE"],
            vec!["root/CON.txt"],
            vec!["root/file", "root/file/child"],
        ] {
            let mut writer = ZipWriter::new(File::create(&path).unwrap());
            for name in names {
                writer
                    .start_file(name, SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(b"data").unwrap();
            }
            writer.finish().unwrap();
            assert!(inspect(&path).is_err());
        }
        for target in ["../../outside", "/outside", "link"] {
            let mut writer = ZipWriter::new(File::create(&path).unwrap());
            writer
                .add_symlink("root/link", target, SimpleFileOptions::default())
                .unwrap();
            writer.finish().unwrap();
            assert!(inspect(&path).is_err());
        }
        let mut writer = ZipWriter::new(File::create(&path).unwrap());
        writer
            .add_symlink("root/link", "directory", SimpleFileOptions::default())
            .unwrap();
        writer
            .start_file("root/link/payload", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"data").unwrap();
        writer.finish().unwrap();
        assert!(inspect(&path).is_err());
    }

    #[test]
    #[ignore = "Uses the explicitly accepted isolated native trial's official archives"]
    fn actual_archives_pass_before_manager_extraction() {
        let root = PathBuf::from(std::env::var_os("LOMI_ANDROID_PROBE_DIRECTORY").unwrap());
        let packages: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("evidence/packages.json")).unwrap())
                .unwrap();
        for package in packages.as_array().unwrap() {
            let name = package["url"].as_str().unwrap().rsplit('/').next().unwrap();
            let path = root.join("cache").join(name);
            let summary = inspect(&path).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert!(summary.entries > 0 && summary.expanded_bytes > 0);
            let mut file = File::open(path).unwrap();
            let mut digest = Context::new(&SHA1_FOR_LEGACY_USE_ONLY);
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let read = file.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                digest.update(&buffer[..read]);
            }
            assert_eq!(
                hex(digest.finish().as_ref()),
                package["sha1"].as_str().unwrap()
            );
        }
    }

    #[tokio::test]
    #[ignore = "Downloads one official archive into the accepted isolated fixture"]
    async fn actual_download_has_progress_integrity_and_cancellation() {
        let root = PathBuf::from(std::env::var_os("LOMI_ANDROID_PROBE_DIRECTORY").unwrap());
        let consent: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("evidence/consent.json")).unwrap())
                .unwrap();
        assert_eq!(consent["accepted"], true);
        let xml = std::fs::read_to_string(root.join("evidence/repository2-3.xml")).unwrap();
        let catalog = super::super::catalog::packages(
            &xml,
            "https://dl.google.com/android/repository/",
            super::super::catalog::Host::native().unwrap(),
        )
        .unwrap();
        let descriptor = catalog
            .packages
            .iter()
            .find(|p| p.id == "platform-tools" && p.revision == "37.0.1")
            .unwrap();
        let directory = tempfile::tempdir_in(root.join("staging")).unwrap();
        let path = directory.path().join("package.zip");
        let (sender, cancel) = watch::channel(false);
        let progress = std::sync::Mutex::new(Vec::new());
        let summary = package(descriptor, &path, cancel, |received, total| {
            progress.lock().unwrap().push((received, total))
        })
        .await
        .unwrap();
        assert!(summary.entries > 0);
        {
            let values = progress.lock().unwrap();
            assert_eq!(values.first(), Some(&(0, descriptor.size)));
            assert_eq!(values.last(), Some(&(descriptor.size, descriptor.size)));
            assert!(values.windows(2).all(|pair| pair[0].0 <= pair[1].0));
        }
        sender.send_replace(true);
        assert!(package(
            descriptor,
            &directory.path().join("cancelled.zip"),
            sender.subscribe(),
            |_, _| {}
        )
        .await
        .unwrap_err()
        .contains("cancelled"));
        assert!(!directory.path().join("cancelled.zip").exists());
        sender.send_replace(false);
        let interrupted = directory.path().join("interrupted.zip");
        let result = package(
            descriptor,
            &interrupted,
            sender.subscribe(),
            |received, total| {
                if received > 0 && received < total {
                    sender.send_replace(true);
                }
            },
        )
        .await;
        assert!(result.unwrap_err().contains("cancelled"));
        assert!(std::fs::metadata(interrupted).unwrap().len() < descriptor.size);
    }
}
