#[cfg(unix)]
use lomi_remote_protocol::ipc::LocalFileAcknowledgement;
use lomi_remote_protocol::ipc::{
    LocalAuthClientId, LocalAuthConfiguration, LocalAuthOrigin, LocalAuthProvider, LocalCall,
    LocalError, LocalLoginPresentation, LocalLoginStart, LocalLoginStatus, LocalNativeGitExecute,
    LocalNativeGitInput, LocalNativeGitPrepare, LocalNativeGitResult, LocalNativeGitState,
    LocalNativeResultJson, LocalOperatorIntent, LocalProcessArgument, LocalRelayEndpoint,
    LocalResult, LocalRootPath, LocalSharing, LocalShellIntegration, LocalShellKind,
    LocalShellProfileRegister, LocalStreamAck, LocalWorkspaceImport, LocalWorkspaceRead,
    LocalWorkspaceSummary, LocalWorkspaceTarget, NativeEditorDisposition, NativeIntentId,
    NativeIntentTarget, WorkspaceBinding as HostWorkspaceBinding, WorkspaceBindingsRead,
};
use lomi_remote_protocol::{
    rpc::{BoundedList, Confirmed},
    FileWatchFrame, HostKeyCommitmentHex, Method, OperationId, RpcCall, RpcRequest, RpcResponse,
    RpcResult, TerminalStreamFrame, TimeoutMs, WorkspaceStreamFrame,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs::{self, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{Manager, State, Webview, Window};

#[cfg(unix)]
type HostClient = lomi_platform::unix::LocalClient;
#[cfg(windows)]
type HostClient = lomi_platform::windows::LocalClient;

#[derive(Clone, Default)]
pub struct SharedHost {
    clients: Arc<Mutex<HashMap<String, Arc<Mutex<HostClient>>>>>,
    streams: Arc<Mutex<HashMap<(String, String), SharedHostStream>>>,
    pumps: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    bindings_lock: Arc<Mutex<()>>,
    native_git_journal_lock: Arc<Mutex<()>>,
    native_git_recovery_lock: Arc<Mutex<()>>,
    native_git_active: Arc<Mutex<BTreeSet<NativeIntentId>>>,
    native_git_exchanges: Arc<Mutex<BTreeSet<NativeIntentId>>>,
    native_git_process_lease: Arc<Mutex<Option<NativeGitProcessLease>>>,
}

const NATIVE_GIT_OUTBOX_LIMIT: u64 = 8 * 1024 * 1024;
const NATIVE_GIT_OUTBOX_ENTRIES: usize = 64;
const NATIVE_GIT_OWNER_LOCK_LIMIT: usize = 132;
const NATIVE_GIT_OUTBOX_RETENTION_MS: u64 = 24 * 60 * 60 * 1000;
const NATIVE_GIT_RECOVERY_BATCH: usize = 4;
const NATIVE_GIT_RECOVERY_RETRY_MS: u64 = 1_000;
const WORKSPACE_BINDING_APPLY_LEDGER_LIMIT: usize = 2_048;

#[derive(Clone, Copy, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum NativeGitOutboxPhase {
    Preparing,
    Prepared,
    ExecuteSent,
    Terminal,
    RetiredNoHost,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeGitOutboxEntry {
    operation_id: NativeIntentId,
    request_hash: HostKeyCommitmentHex,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_id: Option<String>,
    phase: NativeGitOutboxPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plan_hash: Option<HostKeyCommitmentHex>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_result: Option<NativeGitTerminalReceipt>,
    #[serde(default)]
    acknowledgement_pending: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    acknowledged_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_recovery_attempt_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    retired_at_ms: Option<u64>,
    expires_at_ms: u64,
}

struct NativeGitProcessLease {
    owner_id: String,
    path: PathBuf,
    file: fs::File,
}

impl Drop for NativeGitProcessLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

struct NativeGitRecoveryOwnerLease(fs::File);

impl Drop for NativeGitRecoveryOwnerLease {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

struct NativeGitExchangeGuard<'a> {
    host: &'a SharedHost,
    operation_id: NativeIntentId,
}

impl Drop for NativeGitExchangeGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut exchanges) = self.host.native_git_exchanges.lock() {
            exchanges.remove(&self.operation_id);
        }
    }
}

struct NativeGitJournalLease(fs::File);

impl Drop for NativeGitJournalLease {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

struct WorkspaceBindingsFileLease(fs::File);

impl Drop for WorkspaceBindingsFileLease {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[derive(Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeGitTerminalReceipt {
    operation_id: NativeIntentId,
    request_hash: HostKeyCommitmentHex,
    state: LocalNativeGitState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plan_hash: Option<HostKeyCommitmentHex>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result_json: Option<LocalNativeResultJson>,
    custody_settled: bool,
}

#[derive(Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeGitOutbox {
    version: u32,
    operations: BTreeMap<String, NativeGitOutboxEntry>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NativeGitRecoverySummary {
    pub(crate) recovered: usize,
    pub(crate) unresolved: usize,
    pub(crate) retained: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SharedHostStreamKind {
    Terminal,
    Workspace,
    Files,
}

#[derive(Clone)]
struct SharedHostStream {
    kind: SharedHostStreamKind,
    channel: tauri::ipc::Channel<SharedHostStreamEvent>,
    client: Arc<Mutex<HostClient>>,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopTerminalAlias {
    desktop_pane_id: String,
    desktop_tab_id: String,
    host_pane_id: String,
    host_tab_id: String,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkspaceBinding {
    root_path: String,
    #[serde(default)]
    generation: u64,
    #[serde(default)]
    canonical_workspace_id: Option<String>,
    aliases: Vec<DesktopTerminalAlias>,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DesktopWorkspaceBindingSnapshot {
    pub(crate) workspace_id: String,
    pub(crate) root_path: String,
    pub(crate) generation: u64,
    pub(crate) canonical_workspace_id: String,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkspaceBindingApplyRecord {
    digest: String,
    applied_at_ms: u64,
}

#[derive(Clone, serde::Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum DesktopWorkspaceHostBinding {
    Missing {
        workspace_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        previous_root_path: Option<String>,
    },
    Active {
        workspace_id: String,
        canonical_workspace_id: String,
        root_path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        previous_root_path: Option<String>,
    },
    Pending {
        workspace_id: String,
        canonical_workspace_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        previous_root_path: Option<String>,
    },
    Retired {
        workspace_id: String,
        canonical_workspace_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        previous_root_path: Option<String>,
    },
}

pub(crate) struct WorkspaceBindingReconcileResult {
    pub(crate) host_id: String,
    pub(crate) runtime_id: String,
    pub(crate) bindings: Vec<DesktopWorkspaceHostBinding>,
}

pub(crate) struct NativeEditorDispositionSnapshot {
    pub(crate) host_id: String,
    pub(crate) runtime_id: String,
    pub(crate) disposition: NativeEditorDisposition,
}

pub(crate) enum NativeEditorDispositionRead {
    Present(NativeEditorDispositionSnapshot),
    Missing { host_id: String, runtime_id: String },
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DesktopWorkspaceBindingsResponse {
    bindings: Option<Vec<DesktopWorkspaceHostBinding>>,
    binding_error: Option<String>,
    uncertainty_warnings: Vec<crate::desktop_file_effects::DesktopUncertaintyWarning>,
}

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DesktopSharedHostState {
    Ready,
    AdministratorInstallRequired,
    Unavailable,
    UpdateRequired,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopSharedHostStatus {
    pub state: DesktopSharedHostState,
    pub host_id: Option<lomi_remote_protocol::HostId>,
    pub runtime_id: Option<lomi_remote_protocol::RuntimeId>,
    pub sharing: Option<LocalSharing>,
    pub message: Option<String>,
}

const MACOS_ADMIN_INSTALL_REQUIRED: &str = "MACOS_ADMIN_INSTALL_REQUIRED";
const MACOS_HOST_INSTALL_MESSAGE: &str =
    "Install the signed Lomi Remote administrator Host for this macOS account.";
const MACOS_HOST_RESTART_REQUIRED: &str = "The previous local Host must be stopped before this app can reconnect. Use the signed, administrator-installed Lomi Remote CLI to run `daemon stop --confirm`, then restart this app. This ends Host terminal sessions; private editor drafts remain on this device.";

#[cfg(target_os = "macos")]
pub(crate) fn connect_or_start_local_host(profile: &Path) -> Result<HostClient, String> {
    lomi_platform::service_macos::custody::prepare_current_user_host_profile(profile)
        .map_err(|_| MACOS_ADMIN_INSTALL_REQUIRED.to_owned())?;
    let executable =
        lomi_platform::service_macos::custody::installed_privileged_host_executable(profile)
            .map_err(|_| MACOS_ADMIN_INSTALL_REQUIRED.to_owned());

    match lomi_platform::unix::LocalClient::connect(profile) {
        Ok(client) => {
            let executable = executable?;
            client
                .verify_macos_host_executable(profile, &executable)
                .map_err(|_| MACOS_HOST_RESTART_REQUIRED.to_owned())?;
            return Ok(client);
        }
        Err(lomi_platform::unix::Error::Incompatible) => return Err("UPDATE_REQUIRED".into()),
        Err(lomi_platform::unix::Error::PermissionDenied) => {
            return Err("LOCAL_PERMISSION_DENIED".into());
        }
        Err(lomi_platform::unix::Error::InvalidProfile) => {
            return Err("INVALID_PROFILE".into());
        }
        Err(lomi_platform::unix::Error::InvalidEndpoint) => {
            return Err("INVALID_ENDPOINT".into());
        }
        Err(lomi_platform::unix::Error::InvalidFrame) => {
            return Err("INVALID_LOCAL_FRAME".into());
        }
        Err(lomi_platform::unix::Error::Busy) => return Err("HOST_BUSY".into()),
        Err(lomi_platform::unix::Error::Deadline) => return Err("LOCAL_DEADLINE".into()),
        Err(lomi_platform::unix::Error::Io(error))
            if error.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            return Err("LOCAL_PERMISSION_DENIED".into());
        }
        Err(lomi_platform::unix::Error::Io(_)) => {}
    }

    let executable = executable?;
    lomi_host::start(&executable, profile).map_err(|error| error.to_string())?;
    let client =
        lomi_platform::unix::LocalClient::connect(profile).map_err(|error| error.to_string())?;
    client
        .verify_macos_host_executable(profile, &executable)
        .map_err(|_| MACOS_HOST_RESTART_REQUIRED.to_owned())?;
    Ok(client)
}

fn workspace_binding_import_id(
    binding: &HostWorkspaceBinding,
) -> lomi_remote_protocol::WorkspaceId {
    match binding {
        HostWorkspaceBinding::Missing { import_id }
        | HostWorkspaceBinding::Active { import_id, .. }
        | HostWorkspaceBinding::Pending { import_id, .. }
        | HostWorkspaceBinding::Retired { import_id, .. } => *import_id,
    }
}

pub(crate) struct NativeGitBinding {
    pub(crate) root_path: std::path::PathBuf,
    pub(crate) workspace: lomi_remote_protocol::rpc::WorkspaceState,
}

pub(crate) struct NativeGitHostResponse {
    pub(crate) request_hash: String,
    pub(crate) result: Result<LocalNativeGitResult, String>,
}

pub(crate) struct NativeGitBridge<'a> {
    host: &'a SharedHost,
    app: &'a tauri::AppHandle,
    created: BTreeSet<NativeIntentId>,
    retained: BTreeSet<NativeIntentId>,
}

impl Drop for NativeGitBridge<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.host.native_git_active.lock() {
            for operation_id in &self.created {
                if !self.retained.contains(operation_id) {
                    active.remove(operation_id);
                }
            }
        }
    }
}

impl lomi_control_core::broker::GitMutationHost for NativeGitBridge<'_> {
    fn prepare_guarded(
        &mut self,
        request: lomi_control_core::broker::GitMutationHostPrepare,
        check: &mut dyn FnMut() -> bool,
    ) -> Result<lomi_control_core::broker::GitMutationHostPrepared, lomi_control_protocol::ErrorCode>
    {
        let _exchange = self
            .host
            .native_git_exchange(request.operation_id)
            .map_err(|_| lomi_control_protocol::ErrorCode::OutcomeUnknown)?;
        let response = self
            .host
            .native_git_prepare_guarded(
                self.app,
                &request.workspace_id,
                &request.workspace_root,
                request.operation_id.into(),
                request.request_json,
                request.timeout_ms,
                check,
            )
            .map_err(|_| lomi_control_protocol::ErrorCode::TargetNotFound)?;
        self.created.insert(request.operation_id);
        Ok(lomi_control_core::broker::GitMutationHostPrepared {
            request_hash: response.request_hash,
            response: response
                .result
                .map_err(|_| lomi_control_protocol::ErrorCode::OutcomeUnknown),
        })
    }

    fn execute_guarded(
        &mut self,
        operation_id: NativeIntentId,
        request_hash: &str,
        plan_hash: &str,
        timeout_ms: u64,
        check: &mut dyn FnMut() -> bool,
    ) -> Result<LocalNativeGitResult, lomi_control_protocol::ErrorCode> {
        let _exchange = self
            .host
            .native_git_exchange(operation_id)
            .map_err(|_| lomi_control_protocol::ErrorCode::OutcomeUnknown)?;
        self.host
            .native_git_execute_guarded(
                self.app,
                operation_id.into(),
                request_hash.to_owned(),
                plan_hash.to_owned(),
                timeout_ms,
                check,
            )
            .map_err(|_| lomi_control_protocol::ErrorCode::OutcomeUnknown)
    }

    fn status(
        &mut self,
        operation_id: NativeIntentId,
    ) -> Result<LocalNativeGitResult, lomi_control_protocol::ErrorCode> {
        // Status is read-only and must remain available while Execute is in
        // flight. The local IPC client serializes requests on its connection.
        self.host
            .native_git_status(self.app, operation_id.into())
            .map_err(|_| lomi_control_protocol::ErrorCode::OutcomeUnknown)
    }

    fn cancel_prepared(
        &mut self,
        operation_id: NativeIntentId,
    ) -> Result<LocalNativeGitResult, lomi_control_protocol::ErrorCode> {
        let _exchange = self
            .host
            .native_git_exchange(operation_id)
            .map_err(|_| lomi_control_protocol::ErrorCode::OutcomeUnknown)?;
        self.host
            .native_git_cancel_prepared(self.app, operation_id.into())
            .map_err(|_| lomi_control_protocol::ErrorCode::OutcomeUnknown)
    }

    fn acknowledge(
        &mut self,
        operation_id: NativeIntentId,
    ) -> Result<LocalNativeGitResult, lomi_control_protocol::ErrorCode> {
        let _exchange = self
            .host
            .native_git_exchange(operation_id)
            .map_err(|_| lomi_control_protocol::ErrorCode::OutcomeUnknown)?;
        self.host
            .native_git_acknowledge(self.app, operation_id.into())
            .map_err(|_| lomi_control_protocol::ErrorCode::OutcomeUnknown)
    }

    fn retain_prepared_for_approval(&mut self, operation_id: NativeIntentId) {
        self.retained.insert(operation_id);
        self.host.mark_native_git_active(operation_id);
    }
}

#[derive(Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkspaceBindings {
    version: u32,
    workspaces: BTreeMap<String, WorkspaceBinding>,
    #[serde(default)]
    next_generation: u64,
    #[serde(default)]
    apply_ledger: BTreeMap<String, WorkspaceBindingApplyRecord>,
}

#[derive(serde::Serialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum SharedHostStreamEvent {
    Frame(lomi_remote_protocol::ipc::LocalStreamEvent),
    TransportLost,
}

fn hash_local_native_git_input(input: &LocalNativeGitInput) -> Result<String, String> {
    let bytes = serde_json::to_vec(input)
        .map_err(|_| "The native Git request could not be fingerprinted.")?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn native_git_app_data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    crate::chat::storage::reject_link(&directory)?;
    crate::chat::storage::private(&directory, true)?;
    fs::canonicalize(&directory).map_err(|error| error.to_string())
}

fn native_git_outbox_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(native_git_app_data_dir(app)?.join("native-git-outbox.json"))
}

fn native_git_path_overlaps_control_root(path: &Path, control_root: &Path) -> bool {
    path.starts_with(control_root) || control_root.starts_with(path)
}

fn native_git_process_owner_id() -> &'static str {
    static OWNER_ID: OnceLock<String> = OnceLock::new();
    OWNER_ID
        .get_or_init(|| {
            let started_at_ns = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            format!("{}-{started_at_ns}", std::process::id())
        })
        .as_str()
}

fn valid_native_git_owner_id(owner_id: &str) -> bool {
    let Some((pid, started_at_ns)) = owner_id.split_once('-') else {
        return false;
    };
    !owner_id.is_empty()
        && owner_id.len() <= 64
        && pid.parse::<u32>().is_ok_and(|pid| pid != 0)
        && started_at_ns
            .parse::<u128>()
            .is_ok_and(|started| started != 0)
}

fn native_git_owner_lock_path(outbox_path: &Path, owner_id: &str) -> Option<PathBuf> {
    if !valid_native_git_owner_id(owner_id) {
        return None;
    }
    Some(outbox_path.with_file_name(format!("native-git-owner-{owner_id}.lock")))
}

fn open_owner_private_lock_file(
    path: &Path,
    create: bool,
    label: &str,
) -> Result<fs::File, String> {
    let before = match fs::symlink_metadata(path) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !create => {
            return Err(format!("The {label} owner lock is unavailable."));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    if before
        .as_ref()
        .is_some_and(|metadata| !metadata.is_file() || metadata.file_type().is_symlink())
    {
        return Err(format!("The {label} lock is not a regular file."));
    }

    let mut options = OpenOptions::new();
    options.read(true).write(true);
    if create {
        options.create(true).truncate(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path).map_err(|error| error.to_string())?;
    let opened = file.metadata().map_err(|error| error.to_string())?;
    let current = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !opened.is_file()
        || opened.file_type().is_symlink()
        || !current.is_file()
        || current.file_type().is_symlink()
    {
        return Err(format!("The {label} lock is not a regular file."));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.dev() != current.dev()
            || opened.ino() != current.ino()
            || opened.uid() != unsafe { libc::geteuid() }
            || opened.nlink() != 1
            || opened.mode() & 0o077 != 0
        {
            return Err(format!("The {label} lock is not owner-private."));
        }
        if let Some(before) = &before {
            if opened.dev() != before.dev() || opened.ino() != before.ino() {
                return Err(format!("The {label} lock changed while it was opened."));
            }
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if opened.file_attributes() & 0x400 != 0 || current.file_attributes() & 0x400 != 0 {
            return Err(format!("The {label} lock is not a regular file."));
        }
    }
    crate::chat::storage::private(path, false)
        .map_err(|_| format!("The {label} lock is not owner-private."))?;
    Ok(file)
}

fn open_native_git_lock_file(path: &Path, create: bool) -> Result<fs::File, String> {
    open_owner_private_lock_file(path, create, "NativeGit")
}

fn native_git_journal_file_lock(outbox_path: &Path) -> Result<NativeGitJournalLease, String> {
    let lock_path = outbox_path.with_file_name("native-git-outbox.lock");
    let file = open_native_git_lock_file(&lock_path, true)?;
    file.lock()
        .map_err(|_| "The NativeGit recovery journal could not be locked.")?;
    Ok(NativeGitJournalLease(file))
}

fn native_git_recovery_owner_lease(
    outbox_path: &Path,
    owner_id: &str,
) -> Result<Option<NativeGitRecoveryOwnerLease>, String> {
    let Some(path) = native_git_owner_lock_path(outbox_path, owner_id) else {
        return Ok(None);
    };
    let file = match open_native_git_lock_file(&path, false) {
        Ok(file) => file,
        Err(_) => return Ok(None),
    };
    match file.try_lock() {
        Ok(()) => Ok(Some(NativeGitRecoveryOwnerLease(file))),
        Err(fs::TryLockError::WouldBlock) => Ok(None),
        Err(fs::TryLockError::Error(_)) => Ok(None),
    }
}

fn cleanup_native_git_owner_locks(
    outbox_path: &Path,
    outbox: &NativeGitOutbox,
) -> Result<(), String> {
    let directory = outbox_path
        .parent()
        .ok_or("The NativeGit recovery journal has no parent directory.")?;
    let referenced: BTreeSet<_> = outbox
        .operations
        .values()
        .filter(|entry| entry.acknowledged_at_ms.is_none())
        .filter_map(|entry| entry.owner_id.as_deref())
        .map(str::to_owned)
        .collect();
    let mut scanned = 0usize;
    for item in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let item = item.map_err(|error| error.to_string())?;
        let name = item.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(owner_id) = name
            .strip_prefix("native-git-owner-")
            .and_then(|name| name.strip_suffix(".lock"))
        else {
            continue;
        };
        scanned += 1;
        if scanned > NATIVE_GIT_OWNER_LOCK_LIMIT {
            return Err("NativeGit owner lock inventory exceeds its bounded limit.".into());
        }
        if referenced.contains(owner_id) || !valid_native_git_owner_id(owner_id) {
            continue;
        }
        let path = item.path();
        let Ok(file) = open_native_git_lock_file(&path, false) else {
            continue;
        };
        if file.try_lock().is_ok() {
            drop(file);
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}

fn prune_native_git_outbox_for_capacity(outbox: &mut NativeGitOutbox) -> Result<(), String> {
    let unresolved = outbox
        .operations
        .values()
        .filter(|entry| {
            entry.phase != NativeGitOutboxPhase::RetiredNoHost
                && (entry.acknowledged_at_ms.is_none()
                    || entry
                        .terminal_result
                        .as_ref()
                        .is_some_and(|result| result.state == LocalNativeGitState::OutcomeUnknown))
        })
        .count();
    if unresolved >= NATIVE_GIT_OUTBOX_ENTRIES {
        return Err("NativeGit recovery obligations have reached their bounded limit.".into());
    }

    while outbox.operations.len() >= NATIVE_GIT_OUTBOX_ENTRIES {
        let evictable = outbox
            .operations
            .iter()
            .filter_map(|(key, entry)| {
                if entry.phase == NativeGitOutboxPhase::RetiredNoHost {
                    return entry
                        .retired_at_ms
                        .map(|retired_at| (key.clone(), retired_at));
                }
                let settled_success_or_failure =
                    entry.terminal_result.as_ref().is_some_and(|result| {
                        result.custody_settled
                            && matches!(
                                result.state,
                                LocalNativeGitState::Succeeded | LocalNativeGitState::Failed
                            )
                    });
                (entry.acknowledgement_pending
                    && entry.acknowledged_at_ms.is_some()
                    && settled_success_or_failure)
                    .then(|| (key.clone(), entry.acknowledged_at_ms.unwrap_or_default()))
            })
            .min_by_key(|(_, acknowledged_at_ms)| *acknowledged_at_ms)
            .map(|(key, _)| key);
        let Some(key) = evictable else {
            return Err("NativeGit recovery obligations have reached their bounded limit.".into());
        };
        outbox.operations.remove(&key);
    }
    Ok(())
}

fn native_git_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn native_git_entry_retention_expired(entry: &NativeGitOutboxEntry, now: u64) -> bool {
    (entry.phase == NativeGitOutboxPhase::RetiredNoHost
        && entry
            .retired_at_ms
            .is_some_and(|retired| now.saturating_sub(retired) >= NATIVE_GIT_OUTBOX_RETENTION_MS))
        || (entry.acknowledged_at_ms.is_some_and(|acked| {
            now.saturating_sub(acked) >= NATIVE_GIT_OUTBOX_RETENTION_MS
                && entry
                    .terminal_result
                    .as_ref()
                    .is_some_and(|result| result.state != LocalNativeGitState::OutcomeUnknown)
        }))
}

fn native_git_terminal(state: LocalNativeGitState) -> bool {
    matches!(
        state,
        LocalNativeGitState::Succeeded
            | LocalNativeGitState::Failed
            | LocalNativeGitState::OutcomeUnknown
    )
}

fn valid_native_git_outbox_result(
    result: &NativeGitTerminalReceipt,
    entry: &NativeGitOutboxEntry,
) -> bool {
    result.operation_id == entry.operation_id
        && result.request_hash == entry.request_hash
        && result.plan_hash == entry.plan_hash
}

fn read_native_git_outbox(path: &Path) -> Result<NativeGitOutbox, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(NativeGitOutbox {
                version: 1,
                operations: BTreeMap::new(),
            });
        }
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > NATIVE_GIT_OUTBOX_LIMIT
    {
        return Err("The native Git recovery journal is unavailable or invalid.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.mode() & 0o077 != 0
        {
            return Err("The native Git recovery journal is not owner-private.".into());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("The native Git recovery journal is not a regular file.".into());
        }
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = options.open(path).map_err(|error| error.to_string())?;
    let opened = file.metadata().map_err(|error| error.to_string())?;
    if !opened.is_file() || opened.file_type().is_symlink() {
        return Err("The native Git recovery journal is not a regular file.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.dev() != metadata.dev()
            || opened.ino() != metadata.ino()
            || opened.uid() != unsafe { libc::geteuid() }
            || opened.nlink() != 1
            || opened.mode() & 0o077 != 0
        {
            return Err("The native Git recovery journal changed while it was opened.".into());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if opened.file_attributes() & 0x400 != 0 {
            return Err("The native Git recovery journal is not a regular file.".into());
        }
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.by_ref()
        .take(NATIVE_GIT_OUTBOX_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > NATIVE_GIT_OUTBOX_LIMIT {
        return Err("The native Git recovery journal is too large.".into());
    }
    let mut outbox: NativeGitOutbox = serde_json::from_slice(&bytes)
        .map_err(|_| "The native Git recovery journal is invalid.")?;
    if outbox.version != 1 || outbox.operations.len() > NATIVE_GIT_OUTBOX_ENTRIES {
        return Err("The native Git recovery journal has an unsupported version or size.".into());
    }
    for (key, entry) in &outbox.operations {
        if String::from(entry.operation_id) != *key
            || entry.expires_at_ms == 0
            || entry
                .owner_id
                .as_deref()
                .is_some_and(|owner_id| !valid_native_git_owner_id(owner_id))
            || entry.acknowledged_at_ms.is_some() && !entry.acknowledgement_pending
            || entry.acknowledgement_pending
                && !entry.terminal_result.as_ref().is_some_and(|result| {
                    native_git_terminal(result.state) && result.custody_settled
                })
            || entry.phase == NativeGitOutboxPhase::RetiredNoHost
                && (entry.retired_at_ms.is_none()
                    || entry.owner_id.is_some()
                    || entry.plan_hash.is_some()
                    || entry.terminal_result.is_some()
                    || entry.acknowledgement_pending
                    || entry.acknowledged_at_ms.is_some())
            || entry.phase != NativeGitOutboxPhase::RetiredNoHost && entry.retired_at_ms.is_some()
        {
            return Err("The native Git recovery journal contains an invalid operation.".into());
        }
        if let Some(result) = &entry.terminal_result {
            if !native_git_terminal(result.state)
                || !valid_native_git_outbox_result(result, entry)
                || entry.phase != NativeGitOutboxPhase::Terminal
            {
                return Err("The native Git recovery journal contains an invalid receipt.".into());
            }
        }
    }
    outbox.version = 1;
    Ok(outbox)
}

fn write_native_git_outbox(path: &Path, outbox: &NativeGitOutbox) -> Result<(), String> {
    crate::files::write_json(path, outbox, NATIVE_GIT_OUTBOX_LIMIT as usize)
}

fn validate_native_git_result(
    result: &LocalNativeGitResult,
    operation_id: NativeIntentId,
    request_hash: &HostKeyCommitmentHex,
    plan_hash: Option<&HostKeyCommitmentHex>,
) -> bool {
    result.operation_id == operation_id
        && &result.request_hash == request_hash
        && plan_hash.is_none_or(|expected| result.plan_hash.as_ref() == Some(expected))
}

impl SharedHost {
    fn native_git_exchange(
        &self,
        operation_id: NativeIntentId,
    ) -> Result<NativeGitExchangeGuard<'_>, String> {
        let mut exchanges = self
            .native_git_exchanges
            .lock()
            .map_err(|_| "NativeGit exchange state is unavailable.")?;
        if !exchanges.insert(operation_id) {
            return Err("A NativeGit exchange is already active for this operation.".into());
        }
        Ok(NativeGitExchangeGuard {
            host: self,
            operation_id,
        })
    }

    pub(crate) fn native_git_bridge<'a>(
        &'a self,
        app: &'a tauri::AppHandle,
    ) -> Result<NativeGitBridge<'a>, String> {
        self.recover_native_git_outbox(app)?;
        Ok(NativeGitBridge {
            host: self,
            app,
            created: BTreeSet::new(),
            retained: BTreeSet::new(),
        })
    }

    pub(crate) fn recover_native_git_outbox(
        &self,
        app: &tauri::AppHandle,
    ) -> Result<NativeGitRecoverySummary, String> {
        let _recovery = self
            .native_git_recovery_lock
            .lock()
            .map_err(|_| "Native Git recovery is unavailable.")?;
        let path = native_git_outbox_path(app)?;
        let now = native_git_now_ms();
        let outbox = {
            let _journal = self
                .native_git_journal_lock
                .lock()
                .map_err(|_| "Native Git recovery journal is unavailable.")?;
            let _file_lock = native_git_journal_file_lock(&path)?;
            let mut outbox = read_native_git_outbox(&path)?;
            let mut pruned = false;
            outbox.operations.retain(|_, entry| {
                let removable = native_git_entry_retention_expired(entry, now);
                pruned |= removable;
                !removable
            });
            if pruned {
                write_native_git_outbox(&path, &outbox)?;
            }
            cleanup_native_git_owner_locks(&path, &outbox)?;
            outbox
        };
        if outbox.operations.is_empty() {
            return Ok(NativeGitRecoverySummary::default());
        }

        let active = self
            .native_git_active
            .lock()
            .map_err(|_| "Native Git recovery is unavailable.")?
            .clone();
        let mut summary = NativeGitRecoverySummary::default();
        let mut eligible: Vec<_> = outbox
            .operations
            .values()
            .filter(|entry| {
                entry.acknowledged_at_ms.is_none()
                    && entry.phase != NativeGitOutboxPhase::RetiredNoHost
                    && !(entry.expires_at_ms > now && active.contains(&entry.operation_id))
                    && entry
                        .last_recovery_attempt_ms
                        .is_none_or(|last| now.saturating_sub(last) >= NATIVE_GIT_RECOVERY_RETRY_MS)
            })
            .cloned()
            .collect();
        eligible.sort_by(|left, right| {
            left.last_recovery_attempt_ms
                .cmp(&right.last_recovery_attempt_ms)
                .then_with(|| {
                    String::from(left.operation_id).cmp(&String::from(right.operation_id))
                })
        });
        let operations: Vec<_> = eligible
            .into_iter()
            .take(NATIVE_GIT_RECOVERY_BATCH)
            .collect();
        summary.retained = outbox.operations.len();
        for entry in operations {
            self.mark_native_git_recovery_attempt(app, entry.operation_id, now)?;
            let Some(owner_id) = entry.owner_id.as_deref() else {
                summary.unresolved += 1;
                continue;
            };
            let own_process = owner_id == native_git_process_owner_id();
            if own_process && entry.expires_at_ms > now && active.contains(&entry.operation_id) {
                summary.unresolved += 1;
                continue;
            }
            let _owner_lease = if own_process {
                None
            } else {
                match native_git_recovery_owner_lease(&path, owner_id)? {
                    Some(lease) => Some(lease),
                    None => {
                        summary.unresolved += 1;
                        continue;
                    }
                }
            };
            let _exchange = match self.native_git_exchange(entry.operation_id) {
                Ok(exchange) => exchange,
                Err(_) => {
                    summary.unresolved += 1;
                    continue;
                }
            };
            match self.native_git_status_optional(entry.operation_id) {
                Ok(None) => {
                    if entry.acknowledgement_pending
                        && entry.terminal_result.as_ref().is_some_and(|result| {
                            result.custody_settled && native_git_terminal(result.state)
                        })
                    {
                        self.mark_native_git_acknowledged(app, entry.operation_id)?;
                        summary.recovered += 1;
                    } else if entry.phase == NativeGitOutboxPhase::Preparing
                        && entry.plan_hash.is_none()
                        && entry.expires_at_ms <= now
                    {
                        self.retire_native_git_no_host(
                            app,
                            entry.operation_id,
                            &entry.request_hash,
                            now,
                        )?;
                        summary.recovered += 1;
                    } else {
                        summary.unresolved += 1;
                    }
                }
                Err(_) => summary.unresolved += 1,
                Ok(Some(status)) => {
                    if !validate_native_git_result(
                        &status,
                        entry.operation_id,
                        &entry.request_hash,
                        entry.plan_hash.as_ref(),
                    ) {
                        summary.unresolved += 1;
                        continue;
                    }
                    match status.state {
                        LocalNativeGitState::Preparing | LocalNativeGitState::Prepared => {
                            match self.native_git_cancel_prepared(app, entry.operation_id.into()) {
                                Ok(cancelled)
                                    if validate_native_git_result(
                                        &cancelled,
                                        entry.operation_id,
                                        &entry.request_hash,
                                        entry.plan_hash.as_ref(),
                                    ) && cancelled.state == LocalNativeGitState::Failed =>
                                {
                                    self.record_native_git_result(app, &cancelled)?;
                                    if cancelled.custody_settled
                                        && self
                                            .native_git_acknowledge(app, entry.operation_id.into())
                                            .is_ok()
                                    {
                                        summary.recovered += 1;
                                    } else {
                                        summary.unresolved += 1;
                                    }
                                }
                                _ => summary.unresolved += 1,
                            }
                        }
                        LocalNativeGitState::Running => summary.unresolved += 1,
                        LocalNativeGitState::Succeeded
                        | LocalNativeGitState::Failed
                        | LocalNativeGitState::OutcomeUnknown => {
                            if self.record_native_git_result(app, &status).is_err() {
                                summary.unresolved += 1;
                                continue;
                            }
                            if status.custody_settled {
                                if self
                                    .native_git_acknowledge(app, entry.operation_id.into())
                                    .is_ok()
                                {
                                    summary.recovered += 1;
                                } else {
                                    summary.unresolved += 1;
                                }
                            } else {
                                summary.unresolved += 1;
                            }
                        }
                    }
                }
            }
        }
        summary.retained = self.native_git_outbox_len(app).unwrap_or(summary.retained);
        Ok(summary)
    }

    fn native_git_outbox_len(&self, app: &tauri::AppHandle) -> Result<usize, String> {
        let path = native_git_outbox_path(app)?;
        let _journal = self
            .native_git_journal_lock
            .lock()
            .map_err(|_| "Native Git recovery journal is unavailable.")?;
        let _file_lock = native_git_journal_file_lock(&path)?;
        Ok(read_native_git_outbox(&path)?.operations.len())
    }

    fn ensure_native_git_process_lease(&self, outbox_path: &Path) -> Result<String, String> {
        let owner_id = native_git_process_owner_id().to_owned();
        let Some(path) = native_git_owner_lock_path(outbox_path, &owner_id) else {
            return Err("The NativeGit process identity is invalid.".into());
        };
        let mut current = self
            .native_git_process_lease
            .lock()
            .map_err(|_| "NativeGit process ownership is unavailable.")?;
        if let Some(lease) = current.as_ref() {
            if lease.owner_id == owner_id && lease.path == path {
                return Ok(owner_id);
            }
            return Err("NativeGit process ownership changed unexpectedly.".into());
        }
        let file = open_native_git_lock_file(&path, true)?;
        file.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => {
                "Another Lomi process already owns this NativeGit identity.".to_owned()
            }
            fs::TryLockError::Error(_) => {
                "NativeGit process ownership could not be acquired.".to_owned()
            }
        })?;
        *current = Some(NativeGitProcessLease {
            owner_id: owner_id.clone(),
            path,
            file,
        });
        Ok(owner_id)
    }

    fn mark_native_git_active(&self, operation_id: NativeIntentId) {
        if let Ok(mut active) = self.native_git_active.lock() {
            active.insert(operation_id);
        }
    }

    fn mark_native_git_recovery_attempt(
        &self,
        app: &tauri::AppHandle,
        operation_id: NativeIntentId,
        timestamp_ms: u64,
    ) -> Result<(), String> {
        let path = native_git_outbox_path(app)?;
        let _journal = self
            .native_git_journal_lock
            .lock()
            .map_err(|_| "Native Git recovery journal is unavailable.")?;
        let _file_lock = native_git_journal_file_lock(&path)?;
        let mut outbox = read_native_git_outbox(&path)?;
        let entry = outbox
            .operations
            .get_mut(&String::from(operation_id))
            .ok_or("The NativeGit recovery entry disappeared.")?;
        entry.last_recovery_attempt_ms = Some(timestamp_ms);
        write_native_git_outbox(&path, &outbox)
    }

    fn retire_native_git_no_host(
        &self,
        app: &tauri::AppHandle,
        operation_id: NativeIntentId,
        request_hash: &HostKeyCommitmentHex,
        timestamp_ms: u64,
    ) -> Result<(), String> {
        let path = native_git_outbox_path(app)?;
        let _journal = self
            .native_git_journal_lock
            .lock()
            .map_err(|_| "Native Git recovery journal is unavailable.")?;
        let _file_lock = native_git_journal_file_lock(&path)?;
        let mut outbox = read_native_git_outbox(&path)?;
        let entry = outbox
            .operations
            .get_mut(&String::from(operation_id))
            .ok_or("The NativeGit recovery entry disappeared.")?;
        if entry.request_hash != *request_hash
            || entry.phase != NativeGitOutboxPhase::Preparing
            || entry.plan_hash.is_some()
            || entry.terminal_result.is_some()
            || entry.acknowledgement_pending
            || entry.acknowledged_at_ms.is_some()
            || entry.expires_at_ms > timestamp_ms
        {
            return Err("The NativeGit no-Host recovery proof changed unexpectedly.".into());
        }
        entry.phase = NativeGitOutboxPhase::RetiredNoHost;
        entry.owner_id = None;
        entry.retired_at_ms = Some(timestamp_ms);
        write_native_git_outbox(&path, &outbox)?;
        self.mark_native_git_inactive(operation_id);
        Ok(())
    }

    fn begin_native_git_outbox(
        &self,
        app: &tauri::AppHandle,
        operation_id: NativeIntentId,
        request_hash: HostKeyCommitmentHex,
        timeout_ms: u64,
    ) -> Result<(), String> {
        let path = native_git_outbox_path(app)?;
        let _recovery = self
            .native_git_recovery_lock
            .lock()
            .map_err(|_| "Native Git recovery is unavailable.")?;
        let _journal = self
            .native_git_journal_lock
            .lock()
            .map_err(|_| "Native Git recovery journal is unavailable.")?;
        let _file_lock = native_git_journal_file_lock(&path)?;
        let owner_id = self.ensure_native_git_process_lease(&path)?;
        let mut outbox = read_native_git_outbox(&path)?;
        let now = native_git_now_ms();
        outbox
            .operations
            .retain(|_, entry| !native_git_entry_retention_expired(entry, now));
        let key = String::from(operation_id);
        if outbox.operations.contains_key(&key) {
            return Err("This NativeGit operation already has a durable Host intent.".into());
        }
        prune_native_git_outbox_for_capacity(&mut outbox)?;
        cleanup_native_git_owner_locks(&path, &outbox)?;
        outbox.version = 1;
        outbox.operations.insert(
            key,
            NativeGitOutboxEntry {
                operation_id,
                request_hash,
                owner_id: Some(owner_id),
                phase: NativeGitOutboxPhase::Preparing,
                plan_hash: None,
                terminal_result: None,
                acknowledgement_pending: false,
                acknowledged_at_ms: None,
                last_recovery_attempt_ms: None,
                retired_at_ms: None,
                expires_at_ms: now.saturating_add(timeout_ms),
            },
        );
        write_native_git_outbox(&path, &outbox)?;
        self.mark_native_git_active(operation_id);
        Ok(())
    }

    fn mark_native_git_execute_sent(
        &self,
        app: &tauri::AppHandle,
        operation_id: NativeIntentId,
        request_hash: &HostKeyCommitmentHex,
        plan_hash: HostKeyCommitmentHex,
    ) -> Result<(), String> {
        let path = native_git_outbox_path(app)?;
        let _journal = self
            .native_git_journal_lock
            .lock()
            .map_err(|_| "Native Git recovery journal is unavailable.")?;
        let _file_lock = native_git_journal_file_lock(&path)?;
        let mut outbox = read_native_git_outbox(&path)?;
        let entry = outbox
            .operations
            .get_mut(&String::from(operation_id))
            .ok_or("The NativeGit operation is missing from its recovery journal.")?;
        if entry.request_hash != *request_hash
            || entry.phase != NativeGitOutboxPhase::Prepared
            || entry.terminal_result.is_some()
            || entry
                .plan_hash
                .as_ref()
                .is_some_and(|current| current != &plan_hash)
        {
            return Err("The NativeGit recovery journal does not match Execute.".into());
        }
        entry.plan_hash = Some(plan_hash);
        entry.phase = NativeGitOutboxPhase::ExecuteSent;
        write_native_git_outbox(&path, &outbox)
    }

    fn record_native_git_result(
        &self,
        app: &tauri::AppHandle,
        result: &LocalNativeGitResult,
    ) -> Result<(), String> {
        let path = native_git_outbox_path(app)?;
        let _journal = self
            .native_git_journal_lock
            .lock()
            .map_err(|_| "Native Git recovery journal is unavailable.")?;
        let _file_lock = native_git_journal_file_lock(&path)?;
        let mut outbox = read_native_git_outbox(&path)?;
        let entry = outbox
            .operations
            .get_mut(&String::from(result.operation_id))
            .ok_or("The NativeGit result has no durable outbox entry.")?;
        if entry.phase == NativeGitOutboxPhase::RetiredNoHost
            || result.request_hash != entry.request_hash
            || entry
                .plan_hash
                .as_ref()
                .is_some_and(|expected| result.plan_hash.as_ref() != Some(expected))
        {
            return Err("The NativeGit result does not match its opaque request hash.".into());
        }
        if let Some(plan_hash) = &result.plan_hash {
            entry.plan_hash.get_or_insert_with(|| plan_hash.clone());
        }
        if native_git_terminal(result.state) {
            let terminal = NativeGitTerminalReceipt {
                operation_id: result.operation_id,
                request_hash: result.request_hash.clone(),
                state: result.state,
                plan_hash: result.plan_hash.clone(),
                result_json: result.result_json.clone(),
                custody_settled: result.custody_settled,
            };
            if let Some(existing) = &entry.terminal_result {
                if existing.operation_id != terminal.operation_id
                    || existing.request_hash != terminal.request_hash
                    || existing.state != terminal.state
                    || existing.plan_hash != terminal.plan_hash
                    || existing.result_json != terminal.result_json
                    || existing.custody_settled && !terminal.custody_settled
                {
                    return Err("The NativeGit terminal receipt changed unexpectedly.".into());
                }
            }
            entry.phase = NativeGitOutboxPhase::Terminal;
            entry.terminal_result = Some(terminal);
        } else if result.state == LocalNativeGitState::Prepared {
            entry.phase = NativeGitOutboxPhase::Prepared;
        }
        write_native_git_outbox(&path, &outbox)
    }

    fn mark_native_git_ack_pending(
        &self,
        app: &tauri::AppHandle,
        operation_id: NativeIntentId,
    ) -> Result<(), String> {
        let path = native_git_outbox_path(app)?;
        let _journal = self
            .native_git_journal_lock
            .lock()
            .map_err(|_| "Native Git recovery journal is unavailable.")?;
        let _file_lock = native_git_journal_file_lock(&path)?;
        let mut outbox = read_native_git_outbox(&path)?;
        let entry = outbox
            .operations
            .get_mut(&String::from(operation_id))
            .ok_or("The NativeGit ACK has no durable operation entry.")?;
        if !entry
            .terminal_result
            .as_ref()
            .is_some_and(|result| native_git_terminal(result.state) && result.custody_settled)
        {
            return Err("The NativeGit result is not durably settled for ACK.".into());
        }
        entry.acknowledgement_pending = true;
        write_native_git_outbox(&path, &outbox)
    }

    fn mark_native_git_acknowledged(
        &self,
        app: &tauri::AppHandle,
        operation_id: NativeIntentId,
    ) -> Result<(), String> {
        let path = native_git_outbox_path(app)?;
        let _journal = self
            .native_git_journal_lock
            .lock()
            .map_err(|_| "Native Git recovery journal is unavailable.")?;
        let _file_lock = native_git_journal_file_lock(&path)?;
        let mut outbox = read_native_git_outbox(&path)?;
        let entry = outbox
            .operations
            .get_mut(&String::from(operation_id))
            .ok_or("The NativeGit ACK has no durable operation entry.")?;
        if !entry.acknowledgement_pending
            || !entry
                .terminal_result
                .as_ref()
                .is_some_and(|result| native_git_terminal(result.state) && result.custody_settled)
        {
            return Err("The NativeGit ACK has no matching settled result.".into());
        }
        entry
            .acknowledged_at_ms
            .get_or_insert_with(native_git_now_ms);
        entry.owner_id = None;
        write_native_git_outbox(&path, &outbox)?;
        self.mark_native_git_inactive(operation_id);
        Ok(())
    }

    fn mark_native_git_inactive(&self, operation_id: NativeIntentId) {
        if let Ok(mut active) = self.native_git_active.lock() {
            active.remove(&operation_id);
        }
    }

    fn bindings_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
        Ok(app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join("shared-host-bindings.json"))
    }

    fn bindings_file_lock(path: &Path) -> Result<WorkspaceBindingsFileLease, String> {
        let lock_path = path.with_file_name("shared-host-bindings.lock");
        let file = open_owner_private_lock_file(&lock_path, true, "workspace binding")?;
        file.lock()
            .map_err(|_| "The local workspace bindings could not be locked.")?;
        Ok(WorkspaceBindingsFileLease(file))
    }

    fn read_bindings(path: &Path) -> Result<WorkspaceBindings, String> {
        let mut file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(WorkspaceBindings {
                    version: 2,
                    workspaces: BTreeMap::new(),
                    next_generation: 1,
                    apply_ledger: BTreeMap::new(),
                });
            }
            Err(error) => return Err(error.to_string()),
        };
        let mut bytes = Vec::new();
        file.by_ref()
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() > 1024 * 1024 {
            return Err("The saved local workspace bindings exceed 1 MiB.".into());
        }
        let mut bindings: WorkspaceBindings = serde_json::from_slice(&bytes)
            .map_err(|_| "The saved local workspace bindings are invalid.")?;
        if !matches!(bindings.version, 1 | 2) || bindings.workspaces.len() > 256 {
            return Err("The saved local workspace bindings are unsupported.".into());
        }
        if bindings.version == 1 {
            let mut generation = 1u64;
            for binding in bindings.workspaces.values_mut() {
                binding.generation = generation;
                generation = generation
                    .checked_add(1)
                    .ok_or("The local workspace binding generation is exhausted.")?;
            }
            bindings.next_generation = generation;
            bindings.apply_ledger.clear();
            bindings.version = 2;
        } else if bindings.next_generation == 0
            || bindings.apply_ledger.len() > WORKSPACE_BINDING_APPLY_LEDGER_LIMIT
        {
            return Err("The saved local workspace binding state is invalid.".into());
        }
        let mut generations = HashSet::new();
        let max_generation = bindings
            .workspaces
            .values()
            .map(|binding| binding.generation)
            .max()
            .unwrap_or(0);
        if bindings.version == 2
            && (bindings
                .workspaces
                .values()
                .any(|binding| binding.generation == 0 || !generations.insert(binding.generation))
                || bindings.next_generation <= max_generation)
        {
            return Err("The saved local workspace binding generations are invalid.".into());
        }
        for (workspace_id, binding) in &bindings.workspaces {
            lomi_remote_protocol::WorkspaceId::try_from(workspace_id.clone())
                .map_err(|_| "The saved local workspace binding ID is invalid.")?;
            if let Some(canonical_workspace_id) = &binding.canonical_workspace_id {
                lomi_remote_protocol::WorkspaceId::try_from(canonical_workspace_id.clone())
                    .map_err(|_| "The saved canonical workspace binding ID is invalid.")?;
            }
            if binding.aliases.len() > 32 {
                return Err("The saved local terminal aliases exceed the workspace limit.".into());
            }
            for alias in &binding.aliases {
                lomi_remote_protocol::PaneId::try_from(alias.desktop_pane_id.clone())
                    .and_then(|_| {
                        lomi_remote_protocol::PaneId::try_from(alias.host_pane_id.clone())
                    })
                    .map_err(|_| "The saved local terminal alias is invalid.")?;
                lomi_remote_protocol::TabId::try_from(alias.desktop_tab_id.clone())
                    .and_then(|_| lomi_remote_protocol::TabId::try_from(alias.host_tab_id.clone()))
                    .map_err(|_| "The saved local terminal alias is invalid.")?;
            }
        }
        for (operation_id, record) in &bindings.apply_ledger {
            NativeIntentId::try_from(operation_id.clone())
                .map_err(|_| "The saved workspace binding apply ID is invalid.")?;
            if record.digest.len() != 64
                || !record.digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                || record.applied_at_ms == 0
            {
                return Err("The saved workspace binding apply ledger is invalid.".into());
            }
        }
        Ok(bindings)
    }

    fn write_bindings(path: &Path, bindings: &mut WorkspaceBindings) -> Result<(), String> {
        bindings.version = 2;
        crate::files::write_json(path, bindings, 1024 * 1024)
    }

    fn local_workspace_bindings(
        &self,
        app: &tauri::AppHandle,
    ) -> Result<BTreeMap<String, WorkspaceBinding>, String> {
        let path = Self::bindings_path(app)?;
        let _guard = self
            .bindings_lock
            .lock()
            .map_err(|_| "Local workspace binding state is unavailable.")?;
        let _file_lock = Self::bindings_file_lock(&path)?;
        Ok(Self::read_bindings(&path)?.workspaces)
    }

    pub(crate) fn snapshot_desktop_file_bindings(
        &self,
        app: &tauri::AppHandle,
        affected_path: &Path,
        include_descendants: bool,
    ) -> Result<Vec<DesktopWorkspaceBindingSnapshot>, String> {
        if !affected_path.is_absolute() {
            return Err("The desktop file effect path must be absolute.".into());
        }
        let path = Self::bindings_path(app)?;
        let _guard = self
            .bindings_lock
            .lock()
            .map_err(|_| "Local workspace binding state is unavailable.")?;
        let _file_lock = Self::bindings_file_lock(&path)?;
        let bindings = Self::read_bindings(&path)?;
        let snapshots = bindings
            .workspaces
            .into_iter()
            .filter_map(|(workspace_id, binding)| {
                let root_path = PathBuf::from(&binding.root_path);
                (root_path == affected_path
                    || include_descendants && root_path.starts_with(affected_path))
                .then(|| {
                    let canonical_workspace_id = binding
                        .canonical_workspace_id
                        .unwrap_or_else(|| workspace_id.clone());
                    DesktopWorkspaceBindingSnapshot {
                        workspace_id,
                        root_path: binding.root_path,
                        generation: binding.generation,
                        canonical_workspace_id,
                    }
                })
            })
            .collect::<Vec<_>>();
        Ok(snapshots)
    }

    pub(crate) fn apply_desktop_file_binding_effects(
        &self,
        app: &tauri::AppHandle,
        operation_id: NativeIntentId,
        digest: &str,
        snapshots: &[DesktopWorkspaceBindingSnapshot],
        old_path: Option<&Path>,
        new_path: Option<&Path>,
        protected_operation_ids: &HashSet<String>,
    ) -> Result<(), String> {
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("The desktop file binding effect digest is invalid.".into());
        }
        let operation_id = String::from(operation_id);
        let path = Self::bindings_path(app)?;
        let _guard = self
            .bindings_lock
            .lock()
            .map_err(|_| "Local workspace binding state is unavailable.")?;
        let _file_lock = Self::bindings_file_lock(&path)?;
        let mut bindings = Self::read_bindings(&path)?;
        if let Some(existing) = bindings.apply_ledger.get(&operation_id) {
            if existing.digest == digest {
                return Ok(());
            }
            return Err("The desktop file operation has a conflicting binding receipt.".into());
        }
        let old_path = old_path.map(Path::to_path_buf);
        if !snapshots.is_empty() && old_path.is_none() {
            return Err("The successful desktop file receipt omitted its old path.".into());
        }
        let mut seen_ids = HashSet::new();
        for snapshot in snapshots {
            let workspace_id =
                lomi_remote_protocol::WorkspaceId::try_from(snapshot.workspace_id.clone())
                    .map_err(|_| "The desktop file binding snapshot ID is invalid.")?;
            let workspace_id: String = workspace_id.into();
            let canonical_workspace_id = lomi_remote_protocol::WorkspaceId::try_from(
                snapshot.canonical_workspace_id.clone(),
            )
            .map_err(|_| "The desktop file canonical binding snapshot ID is invalid.")?;
            if snapshot.generation == 0
                || !seen_ids.insert(workspace_id.clone())
                || !Path::new(&snapshot.root_path).is_absolute()
            {
                return Err("The desktop file binding snapshot is invalid.".into());
            }
            let Some(binding) = bindings.workspaces.get(&workspace_id) else {
                continue;
            };
            let current_canonical_id = binding
                .canonical_workspace_id
                .as_deref()
                .unwrap_or(&workspace_id);
            if binding.root_path != snapshot.root_path
                || binding.generation != snapshot.generation
                || current_canonical_id != String::from(canonical_workspace_id.clone())
            {
                continue;
            }
            let old_root = Path::new(&snapshot.root_path);
            let effect_old_path = old_path
                .as_deref()
                .ok_or("The successful desktop file receipt omitted its old path.")?;
            if old_root != effect_old_path && !old_root.starts_with(effect_old_path) {
                continue;
            }
            if let Some(new_path) = new_path {
                let relative = old_root
                    .strip_prefix(effect_old_path)
                    .map_err(|_| "The captured workspace binding is outside the moved path.")?;
                let relocated = if relative.as_os_str().is_empty() {
                    new_path.to_path_buf()
                } else {
                    new_path.join(relative)
                };
                let relocated = relocated
                    .to_str()
                    .ok_or("The relocated workspace folder path is not valid UTF-8.")?;
                let generation = bindings.next_generation;
                let next_generation = generation
                    .checked_add(1)
                    .ok_or("The local workspace binding generation is exhausted.")?;
                if let Some(binding) = bindings.workspaces.get_mut(&workspace_id) {
                    binding.root_path = relocated.to_owned();
                    binding.generation = generation;
                    bindings.next_generation = next_generation;
                }
            } else {
                bindings.workspaces.remove(&workspace_id);
            }
        }
        bindings.apply_ledger.insert(
            operation_id.clone(),
            WorkspaceBindingApplyRecord {
                digest: digest.to_owned(),
                applied_at_ms: native_git_now_ms().max(1),
            },
        );
        if bindings.apply_ledger.len() > WORKSPACE_BINDING_APPLY_LEDGER_LIMIT {
            let mut removable: Vec<_> = bindings
                .apply_ledger
                .iter()
                .filter(|(operation_id, _)| !protected_operation_ids.contains(*operation_id))
                .map(|(operation_id, record)| (record.applied_at_ms, operation_id.clone()))
                .collect();
            removable.sort();
            let mut excess = bindings
                .apply_ledger
                .len()
                .saturating_sub(WORKSPACE_BINDING_APPLY_LEDGER_LIMIT);
            for (_, operation_id) in removable {
                if excess == 0 {
                    break;
                }
                bindings.apply_ledger.remove(&operation_id);
                excess -= 1;
            }
            if excess > 0 {
                return Err("The desktop workspace binding recovery ledger is full.".into());
            }
        }
        Self::write_bindings(&path, &mut bindings)
    }

    fn bind_workspace(
        &self,
        app: &tauri::AppHandle,
        workspace_id: &lomi_remote_protocol::WorkspaceId,
        root_path: &Path,
    ) -> Result<(), String> {
        let path = Self::bindings_path(app)?;
        let _guard = self
            .bindings_lock
            .lock()
            .map_err(|_| "Local workspace binding state is unavailable.")?;
        let _file_lock = Self::bindings_file_lock(&path)?;
        let mut bindings = Self::read_bindings(&path)?;
        let workspace_id: String = workspace_id.clone().into();
        let root_path = root_path
            .to_str()
            .ok_or("The workspace folder path is not valid UTF-8.")?
            .to_owned();
        match bindings.workspaces.get(&workspace_id) {
            Some(existing) if existing.root_path != root_path => {
                return Err("The desktop workspace is already bound to a different folder.".into());
            }
            Some(_) => return Ok(()),
            None => {}
        }
        if bindings.workspaces.len() >= 256 {
            return Err("The desktop has reached the local workspace binding limit.".into());
        }
        let generation = bindings.next_generation;
        bindings.next_generation = generation
            .checked_add(1)
            .ok_or("The local workspace binding generation is exhausted.")?;
        bindings.workspaces.insert(
            workspace_id,
            WorkspaceBinding {
                root_path,
                generation,
                canonical_workspace_id: None,
                aliases: Vec::new(),
            },
        );
        Self::write_bindings(&path, &mut bindings)
    }

    fn workspace_root(
        &self,
        app: &tauri::AppHandle,
        workspace_id: &lomi_remote_protocol::WorkspaceId,
    ) -> Result<std::path::PathBuf, String> {
        let workspace_id_text: String = workspace_id.clone().into();
        let reconciled = self.reconcile_workspace_bindings(
            app,
            "workspace-binding-reconcile",
            std::slice::from_ref(&workspace_id_text),
        )?;
        let (root_path, canonical_workspace_id) = reconciled
            .bindings
            .into_iter()
            .find_map(|binding| match binding {
                DesktopWorkspaceHostBinding::Active {
                    workspace_id: import_id,
                    canonical_workspace_id,
                    root_path,
                    ..
                } if import_id == workspace_id_text => Some((root_path, canonical_workspace_id)),
                _ => None,
            })
            .ok_or_else(|| "The local Host workspace binding is unavailable.".to_owned())?;
        let local = self
            .local_workspace_bindings(app)?
            .remove(&workspace_id_text)
            .ok_or("The local Host workspace binding is unavailable.")?;
        let local_canonical = local
            .canonical_workspace_id
            .unwrap_or_else(|| workspace_id_text.clone());
        if local.root_path != root_path || local_canonical != canonical_workspace_id {
            return Err("The local Host workspace binding changed during reconciliation.".into());
        }
        Ok(PathBuf::from(root_path))
    }

    fn native_git_binding_for_path(
        &self,
        app: &tauri::AppHandle,
        repository_path: &Path,
    ) -> Result<(std::path::PathBuf, lomi_remote_protocol::WorkspaceId), String> {
        let reconciled =
            self.reconcile_workspace_bindings(app, "workspace-binding-reconcile", &[])?;
        let repository_path = fs::canonicalize(repository_path)
            .map_err(|_| "The Git repository folder is unavailable.")?;
        if !repository_path.is_dir() {
            return Err("The Git repository folder is unavailable.".into());
        }
        let mut candidates = Vec::new();
        let local_bindings = self.local_workspace_bindings(app)?;
        for binding in reconciled.bindings {
            let DesktopWorkspaceHostBinding::Active {
                workspace_id,
                root_path,
                canonical_workspace_id,
                ..
            } = binding
            else {
                continue;
            };
            let Some(local) = local_bindings.get(&workspace_id) else {
                continue;
            };
            let local_canonical_id = local
                .canonical_workspace_id
                .as_deref()
                .unwrap_or(&workspace_id);
            if local.root_path != root_path || local_canonical_id != canonical_workspace_id {
                continue;
            }
            let Ok(root) = fs::canonicalize(root_path) else {
                continue;
            };
            if !root.is_dir() || !repository_path.starts_with(&root) {
                continue;
            }
            let canonical_id = canonical_workspace_id;
            let canonical_id = lomi_remote_protocol::WorkspaceId::try_from(canonical_id)
                .map_err(|_| "The saved canonical workspace binding ID is invalid.")?;
            candidates.push((root, canonical_id));
        }
        let longest = candidates
            .iter()
            .map(|(root, _)| root.components().count())
            .max()
            .ok_or("Import this workspace into the local host before using Git mutations.")?;
        candidates.retain(|(root, _)| root.components().count() == longest);
        let workspace_ids = candidates
            .iter()
            .map(|(_, workspace_id)| workspace_id.clone())
            .collect::<HashSet<_>>();
        if workspace_ids.len() != 1 {
            return Err(
                "This folder belongs to multiple imported host workspaces. Select one in the shared workspace view before changing Git state."
                    .into(),
            );
        }
        let root_paths = candidates
            .iter()
            .map(|(root, _)| root.clone())
            .collect::<HashSet<_>>();
        if root_paths.len() != 1 {
            return Err("The imported host workspace binding is ambiguous.".into());
        }
        let (root_path, workspace_id) = candidates
            .into_iter()
            .next()
            .ok_or("The imported host workspace binding is unavailable.")?;
        Ok((root_path, workspace_id))
    }

    pub(crate) fn native_git_binding(
        &self,
        app: &tauri::AppHandle,
        repository_path: &Path,
    ) -> Result<NativeGitBinding, String> {
        let (root_path, workspace_id) = self.native_git_binding_for_path(app, repository_path)?;
        let workspace = match self.request(
            "native-git-mutations",
            LocalCall::WorkspaceInspect(LocalWorkspaceRead { workspace_id }),
        )? {
            LocalResult::WorkspaceState(workspace) => *workspace,
            _ => return Err("Local host returned an invalid workspace state response.".into()),
        };
        Ok(NativeGitBinding {
            root_path,
            workspace,
        })
    }

    pub(crate) fn native_git_rpc(
        &self,
        request_id: String,
        call: RpcCall,
    ) -> Result<RpcResponse, String> {
        if !matches!(
            &call,
            RpcCall::GitRepositories(_)
                | RpcCall::GitStatus(_)
                | RpcCall::GitStage(_)
                | RpcCall::GitUnstage(_)
                | RpcCall::GitCommit(_)
                | RpcCall::GitFetch(_)
                | RpcCall::GitPush(_)
                | RpcCall::GitPull(_)
                | RpcCall::OperationStatus(_)
        ) {
            return Err("Invalid local Git request.".into());
        }
        let request = host_rpc_request(request_id, call)?;
        self.request_rpc("native-git-mutations", request, None)
    }

    pub(crate) fn native_git_prepare_guarded(
        &self,
        app: &tauri::AppHandle,
        expected_workspace_id: &lomi_remote_protocol::WorkspaceId,
        workspace_path: &Path,
        operation_id: String,
        request_json: String,
        timeout_ms: u64,
        check: impl FnMut() -> bool,
    ) -> Result<NativeGitHostResponse, String> {
        let control_root = native_git_app_data_dir(app)?;
        let requested_root = fs::canonicalize(workspace_path)
            .map_err(|_| "The requested Git project folder is unavailable.")?;
        if !requested_root.is_dir() {
            return Err("The requested Git project folder is unavailable.".into());
        }
        if native_git_path_overlaps_control_root(&requested_root, &control_root) {
            return Err(
                "Git mutations cannot target Lomi's private application data directory.".into(),
            );
        }
        let (root_path, workspace_id) = self.native_git_binding_for_path(app, &requested_root)?;
        if &workspace_id != expected_workspace_id {
            return Err("The local workspace binding does not match the Git request.".into());
        }
        if native_git_path_overlaps_control_root(&root_path, &control_root) {
            return Err(
                "Git mutations cannot target Lomi's private application data directory.".into(),
            );
        }
        let root = root_path
            .to_str()
            .ok_or("The local Git workspace path is not valid UTF-8.")?;
        let root = LocalRootPath::try_from(root.to_owned())
            .map_err(|_| "The local Git workspace path is invalid.")?;
        let request_json = LocalNativeResultJson::try_from(request_json)
            .map_err(|_| "The native Git request exceeds the supported size.")?;
        let input = LocalNativeGitInput {
            workspace_id,
            root,
            control_roots: BoundedList(vec![LocalRootPath::try_from(
                control_root
                    .to_str()
                    .ok_or("Lomi's private application data path is not valid UTF-8.")?
                    .to_owned(),
            )
            .map_err(|_| "Lomi's private application data path is invalid.")?]),
            request_json,
        };
        let request_hash = hash_local_native_git_input(&input)?;
        let request_hash_commitment = HostKeyCommitmentHex::try_from(request_hash.clone())
            .map_err(|_| "The native Git request hash is invalid.")?;
        let operation_id = NativeIntentId::try_from(operation_id)
            .map_err(|_| "The native Git operation ID is invalid.")?;
        self.begin_native_git_outbox(app, operation_id, request_hash_commitment, timeout_ms)?;
        let result = self.native_git_request_guarded(
            LocalCall::NativeGitPrepareGuarded(LocalNativeGitPrepare {
                operation_id,
                input,
                timeout_ms: TimeoutMs::try_from(timeout_ms)
                    .map_err(|_| "The native Git request budget is invalid.")?,
            }),
            check,
        );
        let result = result.and_then(|result| match result {
            LocalResult::NativeGit(result) => Ok(result),
            _ => Err("Local host returned an invalid native Git prepare response.".into()),
        });
        let result = match result {
            Ok(result) => match self.record_native_git_result(app, &result) {
                Ok(()) => Ok(result),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        Ok(NativeGitHostResponse {
            request_hash,
            result,
        })
    }

    pub(crate) fn native_git_execute_guarded(
        &self,
        app: &tauri::AppHandle,
        operation_id: String,
        request_hash: String,
        plan_hash: String,
        timeout_ms: u64,
        check: impl FnMut() -> bool,
    ) -> Result<LocalNativeGitResult, String> {
        let operation_id = NativeIntentId::try_from(operation_id)
            .map_err(|_| "The native Git operation ID is invalid.")?;
        let request_hash = lomi_remote_protocol::HostKeyCommitmentHex::try_from(request_hash)
            .map_err(|_| "The native Git request hash is invalid.")?;
        let plan_hash = lomi_remote_protocol::HostKeyCommitmentHex::try_from(plan_hash)
            .map_err(|_| "The native Git plan hash is invalid.")?;
        self.mark_native_git_execute_sent(app, operation_id, &request_hash, plan_hash.clone())?;
        match self.native_git_request_guarded(
            LocalCall::NativeGitExecuteGuarded(LocalNativeGitExecute {
                operation_id,
                request_hash,
                plan_hash,
                timeout_ms: TimeoutMs::try_from(timeout_ms)
                    .map_err(|_| "The native Git request budget is invalid.")?,
            }),
            check,
        )? {
            LocalResult::NativeGit(result) => {
                self.record_native_git_result(app, &result)?;
                Ok(result)
            }
            _ => Err("Local host returned an invalid native Git execute response.".into()),
        }
    }

    pub(crate) fn native_git_status(
        &self,
        app: &tauri::AppHandle,
        operation_id: String,
    ) -> Result<LocalNativeGitResult, String> {
        let operation_id = NativeIntentId::try_from(operation_id)
            .map_err(|_| "The native Git operation ID is invalid.")?;
        let result = self
            .native_git_status_optional(operation_id)?
            .ok_or("The local Host no longer has this NativeGit receipt.")?;
        self.record_native_git_result(app, &result)?;
        Ok(result)
    }

    pub(crate) fn native_git_cancel_prepared(
        &self,
        app: &tauri::AppHandle,
        operation_id: String,
    ) -> Result<LocalNativeGitResult, String> {
        let operation_id = NativeIntentId::try_from(operation_id)
            .map_err(|_| "The native Git operation ID is invalid.")?;
        let result =
            self.native_git_simple_request_id(operation_id, LocalCall::NativeGitCancelPrepared)?;
        self.record_native_git_result(app, &result)?;
        Ok(result)
    }

    pub(crate) fn native_git_acknowledge(
        &self,
        app: &tauri::AppHandle,
        operation_id: String,
    ) -> Result<LocalNativeGitResult, String> {
        let operation_id = NativeIntentId::try_from(operation_id)
            .map_err(|_| "The native Git operation ID is invalid.")?;
        self.mark_native_git_ack_pending(app, operation_id)?;
        let result =
            self.native_git_simple_request_id(operation_id, LocalCall::NativeGitAcknowledge)?;
        self.record_native_git_result(app, &result)?;
        self.mark_native_git_acknowledged(app, operation_id)?;
        Ok(result)
    }

    fn native_git_simple_request_id(
        &self,
        intent_id: NativeIntentId,
        call: fn(NativeIntentTarget) -> LocalCall,
    ) -> Result<LocalNativeGitResult, String> {
        match self.request(
            "native-git-mutations",
            call(NativeIntentTarget { intent_id }),
        )? {
            LocalResult::NativeGit(result) => Ok(result),
            _ => Err("Local host returned an invalid native Git response.".into()),
        }
    }

    fn native_git_status_optional(
        &self,
        operation_id: NativeIntentId,
    ) -> Result<Option<LocalNativeGitResult>, String> {
        match self.request_with_local_error(
            "native-git-mutations",
            LocalCall::NativeGitStatus(NativeIntentTarget {
                intent_id: operation_id,
            }),
        )? {
            Ok(LocalResult::NativeGit(result)) => Ok(Some(result)),
            Err(LocalError::InvalidArgument) => Ok(None),
            Err(error) => Err(format!("Local host rejected the request ({error:?}).")),
            Ok(_) => Err("Local host returned an invalid native Git status response.".into()),
        }
    }

    fn native_git_request_guarded(
        &self,
        call: LocalCall,
        check: impl FnMut() -> bool,
    ) -> Result<LocalResult, String> {
        if !matches!(
            &call,
            LocalCall::NativeGitPrepareGuarded(_) | LocalCall::NativeGitExecuteGuarded(_)
        ) {
            return Err("Invalid guarded local Git request.".into());
        }
        self.guarded_local_request("native-git-mutations", call, check)
    }

    fn apply_authoritative_workspace_bindings(
        &self,
        app: &tauri::AppHandle,
        captured: &[DesktopWorkspaceBindingSnapshot],
        host_bindings: &[HostWorkspaceBinding],
    ) -> Result<HashSet<String>, String> {
        let path = Self::bindings_path(app)?;
        let _guard = self
            .bindings_lock
            .lock()
            .map_err(|_| "Local workspace binding state is unavailable.")?;
        let _file_lock = Self::bindings_file_lock(&path)?;
        let mut bindings = Self::read_bindings(&path)?;
        let host_by_import_id: BTreeMap<_, _> = host_bindings
            .iter()
            .map(|binding| (String::from(workspace_binding_import_id(binding)), binding))
            .collect();
        let mut changed = false;
        let mut relocated = HashSet::new();
        for snapshot in captured {
            let Some(host_binding) = host_by_import_id.get(&snapshot.workspace_id) else {
                continue;
            };
            let current_matches =
                bindings
                    .workspaces
                    .get(&snapshot.workspace_id)
                    .is_some_and(|current| {
                        current.root_path == snapshot.root_path
                            && current.generation == snapshot.generation
                            && current
                                .canonical_workspace_id
                                .as_deref()
                                .unwrap_or(&snapshot.workspace_id)
                                == snapshot.canonical_workspace_id
                    });
            if !current_matches {
                continue;
            }
            match host_binding {
                HostWorkspaceBinding::Active {
                    workspace_id,
                    root_path,
                    ..
                } => {
                    let canonical_workspace_id: String = workspace_id.clone().into();
                    let root_path = root_path.as_str();
                    if !Path::new(root_path).is_absolute() || root_path.len() > 4096 {
                        return Err("The Host returned an invalid workspace root path.".into());
                    }
                    let needs_update =
                        bindings
                            .workspaces
                            .get(&snapshot.workspace_id)
                            .is_some_and(|current| {
                                current.root_path != root_path
                                    || current.canonical_workspace_id.as_deref()
                                        != Some(canonical_workspace_id.as_str())
                            });
                    if needs_update {
                        let generation = bindings.next_generation;
                        let next_generation = generation
                            .checked_add(1)
                            .ok_or("The local workspace binding generation is exhausted.")?;
                        let Some(current) = bindings.workspaces.get_mut(&snapshot.workspace_id)
                        else {
                            continue;
                        };
                        current.root_path = root_path.to_owned();
                        current.canonical_workspace_id = Some(canonical_workspace_id);
                        current.generation = generation;
                        bindings.next_generation = next_generation;
                        relocated.insert(snapshot.workspace_id.clone());
                        changed = true;
                    }
                }
                HostWorkspaceBinding::Retired { .. } => {
                    bindings.workspaces.remove(&snapshot.workspace_id);
                    changed = true;
                }
                HostWorkspaceBinding::Missing { .. } | HostWorkspaceBinding::Pending { .. } => {}
            }
        }
        if changed {
            Self::write_bindings(&path, &mut bindings)?;
        }
        Ok(relocated)
    }

    pub(crate) fn reconcile_workspace_bindings(
        &self,
        app: &tauri::AppHandle,
        view: &str,
        requested_workspace_ids: &[String],
    ) -> Result<WorkspaceBindingReconcileResult, String> {
        if requested_workspace_ids.len() > 256 {
            return Err("The desktop workspace binding request exceeds its limit.".into());
        }
        let path = Self::bindings_path(app)?;
        let captured = {
            let _guard = self
                .bindings_lock
                .lock()
                .map_err(|_| "Local workspace binding state is unavailable.")?;
            let _file_lock = Self::bindings_file_lock(&path)?;
            let bindings = Self::read_bindings(&path)?;
            bindings
                .workspaces
                .into_iter()
                .map(|(workspace_id, binding)| DesktopWorkspaceBindingSnapshot {
                    canonical_workspace_id: binding
                        .canonical_workspace_id
                        .unwrap_or_else(|| workspace_id.clone()),
                    workspace_id,
                    root_path: binding.root_path,
                    generation: binding.generation,
                })
                .collect::<Vec<_>>()
        };

        let mut requested_ids = BTreeSet::new();
        for workspace_id in requested_workspace_ids {
            lomi_remote_protocol::WorkspaceId::try_from(workspace_id.clone())
                .map_err(|_| "The desktop workspace ID is invalid.")?;
            requested_ids.insert(workspace_id.clone());
        }
        requested_ids.extend(captured.iter().map(|binding| binding.workspace_id.clone()));
        if requested_ids.len() > 256 {
            return Err("The desktop workspace binding request exceeds its limit.".into());
        }
        let import_ids = requested_ids
            .iter()
            .map(|workspace_id| {
                lomi_remote_protocol::WorkspaceId::try_from(workspace_id.clone())
                    .map_err(|_| "The desktop workspace ID is invalid.")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let host_snapshot = match self.request(
            view,
            LocalCall::WorkspaceBindingsRead(WorkspaceBindingsRead {
                import_ids: BoundedList(import_ids.clone()),
            }),
        )? {
            LocalResult::WorkspaceBindings(snapshot) => snapshot,
            _ => return Err("Local host returned an invalid workspace binding response.".into()),
        };
        if host_snapshot.bindings.0.len() != import_ids.len()
            || host_snapshot
                .bindings
                .0
                .iter()
                .zip(&import_ids)
                .any(|(binding, expected)| workspace_binding_import_id(binding) != *expected)
        {
            return Err("Local host returned workspace bindings in an invalid order.".into());
        }
        for binding in &host_snapshot.bindings.0 {
            if let HostWorkspaceBinding::Active { root_path, .. } = binding {
                let root_path = root_path.as_str();
                if !Path::new(root_path).is_absolute() || root_path.len() > 4096 {
                    return Err("The Host returned an invalid workspace root path.".into());
                }
            }
        }
        let status = match self.request(view, LocalCall::Status {})? {
            LocalResult::Status(status) => status,
            _ => return Err("Local host returned an invalid status response.".into()),
        };
        if host_snapshot.host_id != status.host_id || host_snapshot.runtime_id != status.runtime_id
        {
            return Err("The local Host changed during workspace binding reconciliation.".into());
        }

        let relocated =
            self.apply_authoritative_workspace_bindings(app, &captured, &host_snapshot.bindings.0)?;
        let current_bindings = self.local_workspace_bindings(app)?;
        let captured_roots: BTreeMap<_, _> = captured
            .iter()
            .map(|binding| (binding.workspace_id.as_str(), binding.root_path.as_str()))
            .collect();
        let bindings = host_snapshot
            .bindings
            .0
            .into_iter()
            .map(|binding| {
                let import_id = workspace_binding_import_id(&binding);
                let workspace_id: String = import_id.into();
                let captured_root_path = captured_roots
                    .get(workspace_id.as_str())
                    .map(|path| (*path).to_owned());
                let previous_root_path = relocated
                    .contains(&workspace_id)
                    .then(|| captured_root_path.clone())
                    .flatten();
                match binding {
                    HostWorkspaceBinding::Missing { .. } => DesktopWorkspaceHostBinding::Missing {
                        workspace_id,
                        previous_root_path: captured_root_path,
                    },
                    HostWorkspaceBinding::Active {
                        workspace_id: canonical_workspace_id,
                        root_path,
                        ..
                    } => {
                        let canonical_workspace_id: String = canonical_workspace_id.into();
                        let root_path = root_path.as_str().to_owned();
                        let locally_confirmed =
                            current_bindings.get(&workspace_id).is_some_and(|binding| {
                                binding.root_path == root_path
                                    && binding
                                        .canonical_workspace_id
                                        .as_deref()
                                        .unwrap_or(&workspace_id)
                                        == canonical_workspace_id
                            });
                        if locally_confirmed {
                            DesktopWorkspaceHostBinding::Active {
                                workspace_id,
                                canonical_workspace_id,
                                root_path,
                                previous_root_path,
                            }
                        } else {
                            DesktopWorkspaceHostBinding::Pending {
                                workspace_id,
                                canonical_workspace_id,
                                previous_root_path: captured_root_path,
                            }
                        }
                    }
                    HostWorkspaceBinding::Pending {
                        workspace_id: canonical_workspace_id,
                        ..
                    } => DesktopWorkspaceHostBinding::Pending {
                        workspace_id,
                        canonical_workspace_id: canonical_workspace_id.into(),
                        previous_root_path: captured_root_path,
                    },
                    HostWorkspaceBinding::Retired {
                        workspace_id: canonical_workspace_id,
                        ..
                    } => DesktopWorkspaceHostBinding::Retired {
                        workspace_id,
                        canonical_workspace_id: canonical_workspace_id.into(),
                        previous_root_path: captured_root_path,
                    },
                }
            })
            .collect();
        Ok(WorkspaceBindingReconcileResult {
            host_id: host_snapshot.host_id.into(),
            runtime_id: host_snapshot.runtime_id.into(),
            bindings,
        })
    }

    fn canonical_workspace_id(
        &self,
        app: &tauri::AppHandle,
        workspace_id: &lomi_remote_protocol::WorkspaceId,
    ) -> Result<lomi_remote_protocol::WorkspaceId, String> {
        let workspace_id_text: String = workspace_id.clone().into();
        let result = self.reconcile_workspace_bindings(
            app,
            "workspace-binding-reconcile",
            std::slice::from_ref(&workspace_id_text),
        )?;
        let (canonical_workspace_id, root_path) = result
            .bindings
            .into_iter()
            .find_map(|binding| match binding {
                DesktopWorkspaceHostBinding::Active {
                    workspace_id: import_id,
                    canonical_workspace_id,
                    root_path,
                    ..
                } if import_id == workspace_id_text => Some((canonical_workspace_id, root_path)),
                _ => None,
            })
            .ok_or_else(|| "The local Host workspace binding is unavailable.".to_owned())?;
        let local = self
            .local_workspace_bindings(app)?
            .remove(&workspace_id_text)
            .ok_or("The local Host workspace binding is unavailable.")?;
        let local_canonical = local
            .canonical_workspace_id
            .unwrap_or_else(|| workspace_id_text.clone());
        if local.root_path != root_path || local_canonical != canonical_workspace_id {
            return Err("The local Host workspace binding changed during reconciliation.".into());
        }
        lomi_remote_protocol::WorkspaceId::try_from(canonical_workspace_id)
            .map_err(|_| "The saved canonical workspace binding ID is invalid.".into())
    }

    fn workspace_bindings(
        &self,
        app: &tauri::AppHandle,
        view: &str,
        requested_workspace_ids: &[String],
    ) -> Result<Vec<DesktopWorkspaceHostBinding>, String> {
        Ok(self
            .reconcile_workspace_bindings(app, view, requested_workspace_ids)?
            .bindings)
    }

    fn save_canonical_workspace_id(
        &self,
        app: &tauri::AppHandle,
        import_id: &lomi_remote_protocol::WorkspaceId,
        canonical_workspace_id: &lomi_remote_protocol::WorkspaceId,
    ) -> Result<(), String> {
        let path = Self::bindings_path(app)?;
        let _guard = self
            .bindings_lock
            .lock()
            .map_err(|_| "Local workspace binding state is unavailable.")?;
        let _file_lock = Self::bindings_file_lock(&path)?;
        let mut bindings = Self::read_bindings(&path)?;
        let import_id: String = import_id.clone().into();
        let canonical_workspace_id: String = canonical_workspace_id.clone().into();
        let binding = bindings
            .workspaces
            .get_mut(&import_id)
            .ok_or("The desktop workspace binding is unavailable.")?;
        if let Some(existing) = &binding.canonical_workspace_id {
            if existing != &canonical_workspace_id {
                return Err(
                    "The desktop workspace is already bound to a different host workspace.".into(),
                );
            }
            return Ok(());
        }
        binding.canonical_workspace_id = Some(canonical_workspace_id);
        Self::write_bindings(&path, &mut bindings)
    }

    fn terminal_aliases(
        &self,
        app: &tauri::AppHandle,
        workspace_id: &lomi_remote_protocol::WorkspaceId,
    ) -> Result<Vec<DesktopTerminalAlias>, String> {
        let path = Self::bindings_path(app)?;
        let _guard = self
            .bindings_lock
            .lock()
            .map_err(|_| "Local workspace binding state is unavailable.")?;
        let _file_lock = Self::bindings_file_lock(&path)?;
        let bindings = Self::read_bindings(&path)?;
        let workspace_id: String = workspace_id.clone().into();
        let binding = bindings
            .workspaces
            .get(&workspace_id)
            .ok_or("The desktop workspace has not been imported into the local host.");
        let Ok(binding) = binding else {
            return Ok(Vec::new());
        };
        Ok(binding.aliases.clone())
    }

    fn save_terminal_alias(
        &self,
        app: &tauri::AppHandle,
        workspace_id: &lomi_remote_protocol::WorkspaceId,
        alias: DesktopTerminalAlias,
    ) -> Result<(), String> {
        let path = Self::bindings_path(app)?;
        let _guard = self
            .bindings_lock
            .lock()
            .map_err(|_| "Local workspace binding state is unavailable.")?;
        let _file_lock = Self::bindings_file_lock(&path)?;
        let mut bindings = Self::read_bindings(&path)?;
        let workspace_id: String = workspace_id.clone().into();
        let binding = bindings
            .workspaces
            .get_mut(&workspace_id)
            .ok_or("Import this desktop workspace before saving terminal IDs.")?;
        if let Some(existing) = binding
            .aliases
            .iter_mut()
            .find(|existing| existing.desktop_pane_id == alias.desktop_pane_id)
        {
            if existing.host_pane_id != alias.host_pane_id {
                return Err(
                    "The desktop terminal is already bound to a different host pane.".into(),
                );
            }
            *existing = alias;
        } else {
            if binding.aliases.len() >= 32 {
                return Err("The desktop workspace has reached the host terminal limit.".into());
            }
            binding.aliases.push(alias);
        }
        Self::write_bindings(&path, &mut bindings)
    }

    fn remove_terminal_alias(
        &self,
        app: &tauri::AppHandle,
        workspace_id: &lomi_remote_protocol::WorkspaceId,
        desktop_pane_id: &str,
    ) -> Result<(), String> {
        let path = Self::bindings_path(app)?;
        let _guard = self
            .bindings_lock
            .lock()
            .map_err(|_| "Local workspace binding state is unavailable.")?;
        let _file_lock = Self::bindings_file_lock(&path)?;
        let mut bindings = Self::read_bindings(&path)?;
        let workspace_id: String = workspace_id.clone().into();
        let Some(binding) = bindings.workspaces.get_mut(&workspace_id) else {
            return Ok(());
        };
        let previous = binding.aliases.len();
        binding
            .aliases
            .retain(|alias| alias.desktop_pane_id != desktop_pane_id);
        if binding.aliases.len() == previous {
            return Ok(());
        }
        Self::write_bindings(&path, &mut bindings)
    }

    fn prune_terminal_aliases(
        &self,
        app: &tauri::AppHandle,
        workspace_id: &lomi_remote_protocol::WorkspaceId,
        active_host_panes: &HashSet<String>,
    ) -> Result<(), String> {
        let path = Self::bindings_path(app)?;
        let _guard = self
            .bindings_lock
            .lock()
            .map_err(|_| "Local workspace binding state is unavailable.")?;
        let _file_lock = Self::bindings_file_lock(&path)?;
        let mut bindings = Self::read_bindings(&path)?;
        let workspace_id: String = workspace_id.clone().into();
        let Some(binding) = bindings.workspaces.get_mut(&workspace_id) else {
            return Ok(());
        };
        let previous = binding.aliases.len();
        binding
            .aliases
            .retain(|alias| active_host_panes.contains(&alias.host_pane_id));
        if binding.aliases.len() == previous {
            return Ok(());
        }
        Self::write_bindings(&path, &mut bindings)
    }

    fn client(&self, view: &str) -> Result<Arc<Mutex<HostClient>>, String> {
        let mut clients = self
            .clients
            .lock()
            .map_err(|_| "Local host client state is unavailable.")?;
        if let Some(client) = clients.get(view) {
            return Ok(client.clone());
        }

        let profile = lomi_platform::default_profile()
            .ok_or("The canonical local host profile is unavailable.")?;
        #[cfg(target_os = "macos")]
        let client = connect_or_start_local_host(&profile)?;
        #[cfg(not(target_os = "macos"))]
        let client = {
            let executable = std::env::current_exe().map_err(|error| error.to_string())?;
            lomi_host::start(&executable, &profile).map_err(|error| error.to_string())?;
            HostClient::connect(&profile).map_err(|error| error.to_string())?
        };
        let client = Arc::new(Mutex::new(client));
        clients.insert(view.to_owned(), client.clone());
        Ok(client)
    }

    fn request(&self, view: &str, call: LocalCall) -> Result<LocalResult, String> {
        self.request_with_local_error(view, call)?
            .map_err(|error| format!("Local host rejected the request ({error:?})."))
    }

    fn request_with_local_error(
        &self,
        view: &str,
        call: LocalCall,
    ) -> Result<Result<LocalResult, LocalError>, String> {
        let client = self.client(view)?;
        let response = client
            .lock()
            .map_err(|_| "Local host client is unavailable.")?
            .request(call);
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                if let Ok(mut clients) = self.clients.lock() {
                    if clients
                        .get(view)
                        .is_some_and(|current| Arc::ptr_eq(current, &client))
                    {
                        clients.remove(view);
                    }
                }
                return Err(error.to_string());
            }
        };
        Ok(response.result)
    }

    pub(crate) fn native_file_request(&self, call: LocalCall) -> Result<LocalResult, String> {
        if !matches!(
            &call,
            LocalCall::NativeFileStage(_)
                | LocalCall::NativeEditorExecute(_)
                | LocalCall::NativeEditorStatus(_)
                | LocalCall::NativeEditorAcknowledge(_)
        ) {
            return Err("Invalid local editor effect request.".into());
        }
        self.request("native-file-effects", call)
    }

    pub(crate) fn native_editor_disposition_optional(
        &self,
        operation_id: NativeIntentId,
    ) -> Result<NativeEditorDispositionRead, String> {
        let view = "native-file-effects";
        let initial_status = match self.request(view, LocalCall::Status {})? {
            LocalResult::Status(status) => status,
            _ => return Err("Local host returned an invalid status response.".into()),
        };
        let target = NativeIntentTarget {
            intent_id: operation_id,
        };
        let operation_status =
            self.request_with_local_error(view, LocalCall::NativeEditorStatus(target.clone()))?;
        let disposition =
            self.request_with_local_error(view, LocalCall::NativeEditorDispositionRead(target))?;
        let final_status = match self.request(view, LocalCall::Status {})? {
            LocalResult::Status(status) => status,
            _ => return Err("Local host returned an invalid status response.".into()),
        };
        if initial_status.host_id != final_status.host_id
            || initial_status.runtime_id != final_status.runtime_id
        {
            return Err(
                "The local Host changed during native editor disposition reconciliation.".into(),
            );
        }
        let host_id = String::from(initial_status.host_id);
        let runtime_id = String::from(initial_status.runtime_id);
        // InvalidArgument is the Host's NotFound mapping for this validated
        // operation ID. Permission and transport failures remain errors.
        match (operation_status, disposition) {
            (Err(LocalError::InvalidArgument), Err(LocalError::InvalidArgument)) => {
                Ok(NativeEditorDispositionRead::Missing {
                    host_id,
                    runtime_id,
                })
            }
            (Err(error), _) | (_, Err(error)) => Err(format!(
                "Local host rejected native editor disposition status ({error:?})."
            )),
            (
                Ok(LocalResult::NativeEditor(operation_status)),
                Ok(LocalResult::NativeEditorDisposition(disposition)),
            ) if operation_status.operation_id == operation_id
                && disposition.operation_id == operation_id
                && operation_status.state == disposition.state =>
            {
                Ok(NativeEditorDispositionRead::Present(
                    NativeEditorDispositionSnapshot {
                        host_id,
                        runtime_id,
                        disposition,
                    },
                ))
            }
            _ => Err("Local host returned an invalid native editor disposition.".into()),
        }
    }

    pub(crate) fn native_editor_host_identity(&self) -> Result<(String, String), String> {
        match self.request("native-file-effects", LocalCall::Status {})? {
            LocalResult::Status(status) => Ok((status.host_id.into(), status.runtime_id.into())),
            _ => Err("Local host returned an invalid status response.".into()),
        }
    }

    pub(crate) fn native_file_request_guarded(
        &self,
        call: LocalCall,
        check: impl FnMut() -> bool,
    ) -> Result<LocalResult, String> {
        if !matches!(&call, LocalCall::NativeEditorExecuteGuarded(_)) {
            return Err("Invalid guarded local editor effect request.".into());
        }
        self.guarded_local_request("native-file-effects", call, check)
    }

    pub(crate) fn native_file_request_guarded_result(
        &self,
        call: LocalCall,
        check: impl FnMut() -> bool,
    ) -> Result<Result<LocalResult, LocalError>, String> {
        if !matches!(&call, LocalCall::NativeEditorExecuteGuarded(_)) {
            return Err("Invalid guarded local editor effect request.".into());
        }
        self.guarded_local_request_with_local_error("native-file-effects", call, check)
    }

    fn guarded_local_request(
        &self,
        view: &str,
        call: LocalCall,
        mut check: impl FnMut() -> bool,
    ) -> Result<LocalResult, String> {
        self.guarded_local_request_with_local_error(view, call, &mut check)?
            .map_err(|error| format!("Local host rejected the request ({error:?})."))
    }

    fn guarded_local_request_with_local_error(
        &self,
        view: &str,
        call: LocalCall,
        mut check: impl FnMut() -> bool,
    ) -> Result<Result<LocalResult, LocalError>, String> {
        let client = self.client(view)?;
        let response = {
            let mut connection = client
                .lock()
                .map_err(|_| "Local host client is unavailable.")?;
            #[cfg(unix)]
            {
                connection.request_guarded(call, &mut || {
                    if check() {
                        Ok(())
                    } else {
                        Err(lomi_platform::unix::Error::PermissionDenied)
                    }
                })
            }
            #[cfg(windows)]
            {
                connection.request_guarded(call, &mut || {
                    if check() {
                        Ok(())
                    } else {
                        Err(lomi_platform::windows::Error::PermissionDenied)
                    }
                })
            }
        };
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                if let Ok(mut clients) = self.clients.lock() {
                    if clients
                        .get(view)
                        .is_some_and(|current| Arc::ptr_eq(current, &client))
                    {
                        clients.remove(view);
                    }
                }
                return Err(error.to_string());
            }
        };
        Ok(response.result)
    }

    fn request_rpc(
        &self,
        view: &str,
        request: RpcRequest,
        events: Option<tauri::ipc::Channel<SharedHostStreamEvent>>,
    ) -> Result<RpcResponse, String> {
        let client = self.client(view)?;
        let mut connection = client
            .lock()
            .map_err(|_| "Local host client is unavailable.")?;
        let response = connection.request(LocalCall::Rpc(Box::new(request.clone())));
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                drop(connection);
                self.disconnect_view(view, &client);
                return Err(error.to_string());
            }
        };
        let result = response
            .result
            .map_err(|error| format!("Local host rejected the request ({error:?})."))?;
        let LocalResult::Rpc(response) = result else {
            return Err("Local host returned an invalid RPC response.".into());
        };
        let response = *response;

        let has_events = events.is_some();
        if let Some(channel) = events {
            let (subscription, kind) = match (&request.call, &response) {
                (
                    RpcCall::TerminalAttach(_) | RpcCall::TerminalSnapshot(_),
                    RpcResponse::Result {
                        result:
                            RpcResult::TerminalAttach(metadata) | RpcResult::TerminalSnapshot(metadata),
                        ..
                    },
                ) => (
                    metadata.subscription_id.as_str().to_owned(),
                    SharedHostStreamKind::Terminal,
                ),
                (
                    RpcCall::WorkspaceSubscribe(_),
                    RpcResponse::Result {
                        result: RpcResult::WorkspaceSubscribe(metadata),
                        ..
                    },
                ) => (
                    metadata.subscription_id.as_str().to_owned(),
                    SharedHostStreamKind::Workspace,
                ),
                (
                    RpcCall::FilesWatch(_),
                    RpcResponse::Result {
                        result: RpcResult::FilesWatch(metadata),
                        ..
                    },
                ) => (
                    metadata.subscription_id.as_str().to_owned(),
                    SharedHostStreamKind::Files,
                ),
                _ => {
                    return Err(
                        "The local stream channel requires a successful subscription.".into(),
                    );
                }
            };
            let mut streams = self
                .streams
                .lock()
                .map_err(|_| "Local host stream state is unavailable.")?;
            if kind == SharedHostStreamKind::Workspace || kind == SharedHostStreamKind::Files {
                let replaced = streams
                    .iter()
                    .filter(|((owner, _), stream)| owner == view && stream.kind == kind)
                    .map(|(key, stream)| (key.clone(), stream.channel.clone()))
                    .collect::<Vec<_>>();
                for (key, old_channel) in replaced {
                    streams.remove(&key);
                    let _ = old_channel.send(SharedHostStreamEvent::TransportLost);
                }
            }
            streams.insert(
                (view.to_owned(), subscription),
                SharedHostStream {
                    kind,
                    channel,
                    client: client.clone(),
                },
            );
        }
        drop(connection);
        if has_events {
            self.start_event_pump(view.to_owned(), client);
        }
        Ok(response)
    }

    fn start_event_pump(&self, view: String, client: Arc<Mutex<HostClient>>) {
        let Ok(mut pumps) = self.pumps.lock() else {
            return;
        };
        if pumps.contains_key(&view) {
            return;
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        pumps.insert(view.clone(), cancelled.clone());
        drop(pumps);

        let clients = self.clients.clone();
        let streams = self.streams.clone();
        let pumps = self.pumps.clone();
        let _ = std::thread::Builder::new()
            .name("lomi-local-host-events".into())
            .spawn(move || {
                loop {
                    if cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    let next = client.lock().map_err(|_| ()).and_then(|mut client| {
                        client
                            .next_event(std::time::Duration::from_millis(100))
                            .map_err(|_| ())
                    });
                    if !event_pump_is_current(&clients, &pumps, &view, &client, &cancelled) {
                        break;
                    }
                    match next {
                        Ok(Some(event)) => {
                            let Some((subscription, kind, retired)) = stream_target(&event) else {
                                continue;
                            };
                            let key = (view.clone(), subscription);
                            let channel = streams
                                .lock()
                                .ok()
                                .and_then(|streams| streams.get(&key).cloned())
                                .filter(|stream| {
                                    stream.kind == kind && Arc::ptr_eq(&stream.client, &client)
                                });
                            if let Some(stream) = channel {
                                if !event_pump_is_current(
                                    &clients, &pumps, &view, &client, &cancelled,
                                ) {
                                    break;
                                }
                                let delivered = stream
                                    .channel
                                    .send(SharedHostStreamEvent::Frame(event))
                                    .is_ok();
                                if !delivered || retired {
                                    if let Ok(mut streams) = streams.lock() {
                                        if streams.get(&key).is_some_and(|current| {
                                            Arc::ptr_eq(&current.client, &client)
                                        }) {
                                            streams.remove(&key);
                                        }
                                    }
                                }
                            }
                        }
                        Ok(None) => {}
                        Err(()) => {
                            let channels = if let Ok(mut streams) = streams.lock() {
                                let channels = streams
                                    .iter()
                                    .filter(|((owner, _), stream)| {
                                        owner == &view && Arc::ptr_eq(&stream.client, &client)
                                    })
                                    .map(|(_, stream)| stream.channel.clone())
                                    .collect::<Vec<_>>();
                                streams.retain(|(owner, _), stream| {
                                    owner != &view || !Arc::ptr_eq(&stream.client, &client)
                                });
                                channels
                            } else {
                                Vec::new()
                            };
                            for channel in channels {
                                if event_pump_is_current(
                                    &clients, &pumps, &view, &client, &cancelled,
                                ) {
                                    let _ = channel.send(SharedHostStreamEvent::TransportLost);
                                }
                            }
                            if event_pump_is_current(&clients, &pumps, &view, &client, &cancelled) {
                                if let Ok(mut clients) = clients.lock() {
                                    if clients
                                        .get(&view)
                                        .is_some_and(|current| Arc::ptr_eq(current, &client))
                                    {
                                        clients.remove(&view);
                                    }
                                }
                            }
                            break;
                        }
                    }
                }
                if let Ok(mut pumps) = pumps.lock() {
                    if pumps
                        .get(&view)
                        .is_some_and(|current| Arc::ptr_eq(current, &cancelled))
                    {
                        pumps.remove(&view);
                    }
                }
            });
    }

    fn detach_view(&self, view: &str) {
        if let Ok(mut clients) = self.clients.lock() {
            clients.remove(view);
        }
        if let Ok(mut pumps) = self.pumps.lock() {
            if let Some(cancelled) = pumps.remove(view) {
                cancelled.store(true, Ordering::Release);
            }
        }
        let channels = if let Ok(mut streams) = self.streams.lock() {
            let channels = streams
                .iter()
                .filter(|((owner, _), _)| owner == view)
                .map(|(_, stream)| stream.channel.clone())
                .collect::<Vec<_>>();
            streams.retain(|(owner, _), _| owner != view);
            channels
        } else {
            Vec::new()
        };
        for channel in channels {
            let _ = channel.send(SharedHostStreamEvent::TransportLost);
        }
    }

    fn disconnect_view(&self, view: &str, client: &Arc<Mutex<HostClient>>) {
        let Ok(mut clients) = self.clients.lock() else {
            return;
        };
        if !clients
            .get(view)
            .is_some_and(|current| Arc::ptr_eq(current, client))
        {
            return;
        }
        clients.remove(view);
        if let Ok(mut pumps) = self.pumps.lock() {
            if let Some(cancelled) = pumps.remove(view) {
                cancelled.store(true, Ordering::Release);
            }
        }
        let channels = if let Ok(mut streams) = self.streams.lock() {
            let channels = streams
                .iter()
                .filter(|((owner, _), stream)| owner == view && Arc::ptr_eq(&stream.client, client))
                .map(|(_, stream)| stream.channel.clone())
                .collect::<Vec<_>>();
            streams
                .retain(|(owner, _), stream| owner != view || !Arc::ptr_eq(&stream.client, client));
            channels
        } else {
            Vec::new()
        };
        drop(clients);
        for channel in channels {
            let _ = channel.send(SharedHostStreamEvent::TransportLost);
        }
    }
}

fn event_pump_is_current(
    clients: &Mutex<HashMap<String, Arc<Mutex<HostClient>>>>,
    pumps: &Mutex<HashMap<String, Arc<AtomicBool>>>,
    view: &str,
    client: &Arc<Mutex<HostClient>>,
    cancelled: &Arc<AtomicBool>,
) -> bool {
    if cancelled.load(Ordering::Acquire) {
        return false;
    }
    let client_is_current = clients.lock().ok().is_some_and(|clients| {
        clients
            .get(view)
            .is_some_and(|current| Arc::ptr_eq(current, client))
    });
    if !client_is_current {
        return false;
    }
    let pump_is_current = pumps.lock().ok().is_some_and(|pumps| {
        pumps
            .get(view)
            .is_some_and(|current| Arc::ptr_eq(current, cancelled))
    });
    pump_is_current && !cancelled.load(Ordering::Acquire)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedHostAuthSetup {
    bootstrap_configured: bool,
    providers: Vec<LocalAuthProvider>,
}

fn host_auth_bootstrap_configured() -> bool {
    option_env!("LOMI_HOST_AUTH_BOOTSTRAP_CONFIGURED") == Some("1")
}

fn host_auth_configuration(
    provider: LocalAuthProvider,
    keychain_path: Option<String>,
    acknowledge_protected_file: bool,
) -> Result<LocalAuthConfiguration, String> {
    if !host_auth_bootstrap_configured() {
        return Err("Host authentication is not configured for this build.".into());
    }
    let origin = LocalAuthOrigin::try_from(
        option_env!("LOMI_HOST_IDENTITY_ORIGIN")
            .ok_or("The host authentication bootstrap is unavailable.")?
            .to_owned(),
    )
    .map_err(|_| "The host authentication identity pin is invalid.")?;
    let remote_api_origin = LocalAuthOrigin::try_from(
        option_env!("LOMI_HOST_REMOTE_API_ORIGIN")
            .ok_or("The host authentication bootstrap is unavailable.")?
            .to_owned(),
    )
    .map_err(|_| "The host Remote API pin is invalid.")?;
    let relay_endpoint = LocalRelayEndpoint::try_from(
        option_env!("LOMI_HOST_RELAY_ENDPOINT")
            .ok_or("The host authentication bootstrap is unavailable.")?
            .to_owned(),
    )
    .map_err(|_| "The host relay pin is invalid.")?;
    let client_id = LocalAuthClientId::try_from(
        option_env!("LOMI_HOST_CLIENT_ID")
            .ok_or("The host authentication bootstrap is unavailable.")?
            .to_owned(),
    )
    .map_err(|_| "The host authentication client ID is invalid.")?;

    #[cfg(target_os = "macos")]
    let (keychain_path, protected_file_acknowledgement) = match provider {
        LocalAuthProvider::MacOsKeychain => {
            let path = keychain_path.ok_or("Choose an existing macOS keychain file.")?;
            (
                Some(
                    LocalRootPath::try_from(path)
                        .map_err(|_| "The selected macOS keychain path is invalid.")?,
                ),
                None,
            )
        }
        LocalAuthProvider::ProtectedFile if acknowledge_protected_file => {
            (None, Some(LocalFileAcknowledgement::OsPermissionsOnly))
        }
        LocalAuthProvider::ProtectedFile => {
            return Err(
                "Acknowledge that the protected file relies on operating system permissions."
                    .into(),
            );
        }
        _ => return Err("Select a supported macOS secret provider.".into()),
    };
    #[cfg(target_os = "windows")]
    let (keychain_path, protected_file_acknowledgement) = {
        let _ = keychain_path;
        let _ = acknowledge_protected_file;
        if provider != LocalAuthProvider::WindowsDpapi {
            return Err("Select Windows DPAPI on this device.".into());
        }
        (None, None)
    };
    #[cfg(target_os = "linux")]
    let (keychain_path, protected_file_acknowledgement) = {
        let _ = keychain_path;
        match provider {
            LocalAuthProvider::LinuxSecretService => (None, None),
            LocalAuthProvider::ProtectedFile if acknowledge_protected_file => {
                (None, Some(LocalFileAcknowledgement::OsPermissionsOnly))
            }
            LocalAuthProvider::ProtectedFile => {
                return Err(
                    "Acknowledge that the protected file relies on operating system permissions."
                        .into(),
                );
            }
            _ => return Err("Select a supported Linux secret provider.".into()),
        }
    };
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
    let (keychain_path, protected_file_acknowledgement) = {
        let _ = keychain_path;
        if provider != LocalAuthProvider::ProtectedFile || !acknowledge_protected_file {
            return Err(
                "Acknowledge that the protected file relies on operating system permissions."
                    .into(),
            );
        }
        (None, Some(LocalFileAcknowledgement::OsPermissionsOnly))
    };

    let configuration = LocalAuthConfiguration {
        version: 1,
        origin,
        remote_api_origin: Some(remote_api_origin),
        relay_endpoint: Some(relay_endpoint),
        client_id,
        provider,
        keychain_path,
        protected_file_acknowledgement,
    };
    configuration
        .validate()
        .map_err(|_| "The selected host secret provider configuration is invalid.")?;
    Ok(configuration)
}

#[tauri::command]
pub fn shared_host_auth_setup(
    window: Window,
    webview: Webview,
) -> Result<SharedHostAuthSetup, String> {
    trusted_full_window(&window, &webview)?;
    #[cfg(target_os = "macos")]
    let providers = vec![
        LocalAuthProvider::MacOsKeychain,
        LocalAuthProvider::ProtectedFile,
    ];
    #[cfg(target_os = "windows")]
    let providers = vec![LocalAuthProvider::WindowsDpapi];
    #[cfg(target_os = "linux")]
    let providers = vec![
        LocalAuthProvider::LinuxSecretService,
        LocalAuthProvider::ProtectedFile,
    ];
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
    let providers = vec![LocalAuthProvider::ProtectedFile];
    Ok(SharedHostAuthSetup {
        bootstrap_configured: host_auth_bootstrap_configured(),
        providers,
    })
}

#[tauri::command]
pub async fn shared_host_auth_configure(
    window: Window,
    webview: Webview,
    app: tauri::AppHandle,
    state: State<'_, SharedHost>,
    provider: LocalAuthProvider,
    acknowledge_protected_file: bool,
) -> Result<bool, String> {
    trusted_full_window(&window, &webview)?;
    if !host_auth_bootstrap_configured() {
        return Err("Host authentication is not configured for this build.".into());
    }
    #[cfg(target_os = "macos")]
    let keychain_path = if provider == LocalAuthProvider::MacOsKeychain {
        use tauri_plugin_dialog::DialogExt;
        let picker_window = window.clone();
        tauri::async_runtime::spawn_blocking(move || -> Result<Option<String>, String> {
            let dialog = app
                .dialog()
                .file()
                .set_parent(&picker_window)
                .set_title("Choose an existing macOS keychain")
                .add_filter("macOS keychain", &["keychain-db", "keychain"]);
            let Some(selected) = dialog.blocking_pick_file() else {
                return Ok(None);
            };
            let selected = selected
                .into_path()
                .map_err(|_| "Choose a local macOS keychain file.")?;
            let selected = std::fs::canonicalize(selected)
                .map_err(|_| "The selected macOS keychain file is unavailable.")?;
            selected
                .to_str()
                .map(str::to_owned)
                .map(Some)
                .ok_or_else(|| "The selected macOS keychain path is not valid UTF-8.".into())
        })
        .await
        .map_err(|error| error.to_string())??
    } else {
        None
    };
    #[cfg(not(target_os = "macos"))]
    let keychain_path = None;

    #[cfg(target_os = "macos")]
    if provider == LocalAuthProvider::MacOsKeychain && keychain_path.is_none() {
        return Ok(false);
    }
    let configuration =
        host_auth_configuration(provider, keychain_path, acknowledge_protected_file)?;
    let host = state.inner().clone();
    let view = webview.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        match host.request(
            &view,
            LocalCall::AuthConfigure(lomi_remote_protocol::ipc::LocalAuthConfigure {
                configuration,
                operator_intent: Confirmed::Confirmed,
            }),
        )? {
            LocalResult::AuthConfigured {} => Ok(true),
            _ => {
                Err("Local host returned an invalid authentication configuration response.".into())
            }
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn shared_host_login_status(
    window: Window,
    webview: Webview,
    state: State<'_, SharedHost>,
) -> Result<Option<LocalLoginStatus>, String> {
    trusted_full_window(&window, &webview)?;
    if !host_auth_bootstrap_configured() {
        return Ok(None);
    }
    let host = state.inner().clone();
    let view = webview.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        match host.request(&view, LocalCall::LoginStatus {})? {
            LocalResult::LoginStatus(status) => Ok(Some(status)),
            _ => Err("Local host returned an invalid login status response.".into()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn shared_host_login_start(
    window: Window,
    webview: Webview,
    state: State<'_, SharedHost>,
    operation_id: String,
) -> Result<(), String> {
    trusted_full_window(&window, &webview)?;
    if !host_auth_bootstrap_configured() {
        return Err("Host authentication is not configured for this build.".into());
    }
    let operation_id = OperationId::try_from(operation_id)
        .map_err(|_| "The host login operation ID is invalid.")?;
    let host = state.inner().clone();
    let view = webview.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        match host.request(
            &view,
            LocalCall::LoginStart(LocalLoginStart { operation_id }),
        )? {
            LocalResult::LoginAccepted(accepted) if accepted.operation_id == operation_id => Ok(()),
            LocalResult::LoginAccepted(_) => {
                Err("Local host returned a different login operation ID.".into())
            }
            _ => Err("Local host returned an invalid login start response.".into()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn shared_host_login_presentation(
    window: Window,
    webview: Webview,
    state: State<'_, SharedHost>,
) -> Result<LocalLoginPresentation, String> {
    trusted_full_window(&window, &webview)?;
    if !host_auth_bootstrap_configured() {
        return Err("Host authentication is not configured for this build.".into());
    }
    let host = state.inner().clone();
    let view = webview.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        match host.request(&view, LocalCall::LoginPresentation {})? {
            LocalResult::LoginPresentation(presentation) => Ok(presentation),
            _ => Err("Local host returned an invalid login presentation response.".into()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn shared_host_logout(
    window: Window,
    webview: Webview,
    state: State<'_, SharedHost>,
) -> Result<(), String> {
    trusted_full_window(&window, &webview)?;
    let host = state.inner().clone();
    let view = webview.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        match host.request(
            &view,
            LocalCall::Logout(LocalOperatorIntent {
                operator_intent: Confirmed::Confirmed,
            }),
        )? {
            LocalResult::LoginStatus(_) => Ok(()),
            _ => Err("Local host returned an invalid logout response.".into()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

fn stream_target(
    event: &lomi_remote_protocol::ipc::LocalStreamEvent,
) -> Option<(String, SharedHostStreamKind, bool)> {
    use lomi_remote_protocol::ipc::LocalStreamEvent;
    let (subscription, kind, retired) = match event {
        LocalStreamEvent::Terminal(frame) => {
            let (subscription, retired) = match frame {
                TerminalStreamFrame::SnapshotMetadata(value) => {
                    (value.subscription_id.as_str(), false)
                }
                TerminalStreamFrame::SnapshotChunk(value) => {
                    (value.subscription_id.as_str(), false)
                }
                TerminalStreamFrame::SnapshotComplete(value) => {
                    (value.subscription_id.as_str(), false)
                }
                TerminalStreamFrame::Delta(value) => (
                    value.subscription_id.as_str(),
                    matches!(
                        value.change,
                        lomi_remote_protocol::streams::TerminalChange::Exit(_)
                    ),
                ),
                TerminalStreamFrame::DeltaMetadata(value) => {
                    (value.subscription_id.as_str(), false)
                }
                TerminalStreamFrame::DeltaChunk(value) => (value.subscription_id.as_str(), false),
                TerminalStreamFrame::DeltaComplete(value) => {
                    (value.subscription_id.as_str(), false)
                }
                TerminalStreamFrame::Ack(value) => (value.subscription_id.as_str(), false),
                TerminalStreamFrame::ResyncRequired(value) => {
                    (value.subscription_id.as_str(), true)
                }
            };
            (subscription, SharedHostStreamKind::Terminal, retired)
        }
        LocalStreamEvent::Workspace(frame) => {
            let (subscription, retired) = match frame {
                WorkspaceStreamFrame::SnapshotMetadata(value) => {
                    (value.subscription_id.as_str(), false)
                }
                WorkspaceStreamFrame::SnapshotChunk(value) => {
                    (value.subscription_id.as_str(), false)
                }
                WorkspaceStreamFrame::SnapshotComplete(value) => {
                    (value.subscription_id.as_str(), false)
                }
                WorkspaceStreamFrame::Event(value) => (value.subscription_id.as_str(), false),
                WorkspaceStreamFrame::Ack(value) => (value.subscription_id.as_str(), false),
                WorkspaceStreamFrame::ResyncRequired(value) => {
                    (value.subscription_id.as_str(), true)
                }
            };
            (subscription, SharedHostStreamKind::Workspace, retired)
        }
        LocalStreamEvent::Files(frame) => {
            let (subscription, retired) = match frame {
                FileWatchFrame::Invalidated(value) => (value.subscription_id.as_str(), false),
                FileWatchFrame::Ack(value) => (value.subscription_id.as_str(), false),
                FileWatchFrame::ResyncRequired(value) => (value.subscription_id.as_str(), true),
            };
            (subscription, SharedHostStreamKind::Files, retired)
        }
    };
    Some((subscription.to_owned(), kind, retired))
}

pub(crate) fn trusted_full_window(window: &Window, webview: &Webview) -> Result<(), String> {
    let window_label = window.label();
    let view_label = webview.label();
    if window_label != view_label || view_label.starts_with("browser-") {
        return Err("Local host access is available only to first-party desktop windows.".into());
    }
    Ok(())
}

fn collect_host_panes(
    layout: &lomi_remote_protocol::rpc::TerminalLayout,
    into: &mut HashSet<String>,
) {
    use lomi_remote_protocol::rpc::TerminalLayout;
    match layout {
        TerminalLayout::Pane { pane } => {
            into.insert(pane.pane_id.clone().into());
        }
        TerminalLayout::Split { first, second, .. } => {
            collect_host_panes(first, into);
            collect_host_panes(second, into);
        }
    }
}

#[tauri::command]
pub async fn shared_host_status(
    window: Window,
    webview: Webview,
    state: State<'_, SharedHost>,
) -> Result<DesktopSharedHostStatus, String> {
    trusted_full_window(&window, &webview)?;
    let host = state.inner().clone();
    let view = webview.label().to_owned();
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(target_os = "macos")]
        let administrator_install_required =
            lomi_platform::default_profile().is_some_and(|profile| {
                lomi_platform::service_macos::custody::prepare_current_user_host_profile(&profile)
                    .and_then(|()| {
                        lomi_platform::service_macos::custody::installed_privileged_host_executable(
                            &profile,
                        )
                    })
                    .is_err()
            });
        #[cfg(not(target_os = "macos"))]
        let administrator_install_required = false;

        match host.request(&view, LocalCall::Status {}) {
            Ok(LocalResult::Status(status)) => {
                if host.recover_native_git_outbox(&app).is_err() {
                    eprintln!("Native Git recovery remains pending; its Host fence was retained.");
                }
                if crate::desktop_file_effects::recover_pending(&app).is_err() {
                    eprintln!("Desktop file recovery remains pending; its receipt was retained.");
                }
                Ok(DesktopSharedHostStatus {
                    state: if administrator_install_required {
                        DesktopSharedHostState::AdministratorInstallRequired
                    } else {
                        DesktopSharedHostState::Ready
                    },
                    host_id: Some(status.host_id),
                    runtime_id: Some(status.runtime_id),
                    sharing: Some(status.sharing),
                    message: administrator_install_required.then(|| {
                        "Install the signed Lomi Remote administrator Host for this macOS account."
                            .to_owned()
                    }),
                })
            }
            Ok(_) => Ok(DesktopSharedHostStatus {
                state: DesktopSharedHostState::Unavailable,
                host_id: None,
                runtime_id: None,
                sharing: None,
                message: Some("The local Host returned an invalid status response.".into()),
            }),
            Err(error) if error == MACOS_ADMIN_INSTALL_REQUIRED => Ok(DesktopSharedHostStatus {
                state: DesktopSharedHostState::AdministratorInstallRequired,
                host_id: None,
                runtime_id: None,
                sharing: None,
                message: Some(MACOS_HOST_INSTALL_MESSAGE.into()),
            }),
            Err(error) if error == MACOS_HOST_RESTART_REQUIRED => Ok(DesktopSharedHostStatus {
                state: if administrator_install_required {
                    DesktopSharedHostState::AdministratorInstallRequired
                } else {
                    DesktopSharedHostState::UpdateRequired
                },
                host_id: None,
                runtime_id: None,
                sharing: None,
                message: Some(if administrator_install_required {
                    MACOS_HOST_INSTALL_MESSAGE.to_owned()
                } else {
                    MACOS_HOST_RESTART_REQUIRED.to_owned()
                }),
            }),
            Err(error) if error == "UPDATE_REQUIRED" => Ok(DesktopSharedHostStatus {
                state: if administrator_install_required {
                    DesktopSharedHostState::AdministratorInstallRequired
                } else {
                    DesktopSharedHostState::UpdateRequired
                },
                host_id: None,
                runtime_id: None,
                sharing: None,
                message: Some(if administrator_install_required {
                    MACOS_HOST_INSTALL_MESSAGE.to_owned()
                } else {
                    "Update the installed Lomi Remote Host to match this desktop app.".into()
                }),
            }),
            Err(_) if administrator_install_required => Ok(DesktopSharedHostStatus {
                state: DesktopSharedHostState::AdministratorInstallRequired,
                host_id: None,
                runtime_id: None,
                sharing: None,
                message: Some(MACOS_HOST_INSTALL_MESSAGE.to_owned()),
            }),
            Err(_) => Ok(DesktopSharedHostStatus {
                state: DesktopSharedHostState::Unavailable,
                host_id: None,
                runtime_id: None,
                sharing: None,
                message: Some("The installed local Host could not be started or reached.".into()),
            }),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn shared_host_workspace_bindings(
    window: Window,
    webview: Webview,
    app: tauri::AppHandle,
    state: State<'_, SharedHost>,
    workspace_ids: Vec<String>,
    warning_operation_id: Option<String>,
    warning_request_hash: Option<String>,
) -> Result<DesktopWorkspaceBindingsResponse, String> {
    trusted_full_window(&window, &webview)?;
    let host = state.inner().clone();
    let app = app.clone();
    let view = webview.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        match (warning_operation_id, warning_request_hash) {
            (Some(operation_id), Some(request_hash)) => {
                crate::desktop_file_effects::acknowledge_uncertainty_warning(
                    &app,
                    &operation_id,
                    &request_hash,
                )?;
            }
            (None, None) => {}
            _ => return Err("The desktop uncertainty warning acknowledgement is invalid.".into()),
        }
        let uncertainty_warnings = crate::desktop_file_effects::uncertainty_warnings(&app)?;
        let (bindings, binding_error) = match host.workspace_bindings(&app, &view, &workspace_ids) {
            Ok(bindings) => (Some(bindings), None),
            Err(error) => (None, Some(error)),
        };
        Ok(DesktopWorkspaceBindingsResponse {
            bindings,
            binding_error,
            uncertainty_warnings,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn shared_host_workspaces(
    window: Window,
    webview: Webview,
    state: State<'_, SharedHost>,
) -> Result<Vec<LocalWorkspaceSummary>, String> {
    trusted_full_window(&window, &webview)?;
    let host = state.inner().clone();
    let view = webview.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        match host.request(&view, LocalCall::Workspaces {})? {
            LocalResult::Workspaces(workspaces) => Ok(workspaces.0),
            _ => Err("Local host returned an invalid workspace list.".into()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn shared_host_set_workspace_sharing(
    window: Window,
    webview: Webview,
    app: tauri::AppHandle,
    state: State<'_, SharedHost>,
    workspace_id: String,
    shared: bool,
) -> Result<LocalWorkspaceSummary, String> {
    trusted_full_window(&window, &webview)?;
    let workspace_id = lomi_remote_protocol::WorkspaceId::try_from(workspace_id)
        .map_err(|_| "The desktop workspace ID is invalid.")?;
    let host = state.inner().clone();
    let app = app.clone();
    let view = webview.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let workspace_id = host.canonical_workspace_id(&app, &workspace_id)?;
        let target = LocalWorkspaceTarget {
            workspace_id,
            operator_intent: Confirmed::Confirmed,
        };
        let call = if shared {
            LocalCall::Share(target)
        } else {
            LocalCall::Unshare(target)
        };
        match host.request(&view, call)? {
            LocalResult::Workspace(workspace) => Ok(workspace),
            _ => Err("Local host returned an invalid sharing response.".into()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn shared_host_detach_view(
    window: Window,
    webview: Webview,
    state: State<'_, SharedHost>,
) -> Result<(), String> {
    trusted_full_window(&window, &webview)?;
    state.inner().detach_view(webview.label());
    Ok(())
}

#[tauri::command]
pub async fn shared_host_workspace_state(
    window: Window,
    webview: Webview,
    app: tauri::AppHandle,
    state: State<'_, SharedHost>,
    workspace_id: String,
) -> Result<lomi_remote_protocol::rpc::WorkspaceState, String> {
    trusted_full_window(&window, &webview)?;
    let workspace_id = lomi_remote_protocol::WorkspaceId::try_from(workspace_id)
        .map_err(|_| "The desktop workspace ID is invalid.")?;
    let host = state.inner().clone();
    let app = app.clone();
    let view = webview.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let local_workspace_id = workspace_id.clone();
        let host_workspace_id = host.canonical_workspace_id(&app, &workspace_id)?;
        let workspace = match host.request(
            &view,
            LocalCall::WorkspaceInspect(LocalWorkspaceRead {
                workspace_id: host_workspace_id,
            }),
        )? {
            LocalResult::WorkspaceState(workspace) => *workspace,
            _ => return Err("Local host returned an invalid workspace state response.".into()),
        };
        let mut active_panes = HashSet::new();
        for tab in &workspace.tabs.0 {
            collect_host_panes(&tab.layout, &mut active_panes);
        }
        host.prune_terminal_aliases(&app, &local_workspace_id, &active_panes)?;
        Ok(workspace)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn shared_host_register_shell_profile(
    window: Window,
    webview: Webview,
    app: tauri::AppHandle,
    state: State<'_, SharedHost>,
    shells: State<'_, crate::terminal::Shells>,
    workspace_id: String,
    shell_profile_id: String,
    cwd: String,
) -> Result<String, String> {
    trusted_full_window(&window, &webview)?;
    let workspace_id = lomi_remote_protocol::WorkspaceId::try_from(workspace_id)
        .map_err(|_| "The desktop workspace ID is invalid.")?;
    let shells = shells.inner().clone();
    let host = state.inner().clone();
    let app = app.clone();
    let view = webview.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let workspace_root = host.workspace_root(&app, &workspace_id)?;
        let workspace_id = host.canonical_workspace_id(&app, &workspace_id)?;
        let profile = shells
            .profiles
            .iter()
            .find(|profile| profile.id == shell_profile_id)
            .ok_or("The selected shell profile is unavailable.")?;
        let cwd = if profile.distro.is_some() {
            // The trusted command builder translates this canonical Windows
            // project root to WSL and puts it in the `wsl --cd` arguments.
            workspace_root.to_string_lossy().into_owned()
        } else {
            let resolved = fs::canonicalize(&cwd)
                .map_err(|_| "The selected terminal folder is unavailable.")?;
            if !resolved.starts_with(&workspace_root) {
                return Err("The selected terminal folder is outside this workspace.".into());
            }
            resolved.to_string_lossy().into_owned()
        };
        let shell = match profile.kind.as_str() {
            "bash" => Some(LocalShellKind::Bash),
            "zsh" => Some(LocalShellKind::Zsh),
            "fish" => Some(LocalShellKind::Fish),
            "sh" => Some(LocalShellKind::Sh),
            "pwsh" | "powershell" => Some(LocalShellKind::PowerShell),
            "cmd" => Some(LocalShellKind::Cmd),
            _ => None,
        };
        let (executable, args, integration) = if let Some(distro) = &profile.distro {
            let shell = shell.unwrap_or(LocalShellKind::Other);
            (
                lomi_terminal::shell::wsl_launcher(),
                Vec::new(),
                Some(LocalShellIntegration::WslV1 {
                    distro: lomi_remote_protocol::ipc::LocalProjectName::try_from(distro.clone())
                        .map_err(|_| "The selected WSL distribution is invalid.")?,
                    shell,
                    shell_executable: LocalRootPath::try_from(profile.program.clone())
                        .map_err(|_| "The selected WSL shell path is invalid.")?,
                    home: LocalRootPath::try_from(profile.home.clone())
                        .map_err(|_| "The selected WSL home path is invalid.")?,
                }),
            )
        } else if let Some(shell) = shell {
            let original_zdotdir = if shell == LocalShellKind::Zsh {
                std::env::var_os("ZDOTDIR")
                    .and_then(|value| value.into_string().ok())
                    .map(|value| {
                        LocalRootPath::try_from(value)
                            .map_err(|_| "The selected Zsh startup directory is invalid.")
                    })
                    .transpose()?
            } else {
                None
            };
            (
                PathBuf::from(&profile.program),
                Vec::new(),
                Some(LocalShellIntegration::NativeV1 {
                    shell,
                    home: LocalRootPath::try_from(profile.home.clone())
                        .map_err(|_| "The selected shell home path is invalid.")?,
                    original_zdotdir,
                }),
            )
        } else {
            // Unknown native profiles keep their exact executable and use the
            // same generic interactive launch as the former Desktop builder.
            (PathBuf::from(&profile.program), vec!["-i".to_owned()], None)
        };
        let executable = LocalRootPath::try_from(
            executable
                .to_str()
                .ok_or("The selected shell executable is not valid UTF-8.")?
                .to_owned(),
        )
        .map_err(|_| "The selected shell executable path is invalid.")?;
        let args = BoundedList::<LocalProcessArgument, 32>(
            args.into_iter()
                .map(LocalProcessArgument::try_from)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "The selected shell arguments are invalid.")?,
        );
        let cwd = LocalRootPath::try_from(cwd)
            .map_err(|_| "The selected terminal folder path is invalid.")?;
        let workspace = match host.request(
            &view,
            LocalCall::WorkspaceInspect(LocalWorkspaceRead { workspace_id }),
        )? {
            LocalResult::WorkspaceState(workspace) => *workspace,
            _ => return Err("Local host returned an invalid workspace state response.".into()),
        };
        let mut registration = LocalShellProfileRegister::new(
            workspace_id,
            lomi_remote_protocol::ipc::LocalProjectName::try_from(String::from(workspace.name))
                .map_err(|_| "The workspace name is invalid.")?,
            executable,
            args,
            cwd,
            BoundedList(Vec::new()),
        )
        .map_err(|_| "The selected shell profile is invalid.")?;
        if let Some(integration) = integration {
            registration = registration
                .with_integration(integration)
                .map_err(|_| "The selected shell profile is invalid.")?;
        }
        let profile_id = registration.profile_id.clone();
        match host.request(&view, LocalCall::ShellProfileRegister(registration))? {
            LocalResult::ShellProfileRegistered {
                profile_id: registered,
            } if registered == profile_id => Ok(profile_id.into()),
            LocalResult::ShellProfileRegistered { .. } => {
                Err("Local host registered a different shell profile.".into())
            }
            _ => Err("Local host returned an invalid shell profile response.".into()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn shared_host_terminal_aliases(
    window: Window,
    webview: Webview,
    app: tauri::AppHandle,
    state: State<'_, SharedHost>,
    workspace_id: String,
) -> Result<Vec<DesktopTerminalAlias>, String> {
    trusted_full_window(&window, &webview)?;
    let workspace_id = lomi_remote_protocol::WorkspaceId::try_from(workspace_id)
        .map_err(|_| "The desktop workspace ID is invalid.")?;
    state.inner().terminal_aliases(&app, &workspace_id)
}

#[tauri::command]
pub async fn shared_host_save_terminal_alias(
    window: Window,
    webview: Webview,
    app: tauri::AppHandle,
    state: State<'_, SharedHost>,
    workspace_id: String,
    desktop_pane_id: String,
    desktop_tab_id: String,
    host_pane_id: String,
    host_tab_id: String,
) -> Result<(), String> {
    trusted_full_window(&window, &webview)?;
    let workspace_id = lomi_remote_protocol::WorkspaceId::try_from(workspace_id)
        .map_err(|_| "The desktop workspace ID is invalid.")?;
    lomi_remote_protocol::PaneId::try_from(desktop_pane_id.clone())
        .and_then(|_| lomi_remote_protocol::PaneId::try_from(host_pane_id.clone()))
        .map_err(|_| "The terminal pane ID is invalid.")?;
    lomi_remote_protocol::TabId::try_from(desktop_tab_id.clone())
        .and_then(|_| lomi_remote_protocol::TabId::try_from(host_tab_id.clone()))
        .map_err(|_| "The terminal tab ID is invalid.")?;
    state.inner().save_terminal_alias(
        &app,
        &workspace_id,
        DesktopTerminalAlias {
            desktop_pane_id,
            desktop_tab_id,
            host_pane_id,
            host_tab_id,
        },
    )
}

fn find_host_terminal(
    layout: &lomi_remote_protocol::rpc::TerminalLayout,
    pane_id: &lomi_remote_protocol::PaneId,
) -> Option<lomi_remote_protocol::rpc::TerminalPane> {
    use lomi_remote_protocol::rpc::TerminalLayout;
    match layout {
        TerminalLayout::Pane { pane } if &pane.pane_id == pane_id => Some(pane.clone()),
        TerminalLayout::Pane { .. } => None,
        TerminalLayout::Split { first, second, .. } => {
            find_host_terminal(first, pane_id).or_else(|| find_host_terminal(second, pane_id))
        }
    }
}

fn host_rpc_request(request_id: String, call: RpcCall) -> Result<RpcRequest, String> {
    Ok(RpcRequest {
        v: lomi_remote_protocol::ApplicationMajor::try_from(2)
            .map_err(|_| "The local host protocol version is invalid.")?,
        r#type: lomi_remote_protocol::rpc::RequestKind::Request,
        request_id: lomi_remote_protocol::RequestId::try_from(request_id)
            .map_err(|_| "The terminal operation request ID is invalid.")?,
        timeout_ms: lomi_remote_protocol::TimeoutMs::try_from(30_000u64)
            .map_err(|_| "The local host request timeout is invalid.")?,
        call,
    })
}

fn close_host_terminal(
    host: &SharedHost,
    app: &tauri::AppHandle,
    view: &str,
    workspace_id: lomi_remote_protocol::WorkspaceId,
    desktop_pane_id: String,
    operation_id: String,
    request_id: String,
    status_request_id: String,
) -> Result<(), String> {
    let operation_id = lomi_remote_protocol::OperationId::try_from(operation_id)
        .map_err(|_| "The terminal operation ID is invalid.")?;
    let requested_id = workspace_id.clone();
    let aliases = host.terminal_aliases(app, &workspace_id)?;
    let host_pane_id = aliases
        .iter()
        .find(|alias| alias.desktop_pane_id == desktop_pane_id)
        .map(|alias| alias.host_pane_id.as_str())
        .unwrap_or(&desktop_pane_id);
    let host_pane_id = lomi_remote_protocol::PaneId::try_from(host_pane_id.to_owned())
        .map_err(|_| "The host terminal pane ID is invalid.")?;
    let canonical_id = match host.canonical_workspace_id(app, &workspace_id) {
        Ok(workspace_id) => workspace_id,
        Err(_) if aliases.is_empty() => return Ok(()),
        Err(error) => return Err(error),
    };
    let workspace = match host.request(
        view,
        LocalCall::WorkspaceInspect(LocalWorkspaceRead {
            workspace_id: canonical_id,
        }),
    )? {
        LocalResult::WorkspaceState(workspace) => *workspace,
        _ => return Err("Local host returned an invalid workspace state response.".into()),
    };
    let terminal = workspace
        .tabs
        .0
        .iter()
        .find_map(|tab| find_host_terminal(&tab.layout, &host_pane_id));
    let Some(terminal) = terminal else {
        host.remove_terminal_alias(app, &requested_id, &desktop_pane_id)?;
        return Ok(());
    };
    let mutation = lomi_remote_protocol::MutationContext {
        operation_id: operation_id.clone(),
        context: workspace.context.clone(),
        expected_revision: workspace.domain_revision.clone(),
    };
    let call = RpcCall::TerminalClose(lomi_remote_protocol::rpc::TerminalCloseParams {
        mutation,
        pane_id: host_pane_id,
        expected_session: terminal.session,
    });
    let request = host_rpc_request(request_id, call)?;
    let first_error = match host.request_rpc(view, request, None) {
        Ok(RpcResponse::Result {
            result: RpcResult::TerminalClose(_),
            ..
        }) => None,
        Ok(RpcResponse::Error { .. }) => {
            Some("The local host did not confirm terminal close.".to_owned())
        }
        Ok(_) => Some("Local host returned an invalid terminal close response.".to_owned()),
        Err(error) => Some(error),
    };
    if let Some(first_error) = first_error {
        let status = host_rpc_request(
            status_request_id,
            RpcCall::OperationStatus(lomi_remote_protocol::OperationStatusParams {
                context: workspace.context,
                operation_id: operation_id.clone(),
            }),
        )?;
        let confirmed = matches!(
            host.request_rpc(view, status, None),
            Ok(RpcResponse::Result {
                result: RpcResult::OperationStatus(receipt),
                ..
            }) if receipt.operation_id() == operation_id
                && receipt.stored_rights().original_method() == Method::TerminalClose
                && receipt.state() == lomi_remote_protocol::OperationState::Succeeded
        );
        if !confirmed {
            return Err(format!(
                "The terminal close could not be confirmed. {first_error}"
            ));
        }
    }
    host.remove_terminal_alias(app, &requested_id, &desktop_pane_id)
}

#[tauri::command]
pub async fn shared_host_close_terminal(
    window: Window,
    webview: Webview,
    app: tauri::AppHandle,
    state: State<'_, SharedHost>,
    workspace_id: String,
    desktop_pane_id: String,
    operation_id: String,
    request_id: String,
    status_request_id: String,
) -> Result<(), String> {
    trusted_full_window(&window, &webview)?;
    let workspace_id = lomi_remote_protocol::WorkspaceId::try_from(workspace_id)
        .map_err(|_| "The desktop workspace ID is invalid.")?;
    lomi_remote_protocol::PaneId::try_from(desktop_pane_id.clone())
        .map_err(|_| "The desktop terminal pane ID is invalid.")?;
    let host = state.inner().clone();
    let app = app.clone();
    let view = webview.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        close_host_terminal(
            &host,
            &app,
            &view,
            workspace_id,
            desktop_pane_id,
            operation_id,
            request_id,
            status_request_id,
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn shared_host_import_workspace(
    window: Window,
    webview: Webview,
    app: tauri::AppHandle,
    state: State<'_, SharedHost>,
    workspace_id: String,
    name: String,
    root_path: String,
) -> Result<lomi_remote_protocol::ipc::LocalWorkspaceImportReceipt, String> {
    trusted_full_window(&window, &webview)?;
    let host = state.inner().clone();
    let app = app.clone();
    let view = webview.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let workspace_id = lomi_remote_protocol::WorkspaceId::try_from(workspace_id)
            .map_err(|_| "The desktop workspace ID is invalid.")?;
        let name = lomi_remote_protocol::ipc::LocalProjectName::try_from(name)
            .map_err(|_| "The workspace name is invalid.")?;
        let root_path =
            fs::canonicalize(root_path).map_err(|_| "The workspace folder is unavailable.")?;
        if !root_path.is_dir() {
            return Err("The workspace folder is unavailable.".into());
        }
        host.bind_workspace(&app, &workspace_id, &root_path)?;
        let root_path = root_path
            .into_os_string()
            .into_string()
            .map_err(|_| "The workspace folder path is not valid UTF-8.")?;
        let root_path = lomi_remote_protocol::ipc::LocalRootPath::try_from(root_path)
            .map_err(|_| "The workspace folder path is invalid.")?;
        let import_id = workspace_id.clone();
        let response = host.request(
            &view,
            LocalCall::WorkspaceImport(LocalWorkspaceImport {
                workspace_id,
                name,
                root_path,
                operator_intent: Confirmed::Confirmed,
            }),
        )?;
        match response {
            LocalResult::WorkspaceImported(receipt) if receipt.import_id == import_id => {
                host.save_canonical_workspace_id(
                    &app,
                    &receipt.import_id,
                    &receipt.workspace.workspace_id,
                )?;
                Ok(receipt)
            }
            LocalResult::WorkspaceImported(_) => {
                Err("Local host returned a different workspace import ID.".into())
            }
            _ => Err("Local host returned an invalid workspace response.".into()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn shared_host_rpc(
    window: Window,
    webview: Webview,
    state: State<'_, SharedHost>,
    request: RpcRequest,
    events: tauri::ipc::Channel<SharedHostStreamEvent>,
) -> Result<RpcResponse, String> {
    trusted_full_window(&window, &webview)?;
    let host = state.inner().clone();
    let view = webview.label().to_owned();
    let events = matches!(
        &request.call,
        RpcCall::TerminalAttach(_)
            | RpcCall::TerminalSnapshot(_)
            | RpcCall::WorkspaceSubscribe(_)
            | RpcCall::FilesWatch(_)
    )
    .then_some(events);
    tauri::async_runtime::spawn_blocking(move || host.request_rpc(&view, request, events))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn shared_host_validate_workspace_frame(
    window: Window,
    webview: Webview,
    json: String,
) -> Result<(), String> {
    trusted_full_window(&window, &webview)?;
    lomi_remote_protocol::decode_workspace_frame(json.as_bytes())
        .map(|_| ())
        .map_err(|_| "The local workspace stream frame is invalid.".to_owned())
}

#[tauri::command]
pub async fn shared_host_stream_ack(
    window: Window,
    webview: Webview,
    state: State<'_, SharedHost>,
    stream: String,
    frame: serde_json::Value,
) -> Result<(), String> {
    trusted_full_window(&window, &webview)?;
    let (ack, subscription_id, kind) = match stream.as_str() {
        "workspace" => {
            let ack: lomi_remote_protocol::streams::WorkspaceStreamAck =
                serde_json::from_value(frame)
                    .map_err(|_| "The workspace stream acknowledgement is invalid.")?;
            let subscription_id = ack.subscription_id.as_str().to_owned();
            (
                LocalStreamAck::Workspace(ack),
                subscription_id,
                SharedHostStreamKind::Workspace,
            )
        }
        "files" => {
            let ack: lomi_remote_protocol::streams::FileWatchAck = serde_json::from_value(frame)
                .map_err(|_| "The file stream acknowledgement is invalid.")?;
            let subscription_id = ack.subscription_id.as_str().to_owned();
            (
                LocalStreamAck::Files(ack),
                subscription_id,
                SharedHostStreamKind::Files,
            )
        }
        _ => return Err("The local stream kind is unavailable.".into()),
    };
    let view = webview.label().to_owned();
    let stream_is_active = state
        .streams
        .lock()
        .map_err(|_| "Local host stream state is unavailable.")?
        .get(&(view.clone(), subscription_id.clone()))
        .is_some_and(|active| active.kind == kind);
    if !stream_is_active {
        return Err("The local stream subscription is no longer active.".into());
    }
    let host = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        match host.request(&view, LocalCall::StreamAck(ack))? {
            LocalResult::StreamAcknowledged(acknowledged)
                if acknowledged.as_str() == subscription_id =>
            {
                Ok(())
            }
            _ => Err("Local host returned an invalid stream acknowledgement.".into()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

pub fn canonical_host_entry(args: &[std::ffi::OsString]) -> Option<i32> {
    use std::ffi::OsStr;
    let is =
        |index: usize, value: &str| args.get(index).is_some_and(|arg| arg == OsStr::new(value));

    #[cfg(unix)]
    if is(0, "--internal-pty-worker") {
        if args.len() != 1 || lomi_platform::process::initialize_child_ownership().is_err() {
            return Some(4);
        }
        return Some(lomi_terminal::internal_pty_worker());
    }

    #[cfg(windows)]
    if is(0, "--internal-scm") {
        let (Some(name), Some(profile)) = (args.get(1), args.get(2)) else {
            return Some(2);
        };
        if args.len() != 3 {
            return Some(2);
        }
        return Some(
            if lomi_host::windows_service::dispatch(name.clone(), Path::new(profile).to_path_buf())
                .is_ok()
            {
                0
            } else {
                4
            },
        );
    }

    if is(0, "--profile-dir") && is(2, "daemon") && is(3, "run") {
        let mut expected_host_id = None;
        let mut existing_profile = false;
        let mut next = 4;
        while next < args.len() {
            if is(next, "--existing-profile") && !existing_profile {
                existing_profile = true;
                next += 1;
            } else if is(next, "--expected-host-id") && expected_host_id.is_none() {
                let Some(id) = args
                    .get(next + 1)
                    .and_then(|id| id.to_str())
                    .and_then(|id| lomi_remote_protocol::HostId::try_from(id.to_owned()).ok())
                else {
                    return Some(2);
                };
                expected_host_id = Some(id);
                next += 2;
            } else {
                return Some(2);
            }
        }
        #[cfg(unix)]
        if lomi_platform::process::initialize_child_ownership().is_err() {
            return Some(4);
        }
        let Some(profile) = args.get(1) else {
            return Some(2);
        };
        let result = match expected_host_id {
            Some(id) => lomi_host::run_bound(Path::new(profile), id),
            None if existing_profile => lomi_host::run_existing(Path::new(profile)),
            None => lomi_host::run(Path::new(profile)),
        };
        return Some(match result {
            Ok(()) => 0,
            Err(lomi_host::HostError::NonRootAccountRequired) => {
                eprintln!(
                    "NONROOT_HOST_ACCOUNT_REQUIRED: select a dedicated non-root operating-system account and run setup as that account"
                );
                3
            }
            Err(error) => {
                eprintln!("LOCAL_HOST_FAILED: {error}");
                4
            }
        });
    }

    None
}
