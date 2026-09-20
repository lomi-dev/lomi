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

#[cfg(any(target_os = "linux", test))]
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
            if source_package {
                "simplebench"
            } else {
                "simplebench-bin"
            }
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
                .args(["-Qq", "simplebench"])
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
    if window.label() != "settings" {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn download_server(delay: Duration, chunks: usize) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = [0; 4096];
            socket.read(&mut request).unwrap();
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {chunks}\r\n\r\n"
            )
            .unwrap();
            for index in 0..chunks {
                if index > 0 {
                    std::thread::sleep(delay);
                }
                if socket.write_all(b"x").is_err() {
                    break;
                }
            }
        });
        (url, server)
    }

    fn test_client(timeout: Duration) -> reqwest::ClientBuilder {
        let _ = rustls::crypto::ring::default_provider().install_default();
        configure_client(reqwest::Client::builder().no_proxy(), timeout)
    }

    #[tokio::test]
    async fn active_downloads_can_outlast_the_network_timeout() {
        let (url, server) = download_server(Duration::from_millis(50), 16);
        let client = test_client(Duration::from_millis(400)).build().unwrap();
        let mut response = client.get(url).send().await.unwrap();
        let mut received = Vec::new();
        while let Some(chunk) = response.chunk().await.unwrap() {
            received.extend_from_slice(&chunk);
        }
        server.join().unwrap();
        assert_eq!(received, vec![b'x'; 16]);
    }

    #[tokio::test]
    async fn stalled_downloads_time_out_while_reading_the_body() {
        let (url, server) = download_server(Duration::from_millis(600), 2);
        let client = test_client(Duration::from_millis(200)).build().unwrap();
        let mut response = client.get(url).send().await.unwrap();
        assert_eq!(response.chunk().await.unwrap().unwrap().as_ref(), b"x");
        let error = response.chunk().await.unwrap_err();
        server.join().unwrap();
        assert!(error.is_timeout(), "{error:?}");
    }

    #[tokio::test]
    async fn metadata_checks_keep_their_total_timeout() {
        let (url, server) = download_server(Duration::from_millis(50), 16);
        let client = test_client(Duration::from_millis(400))
            .timeout(Duration::from_millis(200))
            .build()
            .unwrap();
        let response = client.get(url).send().await.unwrap();
        let error = response.bytes().await.unwrap_err();
        server.join().unwrap();
        assert!(error.is_timeout(), "{error:?}");
    }

    #[test]
    fn linux_updates_follow_the_installation_method() {
        assert_eq!(linux_instruction(true, "ID=arch", true), "flatpak update");
        assert_eq!(
            linux_instruction(false, "ID=arch", false),
            "yay -Syu simplebench-bin"
        );
        assert_eq!(
            linux_instruction(false, "ID=endeavouros\nID_LIKE=\"arch\"", true),
            "yay -Syu simplebench"
        );
        for os in ["", "ID=debian", "NAME=arch\nID=ubuntu", "ID=archipelago"] {
            assert!(linux_instruction(false, os, false).starts_with("Download the latest package"));
        }
    }
}
