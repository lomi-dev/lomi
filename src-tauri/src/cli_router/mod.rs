mod adapters;
mod codex;
mod coding_approvals;
#[cfg(unix)]
mod coding_journal;
#[cfg(unix)]
mod coding_runtime;
#[cfg(unix)]
mod coding_transport;
mod commands;
mod credentials;
#[cfg(unix)]
mod effects;
#[cfg(unix)]
mod effects_store;
mod gateway_config;
mod gateway_http;
mod gateway_profiles;
#[cfg(unix)]
mod gateway_runtime;
#[cfg(unix)]
mod gateway_session;
#[cfg(unix)]
mod gateway_store;
#[cfg(test)]
mod gateway_tests;
mod gateway_transform;
mod gateway_url;
mod grants;
mod grok;
mod grok_artifact;
mod hermes;
mod kimi;
mod native_accounts;
#[cfg(unix)]
mod native_approvals;
mod native_capacity;
#[cfg(unix)]
mod native_collect;
#[cfg(unix)]
mod native_handoff;
#[cfg(unix)]
mod native_history;
#[cfg(unix)]
mod native_process;
#[cfg(unix)]
mod native_runtime;
#[cfg(unix)]
mod native_transfer;
#[cfg(unix)]
mod native_transfer_commands;
#[cfg(not(unix))]
mod native_unsupported;
mod native_wire;
mod openclaw;
mod opencode;
mod policy;
pub(crate) mod project_lease;
mod qwen;
mod qwen_store;
mod runtime;
mod store;
#[cfg(test)]
mod tests;
mod transport;
mod types;
mod usage;
mod vibe;

pub(crate) use coding_approvals::*;
pub(crate) use commands::*;
#[cfg(unix)]
pub(crate) use gateway_runtime::{
    terminal_command as gateway_terminal_command, Request as GatewayTerminalRequest,
};
#[cfg(unix)]
pub(crate) use native_approvals::*;
#[cfg(unix)]
pub(crate) use native_collect::*;
#[cfg(unix)]
pub(crate) use native_transfer_commands::*;
#[cfg(not(unix))]
pub(crate) use native_unsupported::*;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, Emitter, Manager};
use types::Snapshot;

#[derive(Clone, Default)]
pub(crate) struct CliRouterService {
    inner: Arc<Mutex<Option<Owner>>>,
    active: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    draining: Arc<Mutex<HashSet<String>>>,
    profile_terminals: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    closing: Arc<AtomicBool>,
    #[cfg(unix)]
    approvals: coding_approvals::Coordinator,
    #[cfg(unix)]
    native_approvals: native_approvals::Coordinator,
    quota_reader: Arc<tokio::sync::Mutex<()>>,
    storage_error: Arc<Mutex<Option<String>>>,
}

struct Owner {
    root: PathBuf,
    store: store::Store,
    quota_checks: HashMap<String, std::time::Instant>,
    quota_retry_at: HashMap<String, std::time::Instant>,
}

impl CliRouterService {
    fn with_store<T>(
        &self,
        app: &AppHandle,
        operation: impl FnOnce(&mut Owner) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| "Router service is unavailable.")?;
        if guard.is_none() {
            let parent = app
                .path()
                .app_data_dir()
                .map_err(|_| "Cannot locate router storage.")?;
            std::fs::create_dir_all(&parent).map_err(|_| "Cannot create application storage.")?;
            let root = parent.join("cli-router");
            let mut store = store::Store::open(&root.join("router.sqlite"))?;
            credentials::recover(&mut store)?;
            *guard = Some(Owner {
                root,
                store,
                quota_checks: HashMap::new(),
                quota_retry_at: HashMap::new(),
            });
        }
        let owner = guard.as_mut().ok_or("Router storage is unavailable.")?;
        let needs_reconciliation = {
            self.storage_error
                .lock()
                .map_err(|_| "Router storage status unavailable.")?
                .is_some()
        };
        if needs_reconciliation {
            if !self
                .active
                .lock()
                .map_err(|_| "Router ownership unavailable.")?
                .is_empty()
            {
                return Err(
                    "The CLI is stopping after a storage failure. Retry when it has finished."
                        .into(),
                );
            }
            // Worker ownership ends after process cleanup. Reconcile retained
            // intents and repair admission only after a successful full commit.
            let recovered = owner.store.reconcile_interrupted()?;
            *self
                .storage_error
                .lock()
                .map_err(|_| "Router storage status unavailable.")? = None;
            self.changed(app, recovered.revision);
        }
        operation(owner)
    }

    pub(crate) fn stop_all(&self) {
        self.closing.store(true, Ordering::SeqCst);
        if let Ok(active) = self.active.lock() {
            for cancellation in active.values() {
                cancellation.store(true, Ordering::SeqCst);
            }
        }
    }

    pub(crate) fn drain(&self, app: &AppHandle) -> Result<(), String> {
        self.stop_all();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if self
                .active
                .lock()
                .map_err(|_| "Router ownership unavailable.")?
                .is_empty()
            {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err("The CLI has not confirmed shutdown. Lomi remains open; stop its work and retry.".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let initialized = self
            .inner
            .lock()
            .map_err(|_| "Router service is unavailable.")?
            .is_some();
        if initialized {
            self.with_store(app, |owner| {
                let snapshot = owner.store.snapshot()?;
                if snapshot
                    .runs
                    .iter()
                    .any(|run| run.attempts.iter().any(|attempt| attempt.state.is_active()))
                {
                    owner.store.reconcile_interrupted()?;
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    fn changed(&self, app: &AppHandle, revision: u64) {
        // Child browser webviews must never receive router transcript events.
        for label in ["main", "settings"] {
            let _ = app.emit_to(
                tauri::EventTarget::webview(label),
                "cli-router-changed",
                serde_json::json!({"revision": revision}),
            );
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct View {
    #[serde(flatten)]
    snapshot: Snapshot,
    capabilities: Vec<adapters::Capability>,
}

fn view(mut snapshot: Snapshot, settings: bool) -> View {
    for profile in &mut snapshot.profiles {
        profile.credential_ref = None;
        profile.quota_group_key = None;
    }
    if settings {
        snapshot.runs.clear();
    }
    View {
        snapshot,
        capabilities: adapters::registry(),
    }
}

fn profile_directory(owner: &Owner, id: &str) -> Result<PathBuf, String> {
    if !crate::chat::process::valid_id(id) || id.len() != 32 {
        return Err("Invalid account profile.".into());
    }
    let parent = owner.root.join("profiles");
    crate::chat::storage::reject_link(&parent)?;
    std::fs::create_dir_all(&parent).map_err(|_| "Cannot create account profile storage.")?;
    crate::chat::storage::private(&parent, true)?;
    let directory = parent.join(id);
    crate::chat::storage::reject_link(&directory)?;
    std::fs::create_dir_all(&directory).map_err(|_| "Cannot create account profile storage.")?;
    crate::chat::storage::private(&directory, true)?;
    Ok(directory)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn new_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| "Cannot create a secure router identifier.")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn profile_writer_busy(state: &CliRouterService, id: &str) -> Result<bool, String> {
    let mut writers = state
        .profile_terminals
        .lock()
        .map_err(|_| "Router writer state unavailable.")?;
    if writers
        .get(id)
        .is_some_and(|done| done.load(Ordering::SeqCst))
    {
        writers.remove(id);
    }
    Ok(writers.contains_key(id))
}
