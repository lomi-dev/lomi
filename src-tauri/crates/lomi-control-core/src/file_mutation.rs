//! Compatibility wrappers for neutral project filesystem mutations.
use crate::{
    atomic_file::ReplaceError,
    project_files::{to_native_error, to_protocol_error, ProjectDirectory},
};
use lomi_control_protocol::{files::FileEntryKind, ErrorCode};
use lomi_native_project_files::{
    NativeProjectEntryKind, NativeProjectMutation, NativeProjectMutationError,
};

fn mutation_error(error: NativeProjectMutationError) -> ReplaceError {
    match error {
        NativeProjectMutationError::Before(code) => ReplaceError::Before(to_protocol_error(code)),
        NativeProjectMutationError::Uncertain => ReplaceError::Uncertain,
    }
}

/// New files are empty and private. Content changes use the revisioned editor.
pub fn create(
    project: &ProjectDirectory,
    relative: &str,
    expected_parent_revision: &str,
    kind: &FileEntryKind,
    check: impl Fn() -> Result<(), ErrorCode>,
) -> Result<(), ReplaceError> {
    let kind = match kind {
        FileEntryKind::File => NativeProjectEntryKind::File,
        FileEntryKind::Directory => NativeProjectEntryKind::Directory,
    };
    project
        .native()
        .apply(
            NativeProjectMutation::Create {
                relative_path: relative.to_owned(),
                kind,
                expected_parent_revision: expected_parent_revision.to_owned(),
            },
            || check().map_err(to_native_error),
        )
        .map(|_| ())
        .map_err(mutation_error)
}

/// Rename one bounded regular file without replacing a destination.
pub fn move_file(
    project: &ProjectDirectory,
    source: &str,
    destination: &str,
    expected_disk_revision: &str,
    expected_parent_revision: &str,
    check: impl Fn() -> Result<(), ErrorCode>,
) -> Result<(), ReplaceError> {
    project
        .native()
        .apply(
            NativeProjectMutation::RenameMoveFile {
                relative_path: source.to_owned(),
                target_relative_path: destination.to_owned(),
                expected_disk_revision: expected_disk_revision.to_owned(),
                expected_parent_revision: expected_parent_revision.to_owned(),
            },
            || check().map_err(to_native_error),
        )
        .map(|_| ())
        .map_err(mutation_error)
}

/// Directory moves are nonrecursive and preserve all descendants in place.
pub fn move_directory(
    project: &ProjectDirectory,
    source: &str,
    destination: &str,
    expected_directory_revision: &str,
    expected_parent_revision: &str,
    check: impl Fn() -> Result<(), ErrorCode>,
) -> Result<(), ReplaceError> {
    project
        .native()
        .apply(
            NativeProjectMutation::RenameMoveDirectory {
                relative_path: source.to_owned(),
                target_relative_path: destination.to_owned(),
                expected_directory_revision: expected_directory_revision.to_owned(),
                expected_parent_revision: expected_parent_revision.to_owned(),
            },
            || check().map_err(to_native_error),
        )
        .map(|_| ())
        .map_err(mutation_error)
}
