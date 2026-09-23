//! Descriptor-relative project reads. No pathname is reopened after selection.
//! This is an application boundary, not an OS sandbox for the host account.
use lomi_control_protocol::ErrorCode;
use std::{
    ffi::CString,
    fs::{File, Metadata},
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path, PathBuf},
};

pub struct ProjectDirectory {
    path: PathBuf,
    directory: File,
}
pub struct ProjectFile {
    pub file: File,
    stamp: FileStamp,
}
pub struct DirectorySnapshot {
    pub revision: String,
    pub entries: Vec<lomi_control_protocol::files::FileEntry>,
}
#[derive(PartialEq, Eq)]
struct FileStamp {
    device: u64,
    inode: u64,
    bytes: u64,
    links: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl FileStamp {
    fn read(m: &Metadata) -> Self {
        Self {
            device: m.dev(),
            inode: m.ino(),
            bytes: m.len(),
            links: m.nlink(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
        }
    }
}
fn open_at(
    directory: &File,
    name: &std::ffi::OsStr,
    directory_only: bool,
) -> Result<File, ErrorCode> {
    let name = CString::new(name.as_bytes()).map_err(|_| ErrorCode::ScopeDenied)?;
    let flags = libc::O_RDONLY
        | libc::O_NOFOLLOW
        | libc::O_CLOEXEC
        | libc::O_NONBLOCK
        | if directory_only { libc::O_DIRECTORY } else { 0 };
    let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err(ErrorCode::TargetNotFound);
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
pub(crate) fn open_absolute_directory(path: &Path) -> Result<File, ErrorCode> {
    if !path.is_absolute() || path.as_os_str().len() > 4096 {
        return Err(ErrorCode::ScopeDenied);
    }
    let mut directory = File::open("/").map_err(|_| ErrorCode::StorageUnavailable)?;
    for (index, component) in path.components().enumerate() {
        if index > 128 {
            return Err(ErrorCode::ResourceExhausted);
        }
        match component {
            Component::RootDir => {}
            Component::Normal(name) => directory = open_at(&directory, name, true)?,
            _ => return Err(ErrorCode::ScopeDenied),
        }
    }
    Ok(directory)
}
pub fn validate_relative(path: &str) -> Result<(), ErrorCode> {
    if path.is_empty()
        || path.len() > 4096
        || path.contains('\0')
        || path.contains('\\')
        || path.split('/').count() > 128
    {
        return Err(ErrorCode::ScopeDenied);
    }
    for part in path.split('/') {
        let lower = part.to_ascii_lowercase();
        if part.is_empty()
            || matches!(part, "." | "..")
            || lower.starts_with(".env")
            || matches!(
                lower.as_str(),
                ".git"
                    | ".ssh"
                    | ".aws"
                    | ".azure"
                    | ".gnupg"
                    | ".codex"
                    | ".netrc"
                    | ".npmrc"
                    | ".pypirc"
                    | "credentials"
                    | "credentials.json"
                    | "id_rsa"
                    | "id_ed25519"
            )
            || [".pem", ".key", ".p12", ".pfx", ".keystore", ".jks"]
                .iter()
                .any(|suffix| lower.ends_with(suffix))
        {
            return Err(ErrorCode::ScopeDenied);
        }
    }
    Ok(())
}
impl ProjectDirectory {
    pub(crate) fn canonical_path(&self) -> &Path {
        &self.path
    }
    pub(crate) fn open_directory(&self, relative: &str) -> Result<File, ErrorCode> {
        self.check()?;
        let mut directory = self
            .directory
            .try_clone()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        if !relative.is_empty() {
            validate_relative(relative)?;
            for name in relative.split('/') {
                directory = open_at(&directory, std::ffi::OsStr::new(name), true)?;
            }
        }
        Ok(directory)
    }
    pub fn list(
        &self,
        relative: &str,
        check: impl Fn() -> Result<(), ErrorCode>,
    ) -> Result<DirectorySnapshot, ErrorCode> {
        use lomi_control_protocol::files::{FileEntry, FileEntryKind};
        use rustix::fs::{statat, AtFlags, Dir, FileType};
        use sha2::{Digest, Sha256};
        check()?;
        let directory = self.open_directory(relative)?;
        let stamp = FileStamp::read(
            &directory
                .metadata()
                .map_err(|_| ErrorCode::StorageUnavailable)?,
        );
        let stream = Dir::read_from(&directory).map_err(|_| ErrorCode::StorageUnavailable)?;
        let mut entries = Vec::new();
        let mut bytes = 0;
        for (visited, entry) in stream.enumerate() {
            check()?;
            if visited >= 10002 {
                return Err(ErrorCode::ResourceExhausted);
            }
            let entry = entry.map_err(|_| ErrorCode::StorageUnavailable)?;
            let Ok(name) = entry.file_name().to_str() else {
                continue;
            };
            if matches!(name, "." | "..") {
                continue;
            }
            let path = if relative.is_empty() {
                name.to_owned()
            } else {
                format!("{relative}/{name}")
            };
            if validate_relative(&path).is_err() {
                continue;
            }
            let metadata = statat(&directory, entry.file_name(), AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|_| ErrorCode::RevisionConflict)?;
            let kind = match FileType::from_raw_mode(metadata.st_mode) {
                FileType::Directory => FileEntryKind::Directory,
                FileType::RegularFile if metadata.st_nlink == 1 && metadata.st_size >= 0 => {
                    FileEntryKind::File
                }
                _ => continue,
            };
            let item = FileEntry {
                name: name.into(),
                relative_path: path,
                byte_length: (kind == FileEntryKind::File).then(|| metadata.st_size.to_string()),
                kind,
            };
            bytes += serde_json::to_vec(&item)
                .map_err(|_| ErrorCode::ResourceExhausted)?
                .len();
            if bytes > 1024 * 1024 {
                return Err(ErrorCode::ResourceExhausted);
            }
            entries.push(item);
        }
        entries.sort_by(|a, b| {
            (a.kind != FileEntryKind::Directory)
                .cmp(&(b.kind != FileEntryKind::Directory))
                .then(a.name.cmp(&b.name))
        });
        if entries
            .windows(2)
            .any(|items| items[0].name == items[1].name)
        {
            return Err(ErrorCode::RevisionConflict);
        }
        check()?;
        let current = self.open_directory(relative)?;
        if FileStamp::read(
            &current
                .metadata()
                .map_err(|_| ErrorCode::StorageUnavailable)?,
        ) != stamp
            || FileStamp::read(
                &directory
                    .metadata()
                    .map_err(|_| ErrorCode::StorageUnavailable)?,
            ) != stamp
        {
            return Err(ErrorCode::RevisionConflict);
        }
        let mut hash = Sha256::new();
        hash.update(stamp.device.to_le_bytes());
        hash.update(stamp.inode.to_le_bytes());
        for value in [
            stamp.modified.0,
            stamp.modified.1,
            stamp.changed.0,
            stamp.changed.1,
        ] {
            hash.update(value.to_le_bytes());
        }
        hash.update(serde_json::to_vec(&entries).map_err(|_| ErrorCode::ResourceExhausted)?);
        check()?;
        Ok(DirectorySnapshot {
            entries,
            revision: format!("{:x}", hash.finalize()),
        })
    }
    pub fn open(path: &Path) -> Result<Self, ErrorCode> {
        let directory = open_absolute_directory(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            directory,
        })
    }
    pub fn check(&self) -> Result<(), ErrorCode> {
        let current = open_absolute_directory(&self.path)?
            .metadata()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        let approved = self
            .directory
            .metadata()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        if (current.dev(), current.ino()) != (approved.dev(), approved.ino()) {
            return Err(ErrorCode::RevisionConflict);
        }
        Ok(())
    }
    pub fn open_file(&self, relative: &str, max_bytes: u64) -> Result<ProjectFile, ErrorCode> {
        validate_relative(relative)?;
        self.check()?;
        let mut directory = self
            .directory
            .try_clone()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        let components: Vec<_> = Path::new(relative).components().collect();
        for (index, component) in components.iter().enumerate() {
            let Component::Normal(name) = component else {
                return Err(ErrorCode::ScopeDenied);
            };
            let last = index + 1 == components.len();
            let file = open_at(&directory, name, !last)?;
            if last {
                let metadata = file.metadata().map_err(|_| ErrorCode::StorageUnavailable)?;
                if !metadata.is_file() || metadata.nlink() != 1 {
                    return Err(ErrorCode::ScopeDenied);
                }
                if metadata.len() > max_bytes {
                    return Err(ErrorCode::ArtifactTooLarge);
                }
                return Ok(ProjectFile {
                    file,
                    stamp: FileStamp::read(&metadata),
                });
            }
            directory = file;
        }
        Err(ErrorCode::ScopeDenied)
    }
}
impl ProjectFile {
    /// Read the selected descriptor with bounded allocation and cooperative
    /// revocation. A changed file never produces a successful snapshot.
    pub fn read_bytes(
        mut self,
        limit: u64,
        check: impl Fn() -> Result<(), ErrorCode>,
    ) -> Result<Vec<u8>, ErrorCode> {
        check()?;
        if self.len() > limit || limit > 4 * 1024 * 1024 {
            return Err(ErrorCode::ResourceExhausted);
        }
        let mut bytes = Vec::with_capacity(self.len() as usize);
        let mut chunk = [0_u8; 65536];
        loop {
            check()?;
            let count = self
                .file
                .read(&mut chunk)
                .map_err(|_| ErrorCode::StorageUnavailable)?;
            if count == 0 {
                break;
            }
            if bytes.len() as u64 + count as u64 > limit {
                return Err(ErrorCode::ResourceExhausted);
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        self.check_unchanged()?;
        check()?;
        if bytes.len() as u64 != self.len() {
            return Err(ErrorCode::RevisionConflict);
        }
        Ok(bytes)
    }
    pub fn len(&self) -> u64 {
        self.stamp.bytes
    }
    pub fn is_empty(&self) -> bool {
        self.stamp.bytes == 0
    }
    pub fn check_unchanged(&self) -> Result<(), ErrorCode> {
        let metadata = self
            .file
            .metadata()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        if !metadata.is_file() || FileStamp::read(&metadata) != self.stamp {
            return Err(ErrorCode::RevisionConflict);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Read, os::unix::fs::symlink};
    #[test]
    fn directory_snapshot_omits_links_secrets_and_special_files_and_detects_changes() {
        use std::cell::Cell;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::create_dir(root.join("folder")).unwrap();
        fs::write(root.join("z.txt"), b"content").unwrap();
        fs::write(root.join("a.txt"), b"a").unwrap();
        fs::write(root.join(".env.production"), b"private").unwrap();
        fs::write(root.join("hidden-hard-link.txt"), b"private").unwrap();
        fs::hard_link(root.join("hidden-hard-link.txt"), root.join("alias.txt")).unwrap();
        symlink("folder", root.join("linked-folder")).unwrap();
        symlink("a.txt", root.join("linked-file")).unwrap();
        let fifo = CString::new(root.join("fifo").as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let project = ProjectDirectory::open(&root).unwrap();
        let first = project.list("", || Ok(())).unwrap();
        assert_eq!(
            first
                .entries
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["folder", "a.txt", "z.txt"]
        );
        assert_eq!(first.entries[0].byte_length, None);
        assert_eq!(first.entries[2].byte_length.as_deref(), Some("7"));
        assert_eq!(
            project.list("", || Ok(())).unwrap().revision,
            first.revision
        );
        assert!(project.list("linked-folder", || Ok(())).is_err());
        assert!(matches!(
            project.list(".env.production", || Ok(())),
            Err(ErrorCode::ScopeDenied)
        ));
        fs::write(root.join("new.txt"), b"new").unwrap();
        assert_ne!(
            project.list("", || Ok(())).unwrap().revision,
            first.revision
        );
        let calls = Cell::new(0);
        let changed = project.list("", || {
            calls.set(calls.get() + 1);
            if calls.get() == 3 {
                fs::write(root.join("changed-during-list.txt"), b"new").unwrap();
            }
            Ok(())
        });
        assert!(matches!(changed, Err(ErrorCode::RevisionConflict)));
        assert!(matches!(
            project.list("", || Err(ErrorCode::ControlRevoked)),
            Err(ErrorCode::ControlRevoked)
        ));
    }
    #[test]
    fn bounded_project_snapshot_checks_revocation_and_changes_during_read() {
        use std::cell::Cell;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let path = root.join("example.txt");
        let original = vec![b'a'; 150000];
        fs::write(&path, &original).unwrap();
        let project = ProjectDirectory::open(&root).unwrap();
        assert_eq!(
            project
                .open_file("example.txt", 200000)
                .unwrap()
                .read_bytes(200000, || Ok(()))
                .unwrap(),
            original
        );
        let calls = Cell::new(0);
        let cancelled = project
            .open_file("example.txt", 200000)
            .unwrap()
            .read_bytes(200000, || {
                calls.set(calls.get() + 1);
                if calls.get() >= 3 {
                    Err(ErrorCode::ControlRevoked)
                } else {
                    Ok(())
                }
            });
        assert!(matches!(cancelled, Err(ErrorCode::ControlRevoked)));
        let calls = Cell::new(0);
        let changed = project
            .open_file("example.txt", 200000)
            .unwrap()
            .read_bytes(200000, || {
                calls.set(calls.get() + 1);
                if calls.get() == 3 {
                    fs::write(&path, b"replaced during read").unwrap();
                }
                Ok(())
            });
        assert!(matches!(changed, Err(ErrorCode::RevisionConflict)));
        let file = project.open_file("example.txt", 200000).unwrap();
        assert!(matches!(
            file.read_bytes(2, || Ok(())),
            Err(ErrorCode::ResourceExhausted)
        ));
    }
    #[test]
    fn approved_project_descriptors_reject_links_secrets_devices_and_escape() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::create_dir(root.join("build")).unwrap();
        fs::write(root.join("build/test.apk"), b"APK").unwrap();
        let project = ProjectDirectory::open(&root).unwrap();
        for path in [
            "../outside.apk",
            "/tmp/other.apk",
            "build/../test.apk",
            "build//test.apk",
            ".ssh/file.apk",
            ".ENV.production/file.apk",
            "build/signing.jks",
            "build/key.pem",
        ] {
            assert!(project.open_file(path, 512).is_err(), "accepted {path}");
        }
        symlink("build", root.join("linked")).unwrap();
        assert!(project.open_file("linked/test.apk", 512).is_err());
        symlink("test.apk", root.join("build/link.apk")).unwrap();
        assert!(project.open_file("build/link.apk", 512).is_err());
        fs::hard_link(root.join("build/test.apk"), root.join("build/hard.apk")).unwrap();
        assert!(project.open_file("build/hard.apk", 512).is_err());
        fs::remove_file(root.join("build/hard.apk")).unwrap();
        assert!(matches!(
            project.open_file("build/test.apk", 2),
            Err(ErrorCode::ArtifactTooLarge)
        ));
        let name = CString::new(root.join("pipe.apk").as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(project.open_file("pipe.apk", 512).is_err());
        assert!(project.open_file("build", 512).is_err());
        let mut file = project.open_file("build/test.apk", 512).unwrap();
        let mut bytes = Vec::new();
        file.file.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"APK");
        file.check_unchanged().unwrap();
        fs::write(root.join("build/test.apk"), b"different bytes").unwrap();
        assert!(matches!(
            file.check_unchanged(),
            Err(ErrorCode::RevisionConflict)
        ));
    }
    #[test]
    fn path_replacement_cannot_redirect_an_open_file_or_project_root() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let root = base.join("project");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("app.apk"), b"selected").unwrap();
        fs::write(base.join("foreign.apk"), b"private").unwrap();
        let project = ProjectDirectory::open(&root).unwrap();
        let mut file = project.open_file("app.apk", 512).unwrap();
        fs::rename(root.join("app.apk"), root.join("original.apk")).unwrap();
        symlink(base.join("foreign.apk"), root.join("app.apk")).unwrap();
        let mut bytes = Vec::new();
        file.file.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"selected");
        assert!(project.open_file("app.apk", 512).is_err());
        fs::rename(&root, base.join("old-project")).unwrap();
        fs::create_dir(&root).unwrap();
        fs::write(root.join("app.apk"), b"replacement").unwrap();
        assert!(matches!(project.check(), Err(ErrorCode::RevisionConflict)));
        assert!(project.open_file("app.apk", 512).is_err());
    }
}
