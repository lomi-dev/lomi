use super::operations::{storage_error, UiMutation};
use super::*;

impl Broker {
    pub(super) fn project_closure(
        command: &ProjectCloseCommand,
        project: &str,
        closed: Option<bool>,
    ) -> ProjectClosure {
        ProjectClosure {
            workspace_id: command.workspace_id.clone(),
            project_id: project.into(),
            workspace_ids: command
                .workspaces
                .iter()
                .map(|w| w.workspace_id.clone())
                .collect(),
            panel_ids: command
                .workspaces
                .iter()
                .flat_map(|w| w.panels.iter().map(|p| p.panel_id.clone()))
                .collect(),
            terminal_session_ids: command
                .workspaces
                .iter()
                .flat_map(|w| {
                    w.panels
                        .iter()
                        .filter_map(|p| p.terminal_session_id.clone())
                })
                .collect(),
            closed,
        }
    }
    pub(super) fn validate_project_close(
        state: &State,
        owner: &str,
        command: &ProjectCloseCommand,
    ) -> Result<(), ErrorCode> {
        let session = state
            .sessions
            .get(owner)
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .ok_or(ErrorCode::ControlRevoked)?;
        let root = session
            .grant
            .workspace(&command.workspace_id)
            .ok_or(ErrorCode::TargetNotFound)?;
        let mut actual: Vec<_> = state
            .projection
            .workspaces
            .iter()
            .filter(|w| w.project_id == root.project_id)
            .map(|w| (w.id.as_str(), w.project_path.as_str()))
            .collect();
        actual.sort_unstable();
        if actual.is_empty()
            || actual.len() != command.workspaces.len()
            || !command
                .workspaces
                .iter()
                .any(|w| w.workspace_id == command.workspace_id)
            || actual
                .iter()
                .zip(&command.workspaces)
                .any(|((id, path), w)| {
                    *id != w.workspace_id
                        || *path != root.project_path
                        || !root.workspaces.contains(*id)
                })
        {
            return Err(ErrorCode::TargetNotFound);
        }
        for workspace in &command.workspaces {
            Self::validate_workspace_close(state, owner, workspace)?;
        }
        Self::closing_chats(
            state,
            owner,
            &command
                .workspaces
                .iter()
                .flat_map(|w| w.panels.iter().map(|p| p.panel_id.clone()))
                .collect::<Vec<_>>(),
        )?;
        Ok(())
    }
    pub(super) fn close_project(self: &Arc<Self>, owner: &str, input: ProjectCloseInput) -> Reply {
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
        if ![
            "project.close",
            "workspace.close",
            "workspace.write",
            "panel.close",
        ]
        .iter()
        .all(|s| session.grant.scopes.contains(*s))
        {
            return error(ErrorCode::ScopeDenied);
        }
        let Some(root) = session.grant.workspace(&input.workspace_id) else {
            return error(ErrorCode::TargetNotFound);
        };
        if input.project_id != root.project_id
            || session.grant.workspace(&input.workspace_id).is_none()
        {
            return error(ErrorCode::TargetNotFound);
        }
        if input.retry_epoch != session.retry_epoch {
            return error(ErrorCode::RetryWindowExpired);
        }
        let project = root.project_id.clone();
        let hash = match receipts::fingerprint(
            &(&input, &root.project_path),
            &receipts::Target {
                workspace_id: &input.workspace_id,
                resource_id: &project,
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
            tool: "lomi_project_close",
        };
        match self.replay(&state, owner, &key, hash) {
            Ok(Some(reply)) => return reply,
            Err(reply) => return *reply,
            Ok(None) => {}
        }
        let expires = (now().saturating_mul(1000) + 120_000).to_string();
        let mut workspaces: Vec<_> = state
            .projection
            .workspaces
            .iter()
            .filter(|w| w.project_id == project)
            .map(|w| WorkspaceCloseCommand {
                workspace_id: w.id.clone(),
                panels: super::panel_move::identities(&state, &w.id),
                not_after_millis: expires.clone(),
            })
            .collect();
        workspaces.sort_by(|a, b| a.workspace_id.cmp(&b.workspace_id));
        let command = ProjectCloseCommand {
            workspace_id: input.workspace_id.clone(),
            workspaces,
            not_after_millis: expires,
        };
        if command.workspaces.len() > 128
            || command
                .workspaces
                .iter()
                .map(|w| w.panels.len())
                .sum::<usize>()
                > 128
            || serde_json::to_vec(&command).map_or(true, |v| v.len() > 48 * 1024)
        {
            return error(ErrorCode::ResourceExhausted);
        }
        if let Err(code) = Self::validate_project_close(&state, owner, &command) {
            return error(code);
        }
        let action = UiAction::CloseProject(command.clone());
        if !Self::action_scopes(&action)
            .iter()
            .all(|s| session.grant.scopes.contains(*s))
        {
            return error(ErrorCode::ScopeDenied);
        }
        for workspace in &command.workspaces {
            if let Err(code) = self.preflight_workspace_close(&state, owner, workspace) {
                return error(code);
            }
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
                tool: "lomi_project_close",
                hash,
                action,
            },
        )
    }
    pub(super) fn commit_project_close(
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
        let UiAction::CloseProject(command) = &work.command.action else {
            return Err(ErrorCode::ScopeDenied);
        };
        let session = state
            .sessions
            .get(&work.pairing)
            .ok_or(ErrorCode::ControlRevoked)?;
        if !Self::action_scopes(&work.command.action)
            .iter()
            .all(|s| session.grant.scopes.contains(*s))
        {
            return Err(ErrorCode::ScopeDenied);
        }
        Self::validate_project_close(&state, &work.pairing, command)?;
        for workspace in &command.workspaces {
            self.preflight_workspace_close(&state, &work.pairing, workspace)?;
        }
        let result = Self::project_closure(command, &work.project, None);
        let panels: Vec<_> = command
            .workspaces
            .iter()
            .flat_map(|w| w.panels.clone())
            .collect();
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
                    &OperationResult::ProjectClosure(result.into()),
                )
                .map_err(|_| ErrorCode::StorageUnavailable)?;
        }
        state.work.get_mut(operation).unwrap().native_committed = true;
        self.commit_close_resources(&state, operation, &panels)
    }
}
