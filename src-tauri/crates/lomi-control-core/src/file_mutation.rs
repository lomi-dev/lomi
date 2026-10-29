//! Compatibility wrappers for neutral project filesystem mutations.
use crate::{
    atomic_file::ReplaceError,
    project_files::{to_native_error, to_protocol_error, ProjectDirectory},
};
use lomi_control_protocol::{files::FileEntryKind, ErrorCode};
use lomi_native_project_files::{
    NativeProjectEntryKind, NativeProjectMutation, NativeProjectMutationError,
};
#[cfg(test)]
use std::os::unix::fs::MetadataExt;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, fs, os::unix::fs::symlink};

    #[test]
    fn directory_move_preserves_descendants_and_refuses_collisions_and_self_moves() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        fs::create_dir_all(root.join("source/nested")).unwrap();
        fs::write(root.join("source/nested/file.txt"), b"retain bytes").unwrap();
        fs::create_dir(root.join("destination")).unwrap();
        let inode = fs::metadata(root.join("source/nested/file.txt"))
            .unwrap()
            .ino();
        let project = ProjectDirectory::open(&root).unwrap();
        let revision = project.list("source", || Ok(())).unwrap().revision;
        let parent = project.list("", || Ok(())).unwrap().revision;
        assert!(matches!(
            move_directory(
                &project,
                "source",
                "source/nested/self",
                &revision,
                &parent,
                || Ok(())
            ),
            Err(ReplaceError::Before(ErrorCode::ScopeDenied))
        ));
        assert!(move_directory(
            &project,
            "source",
            "destination",
            &revision,
            &parent,
            || Ok(())
        )
        .is_err());
        assert!(root.join("source/nested/file.txt").exists());
        let parent = project.list("destination", || Ok(())).unwrap().revision;
        move_directory(
            &project,
            "source",
            "destination/renamed",
            &revision,
            &parent,
            || Ok(()),
        )
        .unwrap();
        assert!(!root.join("source").exists());
        assert_eq!(
            fs::metadata(root.join("destination/renamed/nested/file.txt"))
                .unwrap()
                .ino(),
            inode
        );
        assert_eq!(
            fs::read(root.join("destination/renamed/nested/file.txt")).unwrap(),
            b"retain bytes"
        );
    }

    #[test]
    fn move_preserves_identity_and_rejects_stale_bytes_collision_and_links() {
        use sha2::{Digest, Sha256};
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        fs::create_dir(root.join("folder")).unwrap();
        fs::write(root.join("source.txt"), "Zażółć 🙂").unwrap();
        let inode = fs::metadata(root.join("source.txt")).unwrap().ino();
        let project = ProjectDirectory::open(&root).unwrap();
        let parent = project.list("folder", || Ok(())).unwrap().revision;
        assert!(matches!(
            move_file(
                &project,
                "source.txt",
                "folder/new.txt",
                &"0".repeat(64),
                &parent,
                || Ok(())
            ),
            Err(ReplaceError::Before(ErrorCode::RevisionConflict))
        ));
        let hash = format!("{:x}", Sha256::digest("Zażółć 🙂".as_bytes()));
        move_file(
            &project,
            "source.txt",
            "folder/new.txt",
            &hash,
            &parent,
            || Ok(()),
        )
        .unwrap();
        assert!(!root.join("source.txt").exists());
        assert_eq!(
            fs::metadata(root.join("folder/new.txt")).unwrap().ino(),
            inode
        );
        fs::write(root.join("source.txt"), "keep destination").unwrap();
        let parent = project.list("", || Ok(())).unwrap().revision;
        assert!(matches!(
            move_file(
                &project,
                "folder/new.txt",
                "source.txt",
                &hash,
                &parent,
                || Ok(())
            ),
            Err(ReplaceError::Before(ErrorCode::RevisionConflict))
        ));
        assert_eq!(
            fs::read_to_string(root.join("source.txt")).unwrap(),
            "keep destination"
        );
        symlink(root.join("source.txt"), root.join("alias")).unwrap();
        let parent = project.list("", || Ok(())).unwrap().revision;
        assert!(matches!(
            move_file(&project, "folder/new.txt", "alias", &hash, &parent, || Ok(
                ()
            )),
            Err(ReplaceError::Before(ErrorCode::RevisionConflict))
        ));
        assert_eq!(
            fs::read_to_string(root.join("folder/new.txt")).unwrap(),
            "Zażółć 🙂"
        );
    }

    #[test]
    fn create_checks_parent_revision_and_never_replaces_existing_entries() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let project = ProjectDirectory::open(&root).unwrap();
        let revision = project.list("", || Ok(())).unwrap().revision;
        create(&project, "new.txt", &revision, &FileEntryKind::File, || {
            Ok(())
        })
        .unwrap();
        assert_eq!(fs::read(root.join("new.txt")).unwrap(), b"");
        assert_eq!(
            fs::metadata(root.join("new.txt")).unwrap().mode() & 0o777,
            0o600
        );
        assert!(matches!(
            create(
                &project,
                "folder",
                &revision,
                &FileEntryKind::Directory,
                || Ok(())
            ),
            Err(ReplaceError::Before(ErrorCode::RevisionConflict))
        ));
        let revision = project.list("", || Ok(())).unwrap().revision;
        fs::write(root.join("new.txt"), b"user bytes").unwrap();
        assert!(matches!(
            create(&project, "new.txt", &revision, &FileEntryKind::File, || Ok(
                ()
            )),
            Err(ReplaceError::Before(ErrorCode::RevisionConflict))
        ));
        assert_eq!(fs::read(root.join("new.txt")).unwrap(), b"user bytes");
        let revision = project.list("", || Ok(())).unwrap().revision;
        create(
            &project,
            "folder",
            &revision,
            &FileEntryKind::Directory,
            || Ok(()),
        )
        .unwrap();
        assert!(root.join("folder").is_dir());
        assert!(matches!(
            create(&project, "folder/.env", "", &FileEntryKind::File, || Ok(())),
            Err(ReplaceError::Before(ErrorCode::ScopeDenied))
        ));
    }

    #[test]
    fn cancelled_or_replaced_parent_never_creates_at_a_link_target() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        fs::create_dir(root.join("parent")).unwrap();
        fs::create_dir(root.join("outside")).unwrap();
        let project = ProjectDirectory::open(&root).unwrap();
        let revision = project.list("parent", || Ok(())).unwrap().revision;
        assert!(matches!(
            create(
                &project,
                "parent/new",
                &revision,
                &FileEntryKind::File,
                || Err(ErrorCode::ControlRevoked)
            ),
            Err(ReplaceError::Before(ErrorCode::ControlRevoked))
        ));
        let calls = Cell::new(0);
        let result = create(
            &project,
            "parent/new",
            &revision,
            &FileEntryKind::File,
            || {
                calls.set(calls.get() + 1);
                if calls.get() == 2 {
                    fs::rename(root.join("parent"), root.join("old-parent")).unwrap();
                    symlink(root.join("outside"), root.join("parent")).unwrap();
                }
                Ok(())
            },
        );
        assert!(matches!(result, Err(ReplaceError::Before(_))));
        assert!(!root.join("outside/new").exists());
        assert!(!root.join("old-parent/new").exists());
    }
}
