//! Completion notifications have no execution authority and own no source DB.
//! Lock order: emission guard -> auth read -> preferences/cache. Source locks
//! are acquired only after these locks are released. HTTP never holds them.
mod dispatch;
pub(crate) mod ledger;
use crate::auth::controller::PushRequestError;
use ledger::{Admission, CloudStamp, DesktopStamp, Owner};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, Emitter, Manager, State, Window};
use tauri_plugin_notification::NotificationExt;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopPreferences {
    pub schema_version: u32,
    pub enabled: bool,
    pub background_only: bool,
    pub consent_epoch: u64,
    pub revision: u64,
}
impl Default for DesktopPreferences {
    fn default() -> Self {
        Self {
            schema_version: 1,
            enabled: false,
            background_only: true,
            consent_epoch: 1,
            revision: 1,
        }
    }
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preference {
    pub enabled: bool,
    pub consent_epoch: u64,
    pub revision: u64,
    pub available: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Source {
    pub id: String,
    pub desktop_installation_id: String,
    pub enabled: bool,
    pub source_epoch: u64,
    pub revision: u64,
    pub label: String,
    pub current: bool,
    pub revoked: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Installation {
    pub id: String,
    pub binding_id: String,
    pub binding_generation: u64,
    pub receive_epoch: u64,
    pub token_generation: u64,
    pub revision: u64,
    pub enabled: bool,
    pub label: String,
    pub platform: String,
    pub permission_state: String,
    pub disabled_reason: Option<String>,
    pub retired: bool,
    pub current: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EventStatus {
    Queued,
    Suppressed,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventReceipt {
    pub event_id: String,
    pub status: EventStatus,
    pub duplicate: bool,
    pub recipient_count: u64,
}
fn validate_views(
    sources: &[Source],
    installations: &[Installation],
) -> Result<(), PushRequestError> {
    if sources.len() > 128
        || installations.len() > 64
        || sources.iter().any(|s| s.label.len() > 320)
        || installations.iter().any(|s| s.label.len() > 320)
    {
        return Err(invalid_response());
    }
    Ok(())
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushState {
    pub revision: u64,
    pub account_id: Option<String>,
    pub available: bool,
    pub feature_enabled: bool,
    pub preferences: Preference,
    pub sources: Vec<Source>,
    pub installations: Vec<Installation>,
    pub current_source_id: Option<String>,
    pub error_code: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopState {
    pub revision: u64,
    pub account_id: Option<String>,
    pub available: bool,
    pub preferences: DesktopPreferences,
    pub permission_status: String,
    pub error_code: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cache {
    user_id: String,
    session_id: String,
    preferences: Preference,
    sources: Vec<Source>,
    installations: Vec<Installation>,
}
#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct File {
    schema_version: u32,
    installation_id: String,
    accounts: BTreeMap<String, DesktopPreferences>,
    cache: Option<Cache>,
    retired_sessions: Vec<String>,
}
struct Core {
    path: Option<PathBuf>,
    file: File,
    process_id: String,
    revision: u64,
    refresh_generation: u64,
    error: Option<String>,
    denied: bool,
    tests: Vec<u64>,
}
#[derive(Clone)]
pub struct Push {
    inner: Arc<Mutex<Core>>,
}
impl Default for Push {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Core {
                path: None,
                file: File::default(),
                process_id: uuid().unwrap_or_default(),
                revision: 0,
                refresh_generation: 0,
                error: None,
                denied: false,
                tests: Vec::new(),
            })),
        }
    }
}
fn uuid() -> Result<String, String> {
    let mut b = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b)
        .map_err(|_| "Cannot create notification identity.")?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}
fn valid_id(id: &str) -> Result<(), String> {
    if id.len() != 36
        || !id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
    {
        return Err("INVALID_REQUEST".into());
    }
    Ok(())
}
fn trusted(window: &Window, mutation: bool) -> Result<(), String> {
    if window.label() == "settings" || (!mutation && window.label() == "main") {
        Ok(())
    } else {
        Err("Notifications are available only in trusted Lomi windows; management requires Settings.".into())
    }
}
fn persist(core: &mut Core) -> Result<(), String> {
    let path = core
        .path
        .as_ref()
        .ok_or("Notification preferences are unavailable.")?;
    let data =
        serde_json::to_vec(&core.file).map_err(|_| "Cannot encode notification preferences.")?;
    if let Err(error) = crate::chat::storage::atomic(path, &data) {
        core.error = Some("STORAGE_UNAVAILABLE".into());
        core.path = None;
        return Err(error);
    }
    core.revision += 1;
    Ok(())
}
// Native local failures expose only stable codes; HTTP preserves its typed
// bounded status and Retry-After all the way to Settings.
impl From<String> for PushRequestError {
    fn from(value: String) -> Self {
        Self::local(match value.as_str() {
            "INVALID_REQUEST" => "INVALID_REQUEST",
            "SESSION_REQUIRED" => "SESSION_REQUIRED",
            "REVISION_CONFLICT" => "REVISION_CONFLICT",
            _ => "STORAGE_UNAVAILABLE",
        })
    }
}
impl From<&str> for PushRequestError {
    fn from(value: &str) -> Self {
        value.to_string().into()
    }
}
impl Push {
    pub fn initialize(&self, app: &AppHandle) {
        let result = (|| -> Result<(), String> {
            let root = app
                .path()
                .app_data_dir()
                .map_err(|_| "Cannot locate notifications.")?;
            let environment = app
                .state::<crate::auth::AuthController>()
                .remote_environment()?;
            let namespace = format!("{:x}", Sha256::digest(environment.as_bytes()));
            let root = root.join("notifications");
            crate::chat::storage::reject_link(&root)?;
            std::fs::create_dir_all(&root)
                .map_err(|_| "Cannot create notification preferences.")?;
            crate::chat::storage::private(&root, true)?;
            let path = root.join(format!("{namespace}-desktop-notification-preferences.json"));
            crate::chat::storage::reject_link(&path)?;
            let file = if path.exists() {
                let meta = std::fs::metadata(&path)
                    .map_err(|_| "Cannot read notification preferences.")?;
                if !meta.is_file() || meta.len() > 1024 * 1024 {
                    return Err("Invalid notification preferences.".into());
                }
                let file: File = serde_json::from_slice(
                    &std::fs::read(&path).map_err(|_| "Cannot read notification preferences.")?,
                )
                .map_err(|_| "Invalid notification preferences.")?;
                if file.schema_version != 1
                    || file.accounts.len() > 128
                    || file.retired_sessions.len() > 1024
                    || file
                        .accounts
                        .values()
                        .any(|p| p.schema_version != 1 || p.consent_epoch == 0 || p.revision == 0)
                {
                    return Err("Unknown notification preferences.".into());
                }
                valid_id(&file.installation_id)?;
                file
            } else {
                File {
                    schema_version: 1,
                    installation_id: uuid()?,
                    ..Default::default()
                }
            };
            let mut c = self
                .inner
                .lock()
                .map_err(|_| "Notification preferences unavailable.")?;
            c.file = file;
            c.path = Some(path);
            persist(&mut c)
        })();
        if result.is_err() {
            if let Ok(mut c) = self.inner.lock() {
                c.error = Some("STORAGE_UNAVAILABLE".into());
                c.path = None;
            }
        }
    }
    pub(crate) fn start(&self, app: &AppHandle) {
        dispatch::start(app.clone());
    }
    fn owner(core: &Core, b: &crate::auth::controller::RemoteBinding) -> Owner {
        Owner {
            user_id: b.user_id.clone(),
            session_id: b.session_id.clone(),
            process_id: core.process_id.clone(),
            generation: b.generation,
        }
    }
    pub fn desktop_state(&self, auth: &crate::auth::AuthController) -> DesktopState {
        let binding = auth.local_notification_binding().ok();
        let available = binding.is_some();
        let c = self.inner.lock().unwrap();
        let p = binding
            .as_ref()
            .and_then(|b| c.file.accounts.get(&b.user_id))
            .cloned()
            .unwrap_or_default();
        DesktopState {
            revision: c.revision,
            account_id: binding.map(|b| b.user_id),
            available: c.path.is_some() && available,
            preferences: p,
            permission_status: if c.denied { "denied" } else { "unknown" }.into(),
            error_code: c.error.clone(),
        }
    }
    pub fn state(&self, auth: &crate::auth::AuthController) -> PushState {
        let binding = auth.remote_binding().ok();
        let c = self.inner.lock().unwrap();
        let cache = binding.as_ref().and_then(|b| {
            c.file.cache.as_ref().filter(|v| {
                v.user_id == b.user_id
                    && v.session_id == b.session_id
                    && !c.file.retired_sessions.contains(&b.session_id)
            })
        });
        let preferences = cache.map(|c| c.preferences.clone()).unwrap_or_default();
        PushState {
            revision: c.revision,
            account_id: binding.map(|b| b.user_id),
            available: c.path.is_some() && cache.is_some(),
            feature_enabled: preferences.available,
            preferences,
            sources: cache.map(|c| c.sources.clone()).unwrap_or_default(),
            installations: cache.map(|c| c.installations.clone()).unwrap_or_default(),
            current_source_id: cache.and_then(|cache| {
                cache
                    .sources
                    .iter()
                    .find(|s| {
                        s.current
                            && !s.revoked
                            && s.desktop_installation_id == c.file.installation_id
                    })
                    .map(|s| s.id.clone())
            }),
            error_code: c.error.clone(),
        }
    }
    fn publish(&self, app: &AppHandle) {
        let auth = app.state::<crate::auth::AuthController>();
        let desktop = self.desktop_state(&auth);
        let cloud = self.state(&auth);
        for label in ["main", "settings"] {
            let _ = app.emit_to(
                tauri::EventTarget::webview(label),
                "desktop-notifications-state-changed",
                &desktop,
            );
            let _ = app.emit_to(
                tauri::EventTarget::webview(label),
                "push-state-changed",
                &cloud,
            );
        }
    }
    pub async fn refresh(&self, app: &AppHandle) -> Result<PushState, PushRequestError> {
        let auth = app.state::<crate::auth::AuthController>();
        let b = auth
            .remote_binding()
            .map_err(|_| PushRequestError::local("SESSION_REQUIRED"))?;
        let (installation_id, refresh_generation) = {
            let mut c = self.inner.lock().unwrap();
            if c.path.is_none() || c.file.retired_sessions.contains(&b.session_id) {
                return Err("STORAGE_UNAVAILABLE".into());
            }
            c.refresh_generation = c
                .refresh_generation
                .checked_add(1)
                .ok_or("Revision exhausted.")?;
            (c.file.installation_id.clone(), c.refresh_generation)
        };
        let result = async {
            let (_, p) = auth.push_request_bound(reqwest::Method::GET, "/v1/remote/native/push/preferences", None, &b).await?;
            let preference: Preference = serde_json::from_value(p).map_err(|_| invalid_response())?;
            let (_, sources) = auth.push_request_bound(reqwest::Method::GET, "/v1/remote/native/push/sources", None, &b).await?;
            let mut sources: Vec<Source> = serde_json::from_value(sources["sources"].clone()).map_err(|_| invalid_response())?;
            if preference.available && preference.enabled && !sources.iter().any(|s| s.current && s.desktop_installation_id == installation_id && !s.revoked) {
                let (_, response) = auth.push_request_bound(reqwest::Method::POST, "/v1/remote/native/push/sources/register", Some(&json!({"operationId":uuid().map_err(|_| invalid_response())?,"desktopInstallationId":installation_id,"label":"This computer","enabled":true})), &b).await?;
                let source: Source = serde_json::from_value(response).map_err(|_| invalid_response())?;
                sources.retain(|s| s.id != source.id); sources.truncate(127); sources.push(source);
            }
            let (_, phones) = auth.push_request_bound(reqwest::Method::GET, "/v1/remote/native/push/installations", None, &b).await?;
            let installations: Vec<Installation> = serde_json::from_value(phones["installations"].clone()).map_err(|_| invalid_response())?;
            validate_views(&sources, &installations)?;
            Ok::<_, crate::auth::controller::PushRequestError>((preference,sources,installations))
        }.await;
        let _guard = crate::auth::controller::LOCAL_NOTIFICATION_EMISSION_GUARD
            .lock()
            .map_err(|_| "Notification authority unavailable.")?;
        let current = auth
            .remote_binding()
            .map_err(|_| PushRequestError::local("SESSION_REQUIRED"))?;
        if !current.same_authority(&b) {
            return Err("SESSION_REQUIRED".into());
        }
        let mut c = self.inner.lock().unwrap();
        let cache = result.map(|(preferences, sources, installations)| Cache {
            user_id: b.user_id,
            session_id: b.session_id,
            preferences,
            sources,
            installations,
        });
        let committed = commit_refresh(&mut c, refresh_generation, cache);
        if let Err(error) = committed {
            drop(c);
            drop(_guard);
            self.publish(app);
            return Err(error);
        }
        drop(c);
        drop(_guard);
        self.publish(app);
        Ok(self.state(&auth))
    }
}
fn commit_refresh(
    core: &mut Core,
    generation: u64,
    response: Result<Cache, PushRequestError>,
) -> Result<bool, PushRequestError> {
    // A response from an older same-account read must not revive ON after a
    // newer OFF or overwrite its error/revision with stale state.
    if generation != core.refresh_generation {
        return Ok(false);
    }
    match response {
        Ok(cache) => {
            core.file.cache = Some(cache);
            core.error = None;
            persist(core)?;
            Ok(true)
        }
        Err(error) => {
            core.error = Some(error.code.clone());
            core.revision += 1;
            Err(error)
        }
    }
}
fn invalid_response() -> crate::auth::controller::PushRequestError {
    crate::auth::controller::PushRequestError {
        status: None,
        code: "INVALID_RESPONSE".into(),
        retry_after_seconds: None,
    }
}

pub(crate) fn retire_owner(app: &AppHandle, _user: &str, session: &str) -> Result<(), String> {
    let push = app.state::<Push>();
    let mut c = push
        .inner
        .lock()
        .map_err(|_| "Notification preferences unavailable.")?;
    if !c.file.retired_sessions.iter().any(|s| s == session) {
        c.file.retired_sessions.push(session.into());
    }
    if c.file.retired_sessions.len() > 1024 {
        c.path = None;
        c.error = Some("STORAGE_UNAVAILABLE".into());
        return Err("Notification retirement bound exceeded.".into());
    }
    persist(&mut c)
}
pub(crate) fn capture(app: &AppHandle) -> Admission {
    let auth = app.state::<crate::auth::AuthController>();
    let local = auth.local_notification_binding().ok();
    let cloud = auth.remote_binding().ok();
    let push = app.state::<Push>();
    let c = push.inner.lock().unwrap();
    if c.path.is_none() {
        return Admission::default();
    }
    let desktop = local
        .as_ref()
        .filter(|b| !c.file.retired_sessions.contains(&b.session_id))
        .and_then(|b| {
            c.file
                .accounts
                .get(&b.user_id)
                .filter(|p| p.enabled && !c.denied)
                .map(|p| DesktopStamp {
                    owner: Push::owner(&c, b),
                    installation_id: c.file.installation_id.clone(),
                    consent_epoch: p.consent_epoch,
                })
        });
    let cloud = cloud
        .as_ref()
        .filter(|b| !c.file.retired_sessions.contains(&b.session_id))
        .and_then(|b| {
            c.file
                .cache
                .as_ref()
                .filter(|v| {
                    v.user_id == b.user_id
                        && v.session_id == b.session_id
                        && v.preferences.available
                        && v.preferences.enabled
                })
                .and_then(|v| {
                    v.sources
                        .iter()
                        .find(|s| {
                            s.current
                                && !s.revoked
                                && s.enabled
                                && s.desktop_installation_id == c.file.installation_id
                        })
                        .map(|s| CloudStamp {
                            owner: Push::owner(&c, b),
                            source_id: s.id.clone(),
                            consent_epoch: v.preferences.consent_epoch,
                            source_epoch: s.source_epoch,
                        })
                })
        });
    Admission { desktop, cloud }
}

#[tauri::command(rename_all = "camelCase")]
pub fn push_get_state(
    window: Window,
    app: AppHandle,
    push: State<'_, Push>,
) -> Result<PushState, String> {
    trusted(&window, false)?;
    Ok(push.state(&app.state::<crate::auth::AuthController>()))
}
#[tauri::command(rename_all = "camelCase")]
pub async fn push_refresh_state(
    window: Window,
    app: AppHandle,
    push: State<'_, Push>,
) -> Result<PushState, PushRequestError> {
    trusted(&window, false)?;
    push.refresh(&app).await
}
async fn mutation(app: &AppHandle, path: &str, body: Value) -> Result<PushState, PushRequestError> {
    let auth = app.state::<crate::auth::AuthController>();
    let result = auth
        .push_request(reqwest::Method::POST, path, Some(&body))
        .await;
    let refresh = app.state::<Push>().refresh(app).await;
    result?;
    refresh
}
#[tauri::command(rename_all = "camelCase")]
pub async fn push_update_account_preferences(
    window: Window,
    app: AppHandle,
    operation_id: String,
    expected_revision: u64,
    enabled: bool,
) -> Result<PushState, PushRequestError> {
    trusted(&window, true)?;
    valid_id(&operation_id)?;
    mutation(
        &app,
        "/v1/remote/native/push/preferences",
        json!({"operationId":operation_id,"expectedRevision":expected_revision,"enabled":enabled}),
    )
    .await
}
#[tauri::command(rename_all = "camelCase")]
pub async fn push_set_source_enabled(
    window: Window,
    app: AppHandle,
    source_id: String,
    operation_id: String,
    expected_revision: u64,
    enabled: bool,
) -> Result<PushState, PushRequestError> {
    trusted(&window, true)?;
    valid_id(&source_id)?;
    valid_id(&operation_id)?;
    mutation(
        &app,
        &format!("/v1/remote/native/push/sources/{source_id}/settings"),
        json!({"operationId":operation_id,"expectedRevision":expected_revision,"enabled":enabled}),
    )
    .await
}
#[tauri::command(rename_all = "camelCase")]
pub async fn push_disable_installation(
    window: Window,
    app: AppHandle,
    installation_id: String,
    operation_id: String,
    expected_revision: u64,
) -> Result<PushState, PushRequestError> {
    trusted(&window, true)?;
    valid_id(&installation_id)?;
    valid_id(&operation_id)?;
    mutation(
        &app,
        &format!("/v1/remote/native/push/installations/{installation_id}/disable"),
        json!({"operationId":operation_id,"expectedRevision":expected_revision}),
    )
    .await
}
#[tauri::command(rename_all = "camelCase")]
pub async fn push_send_test(
    window: Window,
    app: AppHandle,
    installation_id: String,
    operation_id: String,
) -> Result<EventReceipt, PushRequestError> {
    trusted(&window, true)?;
    valid_id(&installation_id)?;
    valid_id(&operation_id)?;
    let (_, value) = app
        .state::<crate::auth::AuthController>()
        .push_request(
            reqwest::Method::POST,
            "/v1/remote/native/push/test",
            Some(&json!({"operationId":operation_id,"installationId":installation_id})),
        )
        .await?;
    serde_json::from_value(value).map_err(|_| invalid_response())
}
#[tauri::command(rename_all = "camelCase")]
pub fn desktop_notifications_get_state(
    window: Window,
    app: AppHandle,
    push: State<'_, Push>,
) -> Result<DesktopState, String> {
    trusted(&window, false)?;
    Ok(push.desktop_state(&app.state::<crate::auth::AuthController>()))
}
#[tauri::command(rename_all = "camelCase")]
pub fn desktop_notifications_save_preferences(
    window: Window,
    app: AppHandle,
    push: State<'_, Push>,
    expected_revision: u64,
    enabled: bool,
    background_only: bool,
) -> Result<DesktopState, String> {
    trusted(&window, true)?;
    let _guard = crate::auth::controller::LOCAL_NOTIFICATION_EMISSION_GUARD
        .lock()
        .map_err(|_| "Notification authority unavailable.")?;
    let auth = app.state::<crate::auth::AuthController>();
    let owner = auth.local_notification_binding()?;
    let mut c = push.inner.lock().unwrap();
    save_local_preferences(
        &mut c,
        &owner.user_id,
        &owner.session_id,
        expected_revision,
        enabled,
        background_only,
    )?;
    drop(c);
    drop(_guard);
    push.publish(&app);
    Ok(push.desktop_state(&auth))
}
#[tauri::command(rename_all = "camelCase")]
pub fn desktop_notifications_report_permission(
    window: Window,
    app: AppHandle,
    push: State<'_, Push>,
    denied: bool,
) -> Result<DesktopState, String> {
    trusted(&window, true)?;
    let guard = crate::auth::controller::LOCAL_NOTIFICATION_EMISSION_GUARD
        .lock()
        .map_err(|_| "Notification authority unavailable.")?;
    app.state::<crate::auth::AuthController>()
        .local_notification_binding()?;
    let mut c = push.inner.lock().unwrap();
    c.denied = denied;
    c.revision += 1;
    drop(c);
    drop(guard);
    push.publish(&app);
    Ok(push.desktop_state(&app.state::<crate::auth::AuthController>()))
}
#[tauri::command(rename_all = "camelCase")]
pub fn desktop_notifications_send_test(
    window: Window,
    app: AppHandle,
    push: State<'_, Push>,
) -> Result<Value, String> {
    trusted(&window, true)?;
    let _guard = crate::auth::controller::LOCAL_NOTIFICATION_EMISSION_GUARD
        .lock()
        .map_err(|_| "Notification authority unavailable.")?;
    let b = app
        .state::<crate::auth::AuthController>()
        .local_notification_binding()?;
    let mut c = push.inner.lock().unwrap();
    let p = c.file.accounts.get(&b.user_id).cloned().unwrap_or_default();
    if c.path.is_none() || !p.enabled || c.denied || c.file.retired_sessions.contains(&b.session_id)
    {
        return Err("NOT_ENABLED".into());
    }
    let time = ledger::now();
    c.tests.retain(|t| *t + 60 > time);
    if c.tests.len() >= 3 {
        return Err("RATE_LIMITED".into());
    }
    c.tests.push(time);
    app.notification()
        .builder()
        .title("Lomi")
        .body("Test powiadomień Lomi.")
        .show()
        .map_err(|_| "PLUGIN_ERROR".to_string())?;
    Ok(json!({"status":"plugin-accepted"}))
}

fn save_local_preferences(
    c: &mut Core,
    owner_user_id: &str,
    owner_session_id: &str,
    expected_revision: u64,
    enabled: bool,
    background_only: bool,
) -> Result<(), String> {
    if c.path.is_none()
        || c.file
            .retired_sessions
            .iter()
            .any(|s| s == owner_session_id)
    {
        return Err("STORAGE_UNAVAILABLE".into());
    }
    if c.file.accounts.len() >= 128 && !c.file.accounts.contains_key(owner_user_id) {
        return Err("Notification account bound exceeded.".into());
    }
    let mut p = c
        .file
        .accounts
        .get(owner_user_id)
        .cloned()
        .unwrap_or_default();
    if p.revision != expected_revision {
        return Err("REVISION_CONFLICT".into());
    }
    if p.enabled != enabled {
        p.consent_epoch = p
            .consent_epoch
            .checked_add(1)
            .ok_or("Revision exhausted.")?;
    }
    p.enabled = enabled;
    p.background_only = background_only;
    p.revision = p.revision.checked_add(1).ok_or("Revision exhausted.")?;
    c.file.accounts.insert(owner_user_id.to_string(), p);
    persist(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn core(root: &std::path::Path) -> Core {
        Core {
            path: Some(root.join("preferences.json")),
            file: File {
                schema_version: 1,
                installation_id: "computer".into(),
                ..Default::default()
            },
            process_id: "process".into(),
            revision: 0,
            refresh_generation: 0,
            error: None,
            denied: false,
            tests: Vec::new(),
        }
    }
    #[test]
    fn reversed_refresh_responses_cannot_revive_off_or_replace_latest_error() {
        let root = tempfile::tempdir().unwrap();
        let mut c = core(root.path());
        c.refresh_generation = 2;
        let cache = |enabled, revision| Cache {
            user_id: "a".into(),
            session_id: "s".into(),
            preferences: Preference {
                enabled,
                revision,
                consent_epoch: revision,
                available: true,
            },
            sources: Vec::new(),
            installations: Vec::new(),
        };
        assert!(commit_refresh(&mut c, 2, Ok(cache(false, 3))).unwrap());
        let bytes = std::fs::read(c.path.as_ref().unwrap()).unwrap();
        let revision = c.revision;
        assert!(!commit_refresh(&mut c, 1, Ok(cache(true, 2))).unwrap());
        assert!(!c.file.cache.as_ref().unwrap().preferences.enabled);
        assert_eq!(c.revision, revision);
        assert_eq!(std::fs::read(c.path.as_ref().unwrap()).unwrap(), bytes);
        assert!(!commit_refresh(
            &mut c,
            1,
            Err(PushRequestError::local("SERVICE_UNAVAILABLE"))
        )
        .unwrap());
        assert!(c.error.is_none());
    }
    #[test]
    fn account_preferences_use_cas_and_only_enabled_changes_advance_epoch() {
        let root = tempfile::tempdir().unwrap();
        let mut c = core(root.path());
        save_local_preferences(&mut c, "a", "session", 1, true, true).unwrap();
        assert_eq!(c.file.accounts["a"].consent_epoch, 2);
        let saved = std::fs::read(c.path.as_ref().unwrap()).unwrap();
        assert!(save_local_preferences(&mut c, "a", "session", 1, false, true).is_err());
        assert_eq!(std::fs::read(c.path.as_ref().unwrap()).unwrap(), saved);
        save_local_preferences(&mut c, "a", "session", 2, true, false).unwrap();
        assert_eq!(c.file.accounts["a"].consent_epoch, 2);
        save_local_preferences(&mut c, "a", "session", 3, false, false).unwrap();
        assert_eq!(c.file.accounts["a"].consent_epoch, 3);
        assert!(
            !c.file
                .accounts
                .get("b")
                .cloned()
                .unwrap_or_default()
                .enabled
        );
        c.file.retired_sessions.push("session".into());
        assert!(save_local_preferences(&mut c, "a", "session", 4, true, true).is_err());
        let restored: File =
            serde_json::from_slice(&std::fs::read(c.path.as_ref().unwrap()).unwrap()).unwrap();
        assert!(!restored.accounts["a"].enabled);
        assert_eq!(restored.accounts["a"].revision, 4);
    }
    #[test]
    fn installation_contract_accepts_actual_backend_phone_and_rejects_tokens() {
        let phone = json!({"id":"phone","bindingId":"binding","bindingGeneration":1,"receiveEpoch":1,"tokenGeneration":1,"revision":1,"enabled":true,"label":"My phone","platform":"ios","permissionState":"granted","disabledReason":null,"retired":false,"current":false});
        let installation: Installation = serde_json::from_value(phone.clone()).unwrap();
        assert!(!installation.current);
        let mut invalid = phone;
        invalid["expoPushToken"] = json!("secret");
        assert!(serde_json::from_value::<Installation>(invalid).is_err());
        assert!(!serde_json::to_string(&installation)
            .unwrap()
            .contains("expoPushToken"));
    }
    #[test]
    fn history_views_accept_retired_records_beyond_active_caps() {
        let phone:Installation=serde_json::from_value(json!({"id":"phone","bindingId":"binding","bindingGeneration":1,"receiveEpoch":1,"tokenGeneration":1,"revision":1,"enabled":false,"label":"Old phone","platform":"android","permissionState":"denied","disabledReason":null,"retired":true,"current":false})).unwrap();
        let source = Source {
            id: "source".into(),
            desktop_installation_id: "i".into(),
            enabled: false,
            source_epoch: 1,
            revision: 1,
            label: "Old source".into(),
            current: false,
            revoked: true,
        };
        assert!(validate_views(&vec![source.clone(); 40], &vec![phone.clone(); 24]).is_ok());
        assert!(validate_views(&vec![source; 129], &vec![phone; 65]).is_err());
    }
    #[test]
    fn test_receipt_is_a_strict_safe_dto() {
        let receipt =
            json!({"eventId":"event","status":"queued","duplicate":false,"recipientCount":1});
        assert!(serde_json::from_value::<EventReceipt>(receipt.clone()).is_ok());
        let mut unknown = receipt;
        unknown["token"] = json!("secret");
        assert!(serde_json::from_value::<EventReceipt>(unknown).is_err());
    }
    #[test]
    fn stale_finalization_does_not_revive_completion_after_retention() {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch(ledger::SCHEMA).unwrap();
        let owner = Owner {
            user_id: "u".into(),
            session_id: "s".into(),
            process_id: "p".into(),
            generation: 1,
        };
        ledger::admit(
            &c,
            "turn",
            &Admission {
                desktop: Some(DesktopStamp {
                    owner,
                    installation_id: "i".into(),
                    consent_epoch: 1,
                }),
                cloud: None,
            },
        )
        .unwrap();
        ledger::complete(&c, "chat", "conversation", "turn").unwrap();
        c.execute("UPDATE desktop_notification_intents SET expires_at=0", [])
            .unwrap();
        assert!(ledger::ready(&c, ledger::Channel::Desktop)
            .unwrap()
            .is_empty());
        ledger::complete(&c, "chat", "conversation", "turn").unwrap();
        assert!(ledger::ready(&c, ledger::Channel::Desktop)
            .unwrap()
            .is_empty());
    }
}
