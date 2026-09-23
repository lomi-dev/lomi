//! Bounded preference-file snapshots. Callers hold their existing section mutex
//! during preparation and again during commit; no UI caller checks change here.
use lomi_control_core::{atomic_file, project_files::ProjectDirectory};
use lomi_control_protocol::ErrorCode;
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc};
use tauri::Manager;

pub(crate) struct PreferenceSource {
    pub bytes: Option<Vec<u8>>,
    pub revision: Option<String>,
    root: Arc<ProjectDirectory>,
    path: PathBuf,
}
impl PreferenceSource {
    pub fn open(
        app: &tauri::AppHandle,
        name: &'static str,
        limit: u64,
        check: &dyn Fn() -> Result<(), ErrorCode>,
    ) -> Result<Self, ErrorCode> {
        check()?;
        let directory = app
            .path()
            .app_data_dir()
            .map_err(|_| ErrorCode::StorageUnavailable)?
            .canonicalize()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        let root = Arc::new(ProjectDirectory::open(&directory)?);
        let path = directory.join(name);
        let bytes = match path.symlink_metadata() {
            Ok(_) => Some(root.open_file(name, limit)?.read_bytes(limit, check)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(ErrorCode::StorageUnavailable),
        };
        let revision = bytes.as_ref().map(|b| format!("{:x}", Sha256::digest(b)));
        Ok(Self {
            bytes,
            revision,
            root,
            path,
        })
    }
    pub fn commit(
        &self,
        bytes: &[u8],
        check: &dyn Fn() -> Result<(), ErrorCode>,
    ) -> Result<String, atomic_file::ReplaceError> {
        let allowed = || {
            check()?;
            self.root.check()
        };
        allowed()?;
        match &self.revision {
            Some(revision) => atomic_file::replace(&self.path, revision, bytes, allowed),
            None => atomic_file::create(&self.path, bytes, allowed),
        }
    }
}
