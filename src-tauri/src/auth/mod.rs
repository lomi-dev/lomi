mod callback;
mod config;
pub(crate) mod controller;
pub(crate) mod http;
mod storage;

pub use controller::AuthController;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State, Window};

pub const STATE_CHANGED_EVENT: &str = "auth-state-changed";

#[derive(Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthStorage {
    Persistent,
    Session,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthStatus {
    Unavailable,
    SignedOut,
    Authorizing,
    Checking,
    SignedIn,
    Offline,
    StorageLocked,
    Error,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthUser {
    pub(super) id: String,
    pub(super) display_name: String,
    pub(super) email: String,
    pub(super) github_login: String,
    pub(super) status: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthSession {
    pub(super) id: String,
    pub(super) expires_at: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthAttempt {
    pub(super) id: String,
    pub(super) expires_at: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthState {
    pub revision: u64,
    pub status: AuthStatus,
    pub user: Option<AuthUser>,
    pub session: Option<AuthSession>,
    pub attempt: Option<AuthAttempt>,
    pub storage: Option<AuthStorage>,
    pub message: Option<String>,
    pub remote_revocation_confirmed: Option<bool>,
}

impl AuthState {
    pub(super) fn new(status: AuthStatus, message: Option<String>) -> Self {
        Self {
            revision: 0,
            status,
            user: None,
            session: None,
            attempt: None,
            storage: None,
            message,
            remote_revocation_confirmed: None,
        }
    }
}

fn trusted_account_window(window: &Window) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("Account access is available only to Lomi application windows.".into())
    }
}

fn settings_window(window: &Window) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("Account management is available only in Settings.".into())
    }
}

#[tauri::command(rename_all = "camelCase")]
pub fn auth_get_state(
    window: Window,
    controller: State<'_, AuthController>,
) -> Result<AuthState, String> {
    trusted_account_window(&window)?;
    Ok(controller.snapshot())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn auth_begin_login(
    window: Window,
    controller: State<'_, AuthController>,
    storage: AuthStorage,
) -> Result<AuthState, String> {
    trusted_account_window(&window)?;
    Ok(controller.begin_login(storage).await)
}

#[tauri::command(rename_all = "camelCase")]
pub fn auth_open_verification(
    window: Window,
    app: AppHandle,
    controller: State<'_, AuthController>,
    attempt_id: String,
) -> Result<AuthState, String> {
    trusted_account_window(&window)?;
    controller.open_verification(&app, &attempt_id)?;
    Ok(controller.snapshot())
}

#[tauri::command(rename_all = "camelCase")]
pub fn auth_cancel_login(
    window: Window,
    controller: State<'_, AuthController>,
    attempt_id: String,
) -> Result<AuthState, String> {
    settings_window(&window)?;
    Ok(controller.cancel_login(&attempt_id))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn auth_refresh_state(
    window: Window,
    controller: State<'_, AuthController>,
) -> Result<AuthState, String> {
    trusted_account_window(&window)?;
    Ok(controller.refresh_state().await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn auth_sign_out(
    window: Window,
    controller: State<'_, AuthController>,
) -> Result<AuthState, String> {
    trusted_account_window(&window)?;
    Ok(controller.sign_out().await)
}

#[tauri::command(rename_all = "camelCase")]
pub fn auth_open_account_portal(
    window: Window,
    app: AppHandle,
    controller: State<'_, AuthController>,
) -> Result<AuthState, String> {
    trusted_account_window(&window)?;
    controller.open_account_portal(&app)?;
    Ok(controller.snapshot())
}

pub(super) fn safe_text(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(limit)
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_snapshot_uses_the_stable_camel_case_contract() {
        let mut state = AuthState::new(AuthStatus::Authorizing, None);
        state.attempt = Some(AuthAttempt {
            id: "attempt-1".into(),
            expires_at: "2026-10-01T00:00:00Z".into(),
        });
        let value = serde_json::to_value(state).unwrap();
        for key in [
            "revision",
            "status",
            "user",
            "session",
            "attempt",
            "storage",
            "message",
            "remoteRevocationConfirmed",
        ] {
            assert!(value.get(key).is_some(), "missing {key}");
        }
        assert_eq!(value["status"], "authorizing");
        assert_eq!(
            value["attempt"],
            serde_json::json!({
                "id": "attempt-1",
                "expiresAt": "2026-10-01T00:00:00Z",
            })
        );
        let serialized = value.to_string();
        for forbidden in [
            "accessToken",
            "deviceCode",
            "device_code",
            "userCode",
            "verificationUri",
            "authorizationUrl",
            "codeVerifier",
            "codeChallenge",
            "cookie",
        ] {
            assert!(!serialized.contains(forbidden));
        }
    }

    #[test]
    fn user_facing_text_removes_controls_and_is_bounded() {
        assert_eq!(safe_text("A\nB\0C", 20), "ABC");
        assert_eq!(safe_text("123456", 3), "123");
    }
}
