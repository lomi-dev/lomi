use super::{
    attachments,
    backend::Backend,
    preferences::Preferences,
    store::{Config, Origin, Start},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use tauri::{ipc::Channel, Emitter, State, Window};

static PENDING_OPERATIONS: AtomicUsize = AtomicUsize::new(0);
struct Operation;
impl Operation {
    fn acquire() -> Result<Self, String> {
        PENDING_OPERATIONS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 16).then_some(n + 1)
            })
            .map(|_| Self)
            .map_err(|_| "Chat storage is busy. Retry when the current operation finishes.".into())
    }
}
impl Drop for Operation {
    fn drop(&mut self) {
        PENDING_OPERATIONS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Default)]
pub struct Chats(pub Mutex<Option<Arc<Backend>>>);
impl Chats {
    pub fn backend(&self, app: &tauri::AppHandle) -> Result<Arc<Backend>, String> {
        let mut state = self.0.lock().map_err(|_| "Chat state unavailable.")?;
        if let Some(value) = state.as_ref() {
            return Ok(value.clone());
        }
        let value = Backend::open(app)?;
        *state = Some(value.clone());
        Ok(value)
    }
    pub fn stop(&self) {
        if let Some(backend) = self.0.lock().unwrap().as_ref() {
            backend.stop();
        }
    }
}
fn main(window: &Window) -> Result<(), String> {
    crate::files::main_window(window)
}
fn settings(window: &Window) -> Result<(), String> {
    if window.label() == "settings" {
        Ok(())
    } else {
        Err("Only Settings may change AI connections or run connection tests.".into())
    }
}
fn changed(app: &tauri::AppHandle) {
    for label in ["main", "settings"] {
        let _ = app.emit_to(
            tauri::EventTarget::webview(label),
            "chat-preferences-changed",
            (),
        );
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum MainAction {
    Create {
        id: String,
        origin: Origin,
    },
    Load {
        id: String,
        #[serde(default)]
        offset: usize,
    },
    List {
        query: String,
        workspace: Option<String>,
        project: Option<String>,
        #[serde(default)]
        offset: usize,
    },
    Rename {
        id: String,
        title: String,
    },
    Pin {
        id: String,
        pinned: bool,
    },
    Delete {
        id: String,
    },
    Draft {
        id: String,
        text: String,
        expected: i64,
    },
    Configure {
        id: String,
        config: Config,
        expected: i64,
    },
    Variant {
        id: String,
        target: String,
        expected: i64,
    },
    Import {
        id: String,
        attachment: String,
        path: Option<PathBuf>,
        name: Option<String>,
        bytes: Option<Vec<u8>>,
        expected: i64,
        #[serde(default)]
        approved: bool,
    },
    RemoveAttachment {
        id: String,
        attachment: String,
        expected: i64,
    },
    Attachment {
        attachment: String,
    },
    AttachmentMeta {
        attachment: String,
    },
}
#[tauri::command]
pub async fn chat_main(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Chats>,
    input: MainAction,
) -> Result<Value, String> {
    main(&window)?;
    let backend = state.backend(&app)?;
    let permit = Operation::acquire()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        if let MainAction::Delete { id } = &input {
            backend.close(std::slice::from_ref(id))?;
        }
        let mut services = backend
            .services
            .lock()
            .map_err(|_| "Chat storage unavailable.")?;
        let defaults = services
            .settings
            .as_ref()
            .map(|s| s.data.defaults.clone())
            .unwrap_or_default();
        let store = services.store.as_mut().map_err(|e| e.clone())?;
        let history_changed = matches!(
            &input,
            MainAction::Create { .. }
                | MainAction::Rename { .. }
                | MainAction::Pin { .. }
                | MainAction::Delete { .. }
        );
        let value = match input {
            MainAction::Create { id, origin } => {
                serde_json::to_value(store.create(&id, &origin, &defaults)?).unwrap()
            }
            MainAction::Load { id, offset } => {
                let loaded = store.load(&id, offset)?;
                if let Some(request) = &loaded.request {
                    if request["status"] == "active"
                        && !backend
                            .requests
                            .lock()
                            .unwrap()
                            .contains_key(request["id"].as_str().unwrap_or(""))
                    {
                        store.interrupt_orphan(
                            request["id"].as_str().ok_or("Invalid request ID.")?,
                        )?;
                        serde_json::to_value(store.load(&id, offset)?).unwrap()
                    } else {
                        serde_json::to_value(loaded).unwrap()
                    }
                } else {
                    serde_json::to_value(loaded).unwrap()
                }
            }
            MainAction::List {
                query,
                workspace,
                project,
                offset,
            } => serde_json::to_value(store.list(
                &query,
                workspace.as_deref(),
                project.as_deref(),
                offset,
            )?)
            .unwrap(),
            MainAction::Rename { id, title } => {
                serde_json::to_value(store.rename(&id, &title)?).unwrap()
            }
            MainAction::Pin { id, pinned } => {
                serde_json::to_value(store.pin(&id, pinned)?).unwrap()
            }
            MainAction::Delete { id } => {
                store.delete(&id)?;
                let _ = app.emit_to(
                    tauri::EventTarget::webview("main"),
                    "chat-conversation-deleted",
                    &id,
                );
                attachments::reconcile(store, &backend.owner.root)?;
                Value::Null
            }
            MainAction::Draft { id, text, expected } => {
                serde_json::to_value(store.save_draft(&id, &text, expected)?).unwrap()
            }
            MainAction::Configure {
                id,
                config,
                expected,
            } => serde_json::to_value(store.configure(&id, &config, expected)?).unwrap(),
            MainAction::Variant {
                id,
                target,
                expected,
            } => {
                store.select(&id, &target, expected)?;
                serde_json::to_value(store.load(&id, 0)?).unwrap()
            }
            MainAction::Import {
                id,
                attachment,
                path,
                name,
                bytes,
                expected,
                approved,
            } => {
                let (name, bytes) = match (path, name, bytes) {
                    (Some(path), None, None) => (
                        path.file_name()
                            .and_then(|n| n.to_str())
                            .ok_or("Invalid attachment filename.")?
                            .to_owned(),
                        attachments::read_selected(&path)?,
                    ),
                    (None, Some(name), Some(bytes)) => (name, bytes),
                    _ => return Err("Choose one explicit attachment source.".into()),
                };
                let value = attachments::import(
                    store,
                    &backend.owner.root,
                    attachments::Import {
                        conversation: &id,
                        id: &attachment,
                        name: &name,
                        bytes: &bytes,
                        expected,
                        approved,
                    },
                )?;
                json!({"attachment":value,"draft":store.draft(&id)?})
            }
            MainAction::RemoveAttachment {
                id,
                attachment,
                expected,
            } => {
                let draft = store.remove_attachment(&id, &attachment, expected)?;
                attachments::reconcile(store, &backend.owner.root)?;
                serde_json::to_value(draft).unwrap()
            }
            MainAction::AttachmentMeta { attachment } => store.attachment_meta(&attachment)?,
            MainAction::Attachment { attachment } => {
                let (meta, bytes) =
                    attachments::read_object(store, &backend.owner.root, &attachment)?;
                json!({"attachment":meta,"data":super::process::encode(&bytes)})
            }
        };
        if history_changed {
            let _ = app.emit_to(
                tauri::EventTarget::webview("main"),
                "chat-history-changed",
                (),
            );
        }
        Ok(value)
    })
    .await
    .map_err(|_| "Chat operation failed.")?
}
#[tauri::command]
pub async fn chat_preferences(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Chats>,
) -> Result<Preferences, String> {
    if !matches!(window.label(), "main" | "settings") {
        return Err("Unknown application view.".into());
    }
    let backend = state.backend(&app)?;
    let permit = Operation::acquire()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        backend
            .services
            .lock()
            .map_err(|_| "Settings unavailable.")?
            .settings
            .as_ref()
            .map(|s| s.data.clone())
            .map_err(|e| e.clone())
    })
    .await
    .map_err(|_| "Cannot read Chat AI settings.")?
}
#[tauri::command]
pub async fn chat_preferences_save(
    window: Window,
    state: State<'_, Chats>,
    data: Preferences,
    expected: u64,
    key_connection: Option<String>,
    new_key: Option<String>,
    clear_key: Option<String>,
) -> Result<Preferences, String> {
    use tauri::Manager;
    settings(&window)?;
    let app = window.app_handle().clone();
    let backend = state.backend(&app)?;
    let permit = Operation::acquire()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let changing = {
            let services = backend
                .services
                .lock()
                .map_err(|_| "Settings unavailable.")?;
            let current = &services.settings.as_ref().map_err(|e| e.clone())?.data;
            if current.revision != expected {
                return Err("conflict: Chat AI settings changed.".into());
            }
            let ids = current
                .connections
                .iter()
                .filter(|old| {
                    key_connection.as_ref() == Some(&old.id)
                        || clear_key.as_ref() == Some(&old.id)
                        || !data.connections.iter().any(|next| {
                            next.id == old.id
                                && next.enabled == old.enabled
                                && next.provider == old.provider
                                && next.secret_mode == old.secret_mode
                        })
                })
                .map(|c| c.id.clone())
                .collect::<Vec<_>>();
            backend.changing.lock().unwrap().extend(ids.clone());
            ids
        };
        let result = (|| {
            backend.close_connections(&changing)?;
            let mut services = backend
                .services
                .lock()
                .map_err(|_| "Settings unavailable.")?;
            let settings = services.settings.as_mut().map_err(|e| e.clone())?;
            if let Some(id) = clear_key {
                return settings.clear_key(&id, expected);
            }
            settings.save(
                data,
                expected,
                key_connection.as_deref().zip(new_key.as_deref()),
            )
        })();
        let mut blocked = backend.changing.lock().unwrap();
        for id in changing {
            blocked.remove(&id);
        }
        drop(blocked);
        changed(&app);
        result
    })
    .await
    .map_err(|_| "Cannot save Chat AI settings.")?
}
#[tauri::command]
pub async fn chat_generate(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Chats>,
    input: Start,
    channel: Channel<Value>,
) -> Result<Value, String> {
    main(&window)?;
    let backend = state.backend(&app)?;
    tauri::async_runtime::spawn_blocking(move || backend.start(input, channel))
        .await
        .map_err(|_| "Cannot start the AI request.")?
}
#[tauri::command]
pub fn chat_cancel(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Chats>,
    request_id: String,
) -> Result<(), String> {
    main(&window)?;
    state.backend(&app)?.cancel(&request_id)
}
#[tauri::command]
pub fn chat_subscribe(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Chats>,
    request_id: String,
    channel: Channel<Value>,
) -> Result<bool, String> {
    main(&window)?;
    let backend = state.backend(&app)?;
    let request = backend.requests.lock().unwrap().get(&request_id).cloned();
    if let Some(request) = request {
        let mut delivery = request.delivery.lock().unwrap();
        *request.channel.lock().unwrap() = Some(channel.clone());
        channel
            .send(delivery.subscribe())
            .map_err(|_| "Chat channel unavailable.")?;
        Ok(true)
    } else {
        Ok(false)
    }
}
#[tauri::command]
pub fn chat_ack(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Chats>,
    request_id: String,
    epoch: u64,
    sequence: u64,
) -> Result<(), String> {
    main(&window)?;
    let backend = state.backend(&app)?;
    if let Some(request) = backend.requests.lock().unwrap().get(&request_id).cloned() {
        request.delivery.lock().unwrap().ack(epoch, sequence);
    }
    Ok(())
}
#[tauri::command]
pub async fn chat_close(
    window: Window,
    state: State<'_, Chats>,
    conversations: Vec<String>,
    all: Option<bool>,
) -> Result<(), String> {
    main(&window)?;
    let Some(backend) = state
        .0
        .lock()
        .map_err(|_| "Chat state unavailable.")?
        .clone()
    else {
        return Ok(());
    };
    tauri::async_runtime::spawn_blocking(move || {
        if all.unwrap_or(false) {
            backend.close_all()
        } else {
            backend.close(&conversations)
        }
    })
    .await
    .map_err(|_| "Chat close failed. The views remain open.")?
}

#[tauri::command]
pub async fn chat_preview_models(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Chats>,
    provider: String,
    api_key: String,
) -> Result<Value, String> {
    settings(&window)?;
    let backend = state.backend(&app)?;
    let permit = Operation::acquire()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        backend.preview_models(&provider, &api_key)
    })
    .await
    .map_err(|_| "Could not load models from the provider.")?
}

#[tauri::command]
pub async fn chat_connection_action(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Chats>,
    connection_id: String,
    model: String,
    operation: String,
) -> Result<Value, String> {
    settings(&window)?;
    let backend = state.backend(&app)?;
    let permit = Operation::acquire()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let result = backend.auxiliary(&connection_id, &model, &operation);
        changed(&app);
        result
    })
    .await
    .map_err(|_| "Connection operation failed.")?
}

#[tauri::command]
pub async fn chat_flush(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Chats>,
    conversation: String,
) -> Result<(), String> {
    main(&window)?;
    let backend = state.backend(&app)?;
    tauri::async_runtime::spawn_blocking(move || backend.flush(&conversation))
        .await
        .map_err(|_| "Cannot retry chat persistence.")?
}
#[tauri::command]
pub fn chat_retain(
    window: Window,
    state: State<'_, Chats>,
    conversations: Vec<String>,
) -> Result<(), String> {
    main(&window)?;
    if let Some(backend) = state
        .0
        .lock()
        .map_err(|_| "Chat state unavailable.")?
        .as_ref()
    {
        backend.requests.lock().unwrap().retain(|_, r| {
            r.conversation
                .as_ref()
                .is_none_or(|id| conversations.contains(id))
                || !r
                    .done
                    .0
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|result| result.is_ok())
        });
    }
    Ok(())
}
#[tauri::command]
pub async fn chat_export(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Chats>,
    conversation: String,
    format: String,
    ram: Option<Value>,
) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    main(&window)?;
    if !matches!(format.as_str(), "json" | "markdown") {
        return Err("Unknown chat export format.".into());
    }
    let backend = state.backend(&app)?;
    let permit = Operation::acquire()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let bytes = {
            let services = backend.services.lock().map_err(|_|"Chat storage unavailable.")?;
            let store = services.store.as_ref().map_err(|e|e.clone());
            let exported = store.and_then(|store| store.export(&conversation, &format));
            if let Some(ram) = ram {
                if serde_json::to_vec(&ram).map_err(|_|"Invalid export.")?.len() > 42*1024*1024 {return Err("The in-memory export exceeds its size limit.".into());}
                serde_json::to_vec_pretty(&json!({"version":1,"saved":exported.ok().and_then(|bytes|serde_json::from_slice::<Value>(&bytes).ok()),"unsaved":ram})).map_err(|_|"Cannot encode export.")?
            } else { exported? }
        };
        let extension = if format=="json" {"json"} else {"md"};
        let Some(path) = app.dialog().file().set_parent(&window).set_title("Export conversation (attachment descriptions only)").set_file_name(format!("conversation.{extension}")).add_filter("Conversation", &[extension]).blocking_save_file() else {return Ok(false);};
        let path = path.into_path().map_err(|_|"Invalid export destination.")?;
        super::storage::atomic(&path,&bytes)?;
        Ok(true)
    }).await.map_err(|_|"Cannot export the conversation.")?
}

#[tauri::command]
pub async fn chat_discard(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Chats>,
    conversation: String,
) -> Result<(), String> {
    main(&window)?;
    let backend = state.backend(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let requests = backend
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, r)| r.conversation.as_deref() == Some(&conversation))
            .map(|(id, r)| (id.clone(), r.clone()))
            .collect::<Vec<_>>();
        for (id, request) in requests {
            backend.cancel(&id)?;
            let done = request.done.0.lock().unwrap();
            let mut done = request
                .done
                .1
                .wait_timeout_while(done, std::time::Duration::from_secs(5), |done| {
                    done.is_none()
                })
                .map_err(|_| "Chat shutdown unavailable.")?
                .0;
            if done.is_none() {
                return Err(
                    "Wait for the native request to stop before discarding its unsaved response."
                        .into(),
                );
            }
            // This explicit user action relinquishes the unsaved RAM snapshot only.
            // The next load recovers an orphaned active database row as interrupted.
            *done = Some(Ok(()));
        }
        Ok(())
    })
    .await
    .map_err(|_| "Cannot discard the unsaved conversation.")?
}

#[tauri::command]
pub async fn chat_recover(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Chats>,
    target: String,
    reset: bool,
) -> Result<(), String> {
    use super::{
        preferences::{Settings, SystemSecrets},
        storage,
        store::Store,
    };
    use tauri::Manager;
    if target == "settings" {
        settings(&window)?;
    } else if target == "history" {
        main(&window)?;
    } else {
        return Err("Unknown chat recovery target.".into());
    }
    let backend = state.backend(&app)?;
    let permit = Operation::acquire()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let mut services = backend
            .services
            .lock()
            .map_err(|_| "Chat storage unavailable.")?;
        if target == "settings" {
            if services.settings.is_ok() {
                return Err("Working settings do not require recovery.".into());
            }
            let config = app
                .path()
                .app_config_dir()
                .map_err(|_| "Cannot locate preferences.")?;
            if reset {
                storage::backup_files(&config, &["chat-ai-preferences.json"])?;
            }
            services.settings = Settings::open(
                config.join("chat-ai-preferences.json"),
                &backend.owner.root,
                SystemSecrets,
            );
            services.settings.as_ref().map_err(|e| e.clone())?;
            changed(&app);
        } else {
            if services.store.is_ok() {
                return Err(
                    "Working history does not require recovery. Retry saving instead.".into(),
                );
            }
            if reset {
                storage::backup_files(
                    &backend.owner.root,
                    &[
                        "history.sqlite3",
                        "history.sqlite3-wal",
                        "history.sqlite3-shm",
                        "attachments",
                    ],
                )?;
            }
            services.store = Store::open(&backend.owner.root);
            services.store.as_ref().map_err(|e| e.clone())?;
        }
        Ok(())
    })
    .await
    .map_err(|_| "Chat recovery failed.")?
}
