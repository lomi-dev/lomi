use super::{
    bootstrap, catalog, devices,
    events::Event,
    host,
    installation::Manifest,
    installer::{Plan, Progress, Selection},
    manager::{Android, Manager as AndroidManager},
    repository,
    runtime::Status,
    storage::{Devices, Preferences},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};
use tauri::{Emitter, Manager, State, Window};

fn read_window(window: &Window) -> Result<(), String> {
    if matches!(window.label(), "main" | "settings") {
        Ok(())
    } else {
        Err("Android commands are available only to the application views".into())
    }
}

fn settings_window(window: &Window) -> Result<(), String> {
    if window.label() == "settings" {
        Ok(())
    } else {
        Err("Only Android settings may change managed components and devices".into())
    }
}

fn backend(window: &Window, state: &Android) -> Result<Arc<AndroidManager>, String> {
    read_window(window)?;
    let root = window
        .app_handle()
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?
        .join("android");
    #[cfg(feature = "android-probe")]
    let root = super::fixture::directory()?.unwrap_or(root);
    let manager = state.get(root)?;
    let app = window.app_handle().clone();
    manager.set_emitter(move |event| {
        for label in ["main", "settings"] {
            let _ = app.emit_to(
                tauri::EventTarget::webview(label),
                "android-changed",
                &event,
            );
        }
    });
    Ok(manager)
}

#[tauri::command]
pub fn android_prepare_setup(
    window: Window,
    requests: State<'_, super::open::Requests>,
    workspace_id: String,
    panel_id: String,
) -> Result<super::open::Context, String> {
    crate::files::main_window(&window)?;
    let context = requests.prepare(workspace_id, panel_id)?;
    window
        .app_handle()
        .emit_to(
            tauri::EventTarget::webview("settings"),
            "android-setup-changed",
            &context,
        )
        .map_err(|e| e.to_string())?;
    Ok(context)
}

#[tauri::command]
pub fn android_setup_context(
    window: Window,
    requests: State<'_, super::open::Requests>,
) -> Result<Option<super::open::Context>, String> {
    settings_window(&window)?;
    requests.context()
}

#[tauri::command]
pub async fn android_request_open(
    window: Window,
    state: State<'_, Android>,
    requests: State<'_, super::open::Requests>,
    request_id: Option<String>,
    device_id: String,
    cold_boot: bool,
) -> Result<(), String> {
    settings_window(&window)?;
    host::require_qualification()?;
    backend(&window, &state)?.can_open(&device_id)?;
    let (intent, receive) = requests.request(request_id, device_id, cold_boot)?;
    let result = async {
        window
            .app_handle()
            .emit_to(
                tauri::EventTarget::webview("main"),
                "android-open-request",
                &intent,
            )
            .map_err(|e| e.to_string())?;
        tokio::time::timeout(std::time::Duration::from_secs(15), receive)
            .await
            .map_err(|_| {
                "The workspace did not respond. Return to the main window and retry.".to_string()
            })?
            .map_err(|_| "The workspace closed before opening Android".to_string())?
    }
    .await;
    requests.expired(&intent.id);
    if result.is_ok() {
        if let Some(main) = window.app_handle().get_window("main") {
            main.show().map_err(|e| e.to_string())?;
            main.set_focus().map_err(|e| e.to_string())?;
        }
    }
    result
}

#[tauri::command]
pub fn android_open_result(
    window: Window,
    requests: State<'_, super::open::Requests>,
    id: String,
    error: Option<String>,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    if error.as_ref().is_some_and(|error| error.len() > 4096) {
        return Err("Android open error exceeds its limit".into());
    }
    requests.complete(&id, error.map_or(Ok(()), Err))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    host: String,
    qualified: bool,
    acceleration: host::Acceleration,
    sdk_path: String,
    adb_path: Option<String>,
    toolchain_ready: bool,
    preferences: Option<Preferences>,
    devices: Option<Devices>,
    packages: Option<Manifest>,
    profiles: Vec<catalog::Profile>,
    errors: BTreeMap<&'static str, String>,
    statuses: Vec<Status>,
    operation: Option<Progress>,
    streams: Vec<super::frames::Status>,
    rollbacks: Vec<super::installation::Installed>,
    recovery: Vec<super::recovery::Plan>,
    required_tools: [&'static str; 3],
    toolchain: bootstrap::Distribution,
    toolchain_update_available: bool,
}

fn capture<T>(
    errors: &mut BTreeMap<&'static str, String>,
    name: &'static str,
    result: Result<T, String>,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            errors.insert(name, error);
            None
        }
    }
}

#[tauri::command]
pub async fn android_state(window: Window, state: State<'_, Android>) -> Result<Snapshot, String> {
    let manager = backend(&window, &state)?;
    let directory = manager
        .directory
        .lock()
        .map_err(|_| "Android directory failed")?;
    let mut errors = BTreeMap::new();
    let host = catalog::Host::native()?;
    let packages = capture(&mut errors, "packages", directory.manifest());
    let toolchain_ready = if directory.root.join("toolchain.json").exists() {
        capture(&mut errors, "toolchain", bootstrap::installed(&directory)).is_some()
    } else {
        false
    };
    let target_toolchain = bootstrap::distribution(host)?;
    let installed_toolchain = capture(
        &mut errors,
        "toolchain",
        bootstrap::installed_distribution(&directory),
    )
    .flatten();
    let toolchain_update_available = installed_toolchain.as_ref() != Some(&target_toolchain);
    let profiles = if packages
        .as_ref()
        .is_some_and(|manifest| manifest.packages.contains_key(devices::TOOLS))
    {
        capture(&mut errors, "profiles", devices::profiles(&directory)).unwrap_or_default()
    } else {
        vec![]
    };
    let adb_path = if packages
        .as_ref()
        .is_some_and(|manifest| manifest.packages.contains_key("platform-tools"))
    {
        capture(
            &mut errors,
            "adb",
            directory.installed_path("platform-tools"),
        )
        .map(|path| {
            path.join(if cfg!(windows) { "adb.exe" } else { "adb" })
                .to_string_lossy()
                .into_owned()
        })
    } else {
        None
    };
    for name in [
        "package-removal.json",
        "installation.json",
        "toolchain-installation.json",
        "device-operation.json",
    ] {
        if directory.root.join(name).exists() {
            errors.insert("recovery", "An Android operation was interrupted. Use Repair to recover its publication journal.".into());
        }
    }
    let rollbacks = capture(
        &mut errors,
        "rollbacks",
        super::maintenance::rollbacks(&directory),
    )
    .unwrap_or_default();
    let mut snapshot = Snapshot {
        required_tools: ["emulator", "platform-tools", devices::TOOLS],
        toolchain: installed_toolchain.unwrap_or(target_toolchain),
        toolchain_update_available,
        recovery: capture(
            &mut errors,
            "metadataRecovery",
            super::recovery::plans(&directory),
        )
        .unwrap_or_default(),
        host: bootstrap::host_key(host).into(),
        qualified: bootstrap::distribution(host)?.qualified,
        acceleration: host::acceleration()?,
        sdk_path: directory.root.join("sdk").to_string_lossy().into_owned(),
        adb_path,
        toolchain_ready,
        preferences: capture(&mut errors, "preferences", directory.preferences()),
        devices: capture(&mut errors, "devices", directory.devices()),
        packages,
        profiles,
        errors,
        rollbacks,
        statuses: vec![],
        operation: None,
        streams: manager.streams.statuses()?,
    };
    drop(directory);
    let statuses = match manager.statuses() {
        Ok(statuses) => statuses,
        Err(error) => {
            snapshot.errors.insert("runtimeRecovery", error);
            manager.current_statuses()?
        }
    };
    Ok(Snapshot {
        statuses,
        operation: manager.installer.progress(),
        ..snapshot
    })
}

#[tauri::command]
pub async fn android_catalog(
    window: Window,
    state: State<'_, Android>,
    refresh: bool,
) -> Result<repository::Snapshot, String> {
    backend(&window, &state)?.installer.catalog(refresh).await
}

#[tauri::command]
pub async fn android_storage(
    window: Window,
    state: State<'_, Android>,
) -> Result<super::disk::Usage, String> {
    let manager = backend(&window, &state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let directory = manager
            .directory
            .lock()
            .map_err(|_| "Android directory failed")?;
        super::disk::usage(&directory.root)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn android_export_diagnostics(
    window: Window,
    state: State<'_, Android>,
) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    settings_window(&window)?;
    let manager = backend(&window, &state)?;
    let destination = tauri::async_runtime::spawn_blocking(move || {
        window
            .app_handle()
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("Export Android diagnostics")
            .set_file_name("lomi-android-diagnostics.txt")
            .add_filter("Text", &["txt"])
            .blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(destination) = destination else {
        return Ok(false);
    };
    let destination = destination
        .into_path()
        .map_err(|_| "Select a local destination")?;
    let text = super::diagnostics::collect(manager).await?;
    tauri::async_runtime::spawn_blocking(move || {
        super::storage::write_bytes(&destination, text.as_bytes())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(true)
}

#[tauri::command]
pub async fn android_install_plan(
    window: Window,
    state: State<'_, Android>,
    catalog_revision: String,
    packages: Vec<Selection>,
    prepare_tools: bool,
) -> Result<Plan, String> {
    settings_window(&window)?;
    host::require_qualification()?;
    let manager = backend(&window, &state)?;
    manager
        .installer
        .plan(&manager, &catalog_revision, packages, prepare_tools)
}

#[tauri::command]
pub async fn android_install(
    window: Window,
    state: State<'_, Android>,
    plan_id: String,
    accepted: Vec<String>,
) -> Result<Progress, String> {
    settings_window(&window)?;
    host::require_qualification()?;
    let manager = backend(&window, &state)?;
    manager.installer.start(&manager, &plan_id, accepted)
}

#[tauri::command]
pub async fn android_cancel_operation(
    window: Window,
    state: State<'_, Android>,
    operation_id: String,
) -> Result<(), String> {
    settings_window(&window)?;
    backend(&window, &state)?.installer.cancel(&operation_id)
}

#[tauri::command]
pub async fn android_manage_device(
    window: Window,
    state: State<'_, Android>,
    action: devices::Action,
) -> Result<Progress, String> {
    settings_window(&window)?;
    if matches!(
        &action,
        devices::Action::Create { .. } | devices::Action::Wipe { .. }
    ) {
        host::require_qualification()?;
    }
    let manager = backend(&window, &state)?;
    manager.installer.manage(&manager, action)
}

#[tauri::command]
pub async fn android_maintenance(
    window: Window,
    state: State<'_, Android>,
    action: super::maintenance::Action,
) -> Result<Progress, String> {
    settings_window(&window)?;
    let manager = backend(&window, &state)?;
    manager.installer.maintain(&manager, action)
}

#[tauri::command]
pub async fn save_android_preferences(
    window: Window,
    state: State<'_, Android>,
    data: Preferences,
    expected_revision: u64,
) -> Result<Preferences, String> {
    settings_window(&window)?;
    let manager = backend(&window, &state)?;
    let _lease = manager.begin_mutation(false)?;
    let saved = manager
        .directory
        .lock()
        .map_err(|_| "Android directory failed")?
        .save_preferences(data, expected_revision)?;
    manager.emit(Event::Metadata);
    Ok(saved)
}

#[tauri::command]
pub async fn android_start(
    window: Window,
    state: State<'_, Android>,
    device_id: String,
) -> Result<Status, String> {
    crate::files::main_window(&window)?;
    host::require_qualification()?;
    backend(&window, &state)?.start(&device_id).await
}

#[tauri::command]
pub async fn android_stop(
    window: Window,
    state: State<'_, Android>,
    device_id: String,
    force: bool,
) -> Result<Status, String> {
    backend(&window, &state)?.stop(&device_id, force).await
}

#[tauri::command]
pub fn android_subscribe_frames(
    window: Window,
    state: State<'_, Android>,
    device_id: String,
    generation: String,
    size: super::frames::Size,
    frames: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
) -> Result<u64, String> {
    crate::files::main_window(&window)?;
    if !window.is_visible().map_err(|e| e.to_string())?
        || window.is_minimized().map_err(|e| e.to_string())?
    {
        return Err("Android image is paused while the window is hidden".into());
    }
    let manager = backend(&window, &state)?;
    let runtime = manager.runtime(&device_id, &generation)?;
    manager.streams.subscribe(
        &manager,
        runtime,
        generation,
        size,
        Box::new(move |packet| {
            frames
                .send(tauri::ipc::InvokeResponseBody::Raw(packet))
                .map_err(|e| e.to_string())
        }),
    )
}

#[tauri::command]
pub fn android_ack_frame(
    window: Window,
    state: State<'_, Android>,
    device_id: String,
    generation: String,
    epoch: u64,
    sequence: u64,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    if let Some(manager) = state.loaded() {
        manager
            .streams
            .ack(&device_id, &generation, epoch, sequence)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn android_unsubscribe_frames(
    window: Window,
    state: State<'_, Android>,
    device_id: String,
    generation: String,
    epoch: u64,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    if let Some(manager) = state.loaded() {
        manager
            .streams
            .unsubscribe(&device_id, &generation, epoch)
            .await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn android_input(
    window: Window,
    state: State<'_, Android>,
    device_id: String,
    generation: String,
    input: super::input::Request,
) -> Result<super::input::Reply, String> {
    crate::files::main_window(&window)?;
    if !matches!(input, super::input::Request::Blur { .. })
        && (!window.is_focused().map_err(|e| e.to_string())?
            || !window.is_visible().map_err(|e| e.to_string())?
            || window.is_minimized().map_err(|e| e.to_string())?)
    {
        return Err("Focus the Lomi window before controlling Android.".into());
    }
    super::input::Router::submit(backend(&window, &state)?, device_id, generation, input).await
}

#[tauri::command]
pub async fn android_install_apk(
    window: Window,
    state: State<'_, Android>,
    device_id: String,
    generation: String,
) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    crate::files::main_window(&window)?;
    let manager = backend(&window, &state)?;
    manager.runtime(&device_id, &generation)?;
    let file =
        tauri::async_runtime::spawn_blocking(move || -> Result<Option<std::fs::File>, String> {
            let dialog = window
                .app_handle()
                .dialog()
                .file()
                .set_parent(&window)
                .set_title("Install APK on Android")
                .add_filter("Android application", &["apk"]);
            #[cfg(feature = "android-probe")]
            let dialog = super::fixture::preset_dialog(dialog, false)?;
            let Some(path) = dialog.blocking_pick_file() else {
                return Ok(None);
            };
            let path = path.into_path().map_err(|_| "Select a local APK file")?;
            std::fs::File::open(path)
                .map(Some)
                .map_err(|error| format!("Cannot open the selected APK: {error}"))
        })
        .await
        .map_err(|e| e.to_string())??;
    let Some(file) = file else {
        return Ok(false);
    };
    manager
        .runtime(&device_id, &generation)?
        .install_apk(generation, file)
        .await?;
    Ok(true)
}

#[tauri::command]
pub async fn android_save_screenshot(
    window: Window,
    state: State<'_, Android>,
    device_id: String,
    generation: String,
) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    crate::files::main_window(&window)?;
    let manager = backend(&window, &state)?;
    manager.runtime(&device_id, &generation)?;
    let destination = tauri::async_runtime::spawn_blocking(move || -> Result<_, String> {
        let dialog = window
            .app_handle()
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("Save Android screenshot")
            .set_file_name("android-screenshot.png")
            .add_filter("PNG image", &["png"]);
        #[cfg(feature = "android-probe")]
        let dialog = super::fixture::preset_dialog(dialog, true)?;
        Ok(dialog.blocking_save_file())
    })
    .await
    .map_err(|e| e.to_string())??;
    let Some(path) = destination else {
        return Ok(false);
    };
    let path = path
        .into_path()
        .map_err(|_| "Select a local screenshot destination")?;
    let runtime = manager.runtime(&device_id, &generation)?;
    let connection = runtime.connection().await?;
    let frame = connection
        .client()
        .max_decoding_message_size(super::rpc::MAX_SCREENSHOT_BYTES + 65536)
        .get_screenshot(connection.request(
            "getScreenshot",
            crate::android_protocol::ImageFormat {
                format: 0,
                ..Default::default()
            },
        )?)
        .await
        .map_err(|e| e.to_string())?
        .into_inner();
    let format = frame
        .format
        .as_ref()
        .ok_or("Android screenshot has no format")?;
    if format.width == 0 && format.height == 0 {
        return Err(
            "The Android screen is asleep. Wake the phone and take another screenshot.".into(),
        );
    }
    if format.width > super::rpc::MAX_DISPLAY_EDGE
        || format.height > super::rpc::MAX_DISPLAY_EDGE
        || format
            .width
            .checked_mul(format.height)
            .is_none_or(|n| n == 0 || n > super::rpc::MAX_DISPLAY_PIXELS)
        || frame.image.len() < 33
        || frame.image.len() > super::rpc::MAX_SCREENSHOT_BYTES
        || !frame.image.starts_with(b"\x89PNG\r\n\x1a\n")
        || &frame.image[12..16] != b"IHDR"
        || u32::from_be_bytes(frame.image[16..20].try_into().unwrap()) != format.width
        || u32::from_be_bytes(frame.image[20..24].try_into().unwrap()) != format.height
    {
        return Err(
            "Android returned an invalid or oversized screenshot. Reconnect the panel and retry."
                .into(),
        );
    }
    tauri::async_runtime::spawn_blocking(move || {
        use std::io::Write;
        let parent = path.parent().ok_or("Invalid screenshot destination")?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        temporary
            .write_all(&frame.image)
            .map_err(|e| e.to_string())?;
        temporary.as_file().sync_all().map_err(|e| e.to_string())?;
        temporary.persist(&path).map_err(|e| e.to_string())?;
        super::installation::sync_directory(parent)?;
        Ok::<_, String>(true)
    })
    .await
    .map_err(|e| e.to_string())?
}

pub fn window_event(window: &Window, event: &tauri::WindowEvent) {
    if window.label() != "main" {
        return;
    }
    if matches!(
        event,
        tauri::WindowEvent::Focused(false) | tauri::WindowEvent::Destroyed
    ) {
        if let Some(manager) = window.state::<Android>().loaded() {
            super::input::Router::release_from_host(manager);
        }
    }
    if matches!(
        event,
        tauri::WindowEvent::Resized(_)
            | tauri::WindowEvent::Focused(_)
            | tauri::WindowEvent::Destroyed
    ) && (matches!(event, tauri::WindowEvent::Destroyed)
        || window.is_minimized().unwrap_or(true)
        || !window.is_visible().unwrap_or(false))
    {
        // Never initialize Android in a window event. A native minimize must
        // cancel the source even if WebKit already suspended its JavaScript.
        if let Some(manager) = window.state::<Android>().loaded() {
            manager.streams.hide_all();
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum ExitAction {
    Begin,
    Finish { preparation: String, force: bool },
    Resume { preparation: String },
}

#[tauri::command]
pub async fn android_exit(
    window: Window,
    state: State<'_, Android>,
    action: ExitAction,
) -> Result<Option<String>, String> {
    crate::files::main_window(&window)?;
    // The empty state also needs an exit barrier: Settings could otherwise begin
    // the first installation while main is awaiting a file-save guard.
    match action {
        ExitAction::Begin => state.begin_exit().map(Some),
        ExitAction::Finish { preparation, force } => {
            state.finish_exit(&preparation, force).await?;
            Ok(None)
        }
        ExitAction::Resume { preparation } => {
            state.resume(&preparation)?;
            Ok(None)
        }
    }
}
