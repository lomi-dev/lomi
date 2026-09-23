use super::*;
use crate::{
    project_files::ProjectDirectory,
    staging::{StagedCopy, MAX_IMPORT_BYTES},
};

impl Broker {
    pub(super) fn project_file_access(
        state: &State,
        owner: &str,
        workspace: &str,
    ) -> Result<Arc<ProjectDirectory>, ErrorCode> {
        let session = state
            .sessions
            .get(owner)
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .ok_or(ErrorCode::ControlRevoked)?;
        if session.grant.workspace(workspace).is_none()
            || !state
                .projection
                .workspaces
                .iter()
                .any(|w| w.id == workspace && session.grant.permits(w))
        {
            return Err(ErrorCode::TargetNotFound);
        }
        if !session.grant.scopes.contains("files.read") {
            return Err(ErrorCode::ScopeDenied);
        }
        session
            .grant
            .workspace(workspace)
            .ok_or(ErrorCode::TargetNotFound)?
            .project_directory
            .clone()
            .ok_or(ErrorCode::ScopeDenied)
    }
    pub(super) fn import_artifact(
        self: &Arc<Self>,
        owner: &str,
        input: ArtifactImportInput,
    ) -> Reply {
        use operations::{storage_error, UiMutation};
        if !valid_id(&input.request_key)
            || input.expected_revision.parse::<u64>().is_err()
            || !(4..=MAX_IMPORT_BYTES).contains(&input.expected_byte_length)
            || input.expected_sha256.len() != 64
            || !input
                .expected_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return error(ErrorCode::ResourceExhausted);
        }
        if crate::project_files::validate_relative(&input.relative_path).is_err()
            || !input.relative_path.ends_with(".apk")
        {
            return error(ErrorCode::ScopeDenied);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        if let Err(code) = Self::project_file_access(&state, owner, &input.workspace_id) {
            return error(code);
        }
        let session = &state.sessions[owner];
        if !session.grant.scopes.contains("artifact.import") {
            return error(ErrorCode::ScopeDenied);
        }
        if session.retry_epoch != input.retry_epoch {
            return error(ErrorCode::RetryWindowExpired);
        }
        let Some(root) = session.grant.workspace(&input.workspace_id) else {
            return error(ErrorCode::TargetNotFound);
        };
        let project = root.project_id.clone();
        let hash = match receipts::fingerprint(
            &(&input, &root.project_path),
            &receipts::Target {
                workspace_id: &input.workspace_id,
                resource_id: &format!("{:x}", Sha256::digest(input.relative_path.as_bytes())),
                generation: &state.projection.ui_epoch,
                revision: &input.expected_revision,
            },
        ) {
            Ok(h) => h,
            Err(e) => return storage_error(e),
        };
        self.enqueue_ui(
            &mut state,
            owner,
            UiMutation {
                workspace: input.workspace_id.clone(),
                project,
                revision: input.expected_revision.clone(),
                retry_epoch: input.retry_epoch.clone(),
                request_key: input.request_key.clone(),
                tool: "lomi_artifact_import",
                hash,
                action: UiAction::ImportArtifact(input),
            },
        )
    }
    /// Native entry point only; the renderer supplies solely the one-use work identity.
    /// APK inspection receives the completed read-only copy, never the source path.
    pub fn execute_import(
        &self,
        operation: &str,
        nonce: &str,
        validate: impl FnOnce(&fs::File, &dyn Fn() -> Result<(), ErrorCode>) -> Result<(), ErrorCode>,
    ) -> io::Result<()> {
        let (input, owner, project, directory, permit, connected, revision) = {
            let mut state = self.lock_state().map_err(|_| failure())?;
            let work = state.work.get(operation).ok_or_else(failure)?;
            if !work.claimed
                || work.native_committed
                || work.command.nonce != nonce
                || work.command.ui_epoch != state.projection.ui_epoch
            {
                return Err(failure());
            }
            work.native_permit.check().map_err(|_| failure())?;
            let UiAction::ImportArtifact(input) = &work.command.action else {
                return Err(failure());
            };
            let directory = Self::project_file_access(&state, &work.pairing, &input.workspace_id)
                .map_err(|_| failure())?;
            let session = &state.sessions[&work.pairing];
            if !session.grant.scopes.contains("artifact.import") {
                return Err(failure());
            }
            let tuple = (
                input.clone(),
                work.pairing.clone(),
                work.project.clone(),
                directory,
                work.native_permit.clone(),
                session.alive.clone(),
                state.policy_revision,
            );
            state.work.get_mut(operation).unwrap().native_committed = true;
            tuple
        };
        let check = || {
            permit.check()?;
            if !connected.load(Ordering::SeqCst)
                || self.authorization.load(Ordering::SeqCst) != revision
            {
                return Err(ErrorCode::ControlRevoked);
            }
            Ok(())
        };
        let result = (|| {
            check()?;
            let source = directory.open_file(&input.relative_path, MAX_IMPORT_BYTES)?;
            let classification = ArtifactSource::Project(ProjectArtifactSource {
                workspace_id: input.workspace_id.clone(),
                relative_path: input.relative_path.clone(),
                required_scope: "files.read".into(),
            });
            let (reservation, parent) = {
                let mut store = self
                    .store
                    .lock()
                    .map_err(|_| ErrorCode::StorageUnavailable)?;
                let reservation = store
                    .reserve_artifact(
                        &owner,
                        &project,
                        &classification,
                        input.expected_byte_length as usize,
                        now(),
                    )
                    .map_err(|e| match e {
                        receipts::Error::ResourceExhausted => ErrorCode::ResourceExhausted,
                        _ => ErrorCode::StorageUnavailable,
                    })?;
                match store.staging_directory(&reservation) {
                    Ok(parent) => (reservation, parent),
                    Err(_) => {
                        let _ = store.abandon_artifact(&reservation);
                        return Err(ErrorCode::StorageUnavailable);
                    }
                }
            };
            let copied = (|| {
                let copy = StagedCopy::copy(
                    source,
                    parent,
                    &reservation.id,
                    input.expected_byte_length,
                    &input.expected_sha256,
                    check,
                )?;
                validate(&copy.file, &check)?;
                check()?;
                directory.check()?;
                let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
                let current = Self::project_file_access(&state, &owner, &input.workspace_id)?;
                if !Arc::ptr_eq(&directory, &current) {
                    return Err(ErrorCode::ControlRevoked);
                }
                check()?;
                self.store
                    .lock()
                    .map_err(|_| ErrorCode::StorageUnavailable)?
                    .commit_import(&reservation, copy, now())
                    .map_err(|_| ErrorCode::StorageUnavailable)
            })();
            if copied.is_err() {
                if let Ok(mut store) = self.store.lock() {
                    let _ = store.abandon_artifact(&reservation);
                }
            }
            copied
        })();
        use receipts::{Effect, State as OperationState};
        let mut state = self.lock_state().map_err(|_| failure())?;
        let work = state.work.get(operation).ok_or_else(failure)?;
        if work.command.nonce != nonce {
            return Err(failure());
        }
        let (mut next, effect, output) = match result {
            Ok(artifact) if check().is_ok() => (
                OperationState::Succeeded,
                Effect::Complete,
                OperationResult::ArtifactImported {
                    workspace_id: input.workspace_id,
                    artifact_id: artifact.id,
                    sha256: artifact.sha256,
                    byte_length: artifact.byte_length,
                },
            ),
            Ok(_) => (
                OperationState::OutcomeUnknown,
                Effect::Unknown,
                OperationResult::Failure {
                    code: ErrorCode::OutcomeUnknown,
                },
            ),
            Err(code) => (
                OperationState::Failed,
                Effect::None,
                OperationResult::Failure { code },
            ),
        };
        let mut store = self.store.lock().map_err(|_| failure())?;
        if next == OperationState::Failed
            && store
                .get(&owner, &project, operation)
                .map_err(|_| failure())?
                .state
                == OperationState::Cancelling
        {
            next = OperationState::Cancelled;
        }
        store
            .record_result(&owner, &project, operation, &output)
            .map_err(|_| failure())?;
        store
            .transition(&owner, &project, operation, next, effect, now())
            .map_err(|_| failure())?;
        drop(store);
        state.work.remove(operation);
        Ok(())
    }
    pub(super) fn disclose_project_artifact(
        &self,
        state: &State,
        owner: &str,
        workspace: &str,
        artifact: Artifact,
    ) -> Reply {
        let ArtifactSource::Project(source) = &artifact.source else {
            return error(ErrorCode::ScopeDenied);
        };
        if source.required_scope != "files.read"
            || crate::project_files::validate_relative(&source.relative_path).is_err()
        {
            return error(ErrorCode::ScopeDenied);
        }
        let root = match Self::project_file_access(state, owner, workspace) {
            Ok(root) => root,
            Err(code) => return error(code),
        };
        if let Err(code) = root.check() {
            return error(code);
        }
        Reply::ok(Data::Artifact {
            artifact: Box::new(artifact),
            image: None,
        })
    }
}
