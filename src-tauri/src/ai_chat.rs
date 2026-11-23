//! Chat AI v2: provider registry, thread storage, API streaming and system-CLI chat.
//! Data lives in `app_data/ai-chat/`; API keys live only in the system key store.
//! The send engine lives in `lomi_chat::engine`; this module adapts it to Tauri.
mod commands;

use lomi_chat::{attachments, cli, engine, http, settings, store};

pub(crate) use commands::*;

use engine::{Engine, Hooks, Inner};
use std::sync::{Arc, OnceLock};
use tauri::{AppHandle, Emitter, EventTarget, Manager};

#[derive(Clone, Default)]
pub(crate) struct AiChat {
    engine: Arc<OnceLock<Engine>>,
}

impl AiChat {
    /// The shared engine, created on first use with the app data directory.
    pub(crate) fn engine(&self, app: &AppHandle) -> Result<&Engine, String> {
        if let Some(engine) = self.engine.get() {
            return Ok(engine);
        }
        let root = app
            .path()
            .app_data_dir()
            .map_err(|_| "Chat storage is unavailable.")?
            .join("ai-chat");
        let (keys, changed) = (app.clone(), app.clone());
        Ok(self.engine.get_or_init(|| {
            Engine::new(
                root,
                Hooks {
                    api_key: Box::new(move |id| read_key(&keys, id)),
                    threads_changed: Box::new(move |revision, id| {
                        emit_changed(&changed, "threads", revision, Some(id))
                    }),
                    resolve_cli: Box::new(lomi_chat::cli_bin::resolve),
                },
            )
        }))
    }

    /// Runs `f` against the opened storage. Blocking: call from `spawn_blocking`.
    pub(crate) fn with<T>(
        &self,
        app: &AppHandle,
        f: impl FnOnce(&mut Inner) -> Result<T, String>,
    ) -> Result<T, String> {
        self.engine(app)?.with(f)
    }

    /// Kills every running CLI group; the app is exiting and tasks will not be dropped.
    pub(crate) fn kill_cli_groups(&self) {
        if let Some(engine) = self.engine.get() {
            engine.kill_cli_groups();
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
