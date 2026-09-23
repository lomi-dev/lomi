use super::{
    operations::{storage_error, UiMutation},
    *,
};

pub(super) struct PreviewPermit {
    owner: String,
    workspace: String,
    directory: Arc<crate::project_files::ProjectDirectory>,
    policy: u64,
    deadline: Instant,
    reads: u16,
    source_bytes: usize,
    output_bytes: usize,
}

impl Broker {
    pub(super) fn editor_open(self: &Arc<Self>, owner: &str, input: EditorOpenInput) -> Reply {
        if !valid_id(&input.request_key) || input.expected_revision.parse::<u64>().is_err() {
            return error(ErrorCode::ResourceExhausted);
        }
        if let Err(e) = crate::project_files::validate_relative(&input.relative_path) {
            return error(e);
        }
        if !EditorFileKind::from_relative(&input.relative_path).supports(input.presentation) {
            return error(ErrorCode::UnsupportedCapability);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        let directory = match Self::project_file_access(&state, owner, &input.workspace_id) {
            Ok(d) => d,
            Err(e) => return error(e),
        };
        let session = &state.sessions[owner];
        if !["editor.read", "panel.create", "panel.focus"]
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
                resource_id: &format!("{:x}", Sha256::digest(input.relative_path.as_bytes())),
                generation: &state.projection.ui_epoch,
                revision: &input.expected_revision,
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
            tool: "lomi_editor_open",
        };
        match self.replay(&state, owner, &key, hash) {
            Ok(Some(r)) => return r,
            Err(e) => return *e,
            Ok(None) => {}
        }
        if let Err(e) = directory.open_file(&input.relative_path, 4 * 1024 * 1024) {
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
                retry_epoch: input.retry_epoch,
                request_key: input.request_key,
                tool: "lomi_editor_open",
                hash,
                action: UiAction::EditorOpen(EditorOpenCommand {
                    workspace_id: input.workspace_id,
                    relative_path: input.relative_path,
                    presentation: input.presentation,
                    project_path,
                    not_after_millis,
                }),
            },
        )
    }
    pub fn prepare_editor_open(
        &self,
        operation: &str,
        nonce: &str,
        decode: impl FnOnce(&str, &[u8]) -> Result<PreparedEditorBody, ErrorCode>,
    ) -> Result<PreparedEditorFile, ErrorCode> {
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
            let UiAction::EditorOpen(command) = &work.command.action else {
                return Err(ErrorCode::ScopeDenied);
            };
            let directory =
                Self::project_file_access(&state, &work.pairing, &command.workspace_id)?;
            let session = &state.sessions[&work.pairing];
            if !Self::action_scopes(&work.command.action)
                .iter()
                .all(|s| session.grant.scopes.contains(*s))
            {
                return Err(ErrorCode::ScopeDenied);
            }
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
            Ok(())
        };
        let prepare = || -> Result<PreparedEditorFile, ErrorCode> {
            let _producer = self
                .file_reads
                .clone()
                .try_acquire_owned()
                .map_err(|_| ErrorCode::ResourceExhausted)?;
            check()?;
            let file = directory.open_file(&command.relative_path, 4 * 1024 * 1024)?;
            let read_only = file
                .file
                .metadata()
                .map_err(|_| ErrorCode::StorageUnavailable)?
                .permissions()
                .readonly();
            let bytes = file.read_bytes(4 * 1024 * 1024, check)?;
            let body = decode(&command.relative_path, &bytes)?;
            if matches!(body, PreparedEditorBody::Image(_))
                != (EditorFileKind::from_relative(&command.relative_path) == EditorFileKind::Image)
            {
                return Err(ErrorCode::UnsupportedCapability);
            }
            let mut result = PreparedEditorFile {
                path: Path::new(&command.project_path)
                    .join(&command.relative_path)
                    .to_str()
                    .ok_or(ErrorCode::ScopeDenied)?
                    .into(),
                relative: command.relative_path.clone(),
                revision: format!("{:x}", Sha256::digest(&bytes)),
                read_only,
                body,
                asset_permit: None,
            };
            // Leave room for the optional native-only permit added below.
            if serde_json::to_vec(&result).map_or(true, |v| v.len() > MAX_FRAME_BYTES - 128) {
                return Err(ErrorCode::ResourceExhausted);
            }
            directory.check()?;
            let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let current = Self::project_file_access(&state, &owner, &command.workspace_id)?;
            if !Arc::ptr_eq(&directory, &current)
                || !state
                    .work
                    .get(operation)
                    .is_some_and(|w| w.command.nonce == nonce)
            {
                return Err(ErrorCode::ControlRevoked);
            }
            check()?;
            if let PreparedEditorBody::Image(image) = &result.body {
                state
                    .work
                    .get_mut(operation)
                    .ok_or(ErrorCode::ControlRevoked)?
                    .editor_preview = Some(EditorPreviewed {
                    workspace_id: command.workspace_id.clone(),
                    panel_id: String::new(),
                    relative_path: command.relative_path.clone(),
                    disk_revision: result.revision.clone(),
                    width: image.width,
                    height: image.height,
                    original_width: image.original_width,
                    original_height: image.original_height,
                });
            }
            if EditorFileKind::from_relative(&command.relative_path) == EditorFileKind::Markdown {
                let policy = state.policy_revision;
                let live: HashSet<_> = state
                    .sessions
                    .iter()
                    .filter(|(_, s)| s.alive.load(Ordering::SeqCst))
                    .map(|(id, _)| id.clone())
                    .collect();
                state.preview_permits.retain(|_, p| {
                    p.deadline > Instant::now() && p.policy == policy && live.contains(&p.owner)
                });
                if state.preview_permits.len() >= 64 {
                    return Err(ErrorCode::ResourceExhausted);
                }
                let id = new_id().map_err(|_| ErrorCode::StorageUnavailable)?;
                state.preview_permits.insert(
                    id.clone(),
                    PreviewPermit {
                        owner: owner.clone(),
                        workspace: command.workspace_id.clone(),
                        directory: directory.clone(),
                        policy,
                        deadline: Instant::now() + Duration::from_secs(15 * 60),
                        reads: 0,
                        source_bytes: 0,
                        output_bytes: 0,
                    },
                );
                result.asset_permit = Some(id);
            }
            Ok(result)
        };
        let result = prepare();
        if let Err(code) = result {
            // Preparation has not published a panel. Record a definite failure
            // natively, before returning an error to the main-only caller.
            if let Ok(mut state) = self.lock_state() {
                if let Some(work) = state
                    .work
                    .get(operation)
                    .filter(|w| w.command.nonce == nonce)
                {
                    use receipts::{Effect, State as OperationState};
                    let mut store = self
                        .store
                        .lock()
                        .map_err(|_| ErrorCode::StorageUnavailable)?;
                    let previous = store
                        .get(&work.pairing, &work.project, operation)
                        .map_err(|_| ErrorCode::StorageUnavailable)?;
                    store
                        .record_result(
                            &work.pairing,
                            &work.project,
                            operation,
                            &OperationResult::Failure { code },
                        )
                        .map_err(|_| ErrorCode::StorageUnavailable)?;
                    store
                        .transition(
                            &work.pairing,
                            &work.project,
                            operation,
                            if previous.state == OperationState::Cancelling {
                                OperationState::Cancelled
                            } else {
                                OperationState::Failed
                            },
                            Effect::None,
                            now(),
                        )
                        .map_err(|_| ErrorCode::StorageUnavailable)?;
                    drop(store);
                    state.work.remove(operation);
                }
            }
        }
        result
    }

    /// Main-only follow-up reads. No caller path can replace the pinned project.
    pub fn read_preview_asset(
        &self,
        id: &str,
        relative: &str,
        decode: impl FnOnce(&str, &[u8]) -> Result<PreparedPreviewImage, ErrorCode>,
    ) -> Result<PreparedPreviewImage, ErrorCode> {
        crate::project_files::validate_relative(relative)?;
        let _producer = self
            .file_reads
            .clone()
            .try_acquire_owned()
            .map_err(|_| ErrorCode::ResourceExhausted)?;
        let (directory, owner, workspace, policy, connected, deadline) = {
            let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let p = state
                .preview_permits
                .get(id)
                .ok_or(ErrorCode::ControlRevoked)?;
            if p.deadline <= Instant::now() || p.policy != state.policy_revision {
                return Err(ErrorCode::ControlRevoked);
            }
            let directory = Self::project_file_access(&state, &p.owner, &p.workspace)?;
            let session = &state.sessions[&p.owner];
            if !session.grant.scopes.contains("editor.read")
                || !Arc::ptr_eq(&directory, &p.directory)
            {
                return Err(ErrorCode::ScopeDenied);
            }
            if p.reads >= 64
                || p.source_bytes >= 32 * 1024 * 1024
                || p.output_bytes >= 16 * 1024 * 1024
            {
                return Err(ErrorCode::ResourceExhausted);
            }
            let result = (
                directory,
                p.owner.clone(),
                p.workspace.clone(),
                p.policy,
                session.alive.clone(),
                p.deadline.min(Instant::now() + Duration::from_secs(10)),
            );
            state.preview_permits.get_mut(id).unwrap().reads += 1;
            result
        };
        let check = || {
            if !connected.load(Ordering::SeqCst)
                || self.authorization.load(Ordering::SeqCst) != policy
            {
                return Err(ErrorCode::ControlRevoked);
            }
            if Instant::now() >= deadline {
                return Err(ErrorCode::DeadlineExceeded);
            }
            Ok(())
        };
        check()?;
        let bytes = directory
            .open_file(relative, 4 * 1024 * 1024)?
            .read_bytes(4 * 1024 * 1024, check)?;
        {
            let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let p = state
                .preview_permits
                .get_mut(id)
                .ok_or(ErrorCode::ControlRevoked)?;
            p.source_bytes = p.source_bytes.saturating_add(bytes.len());
            if p.source_bytes > 32 * 1024 * 1024 {
                return Err(ErrorCode::ResourceExhausted);
            }
        }
        let result = decode(relative, &bytes)?;
        check()?;
        directory.check()?;
        let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        let current = Self::project_file_access(&state, &owner, &workspace)?;
        if !Arc::ptr_eq(&current, &directory) {
            return Err(ErrorCode::ControlRevoked);
        }
        let p = state
            .preview_permits
            .get_mut(id)
            .ok_or(ErrorCode::ControlRevoked)?;
        p.output_bytes = p.output_bytes.saturating_add(result.data_base64.len());
        if p.output_bytes > 16 * 1024 * 1024 {
            return Err(ErrorCode::ResourceExhausted);
        }
        check()?;
        Ok(result)
    }
    pub fn release_preview_permit(&self, id: &str) {
        if let Ok(mut state) = self.lock_state() {
            state.preview_permits.remove(id);
        }
    }
}
