//! Non-overwriting project entry creation at a pinned directory descriptor.
use crate::{atomic_file::ReplaceError, project_files::ProjectDirectory};
use lomi_control_protocol::{files::FileEntryKind, ErrorCode};
use std::{fs::File, os::unix::fs::MetadataExt, path::Path};

/// New files are empty and private. Content changes use the revisioned editor.
pub fn create(
    project: &ProjectDirectory,
    relative: &str,
    expected_parent_revision: &str,
    kind: &FileEntryKind,
    check: impl Fn() -> Result<(), ErrorCode>,
) -> Result<(), ReplaceError> {
    crate::project_files::validate_relative(relative)?;
    let path = Path::new(relative);
    let parent_name = path
        .parent()
        .and_then(Path::to_str)
        .ok_or(ErrorCode::ScopeDenied)?;
    let leaf = path.file_name().ok_or(ErrorCode::ScopeDenied)?;
    check()?;
    let parent = project.open_directory(parent_name)?;
    let metadata = parent
        .metadata()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let same_parent = || {
        project.check()?;
        let current = project
            .open_directory(parent_name)?
            .metadata()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        if (current.dev(), current.ino()) != (metadata.dev(), metadata.ino()) {
            return Err(ErrorCode::RevisionConflict);
        }
        Ok(())
    };
    if project.list(parent_name, &check)?.revision != expected_parent_revision {
        return Err(ErrorCode::RevisionConflict.into());
    }
    check()?;
    same_parent()?;
    use rustix::fs::{mkdirat, openat, Mode, OFlags};
    let created: Option<File> = match kind {
        FileEntryKind::File => Some(
            openat(
                &parent,
                leaf,
                OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::WRONLY,
                Mode::from_raw_mode(0o600),
            )
            .map(File::from)
            .map_err(before)?,
        ),
        FileEntryKind::Directory => {
            mkdirat(&parent, leaf, Mode::from_raw_mode(0o700)).map_err(before)?;
            None
        }
    };
    // Creation has happened. Do not remove the new entry on cancellation or a
    // sync error: a user may already have opened it or written to it.
    if let Some(file) = created {
        file.sync_all().map_err(|_| ReplaceError::Uncertain)?;
    } else {
        let created: File = openat(
            &parent,
            leaf,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(|_| ReplaceError::Uncertain)?;
        created.sync_all().map_err(|_| ReplaceError::Uncertain)?;
    }
    parent.sync_all().map_err(|_| ReplaceError::Uncertain)?;
    same_parent().map_err(|_| ReplaceError::Uncertain)?;
    Ok(())
}

fn before(error: rustix::io::Errno) -> ReplaceError {
    ReplaceError::Before(if error == rustix::io::Errno::EXIST {
        ErrorCode::RevisionConflict
    } else if error == rustix::io::Errno::ACCESS || error == rustix::io::Errno::PERM {
        ErrorCode::ScopeDenied
    } else {
        ErrorCode::StorageUnavailable
    })
}

/// Rename one bounded regular file without replacing a destination. Cross-device
/// moves have no copy/delete fallback, so failure leaves the source intact.
pub fn move_file(
    project: &ProjectDirectory,
    source: &str,
    destination: &str,
    expected_disk_revision: &str,
    expected_parent_revision: &str,
    check: impl Fn() -> Result<(), ErrorCode>,
) -> Result<(), ReplaceError> {
    use crate::atomic_file::{open_file, snapshot, stamp};
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    for path in [source, destination] {
        crate::project_files::validate_relative(path)?;
    }
    if source == destination {
        return Err(ErrorCode::RevisionConflict.into());
    }
    check()?;
    let source = Path::new(source);
    let destination = Path::new(destination);
    let source_parent_name = source
        .parent()
        .and_then(Path::to_str)
        .ok_or(ErrorCode::ScopeDenied)?;
    let destination_parent_name = destination
        .parent()
        .and_then(Path::to_str)
        .ok_or(ErrorCode::ScopeDenied)?;
    let source_name = CString::new(source.file_name().ok_or(ErrorCode::ScopeDenied)?.as_bytes())
        .map_err(|_| ErrorCode::ScopeDenied)?;
    let destination_name = CString::new(
        destination
            .file_name()
            .ok_or(ErrorCode::ScopeDenied)?
            .as_bytes(),
    )
    .map_err(|_| ErrorCode::ScopeDenied)?;
    let source_parent = project.open_directory(source_parent_name)?;
    let destination_parent = project.open_directory(destination_parent_name)?;
    let parent_identity = |file: &File| -> Result<_, ErrorCode> {
        let m = file.metadata().map_err(|_| ErrorCode::StorageUnavailable)?;
        Ok((m.dev(), m.ino()))
    };
    let source_identity = parent_identity(&source_parent)?;
    let destination_identity = parent_identity(&destination_parent)?;
    let verify_parents = || {
        project.check()?;
        if parent_identity(&project.open_directory(source_parent_name)?)? != source_identity
            || parent_identity(&project.open_directory(destination_parent_name)?)?
                != destination_identity
        {
            return Err(ErrorCode::RevisionConflict);
        }
        Ok(())
    };
    if project.list(destination_parent_name, &check)?.revision != expected_parent_revision {
        return Err(ErrorCode::RevisionConflict.into());
    }
    let mut original = open_file(&source_parent, &source_name)?;
    if original
        .metadata()
        .map_err(|_| ErrorCode::StorageUnavailable)?
        .len()
        > 4 * 1024 * 1024
    {
        return Err(ErrorCode::ResourceExhausted.into());
    }
    let (revision, metadata) = snapshot(&mut original, &check)?;
    if revision != expected_disk_revision {
        return Err(ErrorCode::RevisionConflict.into());
    }
    let (current_revision, current_metadata) =
        snapshot(&mut open_file(&source_parent, &source_name)?, &check)?;
    if current_revision != revision || stamp(&metadata) != stamp(&current_metadata) {
        return Err(ErrorCode::RevisionConflict.into());
    }
    check()?;
    verify_parents()?;
    rename_exclusive(
        &source_parent,
        &source_name,
        &destination_parent,
        &destination_name,
    )?;
    // After rename, uncertainty must never become a replay or a rollback.
    source_parent
        .sync_all()
        .map_err(|_| ReplaceError::Uncertain)?;
    destination_parent
        .sync_all()
        .map_err(|_| ReplaceError::Uncertain)?;
    verify_parents().map_err(|_| ReplaceError::Uncertain)?;
    let moved = open_file(&destination_parent, &destination_name)
        .map_err(|_| ReplaceError::Uncertain)?
        .metadata()
        .map_err(|_| ReplaceError::Uncertain)?;
    if (moved.dev(), moved.ino()) != (metadata.dev(), metadata.ino()) {
        return Err(ReplaceError::Uncertain);
    }
    Ok(())
}

pub(crate) fn rename_exclusive(
    source: &File,
    source_name: &std::ffi::CStr,
    destination: &File,
    destination_name: &std::ffi::CStr,
) -> Result<(), ReplaceError> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use rustix::fs::{renameat_with, RenameFlags};
        renameat_with(
            source,
            source_name,
            destination,
            destination_name,
            RenameFlags::NOREPLACE,
        )
        .map_err(|e| {
            if e == rustix::io::Errno::XDEV
                || e == rustix::io::Errno::NOSYS
                || e == rustix::io::Errno::NOTSUP
            {
                ErrorCode::UnsupportedCapability.into()
            } else {
                before(e)
            }
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (source, source_name, destination, destination_name);
        Err(ErrorCode::HostUnqualified.into())
    }
}

/// Directory revisions describe immediate entries, not a recursive content
/// snapshot. Renaming preserves every descendant and does not inspect its bytes.
pub fn move_directory(
    project: &ProjectDirectory,
    source: &str,
    destination: &str,
    expected_directory_revision: &str,
    expected_parent_revision: &str,
    check: impl Fn() -> Result<(), ErrorCode>,
) -> Result<(), ReplaceError> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    for path in [source, destination] {
        crate::project_files::validate_relative(path)?;
    }
    if source == destination || destination.starts_with(&format!("{source}/")) {
        return Err(ErrorCode::ScopeDenied.into());
    }
    check()?;
    let source_path = Path::new(source);
    let destination_path = Path::new(destination);
    let source_parent_name = source_path
        .parent()
        .and_then(Path::to_str)
        .ok_or(ErrorCode::ScopeDenied)?;
    let destination_parent_name = destination_path
        .parent()
        .and_then(Path::to_str)
        .ok_or(ErrorCode::ScopeDenied)?;
    let source_name = CString::new(
        source_path
            .file_name()
            .ok_or(ErrorCode::ScopeDenied)?
            .as_bytes(),
    )
    .map_err(|_| ErrorCode::ScopeDenied)?;
    let destination_name = CString::new(
        destination_path
            .file_name()
            .ok_or(ErrorCode::ScopeDenied)?
            .as_bytes(),
    )
    .map_err(|_| ErrorCode::ScopeDenied)?;
    let source_parent = project.open_directory(source_parent_name)?;
    let destination_parent = project.open_directory(destination_parent_name)?;
    let directory = project.open_directory(source)?;
    let identity = |file: &File| -> Result<_, ErrorCode> {
        let m = file.metadata().map_err(|_| ErrorCode::StorageUnavailable)?;
        Ok((m.dev(), m.ino()))
    };
    let source_parent_identity = identity(&source_parent)?;
    let destination_parent_identity = identity(&destination_parent)?;
    let source_identity = identity(&directory)?;
    let verify_parents = || {
        project.check()?;
        if identity(&project.open_directory(source_parent_name)?)? != source_parent_identity
            || identity(&project.open_directory(destination_parent_name)?)?
                != destination_parent_identity
        {
            return Err(ErrorCode::RevisionConflict);
        }
        Ok(())
    };
    if project.list(source, &check)?.revision != expected_directory_revision
        || project.list(destination_parent_name, &check)?.revision != expected_parent_revision
    {
        return Err(ErrorCode::RevisionConflict.into());
    }
    check()?;
    verify_parents()?;
    if identity(&project.open_directory(source)?)? != source_identity {
        return Err(ErrorCode::RevisionConflict.into());
    }
    rename_exclusive(
        &source_parent,
        &source_name,
        &destination_parent,
        &destination_name,
    )?;
    source_parent
        .sync_all()
        .map_err(|_| ReplaceError::Uncertain)?;
    destination_parent
        .sync_all()
        .map_err(|_| ReplaceError::Uncertain)?;
    verify_parents().map_err(|_| ReplaceError::Uncertain)?;
    if identity(
        &project
            .open_directory(destination)
            .map_err(|_| ReplaceError::Uncertain)?,
    )
    .map_err(|_| ReplaceError::Uncertain)?
        != source_identity
    {
        return Err(ReplaceError::Uncertain);
    }
    Ok(())
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
