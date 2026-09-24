use super::{operations::UiMutation, *};
use crate::{atomic_file::ReplaceError, project_files::ProjectDirectory};
use std::{io::Read, os::unix::fs::MetadataExt};

const MAX_EXPORT_BYTES: u32 = 4 * 1024 * 1024;

impl Broker {
    /// Artifact IDs never replace their original source authority.
    pub(super) fn artifact_source_access(
        state: &State,
        owner: &str,
        artifact: &Artifact,
    ) -> Result<(), ErrorCode> {
        match &artifact.source {
            ArtifactSource::Project(source) => {
                if source.required_scope != "files.read" {
                    return Err(ErrorCode::ScopeDenied);
                }
                crate::project_files::validate_relative(&source.relative_path)?;
                Self::project_file_access(state, owner, &source.workspace_id)?.check()
            }
            ArtifactSource::Android(source) => Self::android_capture_access(state, owner, source)?
                .check_generation(&source.generation),
            ArtifactSource::Browser(source) => {
                if !matches!(
                    source.required_scope.as_str(),
                    "browser.capture_composite" | "browser.download"
                ) {
                    return Err(ErrorCode::ScopeDenied);
                }
                let target = Self::browser_target(
                    state,
                    owner,
                    &source.workspace_id,
                    &source.panel_id,
                    &source.browser_generation,
                    &source.required_scope,
                )?;
                let grant = &state.sessions[owner].grant;
                let origin_allowed = lomi_control_protocol::browser::Origin::parse(&source.origin)
                    .is_ok_and(|origin| grant.permits_browser_origin(&origin));
                if !target.control.authorized()
                    || grant.browser_profile.as_deref() != Some(source.profile_id.as_str())
                    || !origin_allowed
                {
                    return Err(ErrorCode::ControlRevoked);
                }
                Ok(())
            }
        }
    }

    pub(super) fn validate_artifact_export(
        state: &State,
        owner: &str,
        input: &ArtifactExportInput,
    ) -> Result<Arc<ProjectDirectory>, ErrorCode> {
        let directory = Self::project_file_access(state, owner, &input.workspace_id)?;
        if !["artifact.export", "files.mutate", "files.create"]
            .iter()
            .all(|scope| state.sessions[owner].grant.scopes.contains(*scope))
        {
            return Err(ErrorCode::ScopeDenied);
        }
        crate::project_files::validate_relative(&input.relative_path)?;
        Ok(directory)
    }

    pub(super) fn export_artifact(
        self: &Arc<Self>,
        owner: &str,
        input: ArtifactExportInput,
    ) -> Reply {
        let hash_valid = |s: &str| {
            s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        if !valid_id(&input.artifact_id)
            || !valid_id(&input.request_key)
            || input.expected_revision.parse::<u64>().is_err()
            || !hash_valid(&input.expected_sha256)
            || !hash_valid(&input.expected_parent_revision)
        {
            return error(ErrorCode::RevisionConflict);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        if let Err(code) = Self::validate_artifact_export(&state, owner, &input) {
            return error(code);
        }
        let session = &state.sessions[owner];
        if session.retry_epoch != input.retry_epoch {
            return error(ErrorCode::RetryWindowExpired);
        }
        let root = session.grant.workspace(&input.workspace_id).unwrap();
        let project = root.project_id.clone();
        let hash = match receipts::fingerprint(
            &(&input, &root.project_path),
            &receipts::Target {
                workspace_id: &input.workspace_id,
                resource_id: &input.artifact_id,
                generation: &project,
                revision: &input.expected_parent_revision,
            },
        ) {
            Ok(hash) => hash,
            Err(e) => return operations::storage_error(e),
        };
        let key = receipts::Key {
            pairing_id: owner,
            retry_epoch: &input.retry_epoch,
            project_id: &project,
            tool: "lomi_artifact_export",
            request_key: &input.request_key,
        };
        match self.replay(&state, owner, &key, hash) {
            Ok(Some(reply)) => return reply,
            Err(reply) => return *reply,
            Ok(None) => {}
        }
        let artifact = match self.store.lock().ok().and_then(|store| {
            store
                .artifact_metadata(owner, &project, &input.artifact_id, now())
                .ok()
        }) {
            Some(a) => a,
            None => return error(ErrorCode::TargetNotFound),
        };
        if artifact.source.workspace() != input.workspace_id {
            return error(ErrorCode::TargetNotFound);
        }
        if let Err(code) = Self::artifact_source_access(&state, owner, &artifact) {
            return error(code);
        }
        if artifact.sha256 != input.expected_sha256 {
            return error(ErrorCode::RevisionConflict);
        }
        if artifact.byte_length > MAX_EXPORT_BYTES {
            return error(ErrorCode::ArtifactTooLarge);
        }
        self.enqueue_ui(
            &mut state,
            owner,
            UiMutation {
                workspace: input.workspace_id.clone(),
                project,
                revision: input.expected_revision.clone(),
                retry_epoch: input.retry_epoch.clone(),
                request_key: input.request_key.clone(),
                tool: "lomi_artifact_export",
                hash,
                action: UiAction::ExportArtifact(ArtifactExportCommand {
                    workspace_id: input.workspace_id.clone(),
                    not_after_millis: ((now() + 30) * 1000).to_string(),
                    input,
                }),
            },
        )
    }

    /// Main calls this under the existing native file writer lock. No source
    /// path or bytes are accepted from the renderer.
    pub fn commit_artifact_export(
        &self,
        operation: &str,
        nonce: &str,
    ) -> Result<ArtifactExported, ErrorCode> {
        let (input, owner, path, directory, permit, policy, mut leased) = {
            let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let work = state.work.get(operation).ok_or(ErrorCode::ControlRevoked)?;
            if !work.claimed
                || work.native_committed
                || work.command.nonce != nonce
                || work.command.ui_epoch != state.projection.ui_epoch
            {
                return Err(ErrorCode::ControlRevoked);
            }
            work.native_permit.check()?;
            let UiAction::ExportArtifact(command) = &work.command.action else {
                return Err(ErrorCode::ScopeDenied);
            };
            let input = &command.input;
            let directory = Self::validate_artifact_export(&state, &work.pairing, input)?;
            let root = state.sessions[&work.pairing]
                .grant
                .workspace(&input.workspace_id)
                .unwrap();
            let leased = self
                .store
                .lock()
                .map_err(|_| ErrorCode::StorageUnavailable)?
                .lease_artifact(&work.pairing, &work.project, &input.artifact_id, now())
                .map_err(|_| ErrorCode::TargetNotFound)?;
            if leased.artifact.source.workspace() != input.workspace_id {
                return Err(ErrorCode::TargetNotFound);
            }
            Self::artifact_source_access(&state, &work.pairing, &leased.artifact)?;
            if leased.artifact.sha256 != input.expected_sha256 {
                return Err(ErrorCode::RevisionConflict);
            }
            if leased.artifact.byte_length > MAX_EXPORT_BYTES {
                return Err(ErrorCode::ArtifactTooLarge);
            }
            let tuple = (
                input.clone(),
                work.pairing.clone(),
                PathBuf::from(&root.project_path),
                directory,
                work.native_permit.clone(),
                state.policy_revision,
                leased,
            );
            state.work.get_mut(operation).unwrap().native_committed = true;
            tuple
        };
        let artifact = leased.artifact.clone();
        let check = || {
            permit.check()?;
            let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            if state.policy_revision != policy {
                return Err(ErrorCode::ControlRevoked);
            }
            let current = Self::validate_artifact_export(&state, &owner, &input)?;
            if !Arc::ptr_eq(&directory, &current) {
                return Err(ErrorCode::ControlRevoked);
            }
            Self::artifact_source_access(&state, &owner, &artifact)?;
            directory.check()
        };
        let write = || -> Result<ArtifactExported, ReplaceError> {
            let _producer = self
                .file_reads
                .clone()
                .try_acquire_owned()
                .map_err(|_| ErrorCode::ResourceExhausted)?;
            leased.verify(check)?;
            let mut bytes = Vec::with_capacity(artifact.byte_length as usize);
            leased
                .file
                .take(u64::from(artifact.byte_length) + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| ErrorCode::StorageUnavailable)?;
            if bytes.len() != artifact.byte_length as usize
                || format!("{:x}", Sha256::digest(&bytes)) != artifact.sha256
            {
                return Err(ErrorCode::StorageUnavailable.into());
            }
            let parent_name = Path::new(&input.relative_path)
                .parent()
                .and_then(Path::to_str)
                .ok_or(ErrorCode::ScopeDenied)?;
            let parent = directory.open_directory(parent_name)?;
            let original = parent
                .metadata()
                .map_err(|_| ErrorCode::StorageUnavailable)?;
            if directory.list(parent_name, check)?.revision != input.expected_parent_revision {
                return Err(ErrorCode::RevisionConflict.into());
            }
            let still_current = || {
                check()?;
                let current = directory
                    .open_directory(parent_name)?
                    .metadata()
                    .map_err(|_| ErrorCode::StorageUnavailable)?;
                if (current.dev(), current.ino()) != (original.dev(), original.ino()) {
                    return Err(ErrorCode::RevisionConflict);
                }
                Ok(())
            };
            crate::atomic_file::create(&path.join(&input.relative_path), &bytes, still_current)?;
            still_current().map_err(|_| ReplaceError::Uncertain)?;
            Ok(ArtifactExported {
                workspace_id: input.workspace_id.clone(),
                relative_path: input.relative_path.clone(),
                artifact: Box::new(artifact.clone()),
            })
        };
        let result = write();
        self.finish_native_file_write(
            operation,
            nonce,
            &owner,
            result
                .as_ref()
                .map(|v| OperationResult::ArtifactExported(Box::new(v.clone())))
                .map_err(|e| match e {
                    ReplaceError::Before(code) => ReplaceError::Before(*code),
                    ReplaceError::Uncertain => ReplaceError::Uncertain,
                }),
        )?;
        result.map_err(|_| ErrorCode::OutcomeUnknown)
    }
}
