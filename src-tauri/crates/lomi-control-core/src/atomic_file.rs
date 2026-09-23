//! Atomic replacement at a pinned parent descriptor, shared by UI and MCP saves.
//! The caller authorizes the canonical target; this module never follows aliases.
use lomi_control_protocol::ErrorCode;
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::{File, Metadata, Permissions},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, PermissionsExt},
        },
    },
    path::Path,
};

const LIMIT: u64 = 16 * 1024 * 1024;

#[derive(Debug)]
pub enum ReplaceError {
    /// The target was not replaced. A temporary sibling may have been created.
    Before(ErrorCode),
    /// Rename completed, but durable completion could not be confirmed.
    Uncertain,
}
impl From<ErrorCode> for ReplaceError {
    fn from(value: ErrorCode) -> Self {
        Self::Before(value)
    }
}

struct Temporary<'a> {
    parent: &'a File,
    name: CString,
    file: File,
    linked: bool,
}
impl Drop for Temporary<'_> {
    fn drop(&mut self) {
        if self.linked {
            unsafe {
                libc::unlinkat(self.parent.as_raw_fd(), self.name.as_ptr(), 0);
            }
        }
    }
}

pub(crate) fn stamp(m: &Metadata) -> (u64, u64, u64, u64, i64, i64, i64, i64) {
    (
        m.dev(),
        m.ino(),
        m.len(),
        m.nlink(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    )
}
fn identity(m: &Metadata) -> (u64, u64) {
    (m.dev(), m.ino())
}
pub(crate) fn open_file(parent: &File, name: &CString) -> Result<File, ErrorCode> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(ErrorCode::RevisionConflict);
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
pub(crate) fn snapshot(
    file: &mut File,
    check: &impl Fn() -> Result<(), ErrorCode>,
) -> Result<(String, Metadata), ErrorCode> {
    let before = file.metadata().map_err(|_| ErrorCode::StorageUnavailable)?;
    if !before.is_file() || before.nlink() != 1 {
        return Err(ErrorCode::ScopeDenied);
    }
    if before.len() > LIMIT {
        return Err(ErrorCode::ResourceExhausted);
    }
    let mut hash = Sha256::new();
    let mut bytes = [0; 65536];
    let mut total = 0;
    loop {
        check()?;
        let count = file
            .read(&mut bytes)
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > LIMIT {
            return Err(ErrorCode::ResourceExhausted);
        }
        hash.update(&bytes[..count]);
    }
    let after = file.metadata().map_err(|_| ErrorCode::StorageUnavailable)?;
    if stamp(&before) != stamp(&after) || total != before.len() {
        return Err(ErrorCode::RevisionConflict);
    }
    Ok((format!("{:x}", hash.finalize()), before))
}

pub fn replace(
    canonical_path: &Path,
    expected_revision: &str,
    bytes: &[u8],
    check: impl Fn() -> Result<(), ErrorCode>,
) -> Result<String, ReplaceError> {
    check()?;
    if bytes.len() as u64 > LIMIT {
        return Err(ErrorCode::ResourceExhausted.into());
    }
    let parent_path = canonical_path.parent().ok_or(ErrorCode::ScopeDenied)?;
    let name = CString::new(
        canonical_path
            .file_name()
            .ok_or(ErrorCode::ScopeDenied)?
            .as_bytes(),
    )
    .map_err(|_| ErrorCode::ScopeDenied)?;
    let parent = crate::project_files::open_absolute_directory(parent_path)?;
    let parent_identity = identity(
        &parent
            .metadata()
            .map_err(|_| ErrorCode::StorageUnavailable)?,
    );
    let mut original = open_file(&parent, &name)?;
    let (revision, metadata) = snapshot(&mut original, &check)?;
    if revision != expected_revision {
        return Err(ErrorCode::RevisionConflict.into());
    }
    if metadata.permissions().readonly() || metadata.mode() & 0o7000 != 0 {
        return Err(ErrorCode::ScopeDenied.into());
    }
    let temporary_name = CString::new(format!(
        ".lomi-{}",
        crate::broker::new_id().map_err(|_| ErrorCode::StorageUnavailable)?
    ))
    .map_err(|_| ErrorCode::StorageUnavailable)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            temporary_name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(ErrorCode::StorageUnavailable.into());
    }
    let mut temporary = Temporary {
        parent: &parent,
        name: temporary_name,
        file: unsafe { File::from_raw_fd(fd) },
        linked: true,
    };
    for chunk in bytes.chunks(65536) {
        check()?;
        temporary
            .file
            .write_all(chunk)
            .map_err(|_| ErrorCode::StorageUnavailable)?;
    }
    let created = temporary
        .file
        .metadata()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    if (created.uid(), created.gid()) != (metadata.uid(), metadata.gid()) {
        let result =
            unsafe { libc::fchown(temporary.file.as_raw_fd(), metadata.uid(), metadata.gid()) };
        if result != 0 {
            return Err(ErrorCode::StorageUnavailable.into());
        }
    }
    temporary
        .file
        .set_permissions(Permissions::from_mode(metadata.mode() & 0o777))
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    temporary
        .file
        .sync_all()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    check()?;
    let current_parent = crate::project_files::open_absolute_directory(parent_path)?;
    if identity(
        &current_parent
            .metadata()
            .map_err(|_| ErrorCode::StorageUnavailable)?,
    ) != parent_identity
    {
        return Err(ErrorCode::RevisionConflict.into());
    }
    let mut current = open_file(&parent, &name)?;
    let (current_revision, current_metadata) = snapshot(&mut current, &check)?;
    if current_revision != expected_revision || stamp(&metadata) != stamp(&current_metadata) {
        return Err(ErrorCode::RevisionConflict.into());
    }
    let staged = open_file(&parent, &temporary.name)?
        .metadata()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let held = temporary
        .file
        .metadata()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    if !staged.is_file() || staged.nlink() != 1 || stamp(&staged) != stamp(&held) {
        return Err(ErrorCode::RevisionConflict.into());
    }
    // Revocation/operation expiry is checked immediately before the sole target mutation.
    check()?;
    let renamed = unsafe {
        libc::renameat(
            parent.as_raw_fd(),
            temporary.name.as_ptr(),
            parent.as_raw_fd(),
            name.as_ptr(),
        )
    };
    if renamed != 0 {
        return Err(ErrorCode::StorageUnavailable.into());
    }
    temporary.linked = false;
    parent.sync_all().map_err(|_| ReplaceError::Uncertain)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

/// Publish a complete new private file without ever replacing an existing entry.
/// The temporary sibling and final link use the same pinned parent descriptor.
pub fn create(
    canonical_path: &Path,
    bytes: &[u8],
    check: impl Fn() -> Result<(), ErrorCode>,
) -> Result<String, ReplaceError> {
    check()?;
    if bytes.len() as u64 > LIMIT {
        return Err(ErrorCode::ResourceExhausted.into());
    }
    let parent_path = canonical_path.parent().ok_or(ErrorCode::ScopeDenied)?;
    let name = CString::new(
        canonical_path
            .file_name()
            .ok_or(ErrorCode::ScopeDenied)?
            .as_bytes(),
    )
    .map_err(|_| ErrorCode::ScopeDenied)?;
    let parent = crate::project_files::open_absolute_directory(parent_path)?;
    let original_parent = identity(
        &parent
            .metadata()
            .map_err(|_| ErrorCode::StorageUnavailable)?,
    );
    let temporary_name = CString::new(format!(
        ".lomi-{}",
        crate::broker::new_id().map_err(|_| ErrorCode::StorageUnavailable)?
    ))
    .map_err(|_| ErrorCode::StorageUnavailable)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            temporary_name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(ErrorCode::StorageUnavailable.into());
    }
    let mut temporary = Temporary {
        parent: &parent,
        name: temporary_name,
        file: unsafe { File::from_raw_fd(fd) },
        linked: true,
    };
    for chunk in bytes.chunks(65536) {
        check()?;
        temporary
            .file
            .write_all(chunk)
            .map_err(|_| ErrorCode::StorageUnavailable)?;
    }
    temporary
        .file
        .sync_all()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    check()?;
    let current_parent = crate::project_files::open_absolute_directory(parent_path)?;
    if identity(
        &current_parent
            .metadata()
            .map_err(|_| ErrorCode::StorageUnavailable)?,
    ) != original_parent
    {
        return Err(ErrorCode::RevisionConflict.into());
    }
    let held = temporary
        .file
        .metadata()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let staged = open_file(&parent, &temporary.name)?
        .metadata()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    if !staged.is_file() || staged.nlink() != 1 || stamp(&held) != stamp(&staged) {
        return Err(ErrorCode::RevisionConflict.into());
    }
    check()?;
    // linkat without replacement is the publication boundary. Readers see all
    // bytes at once, and a concurrently created file or symlink wins unchanged.
    if unsafe {
        libc::linkat(
            parent.as_raw_fd(),
            temporary.name.as_ptr(),
            parent.as_raw_fd(),
            name.as_ptr(),
            0,
        )
    } != 0
    {
        return Err(
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::AlreadyExists {
                ErrorCode::RevisionConflict
            } else {
                ErrorCode::StorageUnavailable
            }
            .into(),
        );
    }
    if unsafe { libc::unlinkat(parent.as_raw_fd(), temporary.name.as_ptr(), 0) } != 0 {
        return Err(ReplaceError::Uncertain);
    }
    temporary.linked = false;
    parent.sync_all().map_err(|_| ReplaceError::Uncertain)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, fs};
    fn hash(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }
    #[test]
    fn new_file_publication_preserves_existing_entries_and_cleans_cancelled_staging() {
        for case in ["success", "existing", "symlink", "cancel", "parent"] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            fs::create_dir(root.join("settings")).unwrap();
            let path = root.join("settings/preferences.json");
            let outside = root.join("outside");
            fs::create_dir(&outside).unwrap();
            fs::write(outside.join("private"), b"keep").unwrap();
            if case == "existing" {
                fs::write(&path, b"user settings").unwrap();
            }
            if case == "symlink" {
                std::os::unix::fs::symlink(outside.join("private"), &path).unwrap();
            }
            let calls = Cell::new(0);
            let result = create(&path, b"complete settings", || {
                calls.set(calls.get() + 1);
                if calls.get() == 3 {
                    if case == "cancel" {
                        return Err(ErrorCode::ControlRevoked);
                    }
                    if case == "parent" {
                        fs::rename(root.join("settings"), root.join("old")).unwrap();
                        std::os::unix::fs::symlink(&outside, root.join("settings")).unwrap();
                    }
                }
                Ok(())
            });
            if case == "success" {
                assert_eq!(result.unwrap(), hash(b"complete settings"));
                assert_eq!(fs::read(&path).unwrap(), b"complete settings");
                let meta = fs::metadata(&path).unwrap();
                assert_eq!(meta.nlink(), 1);
                assert_eq!(meta.mode() & 0o777, 0o600);
            } else {
                assert!(
                    matches!(result, Err(ReplaceError::Before(_))),
                    "{case}: {result:?}"
                );
            }
            if case == "existing" {
                assert_eq!(fs::read(&path).unwrap(), b"user settings");
            }
            if case == "symlink" {
                assert!(path.is_symlink());
            }
            assert_eq!(fs::read(outside.join("private")).unwrap(), b"keep");
            assert!(!outside.join("preferences.json").exists());
            let retained = root.join(if case == "parent" { "old" } else { "settings" });
            assert!(fs::read_dir(retained).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".lomi-")));
        }
    }
    #[test]
    fn replacement_is_atomic_revision_checked_and_preserves_executable_mode() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let path = root.join("script.sh");
        fs::write(&path, "old").unwrap();
        fs::set_permissions(&path, Permissions::from_mode(0o750)).unwrap();
        let old_inode = fs::metadata(&path).unwrap().ino();
        let data = "Zażółć 🙂\r\n".as_bytes();
        assert_eq!(
            replace(&path, &hash(b"old"), data, || Ok(())).unwrap(),
            hash(data)
        );
        let saved = fs::metadata(&path).unwrap();
        assert_ne!(saved.ino(), old_inode);
        assert_eq!(saved.mode() & 0o777, 0o750);
        assert!(matches!(
            replace(&path, &hash(b"old"), b"overwrite", || Ok(())),
            Err(ReplaceError::Before(ErrorCode::RevisionConflict))
        ));
        assert_eq!(fs::read(&path).unwrap(), data);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    }
    #[test]
    fn cancellation_and_replaced_parent_never_follow_an_outside_link() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::create_dir(root.join("project")).unwrap();
        fs::create_dir(root.join("outside")).unwrap();
        let path = root.join("project/file.txt");
        fs::write(&path, b"old").unwrap();
        fs::write(root.join("outside/file.txt"), b"private").unwrap();
        let calls = Cell::new(0);
        let result = replace(&path, &hash(b"old"), b"replacement", || {
            calls.set(calls.get() + 1);
            if calls.get() == 4 {
                return Err(ErrorCode::ControlRevoked);
            }
            Ok(())
        });
        assert!(matches!(
            result,
            Err(ReplaceError::Before(ErrorCode::ControlRevoked))
        ));
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(fs::read_dir(root.join("project")).unwrap().count(), 1);
        let calls = Cell::new(0);
        let result = replace(&path, &hash(b"old"), b"replacement", || {
            calls.set(calls.get() + 1);
            if calls.get() == 4 {
                fs::rename(root.join("project"), root.join("moved")).unwrap();
                std::os::unix::fs::symlink(root.join("outside"), root.join("project")).unwrap();
            }
            Ok(())
        });
        assert!(matches!(result, Err(ReplaceError::Before(_))));
        assert_eq!(fs::read(root.join("outside/file.txt")).unwrap(), b"private");
        assert_eq!(fs::read(root.join("moved/file.txt")).unwrap(), b"old");
        assert_eq!(fs::read_dir(root.join("moved")).unwrap().count(), 1);
    }

    #[test]
    fn concurrent_disk_edit_and_final_symlink_are_not_overwritten() {
        for link in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let path = root.join("file.txt");
            let outside = root.join("outside.txt");
            fs::write(&path, b"old").unwrap();
            fs::write(&outside, b"private").unwrap();
            let calls = Cell::new(0);
            let result = replace(&path, &hash(b"old"), b"replacement", || {
                calls.set(calls.get() + 1);
                if calls.get() == 4 {
                    if link {
                        fs::remove_file(&path).unwrap();
                        std::os::unix::fs::symlink(&outside, &path).unwrap();
                    } else {
                        fs::write(&path, b"external work").unwrap();
                    }
                }
                Ok(())
            });
            assert!(matches!(
                result,
                Err(ReplaceError::Before(ErrorCode::RevisionConflict))
            ));
            assert_eq!(fs::read(&outside).unwrap(), b"private");
            if !link {
                assert_eq!(fs::read(&path).unwrap(), b"external work");
            }
            assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        }
    }
}
