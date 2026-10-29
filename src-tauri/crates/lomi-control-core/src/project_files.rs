//! Compatibility facade over the protocol-neutral native project-files engine.
use lomi_control_protocol::{ErrorCode, files::FileEntry};
use lomi_native_project_files::{
    NativeProjectDirectory, NativeProjectDirectorySnapshot, NativeProjectEntryKind,
    NativeProjectErrorCode, NativeProjectFile,
};
use std::{fs::File, path::Path};

pub struct ProjectDirectory {
    engine: NativeProjectDirectory,
}

pub struct ProjectFile {
    pub file: File,
    selected: NativeProjectFile,
}

pub struct DirectorySnapshot {
    pub revision: String,
    pub entries: Vec<FileEntry>,
}

pub(crate) fn to_protocol_error(value: NativeProjectErrorCode) -> ErrorCode {
    match value {
        NativeProjectErrorCode::AppUnavailable => ErrorCode::AppUnavailable,
        NativeProjectErrorCode::PairingRequired => ErrorCode::PairingRequired,
        NativeProjectErrorCode::ScopeDenied => ErrorCode::ScopeDenied,
        NativeProjectErrorCode::UiNotReady => ErrorCode::UiNotReady,
        NativeProjectErrorCode::AppClosing => ErrorCode::AppClosing,
        NativeProjectErrorCode::TargetNotFound => ErrorCode::TargetNotFound,
        NativeProjectErrorCode::StaleGeneration => ErrorCode::StaleGeneration,
        NativeProjectErrorCode::RevisionConflict => ErrorCode::RevisionConflict,
        NativeProjectErrorCode::StaleSnapshot => ErrorCode::StaleSnapshot,
        NativeProjectErrorCode::TargetBusy => ErrorCode::TargetBusy,
        NativeProjectErrorCode::ControlRevoked => ErrorCode::ControlRevoked,
        NativeProjectErrorCode::PromptStateUnknown => ErrorCode::PromptStateUnknown,
        NativeProjectErrorCode::PanelNotRenderable => ErrorCode::PanelNotRenderable,
        NativeProjectErrorCode::ProtectedOriginTerminal => ErrorCode::ProtectedOriginTerminal,
        NativeProjectErrorCode::UnsupportedCapability => ErrorCode::UnsupportedCapability,
        NativeProjectErrorCode::HostUnqualified => ErrorCode::HostUnqualified,
        NativeProjectErrorCode::DeadlineExceeded => ErrorCode::DeadlineExceeded,
        NativeProjectErrorCode::CursorExpired => ErrorCode::CursorExpired,
        NativeProjectErrorCode::ArtifactTooLarge => ErrorCode::ArtifactTooLarge,
        NativeProjectErrorCode::ArtifactInvalid => ErrorCode::ArtifactInvalid,
        NativeProjectErrorCode::OutcomeUnknown => ErrorCode::OutcomeUnknown,
        NativeProjectErrorCode::IdempotencyConflict => ErrorCode::IdempotencyConflict,
        NativeProjectErrorCode::RetryWindowExpired => ErrorCode::RetryWindowExpired,
        NativeProjectErrorCode::ResourceExhausted => ErrorCode::ResourceExhausted,
        NativeProjectErrorCode::StorageUnavailable => ErrorCode::StorageUnavailable,
    }
}

pub(crate) fn to_native_error(value: ErrorCode) -> NativeProjectErrorCode {
    match value {
        ErrorCode::AppUnavailable => NativeProjectErrorCode::AppUnavailable,
        ErrorCode::PairingRequired => NativeProjectErrorCode::PairingRequired,
        ErrorCode::ScopeDenied => NativeProjectErrorCode::ScopeDenied,
        ErrorCode::UiNotReady => NativeProjectErrorCode::UiNotReady,
        ErrorCode::AppClosing => NativeProjectErrorCode::AppClosing,
        ErrorCode::TargetNotFound => NativeProjectErrorCode::TargetNotFound,
        ErrorCode::StaleGeneration => NativeProjectErrorCode::StaleGeneration,
        ErrorCode::RevisionConflict => NativeProjectErrorCode::RevisionConflict,
        ErrorCode::StaleSnapshot => NativeProjectErrorCode::StaleSnapshot,
        ErrorCode::TargetBusy => NativeProjectErrorCode::TargetBusy,
        ErrorCode::ControlRevoked => NativeProjectErrorCode::ControlRevoked,
        ErrorCode::PromptStateUnknown => NativeProjectErrorCode::PromptStateUnknown,
        ErrorCode::PanelNotRenderable => NativeProjectErrorCode::PanelNotRenderable,
        ErrorCode::ProtectedOriginTerminal => NativeProjectErrorCode::ProtectedOriginTerminal,
        ErrorCode::UnsupportedCapability => NativeProjectErrorCode::UnsupportedCapability,
        ErrorCode::HostUnqualified => NativeProjectErrorCode::HostUnqualified,
        ErrorCode::DeadlineExceeded => NativeProjectErrorCode::DeadlineExceeded,
        ErrorCode::CursorExpired => NativeProjectErrorCode::CursorExpired,
        ErrorCode::ArtifactTooLarge => NativeProjectErrorCode::ArtifactTooLarge,
        ErrorCode::ArtifactInvalid => NativeProjectErrorCode::ArtifactInvalid,
        ErrorCode::OutcomeUnknown => NativeProjectErrorCode::OutcomeUnknown,
        ErrorCode::IdempotencyConflict => NativeProjectErrorCode::IdempotencyConflict,
        ErrorCode::RetryWindowExpired => NativeProjectErrorCode::RetryWindowExpired,
        ErrorCode::ResourceExhausted => NativeProjectErrorCode::ResourceExhausted,
        ErrorCode::StorageUnavailable => NativeProjectErrorCode::StorageUnavailable,
    }
}

impl ProjectDirectory {
    pub(crate) fn canonical_path(&self) -> &Path {
        self.engine.canonical_path()
    }

    pub(crate) fn native(&self) -> &NativeProjectDirectory {
        &self.engine
    }

    pub(crate) fn open_directory(&self, relative: &str) -> Result<File, ErrorCode> {
        self.engine
            .open_directory(relative)
            .map_err(to_protocol_error)
    }

    pub fn list(
        &self,
        relative: &str,
        check: impl Fn() -> Result<(), ErrorCode>,
    ) -> Result<DirectorySnapshot, ErrorCode> {
        let snapshot = self
            .engine
            .list(relative, || check().map_err(to_native_error))
            .map_err(to_protocol_error)?;
        Ok(DirectorySnapshot::from(snapshot))
    }

    pub fn open(path: &Path) -> Result<Self, ErrorCode> {
        Ok(Self {
            engine: NativeProjectDirectory::open(path).map_err(to_protocol_error)?,
        })
    }

    pub fn check(&self) -> Result<(), ErrorCode> {
        self.engine.check().map_err(to_protocol_error)
    }

    pub fn open_file(&self, relative: &str, max_bytes: u64) -> Result<ProjectFile, ErrorCode> {
        let selected = self
            .engine
            .open_file(relative, max_bytes)
            .map_err(to_protocol_error)?;
        let file = selected.try_clone_file().map_err(to_protocol_error)?;
        Ok(ProjectFile { file, selected })
    }
}

impl From<NativeProjectDirectorySnapshot> for DirectorySnapshot {
    fn from(snapshot: NativeProjectDirectorySnapshot) -> Self {
        Self {
            revision: snapshot.revision,
            entries: snapshot
                .entries
                .into_iter()
                .map(|entry| FileEntry {
                    name: entry.name,
                    relative_path: entry.relative_path,
                    kind: match entry.kind {
                        NativeProjectEntryKind::Directory => {
                            lomi_control_protocol::files::FileEntryKind::Directory
                        }
                        NativeProjectEntryKind::File => {
                            lomi_control_protocol::files::FileEntryKind::File
                        }
                    },
                    byte_length: entry.byte_length,
                })
                .collect(),
        }
    }
}

impl ProjectFile {
    pub fn read_bytes(
        self,
        limit: u64,
        check: impl Fn() -> Result<(), ErrorCode>,
    ) -> Result<Vec<u8>, ErrorCode> {
        self.selected
            .read_bytes(limit, || check().map_err(to_native_error))
            .map_err(to_protocol_error)
    }

    pub fn len(&self) -> u64 {
        self.selected.len()
    }

    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    pub fn check_unchanged(&self) -> Result<(), ErrorCode> {
        self.selected.check_unchanged().map_err(to_protocol_error)
    }
}

pub(crate) fn open_absolute_directory(path: &Path) -> Result<File, ErrorCode> {
    lomi_native_project_files::open_absolute_directory(path).map_err(to_protocol_error)
}

pub fn validate_relative(path: &str) -> Result<(), ErrorCode> {
    lomi_native_project_files::validate_relative(path).map_err(to_protocol_error)
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
        let previous_links = fs::metadata(&root).unwrap().nlink();
        let calls = Cell::new(0);
        let changed = project.list("", || {
            calls.set(calls.get() + 1);
            if calls.get() == 3 {
                fs::create_dir(root.join("changed-during-list")).unwrap();
            }
            Ok(())
        });
        assert_ne!(fs::metadata(&root).unwrap().nlink(), previous_links);
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
