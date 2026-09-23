use super::{
    operations::{storage_error, UiMutation},
    *,
};
use crate::atomic_file::ReplaceError;
use std::collections::BTreeSet;

pub type FilesTrashDispatch = Arc<dyn Fn(&Path) -> Result<(), ErrorCode> + Send + Sync>;
pub(super) struct TrashApproval {
    hash: String,
    approved: bool,
}

impl Broker {
    pub fn set_files_trash_dispatch(&self, dispatch: FilesTrashDispatch) -> io::Result<()> {
        *self.files_trash_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    fn trash_work<'a>(
        &self,
        state: &'a State,
        operation: &str,
        nonce: &str,
    ) -> Result<&'a super::operations::Work, ErrorCode> {
        let work = state.work.get(operation).ok_or(ErrorCode::ControlRevoked)?;
        if !work.claimed
            || work.native_committed
            || work.command.nonce != nonce
            || work.command.ui_epoch != state.projection.ui_epoch
        {
            return Err(ErrorCode::ControlRevoked);
        }
        work.native_permit.check()?;
        self.check_policy(state)
            .map_err(|_| ErrorCode::ControlRevoked)?;
        let UiAction::FilesMutate(command) = &work.command.action else {
            return Err(ErrorCode::ScopeDenied);
        };
        if !command.input.operation.is_trash() {
            return Err(ErrorCode::ScopeDenied);
        }
        Self::validate_files_mutate(state, &work.pairing, &command.input)?;
        Ok(work)
    }
    pub fn prepare_file_trash(
        &self,
        operation: &str,
        nonce: &str,
        buffers: Vec<FileTrashBuffer>,
    ) -> Result<FileTrashPlan, ErrorCode> {
        use sha2::{Digest, Sha256};
        let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        let work = self.trash_work(&state, operation, nonce)?;
        if work.command.domain_revision != state.projection.revision {
            return Err(ErrorCode::RevisionConflict);
        }
        if work.file_trash_plan.is_some() {
            return Err(ErrorCode::ControlRevoked);
        }
        let UiAction::FilesMutate(command) = &work.command.action else {
            unreachable!()
        };
        if buffers.len() > 128 {
            return Err(ErrorCode::ResourceExhausted);
        }
        let source = command.input.operation.relative_path();
        let mut ids = BTreeSet::new();
        for buffer in &buffers {
            crate::project_files::validate_relative(&buffer.relative_path)?;
            if !valid_id(&buffer.document_id)
                || !valid_id(&buffer.buffer_revision)
                || !valid_hash(&buffer.disk_revision)
                || !ids.insert(&buffer.document_id)
                || !(buffer.relative_path == source
                    || buffer.relative_path.starts_with(&format!("{source}/")))
            {
                return Err(ErrorCode::RevisionConflict);
            }
        }
        let encoded = serde_json::to_vec(&(&work.command, &buffers))
            .map_err(|_| ErrorCode::ResourceExhausted)?;
        let hash = format!("{:x}", Sha256::digest(encoded));
        let awaiting_user = !buffers.is_empty();
        self.store
            .lock()
            .map_err(|_| ErrorCode::StorageUnavailable)?
            .transition(
                &work.pairing,
                &work.project,
                operation,
                if awaiting_user {
                    receipts::State::AwaitingUser
                } else {
                    receipts::State::Running
                },
                receipts::Effect::None,
                now(),
            )
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        state.work.get_mut(operation).unwrap().file_trash_plan = Some(TrashApproval {
            hash: hash.clone(),
            approved: !awaiting_user,
        });
        Ok(FileTrashPlan {
            plan_hash: hash,
            awaiting_user,
        })
    }
    /// Main dismisses a guard when its native plan is cancelled or expires.
    pub fn file_trash_pending(&self, operation: &str, nonce: &str, plan_hash: &str) -> bool {
        let Ok(state) = self.lock_state() else {
            return false;
        };
        let Ok(work) = self.trash_work(&state, operation, nonce) else {
            return false;
        };
        work.command.domain_revision == state.projection.revision
            && work
                .file_trash_plan
                .as_ref()
                .is_some_and(|plan| !plan.approved && plan.hash == plan_hash)
    }

    /// Only the authenticated main-view decision handler calls this method.
    pub fn decide_file_trash(
        &self,
        operation: &str,
        nonce: &str,
        plan_hash: &str,
        approved: bool,
    ) -> Result<(), ErrorCode> {
        let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        let work = self.trash_work(&state, operation, nonce)?;
        if approved && work.command.domain_revision != state.projection.revision {
            return Err(ErrorCode::RevisionConflict);
        }
        let plan = work
            .file_trash_plan
            .as_ref()
            .ok_or(ErrorCode::ControlRevoked)?;
        if plan.approved || plan.hash != plan_hash {
            return Err(ErrorCode::ControlRevoked);
        }
        let mut store = self
            .store
            .lock()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        let receipt = store
            .get(&work.pairing, &work.project, operation)
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        if receipt.state != receipts::State::AwaitingUser {
            return Err(ErrorCode::ControlRevoked);
        }
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
            store
                .transition(
                    &work.pairing,
                    &work.project,
                    operation,
                    receipts::State::Running,
                    receipts::Effect::None,
                    now(),
                )
                .map_err(|_| ErrorCode::StorageUnavailable)?;
            state
                .work
                .get_mut(operation)
                .unwrap()
                .file_trash_plan
                .as_mut()
                .unwrap()
                .approved = true;
        } else {
            state.work.remove(operation);
        }
        Ok(())
    }
    pub(super) fn validate_files_mutate(
        state: &State,
        owner: &str,
        input: &FilesMutateInput,
    ) -> Result<(), ErrorCode> {
        Self::project_file_access(state, owner, &input.workspace_id)?;
        if !["files.mutate", input.operation.scope()]
            .iter()
            .all(|s| state.sessions[owner].grant.scopes.contains(*s))
        {
            return Err(ErrorCode::ScopeDenied);
        }
        crate::project_files::validate_relative(input.operation.relative_path())?;
        crate::project_files::validate_relative(&input.operation.destination())?;
        if let FileMutation::Rename { new_name, .. }
        | FileMutation::RenameDirectory { new_name, .. } = &input.operation
        {
            if new_name.contains(['/', '\\']) {
                return Err(ErrorCode::ScopeDenied);
            }
        }
        Ok(())
    }
    pub(super) fn files_mutate(self: &Arc<Self>, owner: &str, input: FilesMutateInput) -> Reply {
        if !valid_id(&input.request_key) || input.expected_revision.parse::<u64>().is_err() {
            return error(ErrorCode::ResourceExhausted);
        }
        let expected_parent_revision = input.operation.expected_parent_revision();
        if !valid_hash(expected_parent_revision) {
            return error(ErrorCode::RevisionConflict);
        }
        if let FileMutation::Trash {
            expected_entry_revision,
            ..
        } = &input.operation
        {
            if !valid_hash(expected_entry_revision) {
                return error(ErrorCode::RevisionConflict);
            }
            if self
                .files_trash_dispatch
                .lock()
                .ok()
                .is_none_or(|dispatch| dispatch.is_none())
            {
                return error(ErrorCode::HostUnqualified);
            }
        }
        if let FileMutation::Rename {
            expected_disk_revision,
            ..
        }
        | FileMutation::Move {
            expected_disk_revision,
            ..
        } = &input.operation
        {
            if !valid_hash(expected_disk_revision) {
                return error(ErrorCode::RevisionConflict);
            }
        }
        if let FileMutation::RenameDirectory {
            expected_directory_revision,
            ..
        }
        | FileMutation::MoveDirectory {
            expected_directory_revision,
            ..
        } = &input.operation
        {
            if !valid_hash(expected_directory_revision) {
                return error(ErrorCode::RevisionConflict);
            }
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        if let Err(e) = Self::validate_files_mutate(&state, owner, &input) {
            return error(e);
        }
        let session = &state.sessions[owner];
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
                resource_id: &input.workspace_id,
                generation: &project,
                revision: expected_parent_revision,
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
            tool: "lomi_files_mutate",
        };
        match self.replay(&state, owner, &key, hash) {
            Ok(Some(r)) => return r,
            Err(e) => return *e,
            Ok(None) => {}
        }
        let not_after_millis = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            + if input.operation.is_trash() {
                120_000
            } else {
                30_000
            })
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
                tool: "lomi_files_mutate",
                hash,
                action: UiAction::FilesMutate(FilesMutateCommand {
                    workspace_id: input.workspace_id.clone(),
                    project_path,
                    not_after_millis,
                    input,
                }),
            },
        )
    }
    /// Main claims this exact command before entering the shared file-write lock.
    pub fn commit_files_mutate(
        &self,
        operation: &str,
        nonce: &str,
    ) -> Result<FilesMutated, ErrorCode> {
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
            let UiAction::FilesMutate(command) = &work.command.action else {
                return Err(ErrorCode::ScopeDenied);
            };
            if command.input.operation.is_trash()
                && work
                    .file_trash_plan
                    .as_ref()
                    .is_none_or(|plan| !plan.approved)
            {
                return Err(ErrorCode::ScopeDenied);
            }
            if command.input.operation.is_trash()
                && work.command.domain_revision != state.projection.revision
            {
                return Err(ErrorCode::RevisionConflict);
            }
            Self::validate_files_mutate(&state, &work.pairing, &command.input)?;
            let directory =
                Self::project_file_access(&state, &work.pairing, &command.workspace_id)?;
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
        let write = || -> Result<FilesMutated, ReplaceError> {
            let _producer = self
                .file_reads
                .clone()
                .try_acquire_owned()
                .map_err(|_| ErrorCode::ResourceExhausted)?;
            let target = command.input.operation.destination();
            let (old_path, entry_kind) = match &command.input.operation {
                FileMutation::Trash {
                    relative_path,
                    kind,
                    expected_entry_revision,
                    expected_parent_revision,
                } => {
                    let dispatch = self
                        .files_trash_dispatch
                        .lock()
                        .map_err(|_| ErrorCode::HostUnqualified)?
                        .clone()
                        .ok_or(ErrorCode::HostUnqualified)?;
                    let recovery = self
                        .store
                        .lock()
                        .map_err(|_| ErrorCode::StorageUnavailable)?
                        .trash_recovery_root()?;
                    let staged = crate::file_trash::stage(
                        &directory,
                        crate::file_trash::TrashTarget {
                            relative_path,
                            kind,
                            expected_revision: expected_entry_revision,
                            expected_parent_revision,
                        },
                        &recovery,
                        operation,
                        check,
                    )?;
                    // Staging is already an effect. A revoked session leaves a recoverable
                    // entry instead of proceeding into the system Trash after Stop.
                    check().map_err(|_| ReplaceError::Uncertain)?;
                    staged.finish(|path| dispatch(path))?;
                    (Some(relative_path.clone()), kind.clone())
                }
                FileMutation::RenameDirectory {
                    relative_path,
                    expected_directory_revision,
                    expected_parent_revision,
                    ..
                }
                | FileMutation::MoveDirectory {
                    relative_path,
                    expected_directory_revision,
                    expected_parent_revision,
                    ..
                } => {
                    crate::file_mutation::move_directory(
                        &directory,
                        relative_path,
                        &target,
                        expected_directory_revision,
                        expected_parent_revision,
                        check,
                    )?;
                    (Some(relative_path.clone()), FileEntryKind::Directory)
                }
                FileMutation::Create {
                    relative_path,
                    kind,
                    expected_parent_revision,
                } => {
                    crate::file_mutation::create(
                        &directory,
                        relative_path,
                        expected_parent_revision,
                        kind,
                        check,
                    )?;
                    (None, kind.clone())
                }
                FileMutation::Rename {
                    relative_path,
                    expected_disk_revision,
                    expected_parent_revision,
                    ..
                }
                | FileMutation::Move {
                    relative_path,
                    expected_disk_revision,
                    expected_parent_revision,
                    ..
                } => {
                    crate::file_mutation::move_file(
                        &directory,
                        relative_path,
                        &target,
                        expected_disk_revision,
                        expected_parent_revision,
                        check,
                    )?;
                    (Some(relative_path.clone()), FileEntryKind::File)
                }
            };
            Ok(FilesMutated {
                workspace_id: command.workspace_id.clone(),
                old_path,
                new_path: (!command.input.operation.is_trash()).then_some(target),
                entry_kind,
            })
        };
        let result = write();
        self.finish_native_file_write(
            operation,
            nonce,
            &owner,
            result
                .as_ref()
                .map(|v| OperationResult::FilesMutated(v.clone()))
                .map_err(|e| match e {
                    ReplaceError::Before(code) => ReplaceError::Before(*code),
                    ReplaceError::Uncertain => ReplaceError::Uncertain,
                }),
        )?;
        result.map_err(|_| ErrorCode::OutcomeUnknown)
    }

    /// Persist native evidence before main receives it; a later ACK must match.
    pub(super) fn finish_native_file_write(
        &self,
        operation: &str,
        nonce: &str,
        owner: &str,
        result: Result<OperationResult, ReplaceError>,
    ) -> Result<(), ErrorCode> {
        let mut state = self.lock_state().map_err(|_| ErrorCode::OutcomeUnknown)?;
        let work = state
            .work
            .get(operation)
            .filter(|w| w.pairing == owner && w.command.nonce == nonce)
            .ok_or(ErrorCode::OutcomeUnknown)?;
        let mut store = self.store.lock().map_err(|_| ErrorCode::OutcomeUnknown)?;
        match result {
            Ok(result) => {
                store
                    .record_result(&work.pairing, &work.project, operation, &result)
                    .map_err(|_| ErrorCode::OutcomeUnknown)?;
                Ok(())
            }
            Err(error) => {
                use receipts::{Effect, State as OperationState};
                let (code, next, effect) = match error {
                    ReplaceError::Before(code) => {
                        let previous = store
                            .get(&work.pairing, &work.project, operation)
                            .map_err(|_| ErrorCode::StorageUnavailable)?;
                        (
                            code,
                            if previous.state == OperationState::Cancelling {
                                OperationState::Cancelled
                            } else {
                                OperationState::Failed
                            },
                            Effect::None,
                        )
                    }
                    ReplaceError::Uncertain => (
                        ErrorCode::OutcomeUnknown,
                        OperationState::OutcomeUnknown,
                        Effect::Unknown,
                    ),
                };
                store
                    .record_result(
                        &work.pairing,
                        &work.project,
                        operation,
                        &OperationResult::Failure { code },
                    )
                    .map_err(|_| ErrorCode::StorageUnavailable)?;
                store
                    .transition(&work.pairing, &work.project, operation, next, effect, now())
                    .map_err(|_| ErrorCode::StorageUnavailable)?;
                drop(store);
                state.work.remove(operation);
                Err(code)
            }
        }
    }
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
