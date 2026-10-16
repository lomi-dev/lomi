use std::sync::Mutex;
use tauri::{Emitter, Manager, Window};

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsTabRequest {
    pub revision: u64,
    pub page: Option<String>,
}

#[derive(Default)]
struct Routing {
    ready: bool,
    revision: u64,
    pending: Option<SettingsTabRequest>,
    latest: Option<SettingsTabRequest>,
}

impl Routing {
    fn request(&mut self, page: Option<String>) -> Result<SettingsTabRequest, String> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or("Settings request revision exhausted.")?;
        let request = SettingsTabRequest {
            revision: self.revision,
            // An initial no-page request must not discard a queued destination.
            page: page.or_else(|| {
                self.pending
                    .as_ref()
                    .and_then(|request| request.page.clone())
            }),
        };
        self.pending = Some(request.clone());
        self.latest = Some(request.clone());
        Ok(request)
    }

    fn ready(&mut self) -> Option<SettingsTabRequest> {
        self.ready = true;
        // Keep the latest request available if the renderer reloads or loses a
        // ready response. The frontend deduplicates replay by revision.
        self.pending.take().or_else(|| self.latest.clone())
    }
}

#[derive(Default)]
pub struct SettingsTabs(Mutex<Routing>);

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutInfo {
    platform: String,
    arch: String,
    version: String,
    identifier: String,
}

#[tauri::command]
pub fn about_info(window: Window) -> Result<AboutInfo, String> {
    crate::files::main_window(&window)?;
    let app = window.app_handle();
    Ok(AboutInfo {
        platform: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        version: app.package_info().version.to_string(),
        identifier: app.config().identifier.clone(),
    })
}

#[tauri::command]
pub fn settings_tab_ready(window: Window) -> Result<Option<SettingsTabRequest>, String> {
    crate::files::main_window(&window)?;
    let state = window.state::<SettingsTabs>();
    let request = state.0.lock().map_err(|error| error.to_string())?.ready();
    Ok(request)
}

fn validate_page(page: Option<&str>) -> Result<(), String> {
    if page.is_some_and(|page| {
        !matches!(
            page,
            "keybinds"
                | "notifications"
                | "themes"
                | "plugins"
                | "terminal"
                | "about"
                | "chat-ai"
                | "android"
                | "account"
                | "agent-control"
                | "remote"
        )
    }) {
        return Err("Unknown settings page.".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn open_settings(
    window: Window,
    app: tauri::AppHandle,
    page: Option<String>,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    request_checked(&app, page, || Ok(())).await
}

pub(crate) async fn request_checked(
    app: &tauri::AppHandle,
    page: Option<String>,
    check: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    validate_page(page.as_deref())?;
    check()?;
    let window = app
        .get_window("main")
        .ok_or("The workspace window is unavailable.")?;
    check()?;
    window.unminimize().map_err(|error| error.to_string())?;
    window.show().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())?;
    let state = app.state::<SettingsTabs>();
    let mut routing = state.0.lock().map_err(|error| error.to_string())?;
    // Broker grants can expire while a request waits for routing admission.
    check()?;
    let request = routing.request(page)?;
    if routing.ready {
        // Keep pending on failure, so a subsequent ready handshake can recover it.
        app.emit_to(
            tauri::EventTarget::webview("main"),
            "settings-tab-requested",
            &request,
        )
        .map_err(|error| error.to_string())?;
        routing.pending = None;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queues_latest_request_and_preserves_initial_destination() {
        let mut routing = Routing::default();
        routing.request(Some("themes".into())).unwrap();
        let latest = routing.request(None).unwrap();
        assert_eq!(latest.page.as_deref(), Some("themes"));
        assert_eq!(latest.revision, 2);
        assert_eq!(routing.ready(), Some(latest.clone()));
        assert!(routing.pending.is_none());
        assert_eq!(routing.ready(), Some(latest));
    }

    #[test]
    fn delivered_null_request_reuses_frontend_selection_and_replays() {
        let mut routing = Routing::default();
        routing.ready();
        routing.request(Some("account".into())).unwrap();
        routing.pending = None;
        let latest = routing.request(None).unwrap();
        assert_eq!(latest.page, None);
        routing.pending = None;
        assert_eq!(routing.ready(), Some(latest));
    }

    #[test]
    fn validates_pages_and_never_wraps_revisions() {
        assert!(validate_page(None).is_ok());
        for page in [
            "keybinds",
            "notifications",
            "themes",
            "plugins",
            "terminal",
            "about",
            "chat-ai",
            "android",
            "account",
            "agent-control",
            "remote",
        ] {
            assert!(validate_page(Some(page)).is_ok());
        }
        assert!(validate_page(Some("unknown")).is_err());
        let mut routing = Routing {
            revision: u64::MAX,
            ..Routing::default()
        };
        assert!(routing.request(None).is_err());
        assert!(routing.pending.is_none());
    }
}
