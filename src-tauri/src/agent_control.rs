//! Trusted native UI is the sole source of pairing decisions. An empty state
//! opens no socket, scans no project directory and starts no background task.
use lomi_control_protocol::control::{Projection, UiAck};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use tauri::{Emitter, Manager, State, Window};
#[cfg(unix)]
use {
    lomi_control_core::broker::{Broker, Overview},
    std::sync::Arc,
};

pub(crate) fn supported_host() -> bool {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        objc2_foundation::NSProcessInfo::processInfo()
            .operatingSystemVersion()
            .majorVersion
            >= 14
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        false
    }
}

#[derive(Default)]
pub struct Control {
    #[cfg(unix)]
    broker: Mutex<Option<Arc<Broker>>>,
    transition: tokio::sync::Mutex<()>,
    closing: AtomicBool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlState {
    supported: bool,
    #[cfg(unix)]
    broker: Option<Overview>,
    #[cfg(not(unix))]
    broker: Option<()>,
    helper_path: Option<String>,
}
fn settings(window: &Window) -> Result<(), String> {
    if window.label() == "settings" {
        Ok(())
    } else {
        Err("Agent control permissions require Settings.".into())
    }
}
fn unavailable() -> String {
    "Agent control is unavailable. Reopen its Settings page.".into()
}

#[tauri::command]
pub async fn agent_control_settings_source(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
    revision: String,
) -> Result<serde_json::Value, String> {
    crate::files::main_window(&window)?;
    #[cfg(unix)]
    {
        let broker = state.required()?;
        tauri::async_runtime::spawn_blocking(move || {
            let permit = broker.settings_update_source_permit(&operation_id, &nonce, &revision)?;
            let source = crate::keybindings::agent_source(&app, &|| permit.check())?;
            permit.check()?;
            serde_json::to_value(source)
                .map_err(|_| lomi_control_protocol::ErrorCode::ResourceExhausted)
        })
        .await
        .map_err(|_| "OUTCOME_UNKNOWN".to_string())?
        .map_err(|code| {
            serde_json::to_value(code)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
    }
    #[cfg(not(unix))]
    {
        let _ = (app, state, operation_id, nonce, revision);
        Err("HOST_UNQUALIFIED".into())
    }
}

#[tauri::command]
pub async fn agent_control_settings_prepare(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
    revision: String,
    current: lomi_control_protocol::settings::SettingsUpdateValues,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    #[cfg(unix)]
    {
        let broker = state.required()?;
        let permit = tauri::async_runtime::spawn_blocking(move || {
            broker.prepare_settings_update(&operation_id, &nonce, &revision, current)
        })
        .await
        .map_err(|_| "OUTCOME_UNKNOWN".to_string())?
        .map_err(|code| {
            serde_json::to_value(code)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })?;
        crate::settings_window::request_checked(&app, Some("agent-control".into()), || {
            permit.check().map_err(|_| "CONTROL_REVOKED".into())
        })
        .await
        .map_err(|_| "OUTCOME_UNKNOWN".into())
    }
    #[cfg(not(unix))]
    {
        let _ = (app, state, operation_id, nonce, revision, current);
        Err("HOST_UNQUALIFIED".into())
    }
}

#[tauri::command]
pub async fn agent_control_settings_decide(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    approve: bool,
) -> Result<(), String> {
    settings(&window)?;
    #[cfg(unix)]
    {
        let broker = state.required()?;
        tauri::async_runtime::spawn_blocking(move || {
            broker.decide_settings_update(&operation_id, approve)
        })
        .await
        .map_err(|_| "OUTCOME_UNKNOWN".to_string())?
        .map_err(|code| {
            serde_json::to_value(code)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
    }
    #[cfg(not(unix))]
    {
        let _ = (state, operation_id, approve);
        Err("HOST_UNQUALIFIED".into())
    }
}

#[tauri::command]
pub async fn agent_control_settings_open(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    #[cfg(unix)]
    {
        use lomi_control_protocol::control::UiAction;
        let broker = state.required()?;
        let preparing = broker.clone();
        let operation = operation_id.clone();
        let token = nonce.clone();
        let (command, permit) = tauri::async_runtime::spawn_blocking(move || {
            preparing.begin_settings_open(&operation, &token)
        })
        .await
        .map_err(|_| "OUTCOME_UNKNOWN".to_string())?
        .map_err(|code| {
            serde_json::to_value(code)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })?;
        let UiAction::OpenSettings(command) = command.action else {
            return Err("SCOPE_DENIED".into());
        };
        crate::settings_window::request_checked(&app, Some(command.page.as_str().into()), || {
            permit.check().map_err(|_| "CONTROL_REVOKED".into())
        })
        .await
        .map_err(|_| "OUTCOME_UNKNOWN".to_string())?;
        tauri::async_runtime::spawn_blocking(move || {
            broker.complete_settings_open(&operation_id, &nonce)
        })
        .await
        .map_err(|_| "OUTCOME_UNKNOWN".to_string())?
        .map_err(|_| "OUTCOME_UNKNOWN".to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = (app, state, operation_id, nonce);
        Err("HOST_UNQUALIFIED".into())
    }
}

#[tauri::command]
pub fn agent_control_open_recovery(window: Window, app: tauri::AppHandle) -> Result<bool, String> {
    settings(&window)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        use tauri_plugin_opener::OpenerExt;
        let root = app
            .path()
            .app_data_dir()
            .map_err(|_| unavailable())?
            .join("agent-control");
        let path = root.join("trash-recovery");
        for directory in [&root, &path] {
            let metadata = match directory.symlink_metadata() {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(_) => return Err("Recovery data is unavailable.".into()),
            };
            if !metadata.is_dir()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o777 != 0o700
            {
                return Err(
                    "The recovery directory has unexpected ownership or permissions.".into(),
                );
            }
        }
        app.opener()
            .open_path(path.to_string_lossy().into_owned(), None::<&str>)
            .map_err(|error| error.to_string())?;
        Ok(true)
    }
    #[cfg(not(unix))]
    {
        let _ = app;
        Err("Recovery has not been qualified on this host.".into())
    }
}
fn helper_path() -> Option<String> {
    let path = std::env::current_exe()
        .ok()?
        .parent()?
        .join(if cfg!(windows) {
            "lomi-mcp.exe"
        } else {
            "lomi-mcp"
        });
    path.is_file().then(|| path.to_string_lossy().into_owned())
}
#[cfg(unix)]
impl Control {
    pub(crate) fn current(&self) -> Result<Option<Arc<Broker>>, String> {
        self.broker
            .lock()
            .map(|v| v.clone())
            .map_err(|_| unavailable())
    }
    pub(crate) fn required(&self) -> Result<Arc<Broker>, String> {
        if self.closing.load(Ordering::SeqCst) {
            return Err("Lomi is preparing to close.".into());
        }
        self.current()?.ok_or_else(unavailable)
    }
}

#[tauri::command]
pub async fn agent_control_state(
    window: Window,
    state: State<'_, Control>,
) -> Result<ControlState, String> {
    settings(&window)?;
    Ok(ControlState {
        supported: supported_host(),
        #[cfg(unix)]
        broker: {
            let broker = state.current()?;
            tauri::async_runtime::spawn_blocking(move || broker.map(|b| b.overview()).transpose())
                .await
                .map_err(|_| unavailable())?
                .map_err(|_| unavailable())?
        },
        #[cfg(not(unix))]
        broker: None,
        helper_path: helper_path(),
    })
}

#[tauri::command]
pub async fn agent_control_enable(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Control>,
    enabled: bool,
) -> Result<(), String> {
    settings(&window)?;
    let _transition = state.transition.lock().await;
    #[cfg(unix)]
    {
        if enabled {
            if state.closing.load(Ordering::SeqCst) {
                return Err("Lomi is preparing to close.".into());
            }
            if !supported_host() {
                return Err("This host has not been qualified for agent control.".into());
            }
            if state.current()?.is_none() {
                let root = app
                    .path()
                    .app_data_dir()
                    .map_err(|_| unavailable())?
                    .join("agent-control");
                let broker = Broker::start(&root).map_err(|_| unavailable())?;
                #[cfg(target_os = "macos")]
                broker
                    .set_files_trash_dispatch(Arc::new(crate::files::agent::trash_staged))
                    .map_err(|_| unavailable())?;
                broker
                    .set_files_read_dispatch(Arc::new(crate::files::agent::decode))
                    .map_err(|_| unavailable())?;
                broker
                    .set_files_search_dispatch(Arc::new(crate::files::search::agent_search))
                    .map_err(|_| unavailable())?;
                let logs_app = app.clone();
                broker
                    .set_android_logcat_dispatch(Arc::new(move |control, input, deadline| {
                        crate::android::agent::logcat(&logs_app, control, input, deadline)
                    }))
                    .map_err(|_| unavailable())?;
                let install_app = app.clone();
                broker
                    .set_android_install_dispatch(Arc::new(move |request| {
                        crate::android::agent_install::install(&install_app, request)
                    }))
                    .map_err(|_| unavailable())?;
                let android_app = app.clone();
                broker
                    .set_android_capture_dispatch(Arc::new(move |control, input, deadline| {
                        crate::android::agent_capture::capture(
                            &android_app,
                            control,
                            input,
                            deadline,
                        )
                    }))
                    .map_err(|_| unavailable())?;
                let android_app = app.clone();
                broker
                    .set_android_snapshot_dispatch(Arc::new(
                        move |control, input, snapshot, deadline| {
                            crate::android::agent::snapshot(
                                &android_app,
                                control,
                                input,
                                snapshot,
                                deadline,
                            )
                        },
                    ))
                    .map_err(|_| unavailable())?;
                let setup_app = app.clone();
                broker
                    .set_android_setup_dispatch(Arc::new(move |input, devices, check| {
                        crate::android::agent_setup::prepare(&setup_app, input, devices, check)
                    }))
                    .map_err(|_| unavailable())?;
                let android_app = app.clone();
                broker
                    .set_android_list_dispatch(Arc::new(move |request| {
                        request.check()?;
                        let result = crate::android::agent::list(&android_app, &request.devices)?;
                        request.check()?;
                        Ok(result)
                    }))
                    .map_err(|_| unavailable())?;
                let terminals = app.state::<crate::terminal::Terminals>().inner().clone();
                broker
                    .set_terminal_dispatch(Arc::new(move |request| terminals.agent_run(request)))
                    .map_err(|_| unavailable())?;
                let terminals = app.state::<crate::terminal::Terminals>().inner().clone();
                broker
                    .set_terminal_input_dispatch(Arc::new(move |request| {
                        terminals.agent_input(request)
                    }))
                    .map_err(|_| unavailable())?;
                let terminals = app.state::<crate::terminal::Terminals>().inner().clone();
                broker
                    .set_terminal_close_dispatch(Arc::new(
                        move |generation, control, peers, commit| {
                            terminals.agent_close(generation, control, peers, commit)
                        },
                    ))
                    .map_err(|_| unavailable())?;
                let close_android_app = app.clone();
                broker
                    .set_android_close_dispatch(Arc::new(move |targets, permit| {
                        close_android_app
                            .state::<crate::android::manager::Android>()
                            .close_agent_views(targets, permit)
                    }))
                    .map_err(|_| unavailable())?;
                let close_chat_app = app.clone();
                broker
                    .set_chat_close_dispatch(Arc::new(move |project, conversations, permit| {
                        crate::chat::agent_close::close(
                            &close_chat_app,
                            project,
                            conversations,
                            permit,
                        )
                    }))
                    .map_err(|_| unavailable())?;
                let terminals = app.state::<crate::terminal::Terminals>().inner().clone();
                broker
                    .set_terminal_attach_dispatch(Arc::new(
                        move |generation, control, profile, peers| {
                            terminals.agent_attach(generation, control, profile, peers)
                        },
                    ))
                    .map_err(|_| unavailable())?;
                #[cfg(target_os = "macos")]
                {
                    let close_app = app.clone();
                    broker
                        .set_browser_close_dispatch(Arc::new(move |control| {
                            crate::browser::close_controlled(&close_app, control)
                        }))
                        .map_err(|_| unavailable())?;
                    broker
                        .set_files_read_dispatch(Arc::new(crate::files::agent::decode))
                        .map_err(|_| unavailable())?;
                    broker
                        .set_files_search_dispatch(Arc::new(crate::files::search::agent_search))
                        .map_err(|_| unavailable())?;
                    let logs_app = app.clone();
                    broker
                        .set_browser_logs_dispatch(Arc::new(
                            move |control, input, navigation, after, deadline| {
                                crate::browser::agent_dom::logs(
                                    &logs_app, control, input, navigation, after, deadline,
                                )
                            },
                        ))
                        .map_err(|_| unavailable())?;
                    let capture_app = app.clone();
                    broker
                        .set_browser_capture_dispatch(Arc::new(move |control, input, deadline| {
                            crate::browser::agent_capture::capture(
                                &capture_app,
                                control,
                                input,
                                deadline,
                            )
                        }))
                        .map_err(|_| unavailable())?;
                    let browser_app = app.clone();
                    broker
                        .set_browser_snapshot_dispatch(Arc::new(
                            move |control, input, snapshot, navigation, deadline| {
                                crate::browser::agent_dom::snapshot(
                                    &browser_app,
                                    control,
                                    input,
                                    snapshot,
                                    navigation,
                                    deadline,
                                )
                            },
                        ))
                        .map_err(|_| unavailable())?;
                }
                let dispatch_app = app.clone();
                let settings_app = app.clone();
                let settings_write_app = app.clone();
                broker
                    .set_settings_prepare_dispatch(Arc::new(
                        move |patch, current, check| match patch.section() {
                            lomi_control_protocol::settings::SettingsSection::Editor => {
                                crate::editor_preferences::agent_prepare(
                                    &settings_write_app,
                                    patch,
                                    check,
                                )
                            }
                            lomi_control_protocol::settings::SettingsSection::Terminal => {
                                crate::terminal_preferences::agent_prepare(
                                    &settings_write_app,
                                    patch,
                                    check,
                                )
                            }
                            lomi_control_protocol::settings::SettingsSection::Keybinds => {
                                crate::keybindings::agent_prepare(
                                    &settings_write_app,
                                    patch,
                                    current,
                                    check,
                                )
                            }
                            lomi_control_protocol::settings::SettingsSection::Themes => {
                                crate::themes::agent_prepare(&settings_write_app, patch, check)
                            }
                        },
                    ))
                    .map_err(|_| unavailable())?;
                broker
                    .set_settings_read_dispatch(Arc::new(move |request| {
                        settings_app
                            .emit_to("main", "agent-control-settings-read", request)
                            .map_err(std::io::Error::other)
                    }))
                    .map_err(|_| unavailable())?;
                let chat_list_app = app.clone();
                let chat_export_app = app.clone();
                broker
                    .set_chat_export_dispatch(Arc::new(move |project, input, check| {
                        crate::chat::agent_export::export(&chat_export_app, project, input, check)
                    }))
                    .map_err(|_| unavailable())?;
                broker
                    .set_chat_list_dispatch(Arc::new(move |project, ids, check| {
                        crate::chat::agent::list(&chat_list_app, project, ids, check)
                    }))
                    .map_err(|_| unavailable())?;
                let chat_send_app = app.clone();
                broker
                    .set_chat_send_prepare_dispatch(Arc::new(move |project, command, check| {
                        crate::chat::agent_send::prepare(&chat_send_app, project, command, check)
                    }))
                    .map_err(|_| unavailable())?;
                let chat_draft_app = app.clone();
                let chat_stop_app = app.clone();
                broker
                    .set_chat_stop_dispatch(Arc::new(move |project, input, check| {
                        crate::chat::agent_stop::stop(&chat_stop_app, project, input, check)
                    }))
                    .map_err(|_| unavailable())?;
                broker
                    .set_chat_draft_dispatch(Arc::new(move |project, input, check| {
                        crate::chat::agent::draft(&chat_draft_app, project, input, check)
                    }))
                    .map_err(|_| "Could not initialize Chat AI draft control.")?;
                let chat_open_app = app.clone();
                broker
                    .set_chat_open_dispatch(Arc::new(move |project, command, check| {
                        crate::chat::agent::open(&chat_open_app, project, command, check)
                    }))
                    .map_err(|_| unavailable())?;
                let chat_read_app = app.clone();
                broker
                    .set_chat_read_dispatch(Arc::new(move |project, input, check| {
                        crate::chat::agent::read(&chat_read_app, project, input, check)
                    }))
                    .map_err(|_| unavailable())?;
                let editor_app = app.clone();
                broker
                    .set_editor_read_dispatch(Arc::new(move |request| {
                        editor_app
                            .emit_to("main", "agent-control-editor-read", request)
                            .map_err(std::io::Error::other)
                    }))
                    .map_err(|_| unavailable())?;
                let screen_app = app.clone();
                broker
                    .set_screen_dispatch(Arc::new(move |request| {
                        screen_app
                            .emit_to("main", "agent-control-screen", request)
                            .map_err(std::io::Error::other)
                    }))
                    .map_err(|_| unavailable())?;
                broker
                    .set_ui_dispatch(Arc::new(move |command| {
                        dispatch_app
                            .emit_to("main", "agent-control-command", command)
                            .map_err(std::io::Error::other)
                    }))
                    .map_err(|_| unavailable())?;
                *state.broker.lock().map_err(|_| unavailable())? = Some(broker);
                app.emit_to("main", "agent-control-refresh", ())
                    .map_err(|_| unavailable())?;
            }
        } else {
            let broker = state.broker.lock().map_err(|_| unavailable())?.take();
            #[cfg(target_os = "macos")]
            crate::browser::native_input::clear(&app);
            if let Some(broker) = broker {
                broker.shutdown().await;
            }
            crate::browser::refresh_control_state(&app);
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (app, enabled);
        Err("This host has no qualified local control transport.".into())
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn agent_control_approve(
    window: Window,
    state: State<'_, Control>,
    request_id: String,
    workspace_ids: Vec<String>,
    scopes: Vec<String>,
    browser_origins: Option<Vec<String>>,
    android_devices: Option<Vec<String>>,
    android_packages: Option<Vec<String>>,
    chat_conversations: Option<Vec<String>>,
) -> Result<(), String> {
    settings(&window)?;
    #[cfg(unix)]
    {
        let broker = state.required()?;
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let devices = android_devices.unwrap_or_default();
            if !devices.is_empty() {
                let available = crate::android::agent::list(&app, &devices).map_err(|_| {
                    std::io::Error::other("Managed Android devices are unavailable.")
                })?;
                if devices
                    .iter()
                    .any(|id| !available.items.iter().any(|d| &d.device_id == id))
                {
                    return Err(std::io::Error::other(
                        "A selected Android device no longer exists.",
                    ));
                }
            }
            let conversations = chat_conversations.unwrap_or_default();
            if conversations.len() > 64
                || conversations
                    .iter()
                    .any(|id| !lomi_control_protocol::chat::valid_chat_id(id))
            {
                return Err(std::io::Error::other("Select at most 64 conversations."));
            }
            if !conversations.is_empty() {
                let overview = broker.overview()?;
                let projects: std::collections::HashSet<_> = overview
                    .workspaces
                    .iter()
                    .filter(|w| workspace_ids.contains(&w.id))
                    .map(|w| &w.project_id)
                    .collect();
                let mut found = std::collections::HashSet::new();
                for project in projects {
                    let items = crate::chat::agent::list(&app, project, &conversations, &|| Ok(()))
                        .map_err(|_| std::io::Error::other("Chat history is unavailable."))?;
                    found.extend(items.into_iter().map(|c| c.conversation_id));
                }
                if conversations.iter().any(|id| !found.contains(id)) {
                    return Err(std::io::Error::other(
                        "A selected conversation is no longer available in these projects.",
                    ));
                }
            }
            broker.approve_chat_access(
                &request_id,
                &workspace_ids,
                &scopes,
                &browser_origins.unwrap_or_default(),
                &devices,
                &android_packages.unwrap_or_default(),
                &conversations,
            )
        })
        .await
        .map_err(|_| unavailable())?
        .map_err(|e| e.to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = (
            state,
            request_id,
            workspace_ids,
            scopes,
            browser_origins,
            android_devices,
            android_packages,
            chat_conversations,
        );
        Err(unavailable())
    }
}
#[tauri::command]
pub async fn agent_control_chat_catalog(
    window: Window,
    project_id: String,
    after_id: Option<String>,
) -> Result<serde_json::Value, String> {
    settings(&window)?;
    #[cfg(unix)]
    {
        if !lomi_control_protocol::control::valid_id(&project_id)
            || after_id
                .as_ref()
                .is_some_and(|id| !lomi_control_protocol::chat::valid_chat_id(id))
        {
            return Err(unavailable());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            crate::chat::agent::catalog(&app, &project_id, after_id.as_deref()).map_err(|code| {
                if code == lomi_control_protocol::ErrorCode::TargetBusy {
                    "Chat history is busy. Try loading the conversations again."
                } else {
                    "Chat history is unavailable. Open Chat AI or resolve its storage error first."
                }
                .to_string()
            })
        })
        .await
        .map_err(|_| unavailable())?
    }
    #[cfg(not(unix))]
    {
        let _ = (project_id, after_id);
        Err(unavailable())
    }
}
#[tauri::command]
pub async fn agent_control_reject(
    window: Window,
    state: State<'_, Control>,
    request_id: String,
) -> Result<(), String> {
    settings(&window)?;
    #[cfg(unix)]
    {
        let broker = state.required()?;
        tauri::async_runtime::spawn_blocking(move || broker.reject(&request_id))
            .await
            .map_err(|_| unavailable())?;
    }
    #[cfg(not(unix))]
    let _ = (state, request_id);
    Ok(())
}
#[tauri::command]
pub fn agent_control_revoke(window: Window, app: tauri::AppHandle) -> Result<(), String> {
    if !matches!(window.label(), "main" | "settings") {
        return Err(unavailable());
    }
    revoke(&app);
    Ok(())
}
pub fn revoke(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    crate::browser::native_input::clear(app);
    #[cfg(unix)]
    if let Some(state) = app.try_state::<Control>() {
        if let Ok(Some(broker)) = state.current() {
            broker.revoke();
        }
    }
    crate::browser::refresh_control_state(app);
    let _ = app.emit_to("settings", "agent-control-changed", ());
}

pub async fn shutdown(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    crate::browser::native_input::clear(app);
    #[cfg(unix)]
    if let Some(state) = app.try_state::<Control>() {
        let broker = state
            .broker
            .lock()
            .ok()
            .and_then(|mut broker| broker.take());
        if let Some(broker) = broker {
            broker.shutdown().await;
        }
    }
}

#[tauri::command]
pub async fn agent_control_closing(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Control>,
    closing: bool,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    let _transition = state.transition.lock().await;
    state.closing.store(closing, Ordering::SeqCst);
    if closing {
        revoke(&app);
        #[cfg(unix)]
        if let Some(broker) = state.current()? {
            broker.pause_native_workers().await;
        }
    } else {
        #[cfg(unix)]
        if let Some(broker) = state.current()? {
            broker.resume_native_workers();
        }
        app.emit_to("main", "agent-control-refresh", ())
            .map_err(|_| unavailable())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn agent_control_ui_register(
    window: Window,
    state: State<'_, Control>,
) -> Result<Option<String>, String> {
    crate::files::main_window(&window)?;
    if state.closing.load(Ordering::SeqCst) {
        return Ok(None);
    }
    #[cfg(unix)]
    {
        let broker = state.current()?;
        tauri::async_runtime::spawn_blocking(move || broker.map(|b| b.register_ui()).transpose())
            .await
            .map_err(|_| unavailable())?
            .map_err(|_| unavailable())
    }
    #[cfg(not(unix))]
    {
        let _ = state;
        Ok(None)
    }
}
#[tauri::command]
pub async fn agent_control_ui_publish(
    window: Window,
    shells: State<'_, crate::terminal::Shells>,
    state: State<'_, Control>,
    mut projection: Projection,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    #[cfg(unix)]
    {
        let broker = state.required()?;
        projection.terminal_profile = qualified_profile(&shells);
        tauri::async_runtime::spawn_blocking(move || {
            let epoch = projection.ui_epoch.clone();
            if projection.workspaces.len() > 500 {
                broker.invalidate_ui_epoch(Some(&epoch));
                return Err("Agent control supports at most 500 published workspaces.".into());
            }
            for workspace in &mut projection.workspaces {
                workspace.project_path = std::fs::canonicalize(&workspace.project_path)
                    .map_err(|_| {
                        broker.invalidate_ui_epoch(Some(&epoch));
                        "An agent workspace folder is unavailable."
                    })?
                    .to_string_lossy()
                    .into_owned();
            }
            broker.publish(projection).map_err(|_| {
                broker.invalidate_ui_epoch(Some(&epoch));
                unavailable()
            })
        })
        .await
        .map_err(|_| unavailable())?
    }
    #[cfg(not(unix))]
    {
        let _ = (state, projection);
        Err(unavailable())
    }
}

pub fn page_load(view: &tauri::Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if view.label() == "main" && matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
        #[cfg(unix)]
        if let Some(state) = view.app_handle().try_state::<Control>() {
            if let Ok(Some(broker)) = state.current() {
                broker.invalidate_ui();
            }
        }
    }
}

#[tauri::command]
pub async fn agent_control_ui_claim(
    window: Window,
    state: State<'_, Control>,
    ui_epoch: String,
    operation_id: String,
    nonce: String,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    #[cfg(unix)]
    {
        let broker = state.required()?;
        tauri::async_runtime::spawn_blocking(move || {
            broker.claim_ui(&ui_epoch, &operation_id, &nonce)
        })
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())
    }
    #[cfg(not(unix))]
    {
        let _ = (state, ui_epoch, operation_id, nonce);
        Err(unavailable())
    }
}
#[tauri::command]
pub async fn agent_control_ui_ack(
    window: Window,
    state: State<'_, Control>,
    ack: UiAck,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    #[cfg(unix)]
    {
        let broker = state.required()?;
        tauri::async_runtime::spawn_blocking(move || {
            #[cfg(all(feature = "mcp-probe", target_os = "macos"))]
            let trace = matches!(ack.result, lomi_control_protocol::control::OperationResult::WorkspaceClosure(_) | lomi_control_protocol::control::OperationResult::ProjectClosure(_) | lomi_control_protocol::control::OperationResult::Failure { .. }).then(|| serde_json::json!({"stage":"ack","operation":ack.operation_id,"result":ack.result}));
            let result = broker.acknowledge_ui(ack);
            #[cfg(all(feature = "mcp-probe", target_os = "macos"))]
            if let Some(mut trace) = trace { trace["accepted"] = serde_json::json!(result.is_ok()); crate::mcp_control_probe::record_close(trace); }
            result
        })
            .await
            .map_err(|_| unavailable())?
            .map_err(|_| unavailable())
    }
    #[cfg(not(unix))]
    {
        let _ = (state, ack);
        Err(unavailable())
    }
}

#[cfg(unix)]
pub(crate) fn qualified_profile(
    shells: &crate::terminal::Shells,
) -> Option<lomi_control_protocol::control::TerminalProfile> {
    let profile = shells
        .profiles
        .iter()
        .find(|p| p.kind == "zsh" && p.program == "/bin/zsh" && p.distro.is_none())?;
    Some(lomi_control_protocol::control::TerminalProfile {
        id: profile.id.clone(),
        revision: lomi_control_core::broker::certificate_hash(&serde_json::to_vec(profile).ok()?),
    })
}

#[tauri::command]
pub async fn agent_control_workspace_close_pending(
    window: Window,
    state: State<'_, Control>,
    ui_epoch: String,
    operation_id: String,
    nonce: String,
) -> Result<bool, String> {
    if crate::files::main_window(&window).is_err() {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        let Ok(broker) = state.required() else {
            return Ok(false);
        };
        Ok(tauri::async_runtime::spawn_blocking(move || {
            broker.workspace_close_pending(&operation_id, &nonce, &ui_epoch)
        })
        .await
        .unwrap_or(false))
    }
    #[cfg(not(unix))]
    {
        let _ = (state, ui_epoch, operation_id, nonce);
        Ok(false)
    }
}

#[tauri::command]
pub async fn agent_control_ui_commit_close(
    window: Window,
    state: State<'_, Control>,
    ui_epoch: String,
    operation_id: String,
    nonce: String,
) -> Result<(), lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(unix)]
    {
        let broker = state.required().map_err(|_| ErrorCode::ControlRevoked)?;
        tauri::async_runtime::spawn_blocking(move || {
            let result = broker.commit_panel_close(&operation_id, &nonce, &ui_epoch);
            #[cfg(all(feature = "mcp-probe", target_os = "macos"))]
            crate::mcp_control_probe::record_close(serde_json::json!({"stage":"native-close","operation":operation_id,"error":result.as_ref().err()}));
            result
        })
        .await
        .map_err(|_| ErrorCode::OutcomeUnknown)?
    }
    #[cfg(not(unix))]
    {
        let _ = (state, ui_epoch, operation_id, nonce);
        Err(ErrorCode::HostUnqualified)
    }
}

#[tauri::command]
pub fn agent_control_file_trash_prepare(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
    buffers: Vec<lomi_control_protocol::files::FileTrashBuffer>,
) -> Result<lomi_control_protocol::files::FileTrashPlan, lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(unix)]
    {
        state
            .required()
            .map_err(|_| ErrorCode::ControlRevoked)?
            .prepare_file_trash(&operation_id, &nonce, buffers)
    }
    #[cfg(not(unix))]
    {
        let _ = (state, operation_id, nonce, buffers);
        Err(ErrorCode::HostUnqualified)
    }
}

#[tauri::command]
pub fn agent_control_file_trash_pending(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
    plan_hash: String,
) -> bool {
    if crate::files::main_window(&window).is_err() {
        return false;
    }
    #[cfg(unix)]
    {
        state
            .required()
            .ok()
            .is_some_and(|broker| broker.file_trash_pending(&operation_id, &nonce, &plan_hash))
    }
    #[cfg(not(unix))]
    {
        let _ = (state, operation_id, nonce, plan_hash);
        false
    }
}

#[tauri::command]
pub fn agent_control_file_trash_decide(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
    plan_hash: String,
    approved: bool,
) -> Result<(), lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(unix)]
    {
        state
            .required()
            .map_err(|_| ErrorCode::ControlRevoked)?
            .decide_file_trash(&operation_id, &nonce, &plan_hash, approved)
    }
    #[cfg(not(unix))]
    {
        let _ = (state, operation_id, nonce, plan_hash, approved);
        Err(ErrorCode::HostUnqualified)
    }
}

#[tauri::command]
pub async fn agent_control_artifact_export(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
) -> Result<lomi_control_protocol::artifact::ArtifactExported, lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(unix)]
    {
        let broker = state.required().map_err(|_| ErrorCode::ControlRevoked)?;
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            crate::files::agent::with_writer(&app, || {
                broker.commit_artifact_export(&operation_id, &nonce)
            })
        })
        .await
        .map_err(|_| ErrorCode::OutcomeUnknown)?
    }
    #[cfg(not(unix))]
    {
        let _ = (state, operation_id, nonce);
        Err(ErrorCode::HostUnqualified)
    }
}

#[tauri::command]
pub async fn agent_control_files_mutate(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
) -> Result<lomi_control_protocol::files::FilesMutated, lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(unix)]
    {
        let broker = state.required().map_err(|_| ErrorCode::ControlRevoked)?;
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            crate::files::agent::with_writer(&app, || {
                broker.commit_files_mutate(&operation_id, &nonce)
            })
        })
        .await
        .map_err(|_| ErrorCode::OutcomeUnknown)?
    }
    #[cfg(not(unix))]
    {
        let _ = (state, operation_id, nonce);
        Err(ErrorCode::HostUnqualified)
    }
}

#[tauri::command]
pub async fn agent_control_editor_save_file(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
    body: lomi_control_protocol::editor::EditorSaveBody,
) -> Result<lomi_control_protocol::editor::EditorSaved, lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(unix)]
    {
        let broker = state.required().map_err(|_| ErrorCode::ControlRevoked)?;
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            crate::files::agent::with_writer(&app, || {
                broker.commit_editor_save(
                    &operation_id,
                    &nonce,
                    body,
                    crate::files::agent::encode_preserving,
                )
            })
        })
        .await
        .map_err(|_| ErrorCode::OutcomeUnknown)?
    }
    #[cfg(not(unix))]
    {
        let _ = (state, operation_id, nonce, body);
        Err(ErrorCode::HostUnqualified)
    }
}

#[tauri::command]
pub async fn agent_control_chat_send_prepare(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
) -> Result<lomi_control_protocol::chat::ChatSendPlan, lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(unix)]
    {
        let broker = state.required().map_err(|_| ErrorCode::ControlRevoked)?;
        tauri::async_runtime::spawn_blocking(move || {
            broker.prepare_chat_send(&operation_id, &nonce)
        })
        .await
        .map_err(|_| ErrorCode::OutcomeUnknown)?
    }
    #[cfg(not(unix))]
    {
        let _ = (state, operation_id, nonce);
        Err(ErrorCode::HostUnqualified)
    }
}

#[tauri::command]
pub fn agent_control_chat_send_pending(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
    plan_hash: String,
) -> Result<bool, lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(unix)]
    {
        Ok(state
            .required()
            .map_err(|_| ErrorCode::ControlRevoked)?
            .chat_send_pending(&operation_id, &nonce, &plan_hash))
    }
    #[cfg(not(unix))]
    {
        let _ = (state, operation_id, nonce, plan_hash);
        Err(ErrorCode::HostUnqualified)
    }
}

#[tauri::command]
pub async fn agent_control_chat_send_decide(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
    plan_hash: String,
    approved: bool,
) -> Result<(), lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(unix)]
    {
        let broker = state.required().map_err(|_| ErrorCode::ControlRevoked)?;
        tauri::async_runtime::spawn_blocking(move || {
            broker.decide_chat_send(&operation_id, &nonce, &plan_hash, approved)
        })
        .await
        .map_err(|_| ErrorCode::OutcomeUnknown)?
    }
    #[cfg(not(unix))]
    {
        let _ = (state, operation_id, nonce, plan_hash, approved);
        Err(ErrorCode::HostUnqualified)
    }
}

#[tauri::command]
pub async fn agent_control_chat_send(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
    plan_hash: String,
    channel: tauri::ipc::Channel<serde_json::Value>,
) -> Result<serde_json::Value, lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(unix)]
    {
        let broker = state.required().map_err(|_| ErrorCode::ControlRevoked)?;
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let reply = crate::chat::agent_send::commit(
                &app,
                &broker,
                &operation_id,
                &nonce,
                &plan_hash,
                channel,
            )?;
            serde_json::to_value(reply).map_err(|_| ErrorCode::OutcomeUnknown)
        })
        .await
        .map_err(|_| ErrorCode::OutcomeUnknown)?
    }
    #[cfg(not(unix))]
    {
        let _ = (state, operation_id, nonce, plan_hash, channel);
        Err(ErrorCode::HostUnqualified)
    }
}

#[tauri::command]
pub async fn agent_control_chat_draft(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
) -> Result<lomi_control_protocol::chat::ChatDraftUpdated, lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(unix)]
    {
        let broker = state.required().map_err(|_| ErrorCode::ControlRevoked)?;
        tauri::async_runtime::spawn_blocking(move || {
            broker.commit_chat_draft(&operation_id, &nonce)
        })
        .await
        .map_err(|_| ErrorCode::OutcomeUnknown)?
    }
    #[cfg(not(unix))]
    {
        let _ = (state, operation_id, nonce);
        Err(ErrorCode::HostUnqualified)
    }
}

#[tauri::command]
pub async fn agent_control_chat_open(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
) -> Result<lomi_control_protocol::chat::ChatSummary, lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(unix)]
    {
        let broker = state.required().map_err(|_| ErrorCode::ControlRevoked)?;
        tauri::async_runtime::spawn_blocking(move || {
            broker.prepare_chat_open(&operation_id, &nonce)
        })
        .await
        .map_err(|_| ErrorCode::OutcomeUnknown)?
    }
    #[cfg(not(unix))]
    {
        let _ = (state, operation_id, nonce);
        Err(ErrorCode::HostUnqualified)
    }
}

#[tauri::command]
pub async fn agent_control_editor_open_file(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
) -> Result<lomi_control_protocol::editor::PreparedEditorFile, lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(unix)]
    {
        let broker = state.required().map_err(|_| ErrorCode::ControlRevoked)?;
        tauri::async_runtime::spawn_blocking(move || {
            broker.prepare_editor_open(
                &operation_id,
                &nonce,
                crate::files::agent_image::prepare_editor,
            )
        })
        .await
        .map_err(|_| ErrorCode::OutcomeUnknown)?
    }
    #[cfg(not(unix))]
    {
        let _ = (state, operation_id, nonce);
        Err(ErrorCode::HostUnqualified)
    }
}

#[tauri::command]
pub async fn agent_control_preview_asset(
    window: Window,
    state: State<'_, Control>,
    permit_id: String,
    relative: String,
) -> Result<lomi_control_protocol::editor::PreparedPreviewImage, lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(unix)]
    {
        let broker = state.required().map_err(|_| ErrorCode::ControlRevoked)?;
        tauri::async_runtime::spawn_blocking(move || {
            broker.read_preview_asset(
                &permit_id,
                &relative,
                crate::files::agent_image::prepare_asset,
            )
        })
        .await
        .map_err(|_| ErrorCode::OutcomeUnknown)?
    }
    #[cfg(not(unix))]
    {
        let _ = (state, permit_id, relative);
        Err(ErrorCode::HostUnqualified)
    }
}
#[tauri::command]
pub fn agent_control_preview_release(
    window: Window,
    state: State<'_, Control>,
    permit_id: String,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    #[cfg(unix)]
    {
        if let Ok(broker) = state.required() {
            broker.release_preview_permit(&permit_id);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (state, permit_id);
    }
    Ok(())
}

#[tauri::command]
pub fn agent_control_editor_read_reply(
    window: Window,
    state: State<'_, Control>,
    reply: lomi_control_protocol::editor::EditorReadReply,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    #[cfg(unix)]
    {
        state
            .required()?
            .editor_read_reply(reply)
            .map_err(|_| unavailable())
    }
    #[cfg(not(unix))]
    {
        let _ = (state, reply);
        Err(unavailable())
    }
}

#[tauri::command]
pub fn agent_control_settings_read_reply(
    window: Window,
    state: State<'_, Control>,
    reply: lomi_control_protocol::settings::SettingsReadReply,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    #[cfg(unix)]
    {
        state
            .required()?
            .settings_read_reply(reply)
            .map_err(|_| unavailable())
    }
    #[cfg(not(unix))]
    {
        let _ = (state, reply);
        Err(unavailable())
    }
}

#[tauri::command]
pub fn agent_control_terminal_screen_reply(
    window: Window,
    state: State<'_, Control>,
    reply: lomi_control_protocol::control::ScreenReply,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    #[cfg(unix)]
    {
        state
            .required()?
            .screen_reply(reply)
            .map_err(|_| unavailable())
    }
    #[cfg(not(unix))]
    {
        let _ = (state, reply);
        Err(unavailable())
    }
}

#[tauri::command]
pub async fn agent_control_decide_terminal(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    approve: bool,
) -> Result<(), String> {
    settings(&window)?;
    #[cfg(unix)]
    {
        let broker = state.required()?;
        tauri::async_runtime::spawn_blocking(move || broker.decide_control(&operation_id, approve))
            .await
            .map_err(|_| unavailable())?
            .map_err(|_| "The terminal control request is no longer available.".into())
    }
    #[cfg(not(unix))]
    {
        let _ = (state, operation_id, approve);
        Err(unavailable())
    }
}

#[tauri::command]
pub async fn agent_browser_upload_prepare(
    window: Window,
    operation_id: String,
    nonce: String,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    #[cfg(target_os = "macos")]
    {
        let app = window.app_handle().clone();
        let broker = app.state::<Control>().required()?;
        let native = app.clone();
        let permit = tauri::async_runtime::spawn_blocking(move || {
            broker.prepare_browser_upload(&operation_id, &nonce, |control, input, permit| {
                crate::browser::agent_dom::prepare_upload(&native, control, input, permit)
            })
        })
        .await
        .map_err(|_| "OUTCOME_UNKNOWN".to_string())?
        .map_err(|code| {
            serde_json::to_value(code)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })?;
        crate::settings_window::request_checked(&app, Some("agent-control".into()), || {
            permit.check().map_err(|_| "CONTROL_REVOKED".into())
        })
        .await
        .map_err(|_| "OUTCOME_UNKNOWN".into())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (operation_id, nonce);
        Err("HOST_UNQUALIFIED".into())
    }
}
#[tauri::command]
pub async fn agent_browser_upload_decide(
    window: Window,
    operation_id: String,
    approve: bool,
) -> Result<(), String> {
    settings(&window)?;
    #[cfg(target_os = "macos")]
    {
        let app = window.app_handle().clone();
        let broker = app.state::<Control>().required()?;
        tauri::async_runtime::spawn_blocking(move || {
            broker.decide_browser_upload(&operation_id, approve, |approval| {
                crate::browser::agent_dom::upload(&app, approval)
            })
        })
        .await
        .map_err(|_| "OUTCOME_UNKNOWN".to_string())?
        .map_err(|code| {
            serde_json::to_value(code)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (operation_id, approve);
        Err("HOST_UNQUALIFIED".into())
    }
}

#[tauri::command]
pub async fn agent_browser_download(
    window: Window,
    operation_id: String,
    nonce: String,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    #[cfg(target_os = "macos")]
    {
        let app = window.app_handle().clone();
        let broker = app.state::<Control>().required()?;
        tauri::async_runtime::spawn_blocking(move || {
            broker.execute_browser_download(&operation_id, &nonce, |control, input, permit| {
                crate::browser::agent_dom::download(&app, control, input, permit)
            })
        })
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (operation_id, nonce);
        Err(unavailable())
    }
}

#[tauri::command]
pub async fn agent_artifact_import(
    window: Window,
    operation_id: String,
    nonce: String,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    #[cfg(unix)]
    {
        let broker = window.app_handle().state::<Control>().required()?;
        // Retained native work owns cancellation and its receipt. Suspending the
        // renderer after dispatch cannot claim completion or replay the import.
        tauri::async_runtime::spawn_blocking(move || {
            broker.execute_import(&operation_id, &nonce, crate::android::agent_apk::inspect)
        })
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())
    }
    #[cfg(not(unix))]
    {
        let _ = (operation_id, nonce);
        Err(unavailable())
    }
}

#[tauri::command]
pub async fn agent_control_decide_android_management(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    revision: String,
    approve: bool,
    accepted: Vec<String>,
    confirmation: Option<String>,
) -> Result<(), String> {
    settings(&window)?;
    #[cfg(unix)]
    {
        let broker = state.required()?;
        tauri::async_runtime::spawn_blocking(move || {
            broker.decide_android_management(
                &operation_id,
                &revision,
                approve,
                accepted,
                confirmation,
            )
        })
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| "The Android request changed or is no longer available.".into())
    }
    #[cfg(not(unix))]
    {
        let _ = (
            state,
            operation_id,
            revision,
            approve,
            accepted,
            confirmation,
        );
        Err(unavailable())
    }
}

#[tauri::command]
pub async fn agent_control_decide_install(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    approve: bool,
) -> Result<(), String> {
    settings(&window)?;
    #[cfg(unix)]
    {
        let broker = state.required()?;
        tauri::async_runtime::spawn_blocking(move || broker.decide_install(&operation_id, approve))
            .await
            .map_err(|_| unavailable())?
            .map_err(|_| "The APK installation request is no longer available.".into())
    }
    #[cfg(not(unix))]
    {
        let _ = (state, operation_id, approve);
        Err(unavailable())
    }
}

#[tauri::command]
pub async fn agent_control_git_mutation_prepare(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
) -> Result<lomi_control_protocol::git::GitMutationPlan, lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let broker = state.required().map_err(|_| ErrorCode::ControlRevoked)?;
        broker
            .native_worker(move |broker| {
                broker.prepare_git_mutation(&operation_id, &nonce, |root| {
                    crate::git::mutation_guard(root).map_err(|_| ErrorCode::TargetBusy)
                })
            })
            .await?
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        let _ = (state, operation_id, nonce);
        Err(ErrorCode::HostUnqualified)
    }
}
#[tauri::command]
pub fn agent_control_git_mutation_pending(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
    plan_hash: String,
) -> bool {
    if crate::files::main_window(&window).is_err() {
        return false;
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        state
            .required()
            .ok()
            .is_some_and(|broker| broker.git_mutation_pending(&operation_id, &nonce, &plan_hash))
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        let _ = (state, operation_id, nonce, plan_hash);
        false
    }
}
#[tauri::command]
pub async fn agent_control_git_mutation_decide(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
    plan_hash: String,
    approved: bool,
) -> Result<(), lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        state
            .required()
            .map_err(|_| ErrorCode::ControlRevoked)?
            .native_worker(move |broker| {
                broker.decide_git_mutation(&operation_id, &nonce, &plan_hash, approved)
            })
            .await?
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        let _ = (state, operation_id, nonce, plan_hash, approved);
        Err(ErrorCode::HostUnqualified)
    }
}
#[tauri::command]
pub async fn agent_control_git_mutation_commit(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
    plan_hash: String,
) -> Result<lomi_control_protocol::git::GitMutated, lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        state
            .required()
            .map_err(|_| ErrorCode::ControlRevoked)?
            .native_worker(move |broker| {
                broker.commit_git_mutation(&operation_id, &nonce, &plan_hash, |root| {
                    crate::git::mutation_guard(root).map_err(|_| ErrorCode::TargetBusy)
                })
            })
            .await?
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        let _ = (state, operation_id, nonce, plan_hash);
        Err(ErrorCode::HostUnqualified)
    }
}
#[tauri::command]
pub async fn agent_control_git_open(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    nonce: String,
) -> Result<lomi_control_protocol::git::PreparedGitView, lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let broker = state.required().map_err(|_| ErrorCode::ControlRevoked)?;
        tauri::async_runtime::spawn_blocking(move || broker.prepare_git_open(&operation_id, &nonce))
            .await
            .map_err(|_| ErrorCode::OutcomeUnknown)?
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        let _ = (state, operation_id, nonce);
        Err(ErrorCode::HostUnqualified)
    }
}
#[tauri::command]
pub async fn agent_control_git_read(
    window: Window,
    state: State<'_, Control>,
    permit_id: String,
    relative: Option<String>,
) -> Result<lomi_control_protocol::git::GitViewBody, lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let broker = state.required().map_err(|_| ErrorCode::ControlRevoked)?;
        tauri::async_runtime::spawn_blocking(move || {
            broker.read_git_view(&permit_id, relative.as_deref())
        })
        .await
        .map_err(|_| ErrorCode::OutcomeUnknown)?
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        let _ = (state, permit_id, relative);
        Err(ErrorCode::HostUnqualified)
    }
}
#[tauri::command]
pub fn agent_control_git_release(
    window: Window,
    state: State<'_, Control>,
    permit_id: String,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        state.required()?.release_git_view(&permit_id);
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        let _ = (state, permit_id);
    }
    Ok(())
}

#[tauri::command]
pub async fn agent_control_project_open_decide(
    window: Window,
    state: State<'_, Control>,
    operation_id: String,
    approved: bool,
) -> Result<(), lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    settings(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(unix)]
    {
        let broker = state.required().map_err(|_| ErrorCode::ControlRevoked)?;
        tauri::async_runtime::spawn_blocking(move || {
            broker.decide_project_open(&operation_id, approved)
        })
        .await
        .map_err(|_| ErrorCode::OutcomeUnknown)?
    }
    #[cfg(not(unix))]
    {
        let _ = (state, operation_id, approved);
        Err(ErrorCode::HostUnqualified)
    }
}
#[tauri::command]
pub async fn agent_control_project_open_ready(
    window: Window,
    state: State<'_, Control>,
    ui_epoch: String,
    operation_id: String,
    nonce: String,
) -> Result<bool, lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(unix)]
    {
        let broker = state.required().map_err(|_| ErrorCode::ControlRevoked)?;
        tauri::async_runtime::spawn_blocking(move || {
            broker.project_open_ready(&operation_id, &nonce, &ui_epoch)
        })
        .await
        .map_err(|_| ErrorCode::OutcomeUnknown)?
    }
    #[cfg(not(unix))]
    {
        let _ = (state, ui_epoch, operation_id, nonce);
        Err(ErrorCode::HostUnqualified)
    }
}
#[tauri::command]
pub async fn agent_control_project_open_commit(
    window: Window,
    state: State<'_, Control>,
    ui_epoch: String,
    operation_id: String,
    nonce: String,
) -> Result<(), lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    crate::files::main_window(&window).map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(unix)]
    {
        let broker = state.required().map_err(|_| ErrorCode::ControlRevoked)?;
        tauri::async_runtime::spawn_blocking(move || {
            broker.commit_project_open(&operation_id, &nonce, &ui_epoch)
        })
        .await
        .map_err(|_| ErrorCode::OutcomeUnknown)?
    }
    #[cfg(not(unix))]
    {
        let _ = (state, ui_epoch, operation_id, nonce);
        Err(ErrorCode::HostUnqualified)
    }
}
