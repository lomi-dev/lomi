//! Read-only, source-bound evidence for an owned native task.
//!
//! The canonical host's `native.bindJob` path must call `capture_prelaunch`
//! synchronously from the current Attempt boundary's `release_with_gate`
//! callback. At that point the child job is Held, `held.json` is durable, and
//! `release.json` is absent. The host persists the returned binding before it
//! acknowledges the bind; failure or an Unknown reply must fail the callback
//! and keep the workload held. Desktop input supplies only the stable operation
//! ID and store-root correlation; this reader revalidates the provenance.
//!
//! Later verification consumes the host-persisted binding and independently
//! rereads the durable store and private boundary journals. Desktop must wait
//! for the host's Verified completion path after the Store terminal/settled
//! boundary. Dropping a connection, losing a process handle, or seeing an empty
//! journal is never a completion result.

#![allow(dead_code)] // The fixed-mode helper exposes only its bounded evidence ABI.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const HELPER_BYTES: usize = 128 * 1024;

/// Private fixed-mode helper ABI. The host authenticates the signed executable
/// and capability resource; neither input nor output is an authority token.
pub(crate) fn helper_reply(mode: &str, input: &[u8]) -> Result<Vec<u8>, ()> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct CaptureInput {
        store_root: PathBuf,
        parent_operation_id: String,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct VerifyInput {
        binding: NativeJobBinding,
    }
    #[derive(Serialize)]
    #[serde(tag = "type", content = "value", rename_all = "camelCase")]
    enum Reply {
        Captured(NativeJobBinding),
        Verified(NativeCompletionEvidence),
        Unknown { code: &'static str },
    }
    fn unknown(error: NativeEvidenceUnknown) -> Reply {
        Reply::Unknown {
            code: match error {
                NativeEvidenceUnknown::UnsupportedPlatform => "UNSUPPORTED_PLATFORM",
                NativeEvidenceUnknown::KernelNotAdmitted => "KERNEL_NOT_ADMITTED",
                NativeEvidenceUnknown::SourceUnavailable => "SOURCE_UNAVAILABLE",
                NativeEvidenceUnknown::InvalidProvenance => "INVALID_PROVENANCE",
                NativeEvidenceUnknown::ObligationsPending => "OBLIGATIONS_PENDING",
            },
        }
    }
    if input.is_empty() || input.len() > HELPER_BYTES {
        return Err(());
    }
    let reply = match mode {
        "capture" => {
            let request: CaptureInput = serde_json::from_slice(input).map_err(|_| ())?;
            if request.parent_operation_id.len() > 256
                || request.parent_operation_id.is_empty()
                || request.parent_operation_id.chars().any(char::is_control)
                || !request.store_root.is_absolute()
                || request.store_root.as_os_str().as_encoded_bytes().len() > 4096
            {
                return Err(());
            }
            match capture_prelaunch(&request.store_root, &request.parent_operation_id) {
                Ok(binding) => Reply::Captured(binding),
                Err(error) => unknown(error),
            }
        }
        "verify" => {
            let request: VerifyInput = serde_json::from_slice(input).map_err(|_| ())?;
            match verify_native_job(&request.binding) {
                Ok(evidence) => Reply::Verified(evidence),
                Err(error) => unknown(error),
            }
        }
        _ => return Err(()),
    };
    let output = serde_json::to_vec(&reply).map_err(|_| ())?;
    if output.len() > HELPER_BYTES {
        return Err(());
    }
    Ok(output)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NativeJobBinding {
    schema: u32,
    store_root: PathBuf,
    store_identity: FsIdentity,
    database_identity: FsIdentity,
    project_root: PathBuf,
    project_identity: FsIdentity,
    account_root: PathBuf,
    account_identity: FsIdentity,
    boundary_base_identity: FsIdentity,
    parent_index_identity: FsIdentity,
    indexed_parent_identity: FsIdentity,
    host: KernelWitness,
    operation_id: String,
    task_id: String,
    attempt_id: String,
    generation: u64,
    account_id: String,
    auth_revision: u64,
    ownership_marker: String,
    captured_records: Vec<BoundaryPin>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NativeCompletionEvidence {
    schema: u32,
    operation_id: String,
    task_id: String,
    attempt_id: String,
    generation: u64,
    account_id: String,
    auth_revision: u64,
    boot: String,
    outcome: CompletionOutcome,
    retired_coalitions: Vec<u64>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CompletionOutcome {
    Completed,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FsIdentity {
    device: u64,
    inode: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct KernelWitness {
    boot: String,
    release: String,
    build: String,
    version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BoundaryPin {
    operation_id: String,
    #[serde(default = "legacy_boundary_schema")]
    record_schema: u32,
    #[serde(default)]
    release_attempted_at_capture: bool,
    path: PathBuf,
    directory_identity: FsIdentity,
    capture_phase: BoundaryPhase,
    scope: BoundaryScope,
    host: KernelWitness,
    label: String,
    domain: String,
    worker: PathBuf,
    worker_digest: String,
    worker_file_identity: FsIdentity,
    spec_digest: String,
    member: BoundaryMember,
    held_file_identity: FsIdentity,
    held_digest: String,
    job_file_identity: FsIdentity,
    job_digest: String,
}

fn legacy_boundary_schema() -> u32 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BoundaryScope {
    operation_id: String,
    parent_operation_id: Option<String>,
    physical_account_root: Option<PathBuf>,
    storage_root: Option<PathBuf>,
    account_id: String,
    auth_revision: u64,
    task_id: Option<String>,
    attempt_id: Option<String>,
    generation: Option<u64>,
    project_root: PathBuf,
    purpose: BoundaryPurpose,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum BoundaryPurpose {
    Attempt,
    VersionProbe,
    Resolver,
    Collector,
    AccountTerminal,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BoundaryMember {
    identity: super::process_identity::Identity,
    pid_version: u32,
    resource_coalition: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum BoundaryPhase {
    Intent,
    Held,
    ReleaseIntent,
    Released,
    Sealed,
    Retired,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BoundaryRecord {
    schema: u32,
    #[serde(default)]
    release_attempted: bool,
    scope: BoundaryScope,
    host: KernelWitness,
    label: String,
    domain: String,
    worker: PathBuf,
    worker_digest: String,
    spec_digest: String,
    phase: BoundaryPhase,
    held: Option<BoundaryMember>,
    seal_confirmed: bool,
    #[serde(default)]
    native_exit: Option<i32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HeldRecord {
    schema: u32,
    operation_id: String,
    spec_digest: String,
    member: BoundaryMember,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseRecord {
    operation_id: String,
    spec_digest: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeEvidenceUnknown {
    UnsupportedPlatform,
    KernelNotAdmitted,
    SourceUnavailable,
    InvalidProvenance,
    ObligationsPending,
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
pub(crate) fn capture_prelaunch(
    _store_root: &std::path::Path,
    _parent_operation_id: &str,
) -> Result<NativeJobBinding, NativeEvidenceUnknown> {
    Err(NativeEvidenceUnknown::UnsupportedPlatform)
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
pub(crate) fn verify_native_job(
    _binding: &NativeJobBinding,
) -> Result<NativeCompletionEvidence, NativeEvidenceUnknown> {
    Err(NativeEvidenceUnknown::UnsupportedPlatform)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub(crate) fn native_host_kernel_admitted() -> bool {
    admitted::native_host_kernel_admitted()
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
pub(crate) fn native_host_kernel_admitted() -> bool {
    false
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod admitted {
    use super::*;
    use crate::agent_runtime::{
        native_wire::{NativeOutcome, NativePermission, NativeTool, ToolState},
        process_identity::{AuditToken, Identity},
        service,
        store::workspace,
        types::{HistoryCheckpoint, NativeOperation, Task, TaskState},
    };
    use rusqlite::{Connection, OpenFlags, OptionalExtension};
    use serde::de::DeserializeOwned;
    use sha2::{Digest, Sha256};
    use std::{
        collections::{BTreeMap, BTreeSet},
        ffi::CStr,
        fs,
        io::{Cursor, Read},
        os::unix::fs::{MetadataExt, OpenOptionsExt},
        path::{Path, PathBuf},
        time::Duration,
    };

    const MAX_JSON: u64 = 65_536;
    const MAX_ROWS: usize = 1_000_000;
    const APP_ID: i64 = 1_280_265_545;
    const DB_SCHEMA: i64 = 6;
    const HOST_VERSION: &str = "Darwin Kernel Version 27.0.0: Tue Aug 11 21:05:41 PDT 2026; root:xnu-13432.1.9~1/RELEASE_ARM64_T8122";

    fn unknown(_: impl std::fmt::Debug) -> NativeEvidenceUnknown {
        NativeEvidenceUnknown::SourceUnavailable
    }

    fn valid_id(value: &str, max: usize) -> bool {
        !value.is_empty()
            && value.len() <= max
            && !value.chars().any(char::is_control)
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':'))
    }

    fn digest(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn valid_digest(value: &str) -> bool {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }

    fn fs_identity(meta: &fs::Metadata) -> FsIdentity {
        FsIdentity {
            device: meta.dev(),
            inode: meta.ino(),
        }
    }

    fn private_directory(path: &Path) -> Result<FsIdentity, NativeEvidenceUnknown> {
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(path)
            .map_err(unknown)?;
        lomi_platform::unix::reject_acl_grants_readonly(&file)
            .map_err(|_| NativeEvidenceUnknown::SourceUnavailable)?;
        let meta = file.metadata().map_err(unknown)?;
        let path_meta = fs::symlink_metadata(path).map_err(unknown)?;
        if !meta.is_dir()
            || path_meta.file_type().is_symlink()
            || fs_identity(&path_meta) != fs_identity(&meta)
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o077 != 0
        {
            return Err(NativeEvidenceUnknown::SourceUnavailable);
        }
        Ok(fs_identity(&meta))
    }

    fn private_file(
        path: &Path,
        limit: u64,
    ) -> Result<(Vec<u8>, FsIdentity, String), NativeEvidenceUnknown> {
        let mut file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)
            .map_err(unknown)?;
        lomi_platform::unix::reject_acl_grants_readonly(&file)
            .map_err(|_| NativeEvidenceUnknown::SourceUnavailable)?;
        let before = file.metadata().map_err(unknown)?;
        if !before.is_file()
            || before.uid() != unsafe { libc::geteuid() }
            || before.mode() & 0o077 != 0
            || before.nlink() != 1
            || before.len() > limit
        {
            return Err(NativeEvidenceUnknown::SourceUnavailable);
        }
        let stamp = (
            before.dev(),
            before.ino(),
            before.len(),
            before.mtime(),
            before.mtime_nsec(),
        );
        let mut bytes = Vec::with_capacity(before.len() as usize);
        file.by_ref()
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(unknown)?;
        let after = file.metadata().map_err(unknown)?;
        if bytes.len() as u64 != before.len()
            || bytes.len() as u64 > limit
            || (
                after.dev(),
                after.ino(),
                after.len(),
                after.mtime(),
                after.mtime_nsec(),
            ) != stamp
        {
            return Err(NativeEvidenceUnknown::SourceUnavailable);
        }
        let content_digest = digest(&bytes);
        Ok((bytes, fs_identity(&before), content_digest))
    }

    fn private_file_identity(path: &Path, limit: u64) -> Result<FsIdentity, NativeEvidenceUnknown> {
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)
            .map_err(unknown)?;
        lomi_platform::unix::reject_acl_grants_readonly(&file)
            .map_err(|_| NativeEvidenceUnknown::SourceUnavailable)?;
        let meta = file.metadata().map_err(unknown)?;
        if !meta.is_file()
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o077 != 0
            || meta.nlink() != 1
            || meta.len() > limit
        {
            return Err(NativeEvidenceUnknown::SourceUnavailable);
        }
        Ok(fs_identity(&meta))
    }

    fn executable_digest(path: &Path) -> Result<(FsIdentity, String), NativeEvidenceUnknown> {
        if path.canonicalize().map_err(unknown)? != path {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        let mut file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(unknown)?;
        lomi_platform::unix::reject_acl_grants_readonly(&file)
            .map_err(|_| NativeEvidenceUnknown::SourceUnavailable)?;
        let before = file.metadata().map_err(unknown)?;
        if !before.is_file()
            || before.uid() != unsafe { libc::geteuid() }
            || before.mode() & 0o022 != 0
            || before.nlink() != 1
            || before.len() > 1024 * 1024 * 1024
        {
            return Err(NativeEvidenceUnknown::SourceUnavailable);
        }
        let stamp = (
            before.dev(),
            before.ino(),
            before.len(),
            before.mtime(),
            before.mtime_nsec(),
        );
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65_536];
        loop {
            let count = file.read(&mut buffer).map_err(unknown)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        let after = file.metadata().map_err(unknown)?;
        let path_after = fs::symlink_metadata(path).map_err(unknown)?;
        if (
            after.dev(),
            after.ino(),
            after.len(),
            after.mtime(),
            after.mtime_nsec(),
        ) != stamp
            || fs_identity(&path_after) != fs_identity(&before)
            || after.file_type().is_symlink()
        {
            return Err(NativeEvidenceUnknown::SourceUnavailable);
        }
        Ok((fs_identity(&before), format!("{:x}", hash.finalize())))
    }

    fn read_private_json<T: DeserializeOwned>(path: &Path) -> Result<T, NativeEvidenceUnknown> {
        let (bytes, _, _) = private_file(path, MAX_JSON)?;
        serde_json::from_slice(&bytes).map_err(unknown)
    }

    fn physical_directory(path: &Path) -> Result<(PathBuf, FsIdentity), NativeEvidenceUnknown> {
        let canonical = path.canonicalize().map_err(unknown)?;
        if canonical != path {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        let meta = fs::symlink_metadata(&canonical).map_err(unknown)?;
        if !meta.is_dir()
            || meta.file_type().is_symlink()
            || meta.uid() != unsafe { libc::geteuid() }
        {
            return Err(NativeEvidenceUnknown::SourceUnavailable);
        }
        Ok((canonical, fs_identity(&meta)))
    }

    fn private_database(root: &Path) -> Result<FsIdentity, NativeEvidenceUnknown> {
        let path = root.join("runtime.sqlite");
        let identity = private_file_identity(&path, 1024 * 1024 * 1024)?;
        for suffix in ["-wal", "-shm"] {
            let sidecar = root.join(format!("runtime.sqlite{suffix}"));
            match fs::symlink_metadata(&sidecar) {
                Ok(meta) => {
                    let file = fs::OpenOptions::new()
                        .read(true)
                        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
                        .open(&sidecar)
                        .map_err(unknown)?;
                    lomi_platform::unix::reject_acl_grants_readonly(&file)
                        .map_err(|_| NativeEvidenceUnknown::SourceUnavailable)?;
                    let held = file.metadata().map_err(unknown)?;
                    if !meta.is_file()
                        || meta.file_type().is_symlink()
                        || held.uid() != unsafe { libc::geteuid() }
                        || held.mode() & 0o077 != 0
                        || held.nlink() != 1
                        || fs_identity(&meta) != fs_identity(&held)
                    {
                        return Err(NativeEvidenceUnknown::SourceUnavailable);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(unknown(error)),
            }
        }
        Ok(identity)
    }

    fn open_snapshot(root: &Path) -> Result<Connection, NativeEvidenceUnknown> {
        let database = root.join("runtime.sqlite");
        let connection = Connection::open_with_flags(
            database,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(unknown)?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(unknown)?;
        connection
            .execute_batch("PRAGMA trusted_schema=OFF; PRAGMA query_only=ON; BEGIN DEFERRED;")
            .map_err(unknown)?;
        let application_id: i64 = connection
            .pragma_query_value(None, "application_id", |row| row.get(0))
            .map_err(unknown)?;
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(unknown)?;
        if application_id != APP_ID || version != DB_SCHEMA {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        validate_columns(&connection)?;
        Ok(connection)
    }

    fn validate_columns(connection: &Connection) -> Result<(), NativeEvidenceUnknown> {
        for (table, expected) in [
            (
                "tasks",
                &["id", "record", "shell", "directory_identity"][..],
            ),
            ("checkpoints", &["task_id", "record"][..]),
            (
                "process_ownership",
                &[
                    "task_id",
                    "attempt_id",
                    "account_id",
                    "generation",
                    "process_group",
                    "identity",
                    "ownership_marker",
                    "boot",
                    "unknown_tools",
                    "state",
                ][..],
            ),
            ("native_operations", &["id", "record"][..]),
            (
                "outbox",
                &[
                    "operation_id",
                    "task_id",
                    "attempt_id",
                    "generation",
                    "state",
                ][..],
            ),
        ] {
            let mut statement = connection
                .prepare(&format!("PRAGMA table_info({table})"))
                .map_err(unknown)?;
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(unknown)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(unknown)?;
            if columns.iter().map(String::as_str).collect::<Vec<_>>() != expected {
                return Err(NativeEvidenceUnknown::InvalidProvenance);
            }
        }
        Ok(())
    }

    fn read_task(
        connection: &Connection,
        task_id: &str,
    ) -> Result<(Task, (u64, u64)), NativeEvidenceUnknown> {
        let (raw, identity): (String, String) = connection
            .query_row(
                "SELECT record,directory_identity FROM tasks WHERE id=?1",
                [task_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(unknown)?;
        if raw.len() > 64 * 1024 * 1024 {
            return Err(NativeEvidenceUnknown::SourceUnavailable);
        }
        let task: Task = serde_json::from_str(&raw).map_err(unknown)?;
        let identity = serde_json::from_str::<(u64, u64)>(&identity).map_err(unknown)?;
        if task.task_id != task_id {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        Ok((task, identity))
    }

    fn read_checkpoint(
        connection: &Connection,
        task_id: &str,
    ) -> Result<Option<HistoryCheckpoint>, NativeEvidenceUnknown> {
        let raw: Option<String> = connection
            .query_row(
                "SELECT record FROM checkpoints WHERE task_id=?1",
                [task_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(unknown)?;
        raw.map(|raw| serde_json::from_str(&raw).map_err(unknown))
            .transpose()
    }

    fn read_operations(
        connection: &Connection,
    ) -> Result<Vec<NativeOperation>, NativeEvidenceUnknown> {
        let mut statement = connection
            .prepare("SELECT id,record FROM native_operations ORDER BY id")
            .map_err(unknown)?;
        let mut rows = statement.query([]).map_err(unknown)?;
        let mut operations = Vec::new();
        while let Some(row) = rows.next().map_err(unknown)? {
            if operations.len() >= MAX_ROWS {
                return Err(NativeEvidenceUnknown::SourceUnavailable);
            }
            let id: String = row.get(0).map_err(unknown)?;
            let raw: String = row.get(1).map_err(unknown)?;
            if raw.len() > MAX_JSON as usize {
                return Err(NativeEvidenceUnknown::SourceUnavailable);
            }
            let operation: NativeOperation = serde_json::from_str(&raw).map_err(unknown)?;
            if operation.operation_id != id || !valid_id(&id, 200) {
                return Err(NativeEvidenceUnknown::InvalidProvenance);
            }
            operations.push(operation);
        }
        Ok(operations)
    }

    #[derive(Clone, Serialize)]
    struct ProcessRow {
        attempt_id: String,
        account_id: String,
        generation: u64,
        process_group: Option<u32>,
        identity: Option<Identity>,
        ownership_marker: String,
        boot: String,
        unknown_tools: Vec<String>,
        state: String,
    }

    fn read_process_rows(
        connection: &Connection,
        task_id: &str,
    ) -> Result<Vec<ProcessRow>, NativeEvidenceUnknown> {
        let mut statement = connection
            .prepare("SELECT attempt_id,account_id,generation,process_group,identity,ownership_marker,boot,unknown_tools,state FROM process_ownership WHERE task_id=?1 ORDER BY attempt_id")
            .map_err(unknown)?;
        let mut rows = statement.query([task_id]).map_err(unknown)?;
        let mut result = Vec::new();
        while let Some(row) = rows.next().map_err(unknown)? {
            if result.len() >= MAX_ROWS {
                return Err(NativeEvidenceUnknown::SourceUnavailable);
            }
            let attempt_id: String = row.get(0).map_err(unknown)?;
            let account_id: String = row.get(1).map_err(unknown)?;
            let generation: i64 = row.get(2).map_err(unknown)?;
            let process_group: Option<i64> = row.get(3).map_err(unknown)?;
            let identity: Option<String> = row.get(4).map_err(unknown)?;
            let ownership_marker: String = row.get(5).map_err(unknown)?;
            let boot: String = row.get(6).map_err(unknown)?;
            let unknown_tools: String = row.get(7).map_err(unknown)?;
            let state: String = row.get(8).map_err(unknown)?;
            let generation =
                u64::try_from(generation).map_err(|_| NativeEvidenceUnknown::InvalidProvenance)?;
            let process_group = process_group
                .map(|group| {
                    u32::try_from(group)
                        .ok()
                        .filter(|group| *group >= 2 && *group <= i32::MAX as u32)
                        .ok_or(NativeEvidenceUnknown::InvalidProvenance)
                })
                .transpose()?;
            let identity = identity
                .map(|identity| serde_json::from_str(&identity).map_err(unknown))
                .transpose()?;
            let unknown_tools: Vec<String> =
                serde_json::from_str(&unknown_tools).map_err(unknown)?;
            if !valid_id(&attempt_id, 200)
                || !valid_id(&account_id, 200)
                || ownership_marker.len() != 64
                || !ownership_marker
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
                || boot.is_empty()
                || boot.len() > 1024
                || unknown_tools
                    .iter()
                    .any(|tool| tool.is_empty() || tool.len() > 1024)
                || !matches!(
                    state.as_str(),
                    "spawn_intent" | "spawned" | "drained" | "untracked"
                )
                || identity
                    .as_ref()
                    .is_some_and(|identity: &Identity| identity.boot != boot)
            {
                return Err(NativeEvidenceUnknown::InvalidProvenance);
            }
            result.push(ProcessRow {
                attempt_id,
                account_id,
                generation,
                process_group,
                identity,
                ownership_marker,
                boot,
                unknown_tools,
                state,
            });
        }
        Ok(result)
    }

    fn read_outbox(
        connection: &Connection,
        task_id: &str,
    ) -> Result<Vec<(String, String, u64, String)>, NativeEvidenceUnknown> {
        let mut statement = connection
            .prepare("SELECT operation_id,attempt_id,generation,state FROM outbox WHERE task_id=?1 ORDER BY operation_id")
            .map_err(unknown)?;
        let rows = statement
            .query_map([task_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(unknown)?;
        rows.map(|row| {
            let (operation, attempt, generation, state) = row.map_err(unknown)?;
            let generation =
                u64::try_from(generation).map_err(|_| NativeEvidenceUnknown::InvalidProvenance)?;
            if !valid_id(&operation, 200) || !valid_id(&attempt, 200) {
                return Err(NativeEvidenceUnknown::InvalidProvenance);
            }
            Ok((operation, attempt, generation, state))
        })
        .collect()
    }

    struct Snapshot {
        task: Task,
        directory_identity: (u64, u64),
        checkpoint: Option<HistoryCheckpoint>,
        operations: Vec<NativeOperation>,
        process_rows: Vec<ProcessRow>,
        outbox: Vec<(String, String, u64, String)>,
    }

    fn validate_process_provenance(snapshot: &Snapshot) -> Result<(), NativeEvidenceUnknown> {
        let mut seen = BTreeSet::new();
        for row in &snapshot.process_rows {
            if !seen.insert(row.attempt_id.as_str())
                || !snapshot.task.attempts.iter().any(|attempt| {
                    attempt.attempt_id == row.attempt_id
                        && attempt.account_id == row.account_id
                        && attempt.generation == row.generation
                })
            {
                return Err(NativeEvidenceUnknown::InvalidProvenance);
            }
        }
        Ok(())
    }

    fn read_snapshot(root: &Path, task_id: &str) -> Result<Snapshot, NativeEvidenceUnknown> {
        let database_identity = private_database(root)?;
        let connection = open_snapshot(root)?;
        let (task, directory_identity) = read_task(&connection, task_id)?;
        let checkpoint = read_checkpoint(&connection, task_id)?;
        let operations = read_operations(&connection)?;
        let process_rows = read_process_rows(&connection, task_id)?;
        let outbox = read_outbox(&connection, task_id)?;
        connection.execute_batch("COMMIT").map_err(unknown)?;
        if private_database(root)? != database_identity {
            return Err(NativeEvidenceUnknown::SourceUnavailable);
        }
        let snapshot = Snapshot {
            task,
            directory_identity,
            checkpoint,
            operations,
            process_rows,
            outbox,
        };
        validate_process_provenance(&snapshot)?;
        Ok(snapshot)
    }

    fn snapshot_digest(snapshot: &Snapshot) -> Result<String, NativeEvidenceUnknown> {
        let bytes = serde_json::to_vec(&(
            &snapshot.task,
            snapshot.directory_identity,
            &snapshot.checkpoint,
            &snapshot.operations,
            &snapshot.process_rows,
            &snapshot.outbox,
        ))
        .map_err(unknown)?;
        Ok(digest(&bytes))
    }

    fn current_kernel_witness() -> Result<KernelWitness, NativeEvidenceUnknown> {
        fn read(name: &CStr) -> Result<String, NativeEvidenceUnknown> {
            let mut output = [0u8; 4096];
            let mut length = output.len();
            let status = unsafe {
                libc::sysctlbyname(
                    name.as_ptr(),
                    output.as_mut_ptr().cast(),
                    &mut length,
                    std::ptr::null_mut(),
                    0,
                )
            };
            if status != 0 || length == 0 || length > output.len() {
                return Err(NativeEvidenceUnknown::KernelNotAdmitted);
            }
            CStr::from_bytes_until_nul(&output[..length])
                .map_err(|_| NativeEvidenceUnknown::KernelNotAdmitted)?
                .to_str()
                .map(str::to_owned)
                .map_err(|_| NativeEvidenceUnknown::KernelNotAdmitted)
        }
        let value = KernelWitness {
            boot: read(c"kern.bootsessionuuid")?,
            release: read(c"kern.osrelease")?,
            build: read(c"kern.osversion")?,
            version: read(c"kern.version")?,
        };
        if value.boot.is_empty()
            || value.release != "27.0.0"
            || value.build != "26A434"
            || value.version != HOST_VERSION
        {
            return Err(NativeEvidenceUnknown::KernelNotAdmitted);
        }
        Ok(value)
    }

    pub(super) fn native_host_kernel_admitted() -> bool {
        current_kernel_witness().is_ok()
    }

    #[repr(C)]
    #[derive(Default)]
    struct CoalitionInfo {
        ids: [u64; 2],
        reserved: [u64; 3],
    }

    fn current_member(pid: u32) -> Result<BoundaryMember, NativeEvidenceUnknown> {
        let before = AuditToken::fresh(pid).map_err(unknown)?;
        let identity = Identity::read(pid).map_err(unknown)?;
        let mut info = CoalitionInfo::default();
        let size = std::mem::size_of::<CoalitionInfo>();
        let bytes = unsafe {
            libc::proc_pidinfo(
                pid as i32,
                20,
                0,
                (&mut info as *mut CoalitionInfo).cast(),
                size as i32,
            )
        };
        let after = AuditToken::fresh(pid).map_err(unknown)?;
        if size != 40
            || bytes != 40
            || info.ids[0] <= 1
            || info.reserved != [0; 3]
            || !before.matches(&after)
        {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        Ok(BoundaryMember {
            identity,
            pid_version: after.pid_version(),
            resource_coalition: info.ids[0],
        })
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum CoalitionUsage {
        Alive,
        Retired,
    }

    fn coalition_usage(
        coalition: u64,
        expected_host: &KernelWitness,
    ) -> Result<CoalitionUsage, NativeEvidenceUnknown> {
        if coalition <= 1 {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        if current_kernel_witness()? != *expected_host {
            return Err(NativeEvidenceUnknown::KernelNotAdmitted);
        }
        struct Library(*mut libc::c_void);
        impl Drop for Library {
            fn drop(&mut self) {
                unsafe { libc::dlclose(self.0) };
            }
        }
        let library = Library(unsafe {
            libc::dlopen(
                c"/usr/lib/libSystem.B.dylib".as_ptr(),
                libc::RTLD_NOW | libc::RTLD_LOCAL,
            )
        });
        if library.0.is_null() {
            return Err(NativeEvidenceUnknown::KernelNotAdmitted);
        }
        let symbol = unsafe { libc::dlsym(library.0, c"coalition_info_resource_usage".as_ptr()) };
        if symbol.is_null() {
            return Err(NativeEvidenceUnknown::KernelNotAdmitted);
        }
        let query: unsafe extern "C" fn(u64, *mut libc::c_void, usize) -> i32 =
            unsafe { std::mem::transmute(symbol) };
        let mut ignored = [0u64; 2];
        let status = unsafe {
            query(
                coalition,
                ignored.as_mut_ptr().cast(),
                std::mem::size_of_val(&ignored),
            )
        };
        if status == 0 {
            return Ok(CoalitionUsage::Alive);
        }
        let error = std::io::Error::last_os_error().raw_os_error();
        if status == -1 && error == Some(libc::ESRCH) {
            return Ok(CoalitionUsage::Retired);
        }
        Err(NativeEvidenceUnknown::KernelNotAdmitted)
    }

    fn parent_digest(parent: &str) -> String {
        digest(parent.as_bytes())
    }

    fn validate_scope(
        scope: &BoundaryScope,
        parent: &str,
        operation: &NativeOperation,
        task: &Task,
        store_root: &Path,
    ) -> Result<(), NativeEvidenceUnknown> {
        let allowed = matches!(
            scope.purpose,
            BoundaryPurpose::Attempt | BoundaryPurpose::VersionProbe | BoundaryPurpose::Resolver
        );
        if !allowed
            || scope.parent_operation_id.as_deref() != Some(parent)
            || scope.physical_account_root.as_deref()
                != operation.physical_account_root.as_deref().map(Path::new)
            || scope.storage_root.as_deref() != Some(store_root)
            || scope.account_id != operation.account_id
            || scope.auth_revision != operation.auth_revision
            || scope.task_id.as_deref() != Some(task.task_id.as_str())
            || scope.attempt_id.as_deref() != operation.attempt_id.as_deref()
            || scope.generation != operation.generation
            || scope.project_root != PathBuf::from(&operation.cwd)
        {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        Ok(())
    }

    fn checked_record(
        root: &Path,
        parent: &str,
        operation: &NativeOperation,
        task: &Task,
        host: &KernelWitness,
        store_root: &Path,
    ) -> Result<(BoundaryRecord, BoundaryPin), NativeEvidenceUnknown> {
        let directory_identity = private_directory(root)?;
        let path_name = root.file_name().and_then(|name| name.to_str());
        let (raw, _, _) = private_file(&root.join("record.json"), MAX_JSON)?;
        let record: BoundaryRecord = serde_json::from_slice(&raw).map_err(unknown)?;
        let op_id = &record.scope.operation_id;
        if !matches!(record.schema, 1 | 2)
            || record.schema == 1 && record.release_attempted
            || !valid_id(op_id, 64)
            || op_id.len() != 32
            || !op_id.bytes().all(|byte| byte.is_ascii_hexdigit())
            || path_name != Some(op_id.as_str())
            || record.host != *host
            || record.label != format!("dev.lomi.agent.host.{op_id}")
            || record.domain != format!("gui/{}", unsafe { libc::getuid() })
            || !record.worker.is_absolute()
            || !valid_digest(&record.worker_digest)
            || !valid_digest(&record.spec_digest)
            || record
                .native_exit
                .is_some_and(|exit| !(-255..=255).contains(&exit))
        {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        validate_scope(&record.scope, parent, operation, task, store_root)?;
        let member = record
            .held
            .as_ref()
            .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
        if member.resource_coalition <= 1
            || member.identity.boot != host.boot
            || member.identity.executable != record.worker
        {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        let (worker_file_identity, worker_digest) = executable_digest(&record.worker)?;
        if worker_digest != record.worker_digest {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        let held_path = root.join("held.json");
        let (held_bytes, held_file_identity, held_digest) = private_file(&held_path, MAX_JSON)?;
        let held: HeldRecord = serde_json::from_slice(&held_bytes).map_err(unknown)?;
        if held.schema != 1
            || held.operation_id != *op_id
            || held.spec_digest != record.spec_digest
            || held.member != *member
        {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        let job_path = root.join("job.plist");
        let (job_bytes, job_file_identity, job_digest) = private_file(&job_path, 1024 * 1024)?;
        let job: plist::Value =
            plist::Value::from_reader_xml(Cursor::new(job_bytes)).map_err(unknown)?;
        let dictionary = job
            .as_dictionary()
            .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
        let args = dictionary
            .get("ProgramArguments")
            .and_then(plist::Value::as_array)
            .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
        if dictionary.len() != 5
            || dictionary.get("Label").and_then(plist::Value::as_string)
                != Some(record.label.as_str())
            || dictionary
                .get("RunAtLoad")
                .and_then(plist::Value::as_boolean)
                != Some(true)
            || dictionary
                .get("KeepAlive")
                .and_then(plist::Value::as_boolean)
                != Some(false)
            || dictionary
                .get("ProcessType")
                .and_then(plist::Value::as_string)
                != Some("Background")
            || args.len() != 3
            || args[0].as_string() != record.worker.to_str()
            || args[1].as_string() != Some("--agent-runtime-host-worker")
            || args[2].as_string() != root.to_str()
        {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        let pin = BoundaryPin {
            operation_id: op_id.clone(),
            record_schema: record.schema,
            release_attempted_at_capture: record.release_attempted,
            path: root.to_path_buf(),
            directory_identity,
            capture_phase: record.phase,
            scope: record.scope.clone(),
            host: record.host.clone(),
            label: record.label.clone(),
            domain: record.domain.clone(),
            worker: record.worker.clone(),
            worker_digest: record.worker_digest.clone(),
            worker_file_identity,
            spec_digest: record.spec_digest.clone(),
            member: member.clone(),
            held_file_identity,
            held_digest,
            job_file_identity,
            job_digest,
        };
        Ok((record, pin))
    }

    fn scan_parent_roots(
        root: &Path,
        parent: &str,
    ) -> Result<(FsIdentity, FsIdentity, FsIdentity, Vec<PathBuf>), NativeEvidenceUnknown> {
        let base = root.join("host-boundaries");
        let base_identity = private_directory(&base)?;
        let parents = base.join("parents");
        let parents_identity = private_directory(&parents)?;
        let indexed = parents.join(parent_digest(parent));
        let indexed_identity = private_directory(&indexed)?;
        let mut result = Vec::new();
        for (index, entry) in fs::read_dir(&indexed).map_err(unknown)?.enumerate() {
            if index >= 4096 {
                return Err(NativeEvidenceUnknown::SourceUnavailable);
            }
            let path = entry.map_err(unknown)?.path();
            private_directory(&path)?;
            result.push(path);
        }
        let mut legacy_count = 0usize;
        for entry in fs::read_dir(&base).map_err(unknown)? {
            let path = entry.map_err(unknown)?.path();
            if path == parents {
                continue;
            }
            legacy_count += 1;
            if legacy_count > 4096 {
                return Err(NativeEvidenceUnknown::SourceUnavailable);
            }
            private_directory(&path)?;
            let raw = read_private_json::<BoundaryRecord>(&path.join("record.json"))?;
            if raw.scope.parent_operation_id.as_deref() == Some(parent) {
                result.push(path);
            }
        }
        result.sort();
        if result.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        Ok((base_identity, parents_identity, indexed_identity, result))
    }

    fn absent(path: &Path) -> Result<bool, NativeEvidenceUnknown> {
        match fs::symlink_metadata(path) {
            Ok(_) => Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(unknown(error)),
        }
    }

    fn read_release(
        path: &Path,
        operation_id: &str,
        spec_digest: &str,
    ) -> Result<(), NativeEvidenceUnknown> {
        let release: ReleaseRecord = read_private_json(&path.join("release.json"))?;
        if release.operation_id != operation_id || release.spec_digest != spec_digest {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        Ok(())
    }

    fn validate_inventory(
        root: &Path,
        parent: &str,
        operation: &NativeOperation,
        task: &Task,
        host: &KernelWitness,
        captured: Option<&[BoundaryPin]>,
        capturing: bool,
        allow_no_start: bool,
    ) -> Result<(FsIdentity, FsIdentity, FsIdentity, Vec<BoundaryPin>), NativeEvidenceUnknown> {
        let (base_identity, parents_identity, indexed_identity, roots) =
            scan_parent_roots(root, parent)?;
        if roots.is_empty() {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        let mut pins = Vec::with_capacity(roots.len());
        let mut attempt_count = 0usize;
        for path in roots {
            let (record, pin) = checked_record(&path, parent, operation, task, host, root)?;
            if pins
                .iter()
                .any(|prior: &BoundaryPin| prior.operation_id == pin.operation_id)
            {
                return Err(NativeEvidenceUnknown::InvalidProvenance);
            }
            match record.scope.purpose {
                BoundaryPurpose::Attempt => {
                    attempt_count += 1;
                    if record.scope.attempt_id.as_deref() != operation.attempt_id.as_deref()
                        || record.scope.operation_id.is_empty()
                    {
                        return Err(NativeEvidenceUnknown::InvalidProvenance);
                    }
                    if capturing {
                        if record.phase != BoundaryPhase::Held
                            || record.seal_confirmed
                            || record.schema != 2
                            || record.release_attempted
                            || !absent(&path.join("release.json"))?
                        {
                            return Err(NativeEvidenceUnknown::InvalidProvenance);
                        }
                        let current = current_member(pin.member.identity.pid)?;
                        let own = current_member(std::process::id())?;
                        if current != pin.member
                            || current.resource_coalition == own.resource_coalition
                            || coalition_usage(current.resource_coalition, host)?
                                != CoalitionUsage::Alive
                        {
                            return Err(NativeEvidenceUnknown::InvalidProvenance);
                        }
                    } else {
                        if record.phase != BoundaryPhase::Retired || !record.seal_confirmed {
                            return Err(NativeEvidenceUnknown::ObligationsPending);
                        }
                        if allow_no_start {
                            let captured_attempt = captured
                                .and_then(|pins| {
                                    pins.iter().find(|pin| {
                                        pin.scope.purpose == BoundaryPurpose::Attempt
                                            && pin.operation_id == record.scope.operation_id
                                    })
                                })
                                .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
                            if captured_attempt.record_schema != 2
                                || captured_attempt.release_attempted_at_capture
                                || record.schema != 2
                                || record.release_attempted
                                || !absent(&path.join("release.json"))?
                            {
                                return Err(NativeEvidenceUnknown::InvalidProvenance);
                            }
                        } else {
                            if record.schema == 2 && !record.release_attempted {
                                return Err(NativeEvidenceUnknown::InvalidProvenance);
                            }
                            read_release(&path, &record.scope.operation_id, &record.spec_digest)?;
                        }
                        if coalition_usage(pin.member.resource_coalition, host)?
                            != CoalitionUsage::Retired
                        {
                            return Err(NativeEvidenceUnknown::ObligationsPending);
                        }
                    }
                }
                _ => {
                    if record.phase != BoundaryPhase::Retired || !record.seal_confirmed {
                        return Err(NativeEvidenceUnknown::ObligationsPending);
                    }
                    if record.schema == 2 && !record.release_attempted {
                        return Err(NativeEvidenceUnknown::InvalidProvenance);
                    }
                    read_release(&path, &record.scope.operation_id, &record.spec_digest)?;
                    if coalition_usage(pin.member.resource_coalition, host)?
                        != CoalitionUsage::Retired
                    {
                        return Err(NativeEvidenceUnknown::ObligationsPending);
                    }
                }
            }
            pins.push(pin);
        }
        if attempt_count != 1 {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        if let Some(captured) = captured {
            for before in captured {
                if !pins
                    .iter()
                    .any(|current| same_record_source(current, before))
                {
                    return Err(NativeEvidenceUnknown::InvalidProvenance);
                }
            }
        }
        Ok((base_identity, parents_identity, indexed_identity, pins))
    }

    fn same_record_source(current: &BoundaryPin, captured: &BoundaryPin) -> bool {
        current.operation_id == captured.operation_id
            && current.record_schema == captured.record_schema
            && current.path == captured.path
            && current.directory_identity == captured.directory_identity
            && current.scope == captured.scope
            && current.host == captured.host
            && current.label == captured.label
            && current.domain == captured.domain
            && current.worker == captured.worker
            && current.worker_digest == captured.worker_digest
            && current.worker_file_identity == captured.worker_file_identity
            && current.spec_digest == captured.spec_digest
            && current.member == captured.member
            && current.held_file_identity == captured.held_file_identity
            && current.held_digest == captured.held_digest
            && current.job_file_identity == captured.job_file_identity
            && current.job_digest == captured.job_digest
    }

    fn operation_for_capture(
        snapshot: &Snapshot,
        parent: &str,
    ) -> Result<NativeOperation, NativeEvidenceUnknown> {
        let mut matching = snapshot
            .operations
            .iter()
            .filter(|operation| operation.operation_id == parent);
        let operation = matching
            .next()
            .cloned()
            .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
        if matching.next().is_some()
            || !operation.host_boundary_owned
            || operation.purpose != "attempt"
            || !operation.native_writer_participant
            || !operation.native_writer_bind_attempted
            || operation.native_writer_bind_rejected
            || operation.native_writer_no_start_observed
            || operation.state != "ownership_unknown"
            || operation.task_id.is_none()
            || operation.attempt_id.is_none()
            || operation.generation.is_none()
            || operation.physical_account_root.is_none()
        {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        Ok(operation)
    }

    fn validate_capture_store(
        store_root: &Path,
        operation: &NativeOperation,
        snapshot: &Snapshot,
        host: &KernelWitness,
    ) -> Result<(PathBuf, FsIdentity, PathBuf, FsIdentity, String), NativeEvidenceUnknown> {
        let task_id = operation
            .task_id
            .as_deref()
            .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
        let attempt_id = operation
            .attempt_id
            .as_deref()
            .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
        let generation = operation
            .generation
            .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
        if operation.boot != host.boot
            || operation.auth_revision == 0
            || operation.generation == Some(0)
            || !valid_id(task_id, 256)
            || !valid_id(attempt_id, 256)
            || !valid_id(&operation.account_id, 200)
            || !valid_id(&operation.operation_id, 200)
            || operation.cwd.is_empty()
            || operation
                .physical_account_root
                .as_ref()
                .is_none_or(|path| !Path::new(path).is_absolute())
        {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        let task = &snapshot.task;
        let attempt = task
            .attempts
            .iter()
            .find(|attempt| attempt.attempt_id == attempt_id)
            .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
        if task.task_id != task_id
            || task.generation != generation
            || task.active_attempt_id.as_deref() != Some(attempt_id)
            || task.active_account_id.as_deref() != Some(operation.account_id.as_str())
            || attempt.operation_id != operation.operation_id
            || attempt.account_id != operation.account_id
            || attempt.auth_revision != operation.auth_revision
            || attempt.generation != generation
            || attempt.state != "spawn_intent"
            || attempt.effects_state != "unsettled"
            || !matches!(task.state, TaskState::Starting | TaskState::Running)
        {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        let (physical_project, (project_device, project_inode)) =
            workspace(Path::new(&operation.cwd)).map_err(unknown)?;
        let project_identity = FsIdentity {
            device: project_device,
            inode: project_inode,
        };
        if physical_project != operation.cwd
            || snapshot.directory_identity != (project_device, project_inode)
        {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        let project_root = PathBuf::from(physical_project);
        let account_root = PathBuf::from(
            operation
                .physical_account_root
                .as_deref()
                .ok_or(NativeEvidenceUnknown::InvalidProvenance)?,
        );
        let (account_root, account_identity) = physical_directory(&account_root)?;
        if private_directory(&account_root)? != account_identity {
            return Err(NativeEvidenceUnknown::SourceUnavailable);
        }
        if account_root != PathBuf::from(operation.physical_account_root.as_deref().unwrap()) {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        let (store_root, store_identity) = physical_directory(store_root)?;
        if private_directory(&store_root)? != store_identity {
            return Err(NativeEvidenceUnknown::SourceUnavailable);
        }
        if private_database(&store_root).is_err() {
            return Err(NativeEvidenceUnknown::SourceUnavailable);
        }
        if task.cwd != operation.cwd {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        let process_rows = &snapshot.process_rows;
        let mut current = process_rows
            .iter()
            .filter(|row| row.attempt_id == attempt_id);
        let row = current
            .next()
            .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
        if current.next().is_some()
            || row.account_id != operation.account_id
            || row.generation != generation
            || row.process_group.is_some()
            || row.identity.is_some()
            || row.state != "spawn_intent"
            || row.boot != host.boot
            || row.unknown_tools.as_slice() != ["unqualified-boundary"]
            || row.ownership_marker.len() != 64
        {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        if process_rows.iter().any(|other| {
            other.attempt_id != attempt_id
                && (other.state != "drained" || !other.unknown_tools.is_empty())
        }) {
            return Err(NativeEvidenceUnknown::ObligationsPending);
        }
        if snapshot.operations.iter().any(|other| {
            other.task_id.as_deref() == Some(task_id)
                && other.operation_id != operation.operation_id
                && other.state != "reviewed"
        }) {
            return Err(NativeEvidenceUnknown::ObligationsPending);
        }
        let matching_outbox = snapshot
            .outbox
            .iter()
            .filter(|(outbox_operation, outbox_attempt, outbox_generation, _)| {
                outbox_operation == &operation.operation_id
                    && outbox_attempt == attempt_id
                    && *outbox_generation == generation
            })
            .collect::<Vec<_>>();
        if matching_outbox.len() != 1
            || matching_outbox[0].3 != "pending"
            || snapshot.outbox.iter().any(
                |(outbox_operation, outbox_attempt, outbox_generation, state)| {
                    if outbox_operation == &operation.operation_id
                        && outbox_attempt == attempt_id
                        && *outbox_generation == generation
                    {
                        false
                    } else {
                        state != "settled"
                    }
                },
            )
        {
            return Err(NativeEvidenceUnknown::ObligationsPending);
        }
        Ok((
            project_root,
            project_identity,
            account_root,
            account_identity,
            row.ownership_marker.clone(),
        ))
    }

    pub(crate) fn capture_prelaunch(
        store_root: &Path,
        parent_operation_id: &str,
    ) -> Result<NativeJobBinding, NativeEvidenceUnknown> {
        if !valid_id(parent_operation_id, 200) {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        let host = current_kernel_witness()?;
        let (store_root, store_identity) = physical_directory(store_root)?;
        let database_identity = private_database(&store_root)?;
        let connection = open_snapshot(&store_root)?;
        let operations = read_operations(&connection)?;
        let operation = operations
            .iter()
            .find(|operation| operation.operation_id == parent_operation_id)
            .cloned()
            .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
        let task_id = operation
            .task_id
            .as_deref()
            .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
        let (task, directory_identity) = read_task(&connection, task_id)?;
        let process_rows = read_process_rows(&connection, task_id)?;
        let snapshot = Snapshot {
            task,
            directory_identity,
            checkpoint: read_checkpoint(&connection, task_id)?,
            operations,
            process_rows,
            outbox: read_outbox(&connection, task_id)?,
        };
        validate_process_provenance(&snapshot)?;
        connection.execute_batch("COMMIT").map_err(unknown)?;
        if private_database(&store_root)? != database_identity {
            return Err(NativeEvidenceUnknown::SourceUnavailable);
        }
        let operation = operation_for_capture(&snapshot, parent_operation_id)?;
        let (project_root, project_identity, account_root, account_identity, ownership_marker) =
            validate_capture_store(&store_root, &operation, &snapshot, &host)?;
        let (boundary_base_identity, parent_index_identity, indexed_parent_identity, records) =
            validate_inventory(
                &store_root,
                parent_operation_id,
                &operation,
                &snapshot.task,
                &host,
                None,
                true,
                false,
            )?;
        let confirmation = read_snapshot(&store_root, &operation.task_id.as_deref().unwrap())?;
        if snapshot_digest(&snapshot)? != snapshot_digest(&confirmation)? {
            return Err(NativeEvidenceUnknown::SourceUnavailable);
        }
        let (base_identity, parents_identity, indexed_identity, confirmed_records) =
            validate_inventory(
                &store_root,
                parent_operation_id,
                &operation,
                &confirmation.task,
                &host,
                None,
                true,
                false,
            )?;
        if boundary_base_identity != base_identity
            || parent_index_identity != parents_identity
            || indexed_parent_identity != indexed_identity
            || records != confirmed_records
        {
            return Err(NativeEvidenceUnknown::SourceUnavailable);
        }
        if records.is_empty()
            || records
                .iter()
                .filter(|record| record.scope.purpose == BoundaryPurpose::Attempt)
                .count()
                != 1
        {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        Ok(NativeJobBinding {
            schema: 1,
            store_root,
            store_identity,
            database_identity,
            project_root,
            project_identity,
            account_root,
            account_identity,
            boundary_base_identity,
            parent_index_identity,
            indexed_parent_identity,
            host,
            operation_id: operation.operation_id.clone(),
            task_id: operation.task_id.unwrap(),
            attempt_id: operation.attempt_id.unwrap(),
            generation: operation.generation.unwrap(),
            account_id: operation.account_id,
            auth_revision: operation.auth_revision,
            ownership_marker,
            captured_records: records,
        })
    }

    fn validate_history(
        task: &Task,
        binding: &NativeJobBinding,
        no_start: bool,
    ) -> Result<CompletionOutcome, NativeEvidenceUnknown> {
        if task.history_revision as usize != task.history.len()
            || task
                .history
                .iter()
                .enumerate()
                .any(|(index, record)| record.sequence != index as u64 + 1)
        {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        for attempt in &task.attempts {
            let terminal_attempt =
                matches!(attempt.state.as_str(), "completed" | "stopped" | "rejected");
            let effects_durable = attempt.effects_state == "settled"
                || (attempt.state == "stopped" && attempt.effects_state == "reviewed");
            if !terminal_attempt || !effects_durable {
                return Err(NativeEvidenceUnknown::ObligationsPending);
            }
        }
        let current_attempt = task
            .attempts
            .iter()
            .find(|attempt| attempt.attempt_id == binding.attempt_id)
            .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
        if no_start
            && (current_attempt.state != "rejected"
                || current_attempt.effects_state != "settled"
                || current_attempt.native_ref.is_some()
                || current_attempt.version.is_some()
                || !current_attempt.output.is_empty()
                || task
                    .history
                    .iter()
                    .any(|record| record.attempt_id == binding.attempt_id))
        {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        let mut tools: BTreeMap<(String, String, String, String), ToolState> = BTreeMap::new();
        let mut permissions: BTreeMap<(String, String), (usize, usize)> = BTreeMap::new();
        let mut terminal_attempts = BTreeSet::new();
        let mut current_terminal = None;
        let mut current_terminal_session = None;
        let mut current_user_records = 0usize;
        for record in &task.history {
            if record.sequence == 0
                || !valid_id(&record.attempt_id, 256)
                || !valid_id(&record.account_id, 200)
                || record.auth_revision == 0
            {
                return Err(NativeEvidenceUnknown::InvalidProvenance);
            }
            let attempt = task
                .attempts
                .iter()
                .find(|attempt| attempt.attempt_id == record.attempt_id)
                .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
            if attempt.account_id != record.account_id
                || attempt.auth_revision != record.auth_revision
                || terminal_attempts.contains(&record.attempt_id)
                    && !matches!(record.kind.as_str(), "accepted" | "observation")
            {
                return Err(NativeEvidenceUnknown::InvalidProvenance);
            }
            match record.kind.as_str() {
                "tool" => {
                    let tool: NativeTool =
                        serde_json::from_value(record.content.clone()).map_err(unknown)?;
                    let expected_state = match &tool.state {
                        ToolState::Started => "started",
                        ToolState::Pending => "pending",
                        ToolState::Completed => "completed",
                        ToolState::Failed => "failed",
                    };
                    if tool.tool_id.is_empty()
                        || tool.tool_id.len() > 1024
                        || tool.session_id.is_empty()
                        || tool.turn_id.is_empty()
                        || record.state != expected_state
                    {
                        return Err(NativeEvidenceUnknown::InvalidProvenance);
                    }
                    if record.attempt_id == binding.attempt_id
                        && current_attempt.native_ref.as_deref() != Some(tool.session_id.as_str())
                    {
                        return Err(NativeEvidenceUnknown::InvalidProvenance);
                    }
                    let key = (
                        record.attempt_id.clone(),
                        tool.session_id,
                        tool.turn_id,
                        tool.tool_id,
                    );
                    if let Some(previous) = tools.get(&key) {
                        if matches!(previous, ToolState::Completed | ToolState::Failed) {
                            return Err(NativeEvidenceUnknown::InvalidProvenance);
                        }
                    }
                    tools.insert(key, tool.state);
                }
                "permission" if record.state == "pending" => {
                    let permission: NativePermission =
                        serde_json::from_value(record.content.clone()).map_err(unknown)?;
                    let id = serde_json::to_string(&permission.request_id).map_err(unknown)?;
                    if id.len() > 4096 || id == "null" {
                        return Err(NativeEvidenceUnknown::InvalidProvenance);
                    }
                    if record.attempt_id == binding.attempt_id
                        && current_attempt.native_ref.as_deref()
                            != Some(permission.session_id.as_str())
                    {
                        return Err(NativeEvidenceUnknown::InvalidProvenance);
                    }
                    let item = permissions
                        .entry((record.attempt_id.clone(), id))
                        .or_default();
                    item.0 += 1;
                }
                "permission_decision" if record.state == "committed" => {
                    let id = record
                        .content
                        .get("requestId")
                        .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
                    let id = serde_json::to_string(id).map_err(unknown)?;
                    if record
                        .content
                        .get("allow")
                        .and_then(serde_json::Value::as_bool)
                        .is_none()
                        || record
                            .content
                            .get("choice")
                            .and_then(serde_json::Value::as_str)
                            .is_none()
                    {
                        return Err(NativeEvidenceUnknown::InvalidProvenance);
                    }
                    let item = permissions
                        .entry((record.attempt_id.clone(), id))
                        .or_default();
                    item.1 += 1;
                }
                "permission_withdrawn" if record.state == "settled" => {
                    let id = record
                        .content
                        .get("requestId")
                        .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
                    let id = serde_json::to_string(id).map_err(unknown)?;
                    let item = permissions
                        .entry((record.attempt_id.clone(), id))
                        .or_default();
                    item.1 += 1;
                }
                "terminal" if record.state == "observed" => {
                    let terminal: super::super::native_wire::NativeTerminal =
                        serde_json::from_value(record.content.clone()).map_err(unknown)?;
                    if record.attempt_id == binding.attempt_id {
                        if current_terminal.is_some()
                            || record.account_id != binding.account_id
                            || record.auth_revision != binding.auth_revision
                        {
                            return Err(NativeEvidenceUnknown::InvalidProvenance);
                        }
                        current_terminal = Some(terminal.outcome);
                        current_terminal_session = Some(terminal.session_id);
                    }
                    if !terminal_attempts.insert(record.attempt_id.clone()) {
                        return Err(NativeEvidenceUnknown::InvalidProvenance);
                    }
                }
                "user" => {
                    if record.attempt_id == binding.attempt_id {
                        current_user_records += 1;
                        if record.state != "committed"
                            || record.content.as_str() != Some(current_attempt.input.as_str())
                        {
                            return Err(NativeEvidenceUnknown::InvalidProvenance);
                        }
                    }
                }
                "assistant"
                | "accepted"
                | "session"
                | "turn_started"
                | "observation"
                | "error"
                | "legacy_attempt"
                | "legacy_record"
                | "legacy_account_record"
                | "legacy_tool"
                | "legacy_permission"
                | "legacy_native_observation" => {}
                _ => return Err(NativeEvidenceUnknown::InvalidProvenance),
            }
        }
        if tools
            .values()
            .any(|state| matches!(state, ToolState::Started | ToolState::Pending))
            || permissions
                .values()
                .any(|(pending, settled)| *pending != 1 || *settled != 1)
        {
            return Err(NativeEvidenceUnknown::ObligationsPending);
        }
        if no_start {
            if current_user_records != 0 || current_terminal.is_some() {
                return Err(NativeEvidenceUnknown::InvalidProvenance);
            }
            return Ok(CompletionOutcome::Stopped);
        }
        if current_user_records != 1
            || current_attempt.native_ref.as_deref() != current_terminal_session.as_deref()
        {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        match (
            &task.state,
            current_attempt.state.as_str(),
            current_terminal,
        ) {
            (TaskState::Completed, "completed", Some(NativeOutcome::Completed)) => {
                Ok(CompletionOutcome::Completed)
            }
            (
                TaskState::Stopped | TaskState::Prepared,
                "stopped",
                Some(NativeOutcome::Cancelled),
            ) => Ok(CompletionOutcome::Stopped),
            (TaskState::Stopped, "stopped", Some(NativeOutcome::Failed))
                if current_attempt.effects_state == "reviewed" =>
            {
                Ok(CompletionOutcome::Stopped)
            }
            _ => Err(NativeEvidenceUnknown::ObligationsPending),
        }
    }

    fn validate_completed_source(
        binding: &NativeJobBinding,
        snapshot: &Snapshot,
    ) -> Result<CompletionOutcome, NativeEvidenceUnknown> {
        if binding.schema != 1
            || binding.generation == 0
            || !valid_id(&binding.operation_id, 200)
            || !valid_id(&binding.task_id, 256)
            || !valid_id(&binding.attempt_id, 256)
            || !valid_id(&binding.account_id, 200)
            || binding.auth_revision == 0
            || binding.ownership_marker.len() != 64
            || !binding
                .ownership_marker
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || current_kernel_witness()? != binding.host
        {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        let operation = snapshot
            .operations
            .iter()
            .find(|operation| operation.operation_id == binding.operation_id)
            .cloned()
            .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
        let no_start = operation.native_writer_no_start_observed;
        if !operation.host_boundary_owned
            || !operation.native_writer_participant
            || !operation.native_writer_bind_attempted
            || operation.native_writer_bind_rejected
            || operation.state != "reviewed"
            || operation.purpose != "attempt"
            || operation.task_id.as_deref() != Some(binding.task_id.as_str())
            || operation.attempt_id.as_deref() != Some(binding.attempt_id.as_str())
            || operation.generation != Some(binding.generation)
            || operation.account_id != binding.account_id
            || operation.auth_revision != binding.auth_revision
            || operation.boot != binding.host.boot
            || operation.physical_account_root.as_deref()
                != Some(
                    binding
                        .account_root
                        .to_str()
                        .ok_or(NativeEvidenceUnknown::InvalidProvenance)?,
                )
            || operation.cwd
                != binding
                    .project_root
                    .to_str()
                    .ok_or(NativeEvidenceUnknown::InvalidProvenance)?
        {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        if snapshot.operations.iter().any(|other| {
            other.task_id.as_deref() == Some(binding.task_id.as_str()) && other.state != "reviewed"
        }) {
            return Err(NativeEvidenceUnknown::ObligationsPending);
        }
        let task = &snapshot.task;
        if task.task_id != binding.task_id
            || task.generation != binding.generation
            || task.active_attempt_id.is_some()
            || task.active_account_id.is_some()
            || task.cwd != operation.cwd
            || snapshot.directory_identity
                != (
                    binding.project_identity.device,
                    binding.project_identity.inode,
                )
            || workspace(Path::new(&task.cwd)).map_err(unknown)?
                != (task.cwd.clone(), snapshot.directory_identity)
        {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        for attempt in &task.attempts {
            let terminal_attempt =
                matches!(attempt.state.as_str(), "completed" | "stopped" | "rejected");
            let effects_durable = attempt.effects_state == "settled"
                || (attempt.state == "stopped" && attempt.effects_state == "reviewed");
            if !terminal_attempt || !effects_durable {
                return Err(NativeEvidenceUnknown::ObligationsPending);
            }
        }
        if no_start {
            let attempt = task
                .attempts
                .iter()
                .find(|attempt| attempt.attempt_id == binding.attempt_id)
                .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
            if task.state != TaskState::Stopped
                || attempt.state != "rejected"
                || attempt.effects_state != "settled"
            {
                return Err(NativeEvidenceUnknown::ObligationsPending);
            }
        }
        let process = &snapshot.process_rows;
        let current = process
            .iter()
            .find(|row| row.attempt_id == binding.attempt_id)
            .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
        if process
            .iter()
            .any(|row| row.state != "drained" || !row.unknown_tools.is_empty())
            || current.account_id != binding.account_id
            || current.generation != binding.generation
            || current.boot != binding.host.boot
            || current.ownership_marker != binding.ownership_marker
            || no_start && (current.process_group.is_some() || current.identity.is_some())
        {
            return Err(NativeEvidenceUnknown::ObligationsPending);
        }
        let matching_outbox = snapshot
            .outbox
            .iter()
            .filter(|(outbox_operation, attempt, outbox_generation, _)| {
                outbox_operation == &binding.operation_id
                    && attempt == &binding.attempt_id
                    && *outbox_generation == binding.generation
            })
            .collect::<Vec<_>>();
        if matching_outbox.len() != 1
            || matching_outbox[0].3 != "settled"
            || snapshot
                .outbox
                .iter()
                .any(|(outbox_operation, attempt, generation, state)| {
                    state != "settled"
                        || !task.attempts.iter().any(|saved| {
                            saved.operation_id == *outbox_operation
                                && saved.attempt_id == *attempt
                                && saved.generation == *generation
                        })
                })
        {
            return Err(NativeEvidenceUnknown::ObligationsPending);
        }
        if task
            .switches
            .iter()
            .any(|switch| !matches!(switch.phase.as_str(), "prepared" | "completed"))
        {
            return Err(NativeEvidenceUnknown::ObligationsPending);
        }
        let target = task
            .attempts
            .iter()
            .find(|attempt| attempt.attempt_id == binding.attempt_id)
            .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
        if no_start {
            let checkpoint_is_valid = snapshot.checkpoint.as_ref().is_none_or(|checkpoint| {
                task.attempts
                    .iter()
                    .find(|attempt| attempt.attempt_id == checkpoint.attempt_id)
                    .is_some_and(|attempt| {
                        checkpoint.task_id == binding.task_id
                            && checkpoint.attempt_id != binding.attempt_id
                            && checkpoint.account_id == attempt.account_id
                            && checkpoint.auth_revision == attempt.auth_revision
                            && checkpoint.generation == attempt.generation
                            && checkpoint.history_revision == task.history_revision
                            && service::history_digest(task)
                                .is_ok_and(|digest| checkpoint.digest == digest)
                            && checkpoint.model == task.model
                            && checkpoint.cwd_identity
                                == (
                                    binding.project_identity.device,
                                    binding.project_identity.inode,
                                )
                            && checkpoint.native_ref.as_str() != ""
                            && attempt.native_ref.as_deref() == Some(checkpoint.native_ref.as_str())
                            && attempt.version.as_deref() == Some(checkpoint.version.as_str())
                            && checkpoint.settled
                            && (attempt.effects_state == "settled"
                                || (attempt.state == "stopped"
                                    && attempt.effects_state == "reviewed"))
                    })
            });
            if !checkpoint_is_valid
                || target.operation_id != binding.operation_id
                || target.account_id != binding.account_id
                || target.auth_revision != binding.auth_revision
                || target.generation != binding.generation
                || target.native_ref.is_some()
            {
                return Err(NativeEvidenceUnknown::InvalidProvenance);
            }
        } else {
            let checkpoint = snapshot
                .checkpoint
                .as_ref()
                .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
            if checkpoint.task_id != binding.task_id
                || checkpoint.attempt_id != binding.attempt_id
                || checkpoint.account_id != binding.account_id
                || checkpoint.auth_revision != binding.auth_revision
                || checkpoint.generation != binding.generation
                || checkpoint.history_revision != task.history_revision
                || checkpoint.digest != service::history_digest(task).map_err(unknown)?
                || checkpoint.model != task.model
                || checkpoint.cwd_identity
                    != (
                        binding.project_identity.device,
                        binding.project_identity.inode,
                    )
                || checkpoint.native_ref.is_empty()
                || target.operation_id != binding.operation_id
                || target.account_id != binding.account_id
                || target.auth_revision != binding.auth_revision
                || target.generation != binding.generation
                || target.native_ref.as_deref() != Some(checkpoint.native_ref.as_str())
                || target.version.as_deref() != Some(checkpoint.version.as_str())
                || !checkpoint.settled
            {
                return Err(NativeEvidenceUnknown::InvalidProvenance);
            }
        }
        validate_history(task, binding, no_start)
    }

    fn validate_binding_manifest(binding: &NativeJobBinding) -> Result<(), NativeEvidenceUnknown> {
        if binding.captured_records.is_empty()
            || binding.captured_records.len() > 4096
            || binding.host.boot.is_empty()
            || binding.host.release != "27.0.0"
            || binding.host.build != "26A434"
            || binding.host.version != HOST_VERSION
        {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        let mut attempt_records = 0usize;
        let mut seen = BTreeSet::new();
        for pin in &binding.captured_records {
            if !seen.insert(pin.operation_id.as_str())
                || pin.host != binding.host
                || pin.scope.operation_id != pin.operation_id
                || pin.scope.parent_operation_id.as_deref() != Some(binding.operation_id.as_str())
                || pin.scope.storage_root.as_deref() != Some(binding.store_root.as_path())
                || pin.scope.physical_account_root.as_deref()
                    != Some(binding.account_root.as_path())
                || pin.scope.account_id != binding.account_id
                || pin.scope.auth_revision != binding.auth_revision
                || pin.scope.task_id.as_deref() != Some(binding.task_id.as_str())
                || pin.scope.attempt_id.as_deref() != Some(binding.attempt_id.as_str())
                || pin.scope.generation != Some(binding.generation)
                || pin.scope.project_root != binding.project_root
                || !valid_digest(&pin.worker_digest)
                || !valid_digest(&pin.spec_digest)
                || !valid_digest(&pin.held_digest)
                || !valid_digest(&pin.job_digest)
                || pin.member.identity.boot != binding.host.boot
                || pin.member.identity.executable != pin.worker
                || pin.member.resource_coalition <= 1
            {
                return Err(NativeEvidenceUnknown::InvalidProvenance);
            }
            match pin.scope.purpose {
                BoundaryPurpose::Attempt => {
                    attempt_records += 1;
                    if pin.capture_phase != BoundaryPhase::Held
                        || pin.record_schema == 2 && pin.release_attempted_at_capture
                        || !matches!(pin.record_schema, 1 | 2)
                    {
                        return Err(NativeEvidenceUnknown::InvalidProvenance);
                    }
                }
                BoundaryPurpose::VersionProbe | BoundaryPurpose::Resolver => {
                    if pin.capture_phase != BoundaryPhase::Retired {
                        return Err(NativeEvidenceUnknown::InvalidProvenance);
                    }
                }
                BoundaryPurpose::Collector | BoundaryPurpose::AccountTerminal => {
                    return Err(NativeEvidenceUnknown::InvalidProvenance);
                }
            }
        }
        if attempt_records != 1 {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        Ok(())
    }

    pub(crate) fn verify_native_job(
        binding: &NativeJobBinding,
    ) -> Result<NativeCompletionEvidence, NativeEvidenceUnknown> {
        validate_binding_manifest(binding)?;
        let current_host = current_kernel_witness()?;
        if current_host != binding.host {
            return Err(NativeEvidenceUnknown::KernelNotAdmitted);
        }
        if physical_directory(&binding.store_root)?.1 != binding.store_identity
            || private_directory(&binding.store_root)? != binding.store_identity
            || private_database(&binding.store_root)? != binding.database_identity
            || physical_directory(&binding.account_root)?.1 != binding.account_identity
            || private_directory(&binding.account_root)? != binding.account_identity
        {
            return Err(NativeEvidenceUnknown::SourceUnavailable);
        }
        let snapshot = read_snapshot(&binding.store_root, &binding.task_id)?;
        let operation = snapshot
            .operations
            .iter()
            .find(|operation| operation.operation_id == binding.operation_id)
            .cloned()
            .ok_or(NativeEvidenceUnknown::InvalidProvenance)?;
        let no_start = operation.native_writer_no_start_observed;
        let outcome = validate_completed_source(binding, &snapshot)?;
        let (base, parents, indexed, current_records) = validate_inventory(
            &binding.store_root,
            &binding.operation_id,
            &operation,
            &snapshot.task,
            &binding.host,
            Some(&binding.captured_records),
            false,
            no_start,
        )?;
        if base != binding.boundary_base_identity
            || parents != binding.parent_index_identity
            || indexed != binding.indexed_parent_identity
        {
            return Err(NativeEvidenceUnknown::SourceUnavailable);
        }
        let current_ids = current_records
            .iter()
            .map(|record| record.operation_id.as_str())
            .collect::<BTreeSet<_>>();
        if binding
            .captured_records
            .iter()
            .any(|record| !current_ids.contains(record.operation_id.as_str()))
        {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        let confirmation = read_snapshot(&binding.store_root, &binding.task_id)?;
        if snapshot_digest(&snapshot)? != snapshot_digest(&confirmation)? {
            return Err(NativeEvidenceUnknown::SourceUnavailable);
        }
        let (_, _, _, confirmed_records) = validate_inventory(
            &binding.store_root,
            &binding.operation_id,
            &operation,
            &confirmation.task,
            &binding.host,
            Some(&binding.captured_records),
            false,
            no_start,
        )?;
        if confirmed_records != current_records {
            return Err(NativeEvidenceUnknown::SourceUnavailable);
        }
        let retired_coalitions = current_records
            .iter()
            .map(|record| record.member.resource_coalition)
            .collect::<Vec<_>>();
        if retired_coalitions.is_empty() {
            return Err(NativeEvidenceUnknown::InvalidProvenance);
        }
        Ok(NativeCompletionEvidence {
            schema: 1,
            operation_id: binding.operation_id.clone(),
            task_id: binding.task_id.clone(),
            attempt_id: binding.attempt_id.clone(),
            generation: binding.generation,
            account_id: binding.account_id.clone(),
            auth_revision: binding.auth_revision,
            boot: binding.host.boot.clone(),
            outcome,
            retired_coalitions,
        })
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub(crate) use admitted::{capture_prelaunch, verify_native_job};
