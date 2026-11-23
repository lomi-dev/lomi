use super::{
    attachments::{self, Attachment, Kind},
    cli, emit_changed, http,
    settings::{self, *},
    store::{self, *},
    AiChat, Inner,
};
use std::{sync::Arc, time::Instant};
use tauri::{ipc::Channel, AppHandle, State, Window};
use tokio::{io::AsyncReadExt, sync::Notify};
use zeroize::Zeroizing;

const FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(750);
const FLUSH_BYTES: usize = 16 * 1024;
/// Stored assistant output never exceeds what a later request could carry.
const CONTENT_CAP: usize = store::MAX_CONTEXT;
const NOTE_LIMIT: &str = "Response hit the output limit.";
const NOTE_SIZE: &str = "Response exceeded the size limit and was truncated.";

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|_| "The chat worker failed.".to_string())?
}

// ---- key store -------------------------------------------------------

fn key_entry(app: &AppHandle, instance_id: &str) -> Result<keyring::Entry, String> {
    crate::credential_store::entry(
        &format!("{}.providers", app.config().identifier),
        instance_id,
    )
    .map_err(|_| "The system key store is unavailable.".to_string())
}

fn read_key(app: &AppHandle, instance_id: &str) -> Result<Option<Zeroizing<String>>, String> {
    match key_entry(app, instance_id)?.get_password() {
        Ok(key) => Ok(Some(Zeroizing::new(key))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => {
            Err("Cannot read the API key from the system key store. Re-enter the API key.".into())
        }
    }
}

fn delete_key(app: &AppHandle, instance_id: &str) -> Result<(), String> {
    match key_entry(app, instance_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err("Cannot remove the API key from the system key store.".into()),
    }
}

/// Whether the stored key may be sent for a request targeting `base_url`/`kind`.
/// A webview-supplied target that differs from the stored provider needs its own key.
fn stored_key_applies(p: &ApiProvider, base_url: &str, kind: ApiKind) -> bool {
    p.api_key_set
        && p.kind == kind
        && matches!(
            (reqwest::Url::parse(&p.base_url), reqwest::Url::parse(base_url)),
            (Ok(a), Ok(b)) if a == b
        )
}

fn same_origin(a: &str, b: &str) -> bool {
    let origin = |u: &str| {
        let u = reqwest::Url::parse(u).ok()?;
        Some((
            u.scheme().to_string(),
            u.host_str()?.to_ascii_lowercase(),
            u.port_or_known_default(),
        ))
    };
    origin(a).is_some() && origin(a) == origin(b)
}

// ---- settings --------------------------------------------------------

/// Drops defaults that no longer resolve.
fn prune(s: &mut ChatSettings) {
    if let Some(id) = &s.defaults.instance_id {
        if !s.has_instance(id) {
            s.defaults.instance_id = None;
            s.defaults.model = None;
            s.defaults.effort = None;
        }
    }
    if let Some(f) = &s.defaults.fallback {
        if !s.has_instance(&f.instance_id) {
            s.defaults.fallback = None;
        }
    }
}

fn settings_changed(app: &AppHandle, s: &ChatSettings) -> ChatSettings {
    emit_changed(app, "settings", s.revision, None);
    s.clone()
}

#[tauri::command]
pub(crate) async fn ai_chat_settings(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
) -> Result<ChatSettings, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || chat.with(&app, |i| Ok(i.settings.clone()))).await
}

#[tauri::command]
pub(crate) async fn ai_provider_create(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    request: ProviderCreate,
) -> Result<ChatSettings, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || {
        let out = chat.with(&app, |i| {
            let Inner { settings, dir, .. } = i;
            settings.mutate(dir, Some(request.expected_revision), |s| {
                let ProviderInput::Api {
                    instance_id,
                    kind,
                    label,
                    accent_color,
                    base_url,
                    headers,
                    models,
                } = request.provider;
                s.api_providers.push(ApiProvider {
                    instance_id,
                    kind,
                    label,
                    accent_color,
                    enabled: true,
                    base_url,
                    headers,
                    models,
                    api_key_set: false,
                });
                prune(s);
                Ok(())
            })?;
            Ok(settings.clone())
        })?;
        Ok(settings_changed(&app, &out))
    })
    .await
}

#[tauri::command]
pub(crate) async fn ai_provider_update(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    request: ProviderUpdate,
) -> Result<ChatSettings, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || {
        let mut origin_changed = false;
        let out = chat.with(&app, |i| {
            let Inner { settings, dir, .. } = i;
            settings.mutate(dir, Some(request.expected_revision), |s| {
                let id = &request.instance_id;
                match request.patch {
                    ProviderPatch::Api {
                        label,
                        accent_color,
                        enabled,
                        base_url,
                        headers,
                        models,
                    } => {
                        let p = s
                            .api_providers
                            .iter_mut()
                            .find(|p| &p.instance_id == id)
                            .ok_or("Unknown API provider.")?;
                        p.label = label;
                        p.accent_color = accent_color;
                        p.enabled = enabled;
                        let base_url = normalize_base_url(&base_url)?;
                        // A key must never follow a provider to another host.
                        if !same_origin(&p.base_url, &base_url) {
                            origin_changed = true;
                            p.api_key_set = false;
                        }
                        p.base_url = base_url;
                        p.headers = headers;
                        p.models = models;
                    }
                    ProviderPatch::Cli {
                        label,
                        enabled,
                        models,
                    } => {
                        let p = s
                            .cli_providers
                            .iter_mut()
                            .find(|p| &p.instance_id == id)
                            .ok_or("Unknown CLI provider.")?;
                        p.label = label;
                        p.enabled = enabled;
                        if let Some(models) = models {
                            p.models = normalize_models(&models)?;
                        }
                    }
                }
                prune(s);
                Ok(())
            })?;
            Ok(settings.clone())
        })?;
        let out = settings_changed(&app, &out);
        if origin_changed {
            delete_key(&app, &request.instance_id)?;
        }
        Ok(out)
    })
    .await
}

#[tauri::command]
pub(crate) async fn ai_provider_remove(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    request: ProviderRemove,
) -> Result<ChatSettings, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || {
        let id = request.instance_id.clone();
        let (out, is_api) = chat.with(&app, |i| {
            let Inner { settings, dir, .. } = i;
            if settings.revision != request.expected_revision {
                return Err("Chat settings changed in another view. Reload and try again.".into());
            }
            let is_api = settings.api_providers.iter().any(|p| p.instance_id == id);
            if !is_api && !settings.cli_providers.iter().any(|p| p.instance_id == id) {
                return Err("Unknown provider.".into());
            }
            settings.mutate(dir, Some(request.expected_revision), |s| {
                s.api_providers.retain(|p| p.instance_id != id);
                s.cli_providers.retain(|p| p.instance_id != id);
                // Detection must not bring a removed CLI back.
                if !is_api && !s.auto_detected.contains(&id) {
                    s.auto_detected.push(id.clone());
                }
                prune(s);
                Ok(())
            })?;
            Ok((settings.clone(), is_api))
        })?;
        // Settings first: a failed key delete then leaves an unreachable key, not a dangling provider.
        let out = settings_changed(&app, &out);
        if is_api {
            delete_key(&app, &id)?;
        }
        Ok(out)
    })
    .await
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DetectResult {
    settings: ChatSettings,
    added: Vec<String>,
}

/// Installed CLIs that have no provider yet and were never auto-added or dismissed.
fn detect_plan(installed: &[String], existing: &[String], auto: &[String]) -> Vec<String> {
    installed
        .iter()
        .filter(|id| !existing.contains(id) && !auto.contains(id))
        .cloned()
        .collect()
}

/// Binary lookup only; never spawns.
fn installed_clis() -> Vec<String> {
    cli::CLIS
        .into_iter()
        .filter(|id| crate::cli_bin::resolve(id).is_some())
        .map(String::from)
        .collect()
}

#[tauri::command]
pub(crate) async fn ai_providers_detect(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
) -> Result<DetectResult, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || {
        let installed = installed_clis();
        // Planned and committed under one storage lock, so concurrent scans cannot double-add.
        let (settings, added) = chat.with(&app, |i| {
            let existing: Vec<String> = i
                .settings
                .cli_providers
                .iter()
                .map(|c| c.instance_id.clone())
                .collect();
            let added = detect_plan(&installed, &existing, &i.settings.auto_detected);
            if added.is_empty() {
                return Ok((i.settings.clone(), added));
            }
            let Inner { settings, dir, .. } = i;
            settings.mutate(dir, None, |s| {
                for id in &added {
                    s.cli_providers.push(CliProvider {
                        instance_id: id.clone(),
                        cli: id.clone(),
                        label: cli::display_name(id).into(),
                        enabled: true,
                        models: vec![],
                    });
                    s.auto_detected.push(id.clone());
                }
                Ok(())
            })?;
            Ok((settings.clone(), added))
        })?;
        let settings = if added.is_empty() {
            settings
        } else {
            settings_changed(&app, &settings)
        };
        Ok(DetectResult { settings, added })
    })
    .await
}

fn supported_cli(id: &str) -> Result<(), String> {
    if cli::supported(id) {
        Ok(())
    } else {
        Err("Unsupported CLI.".into())
    }
}

#[tauri::command]
pub(crate) async fn ai_cli_status(window: Window, cli: String) -> Result<cli::CliStatus, String> {
    crate::files::main_window(&window)?;
    supported_cli(&cli)?;
    Ok(cli::status(&cli).await)
}

#[tauri::command]
pub(crate) async fn ai_cli_models(
    window: Window,
    cli: String,
) -> Result<Vec<cli::CliModel>, String> {
    crate::files::main_window(&window)?;
    supported_cli(&cli)?;
    Ok(cli::models(&cli, crate::cli_usage::claude_access_token).await)
}

#[tauri::command]
pub(crate) async fn ai_provider_set_key(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    request: ProviderKeySet,
) -> Result<ChatSettings, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || {
        let key = request
            .api_key
            .as_ref()
            .map(|k| {
                let trimmed = k.trim();
                settings::valid_key(trimmed).map(|()| Zeroizing::new(trimmed.to_string()))
            })
            .transpose()?;
        let id = request.instance_id.clone();
        let exists = |chat: &AiChat| {
            chat.with(&app, |i| {
                i.settings
                    .api_providers
                    .iter()
                    .any(|p| p.instance_id == id)
                    .then_some(())
                    .ok_or_else(|| "Unknown API provider.".to_string())
            })
        };
        exists(&chat)?;
        // Keyring I/O runs without the storage lock held.
        match &key {
            Some(key) => key_entry(&app, &id)?
                .set_password(key)
                .map_err(|_| "Cannot store the API key in the system key store.".to_string())?,
            None => delete_key(&app, &id)?,
        }
        let set = key.is_some();
        let committed = chat.with(&app, |i| {
            let Inner { settings, dir, .. } = i;
            if !settings.api_providers.iter().any(|p| p.instance_id == id) {
                return Err("Unknown API provider.".into());
            }
            settings.mutate(dir, None, |s| {
                if let Some(p) = s.api_providers.iter_mut().find(|p| p.instance_id == id) {
                    p.api_key_set = set;
                }
                Ok(())
            })?;
            Ok(settings.clone())
        });
        match committed {
            Ok(out) => Ok(settings_changed(&app, &out)),
            Err(e) => {
                if set {
                    let _ = delete_key(&app, &id);
                }
                Err(e)
            }
        }
    })
    .await
}

#[tauri::command]
pub(crate) async fn ai_provider_probe(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    request: ProviderProbe,
) -> Result<ProbeResult, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    let (base_url, kind, headers, key) = {
        let chat = chat.clone();
        blocking(move || {
            let base_url = normalize_base_url(&request.base_url)?;
            validate_headers(&request.headers)?;
            let mut key = match &request.api_key {
                Some(k) => {
                    let trimmed = k.trim();
                    settings::valid_key(trimmed)?;
                    Some(Zeroizing::new(trimmed.to_string()))
                }
                None => None,
            };
            if key.is_none() {
                if let Some(id) = &request.instance_id {
                    let applies = chat.with(&app, |i| {
                        let p = i
                            .settings
                            .api_providers
                            .iter()
                            .find(|p| &p.instance_id == id)
                            .ok_or_else(|| "Unknown API provider.".to_string())?;
                        Ok(stored_key_applies(p, &base_url, request.kind))
                    })?;
                    // A changed target never receives the stored key.
                    if applies {
                        key = read_key(&app, id)?;
                    }
                }
            }
            if key.is_none() && request.kind != ApiKind::OpenaiCompatible {
                return Err("Enter an API key first (the stored key only applies to the saved URL and provider type).".into());
            }
            Ok((base_url, request.kind, request.headers.clone(), key))
        })
        .await?
    };
    let models = http::probe(&chat.client()?, kind, &base_url, &headers, key.as_ref()).await?;
    Ok(ProbeResult { models })
}

#[tauri::command]
pub(crate) async fn ai_chat_preferences_update(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    request: PreferencesUpdate,
) -> Result<ChatSettings, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || {
        let out = chat.with(&app, |i| {
            let Inner { settings, dir, .. } = i;
            settings.mutate(dir, Some(request.expected_revision), |s| {
                prune(s);
                s.validate_defaults(&request.defaults)?;
                s.defaults = request.defaults;
                s.behavior = request.behavior;
                Ok(())
            })?;
            Ok(settings.clone())
        })?;
        Ok(settings_changed(&app, &out))
    })
    .await
}

// ---- threads ---------------------------------------------------------

fn valid_operation(op: &str) -> Result<(), String> {
    if op.len() > 120 {
        return Err("Invalid operation identity.".into());
    }
    if op.is_empty()
        || !op
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || matches!(v, b'-' | b'_' | b':'))
    {
        return Err("Invalid operation identity.".into());
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn ai_threads_list(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    project: String,
    workspace_id: String,
) -> Result<Vec<ThreadSummary>, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || chat.with(&app, |i| i.store.list(&project, &workspace_id))).await
}

#[tauri::command]
pub(crate) async fn ai_thread_snapshot(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    thread_id: String,
) -> Result<ThreadSnapshot, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || chat.with(&app, |i| i.store.snapshot(&thread_id))).await
}

#[tauri::command]
pub(crate) async fn ai_thread_create(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    request: ThreadCreate,
) -> Result<ThreadSummary, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || {
        valid_operation(&request.operation_id)?;
        let title = store::valid_title(&request.title)?;
        settings::valid_model_id(&request.model)?;
        request
            .effort
            .as_deref()
            .map_or(Ok(()), settings::valid_effort)?;
        if request.project.is_empty()
            || request.project.len() > 4096
            || request.project.contains('\0')
        {
            return Err("Invalid project path.".into());
        }
        if request
            .workspace_id
            .as_deref()
            .is_some_and(|w| w.is_empty() || w.len() > 128 || w.chars().any(char::is_control))
        {
            return Err("Invalid workspace identity.".into());
        }
        if request.cwd.as_ref().is_some_and(|c| {
            c.len() > 4096 || c.contains('\0') || !std::path::Path::new(c).is_absolute()
        }) {
            return Err("Invalid working directory.".into());
        }
        // One lock for replay, provider lookup and insert.
        let thread = chat.with(&app, |i| {
            if let Some(old) = i.store.create_replay(&request)? {
                return Ok(old);
            }
            let s = &i.settings;
            let kind = if s
                .api_providers
                .iter()
                .any(|p| p.instance_id == request.instance_id)
            {
                ThreadKind::Api
            } else if s
                .cli_providers
                .iter()
                .any(|p| p.instance_id == request.instance_id)
            {
                cli::valid_model(&request.model)?;
                ThreadKind::Cli
            } else {
                return Err("Unknown provider.".into());
            };
            // Permissions and workspace are fixed from the current defaults.
            let mut new = request.clone();
            new.permissions = s.defaults.permissions;
            new.workspace = s.defaults.workspace;
            i.store.create(&new, &title, kind)
        })?;
        emit_changed(&app, "threads", thread.revision, Some(&thread.thread_id));
        Ok(thread)
    })
    .await
}

#[tauri::command]
pub(crate) async fn ai_thread_update(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    request: ThreadUpdate,
) -> Result<ThreadSummary, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || {
        let thread = chat.with(&app, |i| {
            let Inner {
                settings, store, ..
            } = i;
            store.update(&request, |t| {
                if let Some(model) = &request.model {
                    settings::valid_model_id(model)?;
                }
                let Some(id) = &request.instance_id else {
                    return Ok(());
                };
                let api = settings.api_providers.iter().find(|p| &p.instance_id == id);
                match (t.kind, api) {
                    (ThreadKind::Api, Some(p)) if p.enabled => Ok(()),
                    // A native CLI session cannot move to another provider.
                    (ThreadKind::Cli, None) if *id == t.instance_id => Ok(()),
                    _ => Err("That provider cannot be used by this thread.".into()),
                }
            })
        })?;
        emit_changed(&app, "threads", thread.revision, Some(&thread.thread_id));
        Ok(thread)
    })
    .await
}

#[tauri::command]
pub(crate) async fn ai_thread_delete(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    request: ThreadDelete,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || {
        chat.with(&app, |i| {
            // Without a running stream, leftover in-flight rows (e.g. after a panic) must not block deletion.
            if !chat.active()?.contains_key(&request.thread_id) {
                i.store.interrupt_inflight(&request.thread_id, false)?;
            }
            i.store.delete(&request)
        })?;
        emit_changed(&app, "threads", 0, Some(&request.thread_id));
        Ok(())
    })
    .await
}

#[tauri::command]
pub(crate) async fn ai_thread_stop(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    thread_id: String,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || {
        let changed = chat.with(&app, |i| {
            if let Some(token) = chat.active()?.get(&thread_id) {
                token.notify_one();
                return Ok(false);
            }
            i.store.interrupt_inflight(&thread_id, true)
        })?;
        if changed {
            emit_changed(&app, "threads", 0, Some(&thread_id));
        }
        Ok(())
    })
    .await
}

/// Registers a running stream's stop token; dropping it (also on panic) removes the entry.
struct ActiveGuard {
    chat: AiChat,
    thread_id: String,
    stop: Arc<Notify>,
}

impl ActiveGuard {
    fn new(chat: &AiChat, thread_id: String) -> Result<Self, String> {
        let stop = Arc::new(Notify::new());
        chat.active()?.insert(thread_id.clone(), stop.clone());
        Ok(Self {
            chat: chat.clone(),
            thread_id,
            stop,
        })
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.chat.active() {
            // A newer stream of the same thread may have replaced the entry.
            if active
                .get(&self.thread_id)
                .is_some_and(|s| Arc::ptr_eq(s, &self.stop))
            {
                active.remove(&self.thread_id);
            }
        }
    }
}

struct ApiJob {
    provider: ApiProvider,
    max_tokens: Option<u32>,
    context: Vec<Turn>,
    attachments_dir: std::path::PathBuf,
}

struct CliJob {
    cli: String,
    spec: cli::Spec,
    /// Session the CLI was resumed from, if any.
    session: Option<String>,
}

enum Target {
    Api(ApiJob),
    Cli(CliJob),
}

struct Job {
    thread_id: String,
    message_id: String,
    model: String,
    system: String,
    target: Target,
    guard: ActiveGuard,
}

/// Provider state a send needs, captured under the storage lock.
enum Provider {
    Api(ApiProvider),
    Cli(String, std::path::PathBuf),
}

#[tauri::command]
pub(crate) async fn ai_thread_send(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    request: ThreadSend,
    on_event: Channel<ThreadStreamEvent>,
) -> Result<ThreadSendAccepted, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    let accepted = {
        let (chat, app, request) = (chat.clone(), app.clone(), request.clone());
        blocking(move || {
            valid_operation(&request.operation_id)?;
            settings::valid_model_id(&request.model)?;
            let started = chat.with(&app, |i| {
                let Inner {
                    settings,
                    store,
                    dir,
                    ..
                } = i;
                let mut provider = None;
                let begin = store.send_begin(
                    &request,
                    settings.behavior.system_instructions.len(),
                    |t| {
                        provider = Some(match t.kind {
                            ThreadKind::Api => {
                                let p = settings
                                    .api_providers
                                    .iter()
                                    .find(|p| p.instance_id == request.instance_id)
                                    .ok_or("Unknown API provider.")?;
                                if !p.enabled {
                                    return Err("This provider is disabled.".into());
                                }
                                if !p.api_key_set && p.kind != ApiKind::OpenaiCompatible {
                                    return Err("This provider has no API key.".into());
                                }
                                Provider::Api(p.clone())
                            }
                            ThreadKind::Cli => {
                                let p = settings
                                    .cli_providers
                                    .iter()
                                    .find(|p| p.instance_id == request.instance_id)
                                    .filter(|p| p.instance_id == t.instance_id)
                                    .ok_or("Unknown CLI provider.")?;
                                if !p.enabled {
                                    return Err("This provider is disabled.".into());
                                }
                                cli::valid_model(&request.model)?;
                                cli::check_prompt(&p.cli, &request.text)?;
                                if t.cwd
                                    .as_deref()
                                    .is_some_and(|c| !std::path::Path::new(c).is_dir())
                                {
                                    return Err("The project folder no longer exists.".into());
                                }
                                let bin = crate::cli_bin::resolve(&p.cli).ok_or(
                                    "This CLI is not installed. Install it or remove the provider.",
                                )?;
                                Provider::Cli(p.cli.clone(), bin)
                            }
                        });
                        Ok(())
                    },
                )?;
                Ok(match begin {
                    SendBegin::Replay(a) => (a, None),
                    SendBegin::New(a, context) => {
                        let target = match provider.expect("checked during begin") {
                            Provider::Api(provider) => Target::Api(ApiJob {
                                provider,
                                max_tokens: settings.behavior.max_output_tokens,
                                context,
                                attachments_dir: store.attachments_dir(),
                            }),
                            Provider::Cli(id, bin) => {
                                let session = store
                                    .native_session(&a.thread.thread_id)?
                                    .filter(|s| cli::valid_session(s));
                                let agent_file = if id == "kimi"
                                    && session.is_none()
                                    && a.thread.permissions == Permissions::ReadOnly
                                {
                                    let path = dir.join("kimi-chat-agent.md");
                                    crate::storage_guard::atomic(
                                        &path,
                                        cli::KIMI_AGENT.as_bytes(),
                                    )?;
                                    Some(path.to_string_lossy().into_owned())
                                } else {
                                    None
                                };
                                Target::Cli(cli_job(
                                    &id,
                                    bin,
                                    &a,
                                    &store.attachments_dir(),
                                    session,
                                    &settings.behavior.system_instructions,
                                    agent_file.as_deref(),
                                ))
                            }
                        };
                        // Registered under the storage lock so stop/delete never see a started send as stuck.
                        let guard = ActiveGuard::new(&chat, a.thread.thread_id.clone())?;
                        let job = Job {
                            thread_id: a.thread.thread_id.clone(),
                            message_id: a.assistant.message_id.clone(),
                            model: request.model.clone(),
                            system: settings.behavior.system_instructions.clone(),
                            target,
                            guard,
                        };
                        (a, Some(job))
                    }
                })
            })?;
            Ok(started)
        })
        .await?
    };
    let (accepted, job) = accepted;
    if let Some(job) = job {
        emit_changed(
            &app,
            "threads",
            accepted.thread.revision,
            Some(&accepted.thread.thread_id),
        );
        tauri::async_runtime::spawn(run_stream(chat, app, job, on_event));
    }
    Ok(accepted)
}

/// Builds the argv and working directory of a CLI send; a thread without a directory uses home.
fn cli_job(
    cli_id: &str,
    bin: std::path::PathBuf,
    accepted: &ThreadSendAccepted,
    files_dir: &std::path::Path,
    session: Option<String>,
    system: &str,
    agent_file: Option<&str>,
) -> CliJob {
    let (thread, files) = (&accepted.thread, &accepted.user.attachments);
    let cwd = thread
        .cwd
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(crate::shell::home);
    let path = |a: &Attachment| {
        attachments::path(files_dir, a)
            .to_string_lossy()
            .into_owned()
    };
    let listed: Vec<(String, String)> = files.iter().map(|a| (path(a), a.name.clone())).collect();
    let images: Vec<String> = files
        .iter()
        .filter(|a| a.kind == Kind::Image)
        .map(path)
        .collect();
    let prompt = cli::attach_prompt(&accepted.user.content, &listed);
    let attach_dir = files_dir.to_string_lossy();
    let (args, stdin) = cli::argv(&cli::Invocation {
        cli: cli_id,
        model: &accepted.user.model,
        cwd: &cwd.to_string_lossy(),
        session: session.as_deref(),
        system,
        prompt: &prompt,
        agent_file,
        permissions: thread.permissions,
        effort: thread.effort.as_deref(),
        attach_dir: (!files.is_empty()).then_some(&*attach_dir),
        images: &images,
    });
    CliJob {
        cli: cli_id.into(),
        spec: cli::Spec {
            bin,
            args,
            stdin,
            cwd,
        },
        session,
    }
}

#[derive(Clone)]
struct Outcome {
    state: MessageState,
    content: String,
    stream_seq: u64,
    stop_reason: Option<String>,
    error: Option<String>,
}

impl Outcome {
    fn failed(message: String, content: String, stream_seq: u64) -> Self {
        Self {
            state: MessageState::Failed,
            content,
            stream_seq,
            stop_reason: None,
            error: Some(message),
        }
    }
}

/// Visible note for a response the provider cut off at its output limit.
fn truncation_note(reason: Option<&str>) -> Option<&'static str> {
    matches!(reason, Some("length" | "max_tokens")).then_some(NOTE_LIMIT)
}

/// Retries the final commit, then falls back to a minimal state update so the
/// row can never stay in flight.
async fn commit_outcome(
    chat: &AiChat,
    app: &AppHandle,
    job: &Job,
    outcome: &Outcome,
) -> Result<(ThreadSummary, ThreadMessage), String> {
    for attempt in 0..3u64 {
        let (chat, app, o) = (chat.clone(), app.clone(), outcome.clone());
        let (thread_id, message_id) = (job.thread_id.clone(), job.message_id.clone());
        let result = blocking(move || {
            chat.with(&app, |i| {
                i.store.finish(
                    &thread_id,
                    &message_id,
                    &Finish {
                        state: o.state,
                        content: &o.content,
                        stream_seq: o.stream_seq,
                        stop_reason: o.stop_reason.as_deref(),
                        error: o.error.as_deref(),
                    },
                )
            })
        })
        .await;
        if let Ok(done) = result {
            return Ok(done);
        }
        tokio::time::sleep(std::time::Duration::from_millis(200 * (attempt + 1))).await;
    }
    let (chat, app) = (chat.clone(), app.clone());
    let (thread_id, message_id) = (job.thread_id.clone(), job.message_id.clone());
    blocking(move || chat.with(&app, |i| i.store.finish_minimal(&thread_id, &message_id))).await
}

async fn run_stream(chat: AiChat, app: AppHandle, job: Job, channel: Channel<ThreadStreamEvent>) {
    let stop = job.guard.stop.clone();
    let outcome = stream(&chat, &app, &job, &channel, &stop).await;
    let finished = commit_outcome(&chat, &app, &job, &outcome).await;
    // The entry is removed on every path, including panics, when `job` drops.
    drop(job.guard);
    if let Ok((thread, message)) = finished {
        let _ = channel.send(ThreadStreamEvent::Final {
            thread: thread.clone(),
            message,
        });
        emit_changed(&app, "threads", thread.revision, Some(&thread.thread_id));
    }
}

fn stopped_outcome(content: String, seq: u64) -> Outcome {
    Outcome {
        state: MessageState::Stopped,
        content,
        stream_seq: seq,
        stop_reason: Some("stopped".into()),
        error: None,
    }
}

/// Streamed assistant text: delta events to the webview plus periodic persistence.
struct Acc {
    content: String,
    seq: u64,
    pending: usize,
    last_flush: Instant,
}

impl Acc {
    fn new() -> Self {
        Self {
            content: String::new(),
            seq: 0,
            pending: 0,
            last_flush: Instant::now(),
        }
    }

    /// Appends and emits `text`; false when it would exceed the stored-content cap.
    fn push(&mut self, job: &Job, channel: &Channel<ThreadStreamEvent>, text: String) -> bool {
        if self.content.len() + text.len() > CONTENT_CAP {
            return false;
        }
        self.content.push_str(&text);
        self.seq += 1;
        self.pending += text.len();
        let _ = channel.send(ThreadStreamEvent::Delta {
            message_id: job.message_id.clone(),
            seq: self.seq,
            text,
        });
        true
    }

    async fn flush_due(&mut self, chat: &AiChat, app: &AppHandle, job: &Job) {
        if self.pending < FLUSH_BYTES && self.last_flush.elapsed() < FLUSH_INTERVAL {
            return;
        }
        let (chat, app, id, text, seq) = (
            chat.clone(),
            app.clone(),
            job.message_id.clone(),
            self.content.clone(),
            self.seq,
        );
        let _ = blocking(move || chat.with(&app, |i| i.store.flush(&id, &text, seq))).await;
        self.pending = 0;
        self.last_flush = Instant::now();
    }

    fn oversized(self) -> Outcome {
        Outcome {
            state: MessageState::Completed,
            content: self.content,
            stream_seq: self.seq,
            stop_reason: Some("size_limit".into()),
            error: Some(NOTE_SIZE.into()),
        }
    }
}

async fn mark_streaming(chat: &AiChat, app: &AppHandle, job: &Job) -> Result<(), String> {
    let (chat, app, id) = (chat.clone(), app.clone(), job.message_id.clone());
    blocking(move || chat.with(&app, |i| i.store.mark_streaming(&id))).await
}

async fn stream(
    chat: &AiChat,
    app: &AppHandle,
    job: &Job,
    channel: &Channel<ThreadStreamEvent>,
    stop: &Notify,
) -> Outcome {
    match &job.target {
        Target::Api(api) => stream_api(chat, app, job, api, channel, stop).await,
        Target::Cli(cli) => stream_cli(chat, app, job, cli, channel, stop).await,
    }
}

async fn stream_api(
    chat: &AiChat,
    app: &AppHandle,
    job: &Job,
    api: &ApiJob,
    channel: &Channel<ThreadStreamEvent>,
    stop: &Notify,
) -> Outcome {
    let stopped = stop.notified();
    tokio::pin!(stopped);

    let key = if api.provider.api_key_set {
        let (app, id) = (app.clone(), api.provider.instance_id.clone());
        match blocking(move || read_key(&app, &id)).await {
            Ok(Some(key)) => Some(key),
            Ok(None) if api.provider.kind != ApiKind::OpenaiCompatible => {
                return Outcome::failed(
                    "The API key is missing from the system key store. Re-enter the API key."
                        .into(),
                    String::new(),
                    0,
                );
            }
            Ok(None) => None,
            Err(message) => return Outcome::failed(message, String::new(), 0),
        }
    } else {
        None
    };
    let key_text = key.as_ref().map(|k| k.as_str());
    let client = match chat.client() {
        Ok(c) => c,
        Err(m) => return Outcome::failed(m, String::new(), 0),
    };
    let request = http::ChatRequest {
        kind: api.provider.kind,
        base_url: &api.provider.base_url,
        headers: &api.provider.headers,
        key: key_text,
        model: &job.model,
        system: &job.system,
        max_tokens: api.max_tokens,
        messages: &api.context,
        attachments_dir: &api.attachments_dir,
    };
    let mut response = tokio::select! {
        _ = &mut stopped => return stopped_outcome(String::new(), 0),
        r = http::open_stream(&client, &request) => match r {
            Ok(r) => r,
            Err(m) => return Outcome::failed(http::redact(&m, key_text), String::new(), 0),
        },
    };
    if let Err(m) = mark_streaming(chat, app, job).await {
        return Outcome::failed(m, String::new(), 0);
    }

    let mut parser = http::SseParser::default();
    let mut acc = Acc::new();
    let (mut done, mut stop_reason): (bool, Option<String>) = (false, None);
    let mut failure: Option<String> = None;
    let mut oversized = false;

    'read: loop {
        let chunk = tokio::select! {
            _ = &mut stopped => return stopped_outcome(acc.content, acc.seq),
            c = response.chunk() => c,
        };
        let eof = matches!(chunk, Ok(None));
        let events = match chunk {
            Ok(Some(bytes)) => parser.push(&bytes),
            Ok(None) => Ok(parser.finish().into_iter().collect()),
            Err(e) if e.is_timeout() => {
                failure = Some("The provider did not respond for 120 seconds.".into());
                break;
            }
            Err(_) => break,
        };
        let events = match events {
            Ok(e) => e,
            Err(m) => {
                failure = Some(m);
                break;
            }
        };
        for event in events {
            for mapped in http::map_event(api.provider.kind, &event) {
                match mapped {
                    http::Mapped::Text(text) => {
                        if !acc.push(job, channel, text) {
                            oversized = true;
                            break 'read;
                        }
                    }
                    http::Mapped::Stop(reason) => stop_reason = Some(reason),
                    http::Mapped::Done => done = true,
                    http::Mapped::Error(m) => {
                        failure = Some(http::redact(&m, key_text));
                        break 'read;
                    }
                }
            }
        }
        if done || eof {
            break;
        }
        acc.flush_due(chat, app, job).await;
    }

    if let Some(message) = failure {
        return Outcome::failed(message, acc.content, acc.seq);
    }
    if oversized {
        return acc.oversized();
    }
    if !done && stop_reason.is_none() {
        return Outcome::failed(
            "The connection closed before the response finished; the partial response was kept."
                .into(),
            acc.content,
            acc.seq,
        );
    }
    Outcome {
        state: MessageState::Completed,
        content: acc.content,
        stream_seq: acc.seq,
        error: truncation_note(stop_reason.as_deref()).map(Into::into),
        stop_reason,
    }
}

/// How long the pipes may stay open after the CLI exits or reports its final event.
const DRAIN: std::time::Duration = std::time::Duration::from_secs(2);
/// No stdout for this long means the CLI is hung.
const INACTIVITY: std::time::Duration = std::time::Duration::from_secs(600);

/// Runs the CLI, then settles the stored session id. A session id is kept only
/// once the turn produced text or exited successfully (it may not exist
/// otherwise). A resumed send that fails without text clears it, so the next
/// send starts fresh instead of resuming a session the CLI cannot find.
async fn stream_cli(
    chat: &AiChat,
    app: &AppHandle,
    job: &Job,
    run: &CliJob,
    channel: &Channel<ThreadStreamEvent>,
    stop: &Notify,
) -> Outcome {
    let mut seen = None;
    let outcome = run_cli(chat, app, job, run, channel, stop, &mut seen).await;
    let next = if !outcome.content.is_empty() || outcome.state == MessageState::Completed {
        seen.filter(|s| run.session.as_deref() != Some(s.as_str()))
            .map(Some)
    } else if run.session.is_some() && outcome.state == MessageState::Failed {
        Some(None)
    } else {
        None
    };
    if let Some(next) = next {
        let (chat, app, thread) = (chat.clone(), app.clone(), job.thread_id.clone());
        let _ = blocking(move || {
            chat.with(&app, |i| {
                i.store.set_native_session(&thread, next.as_deref())
            })
        })
        .await;
    }
    outcome
}

/// Runs the CLI and maps its JSONL stdout onto the same deltas as an API stream.
/// The process group dies with `proc` on every return path (stop included).
async fn run_cli(
    chat: &AiChat,
    app: &AppHandle,
    job: &Job,
    run: &CliJob,
    channel: &Channel<ThreadStreamEvent>,
    stop: &Notify,
    session: &mut Option<String>,
) -> Outcome {
    let stopped = stop.notified();
    tokio::pin!(stopped);
    let mut proc = match cli::Proc::spawn(&run.spec) {
        Ok(p) => p,
        Err(m) => return Outcome::failed(m, String::new(), 0),
    };
    let _group = chat.track_group(proc.pgid());
    if let Err(m) = mark_streaming(chat, app, job).await {
        return Outcome::failed(m, String::new(), 0);
    }

    let mut parser = cli::Parser::new(&run.cli);
    let mut splitter = cli::LineSplitter::default();
    let mut acc = Acc::new();
    let mut fallback: Option<String> = None;
    let (mut failure, mut oversized) = (None::<String>, false);
    let mut buf = [0u8; 8192];
    let (mut exited, mut hung) = (false, false);
    // Set once the CLI exited or sent its final event; the pipes then get `DRAIN` to close.
    let mut drain_until: Option<tokio::time::Instant> = None;
    let mut last_byte = tokio::time::Instant::now();

    'read: loop {
        let read = tokio::select! {
            _ = &mut stopped => return stopped_outcome(acc.content, acc.seq),
            r = proc.stdout.read(&mut buf) => r,
            _ = proc.child.wait(), if !exited => {
                exited = true;
                drain_until.get_or_insert(tokio::time::Instant::now() + DRAIN);
                continue;
            }
            _ = tokio::time::sleep_until(drain_until.unwrap_or(last_byte)), if drain_until.is_some() => {
                proc.kill();
                break;
            }
            _ = tokio::time::sleep_until(last_byte + INACTIVITY) => {
                hung = true;
                break;
            }
        };
        let n = read.unwrap_or(0);
        if n > 0 {
            last_byte = tokio::time::Instant::now();
        }
        let lines = if n == 0 {
            splitter.finish()
        } else {
            splitter.push(&buf[..n])
        };
        for line in lines {
            for event in parser.line(&line) {
                match event {
                    cli::Ev::Text(text) => {
                        if !acc.push(job, channel, text) {
                            oversized = true;
                            break 'read;
                        }
                    }
                    cli::Ev::Final(text) => {
                        fallback = Some(text);
                        drain_until.get_or_insert(tokio::time::Instant::now() + DRAIN);
                    }
                    cli::Ev::Error(m) => {
                        failure = Some(http::redact(&m, None));
                        break 'read;
                    }
                    cli::Ev::Session(id) => {
                        if cli::valid_session(&id) {
                            *session = Some(id);
                        }
                    }
                }
            }
        }
        if n == 0 {
            break;
        }
        acc.flush_due(chat, app, job).await;
    }

    if hung {
        proc.kill();
        return Outcome::failed("The CLI stopped responding.".into(), acc.content, acc.seq);
    }
    if let Some(message) = failure {
        return Outcome::failed(message, acc.content, acc.seq);
    }
    if oversized {
        return acc.oversized();
    }
    let (success, status, stderr) = tokio::select! {
        _ = &mut stopped => return stopped_outcome(acc.content, acc.seq),
        r = proc.finish() => r,
    };
    if !success {
        let detail = if stderr.is_empty() {
            String::new()
        } else {
            format!(": {stderr}")
        };
        return Outcome::failed(
            format!("The CLI exited ({status}){detail}"),
            acc.content,
            acc.seq,
        );
    }
    if acc.content.is_empty() {
        // Some CLIs only report the complete answer at the end.
        let text = fallback.unwrap_or_default();
        if text.is_empty() || !acc.push(job, channel, text) {
            return Outcome::failed("The CLI returned no response.".into(), acc.content, acc.seq);
        }
    }
    Outcome {
        state: MessageState::Completed,
        content: acc.content.trim_end().to_string(),
        stream_seq: acc.seq,
        stop_reason: Some("stop".into()),
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(key: bool) -> ApiProvider {
        ApiProvider {
            instance_id: "p".into(),
            kind: ApiKind::Openai,
            label: "P".into(),
            accent_color: None,
            enabled: true,
            base_url: "https://api.openai.com/v1".into(),
            headers: vec![],
            models: vec![],
            api_key_set: key,
        }
    }

    #[test]
    fn stored_key_only_for_the_stored_target() {
        let p = provider(true);
        assert!(stored_key_applies(
            &p,
            "https://API.openai.com/v1",
            ApiKind::Openai
        ));
        assert!(!stored_key_applies(
            &p,
            "https://evil.example/v1",
            ApiKind::Openai
        ));
        assert!(!stored_key_applies(
            &p,
            "https://api.openai.com/v2",
            ApiKind::Openai
        ));
        assert!(!stored_key_applies(&p, &p.base_url, ApiKind::Anthropic));
        assert!(!stored_key_applies(
            &provider(false),
            &p.base_url,
            ApiKind::Openai
        ));
    }

    #[test]
    fn origin_comparison() {
        assert!(same_origin(
            "https://a.example/v1",
            "https://A.example/other"
        ));
        assert!(same_origin("https://a.example", "https://a.example:443/v1"));
        assert!(!same_origin("https://a.example", "http://127.0.0.1"));
        assert!(!same_origin("https://a.example", "https://a.example:8443"));
        assert!(!same_origin("https://a.example", "https://b.example"));
    }

    #[test]
    fn detect_plan_skips_existing_and_remembered() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            detect_plan(&v(&["claude", "codex", "pi"]), &v(&["codex"]), &v(&["pi"])),
            v(&["claude"])
        );
        assert!(detect_plan(&[], &v(&["codex"]), &[]).is_empty());
    }

    #[test]
    fn truncation_notes() {
        assert_eq!(truncation_note(Some("length")), Some(NOTE_LIMIT));
        assert_eq!(truncation_note(Some("max_tokens")), Some(NOTE_LIMIT));
        assert_eq!(truncation_note(Some("stop")), None);
        assert_eq!(truncation_note(None), None);
    }

    #[test]
    fn active_guard_removes_its_own_entry_only() {
        let chat = AiChat::default();
        let first = ActiveGuard::new(&chat, "t".into()).unwrap();
        let second = ActiveGuard::new(&chat, "t".into()).unwrap();
        drop(first);
        assert!(chat.active().unwrap().contains_key("t"));
        let handle = std::thread::spawn(move || {
            let _guard = second;
            panic!("stream failed");
        });
        assert!(handle.join().is_err());
        assert!(chat.active().unwrap().is_empty());
    }
}

// ---- attachments -----------------------------------------------------

/// Imports one file already read into memory; `path` is only used for the name.
fn import_file(i: &mut Inner, path: &std::path::Path) -> Result<Attachment, String> {
    let path = std::fs::canonicalize(path).map_err(|_| "Cannot read this file.")?;
    let data = attachments::read_capped(&path)?;
    let name = path
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    i.store.attachment_import(&name, &data)
}

/// Imports every path or none: a failure discards the files imported before it.
fn import_all(i: &mut Inner, paths: &[std::path::PathBuf]) -> Result<Vec<Attachment>, String> {
    if paths.len() > attachments::MAX_COUNT {
        return Err("Attach at most 10 files at a time.".into());
    }
    let mut done: Vec<Attachment> = Vec::new();
    for path in paths {
        match import_file(i, path) {
            Ok(a) => done.push(a),
            Err(m) => {
                for a in &done {
                    let _ = i.store.attachment_discard(&a.id);
                }
                return Err(m);
            }
        }
    }
    Ok(done)
}

#[tauri::command]
pub(crate) async fn ai_attachment_pick(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
) -> Result<Vec<Attachment>, String> {
    use tauri_plugin_dialog::DialogExt;
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    let picker = app.clone();
    let picked = blocking(move || {
        Ok(picker
            .dialog()
            .file()
            .set_parent(&window)
            .blocking_pick_files())
    })
    .await?;
    let Some(picked) = picked else {
        return Ok(Vec::new());
    };
    let paths = picked
        .into_iter()
        .map(|p| {
            p.into_path()
                .map_err(|_| "Cannot read this file.".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    blocking(move || chat.with(&app, |i| import_all(i, &paths))).await
}

#[tauri::command]
pub(crate) async fn ai_attachment_import_paths(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    paths: Vec<String>,
) -> Result<Vec<Attachment>, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || {
        let paths: Vec<_> = paths.into_iter().map(std::path::PathBuf::from).collect();
        chat.with(&app, |i| import_all(i, &paths))
    })
    .await
}

#[tauri::command]
pub(crate) async fn ai_attachment_import_bytes(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    name: String,
    data: String,
) -> Result<Attachment, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || {
        use base64::Engine;
        if data.len() > attachments::MAX_IMAGE / 3 * 4 + 4 {
            return Err("Pasted images are limited to 10 MB.".into());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data.as_bytes())
            .map_err(|_| "The pasted data is not valid.")?;
        chat.with(&app, |i| {
            if !matches!(attachments::classify(&bytes)?.0, Kind::Image) {
                return Err("Only images can be pasted.".into());
            }
            i.store.attachment_import(&name, &bytes)
        })
    })
    .await
}

#[tauri::command]
pub(crate) async fn ai_attachment_discard(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    id: String,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || chat.with(&app, |i| i.store.attachment_discard(&id))).await
}

#[tauri::command]
pub(crate) async fn ai_attachment_preview(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    id: String,
) -> Result<String, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || {
        chat.with(&app, |i| {
            let a = i.store.attachment(&id)?;
            if a.kind != Kind::Image {
                return Err("Only images have a preview.".into());
            }
            let data = attachments::read(&i.store.attachments_dir(), &a)?;
            Ok(attachments::data_url(&a.mime, &data))
        })
    })
    .await
}
