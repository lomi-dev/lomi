use std::sync::Mutex;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder, Window};

#[derive(Default)]
struct Lifecycle {
    ready: bool,
    requested: bool,
    page: Option<String>,
}

#[derive(Default)]
pub struct SettingsWindow {
    creation: tauri::async_runtime::Mutex<()>,
    lifecycle: Mutex<Lifecycle>,
}

pub async fn prepare(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<SettingsWindow>();
    // Preloading and rapid clicks must share one webview. Creation stays off
    // the event thread because WebView2 can deadlock when built there.
    let _creation = state.creation.lock().await;
    if app.get_window("settings").is_some() {
        return Ok(());
    }
    *state.lifecycle.lock().map_err(|error| error.to_string())? = Lifecycle::default();
    let builder = WebviewWindowBuilder::new(
        app,
        "settings",
        WebviewUrl::App("index.html?window=settings".into()),
    )
    .title("Settings — Lomi")
    .inner_size(920.0, 680.0)
    .min_inner_size(560.0, 420.0)
    .visible(false)
    .focused(false)
    .decorations(cfg!(target_os = "macos"))
    .transparent(true)
    .background_color(tauri::window::Color(0, 0, 0, 0));
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(crate::macos::traffic_lights::SETTINGS_POSITION)
        .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Throttle);
    builder.build().map_err(|error| error.to_string())?;
    Ok(())
}

pub fn ready(window: &Window) -> Result<bool, String> {
    let state = window.state::<SettingsWindow>();
    let requested = {
        let mut lifecycle = state.lifecycle.lock().map_err(|error| error.to_string())?;
        lifecycle.ready = true;
        lifecycle.requested
    };
    if requested {
        show(window)?;
    }
    Ok(requested)
}

fn show(window: &Window) -> Result<(), String> {
    let page = window
        .state::<SettingsWindow>()
        .lifecycle
        .lock()
        .map_err(|error| error.to_string())?
        .page
        .take();
    if let Some(page) = page {
        window
            .emit("settings-page-changed", page)
            .map_err(|error| error.to_string())?;
    }
    window.unminimize().map_err(|error| error.to_string())?;
    window.show().map_err(|error| error.to_string())?;
    let _ = window.emit("settings-visibility", true);
    window.set_focus().map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn open_settings(
    window: Window,
    app: tauri::AppHandle,
    page: Option<String>,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    if page.as_deref().is_some_and(|page| {
        !matches!(
            page,
            "keybinds"
                | "themes"
                | "plugins"
                | "editor"
                | "terminal"
                | "about"
                | "chat-ai"
                | "android"
                | "agent-control"
        )
    }) {
        return Err("Unknown settings page.".into());
    }
    request_checked(&app, page, || Ok(())).await
}

pub(crate) async fn request_checked(
    app: &tauri::AppHandle,
    page: Option<String>,
    check: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    check()?;
    prepare(app).await?;
    check()?;
    let state = app.state::<SettingsWindow>();
    let ready = {
        let mut lifecycle = state.lifecycle.lock().map_err(|error| error.to_string())?;
        lifecycle.requested = true;
        if page.is_some() {
            lifecycle.page = page;
        }
        lifecycle.ready
    };
    if ready {
        if let Some(window) = app.get_window("settings") {
            show(&window)?;
        }
    }
    Ok(())
}

pub fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() != "settings" {
        return;
    }
    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
        if window.hide().is_ok() {
            let _ = window.emit("settings-visibility", false);
            api.prevent_close();
            if let Ok(mut lifecycle) = window.state::<SettingsWindow>().lifecycle.lock() {
                lifecycle.requested = false;
                lifecycle.page = None;
            }
        }
    }
}
