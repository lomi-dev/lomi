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
