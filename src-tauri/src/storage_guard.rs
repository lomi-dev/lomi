use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[cfg(windows)]
mod windows_permissions;

pub struct Owner {
    pub root: PathBuf,
    lock: fs::File,
}
impl Owner {
    pub fn acquire(root: PathBuf) -> Result<Self, String> {
        reject_link(&root)?;
        fs::create_dir_all(&root).map_err(|_| "Cannot create the data directory.")?;
        private(&root, true)?;
        let root = fs::canonicalize(root).map_err(|_| "Cannot resolve storage.")?;
        let path = root.join("owner.lock");
        reject_link(&path)?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|_| "Cannot open the owner lock.")?;
        private(&path, false)?;
        lock.try_lock()
            .map_err(|_| "Another Lomi instance owns this data. Close it and retry.")?;
        Ok(Self { root, lock })
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        let _ = self.lock.unlock();
    }
}

pub fn reject_link(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    if let Ok(meta) = path.symlink_metadata() {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err("Chat storage cannot use reparse points.".into());
        }
    }
    match path.symlink_metadata() {
        Ok(meta) if meta.file_type().is_symlink() => {
            Err("Chat storage cannot use symbolic links.".into())
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err("Cannot inspect storage.".into())
        }
        _ => Ok(()),
    }
}
pub fn private(path: &Path, directory: bool) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(if directory { 0o700 } else { 0o600 }),
        )
        .map_err(|_| "Cannot secure storage.")?;
    }
    #[cfg(windows)]
    windows_permissions::private(path, directory)?;
    Ok(())
}
pub fn atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    reject_link(path)?;
    let parent = path.parent().ok_or("Invalid storage path.")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| "Cannot create storage transaction.")?;
    private(temporary.path(), false)?;
    temporary
        .write_all(data)
        .map_err(|_| "Cannot write storage.")?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| "Cannot flush storage.")?;
    temporary
        .persist(path)
        .map_err(|_| "Cannot publish storage.")?;
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|_| "Cannot flush the directory.")?;
    Ok(())
}
