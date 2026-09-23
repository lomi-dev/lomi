use super::{
    operations::{storage_error, UiMutation},
    *,
};
use crate::project_files::{validate_relative, ProjectDirectory};

pub(super) struct GitPermit {
    owner: String,
    command: GitOpenCommand,
    directory: Arc<ProjectDirectory>,
    policy: u64,
    expires: Instant,
    reads: u16,
    bytes: usize,
}
fn validate(repository: &str, view: &GitView) -> Result<(), ErrorCode> {
    if !repository.is_empty() {
        validate_relative(repository)?;
    }
    match view {
        GitView::Diff { relative_path, .. } => validate_relative(relative_path),
        GitView::Commit { commit }
            if matches!(commit.len(), 40 | 64) && commit.bytes().all(|c| c.is_ascii_hexdigit()) =>
        {
            Ok(())
        }
        _ => Err(ErrorCode::ScopeDenied),
    }
}
fn observe(
    command: &GitOpenCommand,
    directory: &ProjectDirectory,
    relative: Option<&str>,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<GitViewBody, ErrorCode> {
    let data = match (&command.view, relative) {
        (
            GitView::Diff {
                relative_path,
                staged,
            },
            None,
        ) => git_observations::observe_diff(
            &GitDiffInput {
                workspace_id: command.workspace_id.clone(),
                repository_relative: command.repository_relative.clone(),
                relative_path: relative_path.clone(),
                comparison: if *staged {
                    GitComparison::Staged
                } else {
                    GitComparison::Worktree
                },
                commit: None,
                start_utf16: 0,
                max_chars: 8192,
                expected_observation_revision: None,
            },
            directory,
            check,
            true,
        )?,
        (GitView::Commit { commit }, Some(path)) => {
            validate_relative(path)?;
            git_observations::observe_diff(
                &GitDiffInput {
                    workspace_id: command.workspace_id.clone(),
                    repository_relative: command.repository_relative.clone(),
                    relative_path: path.into(),
                    comparison: GitComparison::Commit,
                    commit: Some(commit.clone()),
                    start_utf16: 0,
                    max_chars: 8192,
                    expected_observation_revision: None,
                },
                directory,
                check,
                true,
            )?
        }
        (GitView::Commit { commit }, None) => git_observations::observe_commit(
            &GitCommitInput {
                workspace_id: command.workspace_id.clone(),
                repository_relative: command.repository_relative.clone(),
                commit: commit.clone(),
                files_skip: 0,
                files_limit: 200,
            },
            directory,
            check,
            true,
        )?,
        _ => return Err(ErrorCode::ScopeDenied),
    };
    match data {
        Data::GitDiff(value) => Ok(GitViewBody::Diff(value)),
        Data::GitCommit(value) => Ok(GitViewBody::Commit(value)),
        _ => Err(ErrorCode::OutcomeUnknown),
    }
}
fn encode(body: &GitViewBody) -> Result<Vec<u8>, ErrorCode> {
    let bytes = serde_json::to_vec(body).map_err(|_| ErrorCode::ResourceExhausted)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(ErrorCode::ResourceExhausted);
    }
    Ok(bytes)
}
impl Broker {
    pub(super) fn git_open(self: &Arc<Self>, owner: &str, input: GitOpenInput) -> Reply {
        if !valid_id(&input.request_key) || input.expected_revision.parse::<u64>().is_err() {
            return error(ErrorCode::ResourceExhausted);
        }
        if let Err(code) = validate(&input.repository_relative, &input.view) {
            return error(code);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        if let Err(code) = Self::git_access(&state, owner, &input.workspace_id) {
            return error(code);
        }
        let session = &state.sessions[owner];
        if !["panel.create", "panel.focus"]
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
        let hash = match receipts::fingerprint(
            &(&input, &root.project_path),
            &receipts::Target {
                workspace_id: &input.workspace_id,
                resource_id: "git-view",
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
            tool: "lomi_git_open",
        };
        match self.replay(&state, owner, &key, hash) {
            Ok(Some(reply)) => return reply,
            Ok(None) => {}
            Err(reply) => return *reply,
        }
        self.enqueue_ui(
            &mut state,
            owner,
            UiMutation {
                workspace: input.workspace_id.clone(),
                project,
                revision: input.expected_revision,
                retry_epoch: input.retry_epoch,
                request_key: input.request_key,
                tool: "lomi_git_open",
                hash,
                action: UiAction::GitOpen(GitOpenCommand {
                    workspace_id: input.workspace_id,
                    repository_relative: input.repository_relative,
                    view: input.view,
                    not_after_millis: (std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                        + 30000)
                        .to_string(),
                }),
            },
        )
    }
    pub fn prepare_git_open(
        &self,
        operation: &str,
        nonce: &str,
    ) -> Result<PreparedGitView, ErrorCode> {
        let (command, owner, directory, permit, alive, policy) = {
            let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let work = state.work.get(operation).ok_or(ErrorCode::ControlRevoked)?;
            if !work.claimed
                || work.native_committed
                || work.command.nonce != nonce
                || work.command.ui_epoch != state.projection.ui_epoch
            {
                return Err(ErrorCode::ControlRevoked);
            }
            if work.command.domain_revision != state.projection.revision {
                return Err(ErrorCode::RevisionConflict);
            }
            work.native_permit.check()?;
            let UiAction::GitOpen(command) = &work.command.action else {
                return Err(ErrorCode::ScopeDenied);
            };
            let directory = Self::git_access(&state, &work.pairing, &command.workspace_id)?;
            if !Self::action_scopes(&work.command.action)
                .iter()
                .all(|s| state.sessions[&work.pairing].grant.scopes.contains(*s))
            {
                return Err(ErrorCode::ScopeDenied);
            }
            let result = (
                command.clone(),
                work.pairing.clone(),
                directory,
                work.native_permit.clone(),
                state.sessions[&work.pairing].alive.clone(),
                state.policy_revision,
            );
            state.work.get_mut(operation).unwrap().native_committed = true;
            result
        };
        let prepare = || -> Result<PreparedGitView, ErrorCode> {
            let _producer = self
                .file_reads
                .clone()
                .try_acquire_owned()
                .map_err(|_| ErrorCode::ResourceExhausted)?;
            let check = || {
                permit.check()?;
                if !alive.load(Ordering::SeqCst)
                    || self.authorization.load(Ordering::SeqCst) != policy
                {
                    return Err(ErrorCode::ControlRevoked);
                }
                Ok(())
            };
            let body = observe(&command, &directory, None, &check)?;
            let bytes = encode(&body)?;
            let observation_revision = format!("{:x}", Sha256::digest(&bytes));
            let root_path = if command.repository_relative.is_empty() {
                directory.canonical_path().to_path_buf()
            } else {
                directory
                    .canonical_path()
                    .join(&command.repository_relative)
            };
            let root = root_path
                .to_str()
                .ok_or(ErrorCode::ScopeDenied)?
                .to_string();
            directory.check()?;
            check()?;
            let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let current = Self::git_access(&state, &owner, &command.workspace_id)?;
            if !Arc::ptr_eq(&directory, &current) {
                return Err(ErrorCode::ControlRevoked);
            }
            state
                .git_permits
                .retain(|_, p| p.expires > Instant::now() && p.policy == policy);
            if state.git_permits.len() >= 64 {
                return Err(ErrorCode::ResourceExhausted);
            }
            let id = new_id().map_err(|_| ErrorCode::ResourceExhausted)?;
            let work = state
                .work
                .get_mut(operation)
                .ok_or(ErrorCode::ControlRevoked)?;
            work.native_permit.check()?;
            work.git_view_revision = Some(observation_revision.clone());
            state.git_permits.insert(
                id.clone(),
                GitPermit {
                    owner,
                    command,
                    directory,
                    policy,
                    expires: Instant::now() + Duration::from_secs(900),
                    reads: 1,
                    bytes: bytes.len(),
                },
            );
            check()?;
            Ok(PreparedGitView {
                root,
                permit_id: id,
                observation_revision,
                body,
            })
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

    pub fn read_git_view(
        &self,
        id: &str,
        relative: Option<&str>,
    ) -> Result<GitViewBody, ErrorCode> {
        let (command, owner, directory, alive, policy, expires) = {
            let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let p = state.git_permits.get(id).ok_or(ErrorCode::ControlRevoked)?;
            if p.policy != state.policy_revision || p.expires <= Instant::now() {
                return Err(ErrorCode::ControlRevoked);
            }
            if p.reads >= 64 || p.bytes >= 16 * 1024 * 1024 {
                return Err(ErrorCode::ResourceExhausted);
            }
            let directory = Self::git_access(&state, &p.owner, &p.command.workspace_id)?;
            if !Arc::ptr_eq(&directory, &p.directory) {
                return Err(ErrorCode::ControlRevoked);
            }
            let result = (
                p.command.clone(),
                p.owner.clone(),
                directory,
                state.sessions[&p.owner].alive.clone(),
                p.policy,
                p.expires,
            );
            state.git_permits.get_mut(id).unwrap().reads += 1;
            result
        };
        let _producer = self
            .file_reads
            .clone()
            .try_acquire_owned()
            .map_err(|_| ErrorCode::ResourceExhausted)?;
        let check = || {
            if !alive.load(Ordering::SeqCst)
                || self.authorization.load(Ordering::SeqCst) != policy
                || Instant::now() >= expires
            {
                return Err(ErrorCode::ControlRevoked);
            }
            Ok(())
        };
        let body = observe(&command, &directory, relative, &check)?;
        let bytes = encode(&body)?;
        directory.check()?;
        check()?;
        let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        let current = Self::git_access(&state, &owner, &command.workspace_id)?;
        if !Arc::ptr_eq(&directory, &current) {
            return Err(ErrorCode::ControlRevoked);
        }
        let p = state
            .git_permits
            .get_mut(id)
            .ok_or(ErrorCode::ControlRevoked)?;
        if bytes.len() > (16 * 1024 * 1024_usize).saturating_sub(p.bytes) {
            return Err(ErrorCode::ResourceExhausted);
        }
        p.bytes += bytes.len();
        check()?;
        Ok(body)
    }
    pub fn release_git_view(&self, id: &str) {
        if let Ok(mut state) = self.lock_state() {
            state.git_permits.remove(id);
        }
    }
}
