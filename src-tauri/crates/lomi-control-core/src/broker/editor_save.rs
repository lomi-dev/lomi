use super::{
    operations::{storage_error, UiMutation},
    *,
};
use crate::atomic_file::ReplaceError;

impl Broker {
    pub(super) fn validate_editor_save(
        state: &State,
        owner: &str,
        input: &EditorSaveInput,
    ) -> Result<(), ErrorCode> {
        let directory = Self::editor_read_access(state, owner, &input.read_target())?;
        if !["editor.write", "files.mutate"]
            .iter()
            .all(|s| state.sessions[owner].grant.scopes.contains(*s))
        {
            return Err(ErrorCode::ScopeDenied);
        }
        directory.open_file(&input.relative_path, 4 * 1024 * 1024)?;
        Ok(())
    }
    pub(super) fn editor_save(self: &Arc<Self>, owner: &str, input: EditorSaveInput) -> Reply {
        if !valid_id(&input.document_id)
            || !valid_id(&input.request_key)
            || input.expected_revision.parse::<u64>().is_err()
            || editor_edits::revision_number(&input.document_id, &input.expected_buffer_revision)
                .is_none()
            || input.expected_disk_revision.len() != 64
            || !input
                .expected_disk_revision
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return error(ErrorCode::ResourceExhausted);
        }
        if let Err(e) = crate::project_files::validate_relative(&input.relative_path) {
            return error(e);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        if let Err(e) = Self::project_file_access(&state, owner, &input.workspace_id) {
            return error(e);
        }
        let session = &state.sessions[owner];
        if !["editor.read", "editor.write", "files.mutate"]
            .iter()
            .all(|s| session.grant.scopes.contains(*s))
        {
            return error(ErrorCode::ScopeDenied);
        }
        if input.retry_epoch != session.retry_epoch {
            return error(ErrorCode::RetryWindowExpired);
        }
        let Some(root) = session.grant.workspace(&input.workspace_id) else {
            return error(ErrorCode::TargetNotFound);
        };
        let project = root.project_id.clone();
        let project_path = root.project_path.clone();
        let hash = match receipts::fingerprint(
            &(&input, &project_path),
            &receipts::Target {
                workspace_id: &input.workspace_id,
                resource_id: &input.panel_id,
                generation: &input.document_id,
                revision: &input.expected_buffer_revision,
            },
        ) {
            Ok(h) => h,
            Err(e) => return storage_error(e),
        };
        let key = receipts::Key {
            pairing_id: owner,
            project_id: &project,
            retry_epoch: &input.retry_epoch,
            request_key: &input.request_key,
            tool: "lomi_editor_save",
        };
        match self.replay(&state, owner, &key, hash) {
            Ok(Some(r)) => return r,
            Err(e) => return *e,
            Ok(None) => {}
        }
        if let Err(e) = Self::validate_editor_save(&state, owner, &input) {
            return error(e);
        }
        let not_after_millis = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            + 30_000)
            .to_string();
        self.enqueue_ui(
            &mut state,
            owner,
            UiMutation {
                workspace: input.workspace_id.clone(),
                project,
                revision: input.expected_revision.clone(),
                retry_epoch: input.retry_epoch.clone(),
                request_key: input.request_key.clone(),
                tool: "lomi_editor_save",
                hash,
                action: UiAction::EditorSave(EditorSaveCommand {
                    workspace_id: input.workspace_id.clone(),
                    project_path,
                    not_after_millis,
                    input,
                }),
            },
        )
    }
    /// Main-only one-use writer. Encoding belongs to the existing native editor.
    pub fn commit_editor_save(
        &self,
        operation: &str,
        nonce: &str,
        body: EditorSaveBody,
        encode: impl FnOnce(&[u8], &str) -> Result<Vec<u8>, ErrorCode>,
    ) -> Result<EditorSaved, ErrorCode> {
        let (command, owner, directory, permit, connected, policy) = {
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
            let UiAction::EditorSave(command) = &work.command.action else {
                return Err(ErrorCode::ScopeDenied);
            };
            Self::validate_editor_save(&state, &work.pairing, &command.input)?;
            let directory =
                Self::project_file_access(&state, &work.pairing, &command.workspace_id)?;
            let expected_path = Path::new(&command.project_path).join(&command.input.relative_path);
            if body.document_id != command.input.document_id
                || body.buffer_revision != command.input.expected_buffer_revision
                || body.disk_revision != command.input.expected_disk_revision
                || expected_path.to_str() != Some(body.source_path.as_str())
            {
                return Err(ErrorCode::RevisionConflict);
            }
            if body.content.len() > 4 * 1024 * 1024 || body.content.contains('\0') {
                return Err(ErrorCode::ResourceExhausted);
            }
            let session = &state.sessions[&work.pairing];
            let result = (
                command.clone(),
                work.pairing.clone(),
                directory,
                work.native_permit.clone(),
                session.alive.clone(),
                state.policy_revision,
            );
            state.work.get_mut(operation).unwrap().native_committed = true;
            result
        };
        let check = || {
            permit.check()?;
            if !connected.load(Ordering::SeqCst)
                || self.authorization.load(Ordering::SeqCst) != policy
            {
                return Err(ErrorCode::ControlRevoked);
            }
            directory.check()
        };
        let write = || -> Result<EditorSaved, ReplaceError> {
            let _producer = self
                .file_reads
                .clone()
                .try_acquire_owned()
                .map_err(|_| ErrorCode::ResourceExhausted)?;
            check()?;
            let original = directory
                .open_file(&command.input.relative_path, 4 * 1024 * 1024)?
                .read_bytes(4 * 1024 * 1024, check)?;
            if format!("{:x}", Sha256::digest(&original)) != command.input.expected_disk_revision {
                return Err(ErrorCode::RevisionConflict.into());
            }
            let bytes = encode(&original, &body.content)?;
            if bytes.len() > 4 * 1024 * 1024 {
                return Err(ErrorCode::ResourceExhausted.into());
            }
            let path = Path::new(&command.project_path).join(&command.input.relative_path);
            let disk_revision = crate::atomic_file::replace(
                &path,
                &command.input.expected_disk_revision,
                &bytes,
                check,
            )?;
            Ok(EditorSaved {
                workspace_id: command.workspace_id.clone(),
                panel_id: command.input.panel_id.clone(),
                relative_path: command.input.relative_path.clone(),
                document_id: command.input.document_id.clone(),
                saved_buffer_revision: command.input.expected_buffer_revision.clone(),
                previous_disk_revision: command.input.expected_disk_revision.clone(),
                disk_revision,
                byte_length: bytes.len() as u32,
            })
        };
        let result = write();
        self.finish_native_file_write(
            operation,
            nonce,
            &owner,
            result
                .as_ref()
                .map(|v| OperationResult::EditorSaved(Box::new(v.clone())))
                .map_err(|e| match e {
                    ReplaceError::Before(code) => ReplaceError::Before(*code),
                    ReplaceError::Uncertain => ReplaceError::Uncertain,
                }),
        )?;
        result.map_err(|_| ErrorCode::OutcomeUnknown)
    }
}
