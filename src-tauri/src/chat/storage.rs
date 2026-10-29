use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub struct Owner {
    pub root: PathBuf,
    lock: fs::File,
}
impl Owner {
    pub fn acquire(root: PathBuf) -> Result<Self, String> {
        reject_link(&root)?;
        fs::create_dir_all(&root).map_err(|_| "Cannot create the chat data directory.")?;
        private(&root, true)?;
        let root = fs::canonicalize(root).map_err(|_| "Cannot resolve chat storage.")?;
        let path = root.join("owner.lock");
        reject_link(&path)?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|_| "Cannot open the chat owner lock.")?;
        private(&path, false)?;
        lock.try_lock()
            .map_err(|_| "Another Lomi instance owns Chat AI. Close it and retry.")?;
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
            Err("Cannot inspect chat storage.".into())
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
        .map_err(|_| "Cannot secure chat storage.")?;
    }
    #[cfg(windows)]
    super::windows_permissions::private(path, directory)?;
    Ok(())
}
pub fn atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    reject_link(path)?;
    let parent = path.parent().ok_or("Invalid chat storage path.")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| "Cannot create chat storage transaction.")?;
    private(temporary.path(), false)?;
    temporary
        .write_all(data)
        .map_err(|_| "Cannot write chat storage.")?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| "Cannot flush chat storage.")?;
    temporary
        .persist(path)
        .map_err(|_| "Cannot publish chat storage.")?;
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|_| "Cannot flush the chat directory.")?;
    Ok(())
}

pub fn backup_files(parent: &Path, names: &[&str]) -> Result<PathBuf, String> {
    let destination = parent.join(format!(
        "chat-recovery-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir(&destination).map_err(|_| "Cannot create the recovery backup.")?;
    private(&destination, true)?;
    for name in names {
        reject_link(&parent.join(name))?;
    }
    let mut moved = Vec::new();
    for name in names {
        let source = parent.join(name);
        if !source.exists() {
            continue;
        }
        if fs::rename(&source, destination.join(name)).is_err() {
            for name in moved.iter().rev() {
                let _ = fs::rename(destination.join(name), parent.join(name));
            }
            return Err("Recovery could not move all files. Existing files and the recovery backup were preserved.".into());
        }
        moved.push(*name);
    }
    #[cfg(unix)]
    for folder in [parent, destination.as_path()] {
        fs::File::open(folder)
            .and_then(|f| f.sync_all())
            .map_err(|_| "Cannot flush the recovery backup.")?;
    }
    Ok(destination)
}
