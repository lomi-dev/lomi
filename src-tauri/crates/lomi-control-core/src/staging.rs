//! Bounded copy into an owned directory. The finished copy retains only a read
//! descriptor; its hash covers the copy, not a pathname reopened by a consumer.
use crate::project_files::ProjectFile;
use lomi_control_protocol::ErrorCode;
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::MetadataExt,
    },
};

pub const MAX_IMPORT_BYTES: u64 = 512 * 1024 * 1024;
pub struct StagedCopy {
    parent: File,
    name: CString,
    reservation_id: String,
    pub file: File,
    pub sha256: String,
    pub byte_length: u64,
}
struct Temporary {
    parent: File,
    name: CString,
    file: File,
}
impl Drop for Temporary {
    fn drop(&mut self) {
        unsafe {
            libc::unlinkat(self.parent.as_raw_fd(), self.name.as_ptr(), 0);
        }
    }
}
impl Drop for StagedCopy {
    fn drop(&mut self) {
        unsafe {
            libc::unlinkat(self.parent.as_raw_fd(), self.name.as_ptr(), 0);
        }
    }
}
fn storage(_: impl std::fmt::Debug) -> ErrorCode {
    ErrorCode::StorageUnavailable
}
fn same_file(a: &File, b: &File) -> Result<bool, ErrorCode> {
    let a = a.metadata().map_err(storage)?;
    let b = b.metadata().map_err(storage)?;
    Ok(a.is_file() && b.is_file() && (a.dev(), a.ino(), a.len()) == (b.dev(), b.ino(), b.len()))
}
impl StagedCopy {
    pub fn reservation_id(&self) -> &str {
        &self.reservation_id
    }
    /// The caller reserves expected_bytes in the shared durable budget first.
    pub fn copy(
        mut source: ProjectFile,
        parent: File,
        reservation_id: &str,
        expected_bytes: u64,
        expected_hash: &str,
        check: impl Fn() -> Result<(), ErrorCode>,
    ) -> Result<Self, ErrorCode> {
        if reservation_id.len() != 32
            || !reservation_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(ErrorCode::ScopeDenied);
        }
        if expected_bytes > MAX_IMPORT_BYTES
            || expected_hash.len() != 64
            || !expected_hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(ErrorCode::ResourceExhausted);
        }
        if source.len() != expected_bytes {
            return Err(ErrorCode::RevisionConflict);
        }
        check()?;
        let meta = parent.metadata().map_err(storage)?;
        if !meta.is_dir()
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o777 != 0o700
        {
            return Err(ErrorCode::StorageUnavailable);
        }
        let name = CString::new(format!("{reservation_id}.stage")).map_err(storage)?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(ErrorCode::StorageUnavailable);
        }
        let mut stage = Temporary {
            parent,
            name,
            file: unsafe { File::from_raw_fd(fd) },
        };
        let mut hash = Sha256::new();
        let mut remaining = expected_bytes;
        let mut buffer = [0; 65536];
        while remaining > 0 {
            check()?;
            let length = remaining.min(buffer.len() as u64) as usize;
            let count = source.file.read(&mut buffer[..length]).map_err(storage)?;
            if count == 0 {
                return Err(ErrorCode::RevisionConflict);
            }
            stage.file.write_all(&buffer[..count]).map_err(storage)?;
            hash.update(&buffer[..count]);
            remaining -= count as u64;
        }
        let sha256 = format!("{:x}", hash.finalize());
        if source.file.read(&mut buffer[..1]).map_err(storage)? != 0 || sha256 != expected_hash {
            return Err(ErrorCode::RevisionConflict);
        }
        source.check_unchanged()?;
        check()?;
        stage.file.sync_all().map_err(storage)?;
        let fd = unsafe {
            libc::openat(
                stage.parent.as_raw_fd(),
                stage.name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            return Err(ErrorCode::StorageUnavailable);
        }
        let file = unsafe { File::from_raw_fd(fd) };
        if !same_file(&stage.file, &file)? || file.metadata().map_err(storage)?.nlink() != 1 {
            return Err(ErrorCode::StorageUnavailable);
        }
        // A new private name owns the read-only copy. Temporary's Drop removes
        // the original name and closes the sole writer before returning it.
        let name = CString::new(format!("{reservation_id}.ready")).map_err(storage)?;
        let parent = stage.parent.try_clone().map_err(storage)?;
        if unsafe {
            libc::linkat(
                stage.parent.as_raw_fd(),
                stage.name.as_ptr(),
                stage.parent.as_raw_fd(),
                name.as_ptr(),
                0,
            )
        } != 0
        {
            return Err(ErrorCode::StorageUnavailable);
        }
        let result = Self {
            parent,
            name,
            reservation_id: reservation_id.into(),
            file,
            sha256,
            byte_length: expected_bytes,
        };
        drop(stage);
        result.parent.sync_all().map_err(storage)?;
        check()?;
        Ok(result)
    }
    /// Link only a verified copy to a broker-generated destination in this same
    /// directory. The caller commits its durable metadata afterward.
    pub fn publish(&self, extension: &str) -> Result<(), ErrorCode> {
        if !matches!(extension, "apk" | "bin") {
            return Err(ErrorCode::ScopeDenied);
        }
        let destination =
            CString::new(format!("{}.{extension}", self.reservation_id)).map_err(storage)?;
        if unsafe {
            libc::linkat(
                self.parent.as_raw_fd(),
                self.name.as_ptr(),
                self.parent.as_raw_fd(),
                destination.as_ptr(),
                0,
            )
        } != 0
        {
            return Err(ErrorCode::StorageUnavailable);
        }
        let fd = unsafe {
            libc::openat(
                self.parent.as_raw_fd(),
                destination.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        let verified = (|| {
            if fd < 0 || !same_file(&self.file, &unsafe { File::from_raw_fd(fd) })? {
                return Err(ErrorCode::StorageUnavailable);
            }
            self.parent.sync_all().map_err(storage)
        })();
        if verified.is_err() {
            unsafe {
                libc::unlinkat(self.parent.as_raw_fd(), destination.as_ptr(), 0);
            }
        }
        verified
    }
}
