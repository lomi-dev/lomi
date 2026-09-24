//! Session-only local control. Enrollment needs a pinned broker certificate and
//! an explicit approval of the helper certificate in trusted application UI.
use crate::{
    authentication::{require_same_user, Identity},
    receipts,
};
use lomi_control_protocol::{
    control::*,
    framing::{read_frame, write_frame},
    ErrorCode, IPC_VERSION, MAX_FRAME_BYTES, MAX_METADATA_BYTES,
};
use rustls::pki_types::CertificateDer;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{UnixListener, UnixStream},
    sync::{oneshot, watch, Semaphore},
};
mod browsers;
mod chat_close;
mod operations;
pub use browsers::{BrowserNavigationDispatch, BrowserStart};
pub use chat_close::ChatCloseDispatch;
pub use operations::NativePermit;
mod panel_control;
pub use panel_control::{PendingControlView, TerminalAttachDispatch};
mod android;
mod editor;
mod editor_edits;
mod editor_open;
mod editor_save;
mod files;
mod files_mutate;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod git;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod git_mutate;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod git_observations;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod git_open;
pub use editor::EditorReadDispatch;
mod files_search;
pub use files::{DecodedFile, FilesReadDispatch};
pub use files_search::{FileSearchBatch, FilesSearchDispatch};
mod android_logs;
pub use android_logs::{AndroidLogBatch, AndroidLogcatDispatch};
mod android_launch;
pub use android_launch::AndroidLaunchDispatch;
mod android_install;
pub use android_install::{AndroidInstallDispatch, AndroidInstallRequest, PendingInstallView};
mod android_capture;
mod android_input;
mod android_snapshot;
pub use android_capture::{AndroidCapture, AndroidCaptureDispatch};
pub use android_snapshot::AndroidSnapshotDispatch;
mod chat;
mod chat_draft;
mod chat_export;
mod chat_send;
mod chat_stop;
pub use chat_draft::ChatDraftDispatch;
pub use chat_export::ChatExportDispatch;
pub use chat_send::{ChatSendAuthorization, ChatSendPrepareDispatch};
pub use chat_stop::ChatStopDispatch;
mod panel_move;
mod panel_transfer;
mod panels;
mod project_close;
mod project_open;
mod settings;
mod settings_read;
pub use chat::{ChatListDispatch, ChatOpenDispatch, ChatReadDispatch};
mod settings_update;
mod workspace_close;
mod workspaces;
pub use android::{AndroidListDispatch, AndroidListRequest, AndroidRuntimeDispatch};
pub use android_input::{AndroidInputDispatch, AndroidInputDispatchAction};
pub use panels::{BrowserCloseDispatch, TerminalCloseDispatch};
pub use project_open::PendingProjectOpenView;
pub use settings_read::SettingsReadDispatch;
pub use settings_update::{
    PendingSettingsUpdate, SettingsApply, SettingsPlan, SettingsPrepareDispatch,
};
mod artifacts;
mod browser_dom;
mod browser_logs;
mod imports;
pub use artifacts::{BrowserCapture, BrowserCaptureDispatch};
pub use browser_logs::BrowserLogsDispatch;
mod terminal_screen;
pub use browser_dom::BrowserSnapshotDispatch;
pub use terminal_screen::ScreenDispatch;
mod tasks;
mod terminal_input;
mod terminal_runs;
pub use terminal_input::{TerminalInputDispatch, TerminalInputRequest};
mod terminals;
pub use terminal_runs::{TerminalDispatch, TerminalDispatchRequest};
type UiDispatch = Arc<dyn Fn(UiCommand) -> io::Result<()> + Send + Sync>;

pub fn new_id() -> io::Result<String> {
    let mut bytes = [0; 16];
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(&mut bytes)
        .map_err(|_| io::Error::other("Cannot create control identity"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub fn certificate_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    pub instance_id: String,
    pub endpoint: PathBuf,
    pub broker_sha256: String,
    pub ipc_version: u16,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingView {
    pub id: String,
    pub client_label: String,
    pub certificate_sha256: String,
    pub seconds_remaining: u64,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub id: String,
    pub client_label: String,
    pub project_ids: Vec<String>,
    pub workspace_ids: Vec<String>,
    pub scopes: Vec<String>,
    pub browser_origins: Vec<String>,
    pub android_device_ids: Vec<String>,
    pub android_packages: Vec<String>,
    pub chat_conversations: Vec<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub endpoint: Endpoint,
    pub ui_ready: bool,
    pub workspaces: Vec<Workspace>,
    pub terminal_profile: Option<TerminalProfile>,
    pub pending: Vec<PendingView>,
    pub pending_controls: Vec<PendingControlView>,
    pub pending_project_opens: Vec<PendingProjectOpenView>,
    pub pending_settings_updates: Vec<PendingSettingsUpdate>,
    pub pending_installs: Vec<PendingInstallView>,
    pub sessions: Vec<SessionView>,
}

struct Pending {
    label: String,
    certificate_hash: String,
    expires: Instant,
    answer: oneshot::Sender<Grant>,
}
#[derive(Clone)]
struct ProjectGrant {
    project_id: String,
    project_path: String,
    project_directory: Option<Arc<crate::project_files::ProjectDirectory>>,
    workspaces: HashSet<String>,
}
#[derive(Clone)]
pub struct Grant {
    projects: HashMap<String, ProjectGrant>,
    pub scopes: HashSet<String>,
    pub browser_origins: Vec<lomi_control_protocol::browser::Origin>,
    pub browser_profile: Option<String>,
    pub android_devices: HashSet<String>,
    pub android_packages: HashSet<String>,
    pub chat_conversations: HashSet<String>,
    policy_revision: u64,
    terminal_profile: Option<TerminalProfile>,
}
impl Grant {
    fn workspace(&self, id: &str) -> Option<&ProjectGrant> {
        self.projects.values().find(|p| p.workspaces.contains(id))
    }
    fn permits(&self, workspace: &Workspace) -> bool {
        self.projects.get(&workspace.project_id).is_some_and(|p| {
            p.project_path == workspace.project_path && p.workspaces.contains(&workspace.id)
        })
    }
    fn workspace_ids(&self) -> Vec<String> {
        self.projects
            .values()
            .flat_map(|p| p.workspaces.iter().cloned())
            .collect()
    }
    fn operation(
        &self,
        store: &receipts::Store,
        owner: &str,
        operation: &str,
    ) -> Result<(String, receipts::Receipt), receipts::Error> {
        // Project membership remains required even for retired workspace receipts.
        // A grant contains at most sixteen explicitly approved project roots.
        for project in self.projects.keys() {
            match store.get(owner, project, operation) {
                Ok(receipt) => return Ok((project.clone(), receipt)),
                Err(receipts::Error::TargetNotFound) => {}
                Err(error) => return Err(error),
            }
        }
        Err(receipts::Error::TargetNotFound)
    }
}

struct Session {
    alive: Arc<AtomicBool>,
    peer_pid: Option<u32>,
    view: SessionView,
    grant: Grant,
    retry_epoch: String,
    selected: Option<String>,
    cursors: HashMap<String, Cursor>,
}
struct ConnectionAuthority(Arc<AtomicBool>);
impl Drop for ConnectionAuthority {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
struct Cursor {
    revision: String,
    offset: usize,
    view: String,
}
#[derive(Default)]
struct State {
    android: HashMap<String, android::OwnedAndroid>,
    projection: Projection,
    pending: BTreeMap<String, Pending>,
    sessions: HashMap<String, Session>,
    policy_revision: u64,
    work: HashMap<String, operations::Work>,
    terminals: HashMap<String, terminals::OwnedTerminal>,
    browsers: HashMap<String, browsers::OwnedBrowser>,
    runs: HashMap<String, terminal_runs::Run>,
    claims: HashMap<String, panel_control::Claim>,
    installs: HashMap<String, android_install::InstallJob>,
    android_logs: HashMap<String, android_logs::CachedLogs>,
    file_searches: HashMap<String, files_search::CachedSearch>,
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    git_statuses: HashMap<String, git::CachedStatus>,
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    git_permits: HashMap<String, git_open::GitPermit>,
    preview_permits: HashMap<String, editor_open::PreviewPermit>,
    git_panels: HashMap<String, (String, String)>,
}

pub struct Broker {
    pub endpoint: Endpoint,
    cleanup_runtime: tokio::runtime::Handle,
    identity: Identity,
    authorization: Arc<AtomicU64>,
    cleanup_running: AtomicBool,
    cleanup_revision: AtomicU64,
    cleanup_done: tokio::sync::Notify,
    cleanup_tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    workers: Mutex<tasks::Tasks>,
    shutdown: tokio::sync::Mutex<tasks::Shutdown>,
    ui_epoch: Mutex<String>,
    state: Mutex<State>,
    store: Mutex<receipts::Store>,
    stop: watch::Sender<bool>,
    revoked: watch::Sender<u64>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    dispatch: Mutex<Option<UiDispatch>>,
    terminal_dispatch: Mutex<Option<TerminalDispatch>>,
    terminal_input_dispatch: Mutex<Option<TerminalInputDispatch>>,
    terminal_close_dispatch: Mutex<Option<TerminalCloseDispatch>>,
    browser_close_dispatch: Mutex<Option<BrowserCloseDispatch>>,
    chat_close_dispatch: Mutex<Option<ChatCloseDispatch>>,
    terminal_attach_dispatch: Mutex<Option<TerminalAttachDispatch>>,
    files_read_dispatch: Mutex<Option<FilesReadDispatch>>,
    files_search_dispatch: Mutex<Option<FilesSearchDispatch>>,
    files_trash_dispatch: Mutex<Option<files_mutate::FilesTrashDispatch>>,
    file_reads: Arc<Semaphore>,
    settings_prepare_dispatch: Mutex<Option<SettingsPrepareDispatch>>,
    settings_read_dispatch: Mutex<Option<SettingsReadDispatch>>,
    chat_list_dispatch: Mutex<Option<ChatListDispatch>>,
    chat_read_dispatch: Mutex<Option<ChatReadDispatch>>,
    chat_open_dispatch: Mutex<Option<ChatOpenDispatch>>,
    chat_draft_dispatch: Mutex<Option<ChatDraftDispatch>>,
    chat_stop_dispatch: Mutex<Option<ChatStopDispatch>>,
    chat_export_dispatch: Mutex<Option<ChatExportDispatch>>,
    chat_send_prepare_dispatch: Mutex<Option<ChatSendPrepareDispatch>>,
    settings_reads: Mutex<HashMap<String, settings_read::PendingRead>>,
    editor_read_dispatch: Mutex<Option<EditorReadDispatch>>,
    editor_reads: Mutex<HashMap<String, editor::PendingRead>>,
    android_list_dispatch: Mutex<Option<AndroidListDispatch>>,
    android_logcat_dispatch: Mutex<Option<AndroidLogcatDispatch>>,
    android_snapshot_dispatch: Mutex<Option<AndroidSnapshotDispatch>>,
    android_capture_dispatch: Mutex<Option<AndroidCaptureDispatch>>,
    android_install_dispatch: Mutex<Option<AndroidInstallDispatch>>,
    android_observations: Arc<Semaphore>,
    browser_logs_dispatch: Mutex<Option<BrowserLogsDispatch>>,
    browser_snapshot_dispatch: Mutex<Option<BrowserSnapshotDispatch>>,
    browser_capture_dispatch: Mutex<Option<BrowserCaptureDispatch>>,
    screen_dispatch: Mutex<Option<ScreenDispatch>>,
    screens: Mutex<HashMap<String, terminal_screen::PendingScreen>>,
    _runtime: tempfile::TempDir,
}

// Contention is transient, but a native writer never waits indefinitely for a
// policy/parser lock. Revocation interrupts this wait independently of either lock.
fn bounded_lock<T>(mutex: &Mutex<T>, allowed: impl Fn() -> bool) -> Option<MutexGuard<'_, T>> {
    let deadline = Instant::now() + Duration::from_millis(25);
    loop {
        if !allowed() {
            return None;
        }
        match mutex.try_lock() {
            Ok(guard) => return Some(guard),
            Err(std::sync::TryLockError::Poisoned(_)) => return None,
            Err(std::sync::TryLockError::WouldBlock) => {}
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn failure() -> io::Error {
    io::Error::other("Agent control is unavailable")
}
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}
fn error(code: ErrorCode) -> Reply {
    let message = match code {
        ErrorCode::TargetNotFound => "The target is unavailable in this session",
        ErrorCode::UiNotReady => "Wait for the Lomi workspace to become ready",
        ErrorCode::PairingRequired => "Approve this session in Lomi Settings → Agent control",
        ErrorCode::ControlRevoked => "This control session has ended",
        ErrorCode::CursorExpired => "The list changed; request its first page again",
        ErrorCode::StorageUnavailable => "Durable operation storage is unavailable",
        _ => "The requested action is not available in this session",
    };
    Reply::error(code, message)
}

impl Broker {
    pub fn start(root: &Path) -> io::Result<Arc<Self>> {
        use std::os::unix::fs::PermissionsExt;
        let cleanup_runtime = tokio::runtime::Handle::try_current().map_err(|_| failure())?;
        let store = receipts::Store::open(root, now()).map_err(|_| failure())?;
        let runtime = tempfile::Builder::new()
            .prefix("lomi-control-")
            .tempdir_in("/tmp")?;
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700))?;
        let endpoint = runtime.path().join("control.sock");
        let listener = UnixListener::bind(&endpoint)?;
        fs::set_permissions(&endpoint, fs::Permissions::from_mode(0o600))?;
        let instance_id = new_id()?;
        let identity = Identity::ephemeral(&format!("{instance_id}.lomi.invalid"))?;
        let (stop, _) = watch::channel(false);
        let (revoked, _) = watch::channel(0);
        let broker = Arc::new(Self {
            cleanup_runtime,
            endpoint: Endpoint {
                instance_id,
                endpoint,
                broker_sha256: certificate_hash(identity.certificate().as_ref()),
                ipc_version: IPC_VERSION,
            },
            identity,
            authorization: Arc::new(AtomicU64::new(0)),
            cleanup_running: AtomicBool::new(false),
            cleanup_revision: AtomicU64::new(0),
            cleanup_done: tokio::sync::Notify::new(),
            cleanup_tasks: Mutex::new(Vec::new()),
            workers: Mutex::new(tasks::Tasks::default()),
            shutdown: tokio::sync::Mutex::new(tasks::Shutdown::default()),
            ui_epoch: Mutex::new(String::new()),
            state: Mutex::new(State::default()),
            store: Mutex::new(store),
            stop,
            revoked,
            task: Mutex::new(None),
            dispatch: Mutex::new(None),
            terminal_dispatch: Mutex::new(None),
            terminal_input_dispatch: Mutex::new(None),
            terminal_close_dispatch: Mutex::new(None),
            browser_close_dispatch: Mutex::new(None),
            chat_close_dispatch: Mutex::new(None),
            terminal_attach_dispatch: Mutex::new(None),
            files_read_dispatch: Mutex::new(None),
            files_search_dispatch: Mutex::new(None),
            files_trash_dispatch: Mutex::new(None),
            file_reads: Arc::new(Semaphore::new(2)),
            settings_prepare_dispatch: Mutex::new(None),
            settings_read_dispatch: Mutex::new(None),
            chat_list_dispatch: Mutex::new(None),
            chat_read_dispatch: Mutex::new(None),
            chat_open_dispatch: Mutex::new(None),
            chat_draft_dispatch: Mutex::new(None),
            chat_stop_dispatch: Mutex::new(None),
            chat_export_dispatch: Mutex::new(None),
            chat_send_prepare_dispatch: Mutex::new(None),
            settings_reads: Mutex::new(HashMap::new()),
            editor_read_dispatch: Mutex::new(None),
            editor_reads: Mutex::new(HashMap::new()),
            android_list_dispatch: Mutex::new(None),
            android_logcat_dispatch: Mutex::new(None),
            android_snapshot_dispatch: Mutex::new(None),
            android_capture_dispatch: Mutex::new(None),
            android_install_dispatch: Mutex::new(None),
            android_observations: Arc::new(Semaphore::new(2)),
            browser_logs_dispatch: Mutex::new(None),
            browser_snapshot_dispatch: Mutex::new(None),
            browser_capture_dispatch: Mutex::new(None),
            screen_dispatch: Mutex::new(None),
            screens: Mutex::new(HashMap::new()),
            _runtime: runtime,
        });
        let weak = Arc::downgrade(&broker);
        let mut stopping = broker.stop.subscribe();
        let task = tokio::spawn(async move {
            let handshakes = Arc::new(Semaphore::new(4));
            let connections = Arc::new(Semaphore::new(16));
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _=stopping.changed()=>break,
                    Some(_)=tasks.join_next(),if !tasks.is_empty()=>{},
                    incoming=listener.accept()=>{
                        let Ok((stream,_))=incoming else {break};
                        let Ok(permit)=handshakes.clone().try_acquire_owned() else {continue};
                        let Some(broker)=weak.upgrade() else {break};
                        let connections=connections.clone();
                        tasks.spawn(async move {
                            let mut stop=broker.stop.subscribe();
                            let mut revoked=broker.revoked.subscribe();
                            tokio::select! {
                                _=stop.changed()=>{},
                                _=revoked.changed()=>{},
                                _=broker.serve(stream,permit,connections)=>{},
                            }
                        });
                    }
                }
            }
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
        });
        *broker.task.lock().map_err(|_| failure())? = Some(task);
        Ok(broker)
    }

    fn lock_state(&self) -> io::Result<MutexGuard<'_, State>> {
        let state = self.state.lock().map_err(|_| failure())?;
        self.check_policy(&state)?;
        Ok(state)
    }
    #[cfg(test)]
    fn try_state(&self) -> io::Result<MutexGuard<'_, State>> {
        let state = self.state.try_lock().map_err(|_| failure())?;
        self.check_policy(&state)?;
        Ok(state)
    }
    fn writer_state(&self) -> Option<MutexGuard<'_, State>> {
        let expected = self.authorization.load(Ordering::SeqCst);
        let state = bounded_lock(&self.state, || {
            self.authorization.load(Ordering::SeqCst) == expected
        })?;
        self.check_policy(&state).ok()?;
        Some(state)
    }
    fn check_policy(&self, state: &State) -> io::Result<()> {
        if state.policy_revision != self.authorization.load(Ordering::SeqCst)
            || *self.ui_epoch.lock().map_err(|_| failure())? != state.projection.ui_epoch
        {
            return Err(failure());
        }
        Ok(())
    }
    fn revoke_now(&self) {
        let revision = self.authorization.fetch_add(1, Ordering::SeqCst) + 1;
        // This lane never waits for the policy mutex, a writer, or durable storage.
        self.revoked.send_replace(revision);
    }
    pub async fn shutdown(self: &Arc<Self>) {
        let mut shutdown = self.shutdown.lock().await;
        if matches!(*shutdown, tasks::Shutdown::Idle) {
            let broker = self.clone();
            *shutdown = tasks::Shutdown::Running(tokio::spawn(async move {
                broker.finish_shutdown().await;
            }));
        }
        if let tasks::Shutdown::Running(task) = &mut *shutdown {
            // Cancellation of a close caller cannot detach the shutdown owner.
            let _ = task.await;
            *shutdown = tasks::Shutdown::Complete;
        }
    }
    async fn finish_shutdown(self: &Arc<Self>) {
        self.revoke();
        self.stop.send_replace(true);
        let task = self.task.lock().ok().and_then(|mut task| task.take());
        if let Some(task) = task {
            let _ = task.await;
        }
        self.stop_workers().await;
        loop {
            self.wait_for_cleanup().await;
            let tasks = self
                .cleanup_tasks
                .lock()
                .map(|mut tasks| std::mem::take(&mut *tasks))
                .unwrap_or_default();
            if tasks.is_empty() {
                break;
            }
            // The completion notification precedes the worker's final Arc drop.
            // Join it before a caller drops us and reopens the receipt owner lock.
            for task in tasks {
                let _ = task.await;
            }
        }
    }
    /// Await deferred receipt settlement after authority has already been revoked.
    pub async fn wait_for_cleanup(&self) {
        loop {
            let done = self.cleanup_done.notified();
            tokio::pin!(done);
            done.as_mut().enable();
            if !self.cleanup_running.load(Ordering::SeqCst) {
                break;
            }
            done.await;
        }
    }
    pub fn revoke(self: &Arc<Self>) {
        self.revoke_now();
        self.schedule_cleanup();
    }
    fn schedule_cleanup(self: &Arc<Self>) {
        // This bookkeeping lock never contains policy, storage or domain work.
        // Register the task under it so shutdown cannot miss a just-started worker.
        let Ok(mut tasks) = self.cleanup_tasks.lock() else {
            return;
        };
        self.cleanup_revision.fetch_add(1, Ordering::SeqCst);
        if self.cleanup_running.swap(true, Ordering::SeqCst) {
            return;
        }
        tasks.retain(|task| !task.is_finished());
        let broker = self.clone();
        // Native menu and synchronous IPC callbacks have no ambient Tokio
        // context. Revocation uses the runtime that owns this broker's workers.
        tasks.push(self.cleanup_runtime.spawn_blocking(move || loop {
            let requested = broker.cleanup_revision.load(Ordering::SeqCst);
            let settled = if let Ok(mut state) = broker.state.lock() {
                if state.policy_revision != broker.authorization.load(Ordering::SeqCst) {
                    broker.end_work(&mut state, None);
                    state.sessions.clear();
                    state.pending.clear();
                    if broker.ui_epoch.lock().map(|e| e.is_empty()).unwrap_or(true) {
                        state.projection = Projection::default();
                    }
                    state.policy_revision = broker.authorization.load(Ordering::SeqCst);
                }
                let disconnected: Vec<_> = state
                    .sessions
                    .iter()
                    .filter(|(_, session)| !session.alive.load(Ordering::SeqCst))
                    .map(|(id, _)| id.clone())
                    .collect();
                for id in disconnected {
                    broker.end_work(&mut state, Some(&id));
                    state.sessions.remove(&id);
                }
                state.pending.retain(|_, pending| {
                    !pending.answer.is_closed() && pending.expires > Instant::now()
                });
                Some(state.policy_revision)
            } else {
                None
            };
            broker.cleanup_running.store(false, Ordering::SeqCst);
            if settled.is_none()
                || broker.cleanup_revision.load(Ordering::SeqCst) == requested
                || broker.cleanup_running.swap(true, Ordering::SeqCst)
            {
                broker.cleanup_done.notify_waiters();
                break;
            }
        }));
    }
    pub fn invalidate_ui(self: &Arc<Self>) {
        self.invalidate_ui_epoch(None);
    }
    pub fn invalidate_ui_epoch(self: &Arc<Self>, expected: Option<&str>) {
        let Ok(mut epoch) = self.ui_epoch.lock() else {
            self.revoke();
            return;
        };
        if expected.is_some_and(|e| e != *epoch) {
            return;
        }
        epoch.clear();
        self.revoke_now();
        drop(epoch);
        self.schedule_cleanup();
    }
    pub fn register_ui(&self) -> io::Result<String> {
        let epoch = new_id()?;
        let revision = {
            let mut current = self.ui_epoch.lock().map_err(|_| failure())?;
            self.revoke_now();
            *current = epoch.clone();
            self.authorization.load(Ordering::SeqCst)
        };
        let mut state = self.state.lock().map_err(|_| failure())?;
        if *self.ui_epoch.lock().map_err(|_| failure())? != epoch {
            return Err(failure());
        }
        self.end_work(&mut state, None);
        if self.authorization.load(Ordering::SeqCst) != revision
            || *self.ui_epoch.lock().map_err(|_| failure())? != epoch
        {
            return Err(failure());
        }
        state.policy_revision = revision;
        state.projection = Projection {
            ui_epoch: epoch.clone(),
            revision: "0".into(),
            ..Projection::default()
        };
        state.sessions.clear();
        state.pending.clear();
        Ok(epoch)
    }
    pub fn publish(&self, projection: Projection) -> io::Result<()> {
        if !valid_id(&projection.ui_epoch)
            || projection.revision.parse::<u64>().is_err()
            || projection.workspaces.len() > 500
            || serde_json::to_vec(&projection)
                .map_err(|_| failure())?
                .len()
                > MAX_METADATA_BYTES
        {
            return Err(failure());
        }
        let mut ids = HashSet::new();
        let mut roots = HashMap::new();
        for w in &projection.workspaces {
            if !valid_id(&w.id)
                || !valid_id(&w.project_id)
                || !ids.insert(&w.id)
                || w.name.len() > 256
                || w.project_name.len() > 256
                || w.name.contains('\0')
                || w.project_name.contains('\0')
                || w.project_path.len() > 4096
            {
                return Err(failure());
            }
            let path = Path::new(&w.project_path);
            if !path.is_absolute()
                || !path.is_dir()
                || fs::canonicalize(path)?.to_string_lossy() != w.project_path
            {
                return Err(failure());
            }
            if let Some(previous) = roots.insert(&w.project_id, &w.project_path) {
                if previous != &w.project_path {
                    return Err(failure());
                }
            }
        }
        let mut state = self.lock_state().map_err(|_| failure())?;
        if state.projection.ui_epoch != projection.ui_epoch
            || projection.revision.parse::<u64>().ok()
                <= state.projection.revision.parse::<u64>().ok()
        {
            return Err(failure());
        }
        let mut panel_ids = HashSet::new();
        for panel in &projection.panels {
            if !valid_id(&panel.id)
                || !valid_id(&panel.tab_id)
                || !panel_ids.insert(&panel.id)
                || !ids.contains(&panel.workspace_id)
                || panel.android_device_id.as_ref().is_some_and(|id| {
                    !lomi_control_protocol::android::valid_device_id(id) || panel.kind != "android"
                })
                || panel
                    .chat_conversation_id
                    .as_ref()
                    .is_some_and(|id| !valid_chat_id(id) || panel.kind != "chat")
                || panel.title.len() > 1024
                || panel.title.contains('\0')
                || panel
                    .terminal_session_id
                    .as_ref()
                    .is_some_and(|id| !valid_id(id))
                || panel
                    .browser_generation
                    .as_ref()
                    .is_some_and(|id| !valid_id(id) || panel.kind != "browser")
            {
                return Err(failure());
            }
        }
        if projection
            .focused_panel_id
            .as_ref()
            .is_some_and(|id| !panel_ids.contains(id))
        {
            return Err(failure());
        }
        if projection.workspaces.iter().any(|w| {
            w.active_panel_id.as_ref().is_some_and(|id| {
                !projection
                    .panels
                    .iter()
                    .any(|p| &p.id == id && p.workspace_id == w.id)
            })
        }) {
            return Err(failure());
        }
        Self::apply_panel_transfers(&mut state, &projection);
        Self::revoke_missing_settings_targets(&state, &projection);
        let State {
            terminals,
            android,
            browsers,
            work,
            ..
        } = &mut *state;
        terminals.retain(|generation, terminal| {
            let visible = projection.panels.iter().any(|p| p.id == terminal.panel && p.workspace_id == terminal.workspace && p.terminal_session_id.as_ref() == Some(generation));
            // A native start can precede its first domain publication.
            let pending = work.values().any(|w| matches!(&w.command.action, UiAction::CreateTerminal { terminal_session_id, .. } if terminal_session_id == generation));
            if !visible && !pending { if let Ok(mut control) = terminal.control.lock() { control.detach(); } }
            visible || pending
        });
        for target in android.values() {
            let selected = projection.panels.iter().find(|p| {
                Some(&p.id) == projection.focused_panel_id.as_ref()
                    && p.workspace_id == target.workspace
                    && p.kind == "android"
                    && p.android_device_id.as_ref() == Some(&target.control.device)
            });
            target.control.select(selected.map(|p| p.id.clone()));
        }
        browsers.retain(|generation, browser| {
            let visible = projection.panels.iter().any(|p| p.id == browser.control.panel_id && p.workspace_id == browser.workspace && p.browser_generation.as_ref() == Some(generation));
            let focused = projection.panels.iter().find(|p| Some(&p.id) == projection.focused_panel_id.as_ref());
            let selected = focused.is_some_and(|focused| projection.panels.iter().any(|p| p.id == browser.control.panel_id && p.browser_generation.as_ref() == Some(generation) && p.tab_id == focused.tab_id && p.workspace_id == focused.workspace_id));
            browser.control.set_selected_tab(selected);
            let pending = work.values().any(|w| matches!(&w.command.action, UiAction::CreateBrowser { browser_generation, .. } if browser_generation == generation));
            if !visible && !pending { browser.control.revoke(); }
            visible || pending
        });
        state
            .git_panels
            .retain(|id, _| projection.panels.iter().any(|p| &p.id == id));
        state.projection = projection;
        Ok(())
    }
    pub fn overview(&self) -> io::Result<Overview> {
        let mut state = self.state.lock().map_err(|_| failure())?;
        let authorized = self.check_policy(&state).is_ok();
        let ui_current =
            self.ui_epoch.lock().map_err(|_| failure())?.as_str() == state.projection.ui_epoch;
        state.pending.retain(|_, p| p.expires > Instant::now());
        Ok(Overview {
            endpoint: self.endpoint.clone(),
            ui_ready: ui_current
                && state.projection.revision != "0"
                && !state.projection.ui_epoch.is_empty(),
            workspaces: state.projection.workspaces.clone(),
            terminal_profile: state.projection.terminal_profile.clone(),
            pending_project_opens: Self::pending_project_opens(&state, authorized),
            pending_settings_updates: Self::pending_settings_updates(&state, authorized),
            pending_installs: state
                .installs
                .iter()
                .filter(|(_, j)| authorized && !j.running && j.deadline > Instant::now())
                .map(|(id, j)| j.view(id, &state))
                .collect(),
            pending_controls: state
                .claims
                .iter()
                .filter(|(_, c)| authorized && c.deadline > Instant::now())
                .map(|(operation, claim)| claim.view(operation, &state))
                .collect(),
            pending: state
                .pending
                .iter()
                .filter(|_| authorized)
                .map(|(id, p)| PendingView {
                    id: id.clone(),
                    client_label: p.label.clone(),
                    certificate_sha256: p.certificate_hash.clone(),
                    seconds_remaining: p
                        .expires
                        .saturating_duration_since(Instant::now())
                        .as_secs(),
                })
                .collect(),
            sessions: state
                .sessions
                .values()
                .filter(|_| authorized)
                .map(|s| s.view.clone())
                .collect(),
        })
    }
    pub fn approve(&self, id: &str, workspace_ids: &[String]) -> io::Result<()> {
        self.approve_scopes(id, workspace_ids, &["workspace.read".into()])
    }
    pub fn approve_scopes(
        &self,
        id: &str,
        workspace_ids: &[String],
        scopes: &[String],
    ) -> io::Result<()> {
        self.approve_policy(id, workspace_ids, scopes, &[])
    }
    pub fn approve_policy(
        &self,
        id: &str,
        workspace_ids: &[String],
        scopes: &[String],
        browser_origins: &[String],
    ) -> io::Result<()> {
        self.approve_domains(id, workspace_ids, scopes, browser_origins, &[])
    }
    pub fn approve_domains(
        &self,
        id: &str,
        workspace_ids: &[String],
        scopes: &[String],
        browser_origins: &[String],
        android_devices: &[String],
    ) -> io::Result<()> {
        self.approve_android_apps(
            id,
            workspace_ids,
            scopes,
            browser_origins,
            android_devices,
            &[],
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn approve_android_apps(
        &self,
        id: &str,
        workspace_ids: &[String],
        scopes: &[String],
        browser_origins: &[String],
        android_devices: &[String],
        android_packages: &[String],
    ) -> io::Result<()> {
        self.approve_chat_access(
            id,
            workspace_ids,
            scopes,
            browser_origins,
            android_devices,
            android_packages,
            &[],
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn approve_chat_access(
        &self,
        id: &str,
        workspace_ids: &[String],
        scopes: &[String],
        browser_origins: &[String],
        android_devices: &[String],
        android_packages: &[String],
        chat_conversations: &[String],
    ) -> io::Result<()> {
        if chat_conversations.len() > 64
            || chat_conversations.iter().any(|id| !valid_chat_id(id))
            || chat_conversations.iter().collect::<HashSet<_>>().len() != chat_conversations.len()
            || (!chat_conversations.is_empty() && !scopes.iter().any(|s| s == "chat.read"))
        {
            return Err(io::Error::other(
                "Chat history requires at most 64 exact selected conversations.",
            ));
        }
        let apps_enabled = scopes
            .iter()
            .any(|s| matches!(s.as_str(), "android.launch" | "android.logs"));
        if android_packages.len() > 16
            || apps_enabled == android_packages.is_empty()
            || android_packages
                .iter()
                .any(|p| !lomi_control_protocol::android::valid_package(p))
        {
            return Err(io::Error::other(
                "App launch and logs require one to sixteen exact Android package names.",
            ));
        }
        let android_enabled = scopes.iter().any(|s| s.starts_with("android."));
        if android_devices.len() > 16
            || android_enabled == android_devices.is_empty()
            || android_devices
                .iter()
                .any(|id| !lomi_control_protocol::android::valid_device_id(id))
        {
            return Err(io::Error::other(
                "Android access requires one to sixteen selected managed devices.",
            ));
        }
        if (scopes.iter().any(|s| {
            matches!(
                s.as_str(),
                "android.launch"
                    | "android.logs"
                    | "android.interact"
                    | "android.observe"
                    | "android.capture"
                    | "android.install"
            )
        }) && !scopes.iter().any(|s| s == "android.control"))
            || (scopes.iter().any(|s| s == "android.control")
                && !scopes.iter().any(|s| s == "android.read"))
            || (scopes.iter().any(|s| s == "artifact.import")
                && !scopes.iter().any(|s| s == "files.read"))
            || (scopes.iter().any(|s| s == "android.install")
                && !scopes.iter().any(|s| s == "files.read"))
            || (scopes.iter().any(|s| s == "git.read") && !scopes.iter().any(|s| s == "files.read"))
            || (scopes.iter().any(|s| {
                matches!(
                    s.as_str(),
                    "git.write"
                        | "git.execute"
                        | "git.network"
                        | "git.push"
                        | "git.discard"
                        | "git.pull"
                )
            }) && !["git.read", "git.write", "git.execute"]
                .iter()
                .all(|required| scopes.iter().any(|s| s == required)))
            || (scopes.iter().any(|s| s == "editor.read")
                && !scopes.iter().any(|s| s == "files.read"))
            || (scopes.iter().any(|s| s == "editor.write")
                && !scopes.iter().any(|s| s == "editor.read"))
            || (scopes
                .iter()
                .any(|s| matches!(s.as_str(), "files.create" | "files.rename" | "files.trash"))
                && !scopes.iter().any(|s| s == "files.mutate"))
            || (scopes.iter().any(|s| s == "files.mutate")
                && !scopes.iter().any(|s| s == "files.read"))
            || !scopes.iter().any(|s| s == "workspace.read")
            || (scopes
                .iter()
                .any(|s| matches!(s.as_str(), "git.push" | "git.pull"))
                && !scopes.iter().any(|s| s == "git.network"))
            || (scopes.iter().any(|s| s == "panel.move")
                && !scopes.iter().any(|s| s == "workspace.write"))
            || (scopes.iter().any(|s| s == "workspace.close")
                && !["workspace.write", "panel.close"]
                    .iter()
                    .all(|required| scopes.iter().any(|s| s == required)))
            || (scopes.iter().any(|s| s == "project.close")
                && !scopes.iter().any(|s| s == "workspace.close"))
            || (scopes.iter().any(|s| s == "project.open")
                && !["workspace.write", "panel.create"]
                    .iter()
                    .all(|required| scopes.iter().any(|s| s == required)))
            || (scopes.iter().any(|s| s == "settings.write")
                && !scopes.iter().any(|s| s == "settings.read"))
            || (scopes.iter().any(|s| s == "chat.open")
                && !["chat.read", "panel.create", "panel.focus"]
                    .iter()
                    .all(|required| scopes.iter().any(|s| s == required)))
            || (scopes.iter().any(|s| s == "chat.create")
                && !scopes.iter().any(|s| s == "chat.open"))
            || (scopes.iter().any(|s| {
                matches!(
                    s.as_str(),
                    "chat.draft" | "chat.send" | "chat.stop" | "chat.export"
                )
            }) && !scopes.iter().any(|s| s == "chat.read"))
            || scopes.len() > 64
            || scopes.iter().any(|s| {
                !matches!(
                    s.as_str(),
                    "chat.read"
                        | "chat.open"
                        | "chat.create"
                        | "chat.export"
                        | "chat.stop"
                        | "chat.draft"
                        | "chat.send"
                        | "settings.open"
                        | "settings.read"
                        | "settings.write"
                        | "files.read"
                        | "git.read"
                        | "git.write"
                        | "git.execute"
                        | "git.network"
                        | "git.push"
                        | "git.discard"
                        | "git.pull"
                        | "files.trash"
                        | "files.rename"
                        | "files.create"
                        | "files.mutate"
                        | "editor.read"
                        | "editor.write"
                        | "artifact.import"
                        | "android.observe"
                        | "android.capture"
                        | "android.launch"
                        | "android.logs"
                        | "android.install"
                        | "android.interact"
                        | "android.control"
                        | "android.read"
                        | "workspace.read"
                        | "workspace.close"
                        | "project.open"
                        | "project.close"
                        | "workspace.write"
                        | "panel.move"
                        | "panel.focus"
                        | "panel.close"
                        | "panel.create"
                        | "terminal.execute"
                        | "terminal.read"
                        | "browser.navigate"
                        | "browser.read"
                        | "browser.interact"
                        | "browser.capture_composite"
                )
            })
        {
            return Err(failure());
        }
        let browser_enabled = scopes.iter().any(|s| s.starts_with("browser."));
        if browser_origins.len() > 16 || browser_enabled == browser_origins.is_empty() {
            return Err(io::Error::other(
                "Browser access requires one to sixteen exact origins.",
            ));
        }
        let origins = browser_origins
            .iter()
            .map(|value| {
                lomi_control_protocol::browser::Origin::parse(value).map_err(io::Error::other)
            })
            .collect::<io::Result<Vec<_>>>()?;
        let mut state = self.lock_state().map_err(|_| failure())?;
        if state.projection.ui_epoch.is_empty() || state.projection.revision == "0" {
            return Err(failure());
        }
        if workspace_ids.is_empty() || workspace_ids.len() > 500 {
            return Err(failure());
        }
        let mut grant = Grant {
            projects: HashMap::new(),
            scopes: scopes.iter().cloned().collect(),
            policy_revision: state.policy_revision,
            terminal_profile: state.projection.terminal_profile.clone(),
            browser_origins: origins,
            android_devices: android_devices.iter().cloned().collect(),
            android_packages: android_packages.iter().cloned().collect(),
            chat_conversations: chat_conversations.iter().cloned().collect(),
            browser_profile: if browser_enabled {
                Some(new_id()?)
            } else {
                None
            },
        };
        for id in workspace_ids {
            let workspace = state
                .projection
                .workspaces
                .iter()
                .find(|w| w.id == *id)
                .ok_or_else(failure)?;
            if !grant.projects.contains_key(&workspace.project_id) {
                if grant.projects.len() >= 16 {
                    return Err(failure());
                }
                let project_directory = if grant.scopes.contains("files.read") {
                    Some(Arc::new(
                        crate::project_files::ProjectDirectory::open(Path::new(
                            &workspace.project_path,
                        ))
                        .map_err(|_| failure())?,
                    ))
                } else {
                    None
                };
                grant.projects.insert(
                    workspace.project_id.clone(),
                    ProjectGrant {
                        project_id: workspace.project_id.clone(),
                        project_path: workspace.project_path.clone(),
                        project_directory,
                        workspaces: HashSet::new(),
                    },
                );
            }
            let project = grant
                .projects
                .get_mut(&workspace.project_id)
                .ok_or_else(failure)?;
            if project.project_path != workspace.project_path {
                return Err(failure());
            }
            project.workspaces.insert(id.clone());
        }
        let pending = state.pending.remove(id).ok_or_else(failure)?;
        if pending.expires <= Instant::now() {
            return Err(failure());
        }
        pending.answer.send(grant).map_err(|_| failure())
    }
    pub fn reject(&self, id: &str) {
        if let Ok(mut state) = self.lock_state() {
            state.pending.remove(id);
        }
    }

    async fn serve(
        self: Arc<Self>,
        mut stream: UnixStream,
        handshake: tokio::sync::OwnedSemaphorePermit,
        connections: Arc<Semaphore>,
    ) -> io::Result<()> {
        require_same_user(&stream)?;
        let peer_pid = crate::authentication::peer_pid(&stream);
        let hello: Enrollment = read_frame(&mut stream, 16 * 1024, Duration::from_secs(10)).await?;
        if hello.ipc_version != IPC_VERSION
            || hello.instance_id != self.endpoint.instance_id
            || hello.certificate.is_empty()
            || hello.certificate.len() > 8192
            || hello.client_label.is_empty()
            || hello.client_label.len() > 80
            || hello.client_label.chars().any(char::is_control)
        {
            return Err(failure());
        }
        let id = new_id()?;
        let (answer, decision) = oneshot::channel();
        let pending_broker = self.clone();
        let pending_id = id.clone();
        let pending_label = hello.client_label.clone();
        let certificate_sha256 = certificate_hash(&hello.certificate);
        self.spawn_worker(move || {
            let mut state = pending_broker.lock_state()?;
            if state.pending.len() >= 4 || answer.is_closed() {
                return Err(failure());
            }
            state.pending.insert(
                pending_id,
                Pending {
                    label: pending_label,
                    certificate_hash: certificate_sha256,
                    expires: Instant::now() + Duration::from_secs(120),
                    answer,
                },
            );
            Ok(())
        })
        .await
        .map_err(|_| failure())??;
        // Only public certificates are exchanged before trusted UI approval.
        let welcome = Welcome {
            ipc_version: IPC_VERSION,
            instance_id: self.endpoint.instance_id.clone(),
            certificate: self.identity.certificate().as_ref().to_vec(),
            pairing_request_id: id.clone(),
        };
        let exchange = async {
            write_frame(&mut stream, &welcome, 16 * 1024, Duration::from_secs(5)).await?;
            let grant = tokio::time::timeout(Duration::from_secs(120), decision)
                .await
                .map_err(|_| failure())?
                .map_err(|_| failure())?;
            let permit = connections.try_acquire_owned().map_err(|_| failure())?;
            let config = self
                .identity
                .server(CertificateDer::from(hello.certificate))?;
            let limited = HandshakeIo {
                inner: stream,
                remaining: 64 * 1024,
            };
            let mut tls = tokio::time::timeout(
                Duration::from_secs(10),
                tokio_rustls::TlsAcceptor::from(config).accept(limited),
            )
            .await
            .map_err(|_| failure())??;
            tls.get_mut().0.remaining = usize::MAX;
            drop(handshake);
            let session_policy = grant.policy_revision;
            let alive = Arc::new(AtomicBool::new(true));
            let _connection_authority = ConnectionAuthority(alive.clone());
            let session_broker = self.clone();
            let session_id = id.clone();
            let session_label = hello.client_label;
            self.spawn_worker(move || {
                if !alive.load(Ordering::SeqCst) {
                    return Err(failure());
                }
                let epoch = session_broker
                    .store
                    .lock()
                    .map_err(|_| failure())?
                    .issue_epoch(&session_id, now())
                    .map_err(|_| failure())?;
                {
                    let mut state = session_broker.lock_state().map_err(|_| failure())?;
                    if state.policy_revision != grant.policy_revision
                        || !alive.load(Ordering::SeqCst)
                    {
                        return Err(failure());
                    }
                    state.sessions.insert(
                        session_id.clone(),
                        Session {
                            alive,
                            peer_pid,
                            view: SessionView {
                                id: session_id.clone(),
                                client_label: session_label,
                                project_ids: grant.projects.keys().cloned().collect(),
                                workspace_ids: grant.workspace_ids(),
                                scopes: grant.scopes.iter().cloned().collect(),
                                android_device_ids: grant.android_devices.iter().cloned().collect(),
                                android_packages: grant.android_packages.iter().cloned().collect(),
                                chat_conversations: grant
                                    .chat_conversations
                                    .iter()
                                    .cloned()
                                    .collect(),
                                browser_origins: grant
                                    .browser_origins
                                    .iter()
                                    .map(|o| o.as_str().to_owned())
                                    .collect(),
                            },
                            grant,
                            retry_epoch: epoch,
                            selected: None,
                            cursors: HashMap::new(),
                        },
                    );
                }
                Ok(())
            })
            .await
            .map_err(|_| failure())??;
            let _permit = permit;
            loop {
                // Idle connections have no frame deadline; the five-second budget
                // starts with the first prefix byte, not while waiting for a call.
                use tokio::io::AsyncReadExt;
                let mut first = [0];
                tls.read_exact(&mut first).await?;
                let mut reader = first.as_slice().chain(&mut tls);
                let message: Message =
                    read_frame(&mut reader, MAX_FRAME_BYTES, Duration::from_secs(5)).await?;
                if message.ipc_version != IPC_VERSION
                    || !valid_id(&message.request_id)
                    || serde_json::to_vec(&message.request)
                        .map_err(|_| failure())?
                        .len()
                        > MAX_METADATA_BYTES
                {
                    return Err(failure());
                }
                // The private channel has one in-flight request (Client serializes
                // calls). Observe disconnects even while a storage worker is stalled.
                // Pipelining on this channel is a protocol violation, never a queue.
                let mut extra = [0];
                let result = tokio::select! {
                    result = self.call_async(&id, message.request) => result,
                    _ = tls.read(&mut extra) => return Err(failure()),
                };
                // Result construction and disclosure share the policy lock in call.
                // A revoke before writing suppresses the prepared result as well.
                let result = if self.authorization.load(Ordering::SeqCst) == session_policy {
                    result
                } else {
                    error(ErrorCode::ControlRevoked)
                };
                write_frame(
                    &mut tls,
                    &Response {
                        request_id: message.request_id,
                        result,
                    },
                    MAX_FRAME_BYTES,
                    Duration::from_secs(5),
                )
                .await?;
            }
        }
        .await;
        // ConnectionAuthority has already revoked the atomic lease. Cleanup may
        // wait on storage, but must never block the executor that observes EOF.
        self.schedule_cleanup();
        exchange
    }

    fn call(self: &Arc<Self>, id: &str, request: Request) -> Reply {
        match request {
            Request::FilesMutate(input) => return self.files_mutate(id, input),
            Request::EditorSave(input) => return self.editor_save(id, input),
            Request::EditorOpen(input) => return self.editor_open(id, input),
            Request::EditorEdits(input) => return self.editor_edits(id, input),
            Request::EditorRead(input) => return self.editor_read(id, input),
            Request::FilesSearch(input) => return self.files_search(id, input),
            Request::FilesList(input) => return self.files_list(id, input),
            Request::FilesRead(input) => return self.files_read(id, input),
            Request::GitDiff(input) => {
                #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
                {
                    return self.git_diff(id, input);
                }
                #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
                {
                    let _ = input;
                    return error(ErrorCode::HostUnqualified);
                }
            }
            Request::GitHistory(input) => {
                #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
                {
                    return self.git_history(id, input);
                }
                #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
                {
                    let _ = input;
                    return error(ErrorCode::HostUnqualified);
                }
            }
            Request::GitCommit(input) => {
                #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
                {
                    return self.git_commit(id, input);
                }
                #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
                {
                    let _ = input;
                    return error(ErrorCode::HostUnqualified);
                }
            }
            Request::GitRemotes(input) => {
                #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
                {
                    return self.git_remotes(id, input);
                }
                #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
                {
                    let _ = input;
                    return error(ErrorCode::HostUnqualified);
                }
            }
            Request::GitOpen(input) => {
                #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
                {
                    return self.git_open(id, input);
                }
                #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
                {
                    let _ = input;
                    return error(ErrorCode::HostUnqualified);
                }
            }
            Request::GitMutate(input) => {
                #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
                {
                    return self.git_mutate(id, input);
                }
                #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
                {
                    let _ = input;
                    return error(ErrorCode::HostUnqualified);
                }
            }
            Request::GitStatus(input) => {
                #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
                {
                    return self.git_status(id, input);
                }
                #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
                {
                    let _ = input;
                    return error(ErrorCode::HostUnqualified);
                }
            }
            Request::AndroidScreenshot(input) => return self.android_screenshot(id, input),
            Request::AndroidLogcat(input) => return self.android_logcat(id, input),
            Request::AndroidSnapshot(input) => return self.android_snapshot(id, input),
            Request::AndroidInput(input) => return self.android_input(id, input),
            Request::AndroidStart(input) => return self.android_runtime(id, input, None),
            Request::AndroidStop(input) => {
                return self.android_runtime(
                    id,
                    AndroidStartInput {
                        workspace_id: input.workspace_id,
                        panel_id: input.panel_id,
                        device_id: input.device_id,
                        expected_revision: input.expected_revision,
                        retry_epoch: input.retry_epoch,
                        request_key: input.request_key,
                    },
                    Some(input.generation),
                )
            }
            Request::AndroidOpen(input) => return self.android_open(id, input),
            Request::AndroidList(input) => return self.android_list(id, input),
            Request::BrowserLogs(input) => return self.browser_logs(id, input),
            Request::ScreenshotBrowser(input) => return self.screenshot_browser(id, input),
            Request::AndroidLaunch(input) => return self.android_launch(id, input),
            Request::AndroidInstall(input) => return self.android_install(id, input),
            Request::ImportArtifact(input) => return self.import_artifact(id, input),
            Request::ReadArtifact(input) => return self.read_artifact(id, input),
            Request::WaitBrowser(input) => return self.wait_browser(id, input),
            Request::KeyBrowser(input) => {
                let (input, action) = input.split();
                return self.interact_browser(id, input, action);
            }
            Request::ScrollBrowser(input) => {
                let (input, action) = input.split();
                return self.interact_browser(id, input, action);
            }
            Request::ClickBrowser(input) => {
                return self.interact_browser(id, input, BrowserInteraction::Click)
            }
            Request::FillBrowser(input) => {
                let (input, action) = input.split();
                return self.interact_browser(id, input, action);
            }
            Request::SnapshotBrowser(input) => return self.snapshot_browser(id, input),
            Request::NavigateBrowser(input) => return self.navigate_browser(id, input),
            Request::OpenBrowser(input) => return self.open_browser(id, input),
            Request::RenameWorkspace(input) => return self.rename(id, input),
            Request::OpenProject(input) => return self.open_project(id, input),
            Request::OpenSettings(input) => return self.open_settings(id, input),
            Request::ReadSettings(input) => return self.read_settings(id, input),
            Request::ChatList(input) => return self.list_chats(id, input),
            Request::ChatRead(input) => return self.read_chat(id, input),
            Request::ChatSend(input) => return self.send_chat(id, input),
            Request::ChatStop(input) => return self.stop_chat(id, input),
            Request::ChatExport(input) => return self.export_chat(id, input),
            Request::ChatDraft(input) => return self.draft_chat(id, input),
            Request::ChatOpen(input) => return self.open_chat(id, input),
            Request::UpdateSettings(input) => return self.update_settings(id, input),
            Request::CloseProject(input) => return self.close_project(id, input),
            Request::CreateWorkspace(input) => return self.create_workspace(id, input),
            Request::CreateTerminal(input) => return self.create_terminal(id, input),
            Request::ReadTerminal(input) => return self.read_terminal(id, input),
            Request::Panels(input) => return self.list_panels(id, input),
            Request::MovePanel(input) => return self.move_panel(id, input),
            Request::FocusPanel(input) => return self.focus_panel(id, input),
            Request::ControlPanel(input) => {
                return match input {
                    PanelControlRequest::Terminal(input) => self.control_panel(id, input),
                    PanelControlRequest::Android(input) => self.android_input_control(id, input),
                }
            }
            Request::ClosePanel(input) => return self.close_panel(id, input),
            Request::Events(input) => return self.read_events(id, input),
            Request::RunTerminal(input) => return self.run_terminal(id, input),
            Request::InterruptTerminal(input) => return self.interrupt_terminal(id, input),
            Request::CancelOperation(input) => return self.cancel_operation(id, input),
            _ => {}
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::AppUnavailable);
        };
        let projection = state.projection.clone();
        let Some(session) = state
            .sessions
            .get_mut(id)
            .filter(|s| s.alive.load(Ordering::SeqCst))
        else {
            return error(ErrorCode::ControlRevoked);
        };
        let ready = !projection.ui_epoch.is_empty() && projection.revision != "0";
        let visible = |w: &&Workspace| session.grant.permits(w);
        match request {
            Request::Status(_)=>Reply::ok(Data::Status {connection:"connected".into(),pairing_request_id:None,instance_id:Some(self.endpoint.instance_id.clone()),ui_ready:ready,platform:std::env::consts::OS.into(),capabilities:["chat.export","chat.stop","chat.send","chat.draft","chat.read","chat.open","chat.create","settings.write","settings.read","settings.open","git.pull","git.discard","git.push","git.network","git.write","git.execute","git.read","files.trash","files.rename","files.create","files.mutate","editor.write","editor.read","android.logs","android.launch","android.install","files.read","artifact.import","android.capture","android.observe","android.interact","android.control","android.read","project.open", "project.close","workspace.close","workspace.read","workspace.write","panel.move","panel.focus","panel.close","panel.create","terminal.execute","terminal.read","browser.navigate","browser.read","browser.interact","browser.capture_composite"].into_iter().map(|name|Capability {name:name.into(),available:true,authorized:session.grant.scopes.contains(name),qualified:cfg!(all(target_os="macos",target_arch="aarch64"))}).collect(),limitations:vec!["Session-only pairing; authorization is shared by the process using this stdio channel".into(),"Shell cwd and browser origins are not OS or network sandboxes".into()]}),
            Request::Diagnostics(_)=>Reply::ok(Data::Diagnostics {connection:"connected".into(),ui_ready:ready,next_step:if ready{"List workspaces, then connect to an approved workspace"}else{"Wait for the main workspace window"}.into()}),
            Request::Connect(input)=>{
                if !ready{return error(ErrorCode::UiNotReady);}
                let Some(w)=projection.workspaces.iter().filter(visible).find(|w|w.id==input.workspace_id) else {return error(ErrorCode::TargetNotFound)};
                session.selected=Some(w.id.clone());
                Reply::ok(Data::Connected {instance_id:self.endpoint.instance_id.clone(),workspace_id:w.id.clone(),project_id:w.project_id.clone(),retry_epoch:session.retry_epoch.clone(),scopes:session.view.scopes.clone()})
            },
            Request::Workspaces(input)=>{
                if !ready{return error(ErrorCode::UiNotReady);}
                if input.limit==0||input.limit>500{return error(ErrorCode::ResourceExhausted);}
                let offset=if let Some(cursor)=input.cursor {
                    let Some(cursor)=session.cursors.get(&cursor) else {return error(ErrorCode::CursorExpired)};
                    if cursor.revision!=projection.revision || cursor.view!="workspaces" {return error(ErrorCode::CursorExpired);} cursor.offset
                }else{0};
                let all:Vec<_>=projection.workspaces.iter().filter(visible).collect();
                let mut items=Vec::new();let mut bytes=0;
                for w in all.iter().skip(offset).take(input.limit as usize) {
                    let length=serde_json::to_vec(w).map(|b|b.len()).unwrap_or(MAX_METADATA_BYTES);
                    if bytes+length>48*1024{break;}bytes+=length;items.push((*w).clone());
                }
                let next=offset+items.len();
                let next_cursor=if next<all.len(){
                    if session.cursors.len()>=64{session.cursors.clear();}
                    let Ok(cursor)=new_id() else {return error(ErrorCode::ResourceExhausted)};
                    session.cursors.insert(cursor.clone(),Cursor {revision:projection.revision.clone(),offset:next,view:"workspaces".into()});Some(cursor)
                }else{None};
                Reply::ok(Data::Workspaces {items,next_cursor,domain_revision:projection.revision})
            },
            Request::Operation(input)=>{
                if !ready{return error(ErrorCode::UiNotReady);}
                let Ok(store)=self.store.lock() else {return error(ErrorCode::StorageUnavailable)};
                let result=match input {
                    OperationLookup::Id(input)=>session.grant.operation(&store,id,&input.operation_id).map(|(_,receipt)|receipt),
                    OperationLookup::Key(input)=>{
                        let project = input.project_id.as_ref().filter(|p| session.grant.projects.contains_key(*p))
                            .or_else(|| if input.project_id.is_none() && session.grant.projects.len() == 1 {session.grant.projects.keys().next()} else {None});
                        let Some(project) = project else {return error(ErrorCode::TargetNotFound);};
                        store.lookup(&receipts::Key {pairing_id:id,project_id:project,retry_epoch:&input.retry_epoch,tool:&input.tool,request_key:&input.request_key})
                    },
                };
                match result {
                    Ok(receipt)=>{
                        if !Self::receipt_workspace_authorized(&state,id,&receipt){return error(ErrorCode::TargetNotFound);}
                        Self::terminal_receipt(&state, id, receipt)
                    },
                    Err(receipts::Error::TargetNotFound)=>error(ErrorCode::TargetNotFound),
                    _=>error(ErrorCode::StorageUnavailable),
                }
            }
            Request::ChatExport(_)|Request::ChatStop(_)|Request::ChatSend(_)|Request::ChatDraft(_)|Request::ChatOpen(_)|Request::ChatList(_)|Request::ChatRead(_)|Request::UpdateSettings(_)|Request::ReadSettings(_)|Request::OpenSettings(_)|Request::OpenProject(_)|Request::CloseProject(_)|Request::GitMutate(_)|Request::GitDiff(_)|Request::GitHistory(_)|Request::GitCommit(_)|Request::GitRemotes(_)|Request::GitOpen(_)|Request::GitStatus(_)|Request::FilesMutate(_)|Request::EditorSave(_)|Request::EditorOpen(_)|Request::EditorEdits(_)|Request::EditorRead(_)|Request::FilesSearch(_)|Request::FilesList(_)|Request::FilesRead(_)|Request::AndroidLogcat(_)|Request::AndroidLaunch(_)|Request::AndroidInstall(_)|Request::ImportArtifact(_)|Request::AndroidScreenshot(_)|Request::AndroidSnapshot(_)|Request::AndroidInput(_)|Request::AndroidStart(_)|Request::AndroidStop(_)|Request::AndroidOpen(_)|Request::AndroidList(_)|Request::BrowserLogs(_)|Request::ScreenshotBrowser(_)|Request::ReadArtifact(_)|Request::WaitBrowser(_)|Request::KeyBrowser(_)|Request::ScrollBrowser(_)|Request::ClickBrowser(_)|Request::FillBrowser(_)|Request::SnapshotBrowser(_)|Request::NavigateBrowser(_)|Request::OpenBrowser(_)|Request::RenameWorkspace(_)|Request::CreateWorkspace(_)|Request::CreateTerminal(_)|Request::ReadTerminal(_)|Request::RunTerminal(_)|Request::InterruptTerminal(_)|Request::InputTerminal(_)|Request::Panels(_)|Request::MovePanel(_)|Request::FocusPanel(_)|Request::ControlPanel(_)|Request::ClosePanel(_)|Request::Events(_)|Request::CancelOperation(_)=>unreachable!(),
        }
    }
}

impl Broker {
    fn read_events(&self, id: &str, input: WorkspaceListInput) -> Reply {
        if input.limit == 0 || input.limit > 500 {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::AppUnavailable);
        };
        let Some(session) = state
            .sessions
            .get(id)
            .filter(|s| s.alive.load(Ordering::SeqCst))
        else {
            return error(ErrorCode::ControlRevoked);
        };
        if session.grant.workspace(&input.workspace_id).is_none()
            || !state
                .projection
                .workspaces
                .iter()
                .any(|w| w.id == input.workspace_id && session.grant.permits(w))
        {
            return error(ErrorCode::TargetNotFound);
        }
        let view = format!("events:{}", input.workspace_id);
        let offset = if let Some(cursor) = input.cursor {
            let Some(cursor) = session
                .cursors
                .get(&cursor)
                .filter(|c| c.view == view && c.revision == "history-1")
            else {
                return error(ErrorCode::CursorExpired);
            };
            cursor.offset
        } else {
            0
        };
        let Ok(store) = self.store.lock() else {
            return error(ErrorCode::StorageUnavailable);
        };
        let Some(root) = session.grant.workspace(&input.workspace_id) else {
            return error(ErrorCode::TargetNotFound);
        };
        let events = match store.events(
            id,
            &root.project_id,
            &input.workspace_id,
            offset,
            input.limit + 1,
        ) {
            Ok(v) => v,
            Err(e) => return operations::storage_error(e),
        };
        let returned = events.len();
        let mut items = Vec::new();
        let mut bytes = 0;
        for event in events.into_iter().take(input.limit as usize) {
            let length = serde_json::to_vec(&event)
                .map(|v| v.len())
                .unwrap_or(MAX_METADATA_BYTES);
            if bytes + length > 48 * 1024 {
                break;
            }
            bytes += length;
            items.push(event);
        }
        let has_more = items.len() < returned;
        let next = offset + items.len();
        let Ok(cursor) = new_id() else {
            return error(ErrorCode::ResourceExhausted);
        };
        let session = state
            .sessions
            .get_mut(id)
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .unwrap();
        if session.cursors.len() >= 64 {
            session.cursors.clear();
        }
        session.cursors.insert(
            cursor.clone(),
            Cursor {
                revision: "history-1".into(),
                offset: next,
                view,
            },
        );
        Reply::ok(Data::Events {
            items,
            next_cursor: cursor,
            has_more,
            gap: false,
        })
    }
    fn list_panels(&self, id: &str, input: WorkspaceListInput) -> Reply {
        if input.limit == 0 || input.limit > 500 {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::AppUnavailable);
        };
        let Some(session) = state
            .sessions
            .get(id)
            .filter(|s| s.alive.load(Ordering::SeqCst))
        else {
            return error(ErrorCode::ControlRevoked);
        };
        if session.grant.workspace(&input.workspace_id).is_none()
            || !state
                .projection
                .workspaces
                .iter()
                .any(|w| w.id == input.workspace_id && session.grant.permits(w))
        {
            return error(ErrorCode::TargetNotFound);
        }
        let view = format!("panels:{}", input.workspace_id);
        let offset = if let Some(cursor) = input.cursor {
            let Some(cursor) = session
                .cursors
                .get(&cursor)
                .filter(|c| c.view == view && c.revision == state.projection.revision)
            else {
                return error(ErrorCode::CursorExpired);
            };
            cursor.offset
        } else {
            0
        };
        let all: Vec<_> = state
            .projection
            .panels
            .iter()
            .filter(|p| p.workspace_id == input.workspace_id)
            .collect();
        let mut items = Vec::new();
        let mut bytes = 0;
        for panel in all.iter().skip(offset).take(input.limit as usize) {
            let owned = panel
                .terminal_session_id
                .as_ref()
                .and_then(|g| state.terminals.get(g));
            let mut runtime_state = if panel.terminal_session_id.is_some() {
                "unobserved"
            } else {
                "not_started"
            };
            let mut controlled = false;
            let browser = panel
                .browser_generation
                .as_ref()
                .and_then(|g| state.browsers.get(g));
            if let Some(browser) = browser {
                runtime_state = if browser.control.started() {
                    "ready"
                } else {
                    "starting"
                };
                controlled = browser.control.authorized();
            }
            if let Some(owned) = owned {
                if let Ok(control) = owned.control.lock() {
                    runtime_state = if control.exited {
                        "exited"
                    } else if owned.started {
                        "running"
                    } else {
                        "starting"
                    };
                    controlled = control.lease().is_some();
                }
            }
            let item = PanelView {
                id: panel.id.clone(),
                workspace_id: panel.workspace_id.clone(),
                tab_id: panel.tab_id.clone(),
                kind: panel.kind.clone(),
                title: panel.title.clone(),
                terminal_session_id: panel.terminal_session_id.clone(),
                browser_generation: panel.browser_generation.clone(),
                android_device_id: panel.android_device_id.clone().filter(|device| {
                    session.grant.scopes.contains("android.read")
                        && session.grant.android_devices.contains(device)
                }),
                ownership: match owned
                    .map(|t| &t.owner)
                    .or_else(|| browser.map(|b| &b.owner))
                {
                    Some(owner) if owner == id => "this_session",
                    Some(_) => "another_session",
                    None => "human_or_unassigned",
                }
                .into(),
                runtime_state: runtime_state.into(),
                input_controlled: controlled,
            };
            let length = serde_json::to_vec(&item)
                .map(|v| v.len())
                .unwrap_or(MAX_METADATA_BYTES);
            if bytes + length > 48 * 1024 {
                break;
            }
            bytes += length;
            items.push(item);
        }
        let next = offset + items.len();
        let more = next < all.len();
        let revision = state.projection.revision.clone();
        let next_cursor = if more {
            let Ok(cursor) = new_id() else {
                return error(ErrorCode::ResourceExhausted);
            };
            let session = state
                .sessions
                .get_mut(id)
                .filter(|s| s.alive.load(Ordering::SeqCst))
                .unwrap();
            if session.cursors.len() >= 64 {
                session.cursors.clear();
            }
            session.cursors.insert(
                cursor.clone(),
                Cursor {
                    view,
                    revision: revision.clone(),
                    offset: next,
                },
            );
            Some(cursor)
        } else {
            None
        };
        Reply::ok(Data::Panels {
            items,
            next_cursor,
            domain_revision: revision,
        })
    }
}

pub struct HandshakeIo<T> {
    pub inner: T,
    pub remaining: usize,
}
impl<T: AsyncRead + Unpin> AsyncRead for HandshakeIo<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.remaining == 0 {
            return Poll::Ready(Err(failure()));
        }
        let mut bytes = [0; 8192];
        let limit = bytes.len().min(self.remaining).min(buf.remaining());
        let mut bounded = ReadBuf::new(&mut bytes[..limit]);
        match Pin::new(&mut self.inner).poll_read(cx, &mut bounded) {
            Poll::Ready(Ok(())) => {
                self.remaining -= bounded.filled().len();
                buf.put_slice(bounded.filled());
                Poll::Ready(Ok(()))
            }
            value => value,
        }
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for HandshakeIo<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod revoke_tests;
