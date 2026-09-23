use super::operations::{storage_error, UiMutation};
use super::*;

impl Broker {
    pub(super) fn select_workspace(
        self: &Arc<Self>,
        id: &str,
        input: WorkspaceSelectInput,
    ) -> Reply {
        if !valid_id(&input.request_key) || input.expected_revision.parse::<u64>().is_err() {
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
            || !session.grant.scopes.contains("panel.focus")
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
        if input.retry_epoch != session.retry_epoch {
            return error(ErrorCode::RetryWindowExpired);
        }
        let project = workspace.project_id.clone();
        let hash = match receipts::fingerprint(
            &(&input, &workspace.project_path),
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
            pairing_id: id,
            project_id: &project,
            retry_epoch: &input.retry_epoch,
            request_key: &input.request_key,
            tool: "lomi_workspace_update",
        };
        match self.replay(&state, id, &key, hash) {
            Ok(Some(reply)) => return reply,
            Err(reply) => return *reply,
            Ok(None) => {}
        }
        let Some(panel) = state.projection.panels.iter().find(|p| {
            Some(&p.id) == workspace.active_panel_id.as_ref() && p.workspace_id == workspace.id
        }) else {
            return error(ErrorCode::UiNotReady);
        };
        let action = UiAction::SelectWorkspace {
            workspace_id: input.workspace_id.clone(),
            panel_id: panel.id.clone(),
            tab_id: panel.tab_id.clone(),
            terminal_session_id: panel.terminal_session_id.clone(),
            browser_generation: panel.browser_generation.clone(),
        };
        if let Err(e) = Self::validate_panel_action(&state, id, &action) {
            return error(e);
        }
        self.enqueue_ui(
            &mut state,
            id,
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
}
