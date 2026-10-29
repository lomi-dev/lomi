use std::time::Duration;
use tauri::{Emitter, Manager, Webview, Window};
use tauri_plugin_updater::UpdaterExt;

const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
const NETWORK_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMetadata {
    rid: tauri::ResourceId,
    current_version: String,
    version: String,
    body: Option<String>,
    raw_json: serde_json::Value,
}

fn configure_client(client: reqwest::ClientBuilder, timeout: Duration) -> reqwest::ClientBuilder {
    // A read timeout resets on each chunk, so slow downloads can still finish.
    client.connect_timeout(timeout).read_timeout(timeout)
}

#[tauri::command]
pub async fn check_app_update(
    window: Window,
    webview: Webview,
) -> Result<Option<UpdateMetadata>, String> {
    crate::files::main_window(&window)?;
    let updater = webview
        .updater_builder()
        .timeout(CHECK_TIMEOUT)
        .configure_client(|client| configure_client(client, NETWORK_TIMEOUT))
        .build()
        .map_err(|error| error.to_string())?;
    let update = updater.check().await.map_err(|error| error.to_string())?;
    Ok(update.map(|update| UpdateMetadata {
        current_version: update.current_version.clone(),
        version: update.version.clone(),
        body: update.body.clone(),
        raw_json: update.raw_json.clone(),
        // Retain Tauri's signed download, installation and resource cleanup.
        rid: webview.resources_table().add(update),
    }))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateEnvironment {
    linux_instruction: Option<String>,
}

#[cfg(target_os = "linux")]
fn linux_instruction(flatpak: bool, os_release: &str, source_package: bool) -> String {
    if flatpak {
        return "flatpak update".into();
    }
    let arch = os_release.lines().any(|line| {
        line.split_once('=').is_some_and(|(key, value)| {
            matches!(key, "ID" | "ID_LIKE")
                && value
                    .trim_matches(['\"', '\''])
                    .split_whitespace()
                    .any(|id| id == "arch")
        })
    });
    if arch {
        return format!(
            "yay -Syu {}",
            if source_package { "lomi" } else { "lomi-bin" }
        );
    }
    "Download the latest package from GitHub Releases and reinstall it with your distribution’s package manager, or replace your AppImage.".into()
}

#[tauri::command]
pub fn update_environment(window: Window) -> Result<UpdateEnvironment, String> {
    crate::files::main_window(&window)?;
    #[cfg(target_os = "linux")]
    let instruction = {
        let flatpak = std::env::var_os("FLATPAK_ID").is_some()
            || std::path::Path::new("/.flatpak-info").exists();
        let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        let source_package = !flatpak
            && std::process::Command::new("pacman")
                .args(["-Qq", "lomi"])
                .output()
                .is_ok_and(|output| output.status.success());
        Some(linux_instruction(flatpak, &os_release, source_package))
    };
    #[cfg(not(target_os = "linux"))]
    let instruction = None;
    Ok(UpdateEnvironment {
        linux_instruction: instruction,
    })
}

#[tauri::command]
pub fn request_update_check(window: Window, app: tauri::AppHandle) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Update checks can only be requested from settings.".into());
    }
    let main = app.get_window("main").ok_or("The workspace is not open.")?;
    main.unminimize().map_err(|error| error.to_string())?;
    main.show().map_err(|error| error.to_string())?;
    main.set_focus().map_err(|error| error.to_string())?;
    app.emit_to("main", "check-for-updates", ())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn restart_after_update(window: Window, app: tauri::AppHandle) -> Result<(), String> {
    crate::files::main_window(&window)?;
    if cfg!(target_os = "linux") {
        return Err("Linux updates are managed outside the application.".into());
    }
    app.request_restart();
    Ok(())
}
