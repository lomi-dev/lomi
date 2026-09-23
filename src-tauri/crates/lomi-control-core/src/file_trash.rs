//! Crash-recoverable handoff from a pinned project entry to the system Trash.
//! Staging and source must share a volume. There is no copy/delete fallback.
use crate::{
    atomic_file::{open_file, snapshot, stamp, ReplaceError},
    file_mutation::rename_exclusive,
    project_files::{open_absolute_directory, validate_relative, ProjectDirectory},
};
use lomi_control_protocol::{control::valid_id, files::FileEntryKind, ErrorCode};
use rustix::fs::{mkdirat, openat, Mode, OFlags};
use serde::Serialize;
use std::{
    ffi::CString,
    fs::File,
    io::Write,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Path, PathBuf},
};

fn identity(file: &File) -> Result<(u64, u64), ErrorCode> {
    let m = file.metadata().map_err(|_| ErrorCode::StorageUnavailable)?;
    Ok((m.dev(), m.ino()))
}
fn private_directory(parent: &File, name: &str, create: bool) -> Result<File, ErrorCode> {
    if create {
        mkdirat(parent, name, Mode::from_raw_mode(0o700))
            .map_err(|_| ErrorCode::StorageUnavailable)?;
    }
    let directory = File::from(
        openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| ErrorCode::StorageUnavailable)?,
    );
    let metadata = directory
        .metadata()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o777 != 0o700 {
        return Err(ErrorCode::StorageUnavailable);
    }
    Ok(directory)
}
fn durable_json(directory: &File, name: &str, value: &impl Serialize) -> Result<(), ErrorCode> {
    let bytes = serde_json::to_vec(value).map_err(|_| ErrorCode::StorageUnavailable)?;
    if bytes.len() > 16384 {
        return Err(ErrorCode::ResourceExhausted);
    }
    let mut file = File::from(
        openat(
            directory,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|_| ErrorCode::StorageUnavailable)?,
    );
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    directory
        .sync_all()
        .map_err(|_| ErrorCode::StorageUnavailable)
}

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
    path: PathBuf,
    directory: File,
    payload: File,
    leaf: CString,
    entry_identity: (u64, u64),
    kind: FileEntryKind,
}
impl StagedTrash {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn finish(
        self,
        trash: impl FnOnce(&Path) -> Result<(), ErrorCode>,
    ) -> Result<(), ReplaceError> {
        let operation_path = self
            .path
            .parent()
            .and_then(Path::parent)
            .ok_or(ReplaceError::Uncertain)?;
        if identity(&open_absolute_directory(operation_path).map_err(|_| ReplaceError::Uncertain)?)
            .map_err(|_| ReplaceError::Uncertain)?
            != identity(&self.directory).map_err(|_| ReplaceError::Uncertain)?
            || identity(
                &open_absolute_directory(self.path.parent().unwrap())
                    .map_err(|_| ReplaceError::Uncertain)?,
            )
            .map_err(|_| ReplaceError::Uncertain)?
                != identity(&self.payload).map_err(|_| ReplaceError::Uncertain)?
        {
            return Err(ReplaceError::Uncertain);
        }
        let flags = OFlags::RDONLY
            | OFlags::NOFOLLOW
            | OFlags::NONBLOCK
            | OFlags::CLOEXEC
            | if self.kind == FileEntryKind::Directory {
                OFlags::DIRECTORY
            } else {
                OFlags::empty()
            };
        let entry = File::from(
            openat(&self.payload, &self.leaf, flags, Mode::empty())
                .map_err(|_| ReplaceError::Uncertain)?,
        );
        if identity(&entry).map_err(|_| ReplaceError::Uncertain)? != self.entry_identity {
            return Err(ReplaceError::Uncertain);
        }
        trash(&self.path).map_err(|_| ReplaceError::Uncertain)?;
        if !matches!(
            rustix::fs::statat(
                &self.payload,
                &self.leaf,
                rustix::fs::AtFlags::SYMLINK_NOFOLLOW
            ),
            Err(rustix::io::Errno::NOENT)
        ) {
            return Err(ReplaceError::Uncertain);
        }
        // The OS accepted the Trash operation; a missing durable completion record
        // still requires recovery, never replaying the original project mutation.
        durable_json(
            &self.directory,
            "completed.json",
            &serde_json::json!({"trashed":true}),
        )
        .map_err(|_| ReplaceError::Uncertain)?;
        Ok(())
    }
}

pub fn stage(
    project: &ProjectDirectory,
    target: TrashTarget<'_>,
    recovery_root: &Path,
    operation: &str,
    check: impl Fn() -> Result<(), ErrorCode>,
) -> Result<StagedTrash, ReplaceError> {
    if !valid_id(operation) {
        return Err(ErrorCode::ScopeDenied.into());
    }
    validate_relative(target.relative_path)?;
    check()?;
    let relative = Path::new(target.relative_path);
    let parent_name = relative
        .parent()
        .and_then(Path::to_str)
        .ok_or(ErrorCode::ScopeDenied)?;
    let leaf_name = relative.file_name().ok_or(ErrorCode::ScopeDenied)?;
    let leaf = CString::new(leaf_name.as_bytes()).map_err(|_| ErrorCode::ScopeDenied)?;
    let parent = project.open_directory(parent_name)?;
    let parent_identity = identity(&parent)?;
    if project.list(parent_name, &check)?.revision != target.expected_parent_revision {
        return Err(ErrorCode::RevisionConflict.into());
    }
    let (entry, expected_stamp) = match target.kind {
        FileEntryKind::File => {
            let mut entry = open_file(&parent, &leaf)?;
            if entry
                .metadata()
                .map_err(|_| ErrorCode::StorageUnavailable)?
                .len()
                > 4 * 1024 * 1024
            {
                return Err(ErrorCode::ResourceExhausted.into());
            }
            let (revision, metadata) = snapshot(&mut entry, &check)?;
            if revision != target.expected_revision {
                return Err(ErrorCode::RevisionConflict.into());
            }
            (entry, Some(stamp(&metadata)))
        }
        FileEntryKind::Directory => {
            let entry = project.open_directory(target.relative_path)?;
            if project.list(target.relative_path, &check)?.revision != target.expected_revision {
                return Err(ErrorCode::RevisionConflict.into());
            }
            (entry, None)
        }
    };
    let entry_identity = identity(&entry)?;
    let recovery = open_absolute_directory(recovery_root)?;
    let recovery_identity = identity(&recovery)?;
    let metadata = recovery
        .metadata()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o777 != 0o700 {
        return Err(ErrorCode::StorageUnavailable.into());
    }
    if entry_identity.0 != recovery_identity.0 {
        return Err(ErrorCode::UnsupportedCapability.into());
    }
    let directory = private_directory(&recovery, operation, true)?;
    let payload = private_directory(&directory, "entry", true)?;
    durable_json(
        &directory,
        "plan.json",
        &serde_json::json!({
            "version":1, "operationId":operation, "projectPath":project.canonical_path(),
            "relativePath":target.relative_path, "kind":target.kind,
            "expectedRevision":target.expected_revision, "expectedParentRevision":target.expected_parent_revision,
            "device":entry_identity.0.to_string(), "inode":entry_identity.1.to_string()
        }),
    )?;
    recovery
        .sync_all()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    check()?;
    project.check()?;
    if identity(&project.open_directory(parent_name)?)? != parent_identity
        || identity(&open_absolute_directory(recovery_root)?)? != recovery_identity
    {
        return Err(ErrorCode::RevisionConflict.into());
    }
    match target.kind {
        FileEntryKind::File => {
            let (revision, metadata) = snapshot(&mut open_file(&parent, &leaf)?, &check)?;
            if revision != target.expected_revision || Some(stamp(&metadata)) != expected_stamp {
                return Err(ErrorCode::RevisionConflict.into());
            }
        }
        FileEntryKind::Directory => {
            if identity(&project.open_directory(target.relative_path)?)? != entry_identity
                || project.list(target.relative_path, &check)?.revision != target.expected_revision
            {
                return Err(ErrorCode::RevisionConflict.into());
            }
        }
    }
    check()?;
    project.check()?;
    if identity(&project.open_directory(parent_name)?)? != parent_identity
        || identity(&open_absolute_directory(recovery_root)?)? != recovery_identity
        || identity(&open_absolute_directory(&recovery_root.join(operation))?)?
            != identity(&directory)?
    {
        return Err(ErrorCode::RevisionConflict.into());
    }
    rename_exclusive(&parent, &leaf, &payload, &leaf)?;
    parent
        .sync_all()
        .and_then(|_| payload.sync_all())
        .map_err(|_| ReplaceError::Uncertain)?;
    Ok(StagedTrash {
        path: recovery_root.join(operation).join("entry").join(leaf_name),
        directory,
        payload,
        leaf,
        entry_identity,
        kind: target.kind.clone(),
    })
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
