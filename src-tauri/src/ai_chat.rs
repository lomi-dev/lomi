//! Chat AI v2: provider registry, thread storage, API streaming and system-CLI chat.
//! Data lives in `app_data/ai-chat/`; API keys live only in the system key store.
mod attachments;
mod cli;
mod commands;
mod http;
mod settings;
mod store;

pub(crate) use commands::*;

use settings::{ChatSettings, Loaded};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};
use store::Store;
use tauri::{AppHandle, Emitter, EventTarget, Manager};
use tokio::sync::Notify;

pub(crate) struct Inner {
    _owner: crate::storage_guard::Owner,
    pub dir: PathBuf,
    pub settings: ChatSettings,
    pub store: Store,
    invalid: Option<String>,
}

#[derive(Clone, Default)]
pub(crate) struct AiChat {
    inner: Arc<Mutex<Option<Inner>>>,
    /// Stop tokens of running streams by thread. Never held across `.await`.
    active: Arc<Mutex<HashMap<String, Arc<Notify>>>>,
    client: Arc<OnceLock<Result<reqwest::Client, String>>>,
    /// Process-group ids of running CLIs, killed on app exit. Never held across `.await`.
    groups: Arc<Mutex<HashSet<u32>>>,
}

impl AiChat {
    /// Runs `f` against the opened storage. Blocking: call from `spawn_blocking`.
    pub(crate) fn with<T>(
        &self,
        app: &AppHandle,
        f: impl FnOnce(&mut Inner) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| "Chat storage is unavailable.")?;
        if guard.is_none() {
            let root = app
                .path()
                .app_data_dir()
                .map_err(|_| "Chat storage is unavailable.")?
                .join("ai-chat");
            let owner = crate::storage_guard::Owner::acquire(root)?;
            let dir = owner.root.clone();
            let (settings, invalid) = match settings::load(&dir) {
                Loaded::Valid(s) => (s, None),
                Loaded::Invalid(message) => (ChatSettings::default(), Some(message)),
            };
            let store = Store::open(&dir)?;
            *guard = Some(Inner {
                _owner: owner,
                dir,
                settings,
                store,
                invalid,
            });
        }
        let inner = guard.as_mut().expect("initialized above");
        if let Some(message) = &inner.invalid {
            return Err(message.clone());
        }
        f(inner)
    }

    /// Tracks a CLI process group until the guard drops.
    fn track_group(&self, pid: Option<u32>) -> GroupGuard {
        if let (Some(pid), Ok(mut g)) = (pid, self.groups.lock()) {
            g.insert(pid);
        }
        GroupGuard(self.clone(), pid)
    }

    /// Kills every running CLI group; the app is exiting and tasks will not be dropped.
    pub(crate) fn kill_cli_groups(&self) {
        if let Ok(mut g) = self.groups.lock() {
            for pid in g.drain() {
                cli::kill_group(Some(pid));
            }
        }
    }

    fn client(&self) -> Result<reqwest::Client, String> {
        self.client.get_or_init(http::client).clone()
    }

    fn active(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, Arc<Notify>>>, String> {
        self.active
            .lock()
            .map_err(|_| "Chat state is unavailable.".into())
    }
}

struct GroupGuard(AiChat, Option<u32>);

impl Drop for GroupGuard {
    fn drop(&mut self) {
        if let (Some(pid), Ok(mut g)) = (self.1, self.0.groups.lock()) {
            g.remove(&pid);
        }
    }
}

/// Notifies the main webview. `revision` is the settings revision for the
/// "settings" scope and the thread revision (0 when deleted) for "threads".
fn emit_changed(app: &AppHandle, scope: &str, revision: u64, thread_id: Option<&str>) {
    let _ = app.emit_to(
        EventTarget::webview("main"),
        "ai-chat-changed",
        serde_json::json!({"scope": scope, "revision": revision, "threadId": thread_id}),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_guard_untracks_and_exit_drains() {
        let chat = AiChat::default();
        let guard = chat.track_group(Some(4_000_000_000)); // not a real group
        assert_eq!(chat.groups.lock().unwrap().len(), 1);
        drop(guard);
        assert!(chat.groups.lock().unwrap().is_empty());
        let _held = chat.track_group(Some(4_000_000_000));
        chat.kill_cli_groups();
        assert!(chat.groups.lock().unwrap().is_empty());
    }
}
