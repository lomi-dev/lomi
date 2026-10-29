use super::{
    callback::{self, CallbackError},
    config::AuthConfig,
    http::{ApiClient, HttpProblem, RemoteHttpError, SessionCheck},
    storage::{CredentialSaveError, CredentialStore},
    AuthAttempt, AuthState, AuthStatus, AuthStorage, STATE_CHANGED_EVENT,
};
use ring::digest::{digest, SHA256};
use ring::rand::{SecureRandom, SystemRandom};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, EventTarget, Manager};

pub(crate) use super::http::PushRequestError;
pub(crate) use super::http::RemoteHttpError as RemoteV2HttpError;

// Order: emission guard, auth core, notification preferences. Never hold this
// guard across HTTP, source-store access, or an asynchronous OS result.
pub(crate) static LOCAL_NOTIFICATION_EMISSION_GUARD: Mutex<()> = Mutex::new(());

struct AuthorityMutation<'a> {
    core: std::sync::MutexGuard<'a, Core>,
    _emission: std::sync::MutexGuard<'static, ()>,
}

impl std::ops::Deref for AuthorityMutation<'_> {
    type Target = Core;
    fn deref(&self) -> &Core {
        &self.core
    }
}

impl std::ops::DerefMut for AuthorityMutation<'_> {
    fn deref_mut(&mut self) -> &mut Core {
        &mut self.core
    }
}

#[derive(Clone)]
pub struct AuthController {
    inner: Arc<Inner>,
}

struct Inner {
    config: Option<AuthConfig>,
    api: Option<ApiClient>,
    config_message: Option<String>,
    core: Mutex<Core>,
}

struct Core {
    state: AuthState,
    generation: u64,
    active_attempt_id: Option<String>,
    attempt: Option<BrowserAttempt>,
    token: Option<Secret>,
    token_storage: Option<AuthStorage>,
    token_persisted: bool,
    refresh_owner_generation: Option<u64>,
    refresh_started: Option<Instant>,
    store: Option<CredentialStore>,
    app: Option<tauri::AppHandle>,
    session_refresh_stop: Option<tokio::sync::watch::Sender<bool>>,
}

struct BrowserAttempt {
    id: String,
    authorization_url: String,
    cancel: tokio::sync::watch::Sender<bool>,
    expires_at: Instant,
    expires_at_text: String,
}

enum RefreshAction {
    Session(u64, Secret),
    Probe(u64),
}

enum SessionActivation {
    Activated,
    Stale,
    StorageFailure,
}

struct Secret(Vec<u8>);

impl Secret {
    fn new(value: String) -> Self {
        Self(value.into_bytes())
    }

    fn exposed(&self) -> &str {
        std::str::from_utf8(&self.0).expect("auth token was validated as UTF-8")
    }
}

impl Clone for Secret {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

impl Core {
    fn new(status: AuthStatus, message: Option<String>) -> Self {
        Self {
            state: AuthState::new(status, message),
            generation: 0,
            active_attempt_id: None,
            attempt: None,
            token: None,
            token_storage: None,
            token_persisted: false,
            refresh_owner_generation: None,
            refresh_started: None,
            store: None,
            app: None,
            session_refresh_stop: None,
        }
    }

    fn can_refresh_session_automatically(&self) -> bool {
        self.token.is_some()
            && matches!(
                self.state.status,
                AuthStatus::SignedIn | AuthStatus::Offline
            )
            && self.active_attempt_id.is_none()
            && self.refresh_owner_generation != Some(self.generation)
    }

    fn next_generation(&mut self) -> u64 {
        if let Some(attempt) = self.attempt.as_ref() {
            let _ = attempt.cancel.send(true);
        }
        self.generation = self.generation.wrapping_add(1).max(1);
        self.active_attempt_id = None;
        self.refresh_owner_generation = None;
        self.refresh_started = None;
        self.generation
    }

    fn start_attempt(&mut self, id: String) -> u64 {
        let generation = self.next_generation();
        self.active_attempt_id = Some(id);
        generation
    }

    fn current_attempt(&self, generation: u64, id: &str) -> bool {
        self.generation == generation && self.active_attempt_id.as_deref() == Some(id)
    }

    fn invalidate_attempt(&mut self, id: &str) -> bool {
        if self.active_attempt_id.as_deref() != Some(id) {
            return false;
        }
        self.next_generation();
        self.attempt = None;
        self.state.attempt = None;
        true
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteBinding {
    pub user_id: String,
    pub session_id: String,
    pub expires_at: u64,
    pub generation: u64,
}

impl RemoteBinding {
    pub(crate) fn same_authority(&self, other: &Self) -> bool {
        self.user_id == other.user_id
            && self.session_id == other.session_id
            && self.generation == other.generation
    }
}

fn remote_binding(core: &Core) -> Result<RemoteBinding, String> {
    if core.state.status != AuthStatus::SignedIn {
        return Err("An online account session is required.".into());
    }
    let user = core.state.user.as_ref().ok_or("Sign in first.")?;
    let session = core.state.session.as_ref().ok_or("Sign in first.")?;
    let expires = chrono::DateTime::parse_from_rfc3339(&session.expires_at)
        .map_err(|_| "Invalid account expiry.")?
        .timestamp();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "Clock unavailable.")?
        .as_secs();
    if user.status != "active" || expires <= now as i64 || core.token.is_none() {
        return Err("Account authorization expired.".into());
    }
    Ok(RemoteBinding {
        user_id: user.id.clone(),
        session_id: session.id.clone(),
        expires_at: expires as u64,
        generation: core.generation,
    })
}

fn local_notification_binding(core: &Core) -> Result<RemoteBinding, String> {
    if !matches!(
        core.state.status,
        AuthStatus::SignedIn | AuthStatus::Offline
    ) || core.active_attempt_id.is_some()
        || core.attempt.is_some()
    {
        return Err("An active account session is required.".into());
    }
    let user = core.state.user.as_ref().ok_or("Sign in first.")?;
    let session = core.state.session.as_ref().ok_or("Sign in first.")?;
    let expires = chrono::DateTime::parse_from_rfc3339(&session.expires_at)
        .map_err(|_| "Invalid account expiry.")?
        .timestamp();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "Clock unavailable.")?
        .as_secs();
    if core.token.is_none() || user.status != "active" || expires <= now as i64 {
        return Err("Account authorization expired.".into());
    }
    Ok(RemoteBinding {
        user_id: user.id.clone(),
        session_id: session.id.clone(),
        expires_at: expires as u64,
        generation: core.generation,
    })
}

impl Default for AuthController {
    fn default() -> Self {
        let config = AuthConfig::from_build().ok();
        let config_message = config
            .is_none()
            .then(|| "Account sign-in is not configured for this build.".to_string());
        let api = config
            .clone()
            .and_then(|config| ApiClient::new(config).ok());
        let (status, message) = if config.is_none() || api.is_none() {
            (
                AuthStatus::Unavailable,
                Some("Account sign-in is not available in this build.".into()),
            )
        } else {
            (AuthStatus::SignedOut, None)
        };
        Self {
            inner: Arc::new(Inner {
                config,
                api,
                config_message,
                core: Mutex::new(Core::new(status, message)),
            }),
        }
    }
}

impl AuthController {
    fn authority_mutation(&self) -> Result<AuthorityMutation<'_>, &'static str> {
        let emission = LOCAL_NOTIFICATION_EMISSION_GUARD
            .lock()
            .map_err(|_| "Account unavailable.")?;
        let core = self.inner.core.lock().map_err(|_| "Account unavailable.")?;
        Ok(AuthorityMutation {
            core,
            _emission: emission,
        })
    }

    pub(crate) fn local_notification_binding(&self) -> Result<RemoteBinding, String> {
        local_notification_binding(&*self.inner.core.lock().map_err(|_| "Account unavailable.")?)
    }

    #[cfg(test)]
    pub(crate) async fn push_request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<(RemoteBinding, serde_json::Value), PushRequestError> {
        self.push_request_inner(method, path, body, None).await
    }

    pub(crate) async fn push_request_bound(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&serde_json::Value>,
        expected: &RemoteBinding,
    ) -> Result<(RemoteBinding, serde_json::Value), PushRequestError> {
        self.push_request_inner(method, path, body, Some(expected))
            .await
    }

    async fn push_request_inner(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&serde_json::Value>,
        expected: Option<&RemoteBinding>,
    ) -> Result<(RemoteBinding, serde_json::Value), PushRequestError> {
        let (token, binding) = {
            let core = self
                .inner
                .core
                .lock()
                .map_err(|_| PushRequestError::local("SESSION_REQUIRED"))?;
            let binding =
                remote_binding(&core).map_err(|_| PushRequestError::local("SESSION_REQUIRED"))?;
            if expected.is_some_and(|owner| !owner.same_authority(&binding)) {
                return Err(PushRequestError::local("AUTHORITY_CHANGED"));
            }
            (
                core.token
                    .clone()
                    .ok_or_else(|| PushRequestError::local("SESSION_REQUIRED"))?,
                binding,
            )
        };
        let result = self
            .inner
            .api
            .as_ref()
            .ok_or_else(|| PushRequestError::local("SERVICE_UNAVAILABLE"))?
            .push_request(token.exposed(), method, path, body)
            .await;
        let retirement = result.as_ref().is_err_and(|error| {
            matches!(
                (error.status, error.code.as_str()),
                (Some(401), "SESSION_REQUIRED") | (Some(403), "ACCOUNT_UNAVAILABLE")
            )
        });
        if retirement {
            self.retire_denied_push_authority(&binding, &token)?;
            return Err(result.expect_err("known denial requires an error"));
        }
        let core = self
            .inner
            .core
            .lock()
            .map_err(|_| PushRequestError::local("SESSION_REQUIRED"))?;
        let current =
            remote_binding(&core).map_err(|_| PushRequestError::local("SESSION_REQUIRED"))?;
        if !current.same_authority(&binding) {
            return Err(PushRequestError::local("AUTHORITY_CHANGED"));
        }
        Ok((current, result?))
    }

    fn retire_denied_push_authority(
        &self,
        expected: &RemoteBinding,
        expected_token: &Secret,
    ) -> Result<(), PushRequestError> {
        {
            let mut core = self
                .authority_mutation()
                .map_err(|_| PushRequestError::local("SESSION_REQUIRED"))?;
            // A definitive denial fences this captured authority even if a
            // concurrent connectivity check moved it to Offline or Checking.
            let same_owner = core.generation == expected.generation
                && core
                    .state
                    .user
                    .as_ref()
                    .is_some_and(|u| u.id == expected.user_id)
                && core
                    .state
                    .session
                    .as_ref()
                    .is_some_and(|s| s.id == expected.session_id);
            if !same_owner
                || core
                    .token
                    .as_ref()
                    .is_none_or(|t| t.exposed() != expected_token.exposed())
            {
                return Err(PushRequestError::local("AUTHORITY_CHANGED"));
            }
            retire_notification_owner(&core);
            let persisted = tombstone_store(&mut core);
            core.next_generation();
            clear_account(&mut core);
            core.state.status = if persisted {
                AuthStatus::SignedOut
            } else {
                AuthStatus::StorageLocked
            };
            core.state.message =
                Some("The account session is no longer authorized. Sign in again.".into());
        }
        self.publish();
        Ok(())
    }

    pub fn initialize(&self, root: &Path, app: tauri::AppHandle) {
        let mut startup_check = false;
        if let Ok(mut core) = self.authority_mutation() {
            core.app = Some(app);
            if core.state.status == AuthStatus::Unavailable {
                drop(core);
                self.publish();
                return;
            }
            let Some(config) = self.inner.config.as_ref() else {
                core.state.status = AuthStatus::Unavailable;
                core.state.message = self.inner.config_message.clone();
                drop(core);
                self.publish();
                return;
            };
            let storage_namespace = config.storage_namespace();
            let environment_root = root.join(&storage_namespace);
            match CredentialStore::open(&environment_root, &storage_namespace) {
                Ok(mut store) => match store.load_active() {
                    Ok(token) => {
                        core.token_persisted = token.is_some();
                        core.token_storage = token.as_ref().map(|_| AuthStorage::Persistent);
                        core.token = token.map(Secret::new);
                        core.store = Some(store);
                        if core.token.is_some() {
                            core.state.status = AuthStatus::Checking;
                            core.state.storage = Some(AuthStorage::Persistent);
                            core.state.message = None;
                            startup_check = true;
                        } else {
                            core.state.status = AuthStatus::SignedOut;
                            core.state.storage = None;
                        }
                    }
                    Err(message) => {
                        core.store = Some(store);
                        core.state.status = AuthStatus::StorageLocked;
                        core.state.message = Some(message);
                    }
                },
                Err(message) => {
                    core.state.status = AuthStatus::StorageLocked;
                    core.state.message = Some(message);
                }
            }
        }
        self.publish();
        self.start_session_refresh();
        if startup_check {
            let controller = self.clone();
            tauri::async_runtime::spawn(async move {
                controller.refresh_state().await;
            });
        }
    }

    fn start_session_refresh(&self) {
        let mut stopped = {
            let Ok(mut core) = self.authority_mutation() else {
                return;
            };
            if core.session_refresh_stop.is_some() {
                return;
            }
            let (stop, stopped) = tokio::sync::watch::channel(false);
            core.session_refresh_stop = Some(stop);
            stopped
        };
        let controller = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::select! {
                    _ = async {
                        tokio::time::sleep(Duration::from_secs(60)).await;
                        controller.refresh_state_inner(true).await;
                    } => {}
                    _ = stopped.changed() => break,
                }
            }
        });
    }

    pub fn snapshot(&self) -> AuthState {
        self.inner
            .core
            .lock()
            .map(|core| core.state.clone())
            .unwrap_or_else(|_| {
                AuthState::new(
                    AuthStatus::Error,
                    Some("Account state is unavailable.".into()),
                )
            })
    }

    /// Session-bound native HTTP access. Tokens never leave this controller.
    pub(crate) async fn remote_request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<(RemoteBinding, serde_json::Value), String> {
        let (token, binding) = {
            let core = self.inner.core.lock().map_err(|_| "Account unavailable.")?;
            let binding = remote_binding(&core)?;
            (core.token.clone().ok_or("Sign in first.")?, binding)
        };
        let result = self
            .inner
            .api
            .as_ref()
            .ok_or("Account unavailable.")?
            .remote_request(token.exposed(), method, path, body)
            .await;
        let core = self.inner.core.lock().map_err(|_| "Account unavailable.")?;
        let current = remote_binding(&core)?;
        if !current.same_authority(&binding) {
            return Err("Account session changed.".into());
        }
        let value = result.map_err(remote_request_message)?;
        Ok((current, value))
    }

    /// Native Remote v2 access is allowed only when this account issuer is the
    /// public identity authority pinned into the desktop executable.
    pub(crate) async fn remote_v2_request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<(RemoteBinding, serde_json::Value), RemoteHttpError> {
        if !self
            .inner
            .config
            .as_ref()
            .is_some_and(AuthConfig::is_host_identity_origin)
        {
            return Err(RemoteHttpError::local_code("IDENTITY_MISMATCH"));
        }
        let (token, binding) = {
            let core = self
                .inner
                .core
                .lock()
                .map_err(|_| RemoteHttpError::local_code("ACCOUNT_UNAVAILABLE"))?;
            let binding = remote_binding(&core)
                .map_err(|_| RemoteHttpError::local_code("AUTHORIZATION_REQUIRED"))?;
            let token = core
                .token
                .clone()
                .ok_or_else(|| RemoteHttpError::local_code("AUTHORIZATION_REQUIRED"))?;
            (token, binding)
        };
        let result = self
            .inner
            .api
            .as_ref()
            .ok_or_else(|| RemoteHttpError::local_code("SERVICE_UNAVAILABLE"))?
            .remote_v2_request(token.exposed(), method, path, body)
            .await;
        let core = self
            .inner
            .core
            .lock()
            .map_err(|_| RemoteHttpError::local_code("ACCOUNT_UNAVAILABLE"))?;
        let current = remote_binding(&core)
            .map_err(|_| RemoteHttpError::local_code("AUTHORIZATION_REQUIRED"))?;
        if !current.same_authority(&binding) {
            return Err(RemoteHttpError::local_code("SESSION_CHANGED"));
        }
        let value = result?;
        Ok((current, value))
    }

    #[cfg(feature = "remote-probe")]
    pub(crate) fn probe_session(
        &self,
        token: String,
        user_id: String,
        session_id: String,
        expires_at: String,
    ) -> Result<(), String> {
        if self.inner.config.as_ref().is_none_or(|c| {
            !matches!(
                c.origin.as_str(),
                "http://127.0.0.1:4321" | "http://127.0.0.1:4324"
            )
        }) || token.is_empty()
            || token.len() > 4096
            || token.bytes().any(|b| b.is_ascii_control())
        {
            return Err("Probe account must use the isolated local identity fixture.".into());
        }
        let mut core = self.authority_mutation()?;
        core.next_generation();
        core.store = None;
        core.token = Some(Secret::new(token));
        core.token_storage = Some(AuthStorage::Session);
        core.state.status = AuthStatus::SignedIn;
        core.state.user = Some(super::AuthUser {
            id: user_id,
            display_name: "Remote fixture".into(),
            email: "".into(),
            github_login: "".into(),
            status: "active".into(),
        });
        core.state.session = Some(super::AuthSession {
            id: session_id,
            expires_at,
        });
        remote_binding(&core)?;
        Ok(())
    }

    pub(crate) fn remote_environment(&self) -> Result<String, String> {
        self.inner
            .config
            .as_ref()
            .map(|c| c.environment.clone())
            .ok_or_else(|| "Account environment unavailable.".into())
    }

    pub(crate) fn remote_binding(&self) -> Result<RemoteBinding, String> {
        remote_binding(&*self.inner.core.lock().map_err(|_| "Account unavailable.")?)
    }

    pub async fn begin_login(&self, storage: AuthStorage) -> AuthState {
        if self.inner.api.is_none() || self.inner.config.is_none() {
            return self.snapshot();
        }
        let attempt_id = match random_id() {
            Ok(id) => id,
            Err(_) => {
                self.set_error(AuthStatus::Error, "Cannot start account sign-in.");
                return self.snapshot();
            }
        };
        let generation = {
            let Ok(mut core) = self.authority_mutation() else {
                return self.snapshot();
            };
            if core.token.is_some() || core.active_attempt_id.is_some() {
                return core.state.clone();
            }
            if core.store.is_none() && core.state.status == AuthStatus::StorageLocked {
                core.state.message.get_or_insert_with(|| {
                    "Account storage could not be opened. Resolve its ownership or metadata error and restart Lomi before signing in.".into()
                });
                drop(core);
                self.publish();
                return self.snapshot();
            }
            let has_unresolved_session = core
                .store
                .as_ref()
                .is_some_and(CredentialStore::has_active_reference);
            if has_unresolved_session && storage == AuthStorage::Persistent {
                core.state.status = AuthStatus::StorageLocked;
                core.state.message.get_or_insert_with(|| {
                    "The saved session could not be restored. Open Account settings to check it or sign out of this device before signing in again.".into()
                });
                drop(core);
                self.publish();
                return self.snapshot();
            }
            let cleanup_pending = if has_unresolved_session {
                let store = core.store.as_mut().expect("reference requires a store");
                if let Err(message) = store.tombstone() {
                    core.state.status = AuthStatus::StorageLocked;
                    core.state.message = Some(message);
                    drop(core);
                    self.publish();
                    return self.snapshot();
                }
                !store.cleanup_journal()
            } else {
                false
            };
            if storage == AuthStorage::Persistent && core.store.is_none() {
                core.state.status = AuthStatus::StorageLocked;
                core.state.message = Some(
                    "The system key store is unavailable. Choose session-only storage or unlock it and retry.".into(),
                );
                drop(core);
                self.publish();
                return self.snapshot();
            }
            let generation = core.start_attempt(attempt_id.clone());
            core.state.status = AuthStatus::Authorizing;
            core.state.attempt = None;
            core.state.storage = Some(storage);
            core.state.message = cleanup_pending.then(|| {
                "Starting session-only sign-in. An older local credential is blocked from restore; key store cleanup will be retried.".into()
            });
            core.state.remote_revocation_confirmed = None;
            generation
        };
        self.publish();

        let listener = match callback::bind_loopback().await {
            Ok(listener) => listener,
            Err(message) => {
                self.finish_attempt(generation, &attempt_id, AuthStatus::Error, &message);
                return self.snapshot();
            }
        };
        let redirect_uri = match callback::redirect_uri(&listener) {
            Ok(uri) => uri,
            Err(message) => {
                self.finish_attempt(generation, &attempt_id, AuthStatus::Error, &message);
                return self.snapshot();
            }
        };
        let (state, code_verifier, code_challenge) = match create_pkce_pair() {
            Ok(pair) => pair,
            Err(message) => {
                self.finish_attempt(generation, &attempt_id, AuthStatus::Error, &message);
                return self.snapshot();
            }
        };
        let expires_at = Instant::now() + Duration::from_secs(300);
        let expires_at_text = format_timestamp(SystemTime::now() + Duration::from_secs(300));
        let result = self
            .inner
            .api
            .as_ref()
            .expect("checked above")
            .start_desktop(&redirect_uri, state.exposed(), &code_challenge)
            .await;
        match result {
            Ok(start) => {
                let authorization_url = match self.inner.config.as_ref().and_then(|config| {
                    config
                        .desktop_authorization_url(&start.authorization_url)
                        .ok()
                }) {
                    Some(url) => url,
                    None => {
                        self.finish_attempt(
                            generation,
                            &attempt_id,
                            AuthStatus::Error,
                            "The account service returned an untrusted sign-in address.",
                        );
                        return self.snapshot();
                    }
                };
                let (cancel, cancel_receiver) = tokio::sync::watch::channel(false);
                let attempt = BrowserAttempt {
                    id: attempt_id.clone(),
                    authorization_url,
                    cancel,
                    expires_at,
                    expires_at_text: expires_at_text.clone(),
                };
                let mut accepted = false;
                if let Ok(mut core) = self.authority_mutation() {
                    if core.current_attempt(generation, &attempt_id) {
                        core.state.status = AuthStatus::Authorizing;
                        core.state.storage = Some(storage);
                        core.state.attempt = Some(AuthAttempt {
                            id: attempt.id.clone(),
                            expires_at: attempt.expires_at_text.clone(),
                        });
                        core.state.message = None;
                        core.attempt = Some(attempt);
                        accepted = true;
                    }
                }
                self.publish();
                if accepted {
                    let controller = self.clone();
                    tauri::async_runtime::spawn(async move {
                        controller
                            .await_browser_callback_and_exchange(
                                attempt_id,
                                generation,
                                listener,
                                state,
                                code_verifier,
                                redirect_uri,
                                expires_at,
                                storage,
                                cancel_receiver,
                            )
                            .await;
                    });
                }
            }
            Err(problem) => {
                if let Ok(mut core) = self.authority_mutation() {
                    if core.current_attempt(generation, &attempt_id) {
                        core.next_generation();
                        core.state.status = if matches!(
                            problem,
                            HttpProblem::Transport
                                | HttpProblem::Temporary
                                | HttpProblem::RateLimited(_)
                        ) {
                            AuthStatus::Offline
                        } else {
                            AuthStatus::Error
                        };
                        core.state.attempt = None;
                        core.state.storage = None;
                        core.state.message = Some(desktop_start_message(problem).into());
                    }
                }
                self.publish();
            }
        }
        self.snapshot()
    }

    pub fn open_verification(
        &self,
        app: &tauri::AppHandle,
        attempt_id: &str,
    ) -> Result<(), String> {
        let uri = {
            let core = self
                .inner
                .core
                .lock()
                .map_err(|_| "Account state is unavailable.".to_string())?;
            let attempt = core
                .attempt
                .as_ref()
                .filter(|attempt| attempt.id == attempt_id)
                .ok_or_else(|| "This sign-in attempt is no longer active.".to_string())?;
            if Instant::now() >= attempt.expires_at {
                return Err(
                    "This sign-in attempt has expired. Start again to open a new browser request."
                        .into(),
                );
            }
            self.inner
                .config
                .as_ref()
                .ok_or_else(|| "Account sign-in is unavailable in this build.".to_string())?
                .desktop_authorization_url(&attempt.authorization_url)?
        };
        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .open_url(uri, None::<&str>)
            .map_err(|_| "Cannot open the trusted account verification page.".to_string())
    }

    pub fn cancel_login(&self, attempt_id: &str) -> AuthState {
        if let Ok(mut core) = self.authority_mutation() {
            if core.invalidate_attempt(attempt_id) {
                core.state.status = AuthStatus::SignedOut;
                core.state.storage = None;
                core.state.message = None;
                core.state.remote_revocation_confirmed = None;
            }
        }
        self.publish()
    }

    pub async fn refresh_state(&self) -> AuthState {
        self.refresh_state_inner(false).await
    }

    async fn refresh_state_inner(&self, automatic: bool) -> AuthState {
        if self.inner.api.is_none() {
            return self.snapshot();
        }
        let action = {
            let Ok(mut core) = self.authority_mutation() else {
                return self.snapshot();
            };
            // Background checks never load storage or probe without a usable token.
            if automatic && !core.can_refresh_session_automatically() {
                return core.state.clone();
            }
            if core.refresh_owner_generation == Some(core.generation)
                || core.active_attempt_id.is_some()
            {
                return core.state.clone();
            }
            if core
                .refresh_started
                .is_some_and(|started| started.elapsed() < Duration::from_secs(2))
            {
                return core.state.clone();
            }
            if core.token.is_none()
                && core
                    .store
                    .as_ref()
                    .is_some_and(CredentialStore::has_active_reference)
            {
                let loaded = core
                    .store
                    .as_mut()
                    .map(CredentialStore::load_active)
                    .unwrap_or(Ok(None));
                match loaded {
                    Ok(Some(token)) => {
                        core.token = Some(Secret::new(token));
                        core.token_storage = Some(AuthStorage::Persistent);
                        core.token_persisted = true;
                    }
                    Ok(None) => {
                        core.state.status = AuthStatus::SignedOut;
                        core.state.user = None;
                        core.state.session = None;
                        core.state.storage = None;
                        core.state.message = None;
                    }
                    Err(message) => {
                        core.state.status = AuthStatus::StorageLocked;
                        core.state.message = Some(message);
                        drop(core);
                        return self.publish();
                    }
                }
            }
            if core.token.is_none() && core.state.status == AuthStatus::StorageLocked {
                return core.state.clone();
            }
            core.refresh_owner_generation = Some(core.generation);
            core.refresh_started = Some(Instant::now());
            // A live session remains authorized while its online check is pending.
            if remote_binding(&core).is_err() {
                core.state.status = AuthStatus::Checking;
            }
            core.state.message = None;
            match core.token.clone() {
                Some(token) => RefreshAction::Session(core.generation, token),
                None => RefreshAction::Probe(core.generation),
            }
        };
        match action {
            RefreshAction::Probe(generation) => {
                self.publish();
                let result = self
                    .inner
                    .api
                    .as_ref()
                    .expect("checked above")
                    .probe()
                    .await;
                self.apply_probe_result(generation, result);
                self.snapshot()
            }
            RefreshAction::Session(generation, token) => {
                self.publish();
                let result = self
                    .inner
                    .api
                    .as_ref()
                    .expect("checked above")
                    .check_session(token.exposed())
                    .await;
                self.apply_refresh_result(generation, token, result).await;
                self.snapshot()
            }
        }
    }

    fn apply_probe_result(&self, generation: u64, result: Result<(), HttpProblem>) {
        if let Ok(mut core) = self.authority_mutation() {
            if core.generation != generation || core.refresh_owner_generation != Some(generation) {
                return;
            }
            core.refresh_owner_generation = None;
            match result {
                Ok(()) => {
                    core.state.status = AuthStatus::SignedOut;
                    core.state.user = None;
                    core.state.session = None;
                    core.state.attempt = None;
                    core.state.storage = None;
                    core.state.message = None;
                }
                Err(problem) => {
                    let (status, message) = refresh_failure(problem);
                    core.state.status = status;
                    core.state.message = Some(message.into());
                }
            }
        }
        self.publish();
    }

    async fn apply_refresh_result(
        &self,
        generation: u64,
        mut token: Secret,
        result: Result<SessionCheck, HttpProblem>,
    ) {
        let mut revoke_after = Vec::new();
        {
            let Ok(mut core) = self.authority_mutation() else {
                return;
            };
            if core.generation != generation || core.refresh_owner_generation != Some(generation) {
                return;
            }
            core.refresh_owner_generation = None;
            match result {
                Ok(check) => {
                    if let Some(renewed) = check.renewed_token {
                        token = Secret::new(renewed);
                    }
                    let mut storage = core.token_storage;
                    let mut session_fallback = false;
                    if storage == Some(AuthStorage::Persistent)
                        && (!core.token_persisted
                            || core
                                .token
                                .as_ref()
                                .is_some_and(|old| old.exposed() != token.exposed()))
                    {
                        let previous_token = core.token.as_ref().cloned();
                        let saved_storage = save_session(&mut core, token.exposed());
                        if saved_storage.is_none() {
                            retire_notification_owner(&core);
                            if let Some(store) = core.store.as_mut() {
                                let _ = store.tombstone();
                            }
                            core.token = None;
                            core.token_persisted = false;
                            core.token_storage = None;
                            core.state.status = AuthStatus::StorageLocked;
                            core.state.user = None;
                            core.state.session = None;
                            core.state.attempt = None;
                            core.state.storage = None;
                            core.state.message = Some(
                            "The account session could not be stored safely. Local access was blocked; remote sign-out will be attempted.".into(),
                        );
                            revoke_after.push(token.clone());
                            if let Some(previous) =
                                previous_token.filter(|old| old.exposed() != token.exposed())
                            {
                                revoke_after.push(previous);
                            }
                        } else {
                            storage = saved_storage;
                            session_fallback = storage == Some(AuthStorage::Session);
                            core.token_storage = storage;
                            core.token_persisted = storage == Some(AuthStorage::Persistent);
                        }
                    }
                    if revoke_after.is_empty() {
                        core.token = Some(token);
                        core.state.status = AuthStatus::SignedIn;
                        core.state.user = Some(check.user);
                        core.state.session = Some(check.session);
                        core.state.attempt = None;
                        core.state.storage = storage;
                        core.state.message =
                            session_fallback.then(|| SESSION_STORAGE_FALLBACK_MESSAGE.into());
                        core.state.remote_revocation_confirmed = None;
                    }
                }
                Err(HttpProblem::Unauthorized) => {
                    retire_notification_owner(&core);
                    let tombstone_ok = tombstone_store(&mut core);
                    clear_account(&mut core);
                    if tombstone_ok {
                        core.state.status = AuthStatus::SignedOut;
                        core.state.message =
                            Some("The account session has expired. Sign in again.".into());
                    } else {
                        core.state.status = AuthStatus::StorageLocked;
                        core.state.message = Some(
                        "The expired session could not be blocked in local storage. Unlock storage and retry.".into(),
                    );
                    }
                }
                Err(HttpProblem::Forbidden) => {
                    retire_notification_owner(&core);
                    core.state.status = AuthStatus::Error;
                    core.state.message = Some("This account cannot access Lomi.".into());
                }
                Err(problem) => {
                    let (status, message) = refresh_failure(problem);
                    core.state.status = status;
                    core.state.message = Some(message.into());
                }
            }
        }
        let mut revocation_confirmed = true;
        for token in &revoke_after {
            revocation_confirmed &= self.revoke_if_possible(token).await;
        }
        if !revoke_after.is_empty() {
            if let Ok(mut core) = self.authority_mutation() {
                if core.generation == generation {
                    core.state.remote_revocation_confirmed = Some(revocation_confirmed);
                }
            }
        }
        self.publish();
    }

    pub async fn sign_out(&self) -> AuthState {
        let (generation, token, tombstone_error, had_session) = {
            let Ok(mut core) = self.authority_mutation() else {
                return self.snapshot();
            };
            retire_notification_owner(&core);
            let generation = core.next_generation();
            core.attempt = None;
            let token = core.token.take();
            let persistent_reference = core.token_storage == Some(AuthStorage::Persistent)
                || core
                    .store
                    .as_ref()
                    .is_some_and(CredentialStore::has_active_reference)
                || (core.store.is_none() && core.state.status == AuthStatus::StorageLocked);
            let had_session = token.is_some() || persistent_reference;
            let tombstone_error = match core.store.as_mut() {
                Some(store) => store.tombstone().err(),
                None if persistent_reference => {
                    Some("Account storage is not available.".to_string())
                }
                None => None,
            };
            core.token_persisted = false;
            core.token_storage = None;
            core.state.user = None;
            core.state.session = None;
            core.state.attempt = None;
            core.state.storage = None;
            core.state.status = if tombstone_error.is_some() {
                AuthStatus::StorageLocked
            } else {
                AuthStatus::SignedOut
            };
            core.state.message = tombstone_error.as_ref().map(|_| {
                "Could not safely persist local sign-out. Unlock storage and retry.".into()
            });
            core.state.remote_revocation_confirmed = None;
            (generation, token, tombstone_error, had_session)
        };
        self.publish();

        let remote_confirmed = match token.as_ref() {
            Some(token) => match self.inner.api.as_ref() {
                Some(api) => Some(api.sign_out(token.exposed()).await),
                None => Some(false),
            },
            None if had_session => Some(false),
            None => None,
        };
        drop(token);

        if let Ok(mut core) = self.authority_mutation() {
            if core.generation == generation {
                core.state.remote_revocation_confirmed = remote_confirmed;
                if tombstone_error.is_none() {
                    if let Some(store) = core.store.as_mut() {
                        if !store.cleanup_journal() {
                            core.state.message = Some(
                                "Signed out locally. System key store cleanup will be retried."
                                    .into(),
                            );
                        } else if had_session && remote_confirmed == Some(false) {
                            core.state.message = Some(
                                "Signed out locally. The remote session may remain active until it expires or is revoked online.".into(),
                            );
                        }
                    } else if had_session && remote_confirmed == Some(false) {
                        core.state.message = Some(
                            "Signed out locally. The remote session may remain active until it expires or is revoked online.".into(),
                        );
                    }
                }
            }
        }
        self.publish()
    }

    pub fn open_account_portal(&self, app: &tauri::AppHandle) -> Result<(), String> {
        let uri = self
            .inner
            .config
            .as_ref()
            .ok_or_else(|| "Account sign-in is unavailable in this build.".to_string())?
            .account_portal_uri()?;
        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .open_url(uri, None::<&str>)
            .map_err(|_| "Cannot open the trusted account portal.".to_string())
    }

    pub fn shutdown(&self) {
        if let Ok(mut core) = self.authority_mutation() {
            if let Some(stop) = core.session_refresh_stop.take() {
                let _ = stop.send(true);
            }
            core.next_generation();
            core.attempt = None;
            core.token = None;
            core.token_storage = None;
            core.state.status = AuthStatus::SignedOut;
            core.state.user = None;
            core.state.session = None;
            core.state.attempt = None;
            core.state.storage = None;
            core.state.message = None;
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "The callback task owns the exact credentials, expiry, and cancellation values for one attempt."
    )]
    async fn await_browser_callback_and_exchange(
        &self,
        attempt_id: String,
        generation: u64,
        listener: tokio::net::TcpListener,
        state: Secret,
        code_verifier: Secret,
        redirect_uri: String,
        expires_at: Instant,
        storage: AuthStorage,
        cancel: tokio::sync::watch::Receiver<bool>,
    ) {
        let Some(api) = self.inner.api.as_ref() else {
            return;
        };
        let mut pending = match callback::wait_for_callback(
            listener,
            state.exposed(),
            expires_at,
            cancel,
        )
        .await
        {
            Ok(pending) => pending,
            Err(CallbackError::Cancelled) => return,
            Err(CallbackError::Expired) => {
                self.finish_attempt(
                    generation,
                    &attempt_id,
                    AuthStatus::SignedOut,
                    "Sign-in expired. Start again to open a new browser request.",
                );
                return;
            }
            Err(CallbackError::Invalid) => {
                self.finish_attempt(
                    generation,
                    &attempt_id,
                    AuthStatus::Error,
                    "The local sign-in callback could not be completed. Start again to retry.",
                );
                return;
            }
        };
        if !self.is_current_attempt(generation, &attempt_id) {
            pending.complete(false).await;
            return;
        }
        let code = Secret::new(std::mem::take(&mut pending.code));
        match api
            .exchange_desktop_code(code.exposed(), code_verifier.exposed(), &redirect_uri)
            .await
        {
            Ok(token) => {
                let activated = self
                    .complete_device_token(token, generation, &attempt_id, storage)
                    .await;
                pending
                    .complete(activated && self.is_signed_in_generation(generation))
                    .await;
            }
            Err(problem) => {
                let (status, message) = match problem {
                    HttpProblem::Transport | HttpProblem::Temporary | HttpProblem::RateLimited(_) => (
                        AuthStatus::Offline,
                        "The account service could not finish sign-in. Start again to request a new browser session.",
                    ),
                    _ => (
                        AuthStatus::Error,
                        "The account service could not finish sign-in. Start again to request a new browser session.",
                    ),
                };
                self.finish_attempt(generation, &attempt_id, status, message);
                pending.complete(false).await;
            }
        }
    }

    async fn complete_device_token(
        &self,
        token: String,
        generation: u64,
        attempt_id: &str,
        storage: AuthStorage,
    ) -> bool {
        let mut token = Secret::new(token);
        if !self.is_current_attempt(generation, attempt_id) {
            self.revoke_if_possible(&token).await;
            return false;
        }
        let Some(api) = self.inner.api.as_ref() else {
            self.finish_attempt(
                generation,
                attempt_id,
                AuthStatus::Unavailable,
                "Account sign-in is unavailable in this build.",
            );
            return false;
        };
        match api.check_session(token.exposed()).await {
            Ok(check) => {
                if let Some(renewed) = check.renewed_token {
                    token = Secret::new(renewed);
                }
                let revoke_token = token.clone();
                let activation = if let Ok(mut core) = self.authority_mutation() {
                    if !core.current_attempt(generation, attempt_id) {
                        SessionActivation::Stale
                    } else {
                        let saved_storage = if storage == AuthStorage::Persistent {
                            save_session(&mut core, token.exposed())
                        } else {
                            Some(storage)
                        };
                        if let Some(saved_storage) = saved_storage {
                            let session_fallback = saved_storage != storage;
                            let storage = saved_storage;
                            let cleanup_pending = storage == AuthStorage::Persistent
                                && core
                                    .store
                                    .as_mut()
                                    .is_some_and(|store| !store.cleanup_journal());
                            core.active_attempt_id = None;
                            core.attempt = None;
                            core.token = Some(token);
                            core.token_storage = Some(storage);
                            core.token_persisted = storage == AuthStorage::Persistent;
                            core.state.status = AuthStatus::SignedIn;
                            core.state.user = Some(check.user);
                            core.state.session = Some(check.session);
                            core.state.attempt = None;
                            core.state.storage = Some(storage);
                            core.state.message = if session_fallback {
                                Some(SESSION_STORAGE_FALLBACK_MESSAGE.into())
                            } else {
                                cleanup_pending.then(|| {
                                    "Signed in. Cleanup of an older account credential will be retried.".into()
                                })
                            };
                            core.state.remote_revocation_confirmed = None;
                            SessionActivation::Activated
                        } else {
                            if let Some(store) = core.store.as_mut() {
                                let _ = store.tombstone();
                            }
                            core.active_attempt_id = None;
                            core.attempt = None;
                            core.token = None;
                            core.token_storage = None;
                            core.token_persisted = false;
                            core.state.status = AuthStatus::StorageLocked;
                            core.state.user = None;
                            core.state.session = None;
                            core.state.attempt = None;
                            core.state.storage = None;
                            core.state.message = Some(
                                "The account was confirmed, but local session storage could not be updated safely. Check storage access and retry.".into(),
                            );
                            SessionActivation::StorageFailure
                        }
                    }
                } else {
                    SessionActivation::Stale
                };
                let activated = matches!(activation, SessionActivation::Activated);
                match activation {
                    SessionActivation::Activated => {
                        drop(revoke_token);
                        self.publish();
                        self.focus_after_login(generation);
                    }
                    SessionActivation::Stale => {
                        self.revoke_if_possible(&revoke_token).await;
                    }
                    SessionActivation::StorageFailure => {
                        let revoked = self.revoke_if_possible(&revoke_token).await;
                        if let Ok(mut core) = self.authority_mutation() {
                            if core.generation == generation {
                                core.state.remote_revocation_confirmed = Some(revoked);
                            }
                        }
                        self.publish();
                    }
                }
                activated
            }
            Err(HttpProblem::Transport | HttpProblem::Temporary | HttpProblem::RateLimited(_)) => {
                let accepted = {
                    if let Ok(mut core) = self.authority_mutation() {
                        if core.current_attempt(generation, attempt_id) {
                            core.active_attempt_id = None;
                            core.attempt = None;
                            core.token = Some(token.clone());
                            core.token_storage = Some(storage);
                            core.token_persisted = false;
                            core.state.status = AuthStatus::Offline;
                            core.state.user = None;
                            core.state.session = None;
                            core.state.attempt = None;
                            core.state.storage =
                                (storage == AuthStorage::Session).then_some(storage);
                            core.state.message = Some(
                            "The session was issued, but the account service could not confirm it. Check your connection and retry.".into(),
                        );
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };
                if !accepted {
                    self.revoke_if_possible(&token).await;
                    return false;
                }
                drop(token);
                self.publish();
                false
            }
            Err(HttpProblem::Unauthorized) => {
                self.finish_attempt(
                    generation,
                    attempt_id,
                    AuthStatus::SignedOut,
                    "The issued session could not be confirmed. Start sign-in again.",
                );
                self.revoke_if_possible(&token).await;
                false
            }
            Err(HttpProblem::Forbidden) => {
                self.finish_attempt(
                    generation,
                    attempt_id,
                    AuthStatus::Error,
                    "This account cannot access Lomi.",
                );
                self.revoke_if_possible(&token).await;
                false
            }
            Err(_) => {
                let accepted = {
                    if let Ok(mut core) = self.authority_mutation() {
                        if core.current_attempt(generation, attempt_id) {
                            core.active_attempt_id = None;
                            core.attempt = None;
                            core.token = Some(token.clone());
                            core.token_storage = Some(storage);
                            core.token_persisted = false;
                            core.state.status = AuthStatus::Error;
                            core.state.user = None;
                            core.state.session = None;
                            core.state.attempt = None;
                            core.state.storage =
                                (storage == AuthStorage::Session).then_some(storage);
                            core.state.message = Some(
                            "The account service returned an invalid response. Retry the session check.".into(),
                        );
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };
                if !accepted {
                    self.revoke_if_possible(&token).await;
                    return false;
                }
                drop(token);
                self.publish();
                false
            }
        }
    }

    async fn revoke_if_possible(&self, token: &Secret) -> bool {
        if let Some(api) = self.inner.api.as_ref() {
            api.sign_out(token.exposed()).await
        } else {
            false
        }
    }

    fn focus_after_login(&self, generation: u64) {
        let app = self.inner.core.lock().ok().and_then(|core| {
            (core.generation == generation && core.state.status == AuthStatus::SignedIn)
                .then(|| core.app.clone())
                .flatten()
        });
        let Some(app) = app else {
            return;
        };
        let Some(window) = app.get_window("main") else {
            return;
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        let _ = app.run_on_main_thread(move || {
            if Instant::now() < deadline {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        });
    }

    fn is_current_attempt(&self, generation: u64, attempt_id: &str) -> bool {
        self.inner
            .core
            .lock()
            .is_ok_and(|core| core.current_attempt(generation, attempt_id))
    }

    fn is_signed_in_generation(&self, generation: u64) -> bool {
        self.inner.core.lock().is_ok_and(|core| {
            core.generation == generation
                && core.state.status == AuthStatus::SignedIn
                && core.token.is_some()
                && core.state.user.is_some()
                && core.state.session.is_some()
        })
    }

    fn finish_attempt(&self, generation: u64, attempt_id: &str, status: AuthStatus, message: &str) {
        if let Ok(mut core) = self.authority_mutation() {
            if core.current_attempt(generation, attempt_id) {
                core.active_attempt_id = None;
                core.attempt = None;
                core.state.status = status;
                core.state.user = None;
                core.state.session = None;
                core.state.attempt = None;
                core.state.storage = None;
                core.state.message = Some(message.into());
            }
        }
        self.publish();
    }

    fn set_error(&self, status: AuthStatus, message: &str) {
        if let Ok(mut core) = self.authority_mutation() {
            core.state.status = status;
            core.state.message = Some(message.into());
        }
        self.publish();
    }

    fn publish(&self) -> AuthState {
        let snapshot = {
            let Ok(mut core) = self.authority_mutation() else {
                return self.snapshot();
            };
            core.state.revision = core.state.revision.wrapping_add(1);
            let snapshot = core.state.clone();
            let app = core.app.clone();
            (snapshot, app)
        };
        if let Some(app) = snapshot.1 {
            let _ = app.emit_to(
                EventTarget::webview("main"),
                STATE_CHANGED_EVENT,
                &snapshot.0,
            );
        }
        snapshot.0
    }
}

const SESSION_STORAGE_FALLBACK_MESSAGE: &str = "The system credential store is unavailable. Signed in for this session only; sign in again after closing Lomi.";

fn save_session(core: &mut Core, token: &str) -> Option<AuthStorage> {
    let store = core.store.as_mut()?;
    match store.save_active(token) {
        Ok(_) => Some(AuthStorage::Persistent),
        // Fence every journaled credential durably before keeping access in memory.
        Err(CredentialSaveError::KeyStore) if store.tombstone().is_ok() => {
            Some(AuthStorage::Session)
        }
        Err(_) => None,
    }
}

fn tombstone_store(core: &mut Core) -> bool {
    match core.store.as_mut() {
        Some(store) => store.tombstone().is_ok(),
        None => true,
    }
}

fn retire_notification_owner(core: &Core) {
    if let (Some(app), Some(user), Some(session)) = (
        core.app.as_ref(),
        core.state.user.as_ref(),
        core.state.session.as_ref(),
    ) {
        // The module fences capture even when its marker cannot be persisted.
        let _ = crate::push::retire_owner(app, &user.id, &session.id);
    }
}

fn clear_account(core: &mut Core) {
    core.token = None;
    core.token_storage = None;
    core.token_persisted = false;
    core.state.user = None;
    core.state.session = None;
    core.state.attempt = None;
    core.state.storage = None;
}

fn desktop_start_message(problem: HttpProblem) -> &'static str {
    match problem {
        HttpProblem::Transport | HttpProblem::Temporary | HttpProblem::RateLimited(_) => {
            "The account service is unavailable. Check your connection and retry."
        }
        _ => "The account service could not start sign-in. Please retry later.",
    }
}

fn remote_request_message(problem: HttpProblem) -> &'static str {
    match problem {
        HttpProblem::DesktopUpdateRequired => "Update Lomi to share workspaces remotely.",
        HttpProblem::Transport => {
            "Cannot connect to the Remote service. Check your connection and retry."
        }
        HttpProblem::Temporary => {
            "The Remote service is temporarily unavailable. Try again shortly."
        }
        HttpProblem::RateLimited(_) => "Too many Remote requests. Wait a moment and retry.",
        HttpProblem::Unauthorized => {
            "The Remote service could not verify your account session. Check Account settings and retry."
        }
        HttpProblem::Forbidden => "The Remote service denied access to this account.",
        HttpProblem::InvalidResponse => {
            "The Remote service returned an invalid response. Try again later."
        }
        HttpProblem::Rejected => "The Remote service rejected the request. Try again later.",
    }
}

fn refresh_failure(problem: HttpProblem) -> (AuthStatus, &'static str) {
    match problem {
        HttpProblem::Transport | HttpProblem::Temporary | HttpProblem::RateLimited(_) => (
            AuthStatus::Offline,
            "The account service is unavailable. Your current session has not been confirmed.",
        ),
        HttpProblem::Forbidden => (AuthStatus::Error, "This account cannot access Lomi."),
        _ => (
            AuthStatus::Error,
            "The account service returned an invalid response.",
        ),
    }
}

fn create_pkce_pair() -> Result<(Secret, Secret, String), String> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

    let random = SystemRandom::new();
    let mut state_bytes = [0_u8; 32];
    let mut verifier_bytes = [0_u8; 32];
    if random.fill(&mut state_bytes).is_err() {
        state_bytes.fill(0);
        verifier_bytes.fill(0);
        return Err("Cannot create a secure sign-in request.".into());
    }
    if random.fill(&mut verifier_bytes).is_err() {
        state_bytes.fill(0);
        verifier_bytes.fill(0);
        return Err("Cannot create a secure sign-in request.".into());
    }
    let state = Secret::new(URL_SAFE_NO_PAD.encode(state_bytes));
    let verifier = Secret::new(URL_SAFE_NO_PAD.encode(verifier_bytes));
    state_bytes.fill(0);
    verifier_bytes.fill(0);
    let challenge = URL_SAFE_NO_PAD.encode(digest(&SHA256, verifier.exposed().as_bytes()).as_ref());
    Ok((state, verifier, challenge))
}

fn random_id() -> Result<String, String> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let mut bytes = [0_u8; 18];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "Cannot create a sign-in attempt identifier.".to_string())?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn format_timestamp(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (seconds / 86_400) as i64;
    let day_seconds = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        day_seconds / 3600,
        (day_seconds % 3600) / 60,
        day_seconds % 60
    )
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const TEST_CALLBACK_CODE: &str = "A123456789012345678901234567890123456789012";
    const TEST_CALLBACK_STATE: &str = "B123456789012345678901234567890123456789012";
    const TEST_VERIFIER: &str = "C123456789012345678901234567890123456789012";
    const ACTIVE_SESSION_JSON: &str = r#"{"user":{"id":"user-1","displayName":"Ada","email":"ada@example.com","githubLogin":"ada","status":"active"},"session":{"id":"session-1","expiresAt":"2026-10-01T00:00:00Z"}}"#;

    fn authorized_core() -> Core {
        let mut core = Core::new(AuthStatus::SignedIn, None);
        core.token = Some(Secret::new("session-token".into()));
        core.token_storage = Some(AuthStorage::Session);
        core.state.user = Some(super::super::AuthUser {
            id: "user-1".into(),
            display_name: "Ada".into(),
            email: "".into(),
            github_login: "".into(),
            status: "active".into(),
        });
        core.state.session = Some(super::super::AuthSession {
            id: "session-1".into(),
            expires_at: format_timestamp(SystemTime::now() + Duration::from_secs(120)),
        });
        core
    }

    #[test]
    fn automatic_session_refresh_requires_existing_unblocked_idle_credentials() {
        let mut core = authorized_core();
        for status in [AuthStatus::SignedIn, AuthStatus::Offline] {
            core.state.status = status;
            assert!(core.can_refresh_session_automatically());
        }
        for status in [
            AuthStatus::Unavailable,
            AuthStatus::SignedOut,
            AuthStatus::Authorizing,
            AuthStatus::Checking,
            AuthStatus::StorageLocked,
            AuthStatus::Error,
        ] {
            core.state.status = status;
            assert!(!core.can_refresh_session_automatically());
        }
        core.state.status = AuthStatus::SignedIn;
        core.state.session.as_mut().unwrap().expires_at =
            format_timestamp(SystemTime::now() - Duration::from_secs(1));
        assert!(core.can_refresh_session_automatically());
        core.active_attempt_id = Some("login".into());
        assert!(!core.can_refresh_session_automatically());
        core.active_attempt_id = None;
        core.refresh_owner_generation = Some(core.generation);
        assert!(!core.can_refresh_session_automatically());
        core.refresh_owner_generation = None;
        core.token = None;
        assert!(!core.can_refresh_session_automatically());
    }

    #[test]
    fn local_notifications_require_retained_verified_owner_and_future_expiry() {
        let mut core = authorized_core();
        let original = local_notification_binding(&core).unwrap();
        core.state.status = AuthStatus::Offline;
        assert!(local_notification_binding(&core)
            .unwrap()
            .same_authority(&original));
        assert!(remote_binding(&core).is_err());
        for status in [
            AuthStatus::Checking,
            AuthStatus::Authorizing,
            AuthStatus::Error,
            AuthStatus::StorageLocked,
            AuthStatus::SignedOut,
            AuthStatus::Unavailable,
        ] {
            core.state.status = status;
            assert!(local_notification_binding(&core).is_err());
        }
        core.state.status = AuthStatus::Offline;
        core.active_attempt_id = Some("login-in-flight".into());
        assert!(local_notification_binding(&core).is_err());
        core.active_attempt_id = None;
        core.state.user.as_mut().unwrap().status = "suspended".into();
        assert!(local_notification_binding(&core).is_err());
        core.state.user.as_mut().unwrap().status = "active".into();
        core.state.session.as_mut().unwrap().expires_at =
            format_timestamp(SystemTime::now() - Duration::from_secs(1));
        assert!(local_notification_binding(&core).is_err());
        core = authorized_core();
        core.token = None;
        assert!(local_notification_binding(&core).is_err());
        core = authorized_core();
        core.state.status = AuthStatus::Offline;
        core.state.user = None;
        core.state.session = None;
        assert!(local_notification_binding(&core).is_err());
    }

    #[test]
    fn local_emission_guard_serializes_retirement_before_owner_mutation() {
        let controller = controller_for_origin("http://127.0.0.1:9".into(), AuthStatus::SignedIn);
        *controller.inner.core.lock().unwrap() = authorized_core();
        let guard = LOCAL_NOTIFICATION_EMISSION_GUARD.lock().unwrap();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let cloned = controller.clone();
        let retirement = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            let mut core = cloned.authority_mutation().unwrap();
            core.next_generation();
            clear_account(&mut core);
            core.state.status = AuthStatus::SignedOut;
            done_tx.send(()).unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(done_rx.try_recv().is_err());
        assert!(controller.local_notification_binding().is_ok());
        drop(guard);
        done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        retirement.join().unwrap();
        assert!(controller.local_notification_binding().is_err());
    }

    #[tokio::test]
    async fn bound_push_rejects_owner_change_before_http_and_stale_ack_after_http() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let (received_tx, received_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_complete_http_request(&mut stream);
            received_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            write_api_response(&mut stream, "200 OK", r#"{"enabled":true}"#);
            request
        });
        let mut controller = controller_for_origin(origin.clone(), AuthStatus::SignedIn);
        Arc::get_mut(&mut controller.inner)
            .unwrap()
            .api
            .as_mut()
            .unwrap()
            .set_test_remote_origin(origin);
        *controller.inner.core.lock().unwrap() = authorized_core();
        let owner = controller.remote_binding().unwrap();
        let mut wrong = owner.clone();
        wrong.generation += 1;
        assert_eq!(
            controller
                .push_request_bound(
                    reqwest::Method::POST,
                    "/v1/remote/native/push/events",
                    Some(&serde_json::json!({})),
                    &wrong
                )
                .await
                .unwrap_err()
                .code,
            "AUTHORITY_CHANGED"
        );
        let cloned = controller.clone();
        let request = tokio::spawn(async move {
            cloned
                .push_request_bound(
                    reqwest::Method::POST,
                    "/v1/remote/native/push/events",
                    Some(&serde_json::json!({"eventKey":"test-only"})),
                    &owner,
                )
                .await
        });
        received_rx.await.unwrap();
        {
            let mut core = controller.authority_mutation().unwrap();
            core.next_generation();
            core.state.user.as_mut().unwrap().id = "user-b".into();
            core.state.session.as_mut().unwrap().id = "session-b".into();
        }
        release_tx.send(()).unwrap();
        assert_eq!(
            request.await.unwrap().unwrap_err().code,
            "AUTHORITY_CHANGED"
        );
        assert!(server
            .join()
            .unwrap()
            .contains("authorization: Bearer session-token"));
    }

    #[tokio::test]
    async fn denied_push_session_retires_local_authority_but_feature_off_does_not() {
        for (status, code, retired) in [
            ("401 Unauthorized", "SESSION_REQUIRED", true),
            ("403 Forbidden", "ACCOUNT_UNAVAILABLE", true),
            ("403 Forbidden", "FEATURE_DISABLED", false),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                read_complete_http_request(&mut stream);
                write_api_response(&mut stream, status, &format!(r#"{{"error":"{code}"}}"#));
            });
            let mut controller = controller_for_origin(origin.clone(), AuthStatus::SignedIn);
            Arc::get_mut(&mut controller.inner)
                .unwrap()
                .api
                .as_mut()
                .unwrap()
                .set_test_remote_origin(origin);
            *controller.inner.core.lock().unwrap() = authorized_core();
            assert_eq!(
                controller
                    .push_request(
                        reqwest::Method::GET,
                        "/v1/remote/native/push/preferences",
                        None
                    )
                    .await
                    .unwrap_err()
                    .code,
                code
            );
            assert_eq!(controller.local_notification_binding().is_err(), retired);
            server.join().unwrap();
        }
    }

    #[tokio::test]
    async fn delayed_push_denial_retires_offline_owner_and_preserves_replacements() {
        for replacement in ["offline", "account", "credential"] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let (received_tx, received_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                read_complete_http_request(&mut stream);
                received_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                write_api_response(
                    &mut stream,
                    "401 Unauthorized",
                    r#"{"error":"SESSION_REQUIRED"}"#,
                );
            });
            let mut controller = controller_for_origin(origin.clone(), AuthStatus::SignedIn);
            Arc::get_mut(&mut controller.inner)
                .unwrap()
                .api
                .as_mut()
                .unwrap()
                .set_test_remote_origin(origin);
            *controller.inner.core.lock().unwrap() = authorized_core();
            let cloned = controller.clone();
            let request = tokio::spawn(async move {
                cloned
                    .push_request(
                        reqwest::Method::GET,
                        "/v1/remote/native/push/preferences",
                        None,
                    )
                    .await
            });
            received_rx.await.unwrap();
            {
                let mut core = controller.authority_mutation().unwrap();
                match replacement {
                    "offline" => core.state.status = AuthStatus::Offline,
                    "account" => {
                        core.next_generation();
                        core.state.user.as_mut().unwrap().id = "user-b".into();
                        core.state.session.as_mut().unwrap().id = "session-b".into();
                    }
                    _ => core.token = Some(Secret::new("newly-verified-token".into())),
                }
            }
            release_tx.send(()).unwrap();
            let error = request.await.unwrap().unwrap_err();
            if replacement == "offline" {
                assert_eq!(error.code, "SESSION_REQUIRED");
                assert!(controller.local_notification_binding().is_err());
                assert!(controller.inner.core.lock().unwrap().token.is_none());
            } else {
                assert_eq!(error.code, "AUTHORITY_CHANGED");
                assert!(controller.local_notification_binding().is_ok());
                if replacement == "account" {
                    assert_eq!(controller.snapshot().user.unwrap().id, "user-b");
                } else {
                    assert_eq!(
                        controller
                            .inner
                            .core
                            .lock()
                            .unwrap()
                            .token
                            .as_ref()
                            .unwrap()
                            .exposed(),
                        "newly-verified-token"
                    );
                }
            }
            server.join().unwrap();
        }
    }

    #[tokio::test]
    async fn automatic_checks_renew_expiry_and_recover_after_temporary_outage() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let first_expiry = format_timestamp(SystemTime::now() + Duration::from_secs(240));
        let second_expiry = format_timestamp(SystemTime::now() + Duration::from_secs(480));
        let bodies = [
            ACTIVE_SESSION_JSON.replace("2026-10-01T00:00:00Z", &first_expiry),
            "{}".into(),
            ACTIVE_SESSION_JSON.replace("2026-10-01T00:00:00Z", &second_expiry),
        ];
        let server = thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in ["200 OK", "503 Service Unavailable", "200 OK"]
                .into_iter()
                .zip(bodies)
            {
                let (mut stream, _) = listener.accept().unwrap();
                requests.push(read_complete_http_request(&mut stream));
                write_api_response(&mut stream, status, &body);
            }
            requests
        });
        let controller = controller_for_origin(origin, AuthStatus::SignedIn);
        *controller.inner.core.lock().unwrap() = authorized_core();
        let previous = controller.remote_binding().unwrap();
        controller
            .inner
            .core
            .lock()
            .unwrap()
            .state
            .session
            .as_mut()
            .unwrap()
            .expires_at = format_timestamp(SystemTime::now() - Duration::from_secs(1));
        assert_eq!(controller.snapshot().status, AuthStatus::SignedIn);
        assert!(controller.remote_binding().is_err());
        assert_eq!(
            controller.refresh_state_inner(true).await.status,
            AuthStatus::SignedIn
        );
        let renewed = controller.remote_binding().unwrap();
        assert!(renewed.same_authority(&previous));
        assert!(renewed.expires_at > previous.expires_at);
        controller.inner.core.lock().unwrap().refresh_started =
            Some(Instant::now() - Duration::from_secs(60));
        assert_eq!(
            controller.refresh_state_inner(true).await.status,
            AuthStatus::Offline
        );
        assert!(controller.remote_binding().is_err());
        {
            let mut core = controller.inner.core.lock().unwrap();
            assert!(core.token.is_some());
            core.refresh_started = Some(Instant::now() - Duration::from_secs(60));
            core.state.session.as_mut().unwrap().expires_at =
                format_timestamp(SystemTime::now() - Duration::from_secs(1));
        }
        assert_eq!(
            controller.refresh_state_inner(true).await.status,
            AuthStatus::SignedIn
        );
        let recovered = controller.remote_binding().unwrap();
        assert!(recovered.same_authority(&previous));
        assert!(recovered.expires_at > renewed.expires_at);
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests
            .iter()
            .all(|request| request.starts_with("GET /v1/me ")
                && request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer session-token")));
    }

    #[tokio::test]
    async fn automatic_checks_do_not_reload_blocked_storage_or_probe_signed_out() {
        let root = tempfile::tempdir().unwrap();
        let controller =
            controller_for_origin("http://127.0.0.1:9".into(), AuthStatus::StorageLocked);
        attach_unreadable_store(&controller, root.path());
        let before = controller.snapshot();
        let after = controller.refresh_state_inner(true).await;
        assert_eq!(after.status, before.status);
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.message, before.message);
        {
            let mut core = controller.inner.core.lock().unwrap();
            core.state.status = AuthStatus::SignedOut;
            core.store = None;
        }
        assert_eq!(
            controller.refresh_state_inner(true).await.status,
            AuthStatus::SignedOut
        );
        assert!(controller
            .inner
            .core
            .lock()
            .unwrap()
            .refresh_started
            .is_none());
    }

    #[test]
    fn remote_binding_renewal_retains_authority_and_checks_live_access() {
        let mut core = authorized_core();
        let previous = remote_binding(&core).unwrap();
        core.state.session.as_mut().unwrap().expires_at =
            format_timestamp(SystemTime::now() + Duration::from_secs(240));
        let renewed = remote_binding(&core).unwrap();
        assert!(previous.same_authority(&renewed));
        assert_ne!(previous, renewed);
        assert!(renewed.expires_at > previous.expires_at);
        core.next_generation();
        assert!(!previous.same_authority(&remote_binding(&core).unwrap()));
        core.state.session.as_mut().unwrap().expires_at =
            format_timestamp(SystemTime::now() - Duration::from_secs(1));
        assert!(remote_binding(&core).is_err());
        core = authorized_core();
        core.state.user.as_mut().unwrap().status = "disabled".into();
        assert!(remote_binding(&core).is_err());
        core = authorized_core();
        core.token = None;
        assert!(remote_binding(&core).is_err());
        core = authorized_core();
        core.state.status = AuthStatus::Offline;
        assert!(remote_binding(&core).is_err());
    }

    #[tokio::test]
    async fn pending_refresh_retains_live_authority_then_applies_renewal_or_denial() {
        for denied in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let (requested_tx, requested_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let expiry = format_timestamp(SystemTime::now() + Duration::from_secs(240));
            let body = ACTIVE_SESSION_JSON.replace("2026-10-01T00:00:00Z", &expiry);
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_complete_http_request(&mut stream);
                requested_tx.send(request).unwrap();
                release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
                write_api_response(
                    &mut stream,
                    if denied { "401 Unauthorized" } else { "200 OK" },
                    if denied { "{}" } else { &body },
                );
            });
            let controller = controller_for_origin(origin, AuthStatus::SignedIn);
            *controller.inner.core.lock().unwrap() = authorized_core();
            let previous = controller.remote_binding().unwrap();
            let refreshing = controller.clone();
            let task = tokio::spawn(async move { refreshing.refresh_state().await });
            let request = tokio::time::timeout(Duration::from_secs(3), requested_rx)
                .await
                .unwrap()
                .unwrap();
            assert!(request.starts_with("GET /v1/me "));
            assert_eq!(controller.snapshot().status, AuthStatus::SignedIn);
            assert_eq!(controller.remote_binding().unwrap(), previous);
            release_tx.send(()).unwrap();
            let state = task.await.unwrap();
            if denied {
                assert_eq!(state.status, AuthStatus::SignedOut);
                assert!(controller.remote_binding().is_err());
            } else {
                assert_eq!(state.status, AuthStatus::SignedIn);
                let current = controller.remote_binding().unwrap();
                assert!(current.same_authority(&previous));
                assert!(current.expires_at > previous.expires_at);
            }
            server.join().unwrap();
        }
    }

    struct DelayedDesktopServer {
        origin: String,
        session_request: std::sync::mpsc::Receiver<String>,
        release_session: std::sync::mpsc::Sender<()>,
        worker: thread::JoinHandle<Vec<String>>,
    }

    struct BrowserCallbackRun {
        browser: tokio::net::TcpStream,
        worker: tokio::task::JoinHandle<()>,
        generation: u64,
        cancel_observer: tokio::sync::watch::Receiver<bool>,
    }

    fn delayed_desktop_server(expect_revocation: bool) -> DelayedDesktopServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (session_request_tx, session_request_rx) = std::sync::mpsc::channel();
        let (release_session_tx, release_session_rx) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let (mut exchange_stream, _) = listener.accept().unwrap();
            let exchange_request = read_complete_http_request(&mut exchange_stream);
            write_api_response(
                &mut exchange_stream,
                "200 OK",
                r#"{"access_token":"issued-session","token_type":"Bearer","expires_in":900}"#,
            );

            let (mut session_stream, _) = listener.accept().unwrap();
            let session_request = read_complete_http_request(&mut session_stream);
            session_request_tx.send(session_request.clone()).unwrap();
            release_session_rx.recv().unwrap();
            write_api_response(&mut session_stream, "200 OK", ACTIVE_SESSION_JSON);

            let mut requests = vec![exchange_request, session_request];
            if expect_revocation {
                let (mut revoke_stream, _) = listener.accept().unwrap();
                requests.push(read_complete_http_request(&mut revoke_stream));
                write_api_response(&mut revoke_stream, "200 OK", "{}");
            }
            requests
        });
        DelayedDesktopServer {
            origin: format!("http://{address}"),
            session_request: session_request_rx,
            release_session: release_session_tx,
            worker,
        }
    }

    fn read_complete_http_request(stream: &mut std::net::TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 2048];
        let mut header_end = None;
        let mut content_length = 0;
        loop {
            let count = stream.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..count]);
            if header_end.is_none() {
                if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    let end = position + 4;
                    let headers = String::from_utf8_lossy(&request[..end]);
                    content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    header_end = Some(end);
                }
            }
            if header_end.is_some_and(|end| request.len() >= end + content_length) {
                break;
            }
        }
        String::from_utf8(request).unwrap()
    }

    fn write_api_response(stream: &mut std::net::TcpStream, status: &str, body: &str) {
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
    }

    fn start_browser_attempt(
        controller: &AuthController,
        attempt_id: &str,
        storage: AuthStorage,
    ) -> (u64, tokio::sync::watch::Receiver<bool>) {
        let expires_at = Instant::now() + Duration::from_secs(60);
        let expires_at_text = "2026-10-01T00:00:00Z".to_string();
        let (cancel, cancel_receiver) = tokio::sync::watch::channel(false);
        let cancel_observer = cancel_receiver.clone();
        let mut core = controller.inner.core.lock().unwrap();
        let generation = core.start_attempt(attempt_id.to_string());
        core.state.status = AuthStatus::Authorizing;
        core.state.storage = Some(storage);
        core.state.attempt = Some(AuthAttempt {
            id: attempt_id.to_string(),
            expires_at: expires_at_text.clone(),
        });
        core.attempt = Some(BrowserAttempt {
            id: attempt_id.to_string(),
            authorization_url: String::new(),
            cancel,
            expires_at,
            expires_at_text,
        });
        (generation, cancel_observer)
    }

    async fn begin_browser_callback(
        controller: &AuthController,
        attempt_id: &str,
        storage: AuthStorage,
    ) -> BrowserCallbackRun {
        let listener = callback::bind_loopback().await.unwrap();
        let address = listener.local_addr().unwrap();
        let redirect_uri = callback::redirect_uri(&listener).unwrap();
        let expires_at = Instant::now() + Duration::from_secs(60);
        let (generation, cancel_observer) = start_browser_attempt(controller, attempt_id, storage);
        let callback_cancel = cancel_observer.clone();
        let attempt_id = attempt_id.to_string();
        let worker_controller = controller.clone();
        let worker = tokio::spawn(async move {
            worker_controller
                .await_browser_callback_and_exchange(
                    attempt_id,
                    generation,
                    listener,
                    Secret::new(TEST_CALLBACK_STATE.to_string()),
                    Secret::new(TEST_VERIFIER.to_string()),
                    redirect_uri,
                    expires_at,
                    storage,
                    callback_cancel,
                )
                .await;
        });
        let mut browser = tokio::net::TcpStream::connect(address).await.unwrap();
        let request = format!(
            "GET /auth/callback?code={TEST_CALLBACK_CODE}&state={TEST_CALLBACK_STATE} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            address.port()
        );
        browser.write_all(request.as_bytes()).await.unwrap();
        BrowserCallbackRun {
            browser,
            worker,
            generation,
            cancel_observer,
        }
    }

    fn health_server(statuses: Vec<&'static str>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            statuses
                .into_iter()
                .map(|status| {
                    let (mut stream, _) = listener.accept().unwrap();
                    let mut request = Vec::new();
                    let mut buffer = [0_u8; 512];
                    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                        let count = stream.read(&mut buffer).unwrap();
                        if count == 0 {
                            break;
                        }
                        request.extend_from_slice(&buffer[..count]);
                    }
                    let request = String::from_utf8(request).unwrap();
                    let body = "{}";
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).unwrap();
                    request
                })
                .collect()
        });
        (format!("http://{address}"), worker)
    }

    struct DelayedHealthServer {
        origin: String,
        observed: std::sync::mpsc::Receiver<String>,
        release_old: std::sync::mpsc::Sender<()>,
        release_new: std::sync::mpsc::Sender<()>,
        worker: thread::JoinHandle<Vec<String>>,
    }

    fn delayed_health_server() -> DelayedHealthServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = std::sync::mpsc::channel();
        let (release_old_tx, release_old_rx) = std::sync::mpsc::channel();
        let (release_new_tx, release_new_rx) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let (mut old_stream, _) = listener.accept().unwrap();
            let old_request = read_probe_request(&mut old_stream);
            request_tx.send(old_request.clone()).unwrap();

            let (mut new_stream, _) = listener.accept().unwrap();
            let new_request = read_probe_request(&mut new_stream);
            request_tx.send(new_request.clone()).unwrap();

            release_old_rx.recv().unwrap();
            respond_probe(&mut old_stream, "503 Service Unavailable");
            release_new_rx.recv().unwrap();
            respond_probe(&mut new_stream, "200 OK");
            vec![old_request, new_request]
        });
        DelayedHealthServer {
            origin: format!("http://{address}"),
            observed: request_rx,
            release_old: release_old_tx,
            release_new: release_new_tx,
            worker,
        }
    }

    fn read_probe_request(stream: &mut std::net::TcpStream) -> String {
        let mut request = Vec::new();
        let mut buffer = [0_u8; 512];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..count]);
        }
        String::from_utf8(request).unwrap()
    }

    fn respond_probe(stream: &mut std::net::TcpStream, status: &str) {
        let body = "{}";
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
    }

    fn controller_for_origin(origin: String, status: AuthStatus) -> AuthController {
        let config = AuthConfig {
            origin,
            client_id: "lomi-desktop-test".into(),
            environment: "test".into(),
        };
        let api = ApiClient::new(config.clone()).unwrap();
        AuthController {
            inner: Arc::new(Inner {
                config: Some(config),
                api: Some(api),
                config_message: None,
                core: Mutex::new(Core::new(status, None)),
            }),
        }
    }

    fn attach_unreadable_store(controller: &AuthController, root: &Path) {
        std::fs::write(
            root.join("credentials.json"),
            br#"{"version":1,"activeId":"credential-1","journalIds":[],"logoutTombstone":false}"#,
        )
        .unwrap();
        let mut store = CredentialStore::open(root, "test-unreadable-reference").unwrap();
        store.deny_key_store_access();
        let mut core = controller.inner.core.lock().unwrap();
        core.store = Some(store);
        core.state.message =
            Some("The account session is unavailable in the system key store.".into());
    }

    #[tokio::test]
    async fn persistent_sign_in_preserves_unreadable_reference_and_its_diagnosis() {
        let temp = tempfile::tempdir().unwrap();
        let controller =
            controller_for_origin("https://auth.lomi.dev".into(), AuthStatus::StorageLocked);
        attach_unreadable_store(&controller, temp.path());
        let before = std::fs::read(temp.path().join("credentials.json")).unwrap();

        let state = controller.begin_login(AuthStorage::Persistent).await;
        assert_eq!(state.status, AuthStatus::StorageLocked);
        assert_eq!(
            state.message.as_deref(),
            Some("The account session is unavailable in the system key store.")
        );
        assert!(state.attempt.is_none());
        assert_eq!(
            std::fs::read(temp.path().join("credentials.json")).unwrap(),
            before
        );
        assert!(controller
            .inner
            .core
            .lock()
            .unwrap()
            .store
            .as_ref()
            .unwrap()
            .has_active_reference());
    }

    #[tokio::test]
    async fn explicit_session_sign_in_tombstones_unreadable_reference_before_browser_request() {
        let temp = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let metadata_path = temp.path().join("credentials.json");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_complete_http_request(&mut stream);
            let metadata: serde_json::Value =
                serde_json::from_slice(&std::fs::read(metadata_path).unwrap()).unwrap();
            assert_eq!(metadata["logoutTombstone"], true);
            assert_eq!(metadata["journalIds"], serde_json::json!(["credential-1"]));
            respond_probe(&mut stream, "503 Service Unavailable");
            request
        });
        let controller = controller_for_origin(origin, AuthStatus::StorageLocked);
        attach_unreadable_store(&controller, temp.path());

        controller.begin_login(AuthStorage::Session).await;
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /v1/desktop/start "));
        let metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(temp.path().join("credentials.json")).unwrap())
                .unwrap();
        assert_eq!(metadata["logoutTombstone"], true);
        assert_eq!(metadata["activeId"], "credential-1");
        assert_eq!(metadata["journalIds"], serde_json::json!(["credential-1"]));
        assert!(!controller
            .inner
            .core
            .lock()
            .unwrap()
            .store
            .as_ref()
            .unwrap()
            .has_active_reference());
    }

    #[tokio::test]
    async fn failed_tombstone_blocks_session_sign_in_without_changing_reference() {
        let temp = tempfile::tempdir().unwrap();
        let controller =
            controller_for_origin("https://auth.lomi.dev".into(), AuthStatus::StorageLocked);
        attach_unreadable_store(&controller, temp.path());
        let before = std::fs::read(temp.path().join("credentials.json")).unwrap();
        controller
            .inner
            .core
            .lock()
            .unwrap()
            .store
            .as_mut()
            .unwrap()
            .fail_next_metadata_write();

        let state = controller.begin_login(AuthStorage::Session).await;
        assert_eq!(state.status, AuthStatus::StorageLocked);
        assert!(state.attempt.is_none());
        assert_eq!(
            std::fs::read(temp.path().join("credentials.json")).unwrap(),
            before
        );
        assert!(controller
            .inner
            .core
            .lock()
            .unwrap()
            .store
            .as_ref()
            .unwrap()
            .has_active_reference());
    }

    #[tokio::test]
    async fn signing_out_an_unreadable_reference_reports_unconfirmed_remote_revocation() {
        let temp = tempfile::tempdir().unwrap();
        let controller =
            controller_for_origin("https://auth.lomi.dev".into(), AuthStatus::StorageLocked);
        attach_unreadable_store(&controller, temp.path());

        let state = controller.sign_out().await;
        assert_eq!(state.status, AuthStatus::SignedOut);
        assert_eq!(state.remote_revocation_confirmed, Some(false));
        assert!(state.message.unwrap().contains("cleanup will be retried"));
        let mut core = controller.inner.core.lock().unwrap();
        let store = core.store.as_mut().unwrap();
        assert!(!store.has_active_reference());
        assert_eq!(store.load_active().unwrap(), None);
        let metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(temp.path().join("credentials.json")).unwrap())
                .unwrap();
        assert_eq!(metadata["logoutTombstone"], true);
        assert_eq!(metadata["journalIds"], serde_json::json!(["credential-1"]));
    }

    #[tokio::test]
    async fn failed_sign_out_preserves_unreadable_reference_and_reports_storage_lock() {
        let temp = tempfile::tempdir().unwrap();
        let controller =
            controller_for_origin("https://auth.lomi.dev".into(), AuthStatus::StorageLocked);
        attach_unreadable_store(&controller, temp.path());
        let before = std::fs::read(temp.path().join("credentials.json")).unwrap();
        controller
            .inner
            .core
            .lock()
            .unwrap()
            .store
            .as_mut()
            .unwrap()
            .fail_next_metadata_write();

        let state = controller.sign_out().await;
        assert_eq!(state.status, AuthStatus::StorageLocked);
        assert_eq!(state.remote_revocation_confirmed, Some(false));
        assert!(state
            .message
            .unwrap()
            .contains("Could not safely persist local sign-out"));
        assert_eq!(
            std::fs::read(temp.path().join("credentials.json")).unwrap(),
            before
        );
        assert!(controller
            .inner
            .core
            .lock()
            .unwrap()
            .store
            .as_ref()
            .unwrap()
            .has_active_reference());
    }

    #[tokio::test]
    async fn sign_in_preserves_unopened_storage_diagnosis_for_both_storage_choices() {
        for storage in [AuthStorage::Persistent, AuthStorage::Session] {
            for diagnosis in [
                "Another Lomi process owns account storage.",
                "Account credential metadata is invalid and was preserved.",
            ] {
                let controller = controller_for_origin(
                    "https://auth.lomi.dev".into(),
                    AuthStatus::StorageLocked,
                );
                controller.inner.core.lock().unwrap().state.message = Some(diagnosis.into());

                let state = controller.begin_login(storage).await;
                assert_eq!(state.status, AuthStatus::StorageLocked);
                assert_eq!(state.message.as_deref(), Some(diagnosis));
                assert!(state.attempt.is_none());
            }
        }
    }

    #[tokio::test]
    async fn sign_in_explains_unopened_storage_when_diagnosis_is_missing() {
        let controller =
            controller_for_origin("https://auth.lomi.dev".into(), AuthStatus::StorageLocked);

        let state = controller.begin_login(AuthStorage::Persistent).await;
        assert_eq!(state.status, AuthStatus::StorageLocked);
        assert!(state.message.unwrap().contains("could not be opened"));
        assert!(state.attempt.is_none());
    }

    #[tokio::test]
    async fn browser_success_waits_for_exchange_session_check_and_activation() {
        let server = delayed_desktop_server(false);
        let DelayedDesktopServer {
            origin,
            session_request,
            release_session,
            worker: server_worker,
        } = server;
        let controller = controller_for_origin(origin, AuthStatus::SignedOut);
        let mut callback =
            begin_browser_callback(&controller, "confirmed-login", AuthStorage::Session).await;
        let session_request = tokio::task::spawn_blocking(move || {
            session_request
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
        })
        .await
        .unwrap();
        assert!(session_request.starts_with("GET /v1/me "));
        assert!(session_request
            .to_ascii_lowercase()
            .contains("authorization: bearer issued-session"));

        let mut first_byte = [0_u8; 1];
        assert!(tokio::time::timeout(
            Duration::from_millis(30),
            callback.browser.read(&mut first_byte)
        )
        .await
        .is_err());

        let browser_reader = tokio::spawn(async move {
            let mut response = Vec::new();
            callback.browser.read_to_end(&mut response).await.unwrap();
            String::from_utf8(response).unwrap()
        });
        release_session.send(()).unwrap();
        callback.worker.await.unwrap();
        let response = browser_reader.await.unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("Authentication successful"));
        assert_eq!(controller.snapshot().status, AuthStatus::SignedIn);
        assert!(controller.is_signed_in_generation(callback.generation));
        assert!(callback.cancel_observer.has_changed().is_err());

        let requests = server_worker.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("POST /v1/desktop/exchange "));
        assert!(requests[1].starts_with("GET /v1/me "));

        {
            let mut core = controller.inner.core.lock().unwrap();
            core.next_generation();
            core.state.status = AuthStatus::SignedOut;
            core.token = None;
            core.state.user = None;
            core.state.session = None;
        }
        assert!(!controller.is_signed_in_generation(callback.generation));
    }

    #[tokio::test]
    async fn cancellation_during_session_check_revokes_token_and_finishes_with_error() {
        let server = delayed_desktop_server(true);
        let DelayedDesktopServer {
            origin,
            session_request,
            release_session,
            worker: server_worker,
        } = server;
        let controller = controller_for_origin(origin, AuthStatus::SignedOut);
        let temp = tempfile::tempdir().unwrap();
        attach_denied_store(&controller, temp.path());
        let mut callback =
            begin_browser_callback(&controller, "cancel-during-check", AuthStorage::Persistent)
                .await;
        let session_request = tokio::task::spawn_blocking(move || {
            session_request
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
        })
        .await
        .unwrap();
        assert!(session_request.starts_with("GET /v1/me "));

        assert_eq!(
            controller.cancel_login("cancel-during-check").status,
            AuthStatus::SignedOut
        );
        let browser_reader = tokio::spawn(async move {
            let mut response = Vec::new();
            callback.browser.read_to_end(&mut response).await.unwrap();
            String::from_utf8(response).unwrap()
        });
        release_session.send(()).unwrap();
        callback.worker.await.unwrap();
        let response = browser_reader.await.unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("Sign-in was not completed"));
        assert!(!response.contains("Authentication successful"));
        assert_eq!(controller.snapshot().status, AuthStatus::SignedOut);
        assert!(!controller.is_signed_in_generation(callback.generation));

        let requests = server_worker.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[2].starts_with("POST /api/auth/sign-out "));
        assert!(requests[2]
            .to_ascii_lowercase()
            .contains("authorization: bearer issued-session"));
    }

    fn attach_denied_store(controller: &AuthController, root: &Path) {
        let mut store = CredentialStore::open(root, "test-session-fallback").unwrap();
        store.deny_key_store_access();
        controller.inner.core.lock().unwrap().store = Some(store);
    }

    fn assert_memory_only_session(controller: &AuthController, root: &Path, token: &str) {
        let core = controller.inner.core.lock().unwrap();
        assert_eq!(core.state.status, AuthStatus::SignedIn);
        assert!(core.token_storage == Some(AuthStorage::Session));
        assert!(core.state.storage == Some(AuthStorage::Session));
        assert!(!core.token_persisted);
        assert_eq!(core.token.as_ref().unwrap().exposed(), token);
        assert_eq!(
            core.state.message.as_deref(),
            Some(SESSION_STORAGE_FALLBACK_MESSAGE)
        );
        assert_eq!(core.state.remote_revocation_confirmed, None);
        let metadata = std::fs::read_to_string(root.join("credentials.json")).unwrap();
        assert!(!metadata.contains(token));
        let metadata: serde_json::Value = serde_json::from_str(&metadata).unwrap();
        assert_eq!(metadata["logoutTombstone"], true);
        assert!(!metadata["journalIds"].as_array().unwrap().is_empty());
        drop(core);
        assert!(controller
            .inner
            .core
            .lock()
            .unwrap()
            .store
            .as_mut()
            .unwrap()
            .load_active()
            .unwrap()
            .is_none());
    }

    async fn browser_with_denied_store(fail_metadata_after: Option<usize>) {
        let temp = tempfile::tempdir().unwrap();
        let server = delayed_desktop_server(fail_metadata_after.is_some());
        let controller = controller_for_origin(server.origin, AuthStatus::SignedOut);
        attach_denied_store(&controller, temp.path());
        if let Some(writes) = fail_metadata_after {
            controller
                .inner
                .core
                .lock()
                .unwrap()
                .store
                .as_mut()
                .unwrap()
                .fail_metadata_write_after(writes);
        }
        let mut callback =
            begin_browser_callback(&controller, "denied-key-store", AuthStorage::Persistent).await;
        tokio::task::spawn_blocking(move || {
            server
                .session_request
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
        })
        .await
        .unwrap();
        let browser_reader = tokio::spawn(async move {
            let mut response = Vec::new();
            callback.browser.read_to_end(&mut response).await.unwrap();
            String::from_utf8(response).unwrap()
        });
        server.release_session.send(()).unwrap();
        callback.worker.await.unwrap();
        let response = browser_reader.await.unwrap();
        let requests = server.worker.join().unwrap();
        if fail_metadata_after.is_none() {
            assert!(response.contains("Authentication successful"));
            assert_memory_only_session(&controller, temp.path(), "issued-session");
            assert!(controller.is_signed_in_generation(callback.generation));
            assert_eq!(requests.len(), 2);
        } else {
            assert!(response.contains("Sign-in was not completed"));
            assert!(!response.contains("Authentication successful"));
            assert_eq!(controller.snapshot().status, AuthStatus::StorageLocked);
            let core = controller.inner.core.lock().unwrap();
            assert!(core.token.is_none());
            assert!(!core.token_persisted);
            drop(core);
            assert!(!controller.is_signed_in_generation(callback.generation));
            assert_eq!(requests.len(), 3);
            assert!(requests[2].starts_with("POST /api/auth/sign-out "));
        }
    }

    #[tokio::test]
    async fn browser_key_store_failure_activates_memory_only_session_after_tombstone() {
        browser_with_denied_store(None).await;
    }

    #[tokio::test]
    async fn browser_tombstone_failure_revokes_without_activating() {
        browser_with_denied_store(Some(1)).await;
    }

    #[tokio::test]
    async fn browser_metadata_save_failure_revokes_without_session_fallback() {
        browser_with_denied_store(Some(0)).await;
    }

    fn confirmed_session(renewed_token: Option<String>) -> SessionCheck {
        SessionCheck {
            user: super::super::AuthUser {
                id: "user-1".into(),
                display_name: "Ada".into(),
                email: "ada@example.com".into(),
                github_login: "ada".into(),
                status: "active".into(),
            },
            session: super::super::AuthSession {
                id: "session-1".into(),
                expires_at: "2026-10-01T00:00:00Z".into(),
            },
            renewed_token,
        }
    }

    #[tokio::test]
    async fn deferred_confirmation_and_renewal_fall_back_after_durable_tombstone() {
        for persisted in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            // Any unexpected remote revocation fails against this unreachable endpoint.
            let controller =
                controller_for_origin("http://127.0.0.1:1".into(), AuthStatus::Offline);
            if persisted {
                attach_unreadable_store(&controller, temp.path());
            } else {
                attach_denied_store(&controller, temp.path());
            }
            let generation = {
                let mut core = controller.inner.core.lock().unwrap();
                core.token = Some(Secret::new("original-session".into()));
                core.token_storage = Some(AuthStorage::Persistent);
                core.token_persisted = persisted;
                core.refresh_owner_generation = Some(core.generation);
                core.generation
            };
            let token = if persisted {
                "renewed-session"
            } else {
                "original-session"
            };
            controller
                .apply_refresh_result(
                    generation,
                    Secret::new("original-session".into()),
                    Ok(confirmed_session(persisted.then(|| token.into()))),
                )
                .await;
            assert_memory_only_session(&controller, temp.path(), token);
        }
    }

    #[tokio::test]
    async fn deferred_confirmation_and_renewal_tombstone_failure_revoke_all_tokens() {
        for persisted in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let count = if persisted { 2 } else { 1 };
            let (origin, server) = health_server(vec!["200 OK"; count]);
            let controller = controller_for_origin(origin, AuthStatus::Offline);
            if persisted {
                attach_unreadable_store(&controller, temp.path());
            } else {
                attach_denied_store(&controller, temp.path());
            }
            let generation = {
                let mut core = controller.inner.core.lock().unwrap();
                core.token = Some(Secret::new("original-session".into()));
                core.token_storage = Some(AuthStorage::Persistent);
                core.token_persisted = persisted;
                core.refresh_owner_generation = Some(core.generation);
                core.store.as_mut().unwrap().fail_metadata_write_after(1);
                core.generation
            };
            controller
                .apply_refresh_result(
                    generation,
                    Secret::new("original-session".into()),
                    Ok(confirmed_session(
                        persisted.then(|| "renewed-session".into()),
                    )),
                )
                .await;
            let state = controller.snapshot();
            assert_eq!(state.status, AuthStatus::StorageLocked);
            assert_eq!(state.remote_revocation_confirmed, Some(true));
            let core = controller.inner.core.lock().unwrap();
            assert!(core.token.is_none());
            assert!(core.token_storage.is_none());
            assert!(!core.token_persisted);
            assert!(state.user.is_none());
            assert!(state.session.is_none());
            let requests = server.join().unwrap();
            assert_eq!(requests.len(), count);
            assert!(requests
                .iter()
                .all(|request| request.starts_with("POST /api/auth/sign-out ")));
            assert!(requests
                .iter()
                .any(|request| request.contains("original-session")));
            if persisted {
                assert!(requests
                    .iter()
                    .any(|request| request.contains("renewed-session")));
            }
        }
    }

    #[tokio::test]
    async fn session_storage_failure_never_displays_authentication_success() {
        let server = delayed_desktop_server(true);
        let DelayedDesktopServer {
            origin,
            session_request,
            release_session,
            worker: server_worker,
        } = server;
        let controller = controller_for_origin(origin, AuthStatus::SignedOut);
        let mut callback =
            begin_browser_callback(&controller, "storage-fails", AuthStorage::Persistent).await;
        let session_request = tokio::task::spawn_blocking(move || {
            session_request
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
        })
        .await
        .unwrap();
        assert!(session_request.starts_with("GET /v1/me "));

        let browser_reader = tokio::spawn(async move {
            let mut response = Vec::new();
            callback.browser.read_to_end(&mut response).await.unwrap();
            String::from_utf8(response).unwrap()
        });
        release_session.send(()).unwrap();
        callback.worker.await.unwrap();
        let response = browser_reader.await.unwrap();
        assert!(response.contains("Sign-in was not completed"));
        assert!(!response.contains("Authentication successful"));
        assert_eq!(controller.snapshot().status, AuthStatus::StorageLocked);
        assert!(!controller.is_signed_in_generation(callback.generation));

        let requests = server_worker.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[2].starts_with("POST /api/auth/sign-out "));
    }

    #[tokio::test]
    async fn offline_state_can_be_rechecked_and_recovers_with_a_new_revision() {
        let (origin, server) = health_server(vec!["503 Service Unavailable", "200 OK"]);
        let controller = controller_for_origin(origin, AuthStatus::Offline);

        let failed = controller.refresh_state().await;
        assert_eq!(failed.status, AuthStatus::Offline);
        assert!(failed.revision > 0);

        if let Ok(mut core) = controller.inner.core.lock() {
            core.refresh_started = None;
        }
        let recovered = controller.refresh_state().await;
        assert_eq!(recovered.status, AuthStatus::SignedOut);
        assert!(recovered.revision > failed.revision);

        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests
            .iter()
            .all(|request| request.starts_with("GET /health/live ")));
    }

    #[tokio::test]
    async fn delayed_probe_cannot_own_busy_state_after_start_cancel_and_new_probe() {
        let server = delayed_health_server();
        let observed = Arc::new(Mutex::new(server.observed));
        let controller = controller_for_origin(server.origin.clone(), AuthStatus::Offline);

        let old_controller = controller.clone();
        let old_probe = tokio::spawn(async move { old_controller.refresh_state().await });
        let old_request = next_observed_request(Arc::clone(&observed)).await;
        assert!(old_request.starts_with("GET /health/live "));

        let attempt_generation = {
            let mut core = controller.inner.core.lock().unwrap();
            core.state.status = AuthStatus::Authorizing;
            core.start_attempt("cancel-between-probes".into())
        };
        let cancelled = controller.cancel_login("cancel-between-probes");
        assert_eq!(cancelled.status, AuthStatus::SignedOut);

        let new_controller = controller.clone();
        let new_probe = tokio::spawn(async move { new_controller.refresh_state().await });
        let new_request = next_observed_request(Arc::clone(&observed)).await;
        assert!(new_request.starts_with("GET /health/live "));

        server.release_old.send(()).unwrap();
        let stale_result = old_probe.await.unwrap();
        assert_eq!(stale_result.status, AuthStatus::Checking);

        {
            let mut core = controller.inner.core.lock().unwrap();
            assert!(core.generation > attempt_generation);
            assert_eq!(core.refresh_owner_generation, Some(core.generation));
            // Ignore the debounce window so this check exercises busy ownership alone.
            core.refresh_started = None;
        }
        let in_flight =
            tokio::time::timeout(Duration::from_millis(200), controller.refresh_state())
                .await
                .expect("the existing probe owns refresh state");
        assert_eq!(in_flight.status, AuthStatus::Checking);

        server.release_new.send(()).unwrap();
        let recovered = new_probe.await.unwrap();
        assert_eq!(recovered.status, AuthStatus::SignedOut);
        let requests = server.worker.join().unwrap();
        assert_eq!(requests.len(), 2);
    }

    async fn next_observed_request(
        receiver: Arc<Mutex<std::sync::mpsc::Receiver<String>>>,
    ) -> String {
        tokio::task::spawn_blocking(move || {
            receiver
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
        })
        .await
        .unwrap()
    }

    #[test]
    fn explicit_cancel_invalidates_attempt_and_publishes_a_monotonic_snapshot() {
        let controller = AuthController::default();
        let generation = {
            let mut core = controller.inner.core.lock().unwrap();
            core.state.status = AuthStatus::Authorizing;
            let generation = core.start_attempt("cancel-me".into());
            core.state.attempt = Some(AuthAttempt {
                id: "cancel-me".into(),
                expires_at: "2026-10-01T00:00:00Z".into(),
            });
            generation
        };
        let published = controller.publish();
        let cancelled = controller.cancel_login("cancel-me");
        assert_eq!(cancelled.status, AuthStatus::SignedOut);
        assert!(cancelled.revision > published.revision);
        let core = controller.inner.core.lock().unwrap();
        assert!(!core.current_attempt(generation, "cancel-me"));
        assert!(core.attempt.is_none());
    }

    #[test]
    fn generation_ids_reject_stale_browser_attempts_after_cancel_and_restart() {
        let mut core = Core::new(AuthStatus::SignedOut, None);
        let first = core.start_attempt("first".into());
        assert!(core.current_attempt(first, "first"));
        assert!(core.invalidate_attempt("first"));
        assert!(!core.current_attempt(first, "first"));
        let second = core.start_attempt("second".into());
        assert_ne!(first, second);
        assert!(!core.current_attempt(first, "first"));
        assert!(core.current_attempt(second, "second"));
        assert!(!core.invalidate_attempt("first"));
        assert!(core.current_attempt(second, "second"));
    }

    #[test]
    fn pkce_request_uses_random_32_byte_verifier_and_s256_challenge() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

        let (state, verifier, challenge) = create_pkce_pair().unwrap();
        assert_eq!(state.exposed().len(), 43);
        assert_eq!(verifier.exposed().len(), 43);
        assert_eq!(challenge.len(), 43);
        assert_ne!(state.exposed(), verifier.exposed());
        assert_eq!(
            challenge,
            URL_SAFE_NO_PAD.encode(digest(&SHA256, verifier.exposed().as_bytes()).as_ref())
        );
    }

    #[test]
    fn refresh_failures_keep_transport_unknown_and_account_denial_distinct() {
        assert_eq!(
            refresh_failure(HttpProblem::Transport).0,
            AuthStatus::Offline
        );
        assert_eq!(
            refresh_failure(HttpProblem::Temporary).0,
            AuthStatus::Offline
        );
        assert_eq!(
            refresh_failure(HttpProblem::RateLimited(Some(20))).0,
            AuthStatus::Offline
        );
        assert_eq!(refresh_failure(HttpProblem::Forbidden).0, AuthStatus::Error);
        assert_eq!(
            refresh_failure(HttpProblem::InvalidResponse).0,
            AuthStatus::Error
        );
    }

    #[test]
    fn remote_failures_distinguish_authorization_from_service_problems() {
        assert_eq!(
            remote_request_message(HttpProblem::DesktopUpdateRequired),
            "Update Lomi to share workspaces remotely."
        );
        let authorization_message = remote_request_message(HttpProblem::Unauthorized);
        assert_eq!(
            authorization_message,
            "The Remote service could not verify your account session. Check Account settings and retry."
        );
        for problem in [
            HttpProblem::Transport,
            HttpProblem::Temporary,
            HttpProblem::RateLimited(Some(20)),
            HttpProblem::InvalidResponse,
        ] {
            assert_ne!(remote_request_message(problem), authorization_message);
        }
    }

    #[test]
    fn expiry_timestamp_is_iso_utc() {
        assert_eq!(
            format_timestamp(UNIX_EPOCH + Duration::from_secs(0)),
            "1970-01-01T00:00:00Z"
        );
        assert_eq!(
            format_timestamp(UNIX_EPOCH + Duration::from_secs(1_798_761_600)),
            "2027-01-01T00:00:00Z"
        );
    }
}
