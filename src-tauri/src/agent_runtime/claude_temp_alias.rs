//! Short, stable private spelling of the existing physical Claude temp directory.
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::{self, File},
    os::{
        fd::AsRawFd,
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        },
    },
    path::{Path, PathBuf},
};
fn refused() -> String {
    "The private Claude temporary alias could not be verified.".into()
}
fn private_directory(path: &Path) -> Result<File, String> {
    if path.canonicalize().map_err(|_| refused())? != path {
        return Err(refused());
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| refused())?;
    let metadata = file.metadata().map_err(|_| refused())?;
    let path_metadata = fs::symlink_metadata(path).map_err(|_| refused())?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o7777 != 0o700
        || !path_metadata.is_dir()
        || (metadata.dev(), metadata.ino()) != (path_metadata.dev(), path_metadata.ino())
    {
        return Err(refused());
    }
    Ok(file)
}
fn identity(file: &File) -> Result<(u64, u64), String> {
    let metadata = file.metadata().map_err(|_| refused())?;
    Ok((metadata.dev(), metadata.ino()))
}
fn link(directory: &File, target: &Path) -> Result<(u64, u64), String> {
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            c"t".as_ptr(),
            metadata.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(refused());
    }
    let metadata = unsafe { metadata.assume_init() };
    if metadata.st_mode & libc::S_IFMT != libc::S_IFLNK
        || metadata.st_uid != unsafe { libc::geteuid() }
        || metadata.st_nlink != 1
    {
        return Err(refused());
    }
    let expected = target.as_os_str().as_bytes();
    if expected.len() > 16384 {
        return Err(refused());
    }
    let mut bytes = vec![0; expected.len() + 1];
    let count = unsafe {
        libc::readlinkat(
            directory.as_raw_fd(),
            c"t".as_ptr(),
            bytes.as_mut_ptr().cast(),
            bytes.len(),
        )
    };
    if count < 0 || &bytes[..count as usize] != expected {
        return Err(refused());
    }
    Ok((metadata.st_dev as u64, metadata.st_ino))
}
pub(super) struct Alias {
    pub path: PathBuf,
    pub wrapper: PathBuf,
    storage: PathBuf,
    target: PathBuf,
    directory: File,
    physical: File,
    directory_identity: (u64, u64),
    physical_identity: (u64, u64),
    link_identity: (u64, u64),
}
impl Alias {
    pub(super) fn prepare(storage: &Path, target: &Path) -> Result<Self, String> {
        let uid = unsafe { libc::getuid() };
        if uid != unsafe { libc::geteuid() } {
            return Err(refused());
        }
        private_directory(storage)?;
        let physical = private_directory(target)?;
        let suffix = format!("/claude-{uid}");
        let hash_length = 44usize
            .checked_sub("/private/tmp/la/t".len() + suffix.len())
            .ok_or_else(refused)?
            .min(16);
        if hash_length < 14 {
            return Err(refused());
        }
        let mut digest = Sha256::new();
        for path in [storage, target] {
            let bytes = path.as_os_str().as_bytes();
            digest.update((bytes.len() as u64).to_le_bytes());
            digest.update(bytes);
        }
        digest.update(uid.to_le_bytes());
        let hash = format!("{:x}", digest.finalize());
        let wrapper = PathBuf::from(format!("/private/tmp/la{}", &hash[..hash_length]));
        match fs::DirBuilder::new().mode(0o700).create(&wrapper) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(refused()),
        }
        let directory = private_directory(&wrapper)?;
        let bytes = CString::new(target.as_os_str().as_bytes()).map_err(|_| refused())?;
        if unsafe { libc::symlinkat(bytes.as_ptr(), directory.as_raw_fd(), c"t".as_ptr()) } != 0
            && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
        {
            return Err(refused());
        }
        directory.sync_all().map_err(|_| refused())?;
        let alias = Self {
            path: wrapper.join("t"),
            storage: storage.into(),
            target: target.into(),
            directory_identity: identity(&directory)?,
            physical_identity: identity(&physical)?,
            link_identity: link(&directory, target)?,
            wrapper,
            directory,
            physical,
        };
        alias.recheck()?;
        Ok(alias)
    }
    pub(super) fn recheck(&self) -> Result<(), String> {
        if unsafe { libc::getuid() } != unsafe { libc::geteuid() } {
            return Err(refused());
        }
        private_directory(&self.storage)?;
        let observed = private_directory(&self.wrapper)?;
        let physical = private_directory(&self.target)?;
        if identity(&observed)? != self.directory_identity
            || identity(&self.directory)? != self.directory_identity
            || identity(&physical)? != self.physical_identity
            || identity(&self.physical)? != self.physical_identity
            || link(&self.directory, &self.target)? != self.link_identity
            || self.path.canonicalize().map_err(|_| refused())? != self.target
            || fs::read_dir(&self.wrapper)
                .map_err(|_| refused())?
                .any(|entry| entry.map(|entry| entry.file_name() != "t").unwrap_or(true))
        {
            return Err(refused());
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alias_is_short_stable_and_refuses_a_replaced_target() {
        let root = tempfile::tempdir().unwrap();
        let storage = root.path().canonicalize().unwrap();
        fs::set_permissions(
            &storage,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        let target = storage.join("long-owned-account-temporary-directory");
        fs::DirBuilder::new().mode(0o700).create(&target).unwrap();
        let alias = Alias::prepare(&storage, &target).unwrap();
        assert!(
            alias.path.as_os_str().len() + format!("/claude-{}", unsafe { libc::getuid() }).len()
                <= 44
        );
        let again = Alias::prepare(&storage, &target).unwrap();
        assert_eq!(again.path, alias.path);
        fs::remove_file(&alias.path).unwrap();
        std::os::unix::fs::symlink(&storage, &alias.path).unwrap();
        assert!(alias.recheck().is_err());
        assert!(Alias::prepare(&storage, &target).is_err());
        fs::remove_file(&alias.path).unwrap();
        fs::remove_dir(&alias.wrapper).unwrap();
    }
}
