use super::{
    attachments::{self, Attachment, Kind},
    cli, emit_changed,
    engine::{valid_operation, EventSink, Inner},
    http,
    settings::{self, *},
    store::{self, *},
    AiChat,
};
use std::{collections::HashMap, sync::Arc};
use tauri::{ipc::Channel, AppHandle, State, Window};
use zeroize::Zeroizing;

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

pub(super) fn read_key(
    app: &AppHandle,
    instance_id: &str,
) -> Result<Option<Zeroizing<String>>, String> {
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

#[tauri::command]
pub(crate) async fn ai_providers_detect(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
) -> Result<DetectResult, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || {
        let installed = cli::installed_clis();
        // Planned and committed under one storage lock, so concurrent scans cannot double-add.
        let (settings, added) = chat.with(&app, |i| {
            let Inner { settings, dir, .. } = i;
            let added = settings::detect_cli_providers(settings, dir, &installed)?;
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
    let client = chat.engine(&app)?.client()?;
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
    let models = http::probe(&client, kind, &base_url, &headers, key.as_ref()).await?;
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
    limit: Option<usize>,
) -> Result<ThreadSnapshot, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || chat.with(&app, |i| i.store.snapshot_limited(&thread_id, limit))).await
}

/// Up to `limit` messages older than `before_seq`, oldest first.
#[tauri::command]
pub(crate) async fn ai_thread_messages_before(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    thread_id: String,
    before_seq: u64,
    limit: usize,
) -> Result<MessagePage, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || {
        chat.with(&app, |i| {
            i.store.messages_before(&thread_id, before_seq, limit)
        })
    })
    .await
}

/// The in-memory state of a thread's streaming reply, for a client that subscribed
/// to the stream first: it drops deltas with `seq` up to the returned one.
#[tauri::command]
pub(crate) async fn ai_thread_live(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    thread_id: String,
) -> Result<Option<ThreadLive>, String> {
    crate::files::main_window(&window)?;
    Ok(state.engine(&app)?.live(&thread_id))
}

/// Run state of the threads `ai_threads_list` returns for a workspace.
#[tauri::command]
pub(crate) async fn ai_threads_status(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    project: String,
    workspace_id: String,
) -> Result<Vec<ThreadStatus>, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    let engine = chat.engine(&app)?.clone();
    let threads =
        blocking(move || chat.with(&app, |i| i.store.list(&project, &workspace_id))).await?;
    Ok(threads
        .into_iter()
        .map(|t| ThreadStatus {
            state: engine.run_state(&t.thread_id),
            thread_id: t.thread_id,
        })
        .collect())
}

const MENTION_FILE_CAP: usize = 20_000;
const MENTION_WALK_DEPTH: usize = 8;

/// Bounded, symlink-free directory walk used outside Git work trees.
fn walk_files(root: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new(), 0usize)];
    while let Some((dir, prefix, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.')
                || matches!(name.as_str(), "node_modules" | "target" | "dist" | "build")
            {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let rel = format!("{prefix}{name}");
            if kind.is_file() {
                out.push(rel);
                if out.len() >= MENTION_FILE_CAP {
                    return out;
                }
            } else if kind.is_dir() && depth < MENTION_WALK_DEPTH {
                stack.push((entry.path(), format!("{rel}/"), depth + 1));
            }
        }
    }
    out
}

/// Lists files under a CLI thread's working directory for `@` mentions.
#[tauri::command]
pub(crate) async fn ai_thread_files(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    thread_id: String,
) -> Result<Vec<String>, String> {
    crate::files::main_window(&window)?;
    let chat = state.inner().clone();
    blocking(move || {
        let Some(cwd) = chat.with(&app, |i| i.store.thread(&thread_id))?.cwd else {
            return Ok(Vec::new());
        };
        let root = std::path::Path::new(&cwd);
        let mut files =
            match crate::git::checked(root, &["ls-files", "-co", "--exclude-standard", "-z"]) {
                Ok(bytes) => bytes
                    .split(|b| *b == 0)
                    .filter(|p| !p.is_empty())
                    .take(MENTION_FILE_CAP)
                    .map(|p| String::from_utf8_lossy(p).into_owned())
                    .collect(),
                Err(_) => walk_files(root),
            };
        files.sort_unstable();
        files.dedup();
        Ok(files)
    })
    .await
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
        let cwd = chat.with(&app, |i| {
            // Without a running stream, leftover in-flight rows (e.g. after a panic) must not block deletion.
            if !chat.engine(&app)?.is_active(&request.thread_id)? {
                i.store.interrupt_inflight(&request.thread_id, false)?;
            }
            let cwd = i.store.thread(&request.thread_id)?.cwd;
            i.store.delete(&request)?;
            Ok(cwd)
        })?;
        let engine = chat.engine(&app)?;
        engine.forget_session(&request.thread_id);
        engine.remove_checkpoints(&request.thread_id, cwd.as_deref());
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
    let engine = state.engine(&app)?.clone();
    blocking(move || engine.stop(&thread_id)).await
}

/// Unified diff of one file changed by an assistant turn.
#[tauri::command]
pub(crate) async fn ai_turn_diff(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    thread_id: String,
    message_id: String,
    path: String,
) -> Result<String, String> {
    crate::files::main_window(&window)?;
    let engine = state.engine(&app)?.clone();
    blocking(move || engine.turn_diff(&thread_id, &message_id, &path)).await
}

/// Restores the files an assistant turn changed (only `paths` when given); an explicit UI action.
#[tauri::command]
pub(crate) async fn ai_turn_undo(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    thread_id: String,
    message_id: String,
    paths: Option<Vec<String>>,
) -> Result<lomi_chat::checkpoint::UndoReport, String> {
    crate::files::main_window(&window)?;
    let engine = state.engine(&app)?.clone();
    let (report, revision) = {
        let thread_id = thread_id.clone();
        blocking(move || engine.turn_undo(&thread_id, &message_id, paths)).await?
    };
    emit_changed(&app, "threads", revision, Some(&thread_id));
    Ok(report)
}

/// Answers a tool-permission prompt of a running Claude turn.
#[tauri::command]
pub(crate) async fn ai_thread_respond(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    thread_id: String,
    request_id: String,
    decision: Decision,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    let engine = state.engine(&app)?.clone();
    blocking(move || engine.respond(&thread_id, &request_id, decision)).await
}

struct ChannelSink(Channel<ThreadStreamEvent>);

impl EventSink for ChannelSink {
    fn send(&self, event: ThreadStreamEvent) {
        let _ = self.0.send(event);
    }

    fn try_send(&self, event: ThreadStreamEvent) -> bool {
        self.0.send(event).is_ok()
    }
}

/// Subscribes `on_event` to the running turn of a thread and returns its state so
/// far (`None` when no turn runs); events after `live.seq` follow on the channel.
#[tauri::command]
pub(crate) async fn ai_thread_attach(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    thread_id: String,
    on_event: Channel<ThreadStreamEvent>,
) -> Result<Option<ThreadLive>, String> {
    crate::files::main_window(&window)?;
    Ok(state
        .engine(&app)?
        .attach(&thread_id, Arc::new(ChannelSink(on_event))))
}

/// Adds a message to the running turn. Fails with `steer_unsupported` when the CLI
/// cannot take it; the caller then queues the message.
#[tauri::command]
pub(crate) async fn ai_thread_steer(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    thread_id: String,
    text: String,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    state.engine(&app)?.steer(&thread_id, text).await
}

/// Answers a pending question of a running turn by item id; `null` declines it.
#[tauri::command]
pub(crate) async fn ai_thread_answer(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    thread_id: String,
    request_id: String,
    answers: Option<HashMap<String, Vec<String>>>,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    let engine = state.engine(&app)?.clone();
    blocking(move || engine.answer(&thread_id, &request_id, answers)).await
}

/// Slash commands the thread's CLI reported; empty before its session started.
#[tauri::command]
pub(crate) async fn ai_thread_commands(
    window: Window,
    app: AppHandle,
    state: State<'_, AiChat>,
    thread_id: String,
) -> Result<Vec<ThreadCommand>, String> {
    crate::files::main_window(&window)?;
    Ok(state.engine(&app)?.commands(&thread_id))
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
    state
        .engine(&app)?
        .send(request, Arc::new(ChannelSink(on_event)))
        .await
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
