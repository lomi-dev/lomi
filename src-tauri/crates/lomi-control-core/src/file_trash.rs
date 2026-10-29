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

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{
        cell::Cell,
        fs,
        os::unix::fs::{symlink, PermissionsExt},
    };

    #[test]
    fn failed_trash_retains_exact_bytes_and_durable_recovery_plan() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::create_dir(root.join("project")).unwrap();
        fs::create_dir(root.join("recovery")).unwrap();
        fs::set_permissions(root.join("recovery"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(root.join("project/Zażółć 🙂.txt"), b"precious\r\n").unwrap();
        let project = ProjectDirectory::open(&root.join("project")).unwrap();
        let revision = format!("{:x}", Sha256::digest(b"precious\r\n"));
        let parent = project.list("", || Ok(())).unwrap().revision;
        let target = || TrashTarget {
            relative_path: "Zażółć 🙂.txt",
            kind: &FileEntryKind::File,
            expected_revision: &revision,
            expected_parent_revision: &parent,
        };
        let staged = stage(&project, target(), &root.join("recovery"), "op1", || Ok(())).unwrap();
        let entry = staged.path().to_owned();
        assert!(!root.join("project/Zażółć 🙂.txt").exists());
        assert_eq!(fs::read(&entry).unwrap(), b"precious\r\n");
        let plan: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("recovery/op1/plan.json")).unwrap())
                .unwrap();
        assert_eq!(plan["relativePath"], "Zażółć 🙂.txt");
        assert!(matches!(
            staged.finish(|_| Err(ErrorCode::StorageUnavailable)),
            Err(ReplaceError::Uncertain)
        ));
        assert_eq!(fs::read(&entry).unwrap(), b"precious\r\n");
        assert!(!root.join("recovery/op1/completed.json").exists());
        assert!(stage(&project, target(), &root.join("recovery"), "op1", || Ok(())).is_err());
    }

    #[test]
    fn directory_handoff_preserves_descendants_and_records_completion() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("project/folder/nested")).unwrap();
        fs::create_dir(root.join("recovery")).unwrap();
        fs::set_permissions(root.join("recovery"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(root.join("project/folder/nested/data"), b"retained").unwrap();
        let inode = fs::metadata(root.join("project/folder/nested/data"))
            .unwrap()
            .ino();
        let project = ProjectDirectory::open(&root.join("project")).unwrap();
        let revision = project.list("folder", || Ok(())).unwrap().revision;
        let parent = project.list("", || Ok(())).unwrap().revision;
        let staged = stage(
            &project,
            TrashTarget {
                relative_path: "folder",
                kind: &FileEntryKind::Directory,
                expected_revision: &revision,
                expected_parent_revision: &parent,
            },
            &root.join("recovery"),
            "op2",
            || Ok(()),
        )
        .unwrap();
        staged
            .finish(|path| {
                fs::rename(path, root.join("system-trash-fixture"))
                    .map_err(|_| ErrorCode::StorageUnavailable)
            })
            .unwrap();
        assert_eq!(
            fs::read(root.join("system-trash-fixture/nested/data")).unwrap(),
            b"retained"
        );
        assert_eq!(
            fs::metadata(root.join("system-trash-fixture/nested/data"))
                .unwrap()
                .ino(),
            inode
        );
        assert!(root.join("recovery/op2/completed.json").exists());
    }

    #[test]
    fn cancellation_and_parent_replacement_do_not_move_link_target() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("project/folder")).unwrap();
        fs::create_dir(root.join("outside")).unwrap();
        fs::create_dir(root.join("recovery")).unwrap();
        fs::set_permissions(root.join("recovery"), fs::Permissions::from_mode(0o700)).unwrap();
        for directory in ["project/folder", "outside"] {
            fs::write(root.join(directory).join("file"), b"same").unwrap();
        }
        let project = ProjectDirectory::open(&root.join("project")).unwrap();
        let revision = format!("{:x}", Sha256::digest(b"same"));
        let parent = project.list("folder", || Ok(())).unwrap().revision;
        let target = || TrashTarget {
            relative_path: "folder/file",
            kind: &FileEntryKind::File,
            expected_revision: &revision,
            expected_parent_revision: &parent,
        };
        assert!(matches!(
            stage(
                &project,
                target(),
                &root.join("recovery"),
                "cancelled",
                || Err(ErrorCode::ControlRevoked)
            ),
            Err(ReplaceError::Before(ErrorCode::ControlRevoked))
        ));
        let swapped = Cell::new(false);
        let outcome = stage(
            &project,
            target(),
            &root.join("recovery"),
            "swapped",
            || {
                if root.join("recovery/swapped/plan.json").exists() && !swapped.replace(true) {
                    fs::rename(root.join("project/folder"), root.join("project/moved")).unwrap();
                    symlink(root.join("outside"), root.join("project/folder")).unwrap();
                }
                Ok(())
            },
        );
        assert!(matches!(outcome, Err(ReplaceError::Before(_))));
        assert_eq!(fs::read(root.join("outside/file")).unwrap(), b"same");
        assert_eq!(fs::read(root.join("project/moved/file")).unwrap(), b"same");
        assert!(!root.join("recovery/swapped/entry/file").exists());
    }
}
