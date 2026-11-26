//! Immediate terminal delivery uses the same account policy as task alerts.
use super::{DesktopPreferences, Push};
use tauri::{AppHandle, Manager, Window};
use tauri_plugin_notification::NotificationExt;

#[derive(Debug, Default, PartialEq, Eq)]
struct Delivery {
    inbox: bool,
    system: bool,
}

fn policy(
    preferences: Option<&DesktopPreferences>,
    denied: bool,
    focused: Option<bool>,
) -> Delivery {
    let Some(p) = preferences.filter(|p| p.enabled) else {
        return Delivery::default();
    };
    Delivery {
        inbox: true,
        system: !denied && (!p.background_only || focused == Some(false)),
    }
}

pub(crate) fn deliver(
    app: &AppHandle,
    window: &Window,
    kind: crate::notifications::NotificationKind,
    agent: Option<crate::cli_catalog::TitleCli>,
    pane_id: Option<&str>,
    title: &str,
    context: &str,
    on_click: Option<Box<dyn FnOnce() + Send>>,
) -> Result<(bool, Result<(), String>), String> {
    // OFF, logout and retirement use this same guard. Keep it through inbox
    // recording and OS enqueue so authority cannot change between them.
    let _guard = crate::auth::controller::LOCAL_NOTIFICATION_EMISSION_GUARD
        .lock()
        .map_err(|_| "Notification delivery unavailable.")?;
    let auth = app.state::<crate::auth::AuthController>();
    let Ok(binding) = auth.local_notification_binding() else {
        return Ok((false, Ok(())));
    };
    let push = app.state::<Push>();
    let decision = {
        let core = push
            .inner
            .lock()
            .map_err(|_| "Notification settings unavailable.")?;
        if core.path.is_none() || core.file.retired_sessions.contains(&binding.session_id) {
            return Ok((false, Ok(())));
        }
        policy(
            core.file.accounts.get(&binding.user_id),
            core.denied,
            window.is_focused().ok(),
        )
    };
    let recorded = if decision.inbox {
        crate::notifications::record(app, kind, agent, pane_id, title, context)
    } else {
        Ok(())
    };
    // A failed inbox write must not suppress an otherwise valid OS alert.
    if decision.system {
        match on_click {
            #[cfg(target_os = "macos")]
            Some(on_click) => show_clickable(app, title, context, on_click),
            _ => {
                let _ = app
                    .notification()
                    .builder()
                    .title(title)
                    .body(context)
                    .show();
            }
        }
    }
    Ok((decision.system, recorded))
}

/// Shows an alert and runs `on_click` when the user clicks it. The wait holds a thread
/// until the alert is clicked or leaves Notification Center.
#[cfg(target_os = "macos")]
fn show_clickable(app: &AppHandle, title: &str, body: &str, on_click: Box<dyn FnOnce() + Send>) {
    // Same sender as tauri-plugin-notification, which also sets it (only the first call counts).
    let sender = if tauri::is_dev() {
        "com.apple.Terminal".to_string()
    } else {
        app.config().identifier.clone()
    };
    let (title, body) = (title.to_string(), body.to_string());
    std::thread::spawn(move || {
        let _ = mac_notification_sys::set_application(&sender);
        let response = mac_notification_sys::Notification::new()
            .title(&title)
            .message(&body)
            .wait_for_click(true)
            .send();
        if matches!(
            response,
            Ok(mac_notification_sys::NotificationResponse::Click)
        ) {
            on_click();
        }
    });
}
