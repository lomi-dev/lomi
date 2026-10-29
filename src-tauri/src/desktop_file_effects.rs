//! Owner-local Explorer mutations executed by the canonical Host.
//!
//! The app stages only a typed, bounded request. Once Execute might have been
//! sent, recovery uses Host status and never resends the mutation.

use lomi_files::desktop::{
    DesktopFileChange, DesktopFileErrorCode, DesktopFileOperation, DesktopGitChange,
    DesktopMutationError, DesktopMutationRequest, DesktopTrashError, DesktopTrashTicket,
};
use lomi_remote_protocol::{
    ipc::{
        LocalCall, LocalError, LocalNativeEditorEffect, LocalNativeEditorExecute,
        LocalNativeEditorResult, LocalNativeEffectState, LocalNativeFileBlob, LocalNativeFileStage,
        LocalNativeResultJson, LocalResult, LocalRootPath, NativeEditorRetirement, NativeIntentId,
        NativeIntentTarget,
    },
    DecimalU64, HostKeyCommitmentHex, TimeoutMs,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::Manager;

use crate::{
    files::operations::{FileChange, Operation},
    shared_host::{self, DesktopWorkspaceBindingSnapshot},
};

const OUTBOX_LIMIT_BYTES: u64 = 16 * 1024 * 1024;
const OUTBOX_LIMIT_ENTRIES: usize = 64;
const UNCERTAINTY_WARNING_LIMIT: usize = 256;
const REQUEST_LIMIT_BYTES: usize = 1024 * 1024;
const RESULT_LIMIT_BYTES: usize = 131_072;
const RECOVERY_BATCH: usize = 4;
const RECOVERY_RETRY: Duration = Duration::from_secs(1);
const STATUS_POLLS: usize = 4;
const STATUS_INTERVAL: Duration = Duration::from_millis(200);
const MUTATION_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum DesktopPhase {
    Staging,
    ExecuteSent,
    Terminal,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum DesktopReceipt {
    Succeeded(DesktopFileChange),
    Failed(DesktopFileErrorCode),
    FailedMessage(String),
    OutcomeUnknown,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DesktopUnknownDisposition {
    host_id: String,
    runtime_id: String,
    retirement: NativeEditorRetirement,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DesktopUncertaintyWarningRecord {
    operation_id: NativeIntentId,
    request_hash: String,
    created_at_ms: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DesktopUncertaintyWarning {
    operation_id: String,
    request_hash: String,
    message: String,
    created_at_ms: u64,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DesktopOutboxEntry {
    operation_id: NativeIntentId,
    request_hash: String,
    request: DesktopMutationRequest,
    expected_source_path: Option<PathBuf>,
    source_is_directory: Option<bool>,
    phase: DesktopPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    receipt: Option<DesktopReceipt>,
    #[serde(default)]
    acknowledgement_pending: bool,
    #[serde(default)]
    acknowledged: bool,
    #[serde(default)]
    binding_effects_settled: bool,
    #[serde(default)]
    uncertainty_warning_acknowledged: bool,
    #[serde(default)]
    trash_transfer_attempted: bool,
    #[serde(default)]
    trash_transfer_complete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    binding_snapshot: Option<Vec<DesktopWorkspaceBindingSnapshot>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unknown_disposition: Option<DesktopUnknownDisposition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_recovery_attempt_ms: Option<u64>,
    created_at_ms: u64,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DesktopOutbox {
    version: u32,
    operations: BTreeMap<String, DesktopOutboxEntry>,
    #[serde(default)]
    uncertainty_warnings: BTreeMap<String, DesktopUncertaintyWarningRecord>,
}

#[derive(Default)]
struct RecoverySummary {
    warning: Option<String>,
}

fn revision(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn operation_key(operation_id: NativeIntentId) -> String {
    String::from(operation_id)
}

fn new_operation_id() -> Result<NativeIntentId, String> {
    use ring::rand::SecureRandom;
    let mut bytes = [0; 16];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "Secure randomness is unavailable for the file operation.")?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    NativeIntentId::from_bytes(bytes).map_err(|_| "The file operation ID is invalid.".into())
}

fn app_data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let path = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    crate::chat::storage::reject_link(&path)?;
    crate::chat::storage::private(&path, true)?;
    fs::canonicalize(&path).map_err(|error| error.to_string())
}

fn outbox_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join("desktop-file-outbox.json"))
}

fn path_overlaps(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn canonical_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| "The selected path cannot be represented as UTF-8.".into())
}

fn canonical_relative(root: &Path, path: &Path) -> Result<String, String> {
    canonical_text(
        path.strip_prefix(root)
            .map_err(|_| "The selected path is outside the project root.")?,
    )
}

fn request_for_operation(
    app: &tauri::AppHandle,
    root: &str,
    relative: &str,
    operation_id: NativeIntentId,
    operation: Operation,
    expected_path: Option<String>,
) -> Result<(DesktopMutationRequest, Option<PathBuf>, Option<bool>), String> {
    let root = crate::files::directory(root)?;
    crate::files::operations::validate_relative(relative)?;
    let control_root = app_data_dir(app)?;
    if path_overlaps(&root, &control_root) {
        return Err("Desktop file operations cannot target Lomi's private app data.".into());
    }

    let host_operation = match operation {
        Operation::NewFile { name } => {
            lomi_files::desktop::validate_desktop_name(&name).map_err(desktop_error_message)?;
            DesktopFileOperation::NewFile { name }
        }
        Operation::NewFolder { name } => {
            lomi_files::desktop::validate_desktop_name(&name).map_err(desktop_error_message)?;
            DesktopFileOperation::NewFolder { name }
        }
        Operation::Rename { name } => {
            lomi_files::desktop::validate_desktop_name(&name).map_err(desktop_error_message)?;
            DesktopFileOperation::Rename { name }
        }
        Operation::Copy {
            source_root,
            source,
        } => {
            crate::files::operations::validate_relative(&source)?;
            if source.is_empty() {
                return Err("Copy or move items inside the project.".into());
            }
            let source_root = crate::files::directory(&source_root)?;
            if path_overlaps(&source_root, &control_root) {
                return Err("Lomi's private app data cannot be used as a copy source.".into());
            }
            let source_path =
                crate::files::operations::entry_path(&canonical_text(&source_root)?, &source)?;
            let source = canonical_relative(&source_root, &source_path)?;
            DesktopFileOperation::Copy {
                source_root,
                source,
            }
        }
        Operation::Move {
            source_root,
            source,
        } => {
            crate::files::operations::validate_relative(&source)?;
            if source.is_empty() {
                return Err("Copy or move items inside the project.".into());
            }
            let source_root = crate::files::directory(&source_root)?;
            if path_overlaps(&source_root, &control_root) {
                return Err("Lomi's private app data cannot be used as a move source.".into());
            }
            let source_path =
                crate::files::operations::entry_path(&canonical_text(&source_root)?, &source)?;
            let source = canonical_relative(&source_root, &source_path)?;
            DesktopFileOperation::Move {
                source_root,
                source,
            }
        }
        Operation::Duplicate => DesktopFileOperation::Duplicate,
        Operation::Trash => DesktopFileOperation::Trash,
        Operation::Delete => DesktopFileOperation::Delete,
    };

    let selected_path = match &host_operation {
        DesktopFileOperation::NewFile { .. }
        | DesktopFileOperation::NewFolder { .. }
        | DesktopFileOperation::Copy { .. }
        | DesktopFileOperation::Move { .. } => {
            crate::files::inside(&canonical_text(&root)?, relative)?
        }
        DesktopFileOperation::Rename { .. }
        | DesktopFileOperation::Duplicate
        | DesktopFileOperation::Trash
        | DesktopFileOperation::Delete => {
            crate::files::operations::entry_path(&canonical_text(&root)?, relative)?
        }
        DesktopFileOperation::GitDiscard { .. } | DesktopFileOperation::GitIgnore { .. } => {
            return Err("Git actions must use their bound workspace request.".into());
        }
    };
    if let Some(expected) = expected_path {
        if selected_path != Path::new(&expected) {
            return Err("The selected path changed. Please try the operation again.".into());
        }
    }

    let expected_source_path = match &host_operation {
        DesktopFileOperation::Copy {
            source_root,
            source,
        }
        | DesktopFileOperation::Move {
            source_root,
            source,
        } => Some(crate::files::operations::entry_path(
            &canonical_text(source_root)?,
            source,
        )?),
        DesktopFileOperation::Duplicate => Some(selected_path.clone()),
        _ => None,
    };
    let source_is_directory = expected_source_path
        .as_ref()
        .map(|path| fs::symlink_metadata(path).map(|meta| meta.is_dir()))
        .transpose()
        .map_err(|error| error.to_string())?;

    let relative_path = canonical_relative(&root, &selected_path)?;
    let request = DesktopMutationRequest {
        version: 1,
        operation_id: operation_key(operation_id),
        root,
        relative_path,
        expected_path: Some(selected_path),
        control_roots: vec![control_root],
        operation: host_operation,
    };
    lomi_files::desktop::validate_desktop_request(&request).map_err(desktop_error_message)?;
    Ok((request, expected_source_path, source_is_directory))
}

fn expected_result_paths(
    request: &DesktopMutationRequest,
    expected_source_path: Option<&Path>,
    source_is_directory: Option<bool>,
    result: &DesktopFileChange,
) -> Result<(), String> {
    if result.operation_id != request.operation_id {
        return Err("The Host returned a mismatched desktop operation ID.".into());
    }
    let selected = request
        .expected_path
        .as_deref()
        .ok_or("The Host request did not retain its selected path.")?;
    let (expected_old, expected_new, must_have_ticket) = match &request.operation {
        DesktopFileOperation::NewFile { name } | DesktopFileOperation::NewFolder { name } => {
            (None, Some(selected.join(name)), false)
        }
        DesktopFileOperation::Rename { name } => (
            Some(selected.to_path_buf()),
            Some(
                selected
                    .parent()
                    .ok_or("The selected item has no parent directory.")?
                    .join(name),
            ),
            false,
        ),
        DesktopFileOperation::Copy { .. } => (
            None,
            Some(
                selected.join(
                    expected_source_path
                        .and_then(Path::file_name)
                        .ok_or("The copy source path is invalid.")?,
                ),
            ),
            false,
        ),
        DesktopFileOperation::Move { .. } => (
            expected_source_path.map(Path::to_path_buf),
            Some(
                selected.join(
                    expected_source_path
                        .and_then(Path::file_name)
                        .ok_or("The move source path is invalid.")?,
                ),
            ),
            false,
        ),
        DesktopFileOperation::Duplicate => {
            let source = selected;
            let parent = source
                .parent()
                .ok_or("The selected item has no parent directory.")?;
            let leaf = source
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or("The selected filename is invalid.")?;
            let is_dir = source_is_directory.unwrap_or(false);
            let actual = result
                .new_path
                .as_deref()
                .ok_or("The Host omitted the duplicate destination path.")?;
            let actual = Path::new(actual);
            let actual_leaf = actual
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or("The Host returned an invalid duplicate name.")?;
            let matches_copy_name =
                (1..=lomi_files::desktop::DESKTOP_DUPLICATE_LIMIT).any(|number| {
                    lomi_files::desktop::desktop_duplicate_name(leaf, is_dir, number)
                        .is_ok_and(|candidate| candidate == actual_leaf)
                });
            if result.old_path.is_some() || actual.parent() != Some(parent) || !matches_copy_name {
                return Err(
                    "The Host returned a duplicate path outside the selected directory.".into(),
                );
            }
            return validate_ticket(request, result, false);
        }
        DesktopFileOperation::Trash => (Some(selected.to_path_buf()), None, true),
        DesktopFileOperation::Delete => (Some(selected.to_path_buf()), None, false),
        DesktopFileOperation::GitDiscard { expected, .. }
            if expected.index == '?' && expected.worktree == '?' =>
        {
            (Some(selected.to_path_buf()), None, true)
        }
        DesktopFileOperation::GitDiscard { .. } | DesktopFileOperation::GitIgnore { .. } => {
            (None, None, false)
        }
    };
    if result.old_path.as_deref().map(Path::new) != expected_old.as_deref()
        || result.new_path.as_deref().map(Path::new) != expected_new.as_deref()
    {
        return Err("The Host returned desktop paths that do not match the request.".into());
    }
    validate_ticket(request, result, must_have_ticket)
}

fn project_write_scopes(entry: &DesktopOutboxEntry) -> Result<Vec<PathBuf>, String> {
    let selected = entry
        .request
        .expected_path
        .as_deref()
        .ok_or("The desktop file request did not retain its selected path.")?;
    let mut scopes = Vec::with_capacity(3);
    match &entry.request.operation {
        DesktopFileOperation::NewFile { name } | DesktopFileOperation::NewFolder { name } => {
            scopes.push(selected.join(name));
        }
        DesktopFileOperation::Rename { name } => {
            scopes.push(selected.to_path_buf());
            scopes.push(
                selected
                    .parent()
                    .ok_or("The selected item has no parent directory.")?
                    .join(name),
            );
        }
        DesktopFileOperation::Copy { .. } => {
            let source = entry
                .expected_source_path
                .as_deref()
                .and_then(Path::file_name)
                .ok_or("The copy source path is invalid.")?;
            scopes.push(selected.join(source));
            scopes.push(selected.to_path_buf());
        }
        DesktopFileOperation::Move { .. } => {
            scopes.push(
                entry
                    .expected_source_path
                    .clone()
                    .ok_or("The move source path is invalid.")?,
            );
            let source = entry
                .expected_source_path
                .as_deref()
                .and_then(Path::file_name)
                .ok_or("The move source path is invalid.")?;
            scopes.push(selected.join(source));
        }
        DesktopFileOperation::Duplicate => {
            scopes.push(
                selected
                    .parent()
                    .ok_or("The selected item has no parent directory.")?
                    .to_path_buf(),
            );
        }
        DesktopFileOperation::Trash
        | DesktopFileOperation::Delete
        | DesktopFileOperation::GitDiscard { .. } => {
            scopes.push(selected.to_path_buf());
        }
        DesktopFileOperation::GitIgnore {
            repository_relative,
            ..
        } => {
            // Includes Git metadata as well as the worktree ignore file. The
            // Host derives and checks a linked worktree's actual common dir.
            scopes.push(entry.request.root.join(repository_relative));
        }
    }
    Ok(scopes)
}

fn binding_snapshot_base(entry: &DesktopOutboxEntry) -> Option<&Path> {
    let selected = entry.request.expected_path.as_deref()?;
    match &entry.request.operation {
        DesktopFileOperation::Rename { .. }
        | DesktopFileOperation::Trash
        | DesktopFileOperation::Delete => Some(selected),
        DesktopFileOperation::Move { .. } => entry.expected_source_path.as_deref(),
        DesktopFileOperation::GitDiscard { expected, .. }
            if expected.index == '?' && expected.worktree == '?' =>
        {
            Some(selected)
        }
        _ => None,
    }
}

fn binding_snapshot_scope(entry: &DesktopOutboxEntry) -> Result<Option<(PathBuf, bool)>, String> {
    let Some(path) = binding_snapshot_base(entry) else {
        return Ok(None);
    };
    let include_descendants = match &entry.request.operation {
        DesktopFileOperation::Move { .. } => entry.source_is_directory.unwrap_or(false),
        _ => match fs::symlink_metadata(path) {
            Ok(metadata) => metadata.is_dir(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.to_string()),
        },
    };
    Ok(Some((path.to_path_buf(), include_descendants)))
}

fn validate_binding_snapshot(entry: &DesktopOutboxEntry) -> Result<(), String> {
    let Some(snapshots) = &entry.binding_snapshot else {
        return Ok(());
    };
    let base = binding_snapshot_base(entry)
        .ok_or("The desktop file outbox has a binding snapshot for an unrelated operation.")?;
    let mut workspace_ids = HashSet::new();
    for snapshot in snapshots {
        lomi_remote_protocol::WorkspaceId::try_from(snapshot.workspace_id.clone())
            .map_err(|_| "The desktop file outbox binding snapshot ID is invalid.")?;
        lomi_remote_protocol::WorkspaceId::try_from(snapshot.canonical_workspace_id.clone())
            .map_err(|_| "The desktop file outbox canonical binding snapshot ID is invalid.")?;
        let root_path = Path::new(&snapshot.root_path);
        if snapshot.generation == 0
            || !root_path.is_absolute()
            || (root_path != base && !root_path.starts_with(base))
            || !workspace_ids.insert(&snapshot.workspace_id)
        {
            return Err("The desktop file outbox binding snapshot is invalid.".into());
        }
    }
    Ok(())
}

fn binding_effect_digest(
    entry: &DesktopOutboxEntry,
    change: &DesktopFileChange,
) -> Result<String, String> {
    let receipt = serde_json::to_vec(change)
        .map_err(|_| "The desktop file receipt could not be fingerprinted.")?;
    let mut material = Vec::with_capacity(entry.request_hash.len() + receipt.len() + 1);
    material.extend_from_slice(entry.request_hash.as_bytes());
    material.push(0);
    material.extend_from_slice(&receipt);
    Ok(revision(&material))
}

fn validate_ticket(
    request: &DesktopMutationRequest,
    result: &DesktopFileChange,
    required: bool,
) -> Result<(), String> {
    match (&request.operation, &result.trash) {
        (DesktopFileOperation::Trash | DesktopFileOperation::GitDiscard { .. }, Some(ticket))
            if required =>
        {
            ticket.validate_shape().map_err(desktop_error_message)?;
            if ticket.operation_id != request.operation_id
                || ticket.root != request.root
                || Some(ticket.original_path.as_path()) != result.old_path.as_deref().map(Path::new)
            {
                return Err("The Host returned an invalid desktop trash ticket.".into());
            }
            Ok(())
        }
        (DesktopFileOperation::Trash, None) | (_, Some(_)) => {
            Err("The Host returned an unexpected desktop trash ticket.".into())
        }
        (_, None) if !required => Ok(()),
        _ => Err("The Host omitted the desktop trash ticket.".into()),
    }
}

fn outbox_lock<T>(path: &Path, action: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    static LOCAL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _local = LOCAL_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "Desktop file operation state is unavailable.")?;
    let parent = path
        .parent()
        .ok_or("The desktop file outbox has no parent directory.")?;
    let lock_path = parent.join("desktop-file-outbox.lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
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
    let file = options
        .open(&lock_path)
        .map_err(|_| "The desktop file outbox lock is unavailable.")?;
    let metadata = file
        .metadata()
        .map_err(|_| "The desktop file outbox lock is unavailable.")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("The desktop file outbox lock is invalid.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.mode() & 0o077 != 0
        {
            return Err("The desktop file outbox lock is not owner-private.".into());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("The desktop file outbox lock is invalid.".into());
        }
    }
    crate::chat::storage::private(&lock_path, false)
        .map_err(|_| "The desktop file outbox lock is not owner-private.")?;
    file.lock()
        .map_err(|_| "The desktop file outbox lock could not be acquired.")?;
    action()
}

fn read_outbox(path: &Path) -> Result<DesktopOutbox, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DesktopOutbox {
                version: 1,
                operations: BTreeMap::new(),
                uncertainty_warnings: BTreeMap::new(),
            })
        }
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > OUTBOX_LIMIT_BYTES
    {
        return Err("The desktop file outbox is invalid or too large.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.mode() & 0o077 != 0
        {
            return Err("The desktop file outbox is not owner-private.".into());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("The desktop file outbox is invalid.".into());
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
        return Err("The desktop file outbox changed while it was opened.".into());
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
            return Err("The desktop file outbox changed while it was opened.".into());
        }
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    Read::by_ref(&mut file)
        .take(OUTBOX_LIMIT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > OUTBOX_LIMIT_BYTES {
        return Err("The desktop file outbox is too large.".into());
    }
    let mut outbox: DesktopOutbox =
        serde_json::from_slice(&bytes).map_err(|_| "The desktop file outbox is invalid.")?;
    if outbox.version != 1
        || outbox.operations.len() > OUTBOX_LIMIT_ENTRIES
        || outbox.uncertainty_warnings.len() > UNCERTAINTY_WARNING_LIMIT
    {
        return Err("The desktop file outbox has an unsupported version or size.".into());
    }
    for (key, warning) in &outbox.uncertainty_warnings {
        if operation_key(warning.operation_id) != *key
            || warning.request_hash.len() != 64
            || !warning
                .request_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || warning.created_at_ms == 0
        {
            return Err("The desktop file uncertainty warning ledger is invalid.".into());
        }
    }
    for (key, entry) in &outbox.operations {
        let payload = serde_json::to_vec(&entry.request)
            .map_err(|_| "The desktop file outbox request is invalid.")?;
        let ticket_pending = entry.receipt.as_ref().is_some_and(|receipt| {
            matches!(receipt, DesktopReceipt::Succeeded(change)
                if change.trash.is_some() && !entry.trash_transfer_complete)
        });
        let has_trash_ticket = entry.receipt.as_ref().is_some_and(|receipt| {
            matches!(receipt, DesktopReceipt::Succeeded(change) if change.trash.is_some())
        });
        let source_context_valid = match &entry.request.operation {
            DesktopFileOperation::Copy {
                source_root,
                source: source_relative,
            }
            | DesktopFileOperation::Move {
                source_root,
                source: source_relative,
            } => {
                entry
                    .expected_source_path
                    .as_ref()
                    .is_some_and(|expected_source| {
                        expected_source.is_absolute()
                            && expected_source.starts_with(source_root)
                            && source_root.join(source_relative) == *expected_source
                    })
                    && entry.source_is_directory.is_some()
            }
            DesktopFileOperation::Duplicate => {
                entry.expected_source_path.as_deref() == entry.request.expected_path.as_deref()
                    && entry.source_is_directory.is_some()
            }
            _ => entry.expected_source_path.is_none() && entry.source_is_directory.is_none(),
        };
        if operation_key(entry.operation_id) != *key
            || entry.request.operation_id != *key
            || revision(&payload) != entry.request_hash
            || payload.len() > REQUEST_LIMIT_BYTES
            || entry.created_at_ms == 0
            || !source_context_valid
            || lomi_files::desktop::validate_desktop_request(&entry.request).is_err()
            || (entry.phase == DesktopPhase::Terminal) != entry.receipt.is_some()
            || entry.acknowledged && !entry.acknowledgement_pending
            || entry.acknowledgement_pending && entry.receipt.is_none()
            || entry.receipt.is_some() && !entry.acknowledgement_pending
            || entry.phase == DesktopPhase::Staging
                && (entry.acknowledgement_pending || entry.acknowledged)
            || entry.binding_effects_settled
                && !entry.receipt.as_ref().is_some_and(|receipt| match receipt {
                    DesktopReceipt::Succeeded(_) => entry.binding_snapshot.is_some(),
                    DesktopReceipt::OutcomeUnknown => {
                        entry.acknowledged && entry.unknown_disposition.is_some()
                    }
                    DesktopReceipt::Failed(_) | DesktopReceipt::FailedMessage(_) => false,
                })
            || entry.uncertainty_warning_acknowledged
                && !entry
                    .receipt
                    .as_ref()
                    .is_some_and(|receipt| matches!(receipt, DesktopReceipt::OutcomeUnknown))
            || entry
                .unknown_disposition
                .as_ref()
                .is_some_and(|disposition| {
                    !entry
                        .receipt
                        .as_ref()
                        .is_some_and(|receipt| matches!(receipt, DesktopReceipt::OutcomeUnknown))
                        || disposition.retirement == NativeEditorRetirement::Unresolved
                        || lomi_remote_protocol::HostId::try_from(disposition.host_id.clone())
                            .is_err()
                        || lomi_remote_protocol::RuntimeId::try_from(disposition.runtime_id.clone())
                            .is_err()
                })
            || (entry.trash_transfer_attempted || entry.trash_transfer_complete)
                && !has_trash_ticket
            || entry.trash_transfer_complete && !entry.trash_transfer_attempted
            || entry.trash_transfer_attempted && !ticket_pending && !entry.trash_transfer_complete
            || entry.receipt.as_ref().is_some_and(|receipt| match receipt {
                DesktopReceipt::Succeeded(change) => expected_result_paths(
                    &entry.request,
                    entry.expected_source_path.as_deref(),
                    entry.source_is_directory,
                    change,
                )
                .is_err(),
                DesktopReceipt::Failed(_) => false,
                DesktopReceipt::FailedMessage(message) => message.len() > 8_192,
                DesktopReceipt::OutcomeUnknown => false,
            })
            || validate_binding_snapshot(entry).is_err()
        {
            return Err("The desktop file outbox contains an invalid operation.".into());
        }
    }
    for (key, warning) in &outbox.uncertainty_warnings {
        if outbox.operations.get(key).is_some_and(|entry| {
            entry.operation_id != warning.operation_id || entry.request_hash != warning.request_hash
        }) {
            return Err(
                "The desktop file uncertainty warning conflicts with its operation.".into(),
            );
        }
    }
    outbox.version = 1;
    Ok(outbox)
}

fn write_outbox(path: &Path, outbox: &DesktopOutbox) -> Result<(), String> {
    crate::files::write_json(path, outbox, OUTBOX_LIMIT_BYTES as usize)
}

fn ensure_unknown_warning_record(
    outbox: &mut DesktopOutbox,
    entry: &DesktopOutboxEntry,
) -> Result<(), String> {
    if entry.uncertainty_warning_acknowledged {
        return Ok(());
    }
    let key = operation_key(entry.operation_id);
    if let Some(existing) = outbox.uncertainty_warnings.get(&key) {
        if existing.operation_id != entry.operation_id
            || existing.request_hash != entry.request_hash
        {
            return Err("The desktop uncertainty warning conflicts with its receipt.".into());
        }
        return Ok(());
    }
    if outbox.uncertainty_warnings.len() >= UNCERTAINTY_WARNING_LIMIT {
        return Err("The desktop uncertainty warning ledger is full; acknowledge an existing warning before settling another receipt.".into());
    }
    outbox.uncertainty_warnings.insert(
        key,
        DesktopUncertaintyWarningRecord {
            operation_id: entry.operation_id,
            request_hash: entry.request_hash.clone(),
            created_at_ms: entry.created_at_ms,
        },
    );
    Ok(())
}

fn desktop_uncertainty_warnings(outbox: &DesktopOutbox) -> Vec<DesktopUncertaintyWarning> {
    const MESSAGE: &str = "The Host could not determine whether the desktop file operation completed. Review the affected item before retrying.";
    let mut warnings = BTreeMap::new();
    for (key, warning) in &outbox.uncertainty_warnings {
        warnings.insert(
            key.clone(),
            DesktopUncertaintyWarning {
                operation_id: String::from(warning.operation_id),
                request_hash: warning.request_hash.clone(),
                message: MESSAGE.into(),
                created_at_ms: warning.created_at_ms,
            },
        );
    }
    for (key, entry) in &outbox.operations {
        if !entry.uncertainty_warning_acknowledged
            && entry
                .receipt
                .as_ref()
                .is_some_and(|receipt| matches!(receipt, DesktopReceipt::OutcomeUnknown))
        {
            warnings
                .entry(key.clone())
                .or_insert_with(|| DesktopUncertaintyWarning {
                    operation_id: String::from(entry.operation_id),
                    request_hash: entry.request_hash.clone(),
                    message: MESSAGE.into(),
                    created_at_ms: entry.created_at_ms,
                });
        }
    }
    warnings.into_values().collect()
}

pub(crate) fn uncertainty_warnings(
    app: &tauri::AppHandle,
) -> Result<Vec<DesktopUncertaintyWarning>, String> {
    let path = outbox_path(app)?;
    outbox_lock(&path, || {
        Ok(desktop_uncertainty_warnings(&read_outbox(&path)?))
    })
}

pub(crate) fn acknowledge_uncertainty_warning(
    app: &tauri::AppHandle,
    operation_id: &str,
    request_hash: &str,
) -> Result<(), String> {
    let operation_id = NativeIntentId::try_from(operation_id.to_owned())
        .map_err(|_| "The desktop uncertainty warning operation ID is invalid.")?;
    if request_hash.len() != 64 || !request_hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("The desktop uncertainty warning request hash is invalid.".into());
    }
    let key = operation_key(operation_id);
    let path = outbox_path(app)?;
    outbox_lock(&path, || {
        let mut outbox = read_outbox(&path)?;
        if let Some(warning) = outbox.uncertainty_warnings.get(&key) {
            if warning.operation_id != operation_id || warning.request_hash != request_hash {
                return Err(
                    "The desktop uncertainty warning changed before acknowledgement.".into(),
                );
            }
            outbox.uncertainty_warnings.remove(&key);
            if let Some(entry) = outbox.operations.get_mut(&key) {
                if entry.operation_id != operation_id || entry.request_hash != request_hash {
                    return Err(
                        "The desktop uncertainty warning conflicts with its operation.".into(),
                    );
                }
                entry.uncertainty_warning_acknowledged = true;
                if entry.acknowledged && entry.binding_effects_settled {
                    outbox.operations.remove(&key);
                }
            }
            return write_outbox(&path, &outbox);
        }

        let Some(entry) = outbox.operations.get_mut(&key) else {
            return Ok(());
        };
        if entry.operation_id != operation_id || entry.request_hash != request_hash {
            return Err("The desktop uncertainty warning changed before acknowledgement.".into());
        }
        if !entry
            .receipt
            .as_ref()
            .is_some_and(|receipt| matches!(receipt, DesktopReceipt::OutcomeUnknown))
        {
            return Err("The desktop operation has no uncertainty warning to acknowledge.".into());
        }
        entry.uncertainty_warning_acknowledged = true;
        if entry.acknowledged && entry.binding_effects_settled {
            outbox.operations.remove(&key);
        }
        write_outbox(&path, &outbox)
    })
}

fn native_request(app: &tauri::AppHandle, call: LocalCall) -> Result<LocalResult, String> {
    app.state::<shared_host::SharedHost>()
        .native_file_request(call)
}

fn stage_payload(
    operation_id: NativeIntentId,
    staged: &LocalNativeFileStage,
    payload: &[u8],
    expected_hash: &str,
) -> Result<(), String> {
    let profile = lomi_platform::default_profile()
        .ok_or("The canonical local Host profile is unavailable.")?;
    #[cfg(unix)]
    let mut stage = lomi_platform::unix::open_native_editor_stage(&profile, operation_id, false)
        .map_err(|_| "The private desktop file stage is unavailable.")?;
    #[cfg(windows)]
    let mut stage = lomi_platform::windows::open_native_editor_stage(&profile, operation_id, false)
        .map_err(|_| "The private desktop file stage is unavailable.")?;
    let stage_path = stage
        .path()
        .to_str()
        .ok_or("The private desktop file stage path is invalid.")?;
    if staged.operation_id != operation_id || staged.path.as_str() != stage_path {
        return Err("The Host returned an unexpected desktop file stage.".into());
    }
    let file = stage.file_mut();
    file.set_len(0)
        .map_err(|_| "The desktop file stage could not be written.")?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "The desktop file stage could not be written.")?;
    file.write_all(payload)
        .map_err(|_| "The desktop file stage could not be written.")?;
    file.sync_all()
        .map_err(|_| "The desktop file stage could not be synced.")?;
    if file.metadata().map_err(|error| error.to_string())?.len() != payload.len() as u64 {
        return Err("The desktop file stage has an unexpected size.".into());
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "The desktop file stage could not be read back.")?;
    let mut bytes = Vec::with_capacity(payload.len());
    file.take(payload.len() as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "The desktop file stage could not be read back.")?;
    if bytes.len() != payload.len() || revision(&bytes) != expected_hash {
        return Err("The desktop file stage did not retain the requested data.".into());
    }
    Ok(())
}

fn stage_request(
    app: &tauri::AppHandle,
    operation_id: NativeIntentId,
    payload: &[u8],
    content_hash: &str,
) -> Result<LocalNativeFileBlob, String> {
    let blob = LocalNativeFileBlob {
        operation_id,
        byte_length: DecimalU64(payload.len() as u64),
        content_sha256: HostKeyCommitmentHex::try_from(content_hash.to_owned())
            .map_err(|_| "The desktop file request hash is invalid.")?,
    };
    let staged = match native_request(app, LocalCall::NativeFileStage(blob.clone()))? {
        LocalResult::NativeFileStage(staged) if staged.operation_id == operation_id => staged,
        _ => return Err("The Host returned an invalid desktop file stage.".into()),
    };
    stage_payload(operation_id, &staged, payload, content_hash)?;
    Ok(blob)
}

fn remaining_timeout(deadline: Instant) -> Result<TimeoutMs, String> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    let millis = remaining.as_millis().min(u128::from(u64::MAX)) as u64;
    if millis == 0 {
        return Err("The desktop file operation deadline expired before Host execution.".into());
    }
    TimeoutMs::try_from(millis.min(MUTATION_TIMEOUT.as_millis() as u64))
        .map_err(|_| "The desktop file operation timeout is invalid.".into())
}

fn guarded_execute(
    app: &tauri::AppHandle,
    window: &tauri::Window,
    webview: &tauri::Webview,
    deadline: Instant,
    blob: LocalNativeFileBlob,
    root: PathBuf,
) -> Result<LocalResult, String> {
    let operation_id = blob.operation_id;
    let timeout_ms = remaining_timeout(deadline)?;
    let root = host_root(root)?;
    let execute = LocalCall::NativeEditorExecuteGuarded(LocalNativeEditorExecute {
        blob,
        effect: LocalNativeEditorEffect::DesktopFiles { root, timeout_ms },
    });
    match app
        .state::<shared_host::SharedHost>()
        .native_file_request_guarded_result(execute, || {
            Instant::now() < deadline && shared_host::trusted_full_window(window, webview).is_ok()
        })? {
        Ok(result) => Ok(result),
        Err(LocalError::InvalidArgument) => {
            // The Host only returns this before native_editor_begin, so it is
            // a typed no-effect receipt rather than an absent-row inference.
            let json =
                serde_json::to_string(&DesktopMutationError::Before(DesktopFileErrorCode::Invalid))
                    .map_err(|_| "The desktop file rejection could not be encoded.")?;
            Ok(LocalResult::NativeEditor(LocalNativeEditorResult {
                operation_id,
                state: LocalNativeEffectState::Failed,
                result_json: Some(
                    LocalNativeResultJson::try_from(json)
                        .map_err(|_| "The desktop file rejection is invalid.")?,
                ),
            }))
        }
        Err(error) => Err(format!(
            "The Host rejected the desktop file request ({error:?})."
        )),
    }
}

fn host_root(path: PathBuf) -> Result<LocalRootPath, String> {
    let value = canonical_text(&path)?;
    LocalRootPath::try_from(value).map_err(|_| "The desktop project path is invalid.".into())
}

fn status(app: &tauri::AppHandle, operation_id: NativeIntentId) -> Result<LocalResult, String> {
    native_request(
        app,
        LocalCall::NativeEditorStatus(NativeIntentTarget {
            intent_id: operation_id,
        }),
    )
}

fn acknowledge(app: &tauri::AppHandle, operation_id: NativeIntentId) -> Result<(), String> {
    match native_request(
        app,
        LocalCall::NativeEditorAcknowledge(NativeIntentTarget {
            intent_id: operation_id,
        }),
    )? {
        LocalResult::NativeEditorAcknowledged(confirmed) if confirmed == operation_id => Ok(()),
        _ => Err("The Host did not acknowledge the desktop file result.".into()),
    }
}

fn parse_terminal(
    result: &lomi_remote_protocol::ipc::LocalNativeEditorResult,
    entry: &DesktopOutboxEntry,
) -> Result<DesktopReceipt, String> {
    if result.operation_id != entry.operation_id {
        return Err("The Host returned a mismatched desktop file operation ID.".into());
    }
    if result.state == LocalNativeEffectState::OutcomeUnknown {
        return Ok(DesktopReceipt::OutcomeUnknown);
    }
    let json = result
        .result_json
        .as_ref()
        .ok_or("The Host omitted the desktop file operation result.")?
        .as_str();
    if json.len() > RESULT_LIMIT_BYTES {
        return Err("The Host desktop file result is too large.".into());
    }
    match result.state {
        LocalNativeEffectState::Succeeded => {
            let change: DesktopFileChange = serde_json::from_str(json)
                .map_err(|_| "The Host desktop file result is invalid.")?;
            expected_result_paths(
                &entry.request,
                entry.expected_source_path.as_deref(),
                entry.source_is_directory,
                &change,
            )?;
            Ok(DesktopReceipt::Succeeded(change))
        }
        LocalNativeEffectState::Failed => {
            if let Ok(error) = serde_json::from_str::<DesktopMutationError>(json) {
                return match error {
                    DesktopMutationError::Before(code) => Ok(DesktopReceipt::Failed(code)),
                    DesktopMutationError::Uncertain => Err(
                        "The Host could not determine whether the file operation completed.".into(),
                    ),
                };
            }
            // The canonical Host emits this generic error only when it rejects
            // Execute before creating a row (writer busy / receipt capacity).
            // It is therefore a known no-effect result when received directly.
            let error: lomi_native_files::NativeFileError = serde_json::from_str(json)
                .map_err(|_| "The Host desktop file failure result is invalid.")?;
            if error.kind == lomi_native_files::NativeFileErrorKind::Io
                && error.message.len() <= 8_192
            {
                Ok(DesktopReceipt::FailedMessage(error.message))
            } else {
                Err("The Host could not determine whether the file operation completed.".into())
            }
        }
        LocalNativeEffectState::OutcomeUnknown => Ok(DesktopReceipt::OutcomeUnknown),
        LocalNativeEffectState::Running => {
            Err("The Host desktop file operation is still unresolved.".into())
        }
    }
}

fn save_receipt(
    path: &Path,
    outbox: &mut DesktopOutbox,
    operation_id: NativeIntentId,
    receipt: DesktopReceipt,
) -> Result<(), String> {
    let entry = outbox
        .operations
        .get_mut(&operation_key(operation_id))
        .ok_or("The desktop file receipt has no durable request.")?;
    entry.phase = DesktopPhase::Terminal;
    entry.receipt = Some(receipt);
    entry.acknowledgement_pending = true;
    write_outbox(path, outbox)
}

fn transfer_trash_once(ticket: &DesktopTrashTicket) -> Result<(), String> {
    match lomi_files::desktop::finish_desktop_trash(ticket, |path| trash::delete(path)) {
        Ok(()) => Ok(()),
        Err(DesktopTrashError::Before(code)) => Err(format!(
            "The project item was removed, but its Trash transfer needs recovery ({:?}); Lomi retained the recovery ticket and will not repeat the transfer automatically.",
            code
        )),
        Err(DesktopTrashError::Transfer(error)) => Err(format!(
            "The project item was removed, but its Trash transfer needs recovery: {error}. Lomi retained the recovery ticket and will not repeat the transfer automatically."
        )),
        Err(DesktopTrashError::Uncertain) => Err(
            "The project item was removed, but its Trash transfer outcome is uncertain; Lomi retained the recovery ticket and will not repeat the transfer automatically."
                .into(),
        ),
    }
}

fn settle_unknown_receipt(
    app: &tauri::AppHandle,
    path: &Path,
    outbox: &mut DesktopOutbox,
    operation_id: NativeIntentId,
) -> Result<(Option<FileChange>, Option<String>), String> {
    let key = operation_key(operation_id);
    let Some(mut entry) = outbox.operations.get(&key).cloned() else {
        return Err("The desktop file result has no outbox entry.".into());
    };
    let uncertainty = "The Host could not determine whether the desktop file operation completed; Lomi retained the uncertainty warning.";

    if entry.acknowledged && entry.binding_effects_settled {
        if let Err(error) = ensure_unknown_warning_record(outbox, &entry) {
            return Ok((
                None,
                Some(format!(
                    "{uncertainty} Its settled recovery record is retained because the warning could not be persisted: {error}"
                )),
            ));
        }
        if let Err(error) = write_outbox(path, outbox) {
            return Ok((
                None,
                Some(format!(
                    "{uncertainty} Its settled recovery record is retained because the warning could not be persisted: {error}"
                )),
            ));
        }
        outbox.operations.remove(&key);
        if write_outbox(path, outbox).is_err() {
            outbox.operations.insert(key, entry);
            return Ok((
                None,
                Some(format!(
                    "{uncertainty} Its settled local recovery record could not be cleared."
                )),
            ));
        }
        return Ok((None, Some(uncertainty.into())));
    }

    let host = app.state::<shared_host::SharedHost>();
    let saved_disposition = entry.unknown_disposition.clone();
    let disposition = match host.native_editor_disposition_optional(operation_id) {
        Ok(disposition) => disposition,
        Err(error) => {
            return Ok((
                None,
                Some(format!(
                    "{uncertainty} The Host disposition remains unresolved: {error}"
                )),
            ));
        }
    };
    let (host_id, runtime_id) = match disposition {
        shared_host::NativeEditorDispositionRead::Present(disposition) => {
            if saved_disposition
                .as_ref()
                .is_some_and(|saved| saved.host_id != disposition.host_id)
            {
                return Ok((
                    None,
                    Some(format!(
                        "{uncertainty} The Host identity differs from the persisted disposition proof."
                    )),
                ));
            }
            if disposition.disposition.retirement == NativeEditorRetirement::Unresolved {
                return Ok((
                    None,
                    Some(format!(
                        "{uncertainty} The Host still reports an unresolved writer disposition."
                    )),
                ));
            }
            let refreshed = DesktopUnknownDisposition {
                host_id: disposition.host_id.clone(),
                runtime_id: disposition.runtime_id.clone(),
                retirement: disposition.disposition.retirement,
            };
            if saved_disposition.as_ref().is_none_or(|saved| {
                saved.host_id != refreshed.host_id
                    || saved.runtime_id != refreshed.runtime_id
                    || saved.retirement != refreshed.retirement
            }) {
                entry.unknown_disposition = Some(refreshed);
                outbox.operations.insert(key.clone(), entry.clone());
                if let Err(error) = write_outbox(path, outbox) {
                    outbox.operations.insert(key.clone(), entry);
                    return Ok((
                        None,
                        Some(format!(
                            "{uncertainty} Lomi could not persist the current Host disposition proof: {error}"
                        )),
                    ));
                }
            }
            (disposition.host_id, disposition.runtime_id)
        }
        shared_host::NativeEditorDispositionRead::Missing {
            host_id,
            runtime_id,
        } => {
            if !saved_disposition
                .as_ref()
                .is_some_and(|saved| saved.host_id == host_id)
            {
                return Ok((
                    None,
                    Some(format!(
                        "{uncertainty} The Host no longer has this operation, and no matching positive disposition proof was saved."
                    )),
                ));
            }
            (host_id, runtime_id)
        }
    };

    let requested_workspace_ids = entry
        .binding_snapshot
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|binding| binding.workspace_id.clone())
        .collect::<Vec<_>>();
    let reconciled = match host.reconcile_workspace_bindings(
        app,
        "desktop-file-unknown-recovery",
        &requested_workspace_ids,
    ) {
        Ok(reconciled) => reconciled,
        Err(error) => {
            return Ok((
                None,
                Some(format!(
                    "{uncertainty} Current Host workspace bindings remain unreconciled: {error}"
                )),
            ));
        }
    };
    if reconciled.host_id != host_id || reconciled.runtime_id != runtime_id {
        return Ok((
            None,
            Some(format!(
                "{uncertainty} The Host changed before its workspace bindings could be reconciled."
            )),
        ));
    }

    match host.native_editor_host_identity() {
        Ok((current_host_id, current_runtime_id))
            if current_host_id == host_id && current_runtime_id == runtime_id => {}
        Ok(_) => {
            return Ok((
                None,
                Some(format!(
                    "{uncertainty} The Host changed after workspace bindings were reconciled."
                )),
            ));
        }
        Err(error) => {
            return Ok((
                None,
                Some(format!(
                    "{uncertainty} The Host identity could not be checked before acknowledgement: {error}"
                )),
            ));
        }
    }
    if let Err(error) = acknowledge(app, operation_id) {
        return Ok((
            None,
            Some(format!(
                "{uncertainty} Status-only Host acknowledgement remains pending: {error}"
            )),
        ));
    }
    match host.native_editor_host_identity() {
        Ok((current_host_id, current_runtime_id))
            if current_host_id == host_id && current_runtime_id == runtime_id => {}
        Ok(_) => {
            return Ok((
                None,
                Some(format!(
                    "{uncertainty} The Host changed during receipt acknowledgement."
                )),
            ));
        }
        Err(error) => {
            return Ok((
                None,
                Some(format!(
                    "{uncertainty} Receipt acknowledgement could not be confirmed against the Host identity: {error}"
                )),
            ));
        }
    }
    entry.acknowledged = true;
    entry.binding_effects_settled = true;
    if let Err(error) = ensure_unknown_warning_record(outbox, &entry) {
        return Ok((
            None,
            Some(format!(
                "{uncertainty} Its Host receipt is settled, but the uncertainty warning could not be recorded: {error}"
            )),
        ));
    }
    outbox.operations.insert(key.clone(), entry.clone());
    if let Err(error) = write_outbox(path, outbox) {
        entry.acknowledged = false;
        entry.binding_effects_settled = false;
        outbox.operations.insert(key.clone(), entry);
        return Ok((
            None,
            Some(format!(
                "{uncertainty} The Host acknowledged the receipt, but Lomi could not persist its settlement: {error}"
            )),
        ));
    }

    outbox.operations.remove(&key);
    if write_outbox(path, outbox).is_err() {
        outbox.operations.insert(key, entry);
        return Ok((
            None,
            Some(format!(
                "{uncertainty} The settled local recovery record could not be cleared."
            )),
        ));
    }
    Ok((None, Some(uncertainty.into())))
}

fn settle_receipt(
    app: &tauri::AppHandle,
    path: &Path,
    outbox: &mut DesktopOutbox,
    operation_id: NativeIntentId,
    allow_trash_attempt: bool,
) -> Result<(Option<FileChange>, Option<String>), String> {
    let key = operation_key(operation_id);
    let Some(mut entry) = outbox.operations.get(&key).cloned() else {
        return Err("The desktop file result has no outbox entry.".into());
    };
    let Some(receipt) = entry.receipt.clone() else {
        return Ok((None, None));
    };
    if matches!(receipt, DesktopReceipt::OutcomeUnknown) {
        return settle_unknown_receipt(app, path, outbox, operation_id);
    }
    let mut warning = None;
    let binding_effect_required = matches!(
        (&entry.binding_snapshot, &receipt),
        (Some(_), DesktopReceipt::Succeeded(_))
    );
    let mut binding_effects_settled = !binding_effect_required || entry.binding_effects_settled;
    if binding_effect_required && !entry.binding_effects_settled {
        let applied = match (&entry.binding_snapshot, &receipt) {
            (Some(snapshots), DesktopReceipt::Succeeded(change)) => {
                binding_effect_digest(&entry, change).and_then(|digest| {
                    let protected_operation_ids = outbox.operations.keys().cloned().collect();
                    app.state::<shared_host::SharedHost>()
                        .apply_desktop_file_binding_effects(
                            app,
                            entry.operation_id,
                            &digest,
                            snapshots,
                            change.old_path.as_deref(),
                            change.new_path.as_deref(),
                            &protected_operation_ids,
                        )
                })
            }
            _ => Ok(()),
        };
        match applied {
            Ok(()) => {
                entry.binding_effects_settled = true;
                outbox.operations.insert(key.clone(), entry.clone());
                match write_outbox(path, outbox) {
                    Ok(()) => binding_effects_settled = true,
                    Err(error) => {
                        entry.binding_effects_settled = false;
                        outbox.operations.insert(key.clone(), entry.clone());
                        append_warning(
                            &mut warning,
                            Some(format!(
                                "The file change is complete, but Lomi could not record its workspace binding update; recovery remains pending: {error}"
                            )),
                        );
                    }
                }
            }
            Err(error) => {
                append_warning(
                    &mut warning,
                    Some(format!(
                        "The file change is complete, but its workspace binding update remains pending: {error}"
                    )),
                );
            }
        }
    }
    if binding_effects_settled && !entry.acknowledged {
        match acknowledge(app, operation_id) {
            Ok(()) => {
                entry.acknowledged = true;
                outbox.operations.insert(key.clone(), entry.clone());
                if write_outbox(path, outbox).is_err() {
                    append_warning(
                        &mut warning,
                        Some("The Host receipt was acknowledged, but its local acknowledgement record could not be updated; recovery remains pending.".into()),
                    );
                    entry.acknowledged = false;
                    outbox.operations.insert(key.clone(), entry.clone());
                }
            }
            Err(error) => warning = Some(format!(
                "The file change is complete, but its Host receipt acknowledgement is pending: {error}"
            )),
        }
    }

    let change = match receipt {
        DesktopReceipt::Failed(_) | DesktopReceipt::FailedMessage(_) => None,
        DesktopReceipt::OutcomeUnknown => None,
        DesktopReceipt::Succeeded(change) => {
            let ticket = change.trash.as_ref();
            if let Some(ticket) = ticket {
                if !binding_effects_settled {
                    // Retain the receipt and ticket until binding state is durable.
                } else if !entry.trash_transfer_attempted && allow_trash_attempt {
                    entry.trash_transfer_attempted = true;
                    outbox.operations.insert(key.clone(), entry.clone());
                    if write_outbox(path, outbox).is_err() {
                        append_warning(
                            &mut warning,
                            Some("The project item was removed, but Lomi could not persist the Trash transfer attempt; the recovery ticket was retained and the transfer was not started.".into()),
                        );
                        entry.trash_transfer_attempted = false;
                        outbox.operations.insert(key.clone(), entry.clone());
                    } else {
                        match transfer_trash_once(ticket) {
                            Ok(()) => {
                                entry.trash_transfer_complete = true;
                                outbox.operations.insert(key.clone(), entry.clone());
                                if write_outbox(path, outbox).is_err() {
                                    append_warning(
                                        &mut warning,
                                        Some("The item was transferred to Trash, but Lomi could not record that completion; the recovery ticket was retained and will not be transferred again automatically.".into()),
                                    );
                                    entry.trash_transfer_complete = false;
                                    outbox.operations.insert(key.clone(), entry.clone());
                                }
                            }
                            Err(error) => append_warning(&mut warning, Some(error)),
                        }
                    }
                } else if !entry.trash_transfer_attempted {
                    warning = Some(
                        "The project item was removed, but its Trash transfer has not been attempted; Lomi retained the recovery ticket.".into(),
                    );
                } else if entry.trash_transfer_attempted && !entry.trash_transfer_complete {
                    warning = Some(
                        "The project item was removed, but its Trash transfer outcome needs recovery; Lomi retained the recovery ticket and will not repeat the transfer automatically.".into(),
                    );
                }
            }
            Some(FileChange {
                old_path: change
                    .old_path
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
                new_path: change
                    .new_path
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
                warning: warning.clone(),
            })
        }
    };
    let entry = outbox
        .operations
        .get(&key)
        .cloned()
        .ok_or("The desktop file result disappeared from its outbox.")?;
    let trash_pending = entry.receipt.as_ref().is_some_and(|receipt| {
        matches!(receipt, DesktopReceipt::Succeeded(change)
            if change.trash.is_some() && !entry.trash_transfer_complete)
    });
    if binding_effects_settled && entry.acknowledged && !trash_pending {
        outbox.operations.remove(&key);
        if write_outbox(path, outbox).is_err() {
            append_warning(
                &mut warning,
                Some("The file change is complete, but Lomi could not clear its settled local receipt; recovery remains pending.".into()),
            );
            outbox.operations.insert(key, entry);
        }
    }
    Ok((
        change.map(|mut change| {
            change.warning = warning.clone();
            change
        }),
        warning,
    ))
}

fn append_warning(current: &mut Option<String>, next: Option<String>) {
    let Some(next) = next else { return };
    match current {
        Some(current) if !current.contains(&next) => {
            if current.len().saturating_add(next.len()).saturating_add(2) <= 4096 {
                current.push_str("; ");
                current.push_str(&next);
            }
        }
        None => *current = Some(next),
        _ => {}
    }
}

fn recover_locked(
    app: &tauri::AppHandle,
    path: &Path,
    allow_trash_attempt: bool,
) -> Result<RecoverySummary, String> {
    let mut outbox = read_outbox(path)?;
    let now = now_ms();
    let mut eligible: Vec<_> = outbox
        .operations
        .values()
        .filter(|entry| {
            entry.receipt.is_some()
                || entry.last_recovery_attempt_ms.is_none_or(|last| {
                    now.saturating_sub(last) >= RECOVERY_RETRY.as_millis() as u64
                })
        })
        .cloned()
        .collect();
    eligible.sort_by(|left, right| {
        left.last_recovery_attempt_ms
            .cmp(&right.last_recovery_attempt_ms)
            .then_with(|| operation_key(left.operation_id).cmp(&operation_key(right.operation_id)))
    });
    let selected: Vec<_> = eligible.into_iter().take(RECOVERY_BATCH).collect();
    if selected.is_empty() {
        return Ok(RecoverySummary::default());
    }
    for entry in &selected {
        if let Some(current) = outbox
            .operations
            .get_mut(&operation_key(entry.operation_id))
        {
            current.last_recovery_attempt_ms = Some(now);
        }
    }
    write_outbox(path, &outbox)?;

    let mut summary = RecoverySummary::default();
    for selected_entry in selected {
        let key = operation_key(selected_entry.operation_id);
        let Some(entry) = outbox.operations.get(&key).cloned() else {
            continue;
        };
        if entry.receipt.is_some() {
            let (_, warning) = settle_receipt(
                app,
                path,
                &mut outbox,
                entry.operation_id,
                allow_trash_attempt,
            )?;
            append_warning(&mut summary.warning, warning);
            continue;
        }
        if entry.phase == DesktopPhase::Staging {
            // Stage alone cannot mutate the project. No Execute was dispatched.
            outbox.operations.remove(&key);
            write_outbox(path, &outbox)?;
            continue;
        }
        let result = match status(app, entry.operation_id) {
            Ok(LocalResult::NativeEditor(result)) if result.operation_id == entry.operation_id => {
                if !matches!(
                    result.state,
                    LocalNativeEffectState::Succeeded
                        | LocalNativeEffectState::Failed
                        | LocalNativeEffectState::OutcomeUnknown
                ) {
                    continue;
                }
                parse_terminal(&result, &entry).ok()
            }
            _ => None,
        };
        let Some(receipt) = result else {
            continue;
        };
        save_receipt(path, &mut outbox, entry.operation_id, receipt)?;
        let (_, warning) = settle_receipt(
            app,
            path,
            &mut outbox,
            entry.operation_id,
            allow_trash_attempt,
        )?;
        append_warning(&mut summary.warning, warning);
    }
    Ok(summary)
}

/// Read-only status reconciliation for app startup/status paths. An empty
/// outbox does not start or connect to the Host.
pub(crate) fn recover_pending(app: &tauri::AppHandle) -> Result<(), String> {
    let path = outbox_path(app)?;
    outbox_lock(&path, || {
        if !path.exists() {
            return Ok(());
        }
        recover_locked(app, &path, false).map(|_| ())
    })
}

fn ensure_capacity(outbox: &mut DesktopOutbox) -> Result<(), String> {
    if outbox.operations.len() < OUTBOX_LIMIT_ENTRIES {
        return Ok(());
    }
    Err("Desktop file operation recovery has reached its bounded limit. Resolve a pending operation before continuing.".into())
}

fn remove_operation(
    path: &Path,
    outbox: &mut DesktopOutbox,
    operation_id: NativeIntentId,
) -> Result<(), String> {
    outbox.operations.remove(&operation_key(operation_id));
    write_outbox(path, outbox)
}

fn stage_and_execute(
    app: &tauri::AppHandle,
    window: &tauri::Window,
    webview: &tauri::Webview,
    path: &Path,
    outbox: &mut DesktopOutbox,
    operation_id: NativeIntentId,
    payload: &[u8],
    deadline: Instant,
) -> Result<DesktopReceipt, String> {
    let content_hash = revision(payload);
    let blob = match stage_request(app, operation_id, payload, &content_hash) {
        Ok(blob) => blob,
        Err(error) => {
            let _ = remove_operation(path, outbox, operation_id);
            return Err(error);
        }
    };
    if let Some(entry) = outbox.operations.get_mut(&operation_key(operation_id)) {
        entry.phase = DesktopPhase::Staging;
    }
    write_outbox(path, outbox)?;
    if let Err(error) = remaining_timeout(deadline) {
        let _ = remove_operation(path, outbox, operation_id);
        return Err(error);
    }
    if let Some(entry) = outbox.operations.get_mut(&operation_key(operation_id)) {
        entry.phase = DesktopPhase::ExecuteSent;
    }
    write_outbox(path, outbox)?;
    let root = outbox
        .operations
        .get(&operation_key(operation_id))
        .map(|entry| entry.request.root.clone())
        .ok_or("The desktop file request disappeared before Execute.")?;
    let result = guarded_execute(app, window, webview, deadline, blob, root);
    let initial = match result {
        Ok(LocalResult::NativeEditor(result)) if result.operation_id == operation_id => {
            Some(result)
        }
        _ => None,
    };
    let entry = outbox
        .operations
        .get(&operation_key(operation_id))
        .ok_or("The desktop file request disappeared before Execute.")?;
    let terminal = resolve_terminal(app, entry, initial);
    if let Some(receipt) = terminal {
        let receipt_result = match &receipt {
            DesktopReceipt::Succeeded(change) => serde_json::to_string(change),
            DesktopReceipt::Failed(code) => {
                serde_json::to_string(&DesktopMutationError::Before(*code))
            }
            DesktopReceipt::FailedMessage(message) => serde_json::to_string(message),
            DesktopReceipt::OutcomeUnknown => Ok("outcomeUnknown".to_owned()),
        }
        .map_err(|_| "The Host desktop file result could not be retained.")?;
        if receipt_result.len() > RESULT_LIMIT_BYTES {
            return Err("The Host desktop file result is too large to retain safely.".into());
        }
        save_receipt(path, outbox, operation_id, receipt.clone())?;
        Ok(receipt)
    } else {
        Err("The desktop file operation outcome is unresolved. Lomi will query Host status and will not repeat the mutation.".into())
    }
}

fn resolve_terminal(
    app: &tauri::AppHandle,
    entry: &DesktopOutboxEntry,
    mut latest: Option<lomi_remote_protocol::ipc::LocalNativeEditorResult>,
) -> Option<DesktopReceipt> {
    for attempt in 0..=STATUS_POLLS {
        let Some(result) = latest.take() else {
            if attempt > 0 {
                thread::sleep(STATUS_INTERVAL);
            }
            latest = match status(app, entry.operation_id) {
                Ok(LocalResult::NativeEditor(result))
                    if result.operation_id == entry.operation_id =>
                {
                    Some(result)
                }
                _ => None,
            };
            continue;
        };
        if !matches!(
            result.state,
            LocalNativeEffectState::Succeeded
                | LocalNativeEffectState::Failed
                | LocalNativeEffectState::OutcomeUnknown
        ) {
            if attempt == STATUS_POLLS {
                return None;
            }
            thread::sleep(STATUS_INTERVAL);
            latest = None;
            continue;
        }
        return parse_terminal(&result, entry).ok();
    }
    None
}

fn desktop_error_message(code: DesktopFileErrorCode) -> String {
    match code {
        DesktopFileErrorCode::NotFound => "The selected file or folder no longer exists.".into(),
        DesktopFileErrorCode::Changed => "The selected file or folder changed. Try again.".into(),
        DesktopFileErrorCode::Protected => "This file or folder is protected.".into(),
        DesktopFileErrorCode::Exists => "A file or folder with that name already exists.".into(),
        DesktopFileErrorCode::NotEmpty => "The folder is not empty.".into(),
        DesktopFileErrorCode::Invalid => "The desktop file operation is invalid.".into(),
        DesktopFileErrorCode::Unsupported => "This desktop file operation is unsupported.".into(),
        DesktopFileErrorCode::Limit => "The desktop file operation exceeds a safety limit.".into(),
        DesktopFileErrorCode::Io => "The desktop file operation could not be completed.".into(),
        DesktopFileErrorCode::Revoked => "Desktop file operation access was revoked.".into(),
        DesktopFileErrorCode::DeadlineExceeded => "The desktop file operation timed out.".into(),
    }
}

fn outbox_has_unresolved_scope(
    outbox: &DesktopOutbox,
    prospective_entry: &DesktopOutboxEntry,
    prospective_scopes: &[PathBuf],
) -> Result<bool, String> {
    for entry in outbox.operations.values() {
        let unresolved_host_effect = entry.phase != DesktopPhase::Terminal;
        let unresolved_execute =
            entry.phase == DesktopPhase::ExecuteSent && entry.receipt.is_none();
        let unresolved_unknown = entry.receipt.as_ref().is_some_and(|receipt| {
            matches!(receipt, DesktopReceipt::OutcomeUnknown)
                && !(entry.acknowledged && entry.binding_effects_settled)
        });
        let binding_effect_pending = entry.binding_snapshot.is_some()
            && !entry.binding_effects_settled
            && entry
                .receipt
                .as_ref()
                .is_some_and(|receipt| matches!(receipt, DesktopReceipt::Succeeded(_)));
        if unresolved_execute || unresolved_unknown {
            return Ok(true);
        }
        if !unresolved_host_effect && !binding_effect_pending {
            continue;
        }
        if unresolved_host_effect
            && path_overlaps(&prospective_entry.request.root, &entry.request.root)
        {
            return Ok(true);
        }
        let pending_scopes = project_write_scopes(entry)?;
        if prospective_scopes.iter().any(|prospective_scope| {
            pending_scopes
                .iter()
                .any(|pending_scope| path_overlaps(prospective_scope, pending_scope))
        }) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn file_operation(
    app: &tauri::AppHandle,
    window: &tauri::Window,
    webview: &tauri::Webview,
    root: &str,
    relative: &str,
    operation: Operation,
    expected_path: Option<String>,
    deadline: Instant,
) -> Result<crate::files::operations::FileChange, String> {
    shared_host::trusted_full_window(window, webview)?;
    if Instant::now() >= deadline {
        return Err("The desktop file operation deadline expired.".into());
    }
    let operation_id = new_operation_id()?;
    let (request, expected_source_path, source_is_directory) =
        request_for_operation(app, root, relative, operation_id, operation, expected_path)?;
    execute_request(
        app,
        window,
        webview,
        request,
        expected_source_path,
        source_is_directory,
        deadline,
    )
}

pub(crate) fn git_discard(
    app: &tauri::AppHandle,
    window: &tauri::Window,
    webview: &tauri::Webview,
    root: &str,
    expected: DesktopGitChange,
    deadline: Instant,
) -> Result<FileChange, String> {
    let relative = expected.path.clone();
    git_operation(
        app,
        window,
        webview,
        root,
        &relative,
        Some(expected),
        false,
        deadline,
    )
}

pub(crate) fn git_ignore(
    app: &tauri::AppHandle,
    window: &tauri::Window,
    webview: &tauri::Webview,
    root: &str,
    relative: &str,
    local: bool,
    deadline: Instant,
) -> Result<FileChange, String> {
    git_operation(app, window, webview, root, relative, None, local, deadline)
}

#[allow(clippy::too_many_arguments)]
fn git_operation(
    app: &tauri::AppHandle,
    window: &tauri::Window,
    webview: &tauri::Webview,
    root: &str,
    relative: &str,
    expected: Option<DesktopGitChange>,
    local: bool,
    deadline: Instant,
) -> Result<FileChange, String> {
    shared_host::trusted_full_window(window, webview)?;
    if Instant::now() >= deadline {
        return Err("The desktop Git operation deadline expired.".into());
    }
    lomi_files::desktop::validate_desktop_relative(relative, false)
        .map_err(desktop_error_message)?;
    let repository = crate::files::directory(root)?;
    let binding = app
        .state::<shared_host::SharedHost>()
        .native_git_binding(app, &repository)?;
    let repository_relative = canonical_relative(&binding.root_path, &repository)?;
    let selected_path = repository.join(relative);
    let relative_path = canonical_relative(&binding.root_path, &selected_path)?;
    let control_root = app_data_dir(app)?;
    if path_overlaps(&binding.root_path, &control_root) {
        return Err("Desktop Git operations cannot target Lomi's private app data.".into());
    }
    let operation = match expected {
        Some(expected) => DesktopFileOperation::GitDiscard {
            context: binding.workspace.context,
            repository_relative,
            expected,
        },
        None => DesktopFileOperation::GitIgnore {
            context: binding.workspace.context,
            repository_relative,
            local,
        },
    };
    let operation_id = new_operation_id()?;
    let request = DesktopMutationRequest {
        version: 1,
        operation_id: operation_key(operation_id),
        root: binding.root_path,
        relative_path,
        expected_path: Some(selected_path),
        control_roots: vec![control_root],
        operation,
    };
    lomi_files::desktop::validate_desktop_request(&request).map_err(desktop_error_message)?;
    execute_request(app, window, webview, request, None, None, deadline)
}

#[allow(clippy::too_many_arguments)]
fn execute_request(
    app: &tauri::AppHandle,
    window: &tauri::Window,
    webview: &tauri::Webview,
    request: DesktopMutationRequest,
    expected_source_path: Option<PathBuf>,
    source_is_directory: Option<bool>,
    deadline: Instant,
) -> Result<FileChange, String> {
    let operation_id = NativeIntentId::try_from(request.operation_id.clone())
        .map_err(|_| "The desktop operation ID is invalid.")?;
    let payload = serde_json::to_vec(&request)
        .map_err(|_| "The desktop file request could not be serialized.")?;
    if payload.len() > REQUEST_LIMIT_BYTES {
        return Err("The desktop file request exceeds its size limit.".into());
    }
    let request_hash = revision(&payload);
    let path = outbox_path(app)?;
    outbox_lock(&path, || {
        let recovered = recover_locked(app, &path, false)?;
        let mut outbox = read_outbox(&path)?;
        ensure_capacity(&mut outbox)?;
        let mut entry = DesktopOutboxEntry {
            operation_id,
            request_hash: request_hash.clone(),
            request: request.clone(),
            expected_source_path,
            source_is_directory,
            phase: DesktopPhase::Staging,
            receipt: None,
            acknowledgement_pending: false,
            acknowledged: false,
            binding_effects_settled: false,
            uncertainty_warning_acknowledged: false,
            trash_transfer_attempted: false,
            trash_transfer_complete: false,
            binding_snapshot: None,
            unknown_disposition: None,
            last_recovery_attempt_ms: None,
            created_at_ms: now_ms(),
        };
        let scopes = project_write_scopes(&entry)?;
        if outbox_has_unresolved_scope(&outbox, &entry, &scopes)? {
            return Err("A previous desktop file operation affecting these paths is unresolved. Lomi will not repeat it.".into());
        }
        if let Some((affected_path, include_descendants)) = binding_snapshot_scope(&entry)? {
            entry.binding_snapshot = Some(
                app.state::<shared_host::SharedHost>()
                    .snapshot_desktop_file_bindings(app, &affected_path, include_descendants)?,
            );
        }
        let scope_refs: Vec<_> = scopes.iter().map(PathBuf::as_path).collect();
        let _admission = crate::project_write_guard::admit(&scope_refs)?;
        outbox.operations.insert(operation_key(operation_id), entry);
        write_outbox(&path, &outbox)?;

        let receipt = stage_and_execute(
            app,
            window,
            webview,
            &path,
            &mut outbox,
            operation_id,
            &payload,
            deadline,
        )?;
        if !outbox
            .operations
            .get(&operation_key(operation_id))
            .is_some_and(|entry| entry.receipt.is_some())
        {
            save_receipt(&path, &mut outbox, operation_id, receipt.clone())?;
        }
        let (change, current_warning) =
            settle_receipt(app, &path, &mut outbox, operation_id, true)?;
        let Some(mut change) = change else {
            let code = match receipt {
                DesktopReceipt::Failed(code) => desktop_error_message(code),
                DesktopReceipt::FailedMessage(message) => message,
                DesktopReceipt::OutcomeUnknown => {
                    let mut warning = current_warning;
                    append_warning(&mut warning, recovered.warning);
                    return Err(warning.unwrap_or_else(|| {
                        "The Host could not determine whether the desktop file operation completed; Lomi retained the operation for status-only recovery.".into()
                    }));
                }
                DesktopReceipt::Succeeded(_) => {
                    return Err(
                        "The Host completed the file operation without a change record.".into(),
                    )
                }
            };
            return Err(code);
        };
        let mut warning = current_warning;
        append_warning(&mut warning, recovered.warning);
        change.warning = warning;
        Ok(change)
    })
}
