use super::{operations::UiMutation, *};
use crate::{atomic_file::ReplaceError, git_execution, project_files::validate_relative};
use lomi_native_git::types::{GitPullMode as NativeGitPullMode, NativeGitRequest};
use lomi_remote_protocol::{
    ipc::{LocalNativeGitResult, LocalNativeGitState, NativeIntentId},
    WorkspaceId,
};
use std::path::PathBuf;

pub(super) enum Approval {
    Preparing {
        operation_id: NativeIntentId,
        request_hash: Option<String>,
    },
    Ready {
        operation_id: NativeIntentId,
        request_hash: String,
        plan_hash: String,
        workspace_root: PathBuf,
        approved: bool,
        dispatched: bool,
        result: Option<GitMutated>,
        host_acknowledged: bool,
    },
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn native_git_timeout_ms(deadline: Instant) -> Result<u64, ErrorCode> {
    let remaining = deadline
        .saturating_duration_since(Instant::now())
        .as_millis()
        .min(120_000) as u64;
    if remaining == 0 {
        Err(ErrorCode::DeadlineExceeded)
    } else {
        Ok(remaining)
    }
}

#[derive(Clone, Copy)]
enum GuardPhase {
    Preparing,
    Executing,
}

fn native_git_request(input: &GitMutateInput) -> Result<String, ErrorCode> {
    use GitMutation as M;
    let request = match input.operation {
        M::Stage => NativeGitRequest::Stage {
            repository_relative: input.repository_relative.clone(),
            paths: input.paths.clone(),
        },
        M::Unstage => NativeGitRequest::Unstage {
            repository_relative: input.repository_relative.clone(),
            paths: input.paths.clone(),
        },
        M::Discard => NativeGitRequest::Discard {
            repository_relative: input.repository_relative.clone(),
            paths: input.paths.clone(),
        },
        M::Commit => NativeGitRequest::Commit {
            repository_relative: input.repository_relative.clone(),
            paths: input.paths.clone(),
            message: input.message.clone().ok_or(ErrorCode::ScopeDenied)?,
        },
        M::Fetch => NativeGitRequest::Fetch {
            repository_relative: input.repository_relative.clone(),
            remote: input.remote.clone().ok_or(ErrorCode::ScopeDenied)?,
            reference: input.reference.clone().ok_or(ErrorCode::ScopeDenied)?,
        },
        M::Push => NativeGitRequest::Push {
            repository_relative: input.repository_relative.clone(),
            remote: input.remote.clone().ok_or(ErrorCode::ScopeDenied)?,
            reference: input.reference.clone().ok_or(ErrorCode::ScopeDenied)?,
            source_commit: input.source_commit.clone().ok_or(ErrorCode::ScopeDenied)?,
            expected_remote_commit: input.expected_remote_commit.clone(),
        },
        M::Pull => NativeGitRequest::Pull {
            repository_relative: input.repository_relative.clone(),
            remote: input.remote.clone().ok_or(ErrorCode::ScopeDenied)?,
            reference: input.reference.clone().ok_or(ErrorCode::ScopeDenied)?,
            source_commit: input.source_commit.clone().ok_or(ErrorCode::ScopeDenied)?,
            expected_remote_commit: input
                .expected_remote_commit
                .clone()
                .ok_or(ErrorCode::ScopeDenied)?,
            mode: match input.pull_mode.ok_or(ErrorCode::ScopeDenied)? {
                GitPullMode::FfOnly => NativeGitPullMode::FfOnly,
                GitPullMode::Rebase => NativeGitPullMode::Rebase,
            },
        },
    };
    serde_json::to_string(&request).map_err(|_| ErrorCode::ResourceExhausted)
}

fn native_error_code(result: &LocalNativeGitResult) -> ErrorCode {
    result
        .result_json
        .as_ref()
        .and_then(|value| serde_json::from_str::<lomi_native_git::ErrorCode>(value.as_str()).ok())
        .map(crate::project_files::to_protocol_error)
        .unwrap_or(ErrorCode::ScopeDenied)
}

fn valid_native_git_result(
    result: &LocalNativeGitResult,
    operation_id: NativeIntentId,
    request_hash: &str,
    plan_hash: Option<&str>,
) -> bool {
    result.operation_id == operation_id
        && result.request_hash.expose_wire() == request_hash
        && plan_hash.is_none_or(|expected| {
            result
                .plan_hash
                .as_ref()
                .is_some_and(|actual| actual.expose_wire() == expected)
        })
}

fn convert_native_git_value<T: serde::Serialize, U: serde::de::DeserializeOwned>(
    value: T,
) -> Result<U, ErrorCode> {
    serde_json::to_value(value)
        .and_then(serde_json::from_value)
        .map_err(|_| ErrorCode::OutcomeUnknown)
}

fn native_operation_matches(
    operation: GitMutation,
    native: lomi_native_git::types::NativeGitOperation,
) -> bool {
    use lomi_native_git::types::NativeGitOperation as N;
    matches!(
        (operation, native),
        (GitMutation::Stage, N::Stage)
            | (GitMutation::Unstage, N::Unstage)
            | (GitMutation::Discard, N::Discard)
            | (GitMutation::Commit, N::Commit)
            | (GitMutation::Fetch, N::Fetch)
            | (GitMutation::Push, N::Push)
            | (GitMutation::Pull, N::Pull)
    )
}

fn git_mutated_from_native(
    outcome: lomi_native_git::types::NativeGitExecutionOutcome,
    command: &GitMutateCommand,
    plan_hash: &str,
) -> Result<GitMutated, ErrorCode> {
    use lomi_native_git::types::NativeGitProcessSettlement;
    if outcome.plan_hash != plan_hash
        || !native_operation_matches(command.input.operation, outcome.operation)
        || outcome.outcome_unknown
        || outcome.interrupted.is_some()
        || outcome.abandoned_descendants
        || outcome.settlement != NativeGitProcessSettlement::ObservedOwnedBoundaryClosed
        || (outcome.exit_code != Some(0)
            && !outcome.pull.as_ref().is_some_and(|pull| {
                pull.outcome == lomi_native_git::types::GitPullOutcome::Conflicted
            }))
    {
        return Err(ErrorCode::OutcomeUnknown);
    }
    Ok(GitMutated {
        workspace_id: command.workspace_id.clone(),
        repository_relative: command.input.repository_relative.clone(),
        operation: command.input.operation,
        plan_hash: plan_hash.to_owned(),
        before_revision: outcome.before_revision,
        after_revision: outcome.after_revision.ok_or(ErrorCode::OutcomeUnknown)?,
        head: outcome.head,
        index_revision: outcome.index_revision.ok_or(ErrorCode::OutcomeUnknown)?,
        files: outcome
            .files
            .ok_or(ErrorCode::OutcomeUnknown)?
            .into_iter()
            .map(|file| GitMutationFile {
                relative_path: file.relative_path,
                sha256: file.sha256,
                byte_length: file.byte_length,
            })
            .collect(),
        selected_files_changed: outcome
            .selected_files_changed
            .ok_or(ErrorCode::OutcomeUnknown)?,
        commit: outcome.commit.map(convert_native_git_value).transpose()?,
        fetch: outcome.fetch.map(convert_native_git_value).transpose()?,
        push: outcome.push.map(convert_native_git_value).transpose()?,
        pull: outcome.pull.map(convert_native_git_value).transpose()?,
    })
}

fn resolve_native_git_status(
    host: &mut dyn GitMutationHost,
    operation_id: NativeIntentId,
    request_hash: &str,
    plan_hash: Option<&str>,
    mut direct: Option<LocalNativeGitResult>,
) -> Result<LocalNativeGitResult, ErrorCode> {
    for attempt in 0..5 {
        if let Some(result) = direct.take() {
            if !valid_native_git_result(&result, operation_id, request_hash, plan_hash) {
                return Err(ErrorCode::OutcomeUnknown);
            }
            if !matches!(
                result.state,
                LocalNativeGitState::Preparing | LocalNativeGitState::Running
            ) {
                return Ok(result);
            }
        }
        if attempt + 1 < 5 {
            std::thread::sleep(std::time::Duration::from_millis(200));
            direct = host.status(operation_id).ok();
        }
    }
    Err(ErrorCode::OutcomeUnknown)
}

fn cancel_prepared_native_git(
    host: &mut dyn GitMutationHost,
    operation_id: NativeIntentId,
    request_hash: &str,
    plan_hash: &str,
) -> Result<LocalNativeGitResult, ErrorCode> {
    let result = host.cancel_prepared(operation_id)?;
    if !valid_native_git_result(&result, operation_id, request_hash, Some(plan_hash))
        || result.state != LocalNativeGitState::Failed
    {
        return Err(ErrorCode::OutcomeUnknown);
    }
    Ok(result)
}

fn acknowledge_native_git(
    host: &mut dyn GitMutationHost,
    operation_id: NativeIntentId,
    request_hash: &str,
    plan_hash: Option<&str>,
    expected_state: LocalNativeGitState,
) -> Result<(), ErrorCode> {
    for attempt in 0..3 {
        if attempt > 0 {
            let current = host.status(operation_id)?;
            if !valid_native_git_result(&current, operation_id, request_hash, plan_hash)
                || current.state != expected_state
            {
                return Err(ErrorCode::OutcomeUnknown);
            }
            if current.custody_settled {
                return Ok(());
            }
        }
        if let Ok(result) = host.acknowledge(operation_id) {
            if valid_native_git_result(&result, operation_id, request_hash, plan_hash)
                && result.state == expected_state
                && result.custody_settled
            {
                return Ok(());
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(ErrorCode::OutcomeUnknown)
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
        work.native_permit.check_local()?;
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

    fn git_mutation_guard_current(
        &self,
        operation: &str,
        nonce: &str,
        operation_id: NativeIntentId,
        plan_hash: Option<&str>,
        phase: GuardPhase,
    ) -> bool {
        if self.check_work_request(operation).is_err() {
            return false;
        }
        let Ok(state) = self.lock_state() else {
            return false;
        };
        let Some(work) = state.work.get(operation) else {
            return false;
        };
        if !work.claimed
            || work.command.nonce != nonce
            || work.command.ui_epoch != state.projection.ui_epoch
            || work.command.domain_revision != state.projection.revision
            || work.native_committed != matches!(phase, GuardPhase::Executing)
            || work.native_permit.revalidate().is_err()
            || !state
                .sessions
                .get(&work.pairing)
                .is_some_and(|session| session.alive.load(Ordering::SeqCst))
            || self.authorization.load(Ordering::SeqCst) != state.policy_revision
        {
            return false;
        }
        let UiAction::GitMutate(command) = &work.command.action else {
            return false;
        };
        if Self::git_access(&state, &work.pairing, &command.workspace_id).is_err()
            || !Self::action_scopes(&work.command.action)
                .iter()
                .all(|scope| state.sessions[&work.pairing].grant.scopes.contains(*scope))
        {
            return false;
        }
        match (&work.git_mutation, phase) {
            (
                Some(Approval::Preparing {
                    operation_id: current,
                    ..
                }),
                GuardPhase::Preparing,
            ) => *current == operation_id,
            (
                Some(Approval::Ready {
                    operation_id: current,
                    plan_hash: current_hash,
                    approved: true,
                    dispatched: true,
                    ..
                }),
                GuardPhase::Executing,
            ) => *current == operation_id && plan_hash.is_some_and(|hash| hash == current_hash),
            _ => false,
        }
    }

    fn record_git_mutation_failure(
        &self,
        operation: &str,
        nonce: &str,
        code: ErrorCode,
        cancelled: bool,
    ) -> Result<(), ErrorCode> {
        let state = self
            .lock_state()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        let work = state
            .work
            .get(operation)
            .filter(|work| work.command.nonce == nonce)
            .ok_or(ErrorCode::ControlRevoked)?;
        let mut store = self
            .store
            .lock()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        let receipt = store
            .get(&work.pairing, &work.project, operation)
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        if !cancelled
            && receipt.result.is_none()
            && matches!(
                receipt.state,
                receipts::State::Queued | receipts::State::Running | receipts::State::Cancelling
            )
        {
            store
                .record_result(
                    &work.pairing,
                    &work.project,
                    operation,
                    &OperationResult::Failure { code },
                )
                .map_err(|_| ErrorCode::StorageUnavailable)?;
        }
        let next = if cancelled || receipt.state == receipts::State::Cancelling {
            receipts::State::Cancelled
        } else {
            receipts::State::Failed
        };
        if receipt.state != next {
            store
                .transition(
                    &work.pairing,
                    &work.project,
                    operation,
                    next,
                    receipts::Effect::None,
                    now(),
                )
                .map_err(|_| ErrorCode::StorageUnavailable)?;
        }
        Ok(())
    }

    fn remove_git_mutation_work(&self, operation: &str, nonce: &str) {
        if let Ok(mut state) = self.lock_state() {
            if state
                .work
                .get(operation)
                .is_some_and(|work| work.command.nonce == nonce)
            {
                state.work.remove(operation);
            }
        }
    }
    pub fn prepare_git_mutation<G>(
        &self,
        operation: &str,
        nonce: &str,
        lock: impl FnOnce(&str) -> Result<G, ErrorCode>,
        host: &mut dyn GitMutationHost,
    ) -> Result<GitMutationPlan, ErrorCode> {
        let _request_permit = self.check_work_request(operation)?;
        let operation_id = NativeIntentId::try_from(operation.to_owned())
            .map_err(|_| ErrorCode::ResourceExhausted)?;
        let (command, owner, directory, request_json, existing_hash, already_preparing, deadline) = {
            let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let work = self.git_mutation_work(&state, operation, nonce)?;
            let UiAction::GitMutate(command) = &work.command.action else {
                unreachable!()
            };
            let (existing_hash, already_preparing) = match &work.git_mutation {
                None => (None, false),
                Some(Approval::Preparing {
                    operation_id: current,
                    request_hash,
                }) if *current == operation_id => (request_hash.clone(), true),
                _ => return Err(ErrorCode::ControlRevoked),
            };
            let request_json = native_git_request(&command.input)?;
            let result = (
                command.clone(),
                work.pairing.clone(),
                Self::git_access(&state, &work.pairing, &command.workspace_id)?,
                request_json,
                existing_hash,
                already_preparing,
                work.deadline,
            );
            if !already_preparing {
                state.work.get_mut(operation).unwrap().git_mutation = Some(Approval::Preparing {
                    operation_id,
                    request_hash: None,
                });
            }
            result
        };
        let root = match directory.canonical_path().to_str() {
            Some(root) => root.to_owned(),
            None => {
                self.record_git_mutation_failure(operation, nonce, ErrorCode::ScopeDenied, false)?;
                self.remove_git_mutation_work(operation, nonce);
                return Err(ErrorCode::ScopeDenied);
            }
        };
        let _guard = match lock(&root) {
            Ok(guard) => guard,
            Err(code) => {
                self.record_git_mutation_failure(operation, nonce, code, false)?;
                self.remove_git_mutation_work(operation, nonce);
                return Err(code);
            }
        };
        let _producer = match self.file_reads.clone().try_acquire_owned() {
            Ok(producer) => producer,
            Err(_) => {
                self.record_git_mutation_failure(
                    operation,
                    nonce,
                    ErrorCode::ResourceExhausted,
                    false,
                )?;
                self.remove_git_mutation_work(operation, nonce);
                return Err(ErrorCode::ResourceExhausted);
            }
        };
        let request_hash = if already_preparing {
            existing_hash.ok_or(ErrorCode::OutcomeUnknown)?
        } else {
            let timeout_ms = match native_git_timeout_ms(deadline) {
                Ok(timeout_ms) => timeout_ms,
                Err(code) => {
                    self.record_git_mutation_failure(operation, nonce, code, false)?;
                    self.remove_git_mutation_work(operation, nonce);
                    return Err(code);
                }
            };
            let workspace_id = match WorkspaceId::try_from(command.workspace_id.clone()) {
                Ok(workspace_id) => workspace_id,
                Err(_) => {
                    self.record_git_mutation_failure(
                        operation,
                        nonce,
                        ErrorCode::ScopeDenied,
                        false,
                    )?;
                    self.remove_git_mutation_work(operation, nonce);
                    return Err(ErrorCode::ScopeDenied);
                }
            };
            let mut check = || {
                self.git_mutation_guard_current(
                    operation,
                    nonce,
                    operation_id,
                    None,
                    GuardPhase::Preparing,
                )
            };
            let prepared = match host.prepare_guarded(
                GitMutationHostPrepare {
                    operation_id,
                    workspace_id,
                    workspace_root: directory.canonical_path().to_path_buf(),
                    request_json,
                    timeout_ms,
                },
                &mut check,
            ) {
                Ok(prepared) => prepared,
                Err(code) => {
                    self.record_git_mutation_failure(operation, nonce, code, false)?;
                    self.remove_git_mutation_work(operation, nonce);
                    return Err(code);
                }
            };
            if !valid_hash(&prepared.request_hash) {
                self.finish_native_file_write(
                    operation,
                    nonce,
                    &owner,
                    Err(ReplaceError::Uncertain),
                )?;
                return Err(ErrorCode::OutcomeUnknown);
            }
            if let Ok(mut state) = self.lock_state() {
                if let Some(Approval::Preparing {
                    operation_id: current,
                    request_hash,
                }) = state
                    .work
                    .get_mut(operation)
                    .and_then(|work| work.git_mutation.as_mut())
                {
                    if *current == operation_id {
                        *request_hash = Some(prepared.request_hash.clone());
                    }
                }
            }
            let direct = prepared.response.ok();
            let result =
                resolve_native_git_status(host, operation_id, &prepared.request_hash, None, direct);
            let result = match result {
                Ok(result) => result,
                Err(code) => {
                    self.finish_native_file_write(
                        operation,
                        nonce,
                        &owner,
                        Err(ReplaceError::Uncertain),
                    )?;
                    return Err(code);
                }
            };
            if result.state == LocalNativeGitState::Failed {
                let code = native_error_code(&result);
                self.record_git_mutation_failure(operation, nonce, code, false)?;
                if acknowledge_native_git(
                    host,
                    operation_id,
                    &prepared.request_hash,
                    None,
                    LocalNativeGitState::Failed,
                )
                .is_ok()
                {
                    self.remove_git_mutation_work(operation, nonce);
                    return Err(code);
                }
                return Err(ErrorCode::OutcomeUnknown);
            }
            if result.state != LocalNativeGitState::Prepared {
                self.finish_native_file_write(
                    operation,
                    nonce,
                    &owner,
                    Err(ReplaceError::Uncertain),
                )?;
                return Err(ErrorCode::OutcomeUnknown);
            }
            if !self.git_mutation_guard_current(
                operation,
                nonce,
                operation_id,
                None,
                GuardPhase::Preparing,
            ) {
                let Some(prepared_plan_hash) = result
                    .plan_hash
                    .as_ref()
                    .map(|value| value.expose_wire().to_owned())
                else {
                    self.finish_native_file_write(
                        operation,
                        nonce,
                        &owner,
                        Err(ReplaceError::Uncertain),
                    )?;
                    return Err(ErrorCode::OutcomeUnknown);
                };
                if cancel_prepared_native_git(
                    host,
                    operation_id,
                    &prepared.request_hash,
                    &prepared_plan_hash,
                )
                .is_ok()
                {
                    self.record_git_mutation_failure(
                        operation,
                        nonce,
                        ErrorCode::ControlRevoked,
                        false,
                    )?;
                    if acknowledge_native_git(
                        host,
                        operation_id,
                        &prepared.request_hash,
                        Some(&prepared_plan_hash),
                        LocalNativeGitState::Failed,
                    )
                    .is_ok()
                    {
                        self.remove_git_mutation_work(operation, nonce);
                        return Err(ErrorCode::ControlRevoked);
                    }
                    return Err(ErrorCode::OutcomeUnknown);
                }
                self.finish_native_file_write(
                    operation,
                    nonce,
                    &owner,
                    Err(ReplaceError::Uncertain),
                )?;
                return Err(ErrorCode::OutcomeUnknown);
            }
            let plan_hash = result
                .plan_hash
                .as_ref()
                .map(|hash| hash.expose_wire().to_owned())
                .filter(|hash| valid_hash(hash));
            let Some(plan_hash) = plan_hash else {
                self.finish_native_file_write(
                    operation,
                    nonce,
                    &owner,
                    Err(ReplaceError::Uncertain),
                )?;
                return Err(ErrorCode::OutcomeUnknown);
            };
            let preview_json = result.preview_json.as_ref();
            let Some(preview_json) = preview_json else {
                self.finish_native_file_write(
                    operation,
                    nonce,
                    &owner,
                    Err(ReplaceError::Uncertain),
                )?;
                return Err(ErrorCode::OutcomeUnknown);
            };
            let preview: GitMutationPlan = match serde_json::from_str(preview_json.as_str()) {
                Ok(preview) => preview,
                Err(_) => {
                    self.finish_native_file_write(
                        operation,
                        nonce,
                        &owner,
                        Err(ReplaceError::Uncertain),
                    )?;
                    return Err(ErrorCode::OutcomeUnknown);
                }
            };
            let preview_size = serde_json::to_vec(&preview).map(|json| json.len());
            if preview.plan_hash != plan_hash
                || preview.operation != command.input.operation
                || preview_size.map_or(true, |size| size > 32 * 1024)
            {
                self.finish_native_file_write(
                    operation,
                    nonce,
                    &owner,
                    Err(ReplaceError::Uncertain),
                )?;
                return Err(ErrorCode::OutcomeUnknown);
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
                operation_id,
                request_hash: prepared.request_hash.clone(),
                plan_hash,
                workspace_root: directory.canonical_path().to_path_buf(),
                approved: false,
                dispatched: false,
                result: None,
                host_acknowledged: false,
            });
            host.retain_prepared_for_approval(operation_id);
            return Ok(preview);
        };

        let status = match resolve_native_git_status(host, operation_id, &request_hash, None, None)
        {
            Ok(status) => status,
            Err(code) => {
                self.finish_native_file_write(
                    operation,
                    nonce,
                    &owner,
                    Err(ReplaceError::Uncertain),
                )?;
                return Err(code);
            }
        };
        if status.state == LocalNativeGitState::Prepared {
            let plan_hash = status
                .plan_hash
                .as_ref()
                .map(|hash| hash.expose_wire().to_owned())
                .filter(|hash| valid_hash(hash))
                .ok_or(ErrorCode::OutcomeUnknown);
            let plan_hash = match plan_hash {
                Ok(plan_hash) => plan_hash,
                Err(code) => {
                    self.finish_native_file_write(
                        operation,
                        nonce,
                        &owner,
                        Err(ReplaceError::Uncertain),
                    )?;
                    return Err(code);
                }
            };
            let preview_json = status
                .preview_json
                .as_ref()
                .ok_or(ErrorCode::OutcomeUnknown);
            let preview_json = match preview_json {
                Ok(preview_json) => preview_json,
                Err(code) => {
                    self.finish_native_file_write(
                        operation,
                        nonce,
                        &owner,
                        Err(ReplaceError::Uncertain),
                    )?;
                    return Err(code);
                }
            };
            let preview: GitMutationPlan = match serde_json::from_str(preview_json.as_str()) {
                Ok(preview) => preview,
                Err(_) => {
                    self.finish_native_file_write(
                        operation,
                        nonce,
                        &owner,
                        Err(ReplaceError::Uncertain),
                    )?;
                    return Err(ErrorCode::OutcomeUnknown);
                }
            };
            if preview.plan_hash != plan_hash
                || preview.operation != command.input.operation
                || serde_json::to_vec(&preview)
                    .map(|json| json.len() > 32 * 1024)
                    .unwrap_or(true)
            {
                self.finish_native_file_write(
                    operation,
                    nonce,
                    &owner,
                    Err(ReplaceError::Uncertain),
                )?;
                return Err(ErrorCode::OutcomeUnknown);
            }
            let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let work = self.git_mutation_work(&state, operation, nonce)?;
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
                operation_id,
                request_hash,
                plan_hash,
                workspace_root: directory.canonical_path().to_path_buf(),
                approved: false,
                dispatched: false,
                result: None,
                host_acknowledged: false,
            });
            host.retain_prepared_for_approval(operation_id);
            Ok(preview)
        } else if status.state == LocalNativeGitState::Failed {
            let code = native_error_code(&status);
            self.record_git_mutation_failure(operation, nonce, code, false)?;
            if acknowledge_native_git(
                host,
                operation_id,
                &request_hash,
                None,
                LocalNativeGitState::Failed,
            )
            .is_ok()
            {
                self.remove_git_mutation_work(operation, nonce);
                Err(code)
            } else {
                Err(ErrorCode::OutcomeUnknown)
            }
        } else {
            self.finish_native_file_write(operation, nonce, &owner, Err(ReplaceError::Uncertain))?;
            Err(ErrorCode::OutcomeUnknown)
        }
    }
    pub fn git_mutation_pending(&self, operation: &str, nonce: &str, hash: &str) -> bool {
        if self.check_work_request(operation).is_err() {
            return false;
        }
        let Ok(state) = self.lock_state() else {
            return false;
        };
        self.git_mutation_work(&state, operation, nonce).is_ok_and(|work|
            matches!(&work.git_mutation, Some(Approval::Ready { plan_hash, approved: false, dispatched: false, .. }) if plan_hash == hash))
    }
    pub fn decide_git_mutation(
        &self,
        operation: &str,
        nonce: &str,
        hash: &str,
        approved: bool,
        host: &mut dyn GitMutationHost,
    ) -> Result<(), ErrorCode> {
        let _request_permit = if approved {
            Some(self.check_work_request(operation)?)
        } else {
            None
        };
        let (operation_id, request_hash, plan_hash, owner, project) = {
            let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let work = self.git_mutation_work(&state, operation, nonce)?;
            let Some(Approval::Ready {
                operation_id,
                request_hash,
                plan_hash,
                approved: false,
                dispatched: false,
                ..
            }) = &work.git_mutation
            else {
                return Err(ErrorCode::ControlRevoked);
            };
            if plan_hash != hash {
                return Err(ErrorCode::ControlRevoked);
            }
            (
                *operation_id,
                request_hash.clone(),
                plan_hash.clone(),
                work.pairing.clone(),
                work.project.clone(),
            )
        };
        if approved {
            let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            self.git_mutation_work(&state, operation, nonce)?;
            self.store
                .lock()
                .map_err(|_| ErrorCode::StorageUnavailable)?
                .transition(
                    &owner,
                    &project,
                    operation,
                    receipts::State::Queued,
                    receipts::Effect::None,
                    now(),
                )
                .map_err(|_| ErrorCode::StorageUnavailable)?;
            let Some(Approval::Ready { approved, .. }) =
                &mut state.work.get_mut(operation).unwrap().git_mutation
            else {
                unreachable!()
            };
            *approved = true;
        } else {
            if !valid_hash(&request_hash) {
                return Err(ErrorCode::OutcomeUnknown);
            }
            let request_hash = request_hash.as_str();
            let status = resolve_native_git_status(
                host,
                operation_id,
                request_hash,
                Some(&plan_hash),
                None,
            )?;
            let cancelled = if status.state == LocalNativeGitState::Prepared {
                cancel_prepared_native_git(host, operation_id, request_hash, &plan_hash)?
            } else if status.state == LocalNativeGitState::Failed {
                status
            } else {
                return Err(ErrorCode::OutcomeUnknown);
            };
            self.record_git_mutation_failure(operation, nonce, ErrorCode::ControlRevoked, true)?;
            acknowledge_native_git(
                host,
                operation_id,
                request_hash,
                Some(&plan_hash),
                LocalNativeGitState::Failed,
            )?;
            let _ = cancelled;
            self.remove_git_mutation_work(operation, nonce);
        }
        Ok(())
    }
    pub fn commit_git_mutation<G>(
        &self,
        operation: &str,
        nonce: &str,
        hash: &str,
        lock: impl FnOnce(&str) -> Result<G, ErrorCode>,
        host: &mut dyn GitMutationHost,
    ) -> Result<GitMutated, ErrorCode> {
        let (already_dispatched, lock_path) = {
            let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let work = state.work.get(operation).ok_or(ErrorCode::ControlRevoked)?;
            if work.command.nonce != nonce {
                return Err(ErrorCode::ControlRevoked);
            }
            let Some(Approval::Ready {
                dispatched,
                workspace_root,
                plan_hash,
                approved: true,
                ..
            }) = &work.git_mutation
            else {
                return Err(ErrorCode::ControlRevoked);
            };
            if plan_hash != hash {
                return Err(ErrorCode::ControlRevoked);
            }
            let path = if *dispatched {
                None
            } else {
                let UiAction::GitMutate(command) = &work.command.action else {
                    return Err(ErrorCode::ScopeDenied);
                };
                let repository_path = if command.input.repository_relative.is_empty() {
                    workspace_root.clone()
                } else {
                    workspace_root.join(&command.input.repository_relative)
                };
                Some(
                    repository_path
                        .to_str()
                        .ok_or(ErrorCode::ScopeDenied)?
                        .to_owned(),
                )
            };
            (*dispatched, path)
        };
        if !already_dispatched {
            self.check_work_request(operation)?;
        }
        let _guard = match lock_path {
            Some(path) => Some(lock(&path)?),
            None => None,
        };
        let _project_write = if already_dispatched {
            None
        } else {
            Some(self.project_write_admission()?)
        };
        let (
            command,
            owner,
            operation_id,
            request_hash,
            plan_hash,
            saved_result,
            host_acknowledged,
            execute_timeout_ms,
        ) = {
            let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let work = state.work.get(operation).ok_or(ErrorCode::ControlRevoked)?;
            if !work.claimed || work.command.nonce != nonce {
                return Err(ErrorCode::ControlRevoked);
            }
            if !already_dispatched
                && (work.command.ui_epoch != state.projection.ui_epoch
                    || work.command.domain_revision != state.projection.revision)
            {
                return Err(ErrorCode::ControlRevoked);
            }
            let UiAction::GitMutate(command) = &work.command.action else {
                return Err(ErrorCode::ScopeDenied);
            };
            let Some(Approval::Ready {
                operation_id,
                request_hash,
                plan_hash,
                workspace_root: _,
                approved: true,
                dispatched,
                result,
                host_acknowledged,
                ..
            }) = &work.git_mutation
            else {
                return Err(ErrorCode::ControlRevoked);
            };
            let execute_timeout_ms = if already_dispatched {
                None
            } else {
                Some(native_git_timeout_ms(work.deadline)?)
            };
            if plan_hash != hash || *dispatched != already_dispatched {
                return Err(ErrorCode::ControlRevoked);
            }
            if already_dispatched {
                if !work.native_committed {
                    return Err(ErrorCode::ControlRevoked);
                }
            } else {
                self.git_mutation_work(&state, operation, nonce)?;
                Self::git_access(&state, &work.pairing, &command.workspace_id)?;
            }
            let values = (
                command.clone(),
                work.pairing.clone(),
                *operation_id,
                request_hash.clone(),
                plan_hash.clone(),
                result.clone(),
                *host_acknowledged,
                execute_timeout_ms,
            );
            if !already_dispatched {
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
                let Some(Approval::Ready { dispatched, .. }) = &mut work.git_mutation else {
                    unreachable!()
                };
                *dispatched = true;
            }
            values
        };

        let just_dispatched = !already_dispatched;

        if let Some(result) = saved_result {
            if host_acknowledged {
                return Ok(result);
            }
            let status = resolve_native_git_status(
                host,
                operation_id,
                &request_hash,
                Some(&plan_hash),
                None,
            )?;
            if status.state != LocalNativeGitState::Succeeded {
                return Err(ErrorCode::OutcomeUnknown);
            }
            acknowledge_native_git(
                host,
                operation_id,
                &request_hash,
                Some(&plan_hash),
                LocalNativeGitState::Succeeded,
            )?;
            self.mark_git_mutation_acknowledged(operation, nonce, &plan_hash)?;
            return Ok(result);
        }

        let mut direct = if just_dispatched {
            let mut check = || {
                self.git_mutation_guard_current(
                    operation,
                    nonce,
                    operation_id,
                    Some(&plan_hash),
                    GuardPhase::Executing,
                )
            };
            host.execute_guarded(
                operation_id,
                &request_hash,
                &plan_hash,
                execute_timeout_ms.ok_or(ErrorCode::OutcomeUnknown)?,
                &mut check,
            )
            .ok()
        } else {
            None
        };
        let status = match resolve_native_git_status(
            host,
            operation_id,
            &request_hash,
            Some(&plan_hash),
            direct.take(),
        ) {
            Ok(status) => status,
            Err(code) => {
                self.finish_native_file_write(
                    operation,
                    nonce,
                    &owner,
                    Err(ReplaceError::Uncertain),
                )?;
                return Err(code);
            }
        };

        match status.state {
            LocalNativeGitState::Succeeded => {
                let outcome = status.result_json.as_ref().and_then(|json| {
                    serde_json::from_str::<lomi_native_git::types::NativeGitExecutionOutcome>(
                        json.as_str(),
                    )
                    .ok()
                });
                let Some(outcome) = outcome else {
                    self.finish_native_file_write(
                        operation,
                        nonce,
                        &owner,
                        Err(ReplaceError::Uncertain),
                    )?;
                    return Err(ErrorCode::OutcomeUnknown);
                };
                let result = match git_mutated_from_native(outcome, &command, &plan_hash) {
                    Ok(result) => result,
                    Err(code) => {
                        self.finish_native_file_write(
                            operation,
                            nonce,
                            &owner,
                            Err(ReplaceError::Uncertain),
                        )?;
                        return Err(code);
                    }
                };
                self.finish_native_file_write(
                    operation,
                    nonce,
                    &owner,
                    Ok(OperationResult::GitMutated(Box::new(result.clone()))),
                )?;
                if let Ok(mut state) = self.lock_state() {
                    if let Some(Approval::Ready { result: saved, .. }) = state
                        .work
                        .get_mut(operation)
                        .and_then(|work| work.git_mutation.as_mut())
                    {
                        *saved = Some(result.clone());
                    }
                }
                acknowledge_native_git(
                    host,
                    operation_id,
                    &request_hash,
                    Some(&plan_hash),
                    LocalNativeGitState::Succeeded,
                )?;
                self.mark_git_mutation_acknowledged(operation, nonce, &plan_hash)?;
                Ok(result)
            }
            LocalNativeGitState::Failed => {
                let code = native_error_code(&status);
                self.record_git_mutation_failure(operation, nonce, code, false)?;
                acknowledge_native_git(
                    host,
                    operation_id,
                    &request_hash,
                    Some(&plan_hash),
                    LocalNativeGitState::Failed,
                )?;
                self.remove_git_mutation_work(operation, nonce);
                Err(code)
            }
            LocalNativeGitState::Prepared => {
                let cancelled =
                    cancel_prepared_native_git(host, operation_id, &request_hash, &plan_hash)?;
                let _ = cancelled;
                self.record_git_mutation_failure(
                    operation,
                    nonce,
                    ErrorCode::ControlRevoked,
                    false,
                )?;
                acknowledge_native_git(
                    host,
                    operation_id,
                    &request_hash,
                    Some(&plan_hash),
                    LocalNativeGitState::Failed,
                )?;
                self.remove_git_mutation_work(operation, nonce);
                Err(ErrorCode::ControlRevoked)
            }
            LocalNativeGitState::Preparing
            | LocalNativeGitState::Running
            | LocalNativeGitState::OutcomeUnknown => {
                self.finish_native_file_write(
                    operation,
                    nonce,
                    &owner,
                    Err(ReplaceError::Uncertain),
                )?;
                Err(ErrorCode::OutcomeUnknown)
            }
        }
    }

    fn mark_git_mutation_acknowledged(
        &self,
        operation: &str,
        nonce: &str,
        plan_hash: &str,
    ) -> Result<(), ErrorCode> {
        let mut state = self.lock_state().map_err(|_| ErrorCode::OutcomeUnknown)?;
        let work = state
            .work
            .get_mut(operation)
            .ok_or(ErrorCode::OutcomeUnknown)?;
        if work.command.nonce != nonce {
            return Err(ErrorCode::OutcomeUnknown);
        }
        let Some(Approval::Ready {
            plan_hash: current,
            host_acknowledged,
            ..
        }) = &mut work.git_mutation
        else {
            return Err(ErrorCode::OutcomeUnknown);
        };
        if current != plan_hash {
            return Err(ErrorCode::OutcomeUnknown);
        }
        *host_acknowledged = true;
        Ok(())
    }
}
