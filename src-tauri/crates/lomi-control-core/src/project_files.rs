//! Compatibility facade over the protocol-neutral native project-files engine.
use lomi_control_protocol::{files::FileEntry, ErrorCode};
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
