//! Speech model catalog state, verified downloads and deletion.
//!
//! A model file is trusted only after its SHA-256 matched; the `.verified`
//! marker records that for the file's exact size and mtime, so startup never
//! rehashes gigabytes.

use std::collections::{HashMap, VecDeque};
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, UNIX_EPOCH};

use lomi_stt::catalog::{self, Model};
use reqwest::header::{CONTENT_RANGE, RANGE};
use ring::digest::{Context, SHA256};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{Manager, State, Window};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::watch;

use crate::files::main_window;
use crate::storage_guard::reject_link;

const BASE_URL: &str = "https://huggingface.co";
const RETRIES: u32 = 3;
const STALE_PART: Duration = Duration::from_secs(7 * 24 * 3600);

#[derive(Debug)]
pub struct DownloadError {
    code: &'static str,
    message: String,
    retry: bool,
}

impl DownloadError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retry: false,
        }
    }

    /// Stable identifier for the UI: cancelled, network, integrity, disk-full, storage.
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<std::io::Error> for DownloadError {
    fn from(e: std::io::Error) -> Self {
        // 28 is ENOSPC on Unix; Rust maps it to StorageFull on current toolchains.
        let full = e.kind() == std::io::ErrorKind::StorageFull || e.raw_os_error() == Some(28);
        Self::new(if full { "disk-full" } else { "storage" }, e.to_string())
    }
}

fn network(e: reqwest::Error) -> DownloadError {
    DownloadError {
        retry: true,
        ..DownloadError::new("network", e.without_url().to_string())
    }
}

fn cancelled() -> DownloadError {
    DownloadError::new("cancelled", "Download cancelled")
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: &'static str,
    pub bytes: u64,
    pub total: u64,
    pub bytes_per_sec: u64,
}

/// Throttles to one event per 100 ms and measures speed over the last ~5 s.
struct Reporter<'a> {
    emit: &'a (dyn Fn(Progress) + Sync),
    last: Option<Instant>,
    window: VecDeque<(Instant, u64)>,
}

impl Reporter<'_> {
    fn report(&mut self, phase: &'static str, bytes: u64, total: u64, force: bool) {
        let now = Instant::now();
        if !force
            && self
                .last
                .is_some_and(|l| now.duration_since(l) < Duration::from_millis(100))
        {
            return;
        }
        self.last = Some(now);
        self.window.push_back((now, bytes));
        while self.window.len() > 1 && now.duration_since(self.window[0].0) > Duration::from_secs(5)
        {
            self.window.pop_front();
        }
        let (start, from) = self.window[0];
        let secs = now.duration_since(start).as_secs_f64();
        let bytes_per_sec = if secs > 0.0 {
            (bytes.saturating_sub(from) as f64 / secs) as u64
        } else {
            0
        };
        (self.emit)(Progress {
            phase,
            bytes,
            total,
            bytes_per_sec,
        });
    }
}

pub fn path(dir: &Path, model: &Model) -> PathBuf {
    dir.join(&model.filename)
}

fn part_path(dir: &Path, model: &Model) -> PathBuf {
    dir.join(format!("{}.part", model.filename))
}

fn marker_path(dir: &Path, model: &Model) -> PathBuf {
    dir.join(format!("{}.verified", model.filename))
}

fn stamp(meta: &std::fs::Metadata) -> Option<String> {
    let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(format!("{} {}", meta.len(), mtime.as_secs()))
}

pub fn is_ready(dir: &Path, model: &Model) -> bool {
    let Ok(meta) = path(dir, model).symlink_metadata() else {
        return false;
    };
    meta.is_file()
        && meta.len() == model.size
        && stamp(&meta).is_some_and(|s| {
            std::fs::read_to_string(marker_path(dir, model)).is_ok_and(|m| m.trim() == s)
        })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Exact suffix match: `evilhf.co` and `hf.co.evil.com` must not pass.
fn host_allowed(host: &str) -> bool {
    host == "huggingface.co" || host.ends_with(".hf.co")
}

async fn cancel_requested(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow() {
            return;
        }
        if cancel.changed().await.is_err() {
            // Sender dropped: nobody can cancel any more.
            std::future::pending::<()>().await;
        }
    }
}

pub async fn download(
    dir: &Path,
    model: &Model,
    cancel: watch::Receiver<bool>,
    progress: impl Fn(Progress) + Sync,
) -> Result<(), DownloadError> {
    download_from(BASE_URL, false, dir, model, cancel, progress).await
}

/// `loopback` lets tests use plain HTTP on 127.0.0.1; production passes false.
async fn download_from(
    base_url: &str,
    loopback: bool,
    dir: &Path,
    model: &Model,
    mut cancel: watch::Receiver<bool>,
    progress: impl Fn(Progress) + Sync,
) -> Result<(), DownloadError> {
    let target = path(dir, model);
    let part = part_path(dir, model);
    reject_link(dir).map_err(|e| DownloadError::new("storage", e))?;
    tokio::fs::create_dir_all(dir).await?;
    for p in [dir, &target, &part] {
        reject_link(p).map_err(|e| DownloadError::new("storage", e))?;
    }

    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::builder()
        .https_only(!loopback)
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            let host = attempt.url().host_str().unwrap_or_default();
            if attempt.previous().len() < 5
                && (host_allowed(host) || (loopback && host == "127.0.0.1"))
            {
                attempt.follow()
            } else {
                attempt.error("Untrusted model redirect")
            }
        }))
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(60))
        .user_agent("Lomi model downloader")
        .build()
        .map_err(network)?;
    let url = format!(
        "{base_url}/{}/resolve/{}/{}",
        model.repository, model.revision, model.filename
    );
    let mut reporter = Reporter {
        emit: &progress,
        last: None,
        window: VecDeque::new(),
    };

    let mut tries = 0;
    let result = loop {
        reporter.window.clear();
        match attempt(
            &client,
            &url,
            model,
            &target,
            &part,
            &mut cancel,
            &mut reporter,
        )
        .await
        {
            Err(e) if e.retry && tries < RETRIES => {
                let backoff = Duration::from_secs(1 << tries);
                tries += 1;
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = cancel_requested(&mut cancel) => break Err(cancelled()),
                }
            }
            other => break other,
        }
    };
    match &result {
        Err(e) if e.code == "cancelled" => {
            let _ = tokio::fs::remove_file(&part).await;
        }
        Err(e) if e.code != "integrity" => eprintln!("lomi: model download failed: {e}"),
        _ => {}
    }
    result
}

/// One pass: hash what the part already holds, fetch the rest, verify, publish.
async fn attempt(
    client: &reqwest::Client,
    url: &str,
    model: &Model,
    target: &Path,
    part: &Path,
    cancel: &mut watch::Receiver<bool>,
    reporter: &mut Reporter<'_>,
) -> Result<(), DownloadError> {
    let total = model.size;
    let mut file = tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(part)
        .await?;
    let mut digest = Context::new(&SHA256);
    let streamed = stream(client, url, total, &mut file, &mut digest, cancel, reporter).await;
    // tokio buffers writes; flush so a retry sees the real length.
    let flushed = file.flush().await;
    streamed?;
    flushed?;

    reporter.report("verifying", total, total, true);
    if hex(digest.finish().as_ref()) != model.sha256 {
        drop(file);
        let _ = tokio::fs::remove_file(part).await;
        return Err(DownloadError::new(
            "integrity",
            "The downloaded model failed its integrity check. Try again.",
        ));
    }
    file.sync_all().await?;
    drop(file);
    tokio::fs::rename(part, target).await?;
    let meta = tokio::fs::symlink_metadata(target).await?;
    let stamp = stamp(&meta).ok_or_else(|| DownloadError::new("storage", "No file mtime"))?;
    let marker = target.with_file_name(format!("{}.verified", model.filename));
    tokio::fs::write(marker, stamp).await?;
    Ok(())
}

/// Leaves `file` holding exactly `total` bytes, all fed into `digest`.
async fn stream(
    client: &reqwest::Client,
    url: &str,
    total: u64,
    file: &mut tokio::fs::File,
    digest: &mut Context,
    cancel: &mut watch::Receiver<bool>,
    reporter: &mut Reporter<'_>,
) -> Result<(), DownloadError> {
    let mut len = file.metadata().await?.len();
    if len > total {
        file.set_len(0).await?;
        len = 0;
    }
    let mut buf = vec![0u8; 1 << 20];
    let mut left = len;
    while left > 0 {
        if *cancel.borrow() {
            return Err(cancelled());
        }
        let want = left.min(buf.len() as u64) as usize;
        let n = file.read(&mut buf[..want]).await?;
        if n == 0 {
            return Err(DownloadError::new("storage", "Partial download shrank"));
        }
        digest.update(&buf[..n]);
        left -= n as u64;
    }
    drop(buf);
    reporter.report("downloading", len, total, true);
    if len == total {
        return Ok(());
    }

    let mut request = client.get(url);
    if len > 0 {
        request = request.header(RANGE, format!("bytes={len}-"));
    }
    let mut response = tokio::select! {
        r = request.send() => r.map_err(network)?,
        _ = cancel_requested(cancel) => return Err(cancelled()),
    };
    let status = response.status().as_u16();
    if len > 0 && status == 206 {
        let want = format!("bytes {len}-{}/{total}", total - 1);
        let got = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok());
        if got != Some(want.as_str()) {
            return Err(DownloadError::new("network", "Unexpected Content-Range"));
        }
    } else if status == 200 {
        if response.content_length() != Some(total) {
            return Err(DownloadError::new("network", "Unexpected content length"));
        }
        if len > 0 {
            // The server ignored Range: start over.
            file.set_len(0).await?;
            file.seek(SeekFrom::Start(0)).await?;
            *digest = Context::new(&SHA256);
            len = 0;
        }
    } else {
        return Err(DownloadError {
            retry: status >= 500,
            ..DownloadError::new("network", format!("Unexpected HTTP status {status}"))
        });
    }

    let mut received = len;
    loop {
        let chunk = tokio::select! {
            c = response.chunk() => c.map_err(network)?,
            _ = cancel_requested(cancel) => return Err(cancelled()),
        };
        let Some(chunk) = chunk else { break };
        if received + chunk.len() as u64 > total {
            return Err(DownloadError::new(
                "network",
                "Server sent more than the catalog size",
            ));
        }
        file.write_all(&chunk).await?;
        digest.update(&chunk);
        received += chunk.len() as u64;
        reporter.report("downloading", received, total, false);
    }
    if received != total {
        return Err(DownloadError {
            retry: true,
            ..DownloadError::new("network", "Connection closed early")
        });
    }
    Ok(())
}

pub fn delete(dir: &Path, model: &Model) -> Result<(), String> {
    for p in [
        path(dir, model),
        marker_path(dir, model),
        part_path(dir, model),
    ] {
        match std::fs::remove_file(&p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
            _ => {}
        }
    }
    Ok(())
}

/// Removes stale or unknown model artifacts (`*.gguf`, `*.part`, `*.verified`)
/// and leaves every other entry alone. Links are never followed.
pub fn prune_stale_parts(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let known: Vec<String> = catalog::models()
        .iter()
        .flat_map(|m| {
            [
                m.filename.clone(),
                format!("{}.part", m.filename),
                format!("{}.verified", m.filename),
            ]
        })
        .collect();
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !entry.file_type().is_ok_and(|t| t.is_file())
            || !(name.ends_with(".gguf") || name.ends_with(".part") || name.ends_with(".verified"))
        {
            continue;
        }
        let stale = name.ends_with(".part")
            && entry
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|t| t.elapsed().is_ok_and(|age| age > STALE_PART));
        if stale || !known.contains(&name) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[derive(Default)]
pub struct Downloads(Mutex<HashMap<String, watch::Sender<bool>>>);

impl Downloads {
    pub fn is_active(&self, id: &str) -> bool {
        self.0.lock().is_ok_and(|m| m.contains_key(id))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelView {
    id: String,
    name: String,
    role: String,
    size: u64,
    languages: Vec<String>,
    license: String,
    repository: String,
    error_pl: Option<f64>,
    error_en: Option<f64>,
    cpu_seconds_per30s: Option<f64>,
    gpu_seconds_per30s: Option<f64>,
    translate: bool,
    streaming: bool,
    state: &'static str,
    downloaded_bytes: u64,
}

fn models_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("stt")
        .join("models"))
}

fn asr(id: &str) -> Result<&'static Model, String> {
    catalog::find(id)
        .filter(|m| m.role == "asr")
        .ok_or_else(|| "unknown-model".to_string())
}

#[tauri::command]
pub async fn stt_list_models(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Downloads>,
) -> Result<Vec<ModelView>, String> {
    main_window(&window)?;
    let dir = models_dir(&app)?;
    prune_stale_parts(&dir);
    Ok(catalog::models()
        .iter()
        .filter(|m| m.role == "asr")
        .map(|m| {
            let part = part_path(&dir, m)
                .symlink_metadata()
                .ok()
                .filter(|p| p.is_file())
                .map_or(0, |p| p.len());
            let (state, downloaded_bytes) = if is_ready(&dir, m) {
                ("ready", m.size)
            } else if state.is_active(&m.id) {
                ("downloading", part)
            } else if part > 0 {
                ("partial", part)
            } else {
                ("missing", 0)
            };
            ModelView {
                id: m.id.clone(),
                name: m.name.clone(),
                role: m.role.clone(),
                size: m.size,
                languages: m.languages.clone(),
                license: m.license.clone(),
                repository: m.repository.clone(),
                error_pl: m.error_pl,
                error_en: m.error_en,
                cpu_seconds_per30s: m.cpu_seconds_per30s,
                gpu_seconds_per30s: m.gpu_seconds_per30s,
                translate: m.translate,
                streaming: m.streaming,
                state,
                downloaded_bytes,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn stt_download_model(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Downloads>,
    id: String,
    progress: Channel<Progress>,
) -> Result<(), String> {
    main_window(&window)?;
    let model = asr(&id)?;
    let vad = catalog::vad();
    let dir = models_dir(&app)?;
    let need_vad = !is_ready(&dir, vad);
    let need_model = !is_ready(&dir, model);
    let (tx, rx) = watch::channel(false);
    {
        // The VAD file is shared, so it is reserved like a model: two
        // downloads must never write the same part file.
        let mut active = state.0.lock().map_err(|_| "busy")?;
        if active.contains_key(&id) || (need_vad && active.contains_key(&vad.id)) {
            return Err("busy".into());
        }
        active.insert(id.clone(), tx);
        if need_vad {
            active.insert(vad.id.clone(), watch::channel(false).0);
        }
    }
    let vad_size = if need_vad { vad.size } else { 0 };
    let total = vad_size + if need_model { model.size } else { 0 };
    let send = |offset: u64| {
        let progress = &progress;
        move |p: Progress| {
            let _ = progress.send(Progress {
                bytes: p.bytes + offset,
                total,
                ..p
            });
        }
    };
    let result = async {
        if need_vad {
            download(&dir, vad, rx.clone(), send(0)).await?;
        }
        if need_model {
            download(&dir, model, rx.clone(), send(vad_size)).await?;
        }
        Ok(())
    }
    .await;
    if let Ok(mut active) = state.0.lock() {
        active.remove(&id);
        // Only release the VAD reservation this download made.
        if need_vad {
            active.remove(&vad.id);
        }
    }
    result.map_err(|e: DownloadError| format!("{}: {e}", e.code()))
}

#[tauri::command]
pub fn stt_cancel_download(
    window: Window,
    state: State<'_, Downloads>,
    id: String,
) -> Result<(), String> {
    main_window(&window)?;
    if let Some(tx) = state.0.lock().map_err(|_| "busy")?.get(&id) {
        let _ = tx.send(true);
    }
    Ok(())
}

#[tauri::command]
pub fn stt_delete_model(
    window: Window,
    app: tauri::AppHandle,
    downloads: State<'_, Downloads>,
    stt: State<'_, super::Stt>,
    id: String,
) -> Result<(), String> {
    main_window(&window)?;
    let model = asr(&id)?;
    if downloads.is_active(&id) {
        return Err("busy".into());
    }
    // The engine maps the file; Windows cannot delete it while it is open.
    stt.unload()?;
    delete(&models_dir(&app)?, model)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real Hugging Face download through the production client, redirect
    /// allowlist and checksum. Needs network: `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn downloads_the_vad_from_hugging_face() {
        let dir = tempfile::tempdir().unwrap();
        let vad = catalog::vad();
        let (_tx, rx) = watch::channel(false);
        download(dir.path(), vad, rx, |_| {}).await.unwrap();
        assert!(is_ready(dir.path(), vad));
    }
    use std::sync::Arc;
    use std::time::SystemTime;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    #[derive(Clone, Copy)]
    enum Mode {
        Normal,
        IgnoreRange,
        BadRange,
        Stall,
    }

    struct Server {
        base: String,
        requests: Arc<Mutex<Vec<Option<String>>>>,
    }

    fn data() -> Vec<u8> {
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        (0..300_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 32) as u8
            })
            .collect()
    }

    fn model_for(data: &[u8]) -> Model {
        let mut m = catalog::vad().clone();
        m.id = "test".into();
        m.filename = "test.gguf".into();
        m.size = data.len() as u64;
        m.sha256 = hex(ring::digest::digest(&SHA256, data).as_ref());
        m
    }

    async fn serve(data: Vec<u8>, mode: Mode) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let (data, log) = (data.clone(), log.clone());
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        if sock.read_exact(&mut byte).await.is_err() {
                            return;
                        }
                        head.push(byte[0]);
                    }
                    let head = String::from_utf8_lossy(&head).to_lowercase();
                    let range = head
                        .lines()
                        .find_map(|l| l.strip_prefix("range: "))
                        .map(str::to_owned);
                    log.lock().unwrap().push(range.clone());
                    let n = data.len();
                    let start = range
                        .as_deref()
                        .and_then(|r| r.strip_prefix("bytes="))
                        .and_then(|r| r.strip_suffix('-'))
                        .and_then(|r| r.parse::<usize>().ok())
                        .filter(|_| !matches!(mode, Mode::IgnoreRange));
                    let (head, body) = match start {
                        Some(a) => {
                            let cr = if matches!(mode, Mode::BadRange) {
                                format!("bytes 0-{}/{n}", n - 1)
                            } else {
                                format!("bytes {a}-{}/{n}", n - 1)
                            };
                            (
                                format!(
                                    "HTTP/1.1 206 Partial Content\r\nContent-Range: {cr}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                    n - a
                                ),
                                &data[a..],
                            )
                        }
                        None => (
                            format!("HTTP/1.1 200 OK\r\nContent-Length: {n}\r\nConnection: close\r\n\r\n"),
                            &data[..],
                        ),
                    };
                    let _ = sock.write_all(head.as_bytes()).await;
                    if matches!(mode, Mode::Stall) {
                        let _ = sock.write_all(&body[..1000]).await;
                        let _ = sock.flush().await;
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        return;
                    }
                    let _ = sock.write_all(body).await;
                });
            }
        });
        Server { base, requests }
    }

    async fn run(s: &Server, dir: &Path, m: &Model) -> Result<(), DownloadError> {
        let (_tx, rx) = watch::channel(false);
        download_from(&s.base, true, dir, m, rx, |_| {}).await
    }

    fn write_part(dir: &Path, m: &Model, bytes: &[u8]) {
        std::fs::write(part_path(dir, m), bytes).unwrap();
    }

    #[tokio::test]
    async fn fresh_download_is_ready() {
        let (d, dir) = (data(), tempfile::tempdir().unwrap());
        let (m, s) = (model_for(&d), serve(d.clone(), Mode::Normal).await);
        run(&s, dir.path(), &m).await.unwrap();
        assert!(is_ready(dir.path(), &m));
        assert_eq!(std::fs::read(path(dir.path(), &m)).unwrap(), d);
        assert!(marker_path(dir.path(), &m).is_file());
        assert!(!part_path(dir.path(), &m).exists());
    }

    #[tokio::test]
    async fn resumes_only_the_missing_range() {
        let (d, dir) = (data(), tempfile::tempdir().unwrap());
        let (m, s) = (model_for(&d), serve(d.clone(), Mode::Normal).await);
        write_part(dir.path(), &m, &d[..100_000]);
        run(&s, dir.path(), &m).await.unwrap();
        assert_eq!(std::fs::read(path(dir.path(), &m)).unwrap(), d);
        assert_eq!(
            *s.requests.lock().unwrap(),
            vec![Some("bytes=100000-".to_string())]
        );
    }

    #[tokio::test]
    async fn ignored_range_restarts() {
        let (d, dir) = (data(), tempfile::tempdir().unwrap());
        let (m, s) = (model_for(&d), serve(d.clone(), Mode::IgnoreRange).await);
        write_part(dir.path(), &m, &d[..100_000]);
        run(&s, dir.path(), &m).await.unwrap();
        assert_eq!(std::fs::read(path(dir.path(), &m)).unwrap(), d);
        assert!(is_ready(dir.path(), &m));
    }

    #[tokio::test]
    async fn wrong_content_range_keeps_part() {
        let (d, dir) = (data(), tempfile::tempdir().unwrap());
        let (m, s) = (model_for(&d), serve(d.clone(), Mode::BadRange).await);
        write_part(dir.path(), &m, &d[..100_000]);
        let e = run(&s, dir.path(), &m).await.unwrap_err();
        assert_eq!(e.code(), "network");
        assert_eq!(
            std::fs::metadata(part_path(dir.path(), &m)).unwrap().len(),
            100_000
        );
        assert!(!path(dir.path(), &m).exists());
    }

    #[tokio::test]
    async fn wrong_sha_removes_part() {
        let (d, dir) = (data(), tempfile::tempdir().unwrap());
        let (mut m, s) = (model_for(&d), serve(d.clone(), Mode::Normal).await);
        m.sha256 = "0".repeat(64);
        let e = run(&s, dir.path(), &m).await.unwrap_err();
        assert_eq!(e.code(), "integrity");
        assert!(!part_path(dir.path(), &m).exists());
        assert!(!path(dir.path(), &m).exists());
    }

    #[tokio::test]
    async fn cancel_removes_part() {
        let (d, dir) = (data(), tempfile::tempdir().unwrap());
        let (m, s) = (model_for(&d), serve(d.clone(), Mode::Stall).await);
        let (tx, rx) = watch::channel(false);
        let (result, ()) = tokio::join!(
            download_from(&s.base, true, dir.path(), &m, rx, |_| {}),
            async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                tx.send(true).unwrap();
            }
        );
        assert_eq!(result.unwrap_err().code(), "cancelled");
        assert!(!part_path(dir.path(), &m).exists());
    }

    #[test]
    fn redirect_hosts() {
        assert!(host_allowed("huggingface.co"));
        assert!(host_allowed("us.aws.cdn.hf.co"));
        assert!(!host_allowed("evilhf.co"));
        assert!(!host_allowed("hf.co.evil.com"));
        assert!(!host_allowed("huggingface.co.evil.com"));
    }

    #[tokio::test]
    async fn modified_file_is_not_ready() {
        let (d, dir) = (data(), tempfile::tempdir().unwrap());
        let (m, s) = (model_for(&d), serve(d.clone(), Mode::Normal).await);
        run(&s, dir.path(), &m).await.unwrap();
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(path(dir.path(), &m))
            .unwrap();
        f.set_modified(SystemTime::now() + Duration::from_secs(10))
            .unwrap();
        assert!(!is_ready(dir.path(), &m));
        f.set_len(10).unwrap();
        assert!(!is_ready(dir.path(), &m));
    }

    #[test]
    fn prune_removes_old_and_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let known = &catalog::models()[0];
        let old = part_path(dir.path(), known);
        let fresh = path(dir.path(), known);
        let unknown = dir.path().join("gone-model.gguf");
        let other = dir.path().join("notes.txt");
        for p in [&old, &fresh, &unknown, &other] {
            std::fs::write(p, b"x").unwrap();
        }
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(8 * 24 * 3600))
            .unwrap();
        prune_stale_parts(dir.path());
        assert!(!old.exists() && !unknown.exists());
        assert!(fresh.exists() && other.exists());
    }
}
