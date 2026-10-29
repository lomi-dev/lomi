//! Atomic replacement at a pinned parent descriptor, shared by UI and MCP saves.
//! The caller authorizes the canonical target; this module never follows aliases.
use lomi_control_protocol::ErrorCode;
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::{File, Metadata, Permissions},
    io::Write,
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
    lomi_native_project_files::stamp_file(m)
}
fn identity(m: &Metadata) -> (u64, u64) {
    (m.dev(), m.ino())
}
pub(crate) fn open_file(parent: &File, name: &CString) -> Result<File, ErrorCode> {
    lomi_native_project_files::open_file_at(parent, name)
        .map_err(crate::project_files::to_protocol_error)
}
pub(crate) fn snapshot(
    file: &mut File,
    check: &impl Fn() -> Result<(), ErrorCode>,
) -> Result<(String, Metadata), ErrorCode> {
    lomi_native_project_files::snapshot_file(file, || {
        check().map_err(crate::project_files::to_native_error)
    })
    .map_err(crate::project_files::to_protocol_error)
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
