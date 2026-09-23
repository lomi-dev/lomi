use super::{operations::UiMutation, *};
use crate::{atomic_file::ReplaceError, git_execution, project_files::validate_relative};
mod plan;
use plan::Plan;

pub(super) enum Approval {
    Preparing,
    Ready { plan: Box<Plan>, approved: bool },
}
fn files(snapshot: &git_execution::Snapshot) -> Vec<GitMutationFile> {
    snapshot
        .files
        .iter()
        .map(|file| GitMutationFile {
            relative_path: file.relative_path.clone(),
            sha256: file.sha256.clone(),
            byte_length: file.byte_length,
        })
        .collect()
}
impl Broker {
    pub(super) fn git_mutate(self: &Arc<Self>, owner: &str, input: GitMutateInput) -> Reply {
        let attempt = || -> Result<(), ErrorCode> {
            if !valid_id(&input.request_key)
                || input.expected_revision.parse::<u64>().is_err()
                || input.paths.len() > 64
            {
                return Err(ErrorCode::ResourceExhausted);
            }
            if matches!(
                input.operation,
                GitMutation::Fetch | GitMutation::Push | GitMutation::Pull
            ) {
                if !input.paths.is_empty()
                    || input.message.is_some()
                    || !input
                        .remote
                        .as_deref()
                        .is_some_and(crate::git_read::valid_remote_name)
                    || !input.reference.as_deref().is_some_and(|r| {
                        r.starts_with("refs/heads/") && git_execution::fetch::reference(r)
                    })
                {
                    return Err(ErrorCode::ScopeDenied);
                }
            } else if input.paths.is_empty() || input.remote.is_some() || input.reference.is_some()
            {
                return Err(ErrorCode::ScopeDenied);
            }
            if matches!(input.operation, GitMutation::Push | GitMutation::Pull) {
                if !input
                    .source_commit
                    .as_deref()
                    .is_some_and(git_execution::push::oid)
                    || input
                        .expected_remote_commit
                        .as_deref()
                        .is_some_and(|v| !git_execution::push::oid(v))
                {
                    return Err(ErrorCode::ScopeDenied);
                }
            } else if input.source_commit.is_some() || input.expected_remote_commit.is_some() {
                return Err(ErrorCode::ScopeDenied);
            }
            if input.operation == GitMutation::Pull {
                if input.pull_mode.is_none() || input.expected_remote_commit.is_none() {
                    return Err(ErrorCode::ScopeDenied);
                }
            } else if input.pull_mode.is_some() {
                return Err(ErrorCode::ScopeDenied);
            }
            match (input.operation, input.message.as_deref()) {
                (GitMutation::Commit, Some(message))
                    if !message.trim().is_empty()
                        && !message.contains('\0')
                        && message.len() <= 16 * 1024 => {}
                (GitMutation::Stage | GitMutation::Unstage | GitMutation::Discard, None) => {}
                (GitMutation::Fetch | GitMutation::Push | GitMutation::Pull, None) => {}
                _ => return Err(ErrorCode::ScopeDenied),
            }
            if !input.repository_relative.is_empty() {
                validate_relative(&input.repository_relative)?;
            }
            let mut unique = HashSet::new();
            for path in &input.paths {
                validate_relative(path)?;
                if !unique.insert(path) {
                    return Err(ErrorCode::RevisionConflict);
                }
            }
            Ok(())
        };
        if let Err(code) = attempt() {
            return error(code);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        if let Err(code) = Self::git_access(&state, owner, &input.workspace_id) {
            return error(code);
        }
        let session = &state.sessions[owner];
        if !["git.write", "git.execute"]
            .iter()
            .all(|s| session.grant.scopes.contains(*s))
        {
            return error(ErrorCode::ScopeDenied);
        }
        if matches!(
            input.operation,
            GitMutation::Fetch | GitMutation::Push | GitMutation::Pull
        ) && !session.grant.scopes.contains("git.network")
        {
            return error(ErrorCode::ScopeDenied);
        }
        if input.operation == GitMutation::Push && !session.grant.scopes.contains("git.push") {
            return error(ErrorCode::ScopeDenied);
        }
        if input.operation == GitMutation::Pull && !session.grant.scopes.contains("git.pull") {
            return error(ErrorCode::ScopeDenied);
        }
        if input.operation == GitMutation::Discard && !session.grant.scopes.contains("git.discard")
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
                resource_id: "git-mutation",
                generation: &state.projection.ui_epoch,
                revision: &input.expected_revision,
            },
        ) {
            Ok(hash) => hash,
            Err(e) => return operations::storage_error(e),
        };
        let key = receipts::Key {
            pairing_id: owner,
            project_id: &project,
            retry_epoch: &input.retry_epoch,
            request_key: &input.request_key,
            tool: "lomi_git_mutate",
        };
        match self.replay(&state, owner, &key, hash) {
            Ok(Some(reply)) => return reply,
            Ok(None) => {}
            Err(reply) => return *reply,
        }
        let not_after_millis = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            + 120_000)
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
                tool: "lomi_git_mutate",
                hash,
                action: UiAction::GitMutate(GitMutateCommand {
                    workspace_id: input.workspace_id.clone(),
                    input,
                    not_after_millis,
                }),
            },
        )
    }
    fn git_mutation_work<'a>(
        &self,
        state: &'a State,
        operation: &str,
        nonce: &str,
    ) -> Result<&'a operations::Work, ErrorCode> {
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
        let UiAction::GitMutate(command) = &work.command.action else {
            return Err(ErrorCode::ScopeDenied);
        };
        Self::git_access(state, &work.pairing, &command.workspace_id)?;
        if !Self::action_scopes(&work.command.action)
            .iter()
            .all(|s| state.sessions[&work.pairing].grant.scopes.contains(*s))
        {
            return Err(ErrorCode::ScopeDenied);
        }
        Ok(work)
    }
    pub fn prepare_git_mutation<G>(
        &self,
        operation: &str,
        nonce: &str,
        lock: impl FnOnce(&str) -> Result<G, ErrorCode>,
    ) -> Result<GitMutationPlan, ErrorCode> {
        let (command, owner, directory, permit, alive, policy) = {
            let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let work = self.git_mutation_work(&state, operation, nonce)?;
            if work.git_mutation.is_some() {
                return Err(ErrorCode::ControlRevoked);
            }
            let UiAction::GitMutate(command) = &work.command.action else {
                unreachable!()
            };
            let result = (
                command.clone(),
                work.pairing.clone(),
                Self::git_access(&state, &work.pairing, &command.workspace_id)?,
                work.native_permit.clone(),
                state.sessions[&work.pairing].alive.clone(),
                state.policy_revision,
            );
            state.work.get_mut(operation).unwrap().git_mutation = Some(Approval::Preparing);
            result
        };
        let prepare = || -> Result<GitMutationPlan, ErrorCode> {
            let root = if command.input.repository_relative.is_empty() {
                directory.canonical_path().to_path_buf()
            } else {
                directory
                    .canonical_path()
                    .join(&command.input.repository_relative)
            };
            let root = root.to_str().ok_or(ErrorCode::ScopeDenied)?.to_string();
            let _guard = lock(&root)?;
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
                directory.check()
            };
            let plan = Plan::prepare(
                directory.clone(),
                &command.input,
                git_execution::Environment::capture(),
                &check,
            )?;
            check()?;
            let preview = GitMutationPlan {
                plan_hash: plan.revision().into(),
                repository_path: root,
                operation: command.input.operation,
                head: plan.observation().head.clone(),
                branch: plan.observation().branch.clone(),
                files: files(plan.observation()),
                commit: plan.commit_preview(),
                network: plan.network_preview(),
                push: plan.push_preview(),
                discard: plan.discard_preview(),
                pull: plan.pull_preview(),
            };
            // Leave room for native result evidence in the 64 KiB receipt. This
            // must fail before a user can approve an unrecordable mutation.
            if serde_json::to_vec(&preview)
                .map_err(|_| ErrorCode::ResourceExhausted)?
                .len()
                > 32 * 1024
            {
                return Err(ErrorCode::ResourceExhausted);
            }
            let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let work = self.git_mutation_work(&state, operation, nonce)?;
            let current = Self::git_access(&state, &owner, &command.workspace_id)?;
            if !Arc::ptr_eq(&directory, &current) {
                return Err(ErrorCode::ControlRevoked);
            }
            self.store
                .lock()
                .map_err(|_| ErrorCode::StorageUnavailable)?
                .transition(
                    &owner,
                    &work.project,
                    operation,
                    receipts::State::AwaitingUser,
                    receipts::Effect::None,
                    now(),
                )
                .map_err(|_| ErrorCode::StorageUnavailable)?;
            state.work.get_mut(operation).unwrap().git_mutation = Some(Approval::Ready {
                plan: Box::new(plan),
                approved: false,
            });
            Ok(preview)
        };
        let result = prepare();
        if let Err(code) = result {
            self.finish_native_file_write(
                operation,
                nonce,
                &owner,
                Err(ReplaceError::Before(code)),
            )?;
        }
        result
    }
    pub fn git_mutation_pending(&self, operation: &str, nonce: &str, hash: &str) -> bool {
        let Ok(state) = self.lock_state() else {
            return false;
        };
        self.git_mutation_work(&state, operation, nonce).is_ok_and(|work|
            matches!(&work.git_mutation, Some(Approval::Ready { plan, approved: false }) if plan.revision() == hash))
    }
    pub fn decide_git_mutation(
        &self,
        operation: &str,
        nonce: &str,
        hash: &str,
        approved: bool,
    ) -> Result<(), ErrorCode> {
        let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        let work = self.git_mutation_work(&state, operation, nonce)?;
        if !matches!(&work.git_mutation, Some(Approval::Ready { plan, approved: false }) if plan.revision() == hash)
        {
            return Err(ErrorCode::ControlRevoked);
        }
        let mut store = self
            .store
            .lock()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        store
            .transition(
                &work.pairing,
                &work.project,
                operation,
                if approved {
                    receipts::State::Queued
                } else {
                    receipts::State::Cancelled
                },
                receipts::Effect::None,
                now(),
            )
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        if approved {
            let Some(Approval::Ready { approved, .. }) =
                &mut state.work.get_mut(operation).unwrap().git_mutation
            else {
                unreachable!()
            };
            *approved = true;
        } else {
            state.work.remove(operation);
        }
        Ok(())
    }
    pub fn commit_git_mutation<G>(
        &self,
        operation: &str,
        nonce: &str,
        hash: &str,
        lock: impl FnOnce(&str) -> Result<G, ErrorCode>,
    ) -> Result<GitMutated, ErrorCode> {
        let (command, owner, directory, permit, alive, policy, plan) = {
            let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let work = self.git_mutation_work(&state, operation, nonce)?;
            if !matches!(&work.git_mutation, Some(Approval::Ready { plan, approved: true }) if plan.revision() == hash)
            {
                return Err(ErrorCode::ControlRevoked);
            }
            let UiAction::GitMutate(command) = &work.command.action else {
                unreachable!()
            };
            let result = (
                command.clone(),
                work.pairing.clone(),
                Self::git_access(&state, &work.pairing, &command.workspace_id)?,
                work.native_permit.clone(),
                state.sessions[&work.pairing].alive.clone(),
                state.policy_revision,
            );
            self.store
                .lock()
                .map_err(|_| ErrorCode::StorageUnavailable)?
                .transition(
                    &work.pairing,
                    &work.project,
                    operation,
                    receipts::State::Running,
                    receipts::Effect::None,
                    now(),
                )
                .map_err(|_| ErrorCode::StorageUnavailable)?;
            let work = state.work.get_mut(operation).unwrap();
            work.native_committed = true;
            let Some(Approval::Ready { plan, .. }) = work.git_mutation.take() else {
                unreachable!()
            };
            (
                result.0, result.1, result.2, result.3, result.4, result.5, plan,
            )
        };
        let execute = || -> Result<GitMutated, ReplaceError> {
            let root = if command.input.repository_relative.is_empty() {
                directory.canonical_path().to_path_buf()
            } else {
                directory
                    .canonical_path()
                    .join(&command.input.repository_relative)
            };
            let _guard = lock(root.to_str().ok_or(ErrorCode::ScopeDenied)?)?;
            let check = || {
                permit.check()?;
                if !alive.load(Ordering::SeqCst)
                    || self.authorization.load(Ordering::SeqCst) != policy
                {
                    return Err(ErrorCode::ControlRevoked);
                }
                directory.check()
            };
            let before = plan.observation().clone();
            let commit = plan.commit_result();
            let plan::Executed {
                process: outcome,
                fetch,
                push,
                pull,
            } = plan.execute(hash, &check)?;
            if (outcome.exit_code != Some(0)
                && !pull
                    .as_ref()
                    .is_some_and(|p| p.outcome == GitPullOutcome::Conflicted))
                || outcome.interrupted.is_some()
                || outcome.abandoned_descendants
            {
                return Err(ReplaceError::Uncertain);
            }
            let after = outcome.after.ok_or(ReplaceError::Uncertain)?;
            check().map_err(|_| ReplaceError::Uncertain)?;
            Ok(GitMutated {
                workspace_id: command.workspace_id,
                repository_relative: command.input.repository_relative,
                operation: command.input.operation,
                plan_hash: hash.into(),
                before_revision: before.revision().map_err(|_| ReplaceError::Uncertain)?,
                after_revision: after.revision().map_err(|_| ReplaceError::Uncertain)?,
                head: after.head.clone(),
                index_revision: after.index_revision.clone(),
                selected_files_changed: before.files != after.files,
                files: files(&after),
                commit,
                fetch,
                push,
                pull,
            })
        };
        let result = execute();
        self.finish_native_file_write(
            operation,
            nonce,
            &owner,
            result
                .as_ref()
                .map(|v| OperationResult::GitMutated(Box::new(v.clone())))
                .map_err(|e| match e {
                    ReplaceError::Before(code) => ReplaceError::Before(*code),
                    ReplaceError::Uncertain => ReplaceError::Uncertain,
                }),
        )?;
        result.map_err(|e| match e {
            ReplaceError::Before(code) => code,
            ReplaceError::Uncertain => ErrorCode::OutcomeUnknown,
        })
    }
}
