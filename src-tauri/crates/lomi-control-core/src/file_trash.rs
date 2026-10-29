//! Compatibility facade over the protocol-neutral staged-trash engine.
use crate::{
    atomic_file::ReplaceError,
    project_files::{to_native_error, to_protocol_error, ProjectDirectory},
};
use lomi_control_protocol::{files::FileEntryKind, ErrorCode};
use lomi_native_project_files::{
    NativeProjectEntryKind, NativeProjectMutation, NativeProjectMutationError,
    NativeProjectMutationResult, NativeProjectStagedTrash, NativeProjectTrashTicket,
};
use std::path::Path;

pub struct TrashTarget<'a> {
    pub relative_path: &'a str,
    pub kind: &'a FileEntryKind,
    /// File SHA-256 or immediate directory entry revision, respectively.
    pub expected_revision: &'a str,
    pub expected_parent_revision: &'a str,
}

/// A failed handoff retains the staged entry and immutable plan for recovery.
/// Dropping this value never deletes or restores anything automatically.
pub struct StagedTrash {
    inner: NativeProjectStagedTrash,
}

impl StagedTrash {
    pub fn path(&self) -> &Path {
        self.inner.path()
    }

    pub fn ticket(&self) -> NativeProjectTrashTicket {
        self.inner.ticket()
    }

    pub fn finish(
        self,
        trash: impl FnOnce(&Path) -> Result<(), ErrorCode>,
    ) -> Result<(), ReplaceError> {
        self.inner
            .finish(|path| trash(path).map_err(to_native_error))
            .map_err(mutation_error)
    }
}

fn mutation_error(error: NativeProjectMutationError) -> ReplaceError {
    match error {
        NativeProjectMutationError::Before(code) => ReplaceError::Before(to_protocol_error(code)),
        NativeProjectMutationError::Uncertain => ReplaceError::Uncertain,
    }
}

/// Reopen a staged entry after validating its durable plan and private paths.
pub fn from_ticket(ticket: NativeProjectTrashTicket) -> Result<StagedTrash, ReplaceError> {
    lomi_native_project_files::from_ticket(ticket)
        .map(|inner| StagedTrash { inner })
        .map_err(mutation_error)
}

pub fn stage(
    project: &ProjectDirectory,
    target: TrashTarget<'_>,
    recovery_root: &Path,
    operation: &str,
    check: impl Fn() -> Result<(), ErrorCode>,
) -> Result<StagedTrash, ReplaceError> {
    let kind = match target.kind {
        FileEntryKind::File => NativeProjectEntryKind::File,
        FileEntryKind::Directory => NativeProjectEntryKind::Directory,
    };
    let result = project
        .native()
        .apply(
            NativeProjectMutation::TrashStage {
                relative_path: target.relative_path.to_owned(),
                kind,
                expected_revision: target.expected_revision.to_owned(),
                expected_parent_revision: target.expected_parent_revision.to_owned(),
                recovery_root: recovery_root.to_path_buf(),
                operation_id: operation.to_owned(),
            },
            || check().map_err(to_native_error),
        )
        .map_err(mutation_error)?;
    let NativeProjectMutationResult::TrashStaged { ticket } = result else {
        return Err(ReplaceError::Uncertain);
    };
    from_ticket(ticket)
}
