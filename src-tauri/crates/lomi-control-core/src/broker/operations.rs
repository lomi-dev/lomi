use super::*;
use receipts::{Effect, State as OperationState};

pub(super) struct Work {
    pub(super) project_open: Option<super::project_open::Approval>,
    pub(super) settings_update: Option<super::settings_update::SettingsPlan>,
    pub(super) android_input: Option<Arc<crate::android_input::InputLease>>,
    pub(super) pairing: String,
    pub(super) project: String,
    pub(super) workspace: String,
    pub(super) command: UiCommand,
    pub(super) deadline: Instant,
    pub(super) claimed: bool,
    pub(super) native_committed: bool,
    pub(super) file_trash_plan: Option<super::files_mutate::TrashApproval>,
    pub(super) editor_preview: Option<EditorPreviewed>,
    pub(super) git_view_revision: Option<String>,
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    pub(super) git_mutation: Option<super::git_mutate::Approval>,
    pub(super) native_permit: NativePermit,
}
#[derive(Clone)]
pub struct NativePermit {
    active: Arc<AtomicBool>,
    deadline: Instant,
}
impl NativePermit {
    pub(super) fn until(deadline: Instant) -> Self {
        Self {
            active: Arc::new(AtomicBool::new(true)),
            deadline,
        }
    }
    pub(super) fn revoke(&self) {
        self.active.store(false, Ordering::SeqCst);
    }
    pub fn check(&self) -> Result<(), ErrorCode> {
        if !self.active.load(Ordering::SeqCst) {
            return Err(ErrorCode::ControlRevoked);
        }
        if Instant::now() >= self.deadline {
            return Err(ErrorCode::DeadlineExceeded);
        }
        Ok(())
    }
}
impl Drop for Work {
    fn drop(&mut self) {
        if let Some(lease) = &self.android_input {
            lease.revoke();
        }
        self.native_permit.active.store(false, Ordering::SeqCst);
    }
}
pub(super) struct UiMutation {
    pub workspace: String,
    pub project: String,
    pub revision: String,
    pub retry_epoch: String,
    pub request_key: String,
    pub tool: &'static str,
    pub hash: [u8; 32],
    pub action: UiAction,
}
pub(super) fn storage_error(error: receipts::Error) -> Reply {
    super::error(match error {
        receipts::Error::IdempotencyConflict => ErrorCode::IdempotencyConflict,
        receipts::Error::RetryWindowExpired => ErrorCode::RetryWindowExpired,
        receipts::Error::ResourceExhausted => ErrorCode::ResourceExhausted,
        receipts::Error::TargetNotFound => ErrorCode::TargetNotFound,
        _ => ErrorCode::StorageUnavailable,
    })
}
impl Broker {
    pub(super) fn action_scopes(action: &UiAction) -> &'static [&'static str] {
        match action {
            UiAction::OpenSettings(_) => &["settings.open"],
            UiAction::UpdateSettings(_) => &["settings.read", "settings.write"],
            UiAction::OpenProject(_) => &["project.open", "workspace.write", "panel.create"],
            UiAction::CloseProject(command) => {
                if command
                    .workspaces
                    .iter()
                    .any(|w| w.panels.iter().any(|p| p.terminal_session_id.is_some()))
                {
                    &[
                        "project.close",
                        "workspace.write",
                        "workspace.close",
                        "panel.close",
                        "terminal.execute",
                    ]
                } else {
                    &[
                        "project.close",
                        "workspace.write",
                        "workspace.close",
                        "panel.close",
                    ]
                }
            }
            UiAction::CloseWorkspace(command) => {
                if command
                    .panels
                    .iter()
                    .any(|p| p.terminal_session_id.is_some())
                {
                    &[
                        "workspace.write",
                        "workspace.close",
                        "panel.close",
                        "terminal.execute",
                    ]
                } else {
                    &["workspace.write", "workspace.close", "panel.close"]
                }
            }
            UiAction::MovePanel(command) => {
                if matches!(command.movement, PanelMove::ReorderTab { .. }) {
                    &["workspace.write", "panel.move"]
                } else {
                    &["workspace.write", "panel.move", "panel.focus"]
                }
            }
            UiAction::FilesMutate(command) => match command.input.operation {
                FileMutation::Trash { .. } => &["files.read", "files.mutate", "files.trash"],
                FileMutation::Create { .. } => &["files.read", "files.mutate", "files.create"],
                _ => &["files.read", "files.mutate", "files.rename"],
            },
            UiAction::EditorSave(_) => {
                &["files.read", "files.mutate", "editor.read", "editor.write"]
            }
            UiAction::GitOpen(_) => &["files.read", "git.read", "panel.create", "panel.focus"],
            UiAction::GitMutate(command) => match command.input.operation {
                GitMutation::Pull => &[
                    "files.read",
                    "git.read",
                    "git.write",
                    "git.execute",
                    "git.network",
                    "git.pull",
                ],
                GitMutation::Discard => &[
                    "files.read",
                    "git.read",
                    "git.write",
                    "git.execute",
                    "git.discard",
                ],
                GitMutation::Push => &[
                    "files.read",
                    "git.read",
                    "git.write",
                    "git.execute",
                    "git.network",
                    "git.push",
                ],
                GitMutation::Fetch => &[
                    "files.read",
                    "git.read",
                    "git.write",
                    "git.execute",
                    "git.network",
                ],
                _ => &["files.read", "git.read", "git.write", "git.execute"],
            },
            UiAction::EditorOpen(_) => {
                &["files.read", "editor.read", "panel.create", "panel.focus"]
            }
            UiAction::EditorEdits(_) => &["files.read", "editor.read", "editor.write"],
            UiAction::AndroidLaunch(_) => &["android.read", "android.control", "android.launch"],
            UiAction::ImportArtifact(_) => &["files.read", "artifact.import"],
            UiAction::AndroidInput(_) | UiAction::AndroidInputControl(_) => {
                &["android.read", "android.control", "android.interact"]
            }
            UiAction::AndroidRuntime { .. } => &["android.read", "android.control"],
            UiAction::CreateAndroid { .. } => &["panel.create", "android.read"],
            UiAction::CreateBrowser { .. } => &["panel.create", "browser.navigate"],
            UiAction::InteractBrowser(_) => &["browser.interact"],
            UiAction::NavigateBrowser { .. } => &["browser.navigate"],
            UiAction::ClosePanel {
                terminal_session_id: Some(_),
                ..
            } => &["panel.close", "panel.create", "terminal.execute"],
            UiAction::ClosePanel { .. } => &["panel.close", "panel.create"],
            UiAction::SelectWorkspace { .. } => &["workspace.write", "panel.focus"],
            UiAction::FocusPanel { .. } => &["panel.focus"],
            UiAction::RenameWorkspace { .. } => &["workspace.write"],
            UiAction::CreateWorkspace { .. } => &["workspace.write", "panel.create"],
            UiAction::CreateTerminal { .. } => &["panel.create", "terminal.execute"],
        }
    }
    pub fn set_ui_dispatch(&self, dispatch: UiDispatch) -> io::Result<()> {
        *self.dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    pub(super) fn operation_reply(receipt: receipts::Receipt) -> Reply {
        Reply::ok(Data::Operation {
            operation_id: receipt.operation_id,
            state: serde_json::to_value(receipt.state)
                .unwrap()
                .as_str()
                .unwrap()
                .into(),
            effect_state: serde_json::to_value(receipt.effect_state)
                .unwrap()
                .as_str()
                .unwrap()
                .into(),
            workspace_id: receipt.workspace_id,
            result: receipt.result,
        })
    }
    pub(super) fn end_work(&self, state: &mut State, pairing: Option<&str>) {
        state
            .git_panels
            .retain(|_, (owner, _)| pairing.is_some_and(|p| p != owner));
        for (id, session) in &state.sessions {
            if pairing.is_none_or(|p| p == id) {
                session.alive.store(false, Ordering::SeqCst);
            }
        }
        state.android.retain(|_, target| {
            if pairing.is_none_or(|p| target.owner == p) {
                target.control.revoke();
                false
            } else {
                true
            }
        });
        state
            .android_logs
            .retain(|_, batch| pairing.is_some_and(|p| p != batch.owner));
        state
            .file_searches
            .retain(|_, batch| pairing.is_some_and(|p| p != batch.owner));
        self.end_claims(state, pairing);
        self.end_installs(state, pairing);
        state.browsers.retain(|_, browser| {
            if pairing.is_none_or(|p| browser.owner == p) {
                browser.control.revoke();
                false
            } else {
                true
            }
        });
        self.end_runs(state, pairing);
        state.terminals.retain(|_, terminal| {
            if pairing.is_none_or(|p| terminal.owner == p) {
                if let Ok(mut control) = terminal.control.lock() {
                    control.detach();
                }
                false
            } else {
                true
            }
        });
        let ids: Vec<_> = state
            .work
            .iter()
            .filter(|(_, w)| pairing.is_none_or(|p| w.pairing == p))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if let Some(work) = state.work.remove(&id) {
                self.settle_lost(&work);
            }
        }
    }
    pub(super) fn settle_lost(&self, work: &Work) {
        if let Ok(mut store) = self.store.lock() {
            let no_trash_effect = (matches!(&work.command.action, UiAction::FilesMutate(c) if c.input.operation.is_trash())
                || matches!(
                    &work.command.action,
                    UiAction::GitMutate(_) | UiAction::OpenProject(_) | UiAction::UpdateSettings(_)
                ))
                && !work.native_committed;
            let (next, effect) = if work.claimed && !no_trash_effect {
                (OperationState::OutcomeUnknown, Effect::Unknown)
            } else {
                (OperationState::Cancelled, Effect::None)
            };
            if no_trash_effect
                && store
                    .get(&work.pairing, &work.project, &work.command.operation_id)
                    .is_ok_and(|r| r.state == OperationState::Running)
            {
                let _ = store.transition(
                    &work.pairing,
                    &work.project,
                    &work.command.operation_id,
                    OperationState::Cancelling,
                    Effect::None,
                    now(),
                );
            }
            let _ = store.transition(
                &work.pairing,
                &work.project,
                &work.command.operation_id,
                next,
                effect,
                now(),
            );
        }
    }
    pub(super) fn rename(self: &Arc<Self>, id: &str, input: WorkspaceUpdateInput) -> Reply {
        match input {
            WorkspaceUpdateInput::Rename(input) => self.workspace_change(id, input, false),
            WorkspaceUpdateInput::Select(input) => self.select_workspace(id, input),
            WorkspaceUpdateInput::Close(input) => self.close_workspace(id, input),
        }
    }
    pub(super) fn create_workspace(
        self: &Arc<Self>,
        id: &str,
        input: WorkspaceRenameInput,
    ) -> Reply {
        self.workspace_change(id, input, true)
    }
    fn workspace_change(
        self: &Arc<Self>,
        id: &str,
        input: WorkspaceRenameInput,
        create: bool,
    ) -> Reply {
        if input.name.trim().is_empty()
            || input.name.len() > 256
            || input.name.chars().any(char::is_control)
            || !valid_id(&input.request_key)
            || input.expected_revision.parse::<u64>().is_err()
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::AppUnavailable);
        };
        let Some(session) = state
            .sessions
            .get(id)
            .filter(|s| s.alive.load(Ordering::SeqCst))
        else {
            return error(ErrorCode::ControlRevoked);
        };
        if !session.grant.scopes.contains("workspace.write")
            || (create && !session.grant.scopes.contains("panel.create"))
        {
            return error(ErrorCode::ScopeDenied);
        }
        let Some(workspace) = state
            .projection
            .workspaces
            .iter()
            .find(|w| w.id == input.workspace_id && session.grant.permits(w))
        else {
            return error(ErrorCode::TargetNotFound);
        };
        let project = workspace.project_id.clone();
        let project_path = workspace.project_path.clone();
        if input.retry_epoch != session.retry_epoch {
            return error(ErrorCode::RetryWindowExpired);
        }
        let target = receipts::Target {
            workspace_id: &input.workspace_id,
            resource_id: &input.workspace_id,
            generation: &state.projection.ui_epoch,
            revision: &input.expected_revision,
        };
        let hash = match receipts::fingerprint(&(&input, &project_path), &target) {
            Ok(hash) => hash,
            Err(e) => return storage_error(e),
        };
        let action = if create {
            UiAction::CreateWorkspace {
                anchor_workspace_id: input.workspace_id.clone(),
                workspace_id: match new_id() {
                    Ok(id) => id,
                    Err(_) => return error(ErrorCode::ResourceExhausted),
                },
                tab_id: match new_id() {
                    Ok(id) => id,
                    Err(_) => return error(ErrorCode::ResourceExhausted),
                },
                name: input.name,
            }
        } else {
            UiAction::RenameWorkspace {
                workspace_id: input.workspace_id.clone(),
                name: input.name,
            }
        };
        self.enqueue_ui(
            &mut state,
            id,
            UiMutation {
                workspace: input.workspace_id,
                project,
                revision: input.expected_revision,
                retry_epoch: input.retry_epoch,
                request_key: input.request_key,
                tool: if create {
                    "lomi_workspace_create"
                } else {
                    "lomi_workspace_update"
                },
                hash,
                action,
            },
        )
    }
    pub(super) fn replay(
        &self,
        state: &State,
        id: &str,
        key: &receipts::Key<'_>,
        hash: [u8; 32],
    ) -> Result<Option<Reply>, Box<Reply>> {
        let store = self
            .store
            .lock()
            .map_err(|_| error(ErrorCode::StorageUnavailable))?;
        let Some((receipt, recorded)) = store.existing(key, now()).map_err(storage_error)? else {
            return Ok(None);
        };
        if !state.sessions.contains_key(id) {
            return Err(Box::new(error(ErrorCode::ControlRevoked)));
        }
        if !Self::receipt_workspace_authorized(state, id, &receipt) {
            return Err(Box::new(error(ErrorCode::TargetNotFound)));
        }
        if recorded != hash {
            return Err(Box::new(error(ErrorCode::IdempotencyConflict)));
        }
        Ok(Some(Self::terminal_receipt(state, id, receipt)))
    }
    pub(super) fn enqueue_ui(
        self: &Arc<Self>,
        state: &mut State,
        id: &str,
        input: UiMutation,
    ) -> Reply {
        let project_open = match Self::project_open_approval(state, id, &input.action) {
            Ok(value) => value,
            Err(code) => return error(code),
        };
        let Ok(mut store) = self.store.lock() else {
            return error(ErrorCode::StorageUnavailable);
        };
        let key = receipts::Key {
            pairing_id: id,
            project_id: &input.project,
            retry_epoch: &input.retry_epoch,
            request_key: &input.request_key,
            tool: input.tool,
        };
        let reservation = match store.reserve(&key, input.hash, now()) {
            Ok(v) => v,
            Err(e) => return storage_error(e),
        };
        if !reservation.created {
            return Self::terminal_receipt(state, id, reservation.receipt);
        }
        let op = reservation.receipt.operation_id;
        if let Err(e) = store.bind_workspace(id, &input.project, &op, &input.workspace) {
            return storage_error(e);
        }
        if let UiAction::OpenProject(command) = &input.action {
            if let Err(e) = store.record_result(
                id,
                &input.project,
                &op,
                &OperationResult::ProjectOpened(Box::new(Self::project_opened(command, None))),
            ) {
                return storage_error(e);
            }
        }
        let dispatch = self.dispatch.lock().ok().and_then(|d| d.clone());
        let failure_code = if self.check_policy(state).is_err()
            || state
                .sessions
                .get(id)
                .is_none_or(|s| !s.alive.load(Ordering::SeqCst))
        {
            Some(ErrorCode::ControlRevoked)
        } else if state.projection.revision != input.revision {
            Some(ErrorCode::RevisionConflict)
        } else if state.work.len() >= 32 {
            Some(ErrorCode::ResourceExhausted)
        } else if dispatch.is_none() {
            Some(ErrorCode::UiNotReady)
        } else {
            Self::reserve_android_input(state, &input.action, &op).err()
        };
        if let Some(code) = failure_code {
            if !matches!(input.action, UiAction::OpenProject(_)) {
                if let Err(e) =
                    store.record_result(id, &input.project, &op, &OperationResult::Failure { code })
                {
                    return storage_error(e);
                }
            }
            return match store.transition(
                id,
                &input.project,
                &op,
                OperationState::Failed,
                Effect::None,
                now(),
            ) {
                Ok(r) => Self::operation_reply(r),
                Err(e) => storage_error(e),
            };
        }
        let Ok(nonce) = new_id() else {
            return error(ErrorCode::ResourceExhausted);
        };
        let input_lease = Self::android_work_lease(state, &input.action);
        let command = UiCommand {
            operation_id: op.clone(),
            nonce,
            ui_epoch: state.projection.ui_epoch.clone(),
            domain_revision: input.revision,
            project_id: input.project.clone(),
            action: input.action,
        };
        let duration = if matches!(&command.action, UiAction::FilesMutate(c) if c.input.operation.is_trash())
            || matches!(
                &command.action,
                UiAction::GitMutate(_)
                    | UiAction::CloseWorkspace(_)
                    | UiAction::CloseProject(_)
                    | UiAction::OpenProject(_)
                    | UiAction::UpdateSettings(_)
            ) {
            Duration::from_secs(120)
        } else if matches!(
            command.action,
            UiAction::AndroidRuntime { .. } | UiAction::ImportArtifact(_)
        ) {
            Duration::from_secs(180)
        } else {
            Duration::from_secs(30)
        };
        state.work.insert(
            op.clone(),
            Work {
                project_open,
                settings_update: None,
                android_input: input_lease,
                pairing: id.into(),
                project: input.project.clone(),
                workspace: input.workspace,
                command: command.clone(),
                deadline: Instant::now() + duration,
                claimed: false,
                native_committed: false,
                file_trash_plan: None,
                editor_preview: None,
                git_view_revision: None,
                #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
                git_mutation: None,
                native_permit: NativePermit {
                    active: Arc::new(AtomicBool::new(true)),
                    deadline: Instant::now() + duration,
                },
            },
        );
        let receipt = match store.get(id, &input.project, &op) {
            Ok(r) => r,
            Err(e) => return storage_error(e),
        };
        drop(store);
        if dispatch.unwrap()(command).is_err() {
            if let Some(work) = state.work.remove(&op) {
                self.settle_lost(&work);
            }
            return error(ErrorCode::UiNotReady);
        }
        let weak = Arc::downgrade(self);
        self.spawn_background(async move {
            tokio::time::sleep(duration).await;
            if let Some(broker) = weak.upgrade() {
                let worker = broker.clone();
                let _ = broker
                    .spawn_worker(move || {
                        let broker = worker;
                        if let Ok(mut state) = broker.lock_state() {
                            if let Some(work) = state.work.remove(&op) {
                                broker.settle_lost(&work);
                            }
                        };
                    })
                    .await;
            }
        });
        Self::operation_reply(receipt)
    }
    pub fn claim_ui(&self, epoch: &str, operation: &str, nonce: &str) -> io::Result<()> {
        let mut state = self.lock_state().map_err(|_| failure())?;
        let work = state.work.get(operation).ok_or_else(failure)?;
        if work.claimed
            || work.deadline <= Instant::now()
            || work.command.nonce != nonce
            || work.command.ui_epoch != epoch
            || state.projection.ui_epoch != epoch
            || (!matches!(
                work.command.action,
                UiAction::NavigateBrowser { .. }
                    | UiAction::InteractBrowser(_)
                    | UiAction::AndroidInput(_)
                    | UiAction::ImportArtifact(_)
            ) && state.projection.revision != work.command.domain_revision)
        {
            return Err(failure());
        }
        let session = state
            .sessions
            .get(&work.pairing)
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .ok_or_else(failure)?;
        if !Self::action_scopes(&work.command.action)
            .iter()
            .all(|scope| session.grant.scopes.contains(*scope))
            || session
                .grant
                .workspace(&work.workspace)
                .is_none_or(|p| p.project_id != work.project)
            || (!matches!(work.command.action, UiAction::OpenProject(_))
                && !state
                    .projection
                    .workspaces
                    .iter()
                    .any(|w| w.id == work.workspace && session.grant.permits(w)))
        {
            return Err(failure());
        }
        Self::validate_panel_action(&state, &work.pairing, &work.command.action)
            .map_err(|_| failure())?;
        if !matches!(&work.command.action, UiAction::FilesMutate(c) if c.input.operation.is_trash())
            && !matches!(&work.command.action, UiAction::GitMutate(_))
        {
            self.store
                .lock()
                .map_err(|_| failure())?
                .transition(
                    &work.pairing,
                    &work.project,
                    operation,
                    if matches!(
                        work.command.action,
                        UiAction::OpenProject(_) | UiAction::UpdateSettings(_)
                    ) {
                        OperationState::AwaitingUser
                    } else {
                        OperationState::Running
                    },
                    Effect::None,
                    now(),
                )
                .map_err(|_| failure())?;
        }
        self.check_policy(&state)?;
        if !session.alive.load(Ordering::SeqCst) {
            return Err(failure());
        }
        state.work.get_mut(operation).unwrap().claimed = true;
        Ok(())
    }
    pub fn acknowledge_ui(&self, mut ack: UiAck) -> io::Result<()> {
        let mut state = self.lock_state().map_err(|_| failure())?;
        let work = state.work.get(&ack.operation_id).ok_or_else(failure)?;
        if work.deadline <= Instant::now()
            || work.command.nonce != ack.nonce
            || work.command.ui_epoch != ack.ui_epoch
            || state.projection.ui_epoch != ack.ui_epoch
            || !state.sessions.contains_key(&work.pairing)
        {
            return Err(failure());
        }
        let session = state
            .sessions
            .get(&work.pairing)
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .ok_or_else(failure)?;
        if !Self::action_scopes(&work.command.action)
            .iter()
            .all(|scope| session.grant.scopes.contains(*scope))
            || session
                .grant
                .workspace(&work.workspace)
                .is_none_or(|p| p.project_id != work.project)
            || (!state
                .projection
                .workspaces
                .iter()
                .any(|w| w.id == work.workspace && session.grant.permits(w))
                && !matches!(work.command.action, UiAction::OpenProject(_))
                && !(work.native_committed
                    && session
                        .grant
                        .workspace(&work.workspace)
                        .is_some_and(|p| p.project_id == work.project)
                    && matches!(
                        work.command.action,
                        UiAction::CloseWorkspace(_) | UiAction::CloseProject(_)
                    )))
        {
            return Err(failure());
        }
        let opened_project = if let OperationResult::ProjectOpened(result) = &ack.result {
            Some(Self::project_open_completed(&state, work, result).map_err(|_| failure())?)
        } else {
            None
        };
        let (next, effect) = match &ack.result {
            OperationResult::ProjectOpened(_) => (OperationState::Succeeded, Effect::Complete),
            OperationResult::SettingsUpdated(result) => {
                let UiAction::UpdateSettings(command) = &work.command.action else {
                    return Err(failure());
                };
                if !work.claimed
                    || !work.native_committed
                    || !result.applied
                    || result.section != command.input.patch.section()
                    || result.workspace_id != command.workspace_id
                {
                    return Err(failure());
                }
                let receipt = self
                    .store
                    .lock()
                    .map_err(|_| failure())?
                    .get(&work.pairing, &work.project, &ack.operation_id)
                    .map_err(|_| failure())?;
                if !matches!(receipt.result,Some(OperationResult::SettingsUpdated(ref saved)) if saved == result)
                {
                    return Err(failure());
                }
                (OperationState::Succeeded, Effect::Complete)
            }
            OperationResult::SettingsOpened(result) => {
                let UiAction::OpenSettings(command) = &work.command.action else {
                    return Err(failure());
                };
                if !work.claimed
                    || !work.native_committed
                    || !result.requested
                    || result.workspace_id != command.workspace_id
                    || result.page != command.page
                {
                    return Err(failure());
                }
                let receipt = self
                    .store
                    .lock()
                    .map_err(|_| failure())?
                    .get(&work.pairing, &work.project, &ack.operation_id)
                    .map_err(|_| failure())?;
                if !matches!(receipt.result, Some(OperationResult::SettingsOpened(ref saved)) if saved == result)
                {
                    return Err(failure());
                }
                (OperationState::Succeeded, Effect::Complete)
            }
            OperationResult::AndroidPanel {
                workspace_id,
                panel_id,
                device_id,
            } => {
                let UiAction::CreateAndroid {
                    workspace_id: expected_workspace,
                    panel_id: expected_panel,
                    device_id: expected_device,
                    ..
                } = &work.command.action
                else {
                    return Err(failure());
                };
                if !work.claimed
                    || workspace_id != expected_workspace
                    || panel_id != expected_panel
                    || device_id != expected_device
                    || !session.grant.android_devices.contains(device_id)
                    || !state.projection.panels.iter().any(|p| {
                        p.id == *panel_id
                            && p.workspace_id == *workspace_id
                            && p.kind == "android"
                            && p.android_device_id.as_ref() == Some(device_id)
                    })
                {
                    return Err(failure());
                }
                (OperationState::Succeeded, Effect::Complete)
            }
            OperationResult::BrowserInteraction(result) => {
                let UiAction::InteractBrowser(command) = &work.command.action else {
                    return Err(failure());
                };
                let browser = Self::browser_target(
                    &state,
                    &work.pairing,
                    &command.workspace_id,
                    &command.panel_id,
                    &command.browser_generation,
                    "browser.interact",
                )
                .map_err(|_| failure())?;
                if !work.native_committed
                    || browser
                        .control
                        .interaction_result(&ack.operation_id)
                        .map_err(|_| failure())?
                        != **result
                {
                    return Err(failure());
                }
                (OperationState::Succeeded, Effect::Complete)
            }
            OperationResult::BrowserNavigation(result) => {
                let UiAction::NavigateBrowser {
                    workspace_id,
                    panel_id,
                    browser_generation,
                    wait_until,
                    ..
                } = &work.command.action
                else {
                    return Err(failure());
                };
                let browser = Self::browser_target(
                    &state,
                    &work.pairing,
                    workspace_id,
                    panel_id,
                    browser_generation,
                    "browser.navigate",
                )
                .map_err(|_| failure())?;
                if !work.native_committed
                    || result.workspace_id != *workspace_id
                    || result.panel_id != *panel_id
                    || result.browser_generation != *browser_generation
                    || browser
                        .control
                        .navigation_observation(&ack.operation_id, *wait_until)
                        .map_err(|_| failure())?
                        .is_none()
                {
                    return Err(failure());
                }
                (OperationState::Succeeded, Effect::Complete)
            }
            OperationResult::Browser(result) => {
                let BrowserResult {
                    workspace_id,
                    panel_id,
                    browser_generation,
                    profile_id,
                    ..
                } = result.as_ref();
                let UiAction::CreateBrowser {
                    workspace_id: expected_workspace,
                    panel_id: expected_panel,
                    browser_generation: expected_generation,
                    profile_id: expected_profile,
                    ..
                } = &work.command.action
                else {
                    return Err(failure());
                };
                let browser = state.browsers.get(browser_generation).ok_or_else(failure)?;
                if !work.claimed
                    || workspace_id != expected_workspace
                    || panel_id != expected_panel
                    || browser_generation != expected_generation
                    || profile_id != expected_profile
                    || browser.owner != work.pairing
                    || !browser.control.authorized()
                    || !browser.control.started()
                    || !state.projection.panels.iter().any(|p| {
                        p.id == *panel_id && p.workspace_id == *workspace_id && p.kind == "browser"
                    })
                {
                    return Err(failure());
                }
                (OperationState::Succeeded, Effect::Complete)
            }
            OperationResult::ProjectClosure(result) => {
                let UiAction::CloseProject(command) = &work.command.action else {
                    return Err(failure());
                };
                if !command
                    .workspaces
                    .iter()
                    .all(|w| session.grant.workspace(&w.workspace_id).is_some())
                {
                    return Err(failure());
                }
                if result.closed == Some(false) {
                    if !work.claimed
                        || work.native_committed
                        || **result != Self::project_closure(command, &work.project, Some(false))
                    {
                        return Err(failure());
                    }
                    Self::validate_project_close(&state, &work.pairing, command)
                        .map_err(|_| failure())?;
                    (OperationState::Failed, Effect::Partial)
                } else {
                    if !work.native_committed
                        || **result != Self::project_closure(command, &work.project, Some(true))
                        || state.projection.workspaces.iter().any(|w| {
                            w.project_id == work.project || result.workspace_ids.contains(&w.id)
                        })
                        || state
                            .projection
                            .panels
                            .iter()
                            .any(|p| result.panel_ids.contains(&p.id))
                    {
                        return Err(failure());
                    }
                    (OperationState::Succeeded, Effect::Complete)
                }
            }
            OperationResult::WorkspaceClosure(result) => {
                let UiAction::CloseWorkspace(command) = &work.command.action else {
                    return Err(failure());
                };
                if result.closed == Some(false) {
                    if !work.claimed
                        || work.native_committed
                        || *result
                            != Self::workspace_closure(
                                command,
                                &work.project,
                                Some(false),
                                Some(false),
                            )
                    {
                        return Err(failure());
                    }
                    Self::validate_workspace_close(&state, &work.pairing, command)
                        .map_err(|_| failure())?;
                    (OperationState::Failed, Effect::Partial)
                } else {
                    let expected = Self::workspace_closure(
                        command,
                        &work.project,
                        Some(true),
                        Some(
                            !state
                                .projection
                                .workspaces
                                .iter()
                                .any(|w| w.project_id == work.project),
                        ),
                    );
                    if !work.native_committed
                        || *result != expected
                        || state
                            .projection
                            .workspaces
                            .iter()
                            .any(|w| w.id == command.workspace_id)
                        || state
                            .projection
                            .panels
                            .iter()
                            .any(|p| command.panels.iter().any(|old| old.panel_id == p.id))
                    {
                        return Err(failure());
                    }
                    (OperationState::Succeeded, Effect::Complete)
                }
            }
            OperationResult::PanelMoved(result) => {
                let UiAction::MovePanel(command) = &work.command.action else {
                    return Err(failure());
                };
                if matches!(command.movement, PanelMove::TransferTab { .. })
                    && !work.native_committed
                {
                    return Err(failure());
                }
                if !work.claimed || !Self::panel_move_completed(&state, command, result) {
                    return Err(failure());
                }
                (OperationState::Succeeded, Effect::Complete)
            }
            OperationResult::Panel {
                workspace_id,
                panel_id,
                focused,
                closed,
            } => {
                match &work.command.action {
                    UiAction::SelectWorkspace {
                        workspace_id: expected_workspace,
                        panel_id: expected_panel,
                        ..
                    }
                    | UiAction::FocusPanel {
                        workspace_id: expected_workspace,
                        panel_id: expected_panel,
                        ..
                    } => {
                        if !work.claimed
                            || workspace_id != expected_workspace
                            || panel_id != expected_panel
                            || !focused
                            || *closed
                            || state.projection.focused_panel_id.as_ref() != Some(panel_id)
                        {
                            return Err(failure());
                        }
                    }
                    UiAction::ClosePanel {
                        workspace_id: expected_workspace,
                        panel_id: expected_panel,
                        ..
                    } => {
                        if !work.native_committed
                            || workspace_id != expected_workspace
                            || panel_id != expected_panel
                            || *focused
                            || !closed
                            || state.projection.panels.iter().any(|p| p.id == *panel_id)
                        {
                            return Err(failure());
                        }
                    }
                    _ => return Err(failure()),
                }
                (OperationState::Succeeded, Effect::Complete)
            }
            OperationResult::Workspace { workspace_id, name } => {
                let (expected, expected_name) = match &work.command.action {
                    UiAction::RenameWorkspace { workspace_id, name } => (workspace_id, name),
                    UiAction::CreateWorkspace {
                        workspace_id,
                        tab_id,
                        name,
                        ..
                    } => {
                        if !state.projection.panels.iter().any(|p| {
                            p.id == *tab_id && p.workspace_id == *workspace_id && p.kind == "file"
                        }) {
                            return Err(failure());
                        }
                        (workspace_id, name)
                    }
                    _ => return Err(failure()),
                };
                if !work.claimed
                    || workspace_id != expected
                    || name != expected_name
                    || !state.projection.workspaces.iter().any(|w| {
                        &w.id == expected
                            && &w.name == expected_name
                            && w.project_id == work.project
                    })
                {
                    return Err(failure());
                }
                (OperationState::Succeeded, Effect::Complete)
            }
            OperationResult::Terminal {
                workspace_id,
                panel_id,
                terminal_session_id,
                ..
            } => {
                let UiAction::CreateTerminal {
                    workspace_id: expected_workspace,
                    panel_id: expected_panel,
                    terminal_session_id: expected_session,
                    ..
                } = &work.command.action
                else {
                    return Err(failure());
                };
                if !work.claimed
                    || workspace_id != expected_workspace
                    || panel_id != expected_panel
                    || terminal_session_id != expected_session
                    || !state
                        .terminals
                        .get(terminal_session_id)
                        .is_some_and(|t| t.started && t.owner == work.pairing)
                    || !state.projection.panels.iter().any(|p| {
                        p.id == *panel_id
                            && p.workspace_id == *workspace_id
                            && p.terminal_session_id.as_ref() == Some(terminal_session_id)
                    })
                {
                    return Err(failure());
                }
                (OperationState::Succeeded, Effect::Complete)
            }
            OperationResult::GitMutated(result) => {
                let UiAction::GitMutate(command) = &work.command.action else {
                    return Err(failure());
                };
                if !work.claimed
                    || !work.native_committed
                    || result.workspace_id != command.workspace_id
                    || result.repository_relative != command.input.repository_relative
                    || result.operation != command.input.operation
                {
                    return Err(failure());
                }
                let receipt = self
                    .store
                    .lock()
                    .map_err(|_| failure())?
                    .get(&work.pairing, &work.project, &ack.operation_id)
                    .map_err(|_| failure())?;
                if !matches!(receipt.result, Some(OperationResult::GitMutated(ref saved)) if saved == result)
                {
                    return Err(failure());
                }
                if result
                    .pull
                    .as_ref()
                    .is_some_and(|pull| pull.outcome != GitPullOutcome::Applied)
                {
                    (OperationState::Failed, Effect::Partial)
                } else {
                    (OperationState::Succeeded, Effect::Complete)
                }
            }
            OperationResult::FilesMutated(result) => {
                let UiAction::FilesMutate(command) = &work.command.action else {
                    return Err(failure());
                };
                if !work.claimed || !work.native_committed {
                    return Err(failure());
                }
                Self::validate_files_mutate(&state, &work.pairing, &command.input)
                    .map_err(|_| failure())?;
                let receipt = self
                    .store
                    .lock()
                    .map_err(|_| failure())?
                    .get(&work.pairing, &work.project, &ack.operation_id)
                    .map_err(|_| failure())?;
                if !matches!(receipt.result, Some(OperationResult::FilesMutated(ref saved)) if saved == result)
                {
                    return Err(failure());
                }
                (OperationState::Succeeded, Effect::Complete)
            }
            OperationResult::EditorSaved(result) => {
                let UiAction::EditorSave(command) = &work.command.action else {
                    return Err(failure());
                };
                if !work.claimed || !work.native_committed {
                    return Err(failure());
                }
                Self::validate_editor_save(&state, &work.pairing, &command.input)
                    .map_err(|_| failure())?;
                let receipt = self
                    .store
                    .lock()
                    .map_err(|_| failure())?
                    .get(&work.pairing, &work.project, &ack.operation_id)
                    .map_err(|_| failure())?;
                if !matches!(receipt.result, Some(OperationResult::EditorSaved(ref saved)) if saved == result)
                {
                    return Err(failure());
                }
                (OperationState::Succeeded, Effect::Complete)
            }
            OperationResult::GitOpened(result) => {
                let UiAction::GitOpen(command) = &work.command.action else {
                    return Err(failure());
                };
                let kind = match command.view {
                    GitView::Diff { .. } => "diff",
                    GitView::Commit { .. } => "commit",
                };
                if !work.claimed
                    || !work.native_committed
                    || work.git_view_revision.as_ref() != Some(&result.observation_revision)
                    || result.workspace_id != command.workspace_id
                    || result.repository_relative != command.repository_relative
                    || result.view != command.view
                    || !state.projection.panels.iter().any(|p| {
                        p.id == result.panel_id
                            && p.workspace_id == command.workspace_id
                            && p.kind == kind
                    })
                {
                    return Err(failure());
                }
                Self::project_file_access(&state, &work.pairing, &command.workspace_id)
                    .map_err(|_| failure())?;
                (OperationState::Succeeded, Effect::Complete)
            }
            OperationResult::EditorOpened(result) => {
                let UiAction::EditorOpen(command) = &work.command.action else {
                    return Err(failure());
                };
                if !work.claimed
                    || !work.native_committed
                    || work.editor_preview.is_some()
                    || result.presentation != command.presentation
                    || result.workspace_id != command.workspace_id
                    || result.relative_path != command.relative_path
                    || !valid_id(&result.document_id)
                    || editor_edits::revision_number(&result.document_id, &result.buffer_revision)
                        .is_none()
                    || result.disk_revision.len() != 64
                    || !result.disk_revision.bytes().all(|b| b.is_ascii_hexdigit())
                    || !state.projection.panels.iter().any(|p| {
                        p.id == result.panel_id
                            && p.workspace_id == command.workspace_id
                            && p.kind == "file"
                    })
                {
                    return Err(failure());
                }
                Self::project_file_access(&state, &work.pairing, &command.workspace_id)
                    .map_err(|_| failure())?;
                (OperationState::Succeeded, Effect::Complete)
            }
            OperationResult::EditorPreviewed(result) => {
                let UiAction::EditorOpen(command) = &work.command.action else {
                    return Err(failure());
                };
                let Some(prepared) = &work.editor_preview else {
                    return Err(failure());
                };
                if !work.claimed
                    || !work.native_committed
                    || result.workspace_id != prepared.workspace_id
                    || result.relative_path != prepared.relative_path
                    || result.disk_revision != prepared.disk_revision
                    || result.width != prepared.width
                    || result.height != prepared.height
                    || result.original_width != prepared.original_width
                    || result.original_height != prepared.original_height
                    || !state.projection.panels.iter().any(|p| {
                        p.id == result.panel_id
                            && p.workspace_id == command.workspace_id
                            && p.kind == "file"
                    })
                {
                    return Err(failure());
                }
                Self::project_file_access(&state, &work.pairing, &command.workspace_id)
                    .map_err(|_| failure())?;
                (OperationState::Succeeded, Effect::Complete)
            }
            OperationResult::EditorEdited(result) => {
                let UiAction::EditorEdits(command) = &work.command.action else {
                    return Err(failure());
                };
                let input = &command.input;
                if !work.claimed
                    || result.workspace_id != input.workspace_id
                    || result.panel_id != input.panel_id
                    || result.relative_path != input.relative_path
                    || result.document_id != input.document_id
                    || result.previous_buffer_revision != input.expected_buffer_revision
                    || result.disk_revision != input.expected_disk_revision
                    || usize::from(result.edit_count) != input.edits.len()
                    || !editor_edits::next_revision(
                        &input.document_id,
                        &input.expected_buffer_revision,
                        &result.buffer_revision,
                    )
                {
                    return Err(failure());
                }
                Self::validate_editor_edits(&state, &work.pairing, input).map_err(|_| failure())?;
                (OperationState::Succeeded, Effect::Complete)
            }
            OperationResult::AndroidLaunch(_)
            | OperationResult::AndroidInstall(_)
            | OperationResult::ArtifactImported { .. }
            | OperationResult::AndroidInput(_)
            | OperationResult::AndroidControl(_)
            | OperationResult::AndroidRuntime(_)
            | OperationResult::TerminalCommand { .. }
            | OperationResult::TerminalInterrupt { .. }
            | OperationResult::TerminalControl { .. } => return Err(failure()),
            OperationResult::Failure { code } => {
                let rejected_without_effect =
                    if let UiAction::InteractBrowser(command) = &work.command.action {
                        state
                            .browsers
                            .get(&command.browser_generation)
                            .and_then(|b| b.control.interaction_rejection(&ack.operation_id))
                            == Some(*code)
                    } else {
                        false
                    };
                if rejected_without_effect {
                    (OperationState::Failed, Effect::None)
                } else if (work.claimed
                    && matches!(
                        work.command.action,
                        UiAction::MovePanel(_)
                            | UiAction::CloseWorkspace(_)
                            | UiAction::CloseProject(_)
                    )
                    && *code == ErrorCode::OutcomeUnknown)
                    || work.native_committed
                    || (work.claimed
                        && matches!(
                            work.command.action,
                            UiAction::CreateTerminal { .. }
                                | UiAction::CreateBrowser { .. }
                                | UiAction::CreateAndroid { .. }
                        ))
                {
                    (OperationState::OutcomeUnknown, Effect::Unknown)
                } else {
                    (OperationState::Failed, Effect::None)
                }
            }
        };
        // Leases remain in native RAM, never in the durable receipt database.
        if let OperationResult::BrowserNavigation(result) = &mut ack.result {
            let UiAction::NavigateBrowser { wait_until, .. } = &work.command.action else {
                return Err(failure());
            };
            let observed = state
                .browsers
                .get(&result.browser_generation)
                .ok_or_else(failure)?
                .control
                .navigation_observation(&ack.operation_id, *wait_until)
                .map_err(|_| failure())?
                .ok_or_else(failure)?;
            result.navigation_id = observed.navigation_id;
            result.url = observed.url;
            result.committed = observed.committed;
            result.loaded = observed.loaded;
        }
        if let OperationResult::Browser(result) = &mut ack.result {
            let BrowserResult {
                browser_generation,
                navigation_id,
                lease_id,
                ready,
                engine,
                network_isolation,
                ..
            } = result.as_mut();
            *lease_id = None;
            *ready = true;
            *navigation_id = state
                .browsers
                .get(browser_generation)
                .ok_or_else(failure)?
                .control
                .navigation_id();
            *engine = "WKWebView".into();
            *network_isolation = "none".into();
        }
        if let OperationResult::Terminal {
            lease_id, ready, ..
        } = &mut ack.result
        {
            *lease_id = None;
            *ready = true;
        }
        let mut store = self.store.lock().map_err(|_| failure())?;
        // A rejected Settings preflight has no native effects. Its claim waits
        // for approval, but record_result intentionally excludes awaiting_user.
        // Return only this uncommitted failure to queued before recording it;
        // success still requires Settings approval and a native stored result.
        if matches!(work.command.action, UiAction::UpdateSettings(_))
            && !work.native_committed
            && matches!(ack.result, OperationResult::Failure { .. })
            && store
                .get(&work.pairing, &work.project, &ack.operation_id)
                .map_err(|_| failure())?
                .state
                == OperationState::AwaitingUser
        {
            store
                .transition(
                    &work.pairing,
                    &work.project,
                    &ack.operation_id,
                    OperationState::Queued,
                    Effect::None,
                    now(),
                )
                .map_err(|_| failure())?;
        }
        let save_recorded = matches!(
            work.command.action,
            UiAction::EditorSave(_)
                | UiAction::FilesMutate(_)
                | UiAction::GitMutate(_)
                | UiAction::OpenSettings(_)
                | UiAction::UpdateSettings(_)
        ) && matches!(
            store
                .get(&work.pairing, &work.project, &ack.operation_id)
                .map_err(|_| failure())?
                .result,
            Some(
                OperationResult::EditorSaved(_)
                    | OperationResult::FilesMutated(_)
                    | OperationResult::GitMutated(_)
                    | OperationResult::SettingsOpened(_)
                    | OperationResult::SettingsUpdated(_)
            )
        );
        let preserve_close = (matches!(work.command.action, UiAction::OpenProject(_))
            || work.native_committed
                && matches!(
                    work.command.action,
                    UiAction::CloseWorkspace(_) | UiAction::CloseProject(_)
                ))
            && matches!(ack.result, OperationResult::Failure { .. });
        let closed = match &ack.result {
            OperationResult::ProjectOpened(result) if result.opened == Some(true) => {
                Some(&ack.result)
            }
            OperationResult::WorkspaceClosure(result) if result.closed == Some(true) => {
                Some(&ack.result)
            }
            OperationResult::ProjectClosure(result) if result.closed == Some(true) => {
                Some(&ack.result)
            }
            _ => None,
        };
        if let Some(result) = closed {
            store
                .finish_project_lifecycle(
                    &work.pairing,
                    &work.project,
                    &ack.operation_id,
                    result,
                    now(),
                )
                .map_err(|_| failure())?;
        } else {
            if !save_recorded && !preserve_close {
                store
                    .record_result(&work.pairing, &work.project, &ack.operation_id, &ack.result)
                    .map_err(|_| failure())?;
            }
            store
                .transition(
                    &work.pairing,
                    &work.project,
                    &ack.operation_id,
                    next,
                    effect,
                    now(),
                )
                .map_err(|_| failure())?;
        }
        let opened_owner = work.pairing.clone();
        let created = if let UiAction::CreateWorkspace { workspace_id, .. } = &work.command.action {
            Some((
                work.pairing.clone(),
                work.project.clone(),
                workspace_id.clone(),
            ))
        } else {
            None
        };
        if let OperationResult::GitOpened(result) = &ack.result {
            let pairing = work.pairing.clone();
            state.git_panels.insert(
                result.panel_id.clone(),
                (pairing, result.workspace_id.clone()),
            );
        }
        if let Some(project) = opened_project {
            let session = state.sessions.get_mut(&opened_owner).ok_or_else(failure)?;
            session.view.project_ids.push(project.project_id.clone());
            session
                .view
                .workspace_ids
                .extend(project.workspaces.iter().cloned());
            session
                .grant
                .projects
                .insert(project.project_id.clone(), project);
        }
        if let Some((pairing, project, workspace)) = created {
            let session = state.sessions.get_mut(&pairing).ok_or_else(failure)?;
            if session
                .grant
                .projects
                .get_mut(&project)
                .ok_or_else(failure)?
                .workspaces
                .insert(workspace.clone())
            {
                session.view.workspace_ids.push(workspace);
            }
        }
        state.work.remove(&ack.operation_id);
        Ok(())
    }
    pub(super) fn cancel_operation(self: &Arc<Self>, id: &str, input: OperationInput) -> Reply {
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::AppUnavailable);
        };
        let Some(session) = state
            .sessions
            .get(id)
            .filter(|s| s.alive.load(Ordering::SeqCst))
        else {
            return error(ErrorCode::ControlRevoked);
        };
        let Ok(mut store) = self.store.lock() else {
            return error(ErrorCode::StorageUnavailable);
        };
        let (project, receipt) = match session.grant.operation(&store, id, &input.operation_id) {
            Ok(r) => r,
            Err(e) => return storage_error(e),
        };
        if !Self::receipt_workspace_authorized(&state, id, &receipt) {
            return error(ErrorCode::TargetNotFound);
        }
        let (next, effect) = match receipt.state {
            OperationState::Queued | OperationState::AwaitingUser => {
                (OperationState::Cancelled, Effect::None)
            }
            OperationState::Running => (OperationState::Cancelling, Effect::None),
            _ => return Self::operation_reply(receipt),
        };
        match store.transition(id, &project, &input.operation_id, next, effect, now()) {
            Ok(receipt) => {
                if let Some(job) = state.installs.get(&input.operation_id) {
                    job.permit.revoke();
                }
                if let Some(work) = state.work.get(&input.operation_id) {
                    work.native_permit.active.store(false, Ordering::SeqCst);
                    if let UiAction::CreateBrowser {
                        browser_generation, ..
                    } = &work.command.action
                    {
                        if let Some(browser) = state.browsers.get(browser_generation) {
                            browser.control.revoke();
                        }
                    }
                }
                if let Some(run) = state.runs.get_mut(&input.operation_id) {
                    run.cancelling = true;
                }
                if next == OperationState::Cancelling {
                    self.queue_terminal_cancel(&state, &input.operation_id);
                }
                if next == OperationState::Cancelled {
                    state.installs.remove(&input.operation_id);
                    state.runs.remove(&input.operation_id);
                    state.work.remove(&input.operation_id);
                    state.claims.remove(&input.operation_id);
                }
                Self::operation_reply(receipt)
            }
            Err(e) => storage_error(e),
        }
    }
}
