//! Native Remote authority. Cloud records cannot create local grants.
pub(crate) mod activity;
mod channel;
mod policy;
mod runtime;
pub mod workspace;
use workspace::{Domain, WorkspaceInfo};

use crate::{auth::AuthController, terminal::Terminals};
use lomi_remote_crypto::{Identity, PeerApproval, Permissions, SignedBundle, SignedPeerApproval};
use policy::{LocalGrant, Policy};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{Manager, State, Window};

pub(crate) const LIVE_QUALIFIED: bool = cfg!(all(target_os = "macos", target_arch = "aarch64"));

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteState {
    pub qualified: bool,
    pub enabled: bool,
    pub online: bool,
    pub paused: bool,
    pub host_id: Option<String>,
    pub fingerprint: Option<String>,
    pub message: Option<String>,
    pub sessions: Vec<SessionInfo>,
    pub pairings: Vec<Pairing>,
    pub grants: Vec<GrantInfo>,
    pub workspaces: Vec<WorkspaceInfo>,
    pub domain_epoch: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub epoch: String,
    pub label: String,
    pub cols: u16,
    pub rows: u16,
    pub shared: bool,
    pub available: bool,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantInfo {
    pub id: String,
    pub fingerprint: String,
    pub session_ids: Vec<String>,
    pub permissions: Permissions,
    pub expires_at: u64,
    pub revoked: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pairing {
    pub id: String,
    pub host_id: String,
    pub device_id: String,
    pub host_bundle: SignedBundle,
    pub device_bundle: SignedBundle,
    pub host_fingerprint: String,
    pub device_fingerprint: String,
    pub nonce: String,
    pub pairing_fingerprint: String,
    pub status: String,
    #[serde(deserialize_with = "deserialize_expiry")]
    pub expires_at: u64,
    #[serde(deserialize_with = "deserialize_expiry")]
    pub max_approval_expires_at: u64,
    #[serde(default)]
    pub signed_approval: Option<SignedPeerApproval>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub workspace_epoch: Option<String>,
    #[serde(default)]
    pub workspace_revision: Option<u64>,
}

struct Core {
    enabled: bool,
    paused: bool,
    activation_revision: u64,
    binding: Option<crate::auth::controller::RemoteBinding>,
    identity: Option<Identity>,
    policy: Option<Policy>,
    deadline: Option<Instant>,
    message: Option<String>,
    pairings: Vec<Pairing>,
    shares: HashSet<String>,
    legacy_shares: HashSet<String>,
    channels: HashMap<String, Arc<AtomicBool>>,
    channel_workspaces: HashMap<String, String>,
    channel_grants: HashMap<String, String>,
    attempted_channels: HashMap<String, u64>,
    policy_revision: u64,
    domain: Domain,
    restore_binding: Option<crate::auth::controller::RemoteBinding>,
}

impl Core {
    fn activation_is_current(&self, activation_revision: u64, policy_revision: u64) -> bool {
        self.activation_revision == activation_revision && self.policy_revision == policy_revision
    }
    fn close_channels(&mut self) {
        self.deadline = None;
        for stop in self.channels.values() {
            stop.store(true, Ordering::SeqCst);
        }
        self.channels.clear();
        self.channel_workspaces.clear();
        self.channel_grants.clear();
        self.pairings.clear();
    }

    fn pause_idle(&mut self) {
        self.close_channels();
        self.enabled = false;
        self.paused = true;
        self.activation_revision = self.activation_revision.wrapping_add(1);
        self.restore_binding = self.binding.clone();
        self.message =
            Some("Remote paused after an hour of inactivity. Resume it from the desktop.".into());
    }

    fn refresh_bound_authority(&mut self, current: Option<crate::auth::controller::RemoteBinding>) {
        if let Some(binding) = self.binding.as_ref() {
            if !current.as_ref().is_some_and(|b| b.same_authority(binding)) {
                self.reset_bound_authority();
                return;
            }
            self.binding = current.clone();
        }
        if self
            .restore_binding
            .as_ref()
            .is_some_and(|previous| current.as_ref().is_some_and(|b| b.same_authority(previous)))
        {
            self.restore_binding = current;
        }
    }
    fn reset_bound_authority(&mut self) {
        self.activation_revision = self.activation_revision.wrapping_add(1);
        self.enabled = false;
        self.deadline = None;
        self.binding = None;
        self.policy = None;
        self.identity = None;
        self.restore_binding = None;
        self.shares.clear();
        self.legacy_shares.clear();
        self.pairings.clear();
        for stop in self.channels.values() {
            stop.store(true, Ordering::SeqCst);
        }
        self.channels.clear();
        self.channel_workspaces.clear();
        self.channel_grants.clear();
        self.message = Some("Remote account authorization changed.".into());
    }
    fn storage_failed(&mut self) {
        self.activation_revision = self.activation_revision.wrapping_add(1);
        self.enabled = false;
        self.deadline = None;
        self.message = Some("Remote secure policy could not be committed.".into());
        for stop in self.channels.values() {
            stop.store(true, Ordering::SeqCst);
        }
        self.policy.take();
        self.identity.take();
    }
    fn reconcile_cloud_grants(
        &mut self,
        grants: &[Value],
        pending_pairings: &HashSet<String>,
    ) -> Result<(), String> {
        let policy = self.policy.as_mut().ok_or("Remote identity unavailable.")?;
        let mut changed = false;
        let mut retired = HashSet::new();
        for local in &mut policy.grants {
            let uncertain_pending = local.approval.approval.version == 2
                && local
                    .pairing_id
                    .as_ref()
                    .is_some_and(|id| pending_pairings.contains(id));
            let active = grants.iter().any(|g| {
                g.get("id").and_then(Value::as_str) == Some(local.id.as_str())
                    && g.get("revokedAt").is_none_or(Value::is_null)
            });
            if !local.revoked
                && !active
                && (local.confirmed || (local.approval.approval.version == 2 && !uncertain_pending))
            {
                local.revoked = true;
                retired.insert(local.id.clone());
                changed = true;
            }
        }
        for (id, grant) in &self.channel_grants {
            if retired.contains(grant) {
                if let Some(stop) = self.channels.get(id) {
                    stop.store(true, Ordering::SeqCst);
                }
            }
        }
        if changed {
            self.commit_policy()?;
        }
        Ok(())
    }
    fn prune_inactive_grants(&mut self) -> Result<(), String> {
        let policy = self.policy.as_mut().ok_or("Remote identity unavailable.")?;
        let removed: HashSet<String> = policy
            .grants
            .iter()
            .filter(|g| g.revoked || g.approval.approval.expires_at <= now())
            .map(|g| g.id.clone())
            .collect();
        if removed.is_empty() {
            return Ok(());
        }
        for (id, grant) in &self.channel_grants {
            if removed.contains(grant) {
                if let Some(stop) = self.channels.get(id) {
                    stop.store(true, Ordering::SeqCst);
                }
            }
        }
        policy.grants.retain(|g| !removed.contains(&g.id));
        self.commit_policy()
    }
    fn commit_policy(&mut self) -> Result<(), String> {
        if let Err(error) = self
            .policy
            .as_ref()
            .ok_or("Remote identity unavailable.")?
            .save()
        {
            self.storage_failed();
            return Err(error);
        }
        self.policy_revision = self.policy_revision.wrapping_add(1);
        Ok(())
    }
}

impl Default for Core {
    fn default() -> Self {
        Self {
            enabled: false,
            paused: false,
            activation_revision: 0,
            binding: None,
            identity: None,
            policy: None,
            deadline: None,
            message: (!LIVE_QUALIFIED)
                .then(|| "Remote live access is awaiting qualification.".into()),
            pairings: vec![],
            shares: HashSet::new(),
            legacy_shares: HashSet::new(),
            channels: HashMap::new(),
            channel_workspaces: HashMap::new(),
            channel_grants: HashMap::new(),
            attempted_channels: HashMap::new(),
            policy_revision: 0,
            domain: Domain::default(),
            restore_binding: None,
        }
    }
}

#[derive(Clone)]
pub struct Remote {
    core: Arc<Mutex<Core>>,
    publication: Arc<tokio::sync::Mutex<()>>,
    runtime: Arc<Mutex<runtime::Runtime>>,
    stopped: Arc<AtomicBool>,
    healthy: Arc<AtomicBool>,
    events: tokio::sync::broadcast::Sender<Value>,
}

impl Default for Remote {
    fn default() -> Self {
        let (events, _) = tokio::sync::broadcast::channel(256);
        Self {
            core: Arc::new(Mutex::new(Core::default())),
            publication: Arc::new(tokio::sync::Mutex::new(())),
            runtime: Arc::new(Mutex::new(runtime::Runtime::default())),
            stopped: Arc::new(AtomicBool::new(false)),
            healthy: Arc::new(AtomicBool::new(false)),
            events,
        }
    }
}

impl Remote {
    pub fn initialize(&self, app: tauri::AppHandle) {
        let terminals = app.state::<Terminals>().inner().clone();
        let Ok((receiver, healthy, observation)) = terminals.observe_remote() else {
            return;
        };
        self.healthy.store(true, Ordering::SeqCst);
        let controller = self.clone();
        let observer_app = app.clone();
        std::thread::spawn(move || {
            const RECOVERING: &str = "Remote is recovering terminal state. Try again shortly.";
            let revoke_unavailable = |id: &str| {
                if let Ok(mut core) = controller.core.lock() {
                    core.shares.remove(id);
                    core.legacy_shares.remove(id);
                }
                observer_app.state::<Terminals>().remote_revoke(id);
            };
            let emit_delta = |delta: Value, generation: Option<u64>| {
                let kind = delta.get("type").and_then(Value::as_str);
                if let Some(id) = delta.get("sessionId").and_then(Value::as_str) {
                    if matches!(kind, Some("output" | "resize"))
                        && generation
                            .is_none_or(|g| observation.current_generation(id, g) != Some(false))
                    {
                        return;
                    }
                    if kind == Some("unavailable") {
                        revoke_unavailable(id);
                    }
                }
                let _ = controller.events.send(delta);
            };
            let sync_losses = |generations: &mut HashMap<String, u64>| {
                let mut deltas = Vec::new();
                if let Ok(mut runtime) = controller.runtime.lock() {
                    for (id, generation, lost, exited, dimensions) in observation.sessions() {
                        let replaced = generations.get(&id).is_some_and(|old| *old != generation);
                        if lost || replaced {
                            if let Some(delta) = runtime.invalidate(&id) {
                                deltas.push(delta);
                            }
                        }
                        if lost && !exited && (replaced || !runtime.sessions.contains_key(&id)) {
                            if let Some((cols, rows)) = dimensions {
                                if let Ok(Some(delta)) = runtime.discard_lost_event(
                                    crate::terminal::RemoteTerminalEvent::Start {
                                        id: id.clone(),
                                        cols,
                                        rows,
                                    },
                                ) {
                                    deltas.push(delta);
                                }
                                generations.insert(id.clone(), generation);
                            }
                        }
                        if lost && exited {
                            if let Ok(Some(delta)) = runtime.discard_lost_event(
                                crate::terminal::RemoteTerminalEvent::Exit {
                                    id: id.clone(),
                                    code: None,
                                },
                            ) {
                                deltas.push(delta);
                            }
                            observation.retire(&id, generation);
                            generations.remove(&id);
                        }
                    }
                }
                let changed = !deltas.is_empty();
                for delta in deltas {
                    emit_delta(delta, None);
                }
                if changed {
                    let _ = controller.reconcile_workspaces(&observer_app);
                }
            };
            let mut generations = HashMap::new();
            let mut recovering = false;
            while !controller.stopped.load(Ordering::SeqCst) {
                if !healthy.load(Ordering::SeqCst) {
                    controller.healthy.store(false, Ordering::SeqCst);
                    controller.fail_closed("Remote terminal observer is unavailable.");
                    break;
                }
                sync_losses(&mut generations);
                let needs_recovery = controller
                    .runtime
                    .lock()
                    .map(|runtime| runtime.needs_recovery())
                    .unwrap_or(true);
                if recovering || needs_recovery {
                    if !recovering {
                        controller.healthy.store(false, Ordering::SeqCst);
                        controller.fail_closed(RECOVERING);
                    }
                    recovering = true;
                    let result = controller
                        .runtime
                        .lock()
                        .map_err(|_| "Terminal model unavailable.".to_string())
                        .and_then(|mut runtime| {
                            let delta = runtime.recover(&observer_app)?;
                            let unavailable = runtime
                                .sessions
                                .values()
                                .filter(|session| !session.available)
                                .map(|session| session.id.clone())
                                .collect::<Vec<_>>();
                            Ok((delta, runtime.needs_recovery(), unavailable))
                        });
                    if controller.stopped.load(Ordering::SeqCst) {
                        break;
                    }
                    // A producer may have reached its bound during helper IPC.
                    // Fence those sessions before publishing recovery results.
                    sync_losses(&mut generations);
                    if let Ok((delta, still_recovering, unavailable)) = result {
                        for id in unavailable {
                            revoke_unavailable(&id);
                        }
                        if let Some(delta) = delta {
                            let generation = delta
                                .get("sessionId")
                                .and_then(Value::as_str)
                                .and_then(|id| generations.get(id))
                                .copied();
                            emit_delta(delta, generation);
                        }
                        let _ = controller.reconcile_workspaces(&observer_app);
                        let actual_fault = controller
                            .runtime
                            .lock()
                            .map(|runtime| runtime.needs_recovery())
                            .unwrap_or(true);
                        if !still_recovering && !actual_fault && healthy.load(Ordering::SeqCst) {
                            if let Ok(mut core) = controller.core.lock() {
                                if core.message.as_deref() == Some(RECOVERING) {
                                    core.message = None;
                                }
                            }
                            controller.healthy.store(true, Ordering::SeqCst);
                            recovering = false;
                            continue;
                        }
                    }
                    std::thread::sleep(Duration::from_millis(250));
                    continue;
                }
                match receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(observed) => {
                        let Some(lost) = observation.current(&observed) else {
                            continue;
                        };
                        let id = observed.event.session_id().to_owned();
                        let generation = observed.generation;
                        let start = matches!(
                            &observed.event,
                            crate::terminal::RemoteTerminalEvent::Start { .. }
                        );
                        let exit = matches!(
                            &observed.event,
                            crate::terminal::RemoteTerminalEvent::Exit { .. }
                        );
                        let output = matches!(
                            &observed.event,
                            crate::terminal::RemoteTerminalEvent::Output { .. }
                        );
                        if start && lost && generations.get(&id) == Some(&generation) {
                            continue;
                        }
                        if start {
                            generations.insert(id.clone(), generation);
                        }
                        let (result, needs_recovery) = match controller.runtime.lock() {
                            Ok(mut runtime) => {
                                let result = if lost {
                                    runtime.discard_lost_event(observed.event)
                                } else {
                                    runtime.observe(&observer_app, observed.event)
                                };
                                (result, runtime.needs_recovery())
                            }
                            Err(_) => (Err("Terminal model unavailable.".to_string()), true),
                        };
                        if exit {
                            observation.retire(&id, generation);
                            generations.remove(&id);
                        }
                        sync_losses(&mut generations);
                        let unavailable = result
                            .as_ref()
                            .ok()
                            .and_then(Option::as_ref)
                            .is_some_and(|delta| {
                                delta.get("type").and_then(Value::as_str) == Some("unavailable")
                            });
                        if !output || unavailable {
                            let _ = controller.reconcile_workspaces(&observer_app);
                        }
                        let failed = result.is_err();
                        if let Ok(Some(delta)) = result {
                            emit_delta(delta, Some(generation));
                        }
                        let actual_fault = controller
                            .runtime
                            .lock()
                            .map(|runtime| runtime.needs_recovery())
                            .unwrap_or(true);
                        if failed || needs_recovery || actual_fault {
                            controller.healthy.store(false, Ordering::SeqCst);
                            controller.fail_closed(RECOVERING);
                            recovering = true;
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        controller.healthy.store(false, Ordering::SeqCst);
                        if !controller.stopped.load(Ordering::SeqCst) {
                            controller.fail_closed("Remote terminal observer is unavailable.");
                        }
                        break;
                    }
                }
            }
        });
        let controller = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(2));
            while !controller.stopped.load(Ordering::SeqCst) {
                interval.tick().await;
                controller.reset_changed_binding(&app);
                controller.pause_if_idle(&app, SystemTime::now());
                if LIVE_QUALIFIED
                    && controller
                        .core
                        .lock()
                        .map(|c| !c.enabled && !c.paused)
                        .unwrap_or(false)
                {
                    if let Ok(binding) = app.state::<AuthController>().remote_binding() {
                        let unchecked = controller
                            .core
                            .lock()
                            .map(|c| {
                                !c.restore_binding
                                    .as_ref()
                                    .is_some_and(|b| b.same_authority(&binding))
                            })
                            .unwrap_or(false);
                        if unchecked {
                            let account = hex(&Sha256::digest(
                                format!("lomi-remote-account-v1:{}", binding.user_id).as_bytes(),
                            )[..16]);
                            if let Ok(shared) =
                                Policy::has_shared_consent(&account, &binding.session_id)
                            {
                                if shared
                                    && controller.enable_inner(&app, false, false).await.is_err()
                                {
                                    controller.fail_closed(
                                        "Remote workspace consent could not be restored.",
                                    );
                                } else if let Ok(mut c) = controller.core.lock() {
                                    c.restore_binding = Some(binding);
                                }
                            }
                        }
                    }
                }
                if controller.core.lock().map(|c| c.enabled).unwrap_or(false) {
                    let revision = controller
                        .core
                        .lock()
                        .map(|c| c.activation_revision)
                        .unwrap_or(0);
                    if let Err(message) = controller.poll(&app).await {
                        if let Ok(mut core) = controller.core.lock() {
                            if core.enabled && core.activation_revision == revision {
                                core.close_channels();
                                core.message = Some(message);
                            }
                        }
                    }
                }
            }
        });
    }

    fn reset_changed_binding(&self, app: &tauri::AppHandle) {
        let current = app.state::<AuthController>().remote_binding().ok();
        if let Ok(mut core) = self.core.lock() {
            core.refresh_bound_authority(current);
        }
    }
    fn pause_if_idle(&self, app: &tauri::AppHandle, now: SystemTime) {
        let terminals = app.state::<Terminals>();
        let sessions = if let Ok(mut core) = self.core.lock() {
            if core.paused || !terminals.activity.is_idle(now) {
                return;
            }
            let sessions = core.shares.iter().cloned().collect::<Vec<_>>();
            core.pause_idle();
            sessions
        } else {
            return;
        };
        for id in sessions {
            terminals.remote_revoke(&id);
        }
    }
    fn fail_closed(&self, message: &str) {
        if let Ok(mut core) = self.core.lock() {
            core.deadline = None;
            core.message = Some(message.into());
            for stop in core.channels.values() {
                stop.store(true, Ordering::SeqCst);
            }
            core.channels.clear();
            core.channel_workspaces.clear();
            core.channel_grants.clear();
            core.pairings.clear();
        }
    }

    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.fail_closed("Remote stopped.");
        if let Ok(mut r) = self.runtime.lock() {
            r.stop();
        }
        if let Ok(mut c) = self.core.lock() {
            c.identity.take();
            c.enabled = false;
        }
    }

    pub fn state(&self) -> RemoteState {
        let core = self.core.lock().unwrap_or_else(|e| e.into_inner());
        let runtime = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        let policy = core.policy.as_ref();
        RemoteState {
            qualified: LIVE_QUALIFIED,
            enabled: core.enabled,
            paused: core.paused,
            online: core.deadline.is_some_and(|d| Instant::now() < d)
                && self.healthy.load(Ordering::SeqCst),
            host_id: policy.map(|p| p.host_id.clone()),
            fingerprint: policy
                .and_then(|p| p.bundle.bundle.fingerprint().ok())
                .map(|f| hex(&f)),
            message: core.message.clone(),
            sessions: runtime
                .sessions
                .values()
                .map(|s| SessionInfo {
                    id: s.id.clone(),
                    epoch: s.epoch.clone(),
                    label: s.label.clone(),
                    cols: s.cols,
                    rows: s.rows,
                    shared: core.shares.contains(&s.id),
                    available: s.available,
                })
                .collect(),
            domain_epoch: core.domain.epoch.clone(),
            workspaces: core
                .domain
                .workspaces
                .iter()
                .map(|w| {
                    let consent =
                        policy.and_then(|p| p.workspaces.iter().find(|c| c.id == w.id && c.shared));
                    let ready = w.terminals.iter().all(|t| {
                        t.session_id
                            .as_ref()
                            .is_some_and(|id| runtime.sessions.get(id).is_some_and(|s| s.available))
                    });
                    WorkspaceInfo {
                        id: w.id.clone(),
                        shared: consent.is_some(),
                        online: consent.is_some()
                            && ready
                            && core.deadline.is_some_and(|d| d > Instant::now()),
                        message: if consent.is_some() && !ready {
                            workspace::workspace_unavailable(&core.domain, &runtime, &w.id).or_else(
                                || {
                                    Some(
                                        "Waiting for every workspace terminal to become available."
                                            .into(),
                                    )
                                },
                            )
                        } else {
                            None
                        },
                    }
                })
                .collect(),
            pairings: core.pairings.clone(),
            grants: policy
                .map(|p| {
                    p.grants
                        .iter()
                        .map(|g| GrantInfo {
                            id: g.id.clone(),
                            fingerprint: hex(&g.approval.approval.device_fingerprint),
                            session_ids: g
                                .approval
                                .approval
                                .session_ids
                                .iter()
                                .map(uuid_text)
                                .collect(),
                            permissions: g.approval.approval.permissions,
                            expires_at: g.approval.approval.expires_at,
                            revoked: g.revoked,
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    async fn enable(&self, app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
        if !enabled {
            self.fail_closed("Remote is disabled.");
            let mut core = self.core.lock().map_err(|_| "Remote unavailable.")?;
            core.enabled = false;
            core.paused = false;
            core.activation_revision = core.activation_revision.wrapping_add(1);
            app.state::<Terminals>().activity.deactivate();
            core.shares.clear();
            core.legacy_shares.clear();
            core.restore_binding = core.binding.clone();
            if let Some(policy) = core.policy.as_mut() {
                for w in &mut policy.workspaces {
                    w.shared = false;
                    w.sessions.clear();
                    w.projection = None;
                }
                for g in &mut policy.grants {
                    g.revoked = true;
                }
                core.commit_policy()?;
            }
            return Ok(());
        }
        if !LIVE_QUALIFIED {
            return Err("This build has not qualified Remote live access.".into());
        }
        self.enable_inner(app, false, true).await
    }

    #[cfg(feature = "remote-probe")]
    pub(crate) fn probe_lifecycle(
        &self,
        app: &tauri::AppHandle,
        operation: &str,
        id: &str,
    ) -> Result<Value, String> {
        if app.state::<AuthController>().remote_environment()? != "development" {
            return Err("Remote probes require the local fixture account.".into());
        }
        match operation {
            "idle-hour" => {
                app.state::<Terminals>().activity.probe_elapsed_hour()?;
                self.pause_if_idle(app, SystemTime::now());
                Ok(json!({"remote":self.state()}))
            }
            "helper-fault" => {
                let mut runtime = self.runtime.lock().map_err(|_| "Runtime unavailable.")?;
                runtime.probe_kill_helper()?;
                // Detect the dead child through the ordinary snapshot request path;
                // the real observer owns subsequent recovery and channel fencing.
                if runtime.snapshot(id).is_ok() {
                    return Err("Helper fault was not detected.".into());
                }
                Ok(json!({"faultDetected":runtime.needs_recovery()}))
            }
            "terminal-unavailable" => {
                self.runtime
                    .lock()
                    .map_err(|_| "Runtime unavailable.")?
                    .invalidate(id)
                    .ok_or("Terminal model was not available.")?;
                app.state::<Terminals>().remote_revoke(id);
                self.reconcile_workspaces(app)?;
                Ok(json!({"remote":self.state()}))
            }
            "snapshot" => self
                .runtime
                .lock()
                .map_err(|_| "Runtime unavailable.")?
                .snapshot(id),
            _ => Err("Invalid lifecycle probe operation.".into()),
        }
    }

    #[cfg(feature = "remote-probe")]
    pub(crate) async fn probe_enable(&self, app: &tauri::AppHandle) -> Result<(), String> {
        if app.state::<AuthController>().remote_environment()? != "development" {
            return Err("Remote probes require the local fixture account.".into());
        }
        self.enable_inner(app, true, true).await
    }

    async fn enable_inner(
        &self,
        app: &tauri::AppHandle,
        probe: bool,
        reset_activity: bool,
    ) -> Result<(), String> {
        if !self.healthy.load(Ordering::SeqCst) {
            return Err("Remote is recovering terminal state. Try again shortly.".into());
        }
        self.reset_changed_binding(app);
        let (activation_revision, policy_revision) = {
            let mut core = self.core.lock().map_err(|_| "Remote unavailable.")?;
            core.activation_revision = core.activation_revision.wrapping_add(1);
            (core.activation_revision, core.policy_revision)
        };
        let auth = app.state::<AuthController>();
        let (binding, session) = auth
            .remote_request(reqwest::Method::GET, "/v1/remote/native/session", None)
            .await?;
        let account_id: [u8; 16] =
            Sha256::digest(format!("lomi-remote-account-v1:{}", binding.user_id).as_bytes())[..16]
                .try_into()
                .unwrap();
        if session.get("accountId").and_then(Value::as_str) != Some(hex(&account_id).as_str())
            || session.pointer("/user/id").and_then(Value::as_str) != Some(binding.user_id.as_str())
            || session.pointer("/session/id").and_then(Value::as_str)
                != Some(binding.session_id.as_str())
        {
            return Err("Remote session does not match the desktop account.".into());
        }
        let retained = {
            let core = self.core.lock().map_err(|_| "Remote unavailable.")?;
            if core
                .binding
                .as_ref()
                .is_some_and(|b| b.same_authority(&binding))
                && core.identity.is_some()
            {
                core.policy
                    .as_ref()
                    .map(|p| (p.host_id.clone(), p.bundle.clone()))
            } else {
                None
            }
        };
        let loaded = if retained.is_some() {
            None
        } else {
            #[cfg(feature = "remote-probe")]
            let loaded = if probe {
                Policy::probe(&hex(&account_id), &binding.session_id, account_id)?
            } else {
                Policy::load_or_create(&hex(&account_id), &binding.session_id, account_id)?
            };
            #[cfg(not(feature = "remote-probe"))]
            let loaded = {
                let _ = probe;
                Policy::load_or_create(&hex(&account_id), &binding.session_id, account_id)?
            };
            Some(loaded)
        };
        let (host_id, bundle) = retained
            .or_else(|| {
                loaded
                    .as_ref()
                    .map(|(p, _)| (p.host_id.clone(), p.bundle.clone()))
            })
            .ok_or("Remote identity unavailable.")?;
        auth.remote_request(
            reqwest::Method::POST,
            "/v1/remote/native/hosts",
            Some(&json!({"hostId":host_id,"name":"Lomi desktop","bundle":bundle})),
        )
        .await?;
        let current = auth.remote_binding()?;
        if !current.same_authority(&binding) {
            return Err("Account session changed.".into());
        }
        {
            let mut core = self.core.lock().map_err(|_| "Remote unavailable.")?;
            if !core.activation_is_current(activation_revision, policy_revision)
                || !self.healthy.load(Ordering::SeqCst)
            {
                return Err("Remote settings changed while connecting. Try again.".into());
            }
            core.binding = Some(current);
            if let Some((policy, identity)) = loaded {
                core.policy = Some(policy);
                core.identity = Some(identity);
            }
            let terminals = app.state::<Terminals>();
            if reset_activity {
                terminals.activity.activate();
            } else {
                terminals.activity.restore();
                if terminals.activity.is_idle(SystemTime::now()) {
                    core.pause_idle();
                    return Ok(());
                }
            }
            core.enabled = true;
            core.paused = false;
            core.message = None;
            core.deadline = None;
        }
        self.poll(app).await
    }

    async fn publication_last_seen(auth: &AuthController, host_id: &str) -> Result<String, String> {
        let (_, hosts) = auth
            .remote_request(reqwest::Method::GET, "/v1/remote/native/hosts", None)
            .await?;
        hosts
            .get("hosts")
            .and_then(Value::as_array)
            .and_then(|hosts| {
                hosts
                    .iter()
                    .find(|host| host.get("id").and_then(Value::as_str) == Some(host_id))
            })
            .and_then(|host| host.get("lastSeen"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "Remote host publication unavailable.".into())
    }

    async fn poll(&self, app: &tauri::AppHandle) -> Result<(), String> {
        let publication = self.publication.lock().await;
        self.poll_locked(app, &publication).await
    }

    async fn poll_locked(
        &self,
        app: &tauri::AppHandle,
        _publication: &tokio::sync::MutexGuard<'_, ()>,
    ) -> Result<(), String> {
        self.reconcile_workspaces(app)?;
        let auth = app.state::<AuthController>();
        let binding = auth.remote_binding()?;
        let (host_id, shares, workspaces, policy_revision, activation_revision) = {
            let core = self.core.lock().map_err(|_| "Remote unavailable.")?;
            if !self.healthy.load(Ordering::SeqCst) {
                return Err("Remote is recovering terminal state. Try again shortly.".into());
            }
            if !core
                .binding
                .as_ref()
                .is_some_and(|b| b.same_authority(&binding))
                || !core.enabled
            {
                return Err("Remote connection is no longer active.".into());
            }
            let runtime = self
                .runtime
                .lock()
                .map_err(|_| "Terminal model unavailable.")?;
            (core.policy.as_ref().ok_or("Remote identity unavailable.")?.host_id.clone(),
                core.shares.iter().filter_map(|id| runtime.sessions.get(id)).filter(|s| s.available).map(|s| json!({"id":s.id,"epoch":s.epoch,"label":s.label,"cols":s.cols,"rows":s.rows})).collect::<Vec<_>>(), core.policy.as_ref().ok_or("Remote identity unavailable.")?.workspaces.iter().filter(|w| w.shared && core.domain.revision > 0 && core.domain.workspaces.iter().any(|d| d.id == w.id)).enumerate().map(|(i,w)| json!({"id":w.id,"epoch":w.epoch,"revision":w.revision,"label":format!("Workspace {}",i+1),"permissions":"control","sessionIds":w.sessions.iter().map(|s| s.0.clone()).collect::<Vec<_>>()})).collect::<Vec<_>>(),core.policy_revision,core.activation_revision)
        };
        let last_seen = Self::publication_last_seen(&auth, &host_id).await?;
        auth.remote_request(
            reqwest::Method::POST,
            &format!("/v1/remote/native/hosts/{host_id}/heartbeat"),
            Some(&json!({
                "shares":shares,"workspaces":workspaces,"expectedLastSeen":last_seen
            })),
        )
        .await?;
        let (_, state) = auth
            .remote_request(
                reqwest::Method::GET,
                &format!("/v1/remote/native/hosts/{host_id}/state"),
                None,
            )
            .await?;
        let pairings: Vec<Pairing> = serde_json::from_value(
            state
                .get("pairings")
                .cloned()
                .ok_or("Invalid Remote state.")?,
        )
        .map_err(|_| "Invalid Remote pairings.")?;
        let channels = state
            .get("channels")
            .and_then(Value::as_array)
            .ok_or("Invalid Remote channels.")?;
        let grants = state
            .get("grants")
            .and_then(Value::as_array)
            .ok_or("Invalid Remote grants.")?;
        let authorization = state
            .get("authorizationExpiresAt")
            .and_then(Value::as_str)
            .ok_or_else(|| "Invalid Remote deadline.".to_string())
            .and_then(parse_expiry)?;
        let current = auth.remote_binding()?;
        if pairings.len() > 32
            || channels.len() > 64
            || grants.len() > 64
            || authorization <= now()
            || !current.same_authority(&binding)
        {
            return Err("Remote authorization expired.".into());
        }
        {
            let mut core = self.core.lock().map_err(|_| "Remote unavailable.")?;
            if !core
                .binding
                .as_ref()
                .is_some_and(|b| b.same_authority(&binding))
                || !core.enabled
                || core.activation_revision != activation_revision
            {
                return Err("Remote connection is no longer active.".into());
            }
            core.deadline = Some(
                Instant::now()
                    + Duration::from_secs(
                        10.min(authorization - now())
                            .min(current.expires_at.saturating_sub(now())),
                    ),
            );
            core.binding = Some(current);
            core.message = None;
            let pending_pairings: HashSet<String> = pairings
                .iter()
                .filter(|p| p.status == "pending" && p.expires_at > now())
                .map(|p| p.id.clone())
                .collect();
            core.pairings = pairings
                .into_iter()
                .filter(|p| p.host_id == host_id && p.status == "pending" && p.expires_at > now())
                .collect();
            // Server can revoke a local grant, but can never create one.
            if core.policy_revision == policy_revision {
                core.reconcile_cloud_grants(grants, &pending_pairings)?;
            }
            let active: HashSet<&str> = channels
                .iter()
                .filter_map(|c| c.get("id").and_then(Value::as_str))
                .collect();
            core.channels.retain(|id, stop| {
                if active.contains(id.as_str()) {
                    true
                } else {
                    stop.store(true, Ordering::SeqCst);
                    false
                }
            });
        }
        self.update_workspace_grants(app, grants).await?;
        for wire in channels {
            channel::start(self.clone(), app.clone(), wire.clone()).await?;
        }
        Ok(())
    }

    async fn approve(
        &self,
        app: &tauri::AppHandle,
        pairing_id: String,
        fingerprint: String,
        session_ids: Vec<String>,
        permissions: Permissions,
    ) -> Result<(), String> {
        let auth = app.state::<AuthController>();
        let binding = auth.remote_binding()?;
        let (signed, grant_id) = {
            let mut core = self.core.lock().map_err(|_| "Remote unavailable.")?;
            if !core
                .binding
                .as_ref()
                .is_some_and(|b| b.same_authority(&binding))
                || !core.deadline.is_some_and(|d| d > Instant::now())
            {
                return Err("Remote authorization expired.".into());
            }
            if session_ids.is_empty()
                || session_ids.len() > 32
                || session_ids.iter().collect::<HashSet<_>>().len() != session_ids.len()
                || session_ids.iter().any(|id| !core.shares.contains(id))
            {
                return Err("Select explicitly shared sessions.".into());
            }
            let pairing = core
                .pairings
                .iter()
                .find(|p| p.id == pairing_id && p.expires_at > now())
                .ok_or("Pairing expired.")?
                .clone();
            let policy = core.policy.as_ref().ok_or("Remote identity unavailable.")?;
            let hostfp = policy.bundle.bundle.fingerprint()?;
            let devicefp = pairing.device_bundle.bundle.fingerprint()?;
            pairing.device_bundle.verify(&devicefp)?;
            let account_id = policy.bundle.bundle.account_id;
            let host_id = uuid_bytes(&policy.host_id)?;
            let device_id = uuid_bytes(&pairing.device_id)?;
            if pairing.host_bundle != policy.bundle
                || pairing.device_bundle.bundle.account_id != account_id
                || pairing.device_bundle.bundle.subject_id != device_id
                || pairing.device_bundle.bundle.role != lomi_remote_crypto::Role::Device
                || pairing.host_fingerprint != hex(&hostfp)
                || pairing.device_fingerprint != hex(&devicefp)
            {
                return Err("Pairing identity mismatch.".into());
            }
            let nonce = hex_bytes::<32>(&pairing.nonce)?;
            let expected = hex(&lomi_remote_crypto::pairing_fingerprint(
                &account_id,
                &host_id,
                &device_id,
                &hostfp,
                &devicefp,
                &nonce,
            ));
            // Exact full paste is intentional: no suffix matching or automatic clipboard read.
            if fingerprint != expected || pairing.pairing_fingerprint != expected {
                return Err("Paste the exact complete fingerprint shown by your browser.".into());
            }
            let grant_id = uuid()?;
            let signed = core
                .identity
                .as_ref()
                .ok_or("Remote identity unavailable.")?
                .sign_peer_approval(PeerApproval {
                    version: 1,
                    workspace_id: None,
                    workspace_epoch: None,
                    session_epochs: vec![],
                    account_id,
                    host_id,
                    device_id,
                    host_fingerprint: hostfp,
                    device_fingerprint: devicefp,
                    pairing_nonce: nonce,
                    grant_id: uuid_bytes(&grant_id)?,
                    session_ids: session_ids
                        .iter()
                        .map(|id| uuid_bytes(id))
                        .collect::<Result<Vec<_>, _>>()?,
                    permissions,
                    access_epoch: 1,
                    revision: 1,
                    expires_at: binding
                        .expires_at
                        .min(pairing.max_approval_expires_at)
                        .min(now() + 12 * 3600),
                })?;
            let policy = core.policy.as_mut().ok_or("Remote identity unavailable.")?;
            // Removed entries cannot authorize cloud regrants: channels require a retained local grant.
            policy
                .grants
                .retain(|g| !g.revoked && g.approval.approval.expires_at > now());
            if policy.grants.len() >= 32 {
                return Err("Revoke old devices before approving another.".into());
            }
            policy.grants.push(LocalGrant {
                id: grant_id.clone(),
                device_bundle: pairing.device_bundle,
                approval: signed.clone(),
                revoked: false,
                confirmed: false,
                pairing_id: None,
            });
            // Local permission and complete key pins are committed before cloud approval.
            if let Err(error) = policy.save() {
                policy.grants.pop();
                return Err(error);
            }
            core.policy_revision = core.policy_revision.wrapping_add(1);
            (signed, grant_id)
        };
        if auth
            .remote_request(
                reqwest::Method::POST,
                &format!("/v1/remote/native/pairings/{pairing_id}/approve"),
                Some(&json!({"signedApproval":signed})),
            )
            .await
            .is_err()
        {
            self.revoke_local(app, &grant_id)?;
            return Err("Pairing approval was not confirmed. Local access was revoked.".into());
        }
        {
            let mut core = self.core.lock().map_err(|_| "Remote unavailable.")?;
            let policy = core.policy.as_mut().ok_or("Remote identity unavailable.")?;
            let local = policy
                .grants
                .iter_mut()
                .find(|g| g.id == grant_id && !g.revoked)
                .ok_or("Local approval changed.")?;
            local.confirmed = true;
            if let Err(error) = policy.save() {
                if let Some(local) = policy.grants.iter_mut().find(|g| g.id == grant_id) {
                    local.confirmed = false;
                    local.revoked = true;
                }
                core.storage_failed();
                return Err(error);
            }
            core.policy_revision = core.policy_revision.wrapping_add(1);
        }
        self.poll(app).await
    }

    fn revoke_local(&self, app: &tauri::AppHandle, id: &str) -> Result<(), String> {
        let mut core = self.core.lock().map_err(|_| "Remote unavailable.")?;
        // Disconnect before attempting secure persistence or network access.
        for stop in core.channels.values() {
            stop.store(true, Ordering::SeqCst);
        }
        core.channels.clear();
        for session in &core.shares {
            app.state::<Terminals>().remote_revoke(session);
        }
        core.policy_revision = core.policy_revision.wrapping_add(1);
        let policy = core.policy.as_mut().ok_or("Remote identity unavailable.")?;
        let grant = policy
            .grants
            .iter_mut()
            .find(|g| g.id == id)
            .ok_or("Unknown Remote grant.")?;
        grant.revoked = true;
        if let Err(error) = policy.save() {
            core.storage_failed();
            return Err(error);
        }
        Ok(())
    }
}

pub(super) fn parse_expiry(text: &str) -> Result<u64, String> {
    let stamp = chrono::DateTime::parse_from_rfc3339(text)
        .map_err(|_| "Invalid Remote expiry.")?
        .timestamp();
    u64::try_from(stamp).map_err(|_| "Invalid Remote expiry.".into())
}
fn deserialize_expiry<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    let text = String::deserialize(deserializer)?;
    parse_expiry(&text).map_err(serde::de::Error::custom)
}

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(u64::MAX)
}
pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub(super) fn hex_bytes<const N: usize>(text: &str) -> Result<[u8; N], String> {
    if text.len() != N * 2
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("Invalid identity encoding.".into());
    }
    let mut out = [0; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)
            .map_err(|_| "Invalid identity encoding.")?;
    }
    Ok(out)
}
pub(super) fn uuid() -> Result<String, String> {
    use ring::rand::SecureRandom;
    let mut bytes = [0; 16];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "Entropy unavailable.")?;
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    Ok(uuid_text(&bytes))
}
pub(super) fn uuid_text(bytes: &[u8; 16]) -> String {
    let h = hex(bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}
pub(super) fn uuid_bytes(text: &str) -> Result<[u8; 16], String> {
    if text.len() != 36 || ![8, 13, 18, 23].iter().all(|&i| text.as_bytes()[i] == b'-') {
        return Err("Invalid UUID.".into());
    }
    hex_bytes(&text.replace('-', ""))
}

fn settings(window: &Window) -> Result<(), String> {
    if window.label() == "settings" {
        Ok(())
    } else {
        Err("Remote management is available only in Settings.".into())
    }
}
#[tauri::command]
pub fn remote_get_state(window: Window, remote: State<'_, Remote>) -> Result<RemoteState, String> {
    if !matches!(window.label(), "main" | "settings") {
        return Err("Untrusted Remote caller.".into());
    }
    Ok(remote.state())
}
#[tauri::command]
pub async fn remote_set_enabled(
    window: Window,
    remote: State<'_, Remote>,
    enabled: bool,
) -> Result<RemoteState, String> {
    settings(&window)?;
    remote.enable(window.app_handle(), enabled).await?;
    Ok(remote.state())
}
#[tauri::command]
pub async fn remote_resume(
    window: Window,
    remote: State<'_, Remote>,
) -> Result<RemoteState, String> {
    if !matches!(window.label(), "main" | "settings") {
        return Err("Untrusted Remote caller.".into());
    }
    remote.enable(window.app_handle(), true).await?;
    Ok(remote.state())
}
#[tauri::command]
pub fn remote_note_activity(window: Window, terminals: State<'_, Terminals>) -> Result<(), String> {
    if !matches!(window.label(), "main" | "settings") {
        return Err("Untrusted Remote caller.".into());
    }
    terminals.activity.record();
    Ok(())
}
#[tauri::command]
pub fn remote_share_session(
    window: Window,
    remote: State<'_, Remote>,
    id: String,
    shared: bool,
) -> Result<RemoteState, String> {
    settings(&window)?;
    uuid_bytes(&id)?;
    let mut core = remote.core.lock().map_err(|_| "Remote unavailable.")?;
    if shared {
        if !core.enabled {
            return Err("Enable Remote first.".into());
        }
        if !remote
            .runtime
            .lock()
            .map_err(|_| "Terminal unavailable.")?
            .sessions
            .get(&id)
            .is_some_and(|s| s.available)
        {
            return Err("Terminal is no longer running.".into());
        }
        if core.shares.len() >= 32 {
            return Err("Remote supports at most 32 shared sessions.".into());
        }
        core.legacy_shares.insert(id.clone());
        core.shares.insert(id);
    } else {
        core.legacy_shares.remove(&id);
        core.shares.remove(&id);
        window.state::<Terminals>().remote_revoke(&id);
        for stop in core.channels.values() {
            stop.store(true, Ordering::SeqCst);
        }
        core.channels.clear();
    }
    drop(core);
    Ok(remote.state())
}
#[tauri::command(rename_all = "camelCase")]
pub async fn remote_approve_pairing(
    window: Window,
    remote: State<'_, Remote>,
    pairing_id: String,
    fingerprint: String,
    session_ids: Vec<String>,
    permissions: Permissions,
) -> Result<RemoteState, String> {
    settings(&window)?;
    uuid_bytes(&pairing_id)?;
    remote
        .approve(
            window.app_handle(),
            pairing_id,
            fingerprint,
            session_ids,
            permissions,
        )
        .await?;
    Ok(remote.state())
}
#[tauri::command(rename_all = "camelCase")]
pub async fn remote_deny_pairing(
    window: Window,
    remote: State<'_, Remote>,
    pairing_id: String,
) -> Result<RemoteState, String> {
    settings(&window)?;
    uuid_bytes(&pairing_id)?;
    window
        .state::<AuthController>()
        .remote_request(
            reqwest::Method::POST,
            &format!("/v1/remote/native/pairings/{pairing_id}/deny"),
            Some(&json!({})),
        )
        .await?;
    Ok(remote.state())
}
#[tauri::command(rename_all = "camelCase")]
pub async fn remote_revoke_grant(
    window: Window,
    remote: State<'_, Remote>,
    grant_id: String,
) -> Result<RemoteState, String> {
    settings(&window)?;
    uuid_bytes(&grant_id)?;
    remote.revoke_local(window.app_handle(), &grant_id)?;
    window
        .state::<AuthController>()
        .remote_request(
            reqwest::Method::POST,
            &format!("/v1/remote/native/grants/{grant_id}/revoke"),
            Some(&json!({})),
        )
        .await?;
    Ok(remote.state())
}

#[tauri::command]
pub fn hide_main_window(window: Window) -> Result<(), String> {
    crate::files::main_window(&window)?;
    window
        .hide()
        .map_err(|_| "Could not hide the workspace.".into())
}
#[tauri::command]
pub fn request_quit(window: Window) -> Result<(), String> {
    if !matches!(window.label(), "main" | "settings") {
        return Err("Untrusted caller.".into());
    }
    window.app_handle().exit(0);
    Ok(())
}
#[tauri::command]
pub fn reopen_main_window(window: Window) -> Result<(), String> {
    settings(&window)?;
    let main = window
        .app_handle()
        .get_window("main")
        .ok_or("Workspace unavailable.")?;
    main.unminimize().map_err(|_| "Workspace unavailable.")?;
    main.show().map_err(|_| "Workspace unavailable.")?;
    main.set_focus()
        .map_err(|_| "Workspace unavailable.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_pause_retains_consent_and_fences_pending_activation() {
        let (mut policy, grant) = fixture_grant();
        policy.grants.push(grant.clone());
        let binding = crate::auth::controller::RemoteBinding {
            user_id: "account".into(),
            session_id: "parent".into(),
            expires_at: now() + 7200,
            generation: 1,
        };
        let mut core = Core {
            enabled: true,
            policy: Some(policy),
            binding: Some(binding.clone()),
            deadline: Some(Instant::now() + Duration::from_secs(10)),
            ..Core::default()
        };
        let stop = Arc::new(AtomicBool::new(false));
        core.channels.insert("active".into(), stop.clone());
        core.shares.insert("session".into());
        let operation = core.activation_revision;
        let policy_revision = core.policy_revision;
        core.pause_idle();
        assert!(!core.enabled);
        assert!(core.paused);
        assert!(core.deadline.is_none());
        assert!(stop.load(Ordering::SeqCst));
        assert!(core.channels.is_empty());
        assert!(core.shares.contains("session"));
        assert_eq!(
            core.policy.as_ref().unwrap().grants[0].approval,
            grant.approval
        );
        assert_eq!(core.restore_binding.as_ref(), Some(&binding));
        assert!(!core.activation_is_current(operation, policy_revision));
        core.refresh_bound_authority(Some(crate::auth::controller::RemoteBinding {
            expires_at: binding.expires_at + 7200,
            ..binding
        }));
        assert!(core.paused);
        assert!(!core.enabled);
        let operation = core.activation_revision;
        assert!(core.activation_is_current(operation, policy_revision));
        core.policy_revision += 1;
        assert!(!core.activation_is_current(operation, policy_revision));
    }
    fn fixture_grant() -> (Policy, LocalGrant) {
        let (policy, host) = Policy::probe("account", "parent", [1; 16]).unwrap();
        let device =
            Identity::generate([1; 16], [3; 16], lomi_remote_crypto::Role::Device, 1).unwrap();
        let id = uuid().unwrap();
        let signed = host
            .sign_peer_approval(PeerApproval {
                version: 2,
                account_id: [1; 16],
                host_id: uuid_bytes(&policy.host_id).unwrap(),
                device_id: [3; 16],
                host_fingerprint: policy.bundle.bundle.fingerprint().unwrap(),
                device_fingerprint: device.public_bundle().bundle.fingerprint().unwrap(),
                pairing_nonce: [8; 32],
                grant_id: uuid_bytes(&id).unwrap(),
                workspace_id: Some([9; 16]),
                workspace_epoch: Some([10; 16]),
                session_ids: vec![],
                session_epochs: vec![],
                permissions: Permissions::Control,
                access_epoch: 1,
                revision: 1,
                expires_at: now() + 60,
            })
            .unwrap();
        (
            policy,
            LocalGrant {
                id,
                device_bundle: device.public_bundle(),
                approval: signed,
                revoked: false,
                confirmed: false,
                pairing_id: Some("initial-pairing".into()),
            },
        )
    }
    #[test]
    fn binding_renewal_preserves_consent_channels_and_current_expiry() {
        let (policy, _) = fixture_grant();
        let binding = crate::auth::controller::RemoteBinding {
            user_id: "account".into(),
            session_id: "parent".into(),
            expires_at: now() + 60,
            generation: 1,
        };
        let mut core = Core {
            enabled: true,
            binding: Some(binding.clone()),
            restore_binding: Some(binding.clone()),
            policy: Some(policy),
            deadline: Some(Instant::now() + Duration::from_secs(10)),
            ..Core::default()
        };
        let stop = Arc::new(AtomicBool::new(false));
        core.channels.insert("channel".into(), stop.clone());
        core.shares.insert("shared-session".into());
        let renewed = crate::auth::controller::RemoteBinding {
            expires_at: binding.expires_at + 120,
            ..binding
        };
        let deadline = core.deadline;
        core.refresh_bound_authority(Some(renewed.clone()));
        assert!(core.enabled);
        assert_eq!(core.binding.as_ref(), Some(&renewed));
        assert_eq!(core.restore_binding.as_ref(), Some(&renewed));
        assert!(core.policy.is_some());
        assert!(core.shares.contains("shared-session"));
        assert!(core.channels.contains_key("channel"));
        assert!(!stop.load(Ordering::SeqCst));
        assert_eq!(core.deadline, deadline);
    }

    #[test]
    fn authority_changes_and_missing_live_authorization_stop_channels() {
        let binding = crate::auth::controller::RemoteBinding {
            user_id: "account".into(),
            session_id: "parent".into(),
            expires_at: now() + 60,
            generation: 1,
        };
        for current in [
            Some(crate::auth::controller::RemoteBinding {
                user_id: "other".into(),
                ..binding.clone()
            }),
            Some(crate::auth::controller::RemoteBinding {
                session_id: "other".into(),
                ..binding.clone()
            }),
            Some(crate::auth::controller::RemoteBinding {
                generation: 2,
                ..binding.clone()
            }),
            None,
        ] {
            let stop = Arc::new(AtomicBool::new(false));
            let mut core = Core {
                enabled: true,
                binding: Some(binding.clone()),
                ..Core::default()
            };
            core.channels.insert("channel".into(), stop.clone());
            core.shares.insert("shared-session".into());
            core.refresh_bound_authority(current);
            assert!(!core.enabled);
            assert!(core.binding.is_none());
            assert!(core.shares.is_empty());
            assert!(stop.load(Ordering::SeqCst));
        }
    }

    #[test]
    fn account_switch_fences_old_credentials_and_preserves_domain_for_new_parent() {
        let (policy, _) = fixture_grant();
        let mut core = Core {
            enabled: true,
            policy: Some(policy),
            binding: Some(crate::auth::controller::RemoteBinding {
                user_id: "old-account".into(),
                session_id: "old-parent".into(),
                expires_at: now() + 60,
                generation: 1,
            }),
            ..Core::default()
        };
        let domain_epoch = core.domain.begin().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        core.channels.insert("channel".into(), stop.clone());
        core.shares.insert(uuid().unwrap());
        core.reset_bound_authority();
        assert!(!core.enabled);
        assert!(core.binding.is_none());
        assert!(core.policy.is_none());
        assert!(core.identity.is_none());
        assert!(core.shares.is_empty());
        assert!(core.restore_binding.is_none());
        assert!(stop.load(Ordering::SeqCst));
        assert_eq!(core.domain.epoch.as_deref(), Some(domain_epoch.as_str()));
        core.binding = Some(crate::auth::controller::RemoteBinding {
            user_id: "new-account".into(),
            session_id: "new-parent".into(),
            expires_at: now() + 60,
            generation: 2,
        });
        assert_ne!(core.binding.as_ref().unwrap().session_id, "old-parent");
    }
    #[test]
    fn replaced_pending_approval_and_revoked_inflight_scope_do_not_poison_other_enrollment() {
        let (mut policy, grant) = fixture_grant();
        let id = grant.id.clone();
        policy.grants.push(grant);
        let mut core = Core {
            policy: Some(policy),
            ..Core::default()
        };
        let pending = HashSet::from(["initial-pairing".into()]);
        core.reconcile_cloud_grants(&[], &pending).unwrap();
        assert!(!core.policy.as_ref().unwrap().grants[0].revoked);
        // The initial response may be lost after commit; its active cloud grant preserves retry.
        core.reconcile_cloud_grants(&[json!({"id":id})], &HashSet::new())
            .unwrap();
        assert!(!core.policy.as_ref().unwrap().grants[0].revoked);
        let old_stop = Arc::new(AtomicBool::new(false));
        core.channels.insert("old-channel".into(), old_stop.clone());
        core.channel_grants.insert("old-channel".into(), id);
        // Replacement removed both the old pending pairing and its unconfirmed grant.
        core.reconcile_cloud_grants(&[], &HashSet::new()).unwrap();
        assert!(core.policy.as_ref().unwrap().grants[0].revoked);
        assert!(old_stop.load(Ordering::SeqCst));
        let (_, mut next) = fixture_grant();
        next.pairing_id = None;
        let next_id = next.id.clone();
        core.policy.as_mut().unwrap().grants.push(next);
        core.reconcile_cloud_grants(&[], &pending).unwrap();
        assert!(
            core.policy
                .as_ref()
                .unwrap()
                .grants
                .iter()
                .find(|g| g.id == next_id)
                .unwrap()
                .revoked
        );
        core.prune_inactive_grants().unwrap();
        assert!(core.policy.as_ref().unwrap().grants.is_empty());
        let (_, fresh) = fixture_grant();
        core.policy.as_mut().unwrap().grants.push(fresh);
        core.reconcile_cloud_grants(&[], &pending).unwrap();
        assert!(!core.policy.as_ref().unwrap().grants[0].revoked);
    }
    #[test]
    fn identifiers_expiries_and_disabled_defaults_fail_closed() {
        let text = uuid().unwrap();
        assert_eq!(uuid_text(&uuid_bytes(&text).unwrap()), text);
        assert!(uuid_bytes("AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA").is_err());
        assert!(uuid_bytes("12345678_1234_1234_1234_123456789abc").is_err());
        assert!(hex_bytes::<32>(&"a".repeat(63)).is_err());
        assert_eq!(parse_expiry("1970-01-01T00:01:00Z").unwrap(), 60);
        assert!(parse_expiry("-1").is_err());
        let remote = Remote::default();
        assert!(!remote.state().enabled);
        assert_eq!(
            remote.state().qualified,
            cfg!(all(target_os = "macos", target_arch = "aarch64"))
        );
        assert_eq!(remote.state().message.is_none(), LIVE_QUALIFIED);
        assert!(!remote.state().online);
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn install_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::{
        menu::{Menu, MenuItem},
        tray::TrayIconBuilder,
    };
    let show = MenuItem::with_id(
        app,
        "remote-show-workspace",
        "Show Lomi",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "remote-quit", "Quit Lomi", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut builder = TrayIconBuilder::with_id("lomi-background")
        .tooltip("Lomi")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "remote-show-workspace" => {
                if let Some(main) = app.get_window("main") {
                    let _ = main.unminimize();
                    let _ = main.show();
                    let _ = main.set_focus();
                }
            }
            "remote-quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    app.manage(builder.build(app)?);
    Ok(())
}
