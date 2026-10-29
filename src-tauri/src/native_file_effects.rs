use lomi_native_files::{NativeFileError, NativeFileErrorKind, NativeSaveResult};
use lomi_native_project_files::{
    NativeArtifactCreated, NativeProjectErrorCode, NativeProjectMutation,
    NativeProjectMutationError, NativeProjectMutationResult,
};
use lomi_remote_protocol::{
    ipc::{
        LocalCall, LocalNativeEditorEffect, LocalNativeEditorExecute, LocalNativeEditorResult,
        LocalNativeEffectState, LocalNativeFileBlob, LocalNativeFileStage, LocalNativeResultJson,
        LocalResult, NativeIntentId, NativeIntentTarget,
    },
    DecimalU64,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};
use tauri::{AppHandle, Manager};

use crate::{files::editor::EditorError, shared_host};
use lomi_control_protocol::ErrorCode;

const JOURNAL_LIMIT: u64 = 1024 * 1024;
const JOURNAL_ENTRIES: usize = 512;
const STATUS_POLLS: usize = 4;
const STATUS_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ExpectedResult {
    File,
    New,
    ProjectMutation,
    ArtifactCreated,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PendingEditorEffect {
    operation_id: NativeIntentId,
    request_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery_fingerprint: Option<String>,
    expected_result: ExpectedResult,
    execute_sent: bool,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EditorEffectJournal {
    version: u32,
    operations: BTreeMap<String, PendingEditorEffect>,
    #[serde(default)]
    acknowledgements: BTreeSet<NativeIntentId>,
}

enum EffectOutcome {
    Succeeded(NativeSaveResult),
    Failed(NativeFileError),
    Unknown,
}

enum ProjectEffectOutcome {
    Succeeded(NativeProjectMutationResult),
    Failed(NativeProjectErrorCode),
    Unknown,
}

enum ArtifactEffectOutcome {
    Succeeded(NativeArtifactCreated),
    Failed(NativeProjectErrorCode),
    Unknown,
}

pub(crate) enum GuardedSaveError {
    Authorization(ErrorCode),
    Before(ErrorCode),
    Uncertain,
}

impl From<EditorError> for GuardedSaveError {
    fn from(_: EditorError) -> Self {
        Self::Before(ErrorCode::StorageUnavailable)
    }
}

pub(crate) fn save(
    app: &AppHandle,
    destination: &Path,
    content: &str,
    effect: LocalNativeEditorEffect,
    expected_result: ExpectedResult,
) -> Result<NativeSaveResult, EditorError> {
    let destination_key = destination_key(destination)?;
    let payload = content.as_bytes();
    let payload_sha256 = revision(payload);
    let request_fingerprint =
        request_fingerprint(&effect, &payload_sha256, payload.len() as u64, None)?;
    let recovery_fingerprint =
        save_new_recovery_fingerprint(&effect, &payload_sha256, payload.len() as u64)?;
    let journal_path = journal_path(app)?;
    let mut journal = read_journal(&journal_path)?;
    flush_acknowledgements(app, &journal_path, &mut journal)?;

    if let Some(pending) = journal.operations.get(&destination_key) {
        let pending = PendingEditorEffect {
            operation_id: pending.operation_id,
            request_fingerprint: pending.request_fingerprint.clone(),
            recovery_fingerprint: pending.recovery_fingerprint.clone(),
            expected_result: pending.expected_result,
            execute_sent: pending.execute_sent,
        };
        if pending.execute_sent {
            match resolve_status(app, pending.operation_id, pending.expected_result) {
                EffectOutcome::Succeeded(result) => {
                    clear_operation(app, &journal_path, &mut journal, &destination_key)?;
                    if pending.request_fingerprint != request_fingerprint {
                        return Err(EditorError::new(
                            "conflict",
                            "A previous save completed with different editor contents. Reload the file before saving again.",
                        ));
                    }
                    return Ok(result);
                }
                EffectOutcome::Failed(error) => {
                    clear_operation(app, &journal_path, &mut journal, &destination_key)?;
                    if pending.request_fingerprint == request_fingerprint {
                        return Err(editor_error(error));
                    }
                }
                EffectOutcome::Unknown => return Err(uncertain_error()),
            }
        } else if pending.request_fingerprint == request_fingerprint {
            return stage_and_execute(
                app,
                &journal_path,
                &mut journal,
                destination_key,
                pending,
                payload,
                &payload_sha256,
                effect,
                expected_result,
            );
        } else {
            // No execute request was sent, so discarding this staged-only ID
            // cannot hide a file effect. The host owns cleanup of old blobs.
            clear_operation(app, &journal_path, &mut journal, &destination_key)?;
        }
    }

    if journal.operations.len() + journal.acknowledgements.len() >= JOURNAL_ENTRIES {
        return Err(EditorError::new(
            "io",
            "Too many editor saves are awaiting confirmation. Resolve a pending save before continuing.",
        ));
    }
    let pending = PendingEditorEffect {
        operation_id: new_operation_id()?,
        request_fingerprint,
        recovery_fingerprint,
        expected_result,
        execute_sent: false,
    };
    journal.operations.insert(
        destination_key.clone(),
        PendingEditorEffect {
            operation_id: pending.operation_id,
            request_fingerprint: pending.request_fingerprint.clone(),
            recovery_fingerprint: pending.recovery_fingerprint.clone(),
            expected_result: pending.expected_result,
            execute_sent: pending.execute_sent,
        },
    );
    write_journal(&journal_path, &journal)?;

    stage_and_execute(
        app,
        &journal_path,
        &mut journal,
        destination_key,
        pending,
        payload,
        &payload_sha256,
        effect,
        expected_result,
    )
}

pub(crate) fn save_guarded(
    app: &AppHandle,
    destination: &Path,
    content: &str,
    effect: LocalNativeEditorEffect,
    expected_result: ExpectedResult,
    principal: &str,
    mcp_operation: &str,
    check: &mut dyn FnMut() -> Result<(), ErrorCode>,
) -> Result<NativeSaveResult, GuardedSaveError> {
    check().map_err(GuardedSaveError::Authorization)?;
    let destination_key = destination_key(destination)?;
    let payload = content.as_bytes();
    if payload.len() > 4 * 1024 * 1024 {
        return Err(EditorError::new(
            "tooLarge",
            "The editor content exceeds the 4 MiB MCP save limit.",
        )
        .into());
    }
    let payload_sha256 = revision(payload);
    let request_fingerprint = request_fingerprint(
        &effect,
        &payload_sha256,
        payload.len() as u64,
        Some((principal, mcp_operation)),
    )?;
    let journal_path = journal_path(app)?;
    let mut journal = read_journal(&journal_path)?;
    flush_acknowledgements(app, &journal_path, &mut journal)?;

    if let Some(pending) = journal.operations.get(&destination_key) {
        let pending = PendingEditorEffect {
            operation_id: pending.operation_id,
            request_fingerprint: pending.request_fingerprint.clone(),
            recovery_fingerprint: pending.recovery_fingerprint.clone(),
            expected_result: pending.expected_result,
            execute_sent: pending.execute_sent,
        };
        if pending.execute_sent {
            match resolve_status(app, pending.operation_id, pending.expected_result) {
                EffectOutcome::Succeeded(result) => {
                    let _ = clear_operation(app, &journal_path, &mut journal, &destination_key);
                    if pending.request_fingerprint == request_fingerprint {
                        return Ok(result);
                    }
                    return Err(GuardedSaveError::Before(ErrorCode::RevisionConflict));
                }
                EffectOutcome::Failed(error) => {
                    let _ = clear_operation(app, &journal_path, &mut journal, &destination_key);
                    if pending.request_fingerprint == request_fingerprint {
                        return Err(guarded_native_error(error));
                    }
                }
                EffectOutcome::Unknown => return Err(GuardedSaveError::Uncertain),
            }
        } else if pending.request_fingerprint == request_fingerprint {
            return stage_and_execute_guarded(
                app,
                &journal_path,
                &mut journal,
                destination_key,
                pending,
                payload,
                &payload_sha256,
                effect,
                expected_result,
                check,
            );
        } else {
            clear_operation(app, &journal_path, &mut journal, &destination_key)?;
        }
    }

    if journal.operations.len() + journal.acknowledgements.len() >= JOURNAL_ENTRIES {
        return Err(EditorError::new(
            "io",
            "Too many editor saves are awaiting confirmation. Resolve a pending save before continuing.",
        )
        .into());
    }
    let pending = PendingEditorEffect {
        operation_id: new_operation_id()?,
        request_fingerprint,
        recovery_fingerprint: None,
        expected_result,
        execute_sent: false,
    };
    journal.operations.insert(
        destination_key.clone(),
        PendingEditorEffect {
            operation_id: pending.operation_id,
            request_fingerprint: pending.request_fingerprint.clone(),
            recovery_fingerprint: None,
            expected_result: pending.expected_result,
            execute_sent: pending.execute_sent,
        },
    );
    write_journal(&journal_path, &journal)?;

    stage_and_execute_guarded(
        app,
        &journal_path,
        &mut journal,
        destination_key,
        pending,
        payload,
        &payload_sha256,
        effect,
        expected_result,
        check,
    )
}

pub(crate) fn recover_pending(
    app: &AppHandle,
    destination: &Path,
    content: &str,
    effect: &LocalNativeEditorEffect,
    expected_result: ExpectedResult,
) -> Result<Option<NativeSaveResult>, EditorError> {
    let effect_result = match effect {
        LocalNativeEditorEffect::Save { .. } => ExpectedResult::File,
        LocalNativeEditorEffect::SaveNew { .. }
        | LocalNativeEditorEffect::SaveNewChecked { .. } => ExpectedResult::New,
        LocalNativeEditorEffect::ProjectFiles { .. } => ExpectedResult::ProjectMutation,
        LocalNativeEditorEffect::ArtifactCreate { .. } => ExpectedResult::ArtifactCreated,
        LocalNativeEditorEffect::DesktopFiles { .. } => {
            return Err(EditorError::new(
                "io",
                "Desktop file effects are recovered through their operation receipt.",
            ));
        }
    };
    if effect_result != expected_result {
        return Err(EditorError::new(
            "io",
            "The editor effect does not match its expected result type.",
        ));
    }
    if matches!(
        expected_result,
        ExpectedResult::ProjectMutation | ExpectedResult::ArtifactCreated
    ) {
        return Err(EditorError::new(
            "io",
            "Guarded project effects are recovered through their MCP operation receipt.",
        ));
    }
    let key = destination_key(destination)?;
    let content_sha256 = revision(content.as_bytes());
    let fingerprint = request_fingerprint(effect, &content_sha256, content.len() as u64, None)?;
    let path = journal_path(app)?;
    let mut journal = read_journal(&path)?;
    flush_acknowledgements(app, &path, &mut journal)?;
    let Some(pending) = journal.operations.get(&key) else {
        return Ok(None);
    };
    if !pending.execute_sent {
        return Ok(None);
    }
    let pending = PendingEditorEffect {
        operation_id: pending.operation_id,
        request_fingerprint: pending.request_fingerprint.clone(),
        recovery_fingerprint: pending.recovery_fingerprint.clone(),
        expected_result: pending.expected_result,
        execute_sent: true,
    };
    match resolve_status(app, pending.operation_id, pending.expected_result) {
        EffectOutcome::Succeeded(result) => {
            clear_operation(app, &path, &mut journal, &key)?;
            if pending.request_fingerprint != fingerprint {
                return Err(EditorError::new(
                    "conflict",
                    "A previous save completed with different editor contents. Reload the file before saving again.",
                ));
            }
            Ok(Some(result))
        }
        EffectOutcome::Failed(error) => {
            clear_operation(app, &path, &mut journal, &key)?;
            if pending.request_fingerprint == fingerprint {
                Err(editor_error(error))
            } else {
                Ok(None)
            }
        }
        EffectOutcome::Unknown => Err(uncertain_error()),
    }
}

/// Checks old Host-owned effects without querying the Host or changing their
/// recovery journal. Effects with path-scoped keys are matched to the local
/// write; MCP project effects do not retain their paths in this journal, so
/// they conservatively fence local writes until explicitly recovered.
pub(crate) fn local_scope_has_pending_host_effect(
    app: &AppHandle,
    paths: &[PathBuf],
    recursive: bool,
) -> Result<bool, String> {
    let path = journal_path(app).map_err(|error| error.to_string())?;
    let journal = read_journal(&path).map_err(|error| error.to_string())?;
    let path_keys = paths
        .iter()
        .map(|path| destination_key(path).map_err(|error| error.to_string()))
        .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(journal_conflicts_local_scope(
        &journal, &path_keys, recursive,
    ))
}

fn journal_conflicts_local_scope(
    journal: &EditorEffectJournal,
    path_keys: &BTreeSet<String>,
    recursive: bool,
) -> bool {
    for (key, pending) in &journal.operations {
        if !pending.execute_sent {
            continue;
        }
        match pending.expected_result {
            ExpectedResult::File | ExpectedResult::New => {
                if path_keys.contains(key) || recursive {
                    return true;
                }
            }
            ExpectedResult::ProjectMutation | ExpectedResult::ArtifactCreated => {
                return true;
            }
        }
    }
    false
}

pub(crate) fn any_pending_host_effect(app: &AppHandle) -> Result<bool, String> {
    let path = journal_path(app).map_err(|error| error.to_string())?;
    let journal = read_journal(&path).map_err(|error| error.to_string())?;
    Ok(journal
        .operations
        .values()
        .any(|pending| pending.execute_sent))
}

pub(crate) fn local_host_effect_conflicts(
    app: &AppHandle,
    paths: &[PathBuf],
    recursive: bool,
) -> Result<bool, String> {
    if crate::desktop_runtime::pending_host_git_effect(app)? {
        return Ok(true);
    }
    if local_scope_has_pending_host_effect(app, paths, recursive)? {
        return Ok(true);
    }
    crate::desktop_file_effects::local_scope_has_pending_host_effect(app, paths)
}

pub(crate) fn any_local_host_effect_pending(app: &AppHandle) -> Result<bool, String> {
    if crate::desktop_runtime::pending_host_git_effect(app)? {
        return Ok(true);
    }
    if any_pending_host_effect(app)? {
        return Ok(true);
    }
    crate::desktop_file_effects::any_pending_host_effect(app)
}

/// Publish a normalized, no-exec project mutation through the canonical Host.
/// Its journal key is scoped to the MCP principal and operation so an
/// unresolved operation does not lock all future mutations of the same root.
pub(crate) fn project_mutation_guarded(
    app: &AppHandle,
    root: &Path,
    principal: &str,
    mcp_operation: &str,
    mutation: &NativeProjectMutation,
    check: &mut dyn FnMut() -> Result<(), ErrorCode>,
) -> Result<NativeProjectMutationResult, NativeProjectMutationError> {
    const PROJECT_MUTATION_LIMIT: usize = 1024 * 1024;

    check().map_err(|code| NativeProjectMutationError::Before(native_project_error_code(code)))?;
    let root = root.to_str().ok_or_else(|| {
        NativeProjectMutationError::Before(NativeProjectErrorCode::UnsupportedCapability)
    })?;
    let payload = serde_json::to_vec(mutation).map_err(|_| {
        NativeProjectMutationError::Before(NativeProjectErrorCode::StorageUnavailable)
    })?;
    if payload.len() > PROJECT_MUTATION_LIMIT {
        return Err(NativeProjectMutationError::Before(
            NativeProjectErrorCode::ResourceExhausted,
        ));
    }
    let payload_sha256 = revision(&payload);
    let request_fingerprint = project_request_fingerprint(
        root,
        principal,
        mcp_operation,
        &payload_sha256,
        payload.len() as u64,
    )
    .map_err(|_| NativeProjectMutationError::Before(NativeProjectErrorCode::StorageUnavailable))?;
    let operation_key = project_operation_key(principal, mcp_operation);
    let journal_path = journal_path(app).map_err(|_| {
        NativeProjectMutationError::Before(NativeProjectErrorCode::StorageUnavailable)
    })?;
    let mut journal = read_journal(&journal_path).map_err(|_| {
        NativeProjectMutationError::Before(NativeProjectErrorCode::StorageUnavailable)
    })?;
    flush_acknowledgements(app, &journal_path, &mut journal).map_err(|_| {
        NativeProjectMutationError::Before(NativeProjectErrorCode::StorageUnavailable)
    })?;

    if let Some(pending) = journal.operations.get(&operation_key) {
        let pending = PendingEditorEffect {
            operation_id: pending.operation_id,
            request_fingerprint: pending.request_fingerprint.clone(),
            recovery_fingerprint: pending.recovery_fingerprint.clone(),
            expected_result: pending.expected_result,
            execute_sent: pending.execute_sent,
        };
        if pending.expected_result != ExpectedResult::ProjectMutation {
            return Err(NativeProjectMutationError::Before(
                NativeProjectErrorCode::IdempotencyConflict,
            ));
        }
        if pending.execute_sent {
            match resolve_project_status(app, pending.operation_id) {
                ProjectEffectOutcome::Succeeded(result) => {
                    let _ = clear_operation(app, &journal_path, &mut journal, &operation_key);
                    return if pending.request_fingerprint == request_fingerprint {
                        Ok(result)
                    } else {
                        Err(NativeProjectMutationError::Uncertain)
                    };
                }
                ProjectEffectOutcome::Failed(code) => {
                    let _ = clear_operation(app, &journal_path, &mut journal, &operation_key);
                    return Err(if pending.request_fingerprint == request_fingerprint {
                        NativeProjectMutationError::Before(code)
                    } else {
                        NativeProjectMutationError::Before(
                            NativeProjectErrorCode::IdempotencyConflict,
                        )
                    });
                }
                ProjectEffectOutcome::Unknown => return Err(NativeProjectMutationError::Uncertain),
            }
        } else if pending.request_fingerprint == request_fingerprint {
            return stage_and_execute_project_guarded(
                app,
                &journal_path,
                &mut journal,
                operation_key,
                pending,
                &payload,
                &payload_sha256,
                root,
                check,
            );
        } else {
            // The old staged blob has not been sent to Execute, so replacing
            // its Host intent cannot repeat or hide a project filesystem effect.
            clear_operation(app, &journal_path, &mut journal, &operation_key).map_err(|_| {
                NativeProjectMutationError::Before(NativeProjectErrorCode::StorageUnavailable)
            })?;
        }
    }

    if journal.operations.len() + journal.acknowledgements.len() >= JOURNAL_ENTRIES {
        return Err(NativeProjectMutationError::Before(
            NativeProjectErrorCode::ResourceExhausted,
        ));
    }
    let pending = PendingEditorEffect {
        operation_id: new_operation_id().map_err(|_| {
            NativeProjectMutationError::Before(NativeProjectErrorCode::StorageUnavailable)
        })?,
        request_fingerprint,
        recovery_fingerprint: None,
        expected_result: ExpectedResult::ProjectMutation,
        execute_sent: false,
    };
    journal.operations.insert(
        operation_key.clone(),
        PendingEditorEffect {
            operation_id: pending.operation_id,
            request_fingerprint: pending.request_fingerprint.clone(),
            recovery_fingerprint: None,
            expected_result: pending.expected_result,
            execute_sent: false,
        },
    );
    write_journal(&journal_path, &journal).map_err(|_| {
        NativeProjectMutationError::Before(NativeProjectErrorCode::StorageUnavailable)
    })?;

    stage_and_execute_project_guarded(
        app,
        &journal_path,
        &mut journal,
        operation_key,
        pending,
        &payload,
        &payload_sha256,
        root,
        check,
    )
}

/// Publish one bounded artifact through the canonical Host and recover an
/// ambiguous reply from its durable status receipt without resending Execute.
pub(crate) fn artifact_create_guarded(
    app: &AppHandle,
    root: &Path,
    principal: &str,
    mcp_operation: &str,
    relative_path: &str,
    expected_parent_revision: &str,
    payload: &[u8],
    check: &mut dyn FnMut() -> Result<(), ErrorCode>,
) -> Result<NativeArtifactCreated, NativeProjectMutationError> {
    check().map_err(|code| NativeProjectMutationError::Before(native_project_error_code(code)))?;
    let root = root.to_str().ok_or_else(|| {
        NativeProjectMutationError::Before(NativeProjectErrorCode::UnsupportedCapability)
    })?;
    let payload_sha256 = revision(payload);
    let request_fingerprint = artifact_create_request_fingerprint(
        root,
        principal,
        mcp_operation,
        relative_path,
        expected_parent_revision,
        &payload_sha256,
        payload.len() as u64,
    )
    .map_err(|_| NativeProjectMutationError::Before(NativeProjectErrorCode::StorageUnavailable))?;
    let operation_key = artifact_create_operation_key(principal, mcp_operation);
    let journal_path = journal_path(app).map_err(|_| {
        NativeProjectMutationError::Before(NativeProjectErrorCode::StorageUnavailable)
    })?;
    let mut journal = read_journal(&journal_path).map_err(|_| {
        NativeProjectMutationError::Before(NativeProjectErrorCode::StorageUnavailable)
    })?;
    flush_acknowledgements(app, &journal_path, &mut journal).map_err(|_| {
        NativeProjectMutationError::Before(NativeProjectErrorCode::StorageUnavailable)
    })?;

    if let Some(pending) = journal.operations.get(&operation_key) {
        let pending = PendingEditorEffect {
            operation_id: pending.operation_id,
            request_fingerprint: pending.request_fingerprint.clone(),
            recovery_fingerprint: pending.recovery_fingerprint.clone(),
            expected_result: pending.expected_result,
            execute_sent: pending.execute_sent,
        };
        if pending.expected_result != ExpectedResult::ArtifactCreated {
            return Err(NativeProjectMutationError::Before(
                NativeProjectErrorCode::IdempotencyConflict,
            ));
        }
        if pending.execute_sent {
            match resolve_artifact_status(app, pending.operation_id) {
                ArtifactEffectOutcome::Succeeded(result) => {
                    if check().is_err() {
                        return Err(NativeProjectMutationError::Uncertain);
                    }
                    let _ = clear_operation(app, &journal_path, &mut journal, &operation_key);
                    return if pending.request_fingerprint == request_fingerprint {
                        Ok(result)
                    } else {
                        Err(NativeProjectMutationError::Uncertain)
                    };
                }
                ArtifactEffectOutcome::Failed(code) => {
                    let _ = clear_operation(app, &journal_path, &mut journal, &operation_key);
                    return Err(if pending.request_fingerprint == request_fingerprint {
                        NativeProjectMutationError::Before(code)
                    } else {
                        NativeProjectMutationError::Before(
                            NativeProjectErrorCode::IdempotencyConflict,
                        )
                    });
                }
                ArtifactEffectOutcome::Unknown => {
                    return Err(NativeProjectMutationError::Uncertain)
                }
            }
        } else if pending.request_fingerprint == request_fingerprint {
            return stage_and_execute_artifact_guarded(
                app,
                &journal_path,
                &mut journal,
                operation_key,
                pending,
                payload,
                &payload_sha256,
                root,
                relative_path,
                expected_parent_revision,
                check,
            );
        } else {
            // An unstaged Host intent has not reached Execute and can be
            // replaced by a new request for the same MCP operation.
            clear_operation(app, &journal_path, &mut journal, &operation_key).map_err(|_| {
                NativeProjectMutationError::Before(NativeProjectErrorCode::StorageUnavailable)
            })?;
        }
    }

    if journal.operations.len() + journal.acknowledgements.len() >= JOURNAL_ENTRIES {
        return Err(NativeProjectMutationError::Before(
            NativeProjectErrorCode::ResourceExhausted,
        ));
    }
    let pending = PendingEditorEffect {
        operation_id: new_operation_id().map_err(|_| {
            NativeProjectMutationError::Before(NativeProjectErrorCode::StorageUnavailable)
        })?,
        request_fingerprint,
        recovery_fingerprint: None,
        expected_result: ExpectedResult::ArtifactCreated,
        execute_sent: false,
    };
    journal.operations.insert(
        operation_key.clone(),
        PendingEditorEffect {
            operation_id: pending.operation_id,
            request_fingerprint: pending.request_fingerprint.clone(),
            recovery_fingerprint: None,
            expected_result: pending.expected_result,
            execute_sent: false,
        },
    );
    write_journal(&journal_path, &journal).map_err(|_| {
        NativeProjectMutationError::Before(NativeProjectErrorCode::StorageUnavailable)
    })?;

    stage_and_execute_artifact_guarded(
        app,
        &journal_path,
        &mut journal,
        operation_key,
        pending,
        payload,
        &payload_sha256,
        root,
        relative_path,
        expected_parent_revision,
        check,
    )
}

fn stage_and_execute_artifact_guarded(
    app: &AppHandle,
    journal_path: &Path,
    journal: &mut EditorEffectJournal,
    operation_key: String,
    mut pending: PendingEditorEffect,
    payload: &[u8],
    payload_sha256: &str,
    root: &str,
    relative_path: &str,
    expected_parent_revision: &str,
    check: &mut dyn FnMut() -> Result<(), ErrorCode>,
) -> Result<NativeArtifactCreated, NativeProjectMutationError> {
    let root =
        lomi_remote_protocol::ipc::LocalRootPath::try_from(root.to_owned()).map_err(|_| {
            NativeProjectMutationError::Before(NativeProjectErrorCode::UnsupportedCapability)
        })?;
    let relative =
        lomi_remote_protocol::ipc::LocalNativeRelativePath::try_from(relative_path.to_owned())
            .map_err(|_| {
                NativeProjectMutationError::Before(NativeProjectErrorCode::UnsupportedCapability)
            })?;
    let expected_parent_revision =
        lomi_remote_protocol::HostKeyCommitmentHex::try_from(expected_parent_revision.to_owned())
            .map_err(|_| {
            NativeProjectMutationError::Before(NativeProjectErrorCode::RevisionConflict)
        })?;
    let blob = LocalNativeFileBlob {
        operation_id: pending.operation_id,
        byte_length: DecimalU64(payload.len() as u64),
        content_sha256: lomi_remote_protocol::HostKeyCommitmentHex::try_from(
            payload_sha256.to_owned(),
        )
        .map_err(|_| {
            NativeProjectMutationError::Before(NativeProjectErrorCode::StorageUnavailable)
        })?,
    };
    let staged = match request(app, LocalCall::NativeFileStage(blob.clone())) {
        Ok(LocalResult::NativeFileStage(staged)) if staged.operation_id == pending.operation_id => {
            staged
        }
        _ => {
            return Err(NativeProjectMutationError::Before(
                NativeProjectErrorCode::StorageUnavailable,
            ));
        }
    };
    if stage_payload(pending.operation_id, &staged, payload, payload_sha256).is_err() {
        return Err(NativeProjectMutationError::Before(
            NativeProjectErrorCode::StorageUnavailable,
        ));
    }

    pending.execute_sent = true;
    journal.operations.insert(
        operation_key.clone(),
        PendingEditorEffect {
            operation_id: pending.operation_id,
            request_fingerprint: pending.request_fingerprint.clone(),
            recovery_fingerprint: None,
            expected_result: ExpectedResult::ArtifactCreated,
            execute_sent: true,
        },
    );
    write_journal(journal_path, journal).map_err(|_| {
        NativeProjectMutationError::Before(NativeProjectErrorCode::StorageUnavailable)
    })?;

    let mut authorization_error = None;
    let executed = app
        .state::<shared_host::SharedHost>()
        .native_file_request_guarded(
            LocalCall::NativeEditorExecuteGuarded(LocalNativeEditorExecute {
                blob,
                effect: LocalNativeEditorEffect::ArtifactCreate {
                    root,
                    relative,
                    expected_parent_revision,
                },
            }),
            &mut || match check() {
                Ok(()) => true,
                Err(error) => {
                    authorization_error.get_or_insert(error);
                    false
                }
            },
        );
    let outcome = match executed {
        Ok(LocalResult::NativeEditor(result)) => {
            resolve_artifact_effect(app, pending.operation_id, Some(result))
        }
        _ => resolve_artifact_status(app, pending.operation_id),
    };
    match outcome {
        ArtifactEffectOutcome::Succeeded(result) => {
            if authorization_error.is_some() || check().is_err() {
                return Err(NativeProjectMutationError::Uncertain);
            }
            let _ = clear_operation(app, journal_path, journal, &operation_key);
            Ok(result)
        }
        ArtifactEffectOutcome::Failed(code) => {
            let _ = clear_operation(app, journal_path, journal, &operation_key);
            Err(NativeProjectMutationError::Before(
                authorization_error
                    .map(native_project_error_code)
                    .unwrap_or(code),
            ))
        }
        ArtifactEffectOutcome::Unknown => Err(NativeProjectMutationError::Uncertain),
    }
}

fn resolve_artifact_status(app: &AppHandle, operation_id: NativeIntentId) -> ArtifactEffectOutcome {
    resolve_artifact_effect(app, operation_id, None)
}

fn resolve_artifact_effect(
    app: &AppHandle,
    operation_id: NativeIntentId,
    mut latest: Option<LocalNativeEditorResult>,
) -> ArtifactEffectOutcome {
    for attempt in 0..=STATUS_POLLS {
        let Some(result) = latest.take() else {
            if attempt > 0 {
                thread::sleep(STATUS_INTERVAL);
            }
            latest = status_request(app, operation_id);
            continue;
        };
        if result.operation_id != operation_id {
            return ArtifactEffectOutcome::Unknown;
        }
        match result.state {
            LocalNativeEffectState::Succeeded => {
                let Some(json) = result.result_json.as_ref() else {
                    return ArtifactEffectOutcome::Unknown;
                };
                return serde_json::from_str::<NativeArtifactCreated>(json.as_str())
                    .map(ArtifactEffectOutcome::Succeeded)
                    .unwrap_or(ArtifactEffectOutcome::Unknown);
            }
            LocalNativeEffectState::Failed => {
                let Some(json) = result.result_json.as_ref() else {
                    return ArtifactEffectOutcome::Unknown;
                };
                if let Ok(code) = serde_json::from_str::<NativeProjectErrorCode>(json.as_str()) {
                    return if code == NativeProjectErrorCode::OutcomeUnknown {
                        ArtifactEffectOutcome::Unknown
                    } else {
                        ArtifactEffectOutcome::Failed(code)
                    };
                }
                let Ok(error) = serde_json::from_str::<NativeFileError>(json.as_str()) else {
                    return ArtifactEffectOutcome::Unknown;
                };
                return match error.kind {
                    NativeFileErrorKind::Io => {
                        ArtifactEffectOutcome::Failed(NativeProjectErrorCode::StorageUnavailable)
                    }
                    NativeFileErrorKind::Unsupported | NativeFileErrorKind::ReadOnly => {
                        ArtifactEffectOutcome::Failed(NativeProjectErrorCode::UnsupportedCapability)
                    }
                    NativeFileErrorKind::TooLarge => {
                        ArtifactEffectOutcome::Failed(NativeProjectErrorCode::ResourceExhausted)
                    }
                    NativeFileErrorKind::Conflict => {
                        ArtifactEffectOutcome::Failed(NativeProjectErrorCode::RevisionConflict)
                    }
                    NativeFileErrorKind::OpenFile => {
                        ArtifactEffectOutcome::Failed(NativeProjectErrorCode::TargetBusy)
                    }
                    NativeFileErrorKind::Uncertain => ArtifactEffectOutcome::Unknown,
                };
            }
            LocalNativeEffectState::OutcomeUnknown | LocalNativeEffectState::Running
                if attempt < STATUS_POLLS =>
            {
                thread::sleep(STATUS_INTERVAL);
                latest = status_request(app, operation_id);
            }
            LocalNativeEffectState::OutcomeUnknown | LocalNativeEffectState::Running => {
                return ArtifactEffectOutcome::Unknown;
            }
        }
    }
    ArtifactEffectOutcome::Unknown
}

fn stage_and_execute_project_guarded(
    app: &AppHandle,
    journal_path: &Path,
    journal: &mut EditorEffectJournal,
    operation_key: String,
    mut pending: PendingEditorEffect,
    payload: &[u8],
    payload_sha256: &str,
    root: &str,
    check: &mut dyn FnMut() -> Result<(), ErrorCode>,
) -> Result<NativeProjectMutationResult, NativeProjectMutationError> {
    let root =
        lomi_remote_protocol::ipc::LocalRootPath::try_from(root.to_owned()).map_err(|_| {
            NativeProjectMutationError::Before(NativeProjectErrorCode::UnsupportedCapability)
        })?;
    let blob = LocalNativeFileBlob {
        operation_id: pending.operation_id,
        byte_length: DecimalU64(payload.len() as u64),
        content_sha256: lomi_remote_protocol::HostKeyCommitmentHex::try_from(
            payload_sha256.to_owned(),
        )
        .map_err(|_| {
            NativeProjectMutationError::Before(NativeProjectErrorCode::StorageUnavailable)
        })?,
    };
    let staged = match request(app, LocalCall::NativeFileStage(blob.clone())) {
        Ok(LocalResult::NativeFileStage(staged)) if staged.operation_id == pending.operation_id => {
            staged
        }
        _ => {
            return Err(NativeProjectMutationError::Before(
                NativeProjectErrorCode::StorageUnavailable,
            ));
        }
    };
    if stage_payload(pending.operation_id, &staged, payload, payload_sha256).is_err() {
        return Err(NativeProjectMutationError::Before(
            NativeProjectErrorCode::StorageUnavailable,
        ));
    }

    pending.execute_sent = true;
    journal.operations.insert(
        operation_key.clone(),
        PendingEditorEffect {
            operation_id: pending.operation_id,
            request_fingerprint: pending.request_fingerprint.clone(),
            recovery_fingerprint: None,
            expected_result: ExpectedResult::ProjectMutation,
            execute_sent: true,
        },
    );
    write_journal(journal_path, journal).map_err(|_| {
        NativeProjectMutationError::Before(NativeProjectErrorCode::StorageUnavailable)
    })?;

    let mut authorization_error = None;
    let executed = app
        .state::<shared_host::SharedHost>()
        .native_file_request_guarded(
            LocalCall::NativeEditorExecuteGuarded(LocalNativeEditorExecute {
                blob,
                effect: LocalNativeEditorEffect::ProjectFiles { root },
            }),
            &mut || match check() {
                Ok(()) => true,
                Err(error) => {
                    authorization_error.get_or_insert(error);
                    false
                }
            },
        );
    let outcome = match executed {
        Ok(LocalResult::NativeEditor(result)) => {
            resolve_project_effect(app, pending.operation_id, Some(result))
        }
        // The Host may have committed before its response was lost. Query its
        // receipt, and never resend Execute once the journal says it was sent.
        _ => resolve_project_status(app, pending.operation_id),
    };
    match outcome {
        ProjectEffectOutcome::Succeeded(result) => {
            if authorization_error.is_some() {
                return Err(NativeProjectMutationError::Uncertain);
            }
            // The Host result is durable. Cleanup and the ACK outbox are
            // best-effort; retries still query status if persistence failed.
            let _ = clear_operation(app, journal_path, journal, &operation_key);
            Ok(result)
        }
        ProjectEffectOutcome::Failed(code) => {
            let _ = clear_operation(app, journal_path, journal, &operation_key);
            Err(NativeProjectMutationError::Before(
                authorization_error
                    .map(native_project_error_code)
                    .unwrap_or(code),
            ))
        }
        ProjectEffectOutcome::Unknown => Err(NativeProjectMutationError::Uncertain),
    }
}

pub(crate) fn recover_pending_new_checked(
    app: &AppHandle,
    destination: &Path,
    content: &str,
    legacy_effect: &LocalNativeEditorEffect,
    expected_result: ExpectedResult,
) -> Result<Option<NativeSaveResult>, EditorError> {
    if expected_result != ExpectedResult::New
        || !matches!(legacy_effect, LocalNativeEditorEffect::SaveNew { .. })
    {
        return Err(EditorError::new(
            "io",
            "The Save As recovery request is invalid.",
        ));
    }
    let content_sha256 = revision(content.as_bytes());
    let recovery_fingerprint =
        request_fingerprint(legacy_effect, &content_sha256, content.len() as u64, None)?;
    let key = destination_key(destination)?;
    let path = journal_path(app)?;
    let mut journal = read_journal(&path)?;
    flush_acknowledgements(app, &path, &mut journal)?;
    let Some(pending) = journal.operations.get(&key) else {
        return Ok(None);
    };
    let pending = PendingEditorEffect {
        operation_id: pending.operation_id,
        request_fingerprint: pending.request_fingerprint.clone(),
        recovery_fingerprint: pending.recovery_fingerprint.clone(),
        expected_result: pending.expected_result,
        execute_sent: pending.execute_sent,
    };
    if pending.expected_result != expected_result {
        return Err(EditorError::new(
            "conflict",
            "A different editor save is pending for this file. Resolve it before Save As.",
        ));
    }
    let matches_intent = pending
        .recovery_fingerprint
        .as_deref()
        .unwrap_or(&pending.request_fingerprint)
        == recovery_fingerprint.as_str();
    if !pending.execute_sent {
        if !matches_intent {
            clear_operation(app, &path, &mut journal, &key)?;
        }
        return Ok(None);
    }
    match resolve_status(app, pending.operation_id, pending.expected_result) {
        EffectOutcome::Succeeded(result) => {
            let _ = clear_operation(app, &path, &mut journal, &key);
            if matches_intent {
                Ok(Some(result))
            } else {
                Err(EditorError::new(
                    "conflict",
                    "A previous Save As completed with different editor contents. Reload the file before saving again.",
                ))
            }
        }
        EffectOutcome::Failed(error) => {
            let _ = clear_operation(app, &path, &mut journal, &key);
            if matches_intent {
                Err(editor_error(error))
            } else {
                Ok(None)
            }
        }
        EffectOutcome::Unknown => Err(uncertain_error()),
    }
}

fn stage_and_execute(
    app: &AppHandle,
    journal_path: &Path,
    journal: &mut EditorEffectJournal,
    destination_key: String,
    mut pending: PendingEditorEffect,
    payload: &[u8],
    payload_sha256: &str,
    effect: LocalNativeEditorEffect,
    expected_result: ExpectedResult,
) -> Result<NativeSaveResult, EditorError> {
    let blob = LocalNativeFileBlob {
        operation_id: pending.operation_id,
        byte_length: DecimalU64(payload.len() as u64),
        content_sha256: lomi_remote_protocol::HostKeyCommitmentHex::try_from(
            payload_sha256.to_owned(),
        )
        .map_err(|_| EditorError::new("io", "The editor content digest is invalid."))?,
    };
    let staged = match request(app, LocalCall::NativeFileStage(blob.clone())) {
        Ok(LocalResult::NativeFileStage(staged)) if staged.operation_id == pending.operation_id => {
            staged
        }
        _ => return Err(stage_error()),
    };
    if stage_payload(pending.operation_id, &staged, payload, payload_sha256).is_err() {
        return Err(stage_error());
    }

    pending.execute_sent = true;
    journal.operations.insert(
        destination_key.clone(),
        PendingEditorEffect {
            operation_id: pending.operation_id,
            request_fingerprint: pending.request_fingerprint.clone(),
            recovery_fingerprint: pending.recovery_fingerprint.clone(),
            expected_result: pending.expected_result,
            execute_sent: true,
        },
    );
    write_journal(journal_path, journal)?;

    let executed = request(
        app,
        LocalCall::NativeEditorExecute(lomi_remote_protocol::ipc::LocalNativeEditorExecute {
            blob,
            effect,
        }),
    );
    let outcome = match executed {
        Ok(LocalResult::NativeEditor(result)) => {
            resolve_effect(app, pending.operation_id, Some(result), expected_result)
        }
        // Execute may have committed before the IPC response was lost. Query
        // the durable receipt only; never resend Execute for this operation ID.
        _ => resolve_status(app, pending.operation_id, expected_result),
    };
    match outcome {
        EffectOutcome::Succeeded(result) => {
            clear_operation(app, journal_path, journal, &destination_key)?;
            Ok(result)
        }
        EffectOutcome::Failed(error) => {
            clear_operation(app, journal_path, journal, &destination_key)?;
            Err(editor_error(error))
        }
        EffectOutcome::Unknown => Err(uncertain_error()),
    }
}

fn stage_and_execute_guarded(
    app: &AppHandle,
    journal_path: &Path,
    journal: &mut EditorEffectJournal,
    destination_key: String,
    mut pending: PendingEditorEffect,
    payload: &[u8],
    payload_sha256: &str,
    effect: LocalNativeEditorEffect,
    expected_result: ExpectedResult,
    check: &mut dyn FnMut() -> Result<(), ErrorCode>,
) -> Result<NativeSaveResult, GuardedSaveError> {
    let blob = LocalNativeFileBlob {
        operation_id: pending.operation_id,
        byte_length: DecimalU64(payload.len() as u64),
        content_sha256: lomi_remote_protocol::HostKeyCommitmentHex::try_from(
            payload_sha256.to_owned(),
        )
        .map_err(|_| EditorError::new("io", "The editor content digest is invalid."))?,
    };
    let staged = match request(app, LocalCall::NativeFileStage(blob.clone())) {
        Ok(LocalResult::NativeFileStage(staged)) if staged.operation_id == pending.operation_id => {
            staged
        }
        _ => return Err(stage_error().into()),
    };
    if stage_payload(pending.operation_id, &staged, payload, payload_sha256).is_err() {
        return Err(stage_error().into());
    }

    pending.execute_sent = true;
    journal.operations.insert(
        destination_key.clone(),
        PendingEditorEffect {
            operation_id: pending.operation_id,
            request_fingerprint: pending.request_fingerprint.clone(),
            recovery_fingerprint: None,
            expected_result: pending.expected_result,
            execute_sent: true,
        },
    );
    write_journal(journal_path, journal)?;

    let mut authorization_error = None;
    let mut guarded_check = || match check() {
        Ok(()) => true,
        Err(error) => {
            authorization_error.get_or_insert(error);
            false
        }
    };
    let executed = app
        .state::<shared_host::SharedHost>()
        .native_file_request_guarded(
            LocalCall::NativeEditorExecuteGuarded(LocalNativeEditorExecute { blob, effect }),
            &mut guarded_check,
        );
    let outcome = match executed {
        Ok(LocalResult::NativeEditor(result)) => {
            resolve_effect(app, pending.operation_id, Some(result), expected_result)
        }
        // A lost guarded response is resolved only through the durable Host
        // receipt; never resend Execute for an operation whose intent was sent.
        _ => resolve_status(app, pending.operation_id, expected_result),
    };
    match outcome {
        EffectOutcome::Succeeded(result) => {
            let _ = clear_operation(app, journal_path, journal, &destination_key);
            Ok(result)
        }
        EffectOutcome::Failed(error) => {
            let _ = clear_operation(app, journal_path, journal, &destination_key);
            if let Some(error) = authorization_error {
                Err(GuardedSaveError::Authorization(error))
            } else {
                Err(guarded_native_error(error))
            }
        }
        EffectOutcome::Unknown => Err(GuardedSaveError::Uncertain),
    }
}

fn resolve_status(
    app: &AppHandle,
    operation_id: NativeIntentId,
    expected: ExpectedResult,
) -> EffectOutcome {
    resolve_effect(app, operation_id, None, expected)
}

fn resolve_project_status(app: &AppHandle, operation_id: NativeIntentId) -> ProjectEffectOutcome {
    resolve_project_effect(app, operation_id, None)
}

fn resolve_project_effect(
    app: &AppHandle,
    operation_id: NativeIntentId,
    mut latest: Option<LocalNativeEditorResult>,
) -> ProjectEffectOutcome {
    for attempt in 0..=STATUS_POLLS {
        let Some(result) = latest.take() else {
            if attempt > 0 {
                thread::sleep(STATUS_INTERVAL);
            }
            latest = status_request(app, operation_id);
            continue;
        };
        if result.operation_id != operation_id {
            return ProjectEffectOutcome::Unknown;
        }
        match result.state {
            LocalNativeEffectState::Succeeded => {
                let Some(json) = result.result_json.as_ref() else {
                    return ProjectEffectOutcome::Unknown;
                };
                return serde_json::from_str::<NativeProjectMutationResult>(json.as_str())
                    .map(ProjectEffectOutcome::Succeeded)
                    .unwrap_or(ProjectEffectOutcome::Unknown);
            }
            LocalNativeEffectState::Failed => {
                let Some(json) = result.result_json.as_ref() else {
                    return ProjectEffectOutcome::Unknown;
                };
                if let Ok(code) = serde_json::from_str::<NativeProjectErrorCode>(json.as_str()) {
                    return if code == NativeProjectErrorCode::OutcomeUnknown {
                        ProjectEffectOutcome::Unknown
                    } else {
                        ProjectEffectOutcome::Failed(code)
                    };
                }
                // A pre-start failure in the generic native-file envelope is
                // still a known no-effect result. Keep its established mapping.
                let Ok(error) = serde_json::from_str::<NativeFileError>(json.as_str()) else {
                    return ProjectEffectOutcome::Unknown;
                };
                return match error.kind {
                    NativeFileErrorKind::Io => {
                        ProjectEffectOutcome::Failed(NativeProjectErrorCode::StorageUnavailable)
                    }
                    NativeFileErrorKind::Unsupported | NativeFileErrorKind::ReadOnly => {
                        ProjectEffectOutcome::Failed(NativeProjectErrorCode::UnsupportedCapability)
                    }
                    NativeFileErrorKind::TooLarge => {
                        ProjectEffectOutcome::Failed(NativeProjectErrorCode::ResourceExhausted)
                    }
                    NativeFileErrorKind::Conflict => {
                        ProjectEffectOutcome::Failed(NativeProjectErrorCode::RevisionConflict)
                    }
                    NativeFileErrorKind::OpenFile => {
                        ProjectEffectOutcome::Failed(NativeProjectErrorCode::TargetBusy)
                    }
                    NativeFileErrorKind::Uncertain => ProjectEffectOutcome::Unknown,
                };
            }
            LocalNativeEffectState::OutcomeUnknown | LocalNativeEffectState::Running
                if attempt < STATUS_POLLS =>
            {
                thread::sleep(STATUS_INTERVAL);
                latest = status_request(app, operation_id);
            }
            LocalNativeEffectState::OutcomeUnknown | LocalNativeEffectState::Running => {
                return ProjectEffectOutcome::Unknown;
            }
        }
    }
    ProjectEffectOutcome::Unknown
}

fn resolve_effect(
    app: &AppHandle,
    operation_id: NativeIntentId,
    mut latest: Option<LocalNativeEditorResult>,
    expected: ExpectedResult,
) -> EffectOutcome {
    for attempt in 0..=STATUS_POLLS {
        let Some(result) = latest.take() else {
            if attempt > 0 {
                thread::sleep(STATUS_INTERVAL);
            }
            latest = status_request(app, operation_id);
            continue;
        };
        if result.operation_id != operation_id {
            return EffectOutcome::Unknown;
        }
        match result.state {
            LocalNativeEffectState::Succeeded => {
                return parse_success(result.result_json.as_ref(), expected)
                    .map(EffectOutcome::Succeeded)
                    .unwrap_or(EffectOutcome::Unknown);
            }
            LocalNativeEffectState::Failed => {
                let Some(error) = result
                    .result_json
                    .as_ref()
                    .and_then(|json| serde_json::from_str::<NativeFileError>(json.as_str()).ok())
                else {
                    return EffectOutcome::Unknown;
                };
                return if error.kind == NativeFileErrorKind::Uncertain {
                    EffectOutcome::Unknown
                } else {
                    EffectOutcome::Failed(error)
                };
            }
            LocalNativeEffectState::OutcomeUnknown | LocalNativeEffectState::Running
                if attempt < STATUS_POLLS =>
            {
                thread::sleep(STATUS_INTERVAL);
                latest = status_request(app, operation_id);
            }
            LocalNativeEffectState::OutcomeUnknown | LocalNativeEffectState::Running => {
                return EffectOutcome::Unknown;
            }
        }
    }
    EffectOutcome::Unknown
}

fn status_request(
    app: &AppHandle,
    operation_id: NativeIntentId,
) -> Option<LocalNativeEditorResult> {
    match request(
        app,
        LocalCall::NativeEditorStatus(NativeIntentTarget {
            intent_id: operation_id,
        }),
    ) {
        Ok(LocalResult::NativeEditor(result)) => Some(result),
        _ => None,
    }
}

fn parse_success(
    result_json: Option<&LocalNativeResultJson>,
    expected: ExpectedResult,
) -> Option<NativeSaveResult> {
    let json = result_json?;
    let result = serde_json::from_str::<NativeSaveResult>(json.as_str()).ok()?;
    match (&result, expected) {
        (NativeSaveResult::File(_), ExpectedResult::File)
        | (NativeSaveResult::New(_), ExpectedResult::New) => Some(result),
        _ => None,
    }
}

fn stage_payload(
    operation_id: NativeIntentId,
    staged: &LocalNativeFileStage,
    payload: &[u8],
    expected_sha256: &str,
) -> Result<(), EditorError> {
    let profile = lomi_platform::default_profile()
        .ok_or_else(|| EditorError::new("io", "The canonical host profile is unavailable."))?;
    #[cfg(unix)]
    let mut stage = lomi_platform::unix::open_native_editor_stage(&profile, operation_id, false)
        .map_err(|_| EditorError::new("io", "The private editor stage file is unavailable."))?;
    #[cfg(windows)]
    let mut stage = lomi_platform::windows::open_native_editor_stage(&profile, operation_id, false)
        .map_err(|_| EditorError::new("io", "The private editor stage file is unavailable."))?;
    let expected_path = stage
        .path()
        .to_str()
        .ok_or_else(|| EditorError::new("io", "The private editor stage path is invalid."))?;
    if staged.path.as_str() != expected_path {
        return Err(EditorError::new(
            "io",
            "The host returned an unexpected editor stage file.",
        ));
    }

    let file = stage.file_mut();
    file.set_len(0).map_err(EditorError::io)?;
    file.seek(SeekFrom::Start(0)).map_err(EditorError::io)?;
    file.write_all(payload).map_err(EditorError::io)?;
    file.sync_all().map_err(EditorError::io)?;
    if file.metadata().map_err(EditorError::io)?.len() != payload.len() as u64 {
        return Err(EditorError::new(
            "io",
            "The editor stage file has an unexpected length.",
        ));
    }
    file.seek(SeekFrom::Start(0)).map_err(EditorError::io)?;
    let mut staged_bytes = Vec::with_capacity(payload.len());
    file.take(payload.len() as u64 + 1)
        .read_to_end(&mut staged_bytes)
        .map_err(EditorError::io)?;
    if staged_bytes.len() != payload.len() || revision(&staged_bytes) != expected_sha256 {
        return Err(EditorError::new(
            "io",
            "The editor stage file did not retain the requested content.",
        ));
    }
    drop(stage);
    Ok(())
}

fn request(app: &AppHandle, call: LocalCall) -> Result<LocalResult, String> {
    app.state::<shared_host::SharedHost>()
        .native_file_request(call)
}

fn editor_error(error: NativeFileError) -> EditorError {
    let kind = match error.kind {
        NativeFileErrorKind::Io => "io",
        NativeFileErrorKind::Unsupported => "unsupported",
        NativeFileErrorKind::TooLarge => "tooLarge",
        NativeFileErrorKind::Conflict => "conflict",
        NativeFileErrorKind::ReadOnly => "readOnly",
        NativeFileErrorKind::OpenFile => "openFile",
        NativeFileErrorKind::Uncertain => "uncertain",
    };
    EditorError::new(kind, error.message)
}

fn guarded_native_error(error: NativeFileError) -> GuardedSaveError {
    match error.kind {
        NativeFileErrorKind::Io => GuardedSaveError::Before(ErrorCode::StorageUnavailable),
        NativeFileErrorKind::Unsupported => {
            GuardedSaveError::Before(ErrorCode::UnsupportedCapability)
        }
        NativeFileErrorKind::TooLarge => GuardedSaveError::Before(ErrorCode::ResourceExhausted),
        NativeFileErrorKind::Conflict => GuardedSaveError::Before(ErrorCode::RevisionConflict),
        NativeFileErrorKind::ReadOnly => GuardedSaveError::Before(ErrorCode::UnsupportedCapability),
        NativeFileErrorKind::OpenFile => GuardedSaveError::Before(ErrorCode::TargetBusy),
        NativeFileErrorKind::Uncertain => GuardedSaveError::Uncertain,
    }
}

fn native_project_error_code(code: ErrorCode) -> NativeProjectErrorCode {
    use ErrorCode as Core;
    use NativeProjectErrorCode as Native;
    match code {
        Core::AppUnavailable => Native::AppUnavailable,
        Core::PairingRequired => Native::PairingRequired,
        Core::ScopeDenied => Native::ScopeDenied,
        Core::UiNotReady => Native::UiNotReady,
        Core::AppClosing => Native::AppClosing,
        Core::TargetNotFound => Native::TargetNotFound,
        Core::StaleGeneration => Native::StaleGeneration,
        Core::RevisionConflict => Native::RevisionConflict,
        Core::StaleSnapshot => Native::StaleSnapshot,
        Core::TargetBusy => Native::TargetBusy,
        Core::ControlRevoked => Native::ControlRevoked,
        Core::PromptStateUnknown => Native::PromptStateUnknown,
        Core::PanelNotRenderable => Native::PanelNotRenderable,
        Core::ProtectedOriginTerminal => Native::ProtectedOriginTerminal,
        Core::UnsupportedCapability => Native::UnsupportedCapability,
        Core::HostUnqualified => Native::HostUnqualified,
        Core::DeadlineExceeded => Native::DeadlineExceeded,
        Core::CursorExpired => Native::CursorExpired,
        Core::ArtifactTooLarge => Native::ArtifactTooLarge,
        Core::ArtifactInvalid => Native::ArtifactInvalid,
        Core::OutcomeUnknown => Native::OutcomeUnknown,
        Core::IdempotencyConflict => Native::IdempotencyConflict,
        Core::RetryWindowExpired => Native::RetryWindowExpired,
        Core::ResourceExhausted => Native::ResourceExhausted,
        Core::StorageUnavailable => Native::StorageUnavailable,
    }
}

fn uncertain_error() -> EditorError {
    EditorError::new(
        "uncertain",
        "The save outcome could not be confirmed. Your changes are still in the editor.",
    )
}

fn stage_error() -> EditorError {
    EditorError::new(
        "io",
        "The file could not be staged for saving. Your changes are still in the editor.",
    )
}

fn destination_key(destination: &Path) -> Result<String, EditorError> {
    let path = destination
        .to_str()
        .ok_or_else(|| EditorError::new("unsupported", "This filename is not valid Unicode."))?;
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let path = path.to_lowercase();
    Ok(revision(format!("editor:{path}").as_bytes()))
}

fn request_fingerprint(
    effect: &LocalNativeEditorEffect,
    content_sha256: &str,
    byte_length: u64,
    guarded_identity: Option<(&str, &str)>,
) -> Result<String, EditorError> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Fingerprint<'a> {
        effect: &'a LocalNativeEditorEffect,
        content_sha256: &'a str,
        byte_length: u64,
    }
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct GuardedFingerprint<'a> {
        effect: &'a LocalNativeEditorEffect,
        content_sha256: &'a str,
        byte_length: u64,
        guarded: bool,
        principal: &'a str,
        mcp_operation: &'a str,
    }
    let bytes = match guarded_identity {
        Some((principal, mcp_operation)) => serde_json::to_vec(&GuardedFingerprint {
            effect,
            content_sha256,
            byte_length,
            guarded: true,
            principal,
            mcp_operation,
        }),
        None => serde_json::to_vec(&Fingerprint {
            effect,
            content_sha256,
            byte_length,
        }),
    }
    .map_err(EditorError::io)?;
    Ok(revision(&bytes))
}

fn project_request_fingerprint(
    root: &str,
    principal: &str,
    mcp_operation: &str,
    normalized_action_sha256: &str,
    byte_length: u64,
) -> Result<String, EditorError> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Fingerprint<'a> {
        mode: &'static str,
        principal: &'a str,
        mcp_operation: &'a str,
        root: &'a str,
        normalized_action_sha256: &'a str,
        byte_length: u64,
    }
    let bytes = serde_json::to_vec(&Fingerprint {
        mode: "guardedProjectMutation",
        principal,
        mcp_operation,
        root,
        normalized_action_sha256,
        byte_length,
    })
    .map_err(EditorError::io)?;
    Ok(revision(&bytes))
}

fn project_operation_key(principal: &str, mcp_operation: &str) -> String {
    revision(format!("project-mutation:{principal}:{mcp_operation}").as_bytes())
}

fn artifact_create_request_fingerprint(
    root: &str,
    principal: &str,
    mcp_operation: &str,
    relative_path: &str,
    expected_parent_revision: &str,
    content_sha256: &str,
    byte_length: u64,
) -> Result<String, EditorError> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Fingerprint<'a> {
        mode: &'static str,
        principal: &'a str,
        mcp_operation: &'a str,
        root: &'a str,
        relative_path: &'a str,
        expected_parent_revision: &'a str,
        content_sha256: &'a str,
        byte_length: u64,
    }
    let bytes = serde_json::to_vec(&Fingerprint {
        mode: "guardedArtifactCreate",
        principal,
        mcp_operation,
        root,
        relative_path,
        expected_parent_revision,
        content_sha256,
        byte_length,
    })
    .map_err(EditorError::io)?;
    Ok(revision(&bytes))
}

fn artifact_create_operation_key(principal: &str, mcp_operation: &str) -> String {
    revision(format!("artifact-create:{principal}:{mcp_operation}").as_bytes())
}

fn save_new_recovery_fingerprint(
    effect: &LocalNativeEditorEffect,
    content_sha256: &str,
    byte_length: u64,
) -> Result<Option<String>, EditorError> {
    let LocalNativeEditorEffect::SaveNewChecked {
        absolute_path,
        open_files,
        ..
    } = effect
    else {
        return Ok(None);
    };
    let legacy_effect = LocalNativeEditorEffect::SaveNew {
        absolute_path: absolute_path.clone(),
        open_files: open_files.clone(),
    };
    request_fingerprint(&legacy_effect, content_sha256, byte_length, None).map(Some)
}

fn journal_path(app: &AppHandle) -> Result<PathBuf, EditorError> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(EditorError::io)?
        .join("editor-native-effects.json"))
}

fn read_journal(path: &Path) -> Result<EditorEffectJournal, EditorError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(EditorEffectJournal {
                version: 3,
                operations: BTreeMap::new(),
                acknowledgements: BTreeSet::new(),
            });
        }
        Err(error) => return Err(EditorError::io(error)),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > JOURNAL_LIMIT {
        return Err(EditorError::new(
            "io",
            "The pending editor save journal is unavailable or invalid.",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.mode() & 0o077 != 0
        {
            return Err(EditorError::new(
                "io",
                "The pending editor save journal is not owner-private.",
            ));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(EditorError::new(
                "io",
                "The pending editor save journal is not a regular file.",
            ));
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
    let mut file = options.open(path).map_err(EditorError::io)?;
    let opened_metadata = file.metadata().map_err(EditorError::io)?;
    if !opened_metadata.is_file() || opened_metadata.file_type().is_symlink() {
        return Err(EditorError::new(
            "io",
            "The pending editor save journal is not a regular file.",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened_metadata.dev() != metadata.dev()
            || opened_metadata.ino() != metadata.ino()
            || opened_metadata.uid() != unsafe { libc::geteuid() }
            || opened_metadata.nlink() != 1
            || opened_metadata.mode() & 0o077 != 0
        {
            return Err(EditorError::new(
                "io",
                "The pending editor save journal changed while it was opened.",
            ));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if opened_metadata.file_attributes() & 0x400 != 0 {
            return Err(EditorError::new(
                "io",
                "The pending editor save journal is not a regular file.",
            ));
        }
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    Read::by_ref(&mut file)
        .take(JOURNAL_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(EditorError::io)?;
    if bytes.len() as u64 > JOURNAL_LIMIT {
        return Err(EditorError::new(
            "io",
            "The pending editor save journal is too large.",
        ));
    }
    let mut journal: EditorEffectJournal = serde_json::from_slice(&bytes)
        .map_err(|_| EditorError::new("io", "The pending editor save journal is invalid."))?;
    if !matches!(journal.version, 1 | 2 | 3)
        || journal.operations.len() + journal.acknowledgements.len() > JOURNAL_ENTRIES
        || journal.version == 1 && !journal.acknowledgements.is_empty()
    {
        return Err(EditorError::new(
            "io",
            "The pending editor save journal has an unsupported version.",
        ));
    }
    if journal
        .operations
        .iter()
        .any(|(key, operation)| !valid_hash(key) || !valid_hash(&operation.request_fingerprint))
    {
        return Err(EditorError::new(
            "io",
            "The pending editor save journal contains invalid identifiers.",
        ));
    }
    if journal.operations.values().any(|operation| {
        operation
            .recovery_fingerprint
            .as_deref()
            .is_some_and(|fingerprint| !valid_hash(fingerprint))
    }) {
        return Err(EditorError::new(
            "io",
            "The pending editor save journal contains invalid recovery identifiers.",
        ));
    }
    let operation_ids: BTreeSet<_> = journal
        .operations
        .values()
        .map(|operation| operation.operation_id.to_bytes())
        .collect();
    if operation_ids.len() != journal.operations.len() {
        return Err(EditorError::new(
            "io",
            "The pending editor save journal reuses an operation ID.",
        ));
    }
    if journal
        .acknowledgements
        .iter()
        .any(|id| operation_ids.contains(&id.to_bytes()))
    {
        return Err(EditorError::new(
            "io",
            "The pending editor journal acknowledges an unresolved operation.",
        ));
    }
    journal.version = 3;
    Ok(journal)
}

fn write_journal(path: &Path, journal: &EditorEffectJournal) -> Result<(), EditorError> {
    crate::files::write_json(path, journal, JOURNAL_LIMIT as usize).map_err(EditorError::io)
}

fn clear_operation(
    app: &AppHandle,
    path: &Path,
    journal: &mut EditorEffectJournal,
    destination_key: &str,
) -> Result<(), EditorError> {
    if let Some(pending) = journal.operations.remove(destination_key) {
        if pending.execute_sent {
            journal.acknowledgements.insert(pending.operation_id);
        }
    }
    // Persist the resolved operation and its pending acknowledgement together.
    // The host must retain an unacknowledged result while Desktop might still
    // need it after a crash or a lost response.
    write_journal(path, journal)?;
    // Failure to deliver or persist an acknowledgement leaves a retryable
    // outbox entry; it does not change the already confirmed save result.
    let _ = flush_acknowledgements(app, path, journal);
    Ok(())
}

#[cfg(test)]
mod local_scope_tests {
    use super::*;

    fn pending(expected_result: ExpectedResult, execute_sent: bool) -> PendingEditorEffect {
        PendingEditorEffect {
            operation_id: NativeIntentId::try_from(
                "00000000-0000-4000-8000-000000000001".to_owned(),
            )
            .unwrap(),
            request_fingerprint: "a".repeat(64),
            recovery_fingerprint: None,
            expected_result,
            execute_sent,
        }
    }

    #[test]
    fn local_scope_fences_matching_paths_and_preserves_unrelated_journals() {
        let destination = PathBuf::from("/tmp/lomi-pending-scope-test.txt");
        let key = destination_key(&destination).unwrap();
        let unrelated = destination_key(Path::new("/tmp/lomi-unrelated.txt")).unwrap();
        let journal = EditorEffectJournal {
            version: 3,
            operations: BTreeMap::from([(key.clone(), pending(ExpectedResult::File, true))]),
            acknowledgements: BTreeSet::new(),
        };
        let before = serde_json::to_vec(&journal).unwrap();
        assert!(journal_conflicts_local_scope(
            &journal,
            &BTreeSet::from([key]),
            false
        ));
        assert!(!journal_conflicts_local_scope(
            &journal,
            &BTreeSet::from([unrelated]),
            false
        ));
        assert!(journal_conflicts_local_scope(
            &journal,
            &BTreeSet::new(),
            true
        ));
        assert_eq!(serde_json::to_vec(&journal).unwrap(), before);
    }

    #[test]
    fn local_scope_fences_unscoped_host_mutations_but_ignores_staged_only_effects() {
        let mut journal = EditorEffectJournal {
            version: 3,
            operations: BTreeMap::from([(
                "b".repeat(64),
                pending(ExpectedResult::ProjectMutation, true),
            )]),
            acknowledgements: BTreeSet::new(),
        };
        assert!(journal_conflicts_local_scope(
            &journal,
            &BTreeSet::from([destination_key(Path::new("/tmp/elsewhere")).unwrap()]),
            false
        ));
        journal.operations.values_mut().next().unwrap().execute_sent = false;
        assert!(!journal_conflicts_local_scope(
            &journal,
            &BTreeSet::new(),
            true
        ));
    }
}

fn flush_acknowledgements(
    app: &AppHandle,
    path: &Path,
    journal: &mut EditorEffectJournal,
) -> Result<(), EditorError> {
    // A save must not spend an unbounded number of IPC deadlines draining old
    // acknowledgements. Retries carry only confirmed IDs and never file effects.
    let pending: Vec<_> = journal.acknowledgements.iter().take(4).copied().collect();
    let mut changed = false;
    for operation_id in pending {
        match request(
            app,
            LocalCall::NativeEditorAcknowledge(NativeIntentTarget {
                intent_id: operation_id,
            }),
        ) {
            Ok(LocalResult::NativeEditorAcknowledged(confirmed)) if confirmed == operation_id => {
                journal.acknowledgements.remove(&operation_id);
                changed = true;
            }
            _ => break,
        }
    }
    if changed {
        write_journal(path, journal)?;
    }
    Ok(())
}

fn new_operation_id() -> Result<NativeIntentId, EditorError> {
    use ring::rand::SecureRandom;
    let mut bytes = [0; 16];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| EditorError::new("io", "Secure randomness is unavailable."))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    NativeIntentId::from_bytes(bytes)
        .map_err(|_| EditorError::new("io", "The save operation ID is invalid."))
}

fn revision(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
