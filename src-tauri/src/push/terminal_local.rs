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
        let _ = app
            .notification()
            .builder()
            .title(title)
            .body(context)
            .show();
    }
    Ok((decision.system, recorded))
}
