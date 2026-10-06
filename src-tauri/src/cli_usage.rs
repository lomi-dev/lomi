use crate::{
    cli_titles::{self, TitleCli, TitleProcess, UsageProcessPaths},
    files::main_window,
    terminal::Terminals,
};
use chrono::{DateTime, FixedOffset};
use futures_util::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::OpenOptions,
    future::Future,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock, Weak},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{State, Window};
use tokio::sync::{Mutex, Semaphore};

mod agy;

const MAX_TARGETS: usize = 128;
const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const MAX_CREDENTIAL_BYTES: usize = 256 * 1024;
const MAX_CACHE_ENTRIES: usize = 256;
const MAX_CONCURRENT_TARGETS: usize = 8;
const SUCCESS_TTL: Duration = Duration::from_secs(5);
const FORCE_COOLDOWN: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

pub struct CliUsage {
    cache: Mutex<UsageCache>,
    network_slots: Arc<Semaphore>,
    client: OnceLock<reqwest::Client>,
}

impl Default for CliUsage {
    fn default() -> Self {
        Self {
            cache: Mutex::new(UsageCache::default()),
            network_slots: Arc::new(Semaphore::new(4)),
            client: OnceLock::new(),
        }
    }
}

impl CliUsage {
    fn client(&self) -> &reqwest::Client {
        self.client.get_or_init(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("fixed CLI usage HTTP client settings are valid")
        })
    }
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
enum ProfileQuotaError {
    UnknownScope,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum UsageStatus {
    Ready,
    Unauthenticated,
    Unsupported,
    Error,
    RateLimited,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct UsageWindow {
    label: String,
    remaining_percent: Option<f64>,
    used: Option<f64>,
    limit: Option<f64>,
    unit: Option<String>,
    resets_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageEntry {
    id: String,
    process: TitleProcess,
    account_key: Option<String>,
    status: UsageStatus,
    windows: Vec<UsageWindow>,
    updated_at: Option<i64>,
    retry_at: Option<i64>,
    message: Option<String>,
    source: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTarget {
    id: String,
    process: TitleProcess,
}

#[derive(Serialize)]
pub struct UsageResponse {
    entries: Vec<UsageEntry>,
}

#[tauri::command]
pub async fn inspect_cli_usage(
    window: Window,
    terminals: State<'_, Terminals>,
    state: State<'_, CliUsage>,
    targets: Vec<UsageTarget>,
    force: Option<bool>,
) -> Result<UsageResponse, String> {
    main_window(&window)?;
    if targets.len() > MAX_TARGETS {
        return Err(format!(
            "Check at most {MAX_TARGETS} CLI terminals at a time."
        ));
    }
    let force = force.unwrap_or(false);
    let entries = map_bounded_ordered(targets, |target| {
        inspect_target(terminals.inner(), state.inner(), target, force)
    })
    .await;
    Ok(UsageResponse { entries })
}

async fn map_bounded_ordered<I, F, Fut, R>(items: I, map: F) -> Vec<R>
where
    I: IntoIterator,
    F: FnMut(I::Item) -> Fut,
    Fut: Future<Output = R>,
{
    stream::iter(items.into_iter().map(map))
        .buffered(MAX_CONCURRENT_TARGETS)
        .collect()
        .await
}

async fn inspect_target(
    terminals: &Terminals,
    state: &CliUsage,
    target: UsageTarget,
    force: bool,
) -> UsageEntry {
    if terminals
        .check_title_process(&target.id, target.process)
        .is_err()
    {
        return empty_entry(
            target,
            UsageStatus::Error,
            "The CLI is no longer running in this terminal. Check usage again.",
            None,
        );
    }

    if !supports_usage(target.process.cli) {
        let message = unsupported_message(target.process.cli);
        let source = Some(source_name(target.process.cli));
        return empty_entry(target, UsageStatus::Unsupported, message, source);
    }

    let process = target.process;
    let paths = match tauri::async_runtime::spawn_blocking(move || {
        cli_titles::usage_process_paths(process)
    })
    .await
    {
        Ok(paths) => paths,
        Err(_) => {
            return empty_entry(
                target,
                UsageStatus::Error,
                "Cannot safely read this CLI's native account credentials.",
                Some(source_name(process.cli)),
            );
        }
    };

    if terminals.check_title_process(&target.id, process).is_err() {
        return empty_entry(
            target,
            UsageStatus::Error,
            "The CLI is no longer running in this terminal. Check usage again.",
            None,
        );
    }

    let paths = match paths {
        Ok(paths) => paths,
        Err(_) => {
            return empty_entry(
                target,
                UsageStatus::Error,
                "Cannot safely read this CLI's native account environment.",
                Some(source_name(process.cli)),
            );
        }
    };
    if process.cli == TitleCli::Agy {
        return inspect_agy_target(terminals, state, target, paths, force).await;
    }
    let namespace_paths = paths.clone();
    let namespace_result = tauri::async_runtime::spawn_blocking(move || {
        native_namespace(process.cli, &namespace_paths)
    })
    .await;
    let namespace = match namespace_result {
        Ok(Ok(namespace)) => namespace,
        Ok(Err(message)) => {
            return empty_entry(
                target,
                UsageStatus::Error,
                message,
                Some(source_name(process.cli)),
            );
        }
        Err(_) => {
            return empty_entry(
                target,
                UsageStatus::Error,
                "Cannot safely read this CLI's native account settings.",
                Some(source_name(process.cli)),
            );
        }
    };
    let observed_generation = current_generation(state, &namespace).await;
    let credential_result = match tauri::async_runtime::spawn_blocking(move || {
        resolve_native_credentials(process.cli, paths, namespace.clone())
    })
    .await
    {
        Ok(result) => result,
        Err(_) => {
            return empty_entry(
                target,
                UsageStatus::Error,
                "Cannot safely read this CLI's native account credentials.",
                Some(source_name(process.cli)),
            );
        }
    };

    if terminals.check_title_process(&target.id, process).is_err() {
        return empty_entry(
            target,
            UsageStatus::Error,
            "The CLI is no longer running in this terminal. Check usage again.",
            None,
        );
    }

    let credential = match credential_result {
        NativeCredentialResult::Ready(credential) => credential,
        NativeCredentialResult::Status {
            namespace,
            status,
            message,
        } => {
            invalidate_namespace_if_generation(state, &namespace, observed_generation).await;
            return empty_entry(target, status, message, Some(source_name(process.cli)));
        }
    };

    let account_key = credential_account_key(&credential);
    let key = CacheKey {
        namespace: credential.namespace.clone(),
        identity: credential.identity.clone(),
    };
    let Some(generation) = activate_identity(state, &key, observed_generation).await else {
        return empty_entry(
            target,
            UsageStatus::Error,
            "CLI account credentials changed while usage was being checked. Try again.",
            Some(source_name(process.cli)),
        );
    };
    let request_lock = request_lock(state, &key).await;
    let _guard = request_lock.lock().await;

    if !terminal_process_is_current(terminals, &target.id, process)
        || verify_credentials_current(terminals, &target.id, state, process, &key, generation).await
            != Some(generation)
    {
        return empty_entry(
            target,
            UsageStatus::Error,
            "CLI account credentials changed while usage was being checked. Try again.",
            Some(source_name(process.cli)),
        );
    }

    if let Some(cached) = cached_snapshot(state, &key, force).await {
        if terminal_process_is_current(terminals, &target.id, process)
            && verify_credentials_current(terminals, &target.id, state, process, &key, generation)
                .await
                == Some(generation)
        {
            return entry_from_snapshot(target, cached, account_key.as_deref());
        }
        return empty_entry(
            target,
            UsageStatus::Error,
            "CLI account credentials changed while usage was being checked. Try again.",
            Some(source_name(process.cli)),
        );
    }

    let _slot = match state.network_slots.acquire().await {
        Ok(slot) => slot,
        Err(_) => {
            return match failed_snapshot(
                state,
                &key,
                "Usage checks are temporarily unavailable. Try again shortly.",
                UsageStatus::Error,
                None,
                generation,
            )
            .await
            {
                Some(snapshot) => entry_from_snapshot(target, snapshot, account_key.as_deref()),
                None => empty_entry(
                    target,
                    UsageStatus::Error,
                    "CLI account credentials changed while usage was being checked. Try again.",
                    Some(source_name(process.cli)),
                ),
            };
        }
    };
    if !terminal_process_is_current(terminals, &target.id, process)
        || verify_credentials_current(terminals, &target.id, state, process, &key, generation).await
            != Some(generation)
    {
        return empty_entry(
            target,
            UsageStatus::Error,
            "CLI account credentials changed while usage was being checked. Try again.",
            Some(source_name(process.cli)),
        );
    }
    if !mark_request_started(state, &key, force, generation).await {
        return empty_entry(
            target,
            UsageStatus::Error,
            "CLI account credentials changed while usage was being checked. Try again.",
            Some(source_name(process.cli)),
        );
    }
    let result = fetch_usage(state.client(), &credential).await;

    if !terminal_process_is_current(terminals, &target.id, process)
        || verify_credentials_current(terminals, &target.id, state, process, &key, generation).await
            != Some(generation)
    {
        return empty_entry(
            target,
            UsageStatus::Error,
            "The CLI is no longer running in this terminal. Check usage again.",
            None,
        );
    }

    match result {
        Ok(windows) => {
            let snapshot = UsageSnapshot {
                status: UsageStatus::Ready,
                windows,
                updated_at: Some(now_ms()),
                retry_at: None,
                message: None,
                source: Some(source_name(process.cli)),
                next_retry: None,
                last_success: Some(Instant::now()),
                last_forced: force.then(Instant::now),
                failure_count: 0,
                last_used: Instant::now(),
            };
            if !store_snapshot(state, key, generation, snapshot.clone()).await {
                return empty_entry(
                    target,
                    UsageStatus::Error,
                    "CLI account credentials changed while usage was being checked. Try again.",
                    Some(source_name(process.cli)),
                );
            }
            entry_from_snapshot(target, snapshot, account_key.as_deref())
        }
        Err(failure) => {
            if failure.status == UsageStatus::Unauthenticated
                || failure.status == UsageStatus::Unsupported
            {
                invalidate_namespace_if_generation(state, &credential.namespace, generation).await;
                let mut entry = empty_entry(
                    target,
                    failure.status,
                    failure.message,
                    Some(source_name(process.cli)),
                );
                entry.account_key = account_key;
                return entry;
            }
            let snapshot = failed_snapshot(
                state,
                &key,
                failure.message,
                failure.status,
                failure.retry_after,
                generation,
            )
            .await;
            match snapshot {
                Some(snapshot) => entry_from_snapshot(target, snapshot, account_key.as_deref()),
                None => empty_entry(
                    target,
                    UsageStatus::Error,
                    "CLI account credentials changed while usage was being checked. Try again.",
                    Some(source_name(process.cli)),
                ),
            }
        }
    }
}

async fn inspect_agy_target(
    terminals: &Terminals,
    state: &CliUsage,
    target: UsageTarget,
    paths: UsageProcessPaths,
    force: bool,
) -> UsageEntry {
    let process = target.process;
    let context = match tauri::async_runtime::spawn_blocking(move || agy::prepare(&paths)).await {
        Ok(Ok(context)) => context,
        _ => {
            return empty_entry(
                target,
                UsageStatus::Error,
                "Cannot safely read Antigravity CLI account settings.",
                Some(source_name(TitleCli::Agy)),
            );
        }
    };
    if !terminal_process_is_current(terminals, &target.id, process) {
        return empty_entry(
            target,
            UsageStatus::Error,
            "The CLI is no longer running in this terminal. Check usage again.",
            None,
        );
    }
    let key = agy_cache_key(&context);
    let observed_generation = current_generation(state, &key.namespace).await;
    let Some(generation) = activate_identity(state, &key, observed_generation).await else {
        return empty_entry(
            target,
            UsageStatus::Error,
            "Antigravity account context changed while usage was being checked. Try again.",
            Some(source_name(TitleCli::Agy)),
        );
    };
    if let Some(message) = context.unsupported_message {
        return empty_entry(
            target,
            UsageStatus::Unsupported,
            message,
            Some(source_name(TitleCli::Agy)),
        );
    }

    // Authentication is CLI-managed, so only identical native report contexts share a row.
    let account_key = opaque_account_key(
        TitleCli::Agy,
        &format!("report:{}", context.identity),
        None,
        None,
    );
    let request_lock = request_lock(state, &key).await;
    let _guard = request_lock.lock().await;
    if !terminal_process_is_current(terminals, &target.id, process)
        || verify_agy_context_current(terminals, &target.id, state, process, &key, generation).await
            != Some(generation)
    {
        return empty_entry(
            target,
            UsageStatus::Error,
            "Antigravity account context changed while usage was being checked. Try again.",
            Some(source_name(TitleCli::Agy)),
        );
    }
    if let Some(cached) = cached_snapshot(state, &key, force).await {
        if terminal_process_is_current(terminals, &target.id, process)
            && verify_agy_context_current(terminals, &target.id, state, process, &key, generation)
                .await
                == Some(generation)
        {
            return entry_from_snapshot(target, cached, account_key.as_deref());
        }
        return empty_entry(
            target,
            UsageStatus::Error,
            "Antigravity account context changed while usage was being checked. Try again.",
            Some(source_name(TitleCli::Agy)),
        );
    }

    let _slot = match state.network_slots.acquire().await {
        Ok(slot) => slot,
        Err(_) => {
            return match failed_snapshot_clearing_values(
                state,
                &key,
                FetchFailure {
                    status: UsageStatus::Error,
                    message:
                        "Antigravity usage checks are temporarily unavailable. Try again shortly.",
                    retry_after: None,
                },
                generation,
            )
            .await
            {
                Some(snapshot) => entry_from_snapshot(target, snapshot, account_key.as_deref()),
                None => empty_entry(
                    target,
                    UsageStatus::Error,
                    "Antigravity account context changed while usage was being checked. Try again.",
                    Some(source_name(TitleCli::Agy)),
                ),
            };
        }
    };
    if !terminal_process_is_current(terminals, &target.id, process)
        || verify_agy_context_current(terminals, &target.id, state, process, &key, generation).await
            != Some(generation)
    {
        return empty_entry(
            target,
            UsageStatus::Error,
            "Antigravity account context changed while usage was being checked. Try again.",
            Some(source_name(TitleCli::Agy)),
        );
    }
    if !mark_request_started(state, &key, force, generation).await {
        return empty_entry(
            target,
            UsageStatus::Error,
            "Antigravity account context changed while usage was being checked. Try again.",
            Some(source_name(TitleCli::Agy)),
        );
    }
    let version_context = context.clone();
    let version_probe =
        tauri::async_runtime::spawn_blocking(move || agy::check_version(&version_context)).await;
    let version_result = match version_probe {
        Ok(result) => result,
        Err(_) => Err(FetchFailure {
            status: UsageStatus::Error,
            message: "Antigravity usage checks are temporarily unavailable. Try again shortly.",
            retry_after: None,
        }),
    };
    if !terminal_process_is_current(terminals, &target.id, process)
        || verify_agy_context_current(terminals, &target.id, state, process, &key, generation).await
            != Some(generation)
    {
        return empty_entry(
            target,
            UsageStatus::Error,
            "Antigravity account context changed while usage was being checked. Try again.",
            Some(source_name(TitleCli::Agy)),
        );
    }
    let result = match version_result {
        Ok(()) => {
            match tauri::async_runtime::spawn_blocking(move || agy::read_usage(&context)).await {
                Ok(result) => result,
                Err(_) => Err(FetchFailure {
                    status: UsageStatus::Error,
                    message:
                        "Antigravity usage checks are temporarily unavailable. Try again shortly.",
                    retry_after: None,
                }),
            }
        }
        Err(failure) => Err(failure),
    };
    if !terminal_process_is_current(terminals, &target.id, process)
        || verify_agy_context_current(terminals, &target.id, state, process, &key, generation).await
            != Some(generation)
    {
        return empty_entry(
            target,
            UsageStatus::Error,
            "The CLI is no longer running in this terminal. Check usage again.",
            None,
        );
    }
    match result {
        Ok(windows) => {
            let snapshot = UsageSnapshot {
                status: UsageStatus::Ready,
                windows,
                updated_at: Some(now_ms()),
                retry_at: None,
                message: None,
                source: Some(source_name(TitleCli::Agy)),
                next_retry: None,
                last_success: Some(Instant::now()),
                last_forced: force.then(Instant::now),
                failure_count: 0,
                last_used: Instant::now(),
            };
            if !store_snapshot(state, key, generation, snapshot.clone()).await {
                return empty_entry(
                    target,
                    UsageStatus::Error,
                    "Antigravity account context changed while usage was being checked. Try again.",
                    Some(source_name(TitleCli::Agy)),
                );
            }
            entry_from_snapshot(target, snapshot, account_key.as_deref())
        }
        Err(failure) => {
            match failed_snapshot_clearing_values(state, &key, failure, generation).await {
                Some(snapshot) => entry_from_snapshot(target, snapshot, account_key.as_deref()),
                None => empty_entry(
                    target,
                    UsageStatus::Error,
                    "Antigravity account context changed while usage was being checked. Try again.",
                    Some(source_name(TitleCli::Agy)),
                ),
            }
        }
    }
}

fn agy_cache_key(context: &agy::Context) -> CacheKey {
    CacheKey {
        namespace: UsageNamespace {
            cli: TitleCli::Agy,
            directory: context.directory.clone(),
            store: format!("agy-cli-managed-{}", context.identity),
        },
        identity: "agy-cli-managed".to_owned(),
    }
}

async fn verify_agy_context_current(
    terminals: &Terminals,
    id: &str,
    state: &CliUsage,
    process: TitleProcess,
    expected_key: &CacheKey,
    expected_generation: u64,
) -> Option<u64> {
    let paths =
        tauri::async_runtime::spawn_blocking(move || cli_titles::usage_process_paths(process))
            .await
            .ok()?
            .ok()?;
    if !terminal_process_is_current(terminals, id, process) {
        return None;
    }
    let context = tauri::async_runtime::spawn_blocking(move || agy::prepare(&paths))
        .await
        .ok()?
        .ok()?;
    if !terminal_process_is_current(terminals, id, process) {
        return None;
    }
    let current_key = agy_cache_key(&context);
    let observed_generation = current_generation(state, &current_key.namespace).await;
    if current_key != *expected_key {
        let _ = activate_identity(state, &current_key, observed_generation).await;
        return None;
    }
    let generation = activate_identity(state, &current_key, observed_generation).await?;
    (generation == expected_generation).then_some(generation)
}

fn supports_usage(cli: TitleCli) -> bool {
    matches!(
        cli,
        TitleCli::Codex | TitleCli::Claude | TitleCli::Cursor | TitleCli::Kimi | TitleCli::Agy
    )
}

fn source_name(cli: TitleCli) -> String {
    match cli {
        TitleCli::Codex => "codex",
        TitleCli::Claude => "claude",
        TitleCli::Cursor => "cursor",
        TitleCli::Kimi => "kimi",
        TitleCli::Agy => "agy",
        _ => "cli",
    }
    .to_owned()
}

fn unsupported_message(cli: TitleCli) -> &'static str {
    match cli {
        TitleCli::Gemini => "Gemini CLI does not expose a quota check that Lomi can safely call.",
        TitleCli::Agy => "Antigravity CLI usage is not available for this account or version.",
        TitleCli::Cursor => "Cursor account usage is not available for this account or plan.",
        TitleCli::Claude => {
            "Claude Code account usage is not available for this authentication mode."
        }
        TitleCli::Kimi => {
            "Kimi Code account usage is not available for this provider or login mode."
        }
        _ => "Lomi does not have a verified account usage reader for this CLI yet.",
    }
}

fn empty_entry(
    target: UsageTarget,
    status: UsageStatus,
    message: impl Into<String>,
    source: Option<String>,
) -> UsageEntry {
    UsageEntry {
        id: target.id,
        process: target.process,
        account_key: None,
        status,
        windows: Vec::new(),
        updated_at: None,
        retry_at: None,
        message: Some(message.into()),
        source,
    }
}

fn entry_from_snapshot(
    target: UsageTarget,
    snapshot: UsageSnapshot,
    account_key: Option<&str>,
) -> UsageEntry {
    UsageEntry {
        id: target.id,
        process: target.process,
        account_key: account_key.map(str::to_owned),
        status: snapshot.status,
        windows: snapshot.windows,
        updated_at: snapshot.updated_at,
        retry_at: snapshot.retry_at,
        message: snapshot.message,
        source: snapshot.source,
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct UsageNamespace {
    cli: TitleCli,
    directory: PathBuf,
    store: String,
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct CacheKey {
    namespace: UsageNamespace,
    identity: String,
}

#[derive(Clone)]
struct UsageSnapshot {
    status: UsageStatus,
    windows: Vec<UsageWindow>,
    updated_at: Option<i64>,
    retry_at: Option<i64>,
    message: Option<String>,
    source: Option<String>,
    next_retry: Option<Instant>,
    last_success: Option<Instant>,
    last_forced: Option<Instant>,
    failure_count: u32,
    last_used: Instant,
}

#[derive(Default)]
struct UsageCache {
    records: HashMap<CacheKey, UsageSnapshot>,
    active_identity: HashMap<UsageNamespace, String>,
    generation: HashMap<UsageNamespace, u64>,
    locks: HashMap<CacheKey, Weak<Mutex<()>>>,
}

struct NativeCredential {
    cli: TitleCli,
    namespace: UsageNamespace,
    identity: String,
    token: String,
    account_id: Option<String>,
    team_id: Option<String>,
    usage_endpoint: Option<&'static str>,
}

fn credential_account_key(credential: &NativeCredential) -> Option<String> {
    let identity = (credential.cli == TitleCli::Codex)
        .then(|| codex_account_identity(credential))
        .flatten()
        .unwrap_or_else(|| format!("credential:{}", credential.identity));
    opaque_account_key(
        credential.cli,
        &identity,
        credential.team_id.as_deref(),
        credential.usage_endpoint,
    )
}

fn codex_account_identity(credential: &NativeCredential) -> Option<String> {
    use base64::{engine::general_purpose, Engine};

    let account_id = credential.account_id.as_deref()?.trim();
    if account_id.is_empty() || credential.token.len() > MAX_CREDENTIAL_BYTES {
        return None;
    }
    let mut segments = credential.token.split('.');
    let header = segments.next()?;
    let encoded = segments.next()?;
    let signature = segments.next()?;
    if header.is_empty() || encoded.is_empty() || signature.is_empty() || segments.next().is_some()
    {
        return None;
    }
    let payload = general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .or_else(|_| general_purpose::URL_SAFE.decode(encoded))
        .ok()?;
    let claims: Value = serde_json::from_slice(&payload).ok()?;
    let auth = claims.get("https://api.openai.com/auth")?;
    let claim = |name| {
        auth.get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    // Claims group existing credentials only; requests and cache validation remain token-sensitive.
    if claim("chatgpt_account_id")? != account_id {
        return None;
    }
    let principal = claim("chatgpt_user_id")
        .or_else(|| claim("user_id"))
        .map(|id| ("user", id))
        .or_else(|| claim("chatgpt_account_user_id").map(|id| ("membership", id)))?;
    serde_json::to_string(&(account_id, principal)).ok()
}

/// Call only after a successful token-sensitive usage response; this helper
/// qualifies scope using the same account/principal rules as native usage.
#[cfg(test)]
fn codex_profile_quota_group(credential: &NativeCredential) -> Result<String, ProfileQuotaError> {
    let identity = codex_account_identity(credential).ok_or(ProfileQuotaError::UnknownScope)?;
    let mut group = Sha256::new();
    group.update(b"lomi-codex-quota-group-v2\0");
    group.update(identity.as_bytes());
    Ok(hex_digest(&group.finalize()))
}

fn opaque_account_key(
    cli: TitleCli,
    identity: &str,
    team_id: Option<&str>,
    usage_endpoint: Option<&str>,
) -> Option<String> {
    // Keep grouping keys private to this app process without changing credential cache identities.
    static KEY: OnceLock<Option<ring::hmac::Key>> = OnceLock::new();
    let key = KEY
        .get_or_init(|| {
            let mut bytes = [0_u8; 32];
            ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes).ok()?;
            Some(ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &bytes))
        })
        .as_ref()?;
    let provider = format!("{cli:?}");
    let mut fingerprint = ring::hmac::Context::with_key(key);
    for part in [
        "lomi-cli-usage-account-v1",
        provider.as_str(),
        identity,
        team_id.unwrap_or_default(),
        usage_endpoint.unwrap_or_default(),
    ] {
        fingerprint.update(&(part.len() as u64).to_be_bytes());
        fingerprint.update(part.as_bytes());
    }
    Some(hex_digest(fingerprint.sign().as_ref()))
}

type ProviderAuth = (String, Option<String>, Option<String>, Option<&'static str>);

enum NativeCredentialResult {
    Ready(NativeCredential),
    Status {
        namespace: UsageNamespace,
        status: UsageStatus,
        message: &'static str,
    },
}

#[derive(Debug)]
enum AuthReadError {
    Missing,
    Unsupported(&'static str),
    Error(&'static str),
}

fn native_namespace(
    cli: TitleCli,
    paths: &UsageProcessPaths,
) -> Result<UsageNamespace, &'static str> {
    let directory = match paths.directory_for(cli) {
        Ok(directory) => directory,
        Err(_) => return Err("Cannot locate the running CLI's native account directory."),
    };
    let directory = canonical_or_original(&directory);
    let store = match cli {
        TitleCli::Codex
            if paths.has_auth_argument_override
                || paths.has_openai_api_key
                || paths.has_openai_custom_base_url =>
        {
            "codex-override".to_owned()
        }
        TitleCli::Codex => "codex-native".to_owned(),
        TitleCli::Claude if paths.has_auth_argument_override || paths.has_claude_auth_override => {
            "claude-override".to_owned()
        }
        TitleCli::Claude => {
            let secure_storage_path = if paths.claude_secure_storage_config_dir_set {
                paths.claude_secure_storage_config_dir.as_ref()
            } else {
                paths.claude_config_dir.as_ref()
            };
            match secure_storage_path {
                Some(path) => format!("claude-keychain-{}", path_fingerprint(path)),
                None => "claude-keychain-default".to_owned(),
            }
        }
        TitleCli::Cursor
            if paths.has_auth_argument_override
                || paths.has_cursor_api_key
                || paths.has_cursor_custom_backend =>
        {
            "cursor-override".to_owned()
        }
        TitleCli::Cursor if paths.has_cursor_custom_config_dir => "cursor-custom-config".to_owned(),
        TitleCli::Cursor => match paths.cursor_credential_store.as_deref() {
            Some("file") => format!("cursor-file-{}", cursor_config_scope(paths)),
            Some("memory") => format!("cursor-memory-{}", cursor_config_scope(paths)),
            Some(_) => format!("cursor-unknown-store-{}", cursor_config_scope(paths)),
            None if cfg!(target_os = "macos") => {
                format!("cursor-keychain-{}", cursor_config_scope(paths))
            }
            None => format!("cursor-file-{}", cursor_config_scope(paths)),
        },
        TitleCli::Kimi => format!("kimi-{}", kimi_config_scope(paths)?),
        _ => "unsupported".to_owned(),
    };
    Ok(UsageNamespace {
        cli,
        directory,
        store,
    })
}

fn path_fingerprint(path: &Path) -> String {
    let mut hasher = Sha256::new();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        hasher.update(path.as_os_str().as_bytes());
    }
    #[cfg(not(unix))]
    hasher.update(path.to_string_lossy().as_bytes());
    hex_digest(&hasher.finalize()[..8])
}

fn cursor_config_scope(paths: &UsageProcessPaths) -> String {
    paths
        .cursor_config_dir
        .as_deref()
        .map(path_fingerprint)
        .unwrap_or_else(|| "default".to_owned())
}

fn kimi_config_scope(paths: &UsageProcessPaths) -> Result<String, &'static str> {
    let directory = paths
        .directory_for(TitleCli::Kimi)
        .map_err(|_| "Cannot locate the running CLI's native account directory.")?;
    let config = read_optional_bytes(&directory.join("config.toml"), MAX_CREDENTIAL_BYTES)
        .map_err(|_| "Kimi Code settings could not be read safely.")?;
    let mut hasher = Sha256::new();
    hasher.update(b"lomi-kimi-usage-v1\0");
    if let Some(config) = config {
        hasher.update((config.len() as u64).to_be_bytes());
        hasher.update(config);
    } else {
        hasher.update(u64::MAX.to_be_bytes());
    }
    for value in [
        paths.kimi_code_base_url.as_deref(),
        paths.kimi_code_oauth_host.as_deref(),
        paths.kimi_oauth_host.as_deref(),
    ] {
        if let Some(value) = value {
            hasher.update((value.len() as u64).to_be_bytes());
            hasher.update(value.as_bytes());
        } else {
            hasher.update(u64::MAX.to_be_bytes());
        }
    }
    hasher.update([
        u8::from(paths.has_auth_argument_override),
        u8::from(paths.has_kimi_model_name_override),
        u8::from(paths.is_legacy_kimi_cli),
    ]);
    Ok(hex_digest(&hasher.finalize()[..12]))
}

fn resolve_native_credentials(
    cli: TitleCli,
    paths: UsageProcessPaths,
    namespace: UsageNamespace,
) -> NativeCredentialResult {
    let result = match cli {
        TitleCli::Codex => resolve_codex(&paths, &namespace),
        TitleCli::Claude => resolve_claude(&paths, &namespace),
        TitleCli::Cursor => resolve_cursor(&paths, &namespace),
        TitleCli::Kimi => resolve_kimi(&paths, &namespace),
        _ => Err(AuthReadError::Unsupported(
            "Lomi does not have a verified account usage reader for this CLI yet.",
        )),
    };
    match result {
        Ok(Some((token, account_id, team_id, usage_endpoint))) => NativeCredentialResult::Ready(
            make_credential(cli, namespace, token, account_id, team_id, usage_endpoint),
        ),
        Ok(None) | Err(AuthReadError::Missing) => NativeCredentialResult::Status {
            namespace,
            status: UsageStatus::Unauthenticated,
            message: unauthenticated_message(cli),
        },
        Err(AuthReadError::Unsupported(message)) => NativeCredentialResult::Status {
            namespace,
            status: UsageStatus::Unsupported,
            message,
        },
        Err(AuthReadError::Error(message)) => NativeCredentialResult::Status {
            namespace,
            status: UsageStatus::Error,
            message,
        },
    }
}

fn unauthenticated_message(cli: TitleCli) -> &'static str {
    match cli {
        TitleCli::Codex => "Sign in to Codex with a ChatGPT account to see account usage.",
        TitleCli::Claude => "Sign in to Claude Code with a Claude account to see account usage.",
        TitleCli::Cursor => "Sign in to Cursor with an account to see plan usage.",
        TitleCli::Kimi => "Sign in to Kimi Code with an account to see account usage.",
        _ => "Sign in to the CLI with an account to see account usage.",
    }
}

fn make_credential(
    cli: TitleCli,
    namespace: UsageNamespace,
    token: String,
    account_id: Option<String>,
    team_id: Option<String>,
    usage_endpoint: Option<&'static str>,
) -> NativeCredential {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.update([0]);
    if let Some(account_id) = &account_id {
        hasher.update(account_id.as_bytes());
    }
    hasher.update([0]);
    if let Some(team_id) = &team_id {
        hasher.update(team_id.as_bytes());
    }
    let identity = hex_digest(hasher.finalize().as_slice());
    NativeCredential {
        cli,
        namespace,
        identity,
        token,
        account_id,
        team_id,
        usage_endpoint,
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn canonical_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn resolve_codex(
    paths: &UsageProcessPaths,
    namespace: &UsageNamespace,
) -> Result<Option<ProviderAuth>, AuthReadError> {
    if paths.has_auth_argument_override
        || paths.has_openai_api_key
        || paths.has_openai_custom_base_url
    {
        return Err(AuthReadError::Unsupported(
            "Codex is using an API key or custom OpenAI endpoint, so its ChatGPT account usage is unavailable.",
        ));
    }
    let config_path = namespace.directory.join("config.toml");
    reject_codex_custom_provider(&config_path)?;
    let mode = codex_storage_mode(&config_path)?;
    let document = match mode {
        CodexStorageMode::File => read_optional_json(&namespace.directory.join("auth.json"))?,
        CodexStorageMode::Keyring => read_codex_keyring(&namespace.directory)?,
        CodexStorageMode::Auto => match read_codex_keyring(&namespace.directory) {
            Ok(Some(value)) => Some(value),
            Ok(None) | Err(AuthReadError::Error(_)) => {
                read_optional_json(&namespace.directory.join("auth.json"))?
            }
            Err(error) => return Err(error),
        },
        CodexStorageMode::Ephemeral => {
            return Err(AuthReadError::Unsupported(
                "Codex uses session-only credentials that Lomi cannot read for account usage.",
            ));
        }
    };
    let Some(document) = document else {
        return Ok(None);
    };
    let auth: CodexAuth = serde_json::from_value(document)
        .map_err(|_| AuthReadError::Error("Codex credentials could not be read safely."))?;
    let auth_mode = auth.auth_mode.as_deref().unwrap_or_default();
    if matches!(auth_mode, "apikey" | "api_key" | "api-key") || auth.openai_api_key.is_some() {
        return Err(AuthReadError::Unsupported(
            "Codex is using API-key authentication, which has no ChatGPT plan quota to display.",
        ));
    }
    let Some(tokens) = auth.tokens else {
        return Ok(None);
    };
    let token = tokens
        .access_token
        .filter(|token| !token.trim().is_empty())
        .ok_or(AuthReadError::Missing)?;
    Ok(Some((
        token,
        tokens.account_id.filter(|account| !account.is_empty()),
        None,
        None,
    )))
}

#[derive(Deserialize)]
struct CodexAuth {
    #[serde(default)]
    auth_mode: Option<String>,
    #[serde(rename = "OPENAI_API_KEY", default)]
    openai_api_key: Option<String>,
    #[serde(default)]
    tokens: Option<CodexTokens>,
}

#[derive(Deserialize)]
struct CodexTokens {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
}

#[derive(Clone, Copy)]
enum CodexStorageMode {
    Auto,
    File,
    Keyring,
    Ephemeral,
}

fn codex_storage_mode(config_path: &Path) -> Result<CodexStorageMode, AuthReadError> {
    let Some(bytes) = read_optional_bytes(config_path, MAX_CREDENTIAL_BYTES)? else {
        return Ok(CodexStorageMode::File);
    };
    let source = std::str::from_utf8(&bytes)
        .map_err(|_| AuthReadError::Error("Codex settings could not be read safely."))?;
    let document = source
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| AuthReadError::Error("Codex settings could not be read safely."))?;
    match document
        .get("cli_auth_credentials_store")
        .and_then(toml_edit::Item::as_str)
    {
        None | Some("file") => Ok(CodexStorageMode::File),
        Some("auto") => Ok(CodexStorageMode::Auto),
        Some("keyring") => Ok(CodexStorageMode::Keyring),
        Some("ephemeral") => Ok(CodexStorageMode::Ephemeral),
        Some(_) => Err(AuthReadError::Unsupported(
            "Codex uses an unsupported credentials store for account usage.",
        )),
    }
}

fn reject_codex_custom_provider(config_path: &Path) -> Result<(), AuthReadError> {
    let Some(bytes) = read_optional_bytes(config_path, MAX_CREDENTIAL_BYTES)? else {
        return Ok(());
    };
    let source = std::str::from_utf8(&bytes)
        .map_err(|_| AuthReadError::Error("Codex settings could not be read safely."))?;
    let document = source
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| AuthReadError::Error("Codex settings could not be read safely."))?;
    if document
        .get("model_provider")
        .and_then(toml_edit::Item::as_str)
        .is_some_and(|provider| provider != "openai")
    {
        return Err(AuthReadError::Unsupported(
            "Codex is configured for a custom model provider, so ChatGPT account usage is unavailable.",
        ));
    }
    if document
        .get("model_providers")
        .and_then(toml_edit::Item::as_table_like)
        .and_then(|providers| providers.get("openai"))
        .and_then(toml_edit::Item::as_table_like)
        .is_some_and(|provider| provider.get("base_url").is_some())
    {
        return Err(AuthReadError::Unsupported(
            "Codex is configured with a custom endpoint, so ChatGPT account usage is unavailable.",
        ));
    }
    if document
        .get("chatgpt_base_url")
        .and_then(toml_edit::Item::as_str)
        .is_some_and(|base_url| !base_url.is_empty())
    {
        return Err(AuthReadError::Unsupported(
            "Codex is configured with a custom ChatGPT endpoint, so account usage is unavailable.",
        ));
    }
    Ok(())
}

fn read_codex_keyring(directory: &Path) -> Result<Option<Value>, AuthReadError> {
    let canonical = canonical_or_original(directory);
    let path = canonical.to_str().ok_or(AuthReadError::Error(
        "Codex credentials could not be located safely.",
    ))?;
    let digest = Sha256::digest(path.as_bytes());
    let account = format!("cli|{}", hex_digest(&digest[..8]));
    let Some(serialized) = read_native_keychain("Codex Auth", &account)? else {
        return Ok(None);
    };
    serde_json::from_str(&serialized)
        .map(Some)
        .map_err(|_| AuthReadError::Error("Codex credentials could not be read safely."))
}

fn read_native_keychain(service: &str, account: &str) -> Result<Option<String>, AuthReadError> {
    match crate::credential_store::get_password(service, account, MAX_CREDENTIAL_BYTES) {
        Ok(secret) if secret.len() <= MAX_CREDENTIAL_BYTES => {
            #[cfg(target_os = "macos")]
            {
                let secret = secret.trim_end_matches(['\r', '\n']).to_owned();
                Ok((!secret.is_empty()).then_some(secret))
            }
            #[cfg(not(target_os = "macos"))]
            Ok(Some(secret))
        }
        Ok(_) | Err(keyring::Error::TooLong(_, _)) => Err(AuthReadError::Error(
            "CLI credentials exceed the safe size limit.",
        )),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(AuthReadError::Error(
            "The system credential store is locked or unavailable.",
        )),
    }
}

fn resolve_claude(
    paths: &UsageProcessPaths,
    _namespace: &UsageNamespace,
) -> Result<Option<ProviderAuth>, AuthReadError> {
    if paths.has_auth_argument_override || paths.has_claude_auth_override {
        return Err(AuthReadError::Unsupported(
            "Claude Code is using an API key, custom endpoint, or hosted provider; account usage is unavailable.",
        ));
    }
    #[cfg(target_os = "macos")]
    let document: Value = {
        let service = claude_keychain_service(paths)?;
        let account = claude_keychain_account(paths);
        let Some(serialized) = read_native_keychain(&service, &account)? else {
            return Ok(None);
        };
        serde_json::from_str(&serialized).map_err(|_| {
            AuthReadError::Error("Claude Code credentials could not be read safely.")
        })?
    };
    #[cfg(not(target_os = "macos"))]
    let document = match read_optional_json(&_namespace.directory.join(".credentials.json"))? {
        Some(document) => document,
        None => return Ok(None),
    };
    let oauth = document
        .get("claudeAiOauth")
        .and_then(Value::as_object)
        .ok_or(AuthReadError::Error(
            "Claude Code credentials could not be read safely.",
        ))?;
    let token = oauth
        .get("accessToken")
        .and_then(Value::as_str)
        .filter(|token| !token.trim().is_empty())
        .ok_or(AuthReadError::Missing)?
        .to_owned();
    if let Some(scopes) = oauth.get("scopes").and_then(Value::as_array) {
        if !scopes
            .iter()
            .filter_map(Value::as_str)
            .any(|scope| scope == "user:profile")
        {
            return Err(AuthReadError::Unsupported(
                "This Claude Code login does not grant access to account usage details.",
            ));
        }
    }
    Ok(Some((token, None, None, None)))
}

#[cfg(target_os = "macos")]
fn claude_keychain_service(paths: &UsageProcessPaths) -> Result<String, AuthReadError> {
    let custom_path = if paths.claude_secure_storage_config_dir_set {
        paths.claude_secure_storage_config_dir.as_ref()
    } else {
        paths.claude_config_dir.as_ref()
    };
    let Some(path) = custom_path else {
        return Ok("Claude Code-credentials".to_owned());
    };
    let path = path.as_path();
    let path = path
        .to_str()
        .filter(|path| path.is_ascii())
        .ok_or(AuthReadError::Unsupported(
            "Claude Code uses a custom Unicode keychain location that Lomi cannot match safely.",
        ))?;
    let digest = Sha256::digest(path.as_bytes());
    Ok(format!(
        "Claude Code-credentials-{}",
        hex_digest(&digest[..4])
    ))
}

#[cfg(target_os = "macos")]
fn claude_keychain_account(paths: &UsageProcessPaths) -> String {
    let current_user = std::env::var("USER").ok();
    paths
        .user
        .as_deref()
        .or(current_user.as_deref())
        .filter(|user| {
            !user.is_empty()
                && user
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        })
        .unwrap_or("claude-code-user")
        .to_owned()
}

fn resolve_cursor(
    paths: &UsageProcessPaths,
    namespace: &UsageNamespace,
) -> Result<Option<ProviderAuth>, AuthReadError> {
    if paths.has_auth_argument_override
        || paths.has_cursor_api_key
        || paths.has_cursor_custom_backend
    {
        return Err(AuthReadError::Unsupported(
            "Cursor is using an API key or custom endpoint, so plan usage is unavailable.",
        ));
    }
    if paths.has_cursor_custom_config_dir {
        return Err(AuthReadError::Unsupported(
            "Cursor uses a custom configuration directory, so Lomi cannot match its saved account safely.",
        ));
    }
    let store = paths
        .cursor_credential_store
        .as_deref()
        .map(str::to_ascii_lowercase);
    if store.as_deref() == Some("memory") {
        return Err(AuthReadError::Unsupported(
            "Cursor uses session-only credentials that Lomi cannot read for account usage.",
        ));
    }
    if store
        .as_deref()
        .is_some_and(|store| !matches!(store, "file" | "memory"))
    {
        return Err(AuthReadError::Unsupported(
            "Cursor uses an unsupported credential store for account usage.",
        ));
    }
    let use_file = store.as_deref() == Some("file") || !cfg!(target_os = "macos");
    let token = if use_file {
        let document = read_optional_json(&namespace.directory.join("auth.json"))?;
        let Some(document) = document else {
            return Ok(None);
        };
        if document
            .get("apiKey")
            .and_then(Value::as_str)
            .is_some_and(|api_key| !api_key.is_empty())
        {
            return Err(AuthReadError::Unsupported(
                "Cursor is using API-key authentication, so account plan usage is unavailable.",
            ));
        }
        document
            .get("accessToken")
            .and_then(Value::as_str)
            .filter(|token| !token.trim().is_empty())
            .ok_or(AuthReadError::Missing)?
            .to_owned()
    } else {
        if !process_home_matches(paths) {
            return Err(AuthReadError::Unsupported(
                "Cursor uses the current macOS keychain with a custom process home, so Lomi cannot match the right account safely.",
            ));
        }
        let Some(token) = read_native_keychain("cursor-access-token", "cursor-user")? else {
            return Ok(None);
        };
        token
    };
    let team_id = read_cursor_active_team(paths)?;
    Ok(Some((token, None, team_id, None)))
}

const KIMI_MAINLAND_BASE_URL: &str = "https://api.kimi.com/coding/v1";
const KIMI_GLOBAL_BASE_URL: &str = "https://api.kimi.ai/coding/v1";
const KIMI_MAINLAND_OAUTH_HOST: &str = "https://auth.kimi.com";
const KIMI_GLOBAL_OAUTH_HOST: &str = "https://auth.kimi.ai";
const KIMI_MAINLAND_USAGE_ENDPOINT: &str = "https://api.kimi.com/coding/v1/usages";
const KIMI_GLOBAL_USAGE_ENDPOINT: &str = "https://api.kimi.ai/coding/v1/usages";

struct KimiSelection {
    credential_file: String,
    usage_endpoint: &'static str,
}

fn resolve_kimi(
    paths: &UsageProcessPaths,
    namespace: &UsageNamespace,
) -> Result<Option<ProviderAuth>, AuthReadError> {
    if paths.is_legacy_kimi_cli {
        return Err(AuthReadError::Unsupported(
            "Legacy Python Kimi uses a separate account store that Lomi does not read.",
        ));
    }
    if paths.has_auth_argument_override || paths.has_kimi_model_name_override {
        return Err(AuthReadError::Unsupported(
            "Kimi Code is using a temporary model override, so its managed account usage is unavailable.",
        ));
    }
    let expected_store = format!(
        "kimi-{}",
        kimi_config_scope(paths).map_err(AuthReadError::Error)?
    );
    if namespace.store != expected_store {
        return Err(AuthReadError::Error(
            "Kimi Code settings changed while usage was being checked.",
        ));
    }
    let selection = resolve_kimi_selection(paths)?;
    let directory = paths
        .directory_for(TitleCli::Kimi)
        .map_err(|_| AuthReadError::Error("Kimi Code credentials could not be located safely."))?;
    let Some(document) = read_optional_json(
        &directory
            .join("credentials")
            .join(&selection.credential_file),
    )?
    else {
        return Ok(None);
    };
    let token = document
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|token| !token.trim().is_empty())
        .ok_or(AuthReadError::Missing)?
        .to_owned();
    let expires_at = document
        .get("expires_at")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or(AuthReadError::Error(
            "Kimi Code credentials could not be read safely.",
        ))?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    if expires_at <= now + 30.0 {
        return Err(AuthReadError::Missing);
    }
    Ok(Some((token, None, None, Some(selection.usage_endpoint))))
}

fn resolve_kimi_selection(paths: &UsageProcessPaths) -> Result<KimiSelection, AuthReadError> {
    let directory = paths
        .directory_for(TitleCli::Kimi)
        .map_err(|_| AuthReadError::Error("Kimi Code settings could not be located safely."))?;
    let config = read_optional_bytes(&directory.join("config.toml"), MAX_CREDENTIAL_BYTES)?;
    let mut configured_base_url = None;
    let mut configured_oauth_host = None;
    let mut configured_oauth_key = None;
    if let Some(config) = config {
        let source = std::str::from_utf8(&config)
            .map_err(|_| AuthReadError::Error("Kimi Code settings could not be read safely."))?;
        let document = source
            .parse::<toml_edit::DocumentMut>()
            .map_err(|_| AuthReadError::Error("Kimi Code settings could not be read safely."))?;
        let default_model = document
            .get("default_model")
            .and_then(toml_edit::Item::as_str)
            .ok_or(AuthReadError::Unsupported(
                "Kimi Code does not have a verified default account model selected.",
            ))?;
        let models = document
            .get("models")
            .and_then(toml_edit::Item::as_table_like)
            .ok_or(AuthReadError::Unsupported(
                "Kimi Code's selected model provider cannot be verified safely.",
            ))?;
        let model = models
            .get(default_model)
            .and_then(toml_edit::Item::as_table_like)
            .ok_or(AuthReadError::Unsupported(
                "Kimi Code's selected model provider cannot be verified safely.",
            ))?;
        if model.get("provider").and_then(toml_edit::Item::as_str) != Some("managed:kimi-code")
            || model.get("base_url").is_some()
            || model.get("protocol").is_some()
        {
            return Err(AuthReadError::Unsupported(
                "Kimi Code's selected model does not use its verified managed account provider.",
            ));
        }
        if let Some(overrides) = model
            .get("overrides")
            .and_then(toml_edit::Item::as_table_like)
        {
            if ["provider", "model", "base_url", "protocol"]
                .iter()
                .any(|field| overrides.get(field).is_some())
            {
                return Err(AuthReadError::Unsupported(
                    "Kimi Code's selected model has a custom provider override.",
                ));
            }
        }
        let provider = document
            .get("providers")
            .and_then(toml_edit::Item::as_table_like)
            .and_then(|providers| providers.get("managed:kimi-code"))
            .and_then(toml_edit::Item::as_table_like)
            .ok_or(AuthReadError::Unsupported(
                "Kimi Code's managed account provider cannot be verified safely.",
            ))?;
        if provider.get("type").and_then(toml_edit::Item::as_str) != Some("kimi") {
            return Err(AuthReadError::Unsupported(
                "Kimi Code's selected provider is not the verified Kimi account provider.",
            ));
        }
        if provider.get("base_url").is_some()
            && provider
                .get("base_url")
                .and_then(toml_edit::Item::as_str)
                .is_none()
        {
            return Err(AuthReadError::Error(
                "Kimi Code settings could not be read safely.",
            ));
        }
        let api_key = provider.get("api_key");
        if api_key.is_some_and(|item| item.as_str().is_none_or(|key| !key.is_empty()))
            || provider.get("api_key_env").is_some()
            || provider.get("custom_headers").is_some()
        {
            return Err(AuthReadError::Unsupported(
                "Kimi Code is configured with API-key or custom-header authentication.",
            ));
        }
        if let Some(env) = provider.get("env").and_then(toml_edit::Item::as_table_like) {
            if env.get("KIMI_API_KEY").is_some() || env.get("KIMI_BASE_URL").is_some() {
                return Err(AuthReadError::Unsupported(
                    "Kimi Code is configured with API-key or custom-endpoint authentication.",
                ));
            }
        }
        configured_base_url = provider
            .get("base_url")
            .and_then(toml_edit::Item::as_str)
            .map(str::to_owned);
        if let Some(oauth) = provider.get("oauth") {
            let oauth = oauth.as_table_like().ok_or(AuthReadError::Unsupported(
                "Kimi Code uses an unsupported account credential store.",
            ))?;
            if oauth
                .get("storage")
                .is_some_and(|storage| storage.as_str() != Some("file"))
            {
                return Err(AuthReadError::Unsupported(
                    "Kimi Code uses an unsupported account credential store.",
                ));
            }
            if oauth.get("key").is_some()
                && oauth.get("key").and_then(toml_edit::Item::as_str).is_none()
            {
                return Err(AuthReadError::Unsupported(
                    "Kimi Code uses an unsupported account credential name.",
                ));
            }
            if ["oauth_host", "oauthHost"]
                .iter()
                .any(|field| oauth.get(field).is_some_and(|host| host.as_str().is_none()))
            {
                return Err(AuthReadError::Unsupported(
                    "Kimi Code uses an unverified account region or custom endpoint.",
                ));
            }
            configured_oauth_key = oauth
                .get("key")
                .and_then(toml_edit::Item::as_str)
                .map(str::to_owned);
            configured_oauth_host = oauth
                .get("oauth_host")
                .or_else(|| oauth.get("oauthHost"))
                .and_then(toml_edit::Item::as_str)
                .map(str::to_owned);
        }
    }

    let env_base_url = paths.kimi_code_base_url.as_deref();
    let env_oauth_host = paths
        .kimi_code_oauth_host
        .as_deref()
        .or(paths.kimi_oauth_host.as_deref());
    let has_env_override = env_base_url.is_some() || env_oauth_host.is_some();
    let base_url = env_base_url
        .or(configured_base_url.as_deref())
        .unwrap_or(KIMI_MAINLAND_BASE_URL)
        .trim_end_matches('/');
    let oauth_host = if has_env_override {
        env_oauth_host.unwrap_or(KIMI_MAINLAND_OAUTH_HOST)
    } else {
        configured_oauth_host
            .as_deref()
            .unwrap_or(KIMI_MAINLAND_OAUTH_HOST)
    }
    .trim_end_matches('/');
    let (expected_key, usage_endpoint) = match (oauth_host, base_url) {
        (KIMI_MAINLAND_OAUTH_HOST, KIMI_MAINLAND_BASE_URL) => {
            ("oauth/kimi-code".to_owned(), KIMI_MAINLAND_USAGE_ENDPOINT)
        }
        (KIMI_GLOBAL_OAUTH_HOST, KIMI_GLOBAL_BASE_URL) => (
            kimi_scoped_oauth_key(oauth_host, base_url),
            KIMI_GLOBAL_USAGE_ENDPOINT,
        ),
        _ => {
            return Err(AuthReadError::Unsupported(
                "Kimi Code is using an unverified account region or custom endpoint.",
            ));
        }
    };
    if !has_env_override
        && configured_oauth_key
            .as_deref()
            .is_some_and(|key| key != expected_key)
    {
        return Err(AuthReadError::Unsupported(
            "Kimi Code's selected account credential does not match its configured region.",
        ));
    }
    let credential_name = expected_key
        .strip_prefix("oauth/")
        .ok_or(AuthReadError::Error(
            "Kimi Code credentials could not be located safely.",
        ))?;
    if credential_name.is_empty()
        || !credential_name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(AuthReadError::Unsupported(
            "Kimi Code uses an unsupported account credential name.",
        ));
    }
    Ok(KimiSelection {
        credential_file: format!("{credential_name}.json"),
        usage_endpoint,
    })
}

fn kimi_scoped_oauth_key(oauth_host: &str, base_url: &str) -> String {
    let serialized = format!(r#"{{"oauthHost":"{oauth_host}","baseUrl":"{base_url}"}}"#);
    let digest = Sha256::digest(serialized.as_bytes());
    format!("oauth/kimi-code-env-{}", hex_digest(&digest[..8]))
}

fn process_home_matches(paths: &UsageProcessPaths) -> bool {
    let Some(process_home) = paths.home.as_ref() else {
        return false;
    };
    let Some(host_home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return false;
    };
    canonical_or_original(process_home) == canonical_or_original(&host_home)
}

fn read_cursor_active_team(paths: &UsageProcessPaths) -> Result<Option<String>, AuthReadError> {
    let directory = paths.cursor_cli_config_directory().map_err(|_| {
        AuthReadError::Error("Cursor account settings could not be located safely.")
    })?;
    let Some(document) = read_optional_json(&directory.join("cli-config.json"))? else {
        return Ok(None);
    };
    let team_id = document
        .get("authInfo")
        .and_then(Value::as_object)
        .and_then(|auth| auth.get("activeTeamId"))
        .and_then(Value::as_u64)
        .filter(|team| (1..=9_007_199_254_740_991).contains(team))
        .map(|team| team.to_string());
    Ok(team_id)
}

fn read_optional_json(path: &Path) -> Result<Option<Value>, AuthReadError> {
    let Some(bytes) = read_optional_bytes(path, MAX_CREDENTIAL_BYTES)? else {
        return Ok(None);
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| AuthReadError::Error("CLI credentials could not be read safely."))
}

fn read_optional_bytes(path: &Path, limit: usize) -> Result<Option<Vec<u8>>, AuthReadError> {
    let file = match open_regular_file(path) {
        Ok(file) => file,
        Err(OpenCredentialError::Missing) => return Ok(None),
        Err(OpenCredentialError::Unsafe) => {
            return Err(AuthReadError::Error(
                "CLI credentials could not be read safely.",
            ));
        }
    };
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| AuthReadError::Error("CLI credentials could not be read safely."))?;
    if bytes.len() > limit {
        return Err(AuthReadError::Error(
            "CLI credentials exceed the safe size limit.",
        ));
    }
    Ok(Some(bytes))
}

enum OpenCredentialError {
    Missing,
    Unsafe,
}

fn open_regular_file(path: &Path) -> Result<std::fs::File, OpenCredentialError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Opening a FIFO without O_NONBLOCK can block before we have a chance to
        // inspect the descriptor's file type.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(OpenCredentialError::Missing);
        }
        Err(_) => return Err(OpenCredentialError::Unsafe),
    };
    if !file
        .metadata()
        .map_err(|_| OpenCredentialError::Unsafe)?
        .is_file()
    {
        return Err(OpenCredentialError::Unsafe);
    }
    Ok(file)
}

async fn current_generation(state: &CliUsage, namespace: &UsageNamespace) -> u64 {
    state
        .cache
        .lock()
        .await
        .generation
        .get(namespace)
        .copied()
        .unwrap_or_default()
}

async fn activate_identity(
    state: &CliUsage,
    key: &CacheKey,
    observed_generation: u64,
) -> Option<u64> {
    let mut cache = state.cache.lock().await;
    let current_generation = cache
        .generation
        .get(&key.namespace)
        .copied()
        .unwrap_or_default();
    let current_identity = cache.active_identity.get(&key.namespace);
    if current_generation != observed_generation
        && current_identity.is_none_or(|identity| identity != &key.identity)
    {
        return None;
    }
    if current_identity.is_some_and(|identity| identity != &key.identity) {
        cache
            .records
            .retain(|old_key, _| old_key.namespace != key.namespace);
        let next_generation = current_generation.saturating_add(1);
        cache
            .generation
            .insert(key.namespace.clone(), next_generation);
        cache
            .active_identity
            .insert(key.namespace.clone(), key.identity.clone());
        prune_cache(&mut cache);
        return Some(next_generation);
    }
    if current_identity.is_none() {
        let next_generation = current_generation.saturating_add(1);
        cache
            .generation
            .insert(key.namespace.clone(), next_generation);
        cache
            .active_identity
            .insert(key.namespace.clone(), key.identity.clone());
        prune_cache(&mut cache);
        return Some(next_generation);
    }
    prune_cache(&mut cache);
    Some(current_generation)
}

async fn invalidate_namespace_if_generation(
    state: &CliUsage,
    namespace: &UsageNamespace,
    expected_generation: u64,
) {
    let mut cache = state.cache.lock().await;
    if cache.generation.get(namespace).copied().unwrap_or_default() != expected_generation {
        return;
    }
    cache.records.retain(|key, _| &key.namespace != namespace);
    cache.active_identity.remove(namespace);
    cache
        .generation
        .insert(namespace.clone(), expected_generation.saturating_add(1));
    cache
        .locks
        .retain(|key, weak| &key.namespace != namespace || weak.strong_count() > 0);
}

fn terminal_process_is_current(terminals: &Terminals, id: &str, process: TitleProcess) -> bool {
    terminals.check_title_process(id, process).is_ok()
}

async fn verify_credentials_current(
    terminals: &Terminals,
    id: &str,
    state: &CliUsage,
    process: TitleProcess,
    expected_key: &CacheKey,
    expected_generation: u64,
) -> Option<u64> {
    let paths =
        tauri::async_runtime::spawn_blocking(move || cli_titles::usage_process_paths(process))
            .await
            .ok()?
            .ok()?;
    if !terminal_process_is_current(terminals, id, process) {
        return None;
    }
    let namespace_paths = paths.clone();
    let namespace = tauri::async_runtime::spawn_blocking(move || {
        native_namespace(process.cli, &namespace_paths)
    })
    .await
    .ok()?
    .ok()?;
    // Take the generation before reading credentials. Otherwise a concurrent logout/token
    // replacement could be overwritten by activating an identity from a stale read.
    let observed_generation = current_generation(state, &namespace).await;
    let result = tauri::async_runtime::spawn_blocking(move || {
        resolve_native_credentials(process.cli, paths, namespace)
    })
    .await
    .ok()?;
    if !terminal_process_is_current(terminals, id, process) {
        return None;
    }
    match result {
        NativeCredentialResult::Ready(current) => {
            let current_key = CacheKey {
                namespace: current.namespace.clone(),
                identity: current.identity.clone(),
            };
            if current_key != *expected_key {
                let _ = activate_identity(state, &current_key, observed_generation).await;
                return None;
            }
            let generation = activate_identity(state, &current_key, observed_generation).await?;
            (generation == expected_generation).then_some(generation)
        }
        NativeCredentialResult::Status { namespace, .. } => {
            invalidate_namespace_if_generation(state, &namespace, observed_generation).await;
            None
        }
    }
}

async fn request_lock(state: &CliUsage, key: &CacheKey) -> Arc<Mutex<()>> {
    let mut cache = state.cache.lock().await;
    cache.locks.retain(|_, weak| weak.strong_count() > 0);
    if let Some(lock) = cache.locks.get(key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    cache.locks.insert(key.clone(), Arc::downgrade(&lock));
    lock
}

async fn cached_snapshot(state: &CliUsage, key: &CacheKey, force: bool) -> Option<UsageSnapshot> {
    let mut cache = state.cache.lock().await;
    let record = cache.records.get_mut(key)?;
    record.last_used = Instant::now();
    let now = Instant::now();
    let forced_cooldown = force
        && record
            .last_forced
            .is_some_and(|last| now.duration_since(last) < FORCE_COOLDOWN);
    let fresh_success = !force
        && record.status == UsageStatus::Ready
        && record
            .last_success
            .is_some_and(|last| now.duration_since(last) < SUCCESS_TTL);
    let backoff = record.next_retry.is_some_and(|retry| now < retry);
    (forced_cooldown || fresh_success || backoff).then(|| record.clone())
}

async fn mark_request_started(
    state: &CliUsage,
    key: &CacheKey,
    force: bool,
    expected_generation: u64,
) -> bool {
    let mut cache = state.cache.lock().await;
    if cache
        .generation
        .get(&key.namespace)
        .copied()
        .unwrap_or_default()
        != expected_generation
        || cache.active_identity.get(&key.namespace) != Some(&key.identity)
    {
        return false;
    }
    if let Some(record) = cache.records.get_mut(key) {
        if force {
            record.last_forced = Some(Instant::now());
        }
        record.last_used = Instant::now();
    } else {
        cache.records.insert(
            key.clone(),
            UsageSnapshot {
                status: UsageStatus::Error,
                windows: Vec::new(),
                updated_at: None,
                retry_at: None,
                message: None,
                source: Some(source_name(key.namespace.cli)),
                next_retry: None,
                last_success: None,
                last_forced: force.then(Instant::now),
                failure_count: 0,
                last_used: Instant::now(),
            },
        );
    }
    true
}

async fn store_snapshot(
    state: &CliUsage,
    key: CacheKey,
    expected_generation: u64,
    snapshot: UsageSnapshot,
) -> bool {
    let mut cache = state.cache.lock().await;
    if cache
        .generation
        .get(&key.namespace)
        .copied()
        .unwrap_or_default()
        != expected_generation
        || cache.active_identity.get(&key.namespace) != Some(&key.identity)
    {
        return false;
    }
    cache.records.insert(key, snapshot);
    prune_cache(&mut cache);
    true
}

async fn failed_snapshot(
    state: &CliUsage,
    key: &CacheKey,
    message: &'static str,
    status: UsageStatus,
    retry_after: Option<Duration>,
    expected_generation: u64,
) -> Option<UsageSnapshot> {
    failed_snapshot_with_policy(
        state,
        key,
        FetchFailure {
            status,
            message,
            retry_after,
        },
        expected_generation,
        true,
    )
    .await
}

async fn failed_snapshot_clearing_values(
    state: &CliUsage,
    key: &CacheKey,
    failure: FetchFailure,
    expected_generation: u64,
) -> Option<UsageSnapshot> {
    failed_snapshot_with_policy(state, key, failure, expected_generation, false).await
}

async fn failed_snapshot_with_policy(
    state: &CliUsage,
    key: &CacheKey,
    failure: FetchFailure,
    expected_generation: u64,
    preserve_values: bool,
) -> Option<UsageSnapshot> {
    let mut cache = state.cache.lock().await;
    if cache
        .generation
        .get(&key.namespace)
        .copied()
        .unwrap_or_default()
        != expected_generation
        || cache.active_identity.get(&key.namespace) != Some(&key.identity)
    {
        return None;
    }
    let previous = cache.records.get(key).cloned();
    if !preserve_values {
        cache
            .records
            .retain(|old_key, _| old_key.namespace != key.namespace);
    }
    let count = previous
        .as_ref()
        .map(|snapshot| snapshot.failure_count.saturating_add(1))
        .unwrap_or(1);
    let delay = failure
        .retry_after
        .unwrap_or_else(|| backoff_delay(count, failure.status));
    let next_retry = Instant::now() + delay;
    let mut snapshot = previous.unwrap_or_else(|| UsageSnapshot {
        status: failure.status,
        windows: Vec::new(),
        updated_at: None,
        retry_at: None,
        message: None,
        source: Some(source_name(key.namespace.cli)),
        next_retry: None,
        last_success: None,
        last_forced: None,
        failure_count: 0,
        last_used: Instant::now(),
    });
    snapshot.status = failure.status;
    snapshot.message = Some(failure.message.to_owned());
    if !preserve_values {
        snapshot.windows.clear();
        snapshot.updated_at = None;
        snapshot.last_success = None;
    }
    snapshot.retry_at =
        Some(now_ms().saturating_add(delay.as_millis().min(i64::MAX as u128) as i64));
    snapshot.next_retry = Some(next_retry);
    snapshot.failure_count = count;
    snapshot.last_used = Instant::now();
    cache.records.insert(key.clone(), snapshot.clone());
    prune_cache(&mut cache);
    Some(snapshot)
}

fn backoff_delay(failure_count: u32, status: UsageStatus) -> Duration {
    let base = if status == UsageStatus::RateLimited {
        30
    } else {
        5
    };
    let exponent = failure_count.saturating_sub(1).min(8);
    Duration::from_secs((base * (1u64 << exponent)).min(900))
}

fn prune_cache(cache: &mut UsageCache) {
    if cache.records.len() > MAX_CACHE_ENTRIES {
        let mut keys = cache
            .records
            .iter()
            .map(|(key, value)| (key.clone(), value.last_used))
            .collect::<Vec<_>>();
        keys.sort_by_key(|(_, last_used)| *last_used);
        let excess = keys.len() - MAX_CACHE_ENTRIES;
        for (key, _) in keys.into_iter().take(excess) {
            cache.records.remove(&key);
        }
    }
    if cache.active_identity.len() > MAX_CACHE_ENTRIES {
        let retained = cache
            .records
            .keys()
            .map(|key| key.namespace.clone())
            .collect::<std::collections::HashSet<_>>();
        cache
            .active_identity
            .retain(|namespace, _| retained.contains(namespace));
    }
}

#[derive(Debug)]
struct FetchFailure {
    status: UsageStatus,
    message: &'static str,
    retry_after: Option<Duration>,
}

async fn fetch_usage(
    client: &reqwest::Client,
    credential: &NativeCredential,
) -> Result<Vec<UsageWindow>, FetchFailure> {
    let mut request = match credential.cli {
        TitleCli::Codex => client
            .get("https://chatgpt.com/backend-api/wham/usage")
            .header(reqwest::header::USER_AGENT, "codex-cli"),
        TitleCli::Claude => client
            .get("https://api.anthropic.com/api/oauth/usage")
            .header("anthropic-beta", "oauth-2025-04-20"),
        TitleCli::Cursor => client
            .post("https://api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("connect-protocol-version", "1")
            .header("x-cursor-client-type", "cli")
            .body("{}"),
        TitleCli::Kimi => {
            let Some(endpoint) = credential.usage_endpoint else {
                return Err(FetchFailure {
                    status: UsageStatus::Unsupported,
                    message: "Kimi Code is using an unverified account region or custom endpoint.",
                    retry_after: None,
                });
            };
            client
                .get(endpoint)
                .header(reqwest::header::ACCEPT, "application/json")
        }
        _ => {
            return Err(FetchFailure {
                status: UsageStatus::Unsupported,
                message: "Lomi does not have a verified account usage reader for this CLI yet.",
                retry_after: None,
            });
        }
    }
    .bearer_auth(&credential.token);
    if let Some(account_id) = &credential.account_id {
        request = request.header("ChatGPT-Account-Id", account_id);
    }
    if let Some(team_id) = &credential.team_id {
        request = request.header("x-cursor-team-id", team_id);
    }
    let response = request.send().await.map_err(|_| FetchFailure {
        status: UsageStatus::Error,
        message: "Could not reach the CLI account usage service. Try again shortly.",
        retry_after: None,
    })?;
    let status = response.status();
    if status.as_u16() == 429 {
        let retry_after = parse_retry_after(response.headers().get(reqwest::header::RETRY_AFTER));
        return Err(FetchFailure {
            status: UsageStatus::RateLimited,
            message: "The account usage service is rate limiting checks. Lomi will retry later.",
            retry_after,
        });
    }
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(FetchFailure {
            status: UsageStatus::Unauthenticated,
            message: unauthenticated_message(credential.cli),
            retry_after: None,
        });
    }
    if !status.is_success() {
        return Err(FetchFailure {
            status: UsageStatus::Error,
            message: "The account usage service returned an error. Try again later.",
            retry_after: None,
        });
    }
    let body = bounded_response_body(response)
        .await
        .map_err(|_| FetchFailure {
            status: UsageStatus::Error,
            message: "The account usage response could not be read safely.",
            retry_after: None,
        })?;
    let value: Value = serde_json::from_slice(&body).map_err(|_| FetchFailure {
        status: UsageStatus::Error,
        message: "The account usage response format is not recognized.",
        retry_after: None,
    })?;
    match credential.cli {
        TitleCli::Codex => parse_codex_usage(&value),
        TitleCli::Claude => parse_claude_usage(&value),
        TitleCli::Cursor => parse_cursor_usage(&value),
        TitleCli::Kimi => parse_kimi_usage(&value),
        _ => unreachable!("unsupported providers return before parsing"),
    }
}

async fn bounded_response_body(mut response: reqwest::Response) -> Result<Vec<u8>, ()> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn parse_retry_after(value: Option<&reqwest::header::HeaderValue>) -> Option<Duration> {
    let value = value?.to_str().ok()?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds.clamp(1, 3600)));
    }
    let date = DateTime::<FixedOffset>::parse_from_rfc2822(value).ok()?;
    let delay = date.timestamp().saturating_sub(now_ms() / 1000);
    Some(Duration::from_secs(delay.clamp(1, 3600) as u64))
}

fn parse_codex_usage(value: &Value) -> Result<Vec<UsageWindow>, FetchFailure> {
    let Some(rate_limit) = value.get("rate_limit").and_then(Value::as_object) else {
        return Err(FetchFailure {
            status: UsageStatus::Error,
            message: "Codex account usage did not include rate limit details.",
            retry_after: None,
        });
    };
    let mut windows = Vec::new();
    append_codex_rate_windows(&mut windows, "Codex", rate_limit);
    if let Some(limits) = value
        .get("additional_rate_limits")
        .and_then(Value::as_array)
    {
        for limit in limits.iter().take(24) {
            let Some(details) = limit.get("rate_limit").and_then(Value::as_object) else {
                continue;
            };
            let label = limit
                .get("limit_name")
                .and_then(Value::as_str)
                .or_else(|| limit.get("metered_feature").and_then(Value::as_str))
                .unwrap_or("Additional quota");
            append_codex_rate_windows(&mut windows, label, details);
        }
    }
    if windows.is_empty() {
        return Err(FetchFailure {
            status: UsageStatus::Error,
            message: "Codex account usage did not include recognized quota windows.",
            retry_after: None,
        });
    }
    Ok(windows)
}

fn append_codex_rate_windows(
    output: &mut Vec<UsageWindow>,
    prefix: &str,
    rate_limit: &serde_json::Map<String, Value>,
) {
    for (field, fallback) in [
        ("primary_window", "Primary"),
        ("secondary_window", "Secondary"),
    ] {
        let Some(window) = rate_limit.get(field).and_then(Value::as_object) else {
            continue;
        };
        let used_percent =
            number_any(window, &["used_percent", "usedPercent"]).filter(|value| *value >= 0.0);
        let Some(used_percent) = used_percent else {
            continue;
        };
        let duration_seconds =
            number_any(window, &["limit_window_seconds", "windowDurationSeconds"]).or_else(|| {
                number_any(window, &["window_duration_mins", "windowDurationMins"])
                    .map(|minutes| minutes * 60.0)
            });
        let window_label = duration_seconds
            .map(format_duration)
            .unwrap_or_else(|| fallback.to_owned());
        let base = if prefix == "Codex" {
            String::new()
        } else {
            format!("{prefix} · ")
        };
        output.push(UsageWindow {
            label: format!("{base}{window_label}"),
            remaining_percent: Some(remaining_percent(used_percent)),
            used: None,
            limit: None,
            unit: None,
            resets_at: epoch_millis_any(window, &["reset_at", "resetsAt"]),
        });
    }
}

fn format_duration(seconds: f64) -> String {
    if (seconds / 86400.0).fract() == 0.0 && seconds >= 86400.0 {
        let days = (seconds / 86400.0) as u64;
        if days == 7 {
            "Weekly".to_owned()
        } else {
            format!("{days}-day")
        }
    } else if (seconds / 3600.0).fract() == 0.0 && seconds >= 3600.0 {
        format!("{}-hour", (seconds / 3600.0) as u64)
    } else if (seconds / 60.0).fract() == 0.0 && seconds >= 60.0 {
        format!("{}-minute", (seconds / 60.0) as u64)
    } else {
        format!("{}-second", seconds as u64)
    }
}

fn parse_claude_usage(value: &Value) -> Result<Vec<UsageWindow>, FetchFailure> {
    let Some(object) = value.as_object() else {
        return Err(unrecognized_response());
    };
    let known = [
        ("five_hour", "5-hour"),
        ("seven_day", "Weekly"),
        ("seven_day_opus", "Weekly Opus"),
        ("seven_day_sonnet", "Weekly Sonnet"),
        ("seven_day_oauth_apps", "Weekly OAuth apps"),
    ];
    let mut windows = Vec::new();
    for (field, label) in known {
        if let Some(bucket) = object.get(field).and_then(Value::as_object) {
            let utilization = number_any(bucket, &["utilization"]).filter(|value| *value >= 0.0);
            if let Some(utilization) = utilization {
                windows.push(UsageWindow {
                    label: label.into(),
                    remaining_percent: Some(remaining_percent(utilization)),
                    used: None,
                    limit: None,
                    unit: None,
                    resets_at: epoch_millis_any(bucket, &["resets_at", "resetsAt"]),
                });
            }
        }
    }
    if windows.is_empty() {
        return Err(unrecognized_response());
    }
    Ok(windows)
}

fn parse_cursor_usage(value: &Value) -> Result<Vec<UsageWindow>, FetchFailure> {
    let Some(plan_usage) = value.get("planUsage").and_then(Value::as_object) else {
        return Err(FetchFailure {
            status: UsageStatus::Unsupported,
            message: "Cursor does not provide usage details for this account or plan.",
            retry_after: None,
        });
    };
    let Some(used_percent) =
        number_any(plan_usage, &["totalPercentUsed"]).filter(|value| *value >= 0.0)
    else {
        return Err(unrecognized_response());
    };
    let resets_at = value
        .as_object()
        .and_then(|object| epoch_millis_any(object, &["billingCycleEnd"]));
    Ok(vec![UsageWindow {
        label: "Billing cycle".into(),
        remaining_percent: Some(remaining_percent(used_percent)),
        used: None,
        limit: None,
        unit: None,
        resets_at,
    }])
}

fn parse_kimi_usage(value: &Value) -> Result<Vec<UsageWindow>, FetchFailure> {
    let Some(usages) = value.get("usages").and_then(Value::as_object) else {
        return Err(unrecognized_response());
    };
    let known = [
        ("limit_5h", "5-hour"),
        ("limit_7d", "Weekly"),
        ("limit_month_total", "Monthly"),
        ("limit_month_code", "Monthly code"),
    ];
    let mut windows = Vec::new();
    for (field, label) in known {
        let Some(bucket) = usages.get(field).and_then(Value::as_object) else {
            continue;
        };
        let ratio = number_any(bucket, &["used_ratio"]).filter(|ratio| *ratio >= 0.0);
        let Some(ratio) = ratio else {
            continue;
        };
        windows.push(UsageWindow {
            label: label.into(),
            remaining_percent: Some(((1.0 - ratio).clamp(0.0, 1.0)) * 100.0),
            used: None,
            limit: None,
            unit: None,
            resets_at: epoch_millis_any(bucket, &["reset_time"]),
        });
    }
    if windows.is_empty() {
        return Err(unrecognized_response());
    }
    Ok(windows)
}

fn remaining_percent(used_percent: f64) -> f64 {
    (100.0 - used_percent).clamp(0.0, 100.0)
}

fn unrecognized_response() -> FetchFailure {
    FetchFailure {
        status: UsageStatus::Error,
        message: "The account usage response format is not recognized.",
        retry_after: None,
    }
}

fn number_any(object: &serde_json::Map<String, Value>, names: &[&str]) -> Option<f64> {
    names.iter().find_map(|name| {
        let value = object.get(*name)?;
        let number = value
            .as_f64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))?;
        number.is_finite().then_some(number)
    })
}

fn epoch_millis_any(object: &serde_json::Map<String, Value>, names: &[&str]) -> Option<i64> {
    names.iter().find_map(|name| {
        let value = object.get(*name)?;
        if let Some(number) = value.as_i64() {
            return epoch_millis(number);
        }
        let value = value.as_str()?;
        if let Ok(number) = value.parse::<i64>() {
            return epoch_millis(number);
        }
        DateTime::<FixedOffset>::parse_from_rfc3339(value)
            .ok()
            .map(|date| date.timestamp_millis())
            .and_then(valid_epoch_millis)
    })
}

fn epoch_millis(value: i64) -> Option<i64> {
    let millis = if value < 0 {
        return None;
    } else if value < 10_000_000_000 {
        value.checked_mul(1000)?
    } else {
        value
    };
    valid_epoch_millis(millis)
}

fn valid_epoch_millis(value: i64) -> Option<i64> {
    (DateTime::<chrono::Utc>::from_timestamp_millis(value).is_some() && value >= 0).then_some(value)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn test_key(identity: &str) -> CacheKey {
        CacheKey {
            namespace: UsageNamespace {
                cli: TitleCli::Codex,
                directory: PathBuf::from("/test/.codex"),
                store: "codex-native".into(),
            },
            identity: identity.to_owned(),
        }
    }

    fn successful_snapshot() -> UsageSnapshot {
        UsageSnapshot {
            status: UsageStatus::Ready,
            windows: vec![UsageWindow {
                label: "Weekly".into(),
                remaining_percent: Some(73.0),
                used: None,
                limit: None,
                unit: None,
                resets_at: Some(1_790_668_800_000),
            }],
            updated_at: Some(4242),
            retry_at: None,
            message: None,
            source: Some("codex".into()),
            next_retry: None,
            last_success: Some(Instant::now()),
            last_forced: None,
            failure_count: 0,
            last_used: Instant::now(),
        }
    }

    fn test_credential(
        cli: TitleCli,
        token: &str,
        account_id: Option<&str>,
        team_id: Option<&str>,
        usage_endpoint: Option<&'static str>,
    ) -> NativeCredential {
        make_credential(
            cli,
            UsageNamespace {
                cli,
                directory: PathBuf::from("/test/account-home"),
                store: "native".into(),
            },
            token.into(),
            account_id.map(str::to_owned),
            team_id.map(str::to_owned),
            usage_endpoint,
        )
    }

    fn codex_test_token(account_id: &str, user_id: &str, revision: &str) -> String {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

        let payload = json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": account_id,
                "chatgpt_user_id": user_id,
            },
            "revision": revision,
        });
        format!(
            "e30.{}.signature",
            URL_SAFE_NO_PAD.encode(payload.to_string())
        )
    }

    #[test]
    fn account_grouping_ignores_codex_token_rotation_and_configuration_directory() {
        let first = test_credential(
            TitleCli::Codex,
            &codex_test_token("account-x", "user-a", "first"),
            Some("account-x"),
            None,
            None,
        );
        let mut second = test_credential(
            TitleCli::Codex,
            &codex_test_token("account-x", "user-a", "second"),
            Some("account-x"),
            None,
            None,
        );
        second.namespace.directory = PathBuf::from("/another/codex-home");
        second.namespace.store = "another-store".into();
        let different = test_credential(
            TitleCli::Codex,
            &codex_test_token("account-y", "user-a", "first"),
            Some("account-y"),
            None,
            None,
        );

        let key = credential_account_key(&first).unwrap();
        assert_eq!(Some(key.clone()), credential_account_key(&second));
        assert_ne!(Some(key), credential_account_key(&different));
        assert_ne!(first.identity, second.identity);
        assert_ne!(first.namespace, second.namespace);
    }

    #[test]
    fn codex_workspace_members_and_unqualified_principals_stay_separate() {
        let first = test_credential(
            TitleCli::Codex,
            &codex_test_token("shared-workspace", "user-a", "first"),
            Some("shared-workspace"),
            None,
            None,
        );
        let another_user = test_credential(
            TitleCli::Codex,
            &codex_test_token("shared-workspace", "user-b", "first"),
            Some("shared-workspace"),
            None,
            None,
        );
        assert_ne!(
            credential_account_key(&first),
            credential_account_key(&another_user)
        );

        for token in [
            "opaque-token-a".to_owned(),
            "opaque-token-b".to_owned(),
            codex_test_token("different-workspace", "user-a", "first"),
            codex_test_token("shared-workspace", "", "first"),
            "e30.invalid.signature".to_owned(),
        ] {
            let unqualified = test_credential(
                TitleCli::Codex,
                &token,
                Some("shared-workspace"),
                None,
                None,
            );
            assert!(codex_account_identity(&unqualified).is_none());
            assert_ne!(
                credential_account_key(&first),
                credential_account_key(&unqualified)
            );
        }
        let missing_user_a = test_credential(
            TitleCli::Codex,
            "opaque-token-a",
            Some("shared-workspace"),
            None,
            None,
        );
        let missing_user_b = test_credential(
            TitleCli::Codex,
            "opaque-token-b",
            Some("shared-workspace"),
            None,
            None,
        );
        assert_ne!(
            credential_account_key(&missing_user_a),
            credential_account_key(&missing_user_b)
        );
    }

    #[test]
    fn codex_profile_quota_groups_separate_workspace_users_and_survive_token_rotation() {
        let first = test_credential(
            TitleCli::Codex,
            &codex_test_token("workspace", "user-a", "first"),
            Some("workspace"),
            None,
            None,
        );
        let rotated = test_credential(
            TitleCli::Codex,
            &codex_test_token("workspace", "user-a", "second"),
            Some("workspace"),
            None,
            None,
        );
        let other_user = test_credential(
            TitleCli::Codex,
            &codex_test_token("workspace", "user-b", "first"),
            Some("workspace"),
            None,
            None,
        );
        assert_eq!(
            codex_profile_quota_group(&first),
            codex_profile_quota_group(&rotated)
        );
        assert_ne!(
            codex_profile_quota_group(&first),
            codex_profile_quota_group(&other_user)
        );
        assert_ne!(first.identity, rotated.identity);
    }

    #[test]
    fn codex_profile_quota_groups_require_qualified_matching_principals() {
        for token in [
            "opaque-token".into(),
            codex_test_token("other-workspace", "user", "first"),
            codex_test_token("workspace", "", "first"),
        ] {
            let credential =
                test_credential(TitleCli::Codex, &token, Some("workspace"), None, None);
            assert_eq!(
                codex_profile_quota_group(&credential),
                Err(ProfileQuotaError::UnknownScope)
            );
        }
    }

    #[test]
    fn codex_account_identity_accepts_qualified_user_and_membership_claims() {
        use base64::{engine::general_purpose::URL_SAFE, Engine};

        for field in ["chatgpt_user_id", "user_id", "chatgpt_account_user_id"] {
            let payload = json!({
                "https://api.openai.com/auth": {
                    "chatgpt_account_id": "workspace",
                    field: "principal",
                },
            });
            let token = format!("e30.{}.signature", URL_SAFE.encode(payload.to_string()));
            let credential =
                test_credential(TitleCli::Codex, &token, Some("workspace"), None, None);
            assert!(codex_account_identity(&credential).is_some(), "{field}");
        }
    }

    #[test]
    fn credential_grouping_keeps_unknown_accounts_providers_and_scopes_separate() {
        for cli in [
            TitleCli::Codex,
            TitleCli::Claude,
            TitleCli::Cursor,
            TitleCli::Kimi,
        ] {
            let first = test_credential(cli, "first-token", None, None, None);
            let mut duplicate = test_credential(cli, "first-token", None, None, None);
            duplicate.namespace.directory = PathBuf::from("/another/account-home");
            let different = test_credential(cli, "second-token", None, None, None);
            let key = credential_account_key(&first).unwrap();
            assert_eq!(Some(key.clone()), credential_account_key(&duplicate));
            assert_ne!(Some(key), credential_account_key(&different));
        }

        let codex = test_credential(TitleCli::Codex, "shared-token", None, None, None);
        let claude = test_credential(TitleCli::Claude, "shared-token", None, None, None);
        assert_ne!(
            credential_account_key(&codex),
            credential_account_key(&claude)
        );
        let team_a = test_credential(TitleCli::Cursor, "shared-token", None, Some("team-a"), None);
        let team_b = test_credential(TitleCli::Cursor, "shared-token", None, Some("team-b"), None);
        assert_ne!(
            credential_account_key(&team_a),
            credential_account_key(&team_b)
        );
        let mainland = test_credential(
            TitleCli::Kimi,
            "shared-token",
            None,
            None,
            Some(KIMI_MAINLAND_USAGE_ENDPOINT),
        );
        let global = test_credential(
            TitleCli::Kimi,
            "shared-token",
            None,
            None,
            Some(KIMI_GLOBAL_USAGE_ENDPOINT),
        );
        assert_ne!(
            credential_account_key(&mainland),
            credential_account_key(&global)
        );
    }

    #[test]
    fn usage_entries_expose_only_opaque_account_keys_and_retain_failed_snapshot_grouping() {
        let token = codex_test_token("private-account-id", "private-user-id", "first");
        let credential = test_credential(
            TitleCli::Codex,
            &token,
            Some("private-account-id"),
            Some("private-team-id"),
            None,
        );
        let key = credential_account_key(&credential).unwrap();
        let mut snapshot = successful_snapshot();
        snapshot.status = UsageStatus::RateLimited;
        let entry = entry_from_snapshot(
            UsageTarget {
                id: "terminal-x".into(),
                process: TitleProcess {
                    cli: TitleCli::Codex,
                    pid: 123,
                },
            },
            snapshot,
            Some(&key),
        );
        let serialized = serde_json::to_value(entry).unwrap();
        assert_eq!(serialized["accountKey"], key);
        assert_eq!(serialized["status"], "rate-limited");
        assert_eq!(serialized["windows"][0]["remainingPercent"], 73.0);
        for private in [
            token.as_str(),
            "private-account-id",
            "private-user-id",
            "private-team-id",
            "/test/account-home",
        ] {
            assert!(!serialized.to_string().contains(private));
        }

        let unknown = empty_entry(
            UsageTarget {
                id: "stopped-terminal".into(),
                process: TitleProcess {
                    cli: TitleCli::Codex,
                    pid: 124,
                },
            },
            UsageStatus::Error,
            "The CLI is no longer running.",
            None,
        );
        assert!(serde_json::to_value(unknown).unwrap()["accountKey"].is_null());
    }

    #[tokio::test]
    async fn bounded_target_mapping_overlaps_work_and_keeps_input_order() {
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let results = tokio::time::timeout(
            Duration::from_secs(1),
            map_bounded_ordered(vec![0, 1], {
                let barrier = barrier.clone();
                let active = active.clone();
                let peak = peak.clone();
                move |index| {
                    let barrier = barrier.clone();
                    let active = active.clone();
                    let peak = peak.clone();
                    async move {
                        let running = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(running, Ordering::SeqCst);
                        barrier.wait().await;
                        active.fetch_sub(1, Ordering::SeqCst);
                        index
                    }
                }
            }),
        )
        .await
        .expect("the two target operations should overlap");
        assert_eq!(results, vec![0, 1]);
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn old_identity_cannot_publish_or_invalidate_after_token_change() {
        let state = CliUsage::default();
        let old = test_key("old-token");
        let new = test_key("new-token");
        let old_generation = activate_identity(&state, &old, 0).await.unwrap();
        assert!(store_snapshot(&state, old.clone(), old_generation, successful_snapshot()).await);
        let observed_generation = current_generation(&state, &old.namespace).await;
        let new_generation = activate_identity(&state, &new, observed_generation)
            .await
            .unwrap();
        assert!(store_snapshot(&state, new.clone(), new_generation, successful_snapshot()).await);

        assert!(!store_snapshot(&state, old.clone(), old_generation, successful_snapshot()).await);
        invalidate_namespace_if_generation(&state, &old.namespace, old_generation).await;

        let cache = state.cache.lock().await;
        assert_eq!(
            cache.active_identity.get(&new.namespace),
            Some(&new.identity)
        );
        assert!(cache.records.contains_key(&new));
        assert!(!cache.records.contains_key(&old));
    }

    #[tokio::test]
    async fn retry_backoff_preserves_last_good_windows_and_timestamp() {
        let state = CliUsage::default();
        let key = test_key("same-account");
        let generation = activate_identity(&state, &key, 0).await.unwrap();
        let previous = successful_snapshot();
        assert!(store_snapshot(&state, key.clone(), generation, previous.clone()).await);

        let failed = failed_snapshot(
            &state,
            &key,
            "rate limited",
            UsageStatus::RateLimited,
            Some(Duration::from_secs(45)),
            generation,
        )
        .await
        .unwrap();
        assert_eq!(failed.status, UsageStatus::RateLimited);
        assert_eq!(failed.windows, previous.windows);
        assert_eq!(failed.updated_at, previous.updated_at);
        assert!(failed.retry_at.unwrap() > now_ms());
        assert!(cached_snapshot(&state, &key, true).await.is_some());
    }

    #[tokio::test]
    async fn successful_cache_ttl_and_force_cooldown_are_enforced() {
        let state = CliUsage::default();
        let key = test_key("same-account");
        let generation = activate_identity(&state, &key, 0).await.unwrap();
        assert!(store_snapshot(&state, key.clone(), generation, successful_snapshot()).await);
        assert!(cached_snapshot(&state, &key, false).await.is_some());
        assert!(mark_request_started(&state, &key, true, generation).await);
        assert!(cached_snapshot(&state, &key, true).await.is_some());

        let mut cache = state.cache.lock().await;
        let record = cache.records.get_mut(&key).unwrap();
        record.last_success = Some(Instant::now() - SUCCESS_TTL - Duration::from_millis(1));
        record.last_forced = None;
        drop(cache);
        assert!(cached_snapshot(&state, &key, false).await.is_none());
    }

    #[test]
    fn codex_usage_maps_windows_and_ignores_invalid_values() {
        let parsed = parse_codex_usage(&json!({
            "rate_limit": {
                "primary_window": {"used_percent": 34.5, "limit_window_seconds": 18000, "reset_at": 1234},
                "secondary_window": {"used_percent": 110, "limit_window_seconds": 604800, "reset_at": 2345}
            },
            "additional_rate_limits": [{
                "limit_name": "code review",
                "rate_limit": {"primary_window": {"used_percent": 75, "window_duration_mins": 15}}
            }]
        }))
        .unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].label, "5-hour");
        assert_eq!(parsed[0].remaining_percent, Some(65.5));
        assert_eq!(parsed[0].resets_at, Some(1_234_000));
        assert_eq!(parsed[1].label, "Weekly");
        assert_eq!(parsed[1].remaining_percent, Some(0.0));
        assert_eq!(parsed[2].label, "code review · 15-minute");
        assert_eq!(parsed[2].remaining_percent, Some(25.0));
        assert_eq!(parsed[2].unit, None);

        for (used_percent, remaining_percent) in [(0, 100.0), (25, 75.0), (100, 0.0)] {
            let parsed = parse_codex_usage(&json!({
                "rate_limit": {
                    "primary_window": {"used_percent": used_percent}
                }
            }))
            .unwrap();
            assert_eq!(parsed[0].remaining_percent, Some(remaining_percent));
        }
    }

    #[test]
    fn claude_usage_maps_known_periods_and_rfc3339_resets() {
        let parsed = parse_claude_usage(&json!({
            "five_hour": {"utilization": 42.25, "resets_at": "2026-09-29T10:00:00+02:00"},
            "seven_day": {"utilization": 15},
            "seven_day_opus": {"utilization": 100},
            "seven_day_sonnet": {"utilization": 0},
            "seven_day_oauth_apps": {"utilization": 25},
            "unknown": {"utilization": 99}
        }))
        .unwrap();
        assert_eq!(parsed.len(), 5);
        assert_eq!(parsed[0].remaining_percent, Some(57.75));
        assert_eq!(parsed[0].resets_at, Some(1_790_668_800_000));
        assert_eq!(parsed[1].label, "Weekly");
        assert_eq!(parsed[2].remaining_percent, Some(0.0));
        assert_eq!(parsed[3].remaining_percent, Some(100.0));
        assert_eq!(parsed[4].remaining_percent, Some(75.0));
    }

    #[test]
    fn cursor_usage_parses_protobuf_json_cycle_end() {
        let parsed = parse_cursor_usage(&json!({
            "billingCycleEnd": "1790678400000",
            "planUsage": {"totalPercentUsed": 48, "includedSpend": 10, "limit": 20}
        }))
        .unwrap();
        assert_eq!(parsed[0].remaining_percent, Some(52.0));
        assert_eq!(parsed[0].resets_at, Some(1_790_678_400_000));
        assert_eq!(parsed[0].used, None);
        for (used_percent, remaining_percent) in [(0, 100.0), (25, 75.0), (100, 0.0)] {
            let parsed = parse_cursor_usage(&json!({
                "planUsage": {"totalPercentUsed": used_percent}
            }))
            .unwrap();
            assert_eq!(parsed[0].remaining_percent, Some(remaining_percent));
        }
    }

    #[test]
    fn kimi_usage_maps_verified_windows_and_reset_times() {
        let parsed = parse_kimi_usage(&json!({
            "usages": {
                "limit_5h": {
                    "used_ratio": 0.25,
                    "reset_time": "2026-09-29T10:00:00+02:00"
                },
                "limit_7d": {"used_ratio": 0.5},
                "limit_month_total": {"used_ratio": 0},
                "limit_month_code": {"used_ratio": "1.2"},
                "unknown": {"used_ratio": 0.99}
            }
        }))
        .unwrap();
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[0].label, "5-hour");
        assert_eq!(parsed[0].remaining_percent, Some(75.0));
        assert_eq!(parsed[0].resets_at, Some(1_790_668_800_000));
        assert_eq!(parsed[1].label, "Weekly");
        assert_eq!(parsed[1].remaining_percent, Some(50.0));
        assert_eq!(parsed[2].label, "Monthly");
        assert_eq!(parsed[2].remaining_percent, Some(100.0));
        assert_eq!(parsed[3].label, "Monthly code");
        assert_eq!(parsed[3].remaining_percent, Some(0.0));
        let exhausted = parse_kimi_usage(&json!({
            "usages": {"limit_5h": {"used_ratio": 1.0}}
        }))
        .unwrap();
        assert_eq!(exhausted[0].remaining_percent, Some(0.0));
    }

    #[test]
    fn kimi_usage_rejects_unknown_malformed_and_nonfinite_buckets() {
        assert!(
            parse_kimi_usage(&json!({"usages": {"future_limit": {"used_ratio": 0.2}}})).is_err()
        );
        assert!(parse_kimi_usage(&json!({"usages": {"limit_5h": {"used_ratio": -0.1}}})).is_err());
        assert!(parse_kimi_usage(&json!({"usages": {"limit_5h": {"used_ratio": "NaN"}}})).is_err());
        assert!(parse_kimi_usage(&json!({"usages": null})).is_err());
    }

    fn kimi_config() -> &'static str {
        r#"
default_model = "kimi-code/k3"

[providers."managed:kimi-code"]
type = "kimi"
base_url = "https://api.kimi.com/coding/v1"
api_key = ""

[providers."managed:kimi-code".oauth]
storage = "file"
key = "oauth/kimi-code"

[models."kimi-code/k3"]
provider = "managed:kimi-code"
model = "k3"
"#
    }

    fn kimi_test_paths(home: &Path) -> UsageProcessPaths {
        UsageProcessPaths {
            home: Some(home.to_path_buf()),
            kimi_code_home: Some(home.to_path_buf()),
            ..UsageProcessPaths::default()
        }
    }

    #[test]
    fn kimi_selection_binds_the_running_managed_provider_and_fixed_region() {
        let temp = tempfile::tempdir().unwrap();
        let paths = kimi_test_paths(temp.path());
        std::fs::write(temp.path().join("config.toml"), kimi_config()).unwrap();
        let selection = resolve_kimi_selection(&paths).unwrap();
        assert_eq!(selection.credential_file, "kimi-code.json");
        assert_eq!(selection.usage_endpoint, KIMI_MAINLAND_USAGE_ENDPOINT);

        let global_key = kimi_scoped_oauth_key(KIMI_GLOBAL_OAUTH_HOST, KIMI_GLOBAL_BASE_URL);
        let global_config = kimi_config()
            .replace(KIMI_MAINLAND_BASE_URL, KIMI_GLOBAL_BASE_URL)
            .replace(
                "key = \"oauth/kimi-code\"",
                &format!("key = \"{global_key}\""),
            )
            .replace(
                "storage = \"file\"",
                &format!("storage = \"file\"\noauth_host = \"{KIMI_GLOBAL_OAUTH_HOST}\""),
            );
        std::fs::write(temp.path().join("config.toml"), global_config).unwrap();
        let configured_global = resolve_kimi_selection(&paths).unwrap();
        assert_eq!(configured_global.usage_endpoint, KIMI_GLOBAL_USAGE_ENDPOINT);
        assert_eq!(
            configured_global.credential_file,
            format!("{}.json", global_key.strip_prefix("oauth/").unwrap())
        );

        let global = UsageProcessPaths {
            kimi_code_base_url: Some(KIMI_GLOBAL_BASE_URL.into()),
            kimi_code_oauth_host: Some(KIMI_GLOBAL_OAUTH_HOST.into()),
            ..paths.clone()
        };
        let selection = resolve_kimi_selection(&global).unwrap();
        assert_eq!(selection.usage_endpoint, KIMI_GLOBAL_USAGE_ENDPOINT);
        assert_eq!(
            selection.credential_file,
            "kimi-code-env-0e4f99c69cc27850.json"
        );
    }

    #[test]
    fn kimi_rejects_custom_provider_and_custom_endpoint() {
        let temp = tempfile::tempdir().unwrap();
        let paths = kimi_test_paths(temp.path());
        let config =
            kimi_config().replace("provider = \"managed:kimi-code\"", "provider = \"custom\"");
        std::fs::write(temp.path().join("config.toml"), config).unwrap();
        assert!(matches!(
            resolve_kimi_selection(&paths),
            Err(AuthReadError::Unsupported(_))
        ));

        std::fs::write(temp.path().join("config.toml"), kimi_config()).unwrap();
        let custom = UsageProcessPaths {
            kimi_code_base_url: Some("https://example.invalid/api".into()),
            ..paths.clone()
        };
        assert!(matches!(
            resolve_kimi_selection(&custom),
            Err(AuthReadError::Unsupported(_))
        ));

        let configured_api_key = format!(
            "{}\n[providers.\"managed:kimi-code\".env]\nKIMI_API_KEY = \"configured-key\"\n",
            kimi_config()
        );
        std::fs::write(temp.path().join("config.toml"), configured_api_key).unwrap();
        assert!(matches!(
            resolve_kimi_selection(&paths),
            Err(AuthReadError::Unsupported(_))
        ));
    }

    #[test]
    fn kimi_legacy_api_env_does_not_override_managed_account_but_model_name_does() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("config.toml"), kimi_config()).unwrap();
        let credentials = temp.path().join("credentials");
        std::fs::create_dir_all(&credentials).unwrap();
        let expiry = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
            + 3600.0;
        std::fs::write(
            credentials.join("kimi-code.json"),
            serde_json::to_vec(&json!({
                "access_token": "fixture-token",
                "expires_at": expiry,
            }))
            .unwrap(),
        )
        .unwrap();

        let home = format!("KIMI_CODE_HOME={}", temp.path().display());
        let plain = UsageProcessPaths::from_entries(&[home.as_bytes()]);
        let legacy_api_environment = UsageProcessPaths::from_entries(&[
            home.as_bytes(),
            b"KIMI_API_KEY=legacy-api-key",
            b"KIMI_BASE_URL=https://legacy.invalid/v1",
            b"KIMI_MODEL_API_KEY=legacy-model-key",
            b"KIMI_MODEL_THINKING_EFFORT=max",
        ]);
        assert!(!legacy_api_environment.has_kimi_model_name_override);
        assert_eq!(
            native_namespace(TitleCli::Kimi, &plain).unwrap(),
            native_namespace(TitleCli::Kimi, &legacy_api_environment).unwrap()
        );

        let result = resolve_native_credentials(
            TitleCli::Kimi,
            legacy_api_environment,
            native_namespace(TitleCli::Kimi, &plain).unwrap(),
        );
        let NativeCredentialResult::Ready(account) = result else {
            panic!("unconsumed process API-key names must not hide managed account usage")
        };
        assert_eq!(account.usage_endpoint, Some(KIMI_MAINLAND_USAGE_ENDPOINT));

        let temporary_model = UsageProcessPaths::from_entries(&[
            home.as_bytes(),
            b"KIMI_MODEL_NAME=temporary-model",
            b"KIMI_MODEL_API_KEY=temporary-model-key",
        ]);
        let result = resolve_native_credentials(
            TitleCli::Kimi,
            temporary_model.clone(),
            native_namespace(TitleCli::Kimi, &temporary_model).unwrap(),
        );
        assert!(matches!(
            result,
            NativeCredentialResult::Status {
                status: UsageStatus::Unsupported,
                ..
            }
        ));
    }

    #[test]
    fn kimi_reads_only_unexpired_existing_file_tokens_without_refreshing() {
        let temp = tempfile::tempdir().unwrap();
        let paths = kimi_test_paths(temp.path());
        let credentials = temp.path().join("credentials");
        std::fs::create_dir_all(&credentials).unwrap();
        let credential = credentials.join("kimi-code.json");
        let expiry = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
            + 3600.0;
        std::fs::write(
            &credential,
            serde_json::to_vec(&json!({"access_token": "fixture-token", "expires_at": expiry}))
                .unwrap(),
        )
        .unwrap();
        let namespace = native_namespace(TitleCli::Kimi, &paths).unwrap();
        let result = resolve_native_credentials(TitleCli::Kimi, paths.clone(), namespace.clone());
        let NativeCredentialResult::Ready(credential_result) = result else {
            panic!("an unexpired Kimi Code file token should be usable")
        };
        assert_eq!(
            credential_result.usage_endpoint,
            Some(KIMI_MAINLAND_USAGE_ENDPOINT)
        );
        assert_ne!(credential_result.identity, "fixture-token");

        std::fs::write(
            temp.path().join("credentials/kimi-code.json"),
            serde_json::to_vec(&json!({"access_token": "expired-token", "expires_at": 1})).unwrap(),
        )
        .unwrap();
        let result = resolve_native_credentials(TitleCli::Kimi, paths.clone(), namespace);
        assert!(matches!(
            result,
            NativeCredentialResult::Status {
                status: UsageStatus::Unauthenticated,
                ..
            }
        ));

        std::fs::write(
            temp.path().join("credentials/kimi-code.json"),
            br#"{"access_token":"token","expires_at":"tomorrow"}"#,
        )
        .unwrap();
        let namespace = native_namespace(TitleCli::Kimi, &paths).unwrap();
        assert!(matches!(
            resolve_native_credentials(TitleCli::Kimi, paths, namespace),
            NativeCredentialResult::Status {
                status: UsageStatus::Error,
                ..
            }
        ));
    }

    #[test]
    fn kimi_empty_process_home_reports_unauthenticated_without_windows() {
        let temp = tempfile::tempdir().unwrap();
        let paths = kimi_test_paths(temp.path());
        let namespace = native_namespace(TitleCli::Kimi, &paths).unwrap();
        let result = resolve_native_credentials(TitleCli::Kimi, paths, namespace);
        assert!(matches!(
            result,
            NativeCredentialResult::Status {
                status: UsageStatus::Unauthenticated,
                message: "Sign in to Kimi Code with an account to see account usage.",
                ..
            }
        ));
    }

    #[test]
    fn kimi_namespace_changes_with_region_config_and_python_kimi_is_unsupported() {
        let temp = tempfile::tempdir().unwrap();
        let paths = kimi_test_paths(temp.path());
        std::fs::write(temp.path().join("config.toml"), kimi_config()).unwrap();
        let mainland = native_namespace(TitleCli::Kimi, &paths).unwrap();
        let global = UsageProcessPaths {
            kimi_code_base_url: Some(KIMI_GLOBAL_BASE_URL.into()),
            kimi_code_oauth_host: Some(KIMI_GLOBAL_OAUTH_HOST.into()),
            ..paths.clone()
        };
        let other_region = native_namespace(TitleCli::Kimi, &global).unwrap();
        assert_ne!(mainland, other_region);
        let legacy = UsageProcessPaths {
            is_legacy_kimi_cli: true,
            ..paths.clone()
        };
        let legacy_namespace = native_namespace(TitleCli::Kimi, &legacy).unwrap();
        assert_ne!(mainland, legacy_namespace);
        assert!(matches!(
            resolve_native_credentials(TitleCli::Kimi, legacy, legacy_namespace),
            NativeCredentialResult::Status {
                status: UsageStatus::Unsupported,
                ..
            }
        ));
    }

    #[test]
    fn usage_parsers_clamp_exhausted_windows_and_reject_invalid_usage() {
        let claude = parse_claude_usage(&json!({"five_hour": {"utilization": 150}})).unwrap();
        assert_eq!(claude[0].remaining_percent, Some(0.0));
        assert!(parse_claude_usage(&json!({"five_hour": {"utilization": -1}})).is_err());
        assert!(parse_cursor_usage(&json!({"planUsage": {"totalPercentUsed": null}})).is_err());
        let cursor = parse_cursor_usage(&json!({"planUsage": {"totalPercentUsed": 123}})).unwrap();
        assert_eq!(cursor[0].remaining_percent, Some(0.0));
        assert!(parse_codex_usage(
            &json!({"rate_limit": {"primary_window": {"used_percent": "NaN"}}})
        )
        .is_err());
    }

    #[test]
    fn rfc3339_parser_normalizes_offsets_and_rejects_invalid_dates() {
        let value = json!({"resets_at": "2026-09-29T10:00:00+02:00"});
        let object = value.as_object().unwrap();
        assert_eq!(
            epoch_millis_any(object, &["resets_at"]),
            Some(1_790_668_800_000)
        );
        let invalid = json!({"resets_at": "not a date"});
        assert_eq!(
            epoch_millis_any(invalid.as_object().unwrap(), &["resets_at"]),
            None
        );
        for invalid in [json!(-1), json!(i64::MIN), json!(i64::MAX)] {
            assert_eq!(
                epoch_millis_any(json!({"reset": invalid}).as_object().unwrap(), &["reset"]),
                None
            );
        }
    }

    #[test]
    fn retry_after_accepts_seconds_and_http_dates() {
        let seconds = reqwest::header::HeaderValue::from_static("12");
        assert_eq!(
            parse_retry_after(Some(&seconds)),
            Some(Duration::from_secs(12))
        );
        let date = reqwest::header::HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT");
        assert!(parse_retry_after(Some(&date)).is_some());
    }

    #[test]
    fn codex_auth_parser_uses_only_chatgpt_oauth_fields() {
        let auth = serde_json::from_value::<CodexAuth>(json!({
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": null,
            "tokens": {"access_token": "token-value", "account_id": "acct"}
        }))
        .unwrap();
        assert_eq!(
            auth.tokens.unwrap().access_token.as_deref(),
            Some("token-value")
        );
    }

    #[test]
    fn response_body_and_credential_limits_are_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("large.json");
        std::fs::write(&path, vec![b'x'; MAX_CREDENTIAL_BYTES + 1]).unwrap();
        assert!(matches!(
            read_optional_bytes(&path, MAX_CREDENTIAL_BYTES),
            Err(AuthReadError::Error(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn credential_reader_rejects_fifo_without_blocking() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("credentials.fifo");
        let path_c = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path_c.as_ptr(), 0o600) }, 0);
        assert!(matches!(
            read_optional_bytes(&path, MAX_CREDENTIAL_BYTES),
            Err(AuthReadError::Error(_))
        ));
    }

    #[test]
    fn codex_storage_modes_fail_closed_for_ephemeral_and_unknown_modes() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.toml");
        std::fs::write(&config, "cli_auth_credentials_store = 'ephemeral'").unwrap();
        assert!(matches!(
            codex_storage_mode(&config),
            Ok(CodexStorageMode::Ephemeral)
        ));
        std::fs::write(&config, "cli_auth_credentials_store = 'secret'").unwrap();
        assert!(matches!(
            codex_storage_mode(&config),
            Err(AuthReadError::Unsupported(_))
        ));
    }

    #[test]
    fn codex_missing_storage_setting_defaults_to_file() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("absent.toml");
        assert!(matches!(
            codex_storage_mode(&config),
            Ok(CodexStorageMode::File)
        ));
        std::fs::write(&config, "model = 'gpt-5'").unwrap();
        assert!(matches!(
            codex_storage_mode(&config),
            Ok(CodexStorageMode::File)
        ));
    }

    #[test]
    fn codex_custom_chatgpt_base_url_disables_account_usage() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.toml");
        std::fs::write(&config, "chatgpt_base_url = 'https://example.test'").unwrap();
        assert!(matches!(
            reject_codex_custom_provider(&config),
            Err(AuthReadError::Unsupported(_))
        ));
    }

    #[test]
    fn cursor_team_id_is_read_from_numeric_cli_config() {
        let temp = tempfile::tempdir().unwrap();
        let paths = UsageProcessPaths {
            home: Some(temp.path().to_path_buf()),
            cursor_config_dir: Some(temp.path().to_path_buf()),
            ..UsageProcessPaths::default()
        };
        let config = temp.path().join("cli-config.json");
        std::fs::write(&config, r#"{"authInfo":{"activeTeamId":42}}"#).unwrap();
        assert_eq!(
            read_cursor_active_team(&paths).unwrap().as_deref(),
            Some("42")
        );
        std::fs::write(&config, r#"{"authInfo":{"activeTeamId":0}}"#).unwrap();
        assert_eq!(read_cursor_active_team(&paths).unwrap(), None);
        std::fs::write(&config, r#"{"authInfo":{"activeTeamId":9007199254740992}}"#).unwrap();
        assert_eq!(read_cursor_active_team(&paths).unwrap(), None);
    }

    #[test]
    fn cache_identity_includes_token_and_directory_is_process_local() {
        let one = make_credential(
            TitleCli::Codex,
            UsageNamespace {
                cli: TitleCli::Codex,
                directory: PathBuf::from("/first/.codex"),
                store: "codex-native".into(),
            },
            "first-token".into(),
            None,
            None,
            None,
        );
        let changed_token = make_credential(
            TitleCli::Codex,
            one.namespace.clone(),
            "second-token".into(),
            None,
            None,
            None,
        );
        let changed_directory = make_credential(
            TitleCli::Codex,
            UsageNamespace {
                cli: TitleCli::Codex,
                directory: PathBuf::from("/second/.codex"),
                store: "codex-native".into(),
            },
            "first-token".into(),
            None,
            None,
            None,
        );
        assert_ne!(one.identity, changed_token.identity);
        assert_ne!(one.namespace, changed_directory.namespace);
    }
}
