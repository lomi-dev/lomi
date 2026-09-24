use super::operations::{storage_error, UiMutation};
use super::*;

impl Broker {
    pub(super) fn receipt_workspace_authorized(
        state: &State,
        owner: &str,
        receipt: &receipts::Receipt,
    ) -> bool {
        let Some(session) = state
            .sessions
            .get(owner)
            .filter(|s| s.alive.load(Ordering::SeqCst))
        else {
            return false;
        };
        let Some(workspace) = &receipt.workspace_id else {
            return false;
        };
        let Some(root) = session.grant.workspace(workspace) else {
            return false;
        };
        if let Some(OperationResult::ProjectOpened(result)) = &receipt.result {
            return result.anchor_workspace_id == *workspace
                && ["project.open", "workspace.write", "panel.create"]
                    .iter()
                    .all(|s| session.grant.scopes.contains(*s));
        }
        if let Some(OperationResult::ProjectClosure(result)) = &receipt.result {
            return result.workspace_id == *workspace
                && result.project_id == root.project_id
                && !result.workspace_ids.is_empty()
                && result.workspace_ids.contains(workspace)
                && result
                    .workspace_ids
                    .iter()
                    .all(|id| root.workspaces.contains(id))
                && [
                    "project.close",
                    "workspace.write",
                    "workspace.close",
                    "panel.close",
                ]
                .iter()
                .all(|s| session.grant.scopes.contains(*s))
                && (result.terminal_session_ids.is_empty()
                    || session.grant.scopes.contains("terminal.execute"));
        }
        if let Some(OperationResult::WorkspaceClosure(result)) = &receipt.result {
            return result.workspace_id == *workspace
                && result.project_id == root.project_id
                && ["workspace.write", "workspace.close", "panel.close"]
                    .iter()
                    .all(|s| session.grant.scopes.contains(*s))
                && (result.terminal_session_ids.is_empty()
                    || session.grant.scopes.contains("terminal.execute"));
        }
        if matches!(receipt.result, Some(OperationResult::SettingsOpened(_)))
            && !session.grant.scopes.contains("settings.open")
        {
            return false;
        }
        if matches!(receipt.result, Some(OperationResult::SettingsUpdated(_)))
            && !["settings.read", "settings.write"]
                .iter()
                .all(|s| session.grant.scopes.contains(*s))
        {
            return false;
        }
        state
            .projection
            .workspaces
            .iter()
            .any(|w| w.id == *workspace && session.grant.permits(w))
    }
    pub(super) fn workspace_closure(
        command: &WorkspaceCloseCommand,
        project: &str,
        closed: Option<bool>,
        project_closed: Option<bool>,
    ) -> WorkspaceClosure {
        WorkspaceClosure {
            workspace_id: command.workspace_id.clone(),
            project_id: project.into(),
            panel_ids: command.panels.iter().map(|p| p.panel_id.clone()).collect(),
            terminal_session_ids: command
                .panels
                .iter()
                .filter_map(|p| p.terminal_session_id.clone())
                .collect(),
            closed,
            project_closed,
        }
    }
    pub(super) fn validate_workspace_close(
        state: &State,
        owner: &str,
        command: &WorkspaceCloseCommand,
    ) -> Result<(), ErrorCode> {
        if super::panel_move::identities(state, &command.workspace_id) != command.panels {
            return Err(ErrorCode::RevisionConflict);
        }
        for panel in &command.panels {
            match panel.kind.as_str() {
                "file" => {}
                "chat" => {
                    Self::chat_panel_scope(state, owner, &panel.panel_id, "chat.read")?;
                }
                "diff" | "commit" => {
                    if !state
                        .git_panels
                        .get(&panel.panel_id)
                        .is_some_and(|(pairing, workspace)| {
                            pairing == owner && workspace == &command.workspace_id
                        })
                    {
                        return Err(ErrorCode::ScopeDenied);
                    }
                }
                "terminal" => {
                    if let Some(generation) = &panel.terminal_session_id {
                        let target = state
                            .terminals
                            .get(generation)
                            .filter(|t| {
                                t.owner == owner
                                    && t.workspace == command.workspace_id
                                    && t.panel == panel.panel_id
                            })
                            .ok_or(ErrorCode::ProtectedOriginTerminal)?;
                        let control = target
                            .control
                            .try_lock()
                            .map_err(|_| ErrorCode::TargetBusy)?;
                        if control.human_owned || !control.authorized() {
                            return Err(ErrorCode::ControlRevoked);
                        }
                    }
                }
                "browser" => {
                    let generation = panel
                        .browser_generation
                        .as_deref()
                        .ok_or(ErrorCode::ControlRevoked)?;
                    let target = Self::browser_target(
                        state,
                        owner,
                        &command.workspace_id,
                        &panel.panel_id,
                        generation,
                        "panel.close",
                    )?;
                    if !target.control.authorized() {
                        return Err(ErrorCode::ControlRevoked);
                    }
                }
                _ => return Err(ErrorCode::UnsupportedCapability),
            }
        }
        Self::closing_chats(
            state,
            owner,
            &command
                .panels
                .iter()
                .map(|p| p.panel_id.clone())
                .collect::<Vec<_>>(),
        )?;
        Ok(())
    }
    pub(super) fn preflight_workspace_close(
        &self,
        state: &State,
        owner: &str,
        command: &WorkspaceCloseCommand,
    ) -> Result<(), ErrorCode> {
        Self::validate_workspace_close(state, owner, command)?;
        let peers = state
            .sessions
            .values()
            .map(|s| s.peer_pid)
            .collect::<Option<Vec<_>>>()
            .ok_or(ErrorCode::ProtectedOriginTerminal)?;
        for panel in &command.panels {
            if let Some(generation) = &panel.terminal_session_id {
                let target = state
                    .terminals
                    .get(generation)
                    .ok_or(ErrorCode::StaleGeneration)?;
                let dispatch = self
                    .terminal_close_dispatch
                    .lock()
                    .ok()
                    .and_then(|d| d.clone())
                    .ok_or(ErrorCode::HostUnqualified)?;
                dispatch(generation, &target.control, &peers, false)?;
            }
            if panel.browser_generation.is_some()
                && self
                    .browser_close_dispatch
                    .lock()
                    .ok()
                    .and_then(|d| d.clone())
                    .is_none()
            {
                return Err(ErrorCode::HostUnqualified);
            }
        }
        Ok(())
    }
    pub(super) fn close_workspace(
        self: &Arc<Self>,
        owner: &str,
        input: WorkspaceCloseInput,
    ) -> Reply {
        if !valid_id(&input.request_key) || input.expected_revision.parse::<u64>().is_err() {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::AppUnavailable);
        };
        let Some(session) = state
            .sessions
            .get(owner)
            .filter(|s| s.alive.load(Ordering::SeqCst))
        else {
            return error(ErrorCode::ControlRevoked);
        };
        if !["workspace.write", "workspace.close", "panel.close"]
            .iter()
            .all(|scope| session.grant.scopes.contains(*scope))
        {
            return error(ErrorCode::ScopeDenied);
        }
        if session.grant.workspace(&input.workspace_id).is_none() {
            return error(ErrorCode::TargetNotFound);
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
                resource_id: &input.workspace_id,
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
            tool: "lomi_workspace_update",
        };
        match self.replay(&state, owner, &key, hash) {
            Ok(Some(reply)) => return reply,
            Err(reply) => return *reply,
            Ok(None) => {}
        }
        if !state.projection.workspaces.iter().any(|w| {
            w.id == input.workspace_id
                && w.project_id == project
                && w.project_path == root.project_path
        }) {
            return error(ErrorCode::TargetNotFound);
        }
        let command = WorkspaceCloseCommand {
            workspace_id: input.workspace_id.clone(),
            panels: super::panel_move::identities(&state, &input.workspace_id),
            not_after_millis: (now().saturating_mul(1000) + 120_000).to_string(),
        };
        if command.panels.len() > 128
            || serde_json::to_vec(&command).map_or(true, |b| b.len() > 48 * 1024)
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let action = UiAction::CloseWorkspace(command.clone());
        if !Self::action_scopes(&action)
            .iter()
            .all(|s| session.grant.scopes.contains(*s))
        {
            return error(ErrorCode::ScopeDenied);
        }
        if let Err(e) = self.preflight_workspace_close(&state, owner, &command) {
            return error(e);
        }
        self.enqueue_ui(
            &mut state,
            owner,
            UiMutation {
                workspace: input.workspace_id,
                project,
                revision: input.expected_revision,
                retry_epoch: input.retry_epoch,
                request_key: input.request_key,
                tool: "lomi_workspace_update",
                hash,
                action,
            },
        )
    }
    pub fn workspace_close_pending(&self, operation: &str, nonce: &str, epoch: &str) -> bool {
        let Ok(state) = self.lock_state() else {
            return false;
        };
        let Some(work) = state.work.get(operation) else {
            return false;
        };
        if !work.claimed
            || work.native_committed
            || work.command.nonce != nonce
            || work.command.ui_epoch != epoch
            || state.projection.ui_epoch != epoch
            || state.projection.revision != work.command.domain_revision
            || !matches!(
                work.command.action,
                UiAction::CloseWorkspace(_) | UiAction::CloseProject(_)
            )
            || work.native_permit.check().is_err()
            || self.check_policy(&state).is_err()
        {
            return false;
        }
        state.sessions.get(&work.pairing).is_some_and(|session| {
            session.alive.load(Ordering::SeqCst)
                && Self::action_scopes(&work.command.action)
                    .iter()
                    .all(|s| session.grant.scopes.contains(*s))
        })
    }
    pub(super) fn commit_workspace_close(
        &self,
        operation: &str,
        nonce: &str,
        epoch: &str,
    ) -> Result<(), ErrorCode> {
        let mut state = self.lock_state().map_err(|_| ErrorCode::AppUnavailable)?;
        let work = state.work.get(operation).ok_or(ErrorCode::TargetNotFound)?;
        if !work.claimed
            || work.native_committed
            || work.command.nonce != nonce
            || work.command.ui_epoch != epoch
            || state.projection.ui_epoch != epoch
            || state.projection.revision != work.command.domain_revision
        {
            return Err(ErrorCode::ControlRevoked);
        }
        work.native_permit.check()?;
        self.check_policy(&state)
            .map_err(|_| ErrorCode::ControlRevoked)?;
        let UiAction::CloseWorkspace(command) = &work.command.action else {
            return Err(ErrorCode::ScopeDenied);
        };
        let session = state
            .sessions
            .get(&work.pairing)
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .ok_or(ErrorCode::ControlRevoked)?;
        if !Self::action_scopes(&work.command.action)
            .iter()
            .all(|scope| session.grant.scopes.contains(*scope))
            || session.grant.workspace(&command.workspace_id).is_none()
            || !state
                .projection
                .workspaces
                .iter()
                .any(|w| w.id == command.workspace_id && session.grant.permits(w))
        {
            return Err(ErrorCode::ScopeDenied);
        }
        self.preflight_workspace_close(&state, &work.pairing, command)?;
        let command = command.clone();
        {
            let mut store = self
                .store
                .lock()
                .map_err(|_| ErrorCode::StorageUnavailable)?;
            if store
                .get(&work.pairing, &work.project, operation)
                .map_err(|_| ErrorCode::StorageUnavailable)?
                .state
                != receipts::State::Running
            {
                return Err(ErrorCode::ControlRevoked);
            }
            store
                .record_result(
                    &work.pairing,
                    &work.project,
                    operation,
                    &OperationResult::WorkspaceClosure(Self::workspace_closure(
                        &command,
                        &work.project,
                        None,
                        None,
                    )),
                )
                .map_err(|_| ErrorCode::StorageUnavailable)?;
        }
        // From the first native close, cancellation cannot claim that every resource survived.
        state.work.get_mut(operation).unwrap().native_committed = true;
        self.commit_close_resources(&state, operation, &command.panels)
    }
    pub(super) fn commit_close_resources(
        &self,
        state: &State,
        operation: &str,
        panels: &[PanelMoveIdentity],
    ) -> Result<(), ErrorCode> {
        let work = state.work.get(operation).ok_or(ErrorCode::ControlRevoked)?;
        let ids = panels
            .iter()
            .map(|p| p.panel_id.clone())
            .collect::<Vec<_>>();
        self.close_chats(state, &work.pairing, &work.project, &ids, None)?;
        self.close_chats(
            state,
            &work.pairing,
            &work.project,
            &ids,
            Some(work.native_permit.clone()),
        )?;
        let peers = state
            .sessions
            .values()
            .map(|s| s.peer_pid)
            .collect::<Option<Vec<_>>>()
            .ok_or(ErrorCode::ProtectedOriginTerminal)?;
        for panel in panels {
            if let Some(generation) = &panel.browser_generation {
                let target = state
                    .browsers
                    .get(generation)
                    .ok_or(ErrorCode::OutcomeUnknown)?;
                self.browser_close_dispatch
                    .lock()
                    .ok()
                    .and_then(|d| d.clone())
                    .ok_or(ErrorCode::HostUnqualified)?(target.control.clone())?;
            }
            if let Some(generation) = &panel.terminal_session_id {
                let target = state
                    .terminals
                    .get(generation)
                    .ok_or(ErrorCode::OutcomeUnknown)?;
                self.terminal_close_dispatch
                    .lock()
                    .ok()
                    .and_then(|d| d.clone())
                    .ok_or(ErrorCode::HostUnqualified)?(
                    generation, &target.control, &peers, true
                )?;
            }
        }
        Ok(())
    }
}
