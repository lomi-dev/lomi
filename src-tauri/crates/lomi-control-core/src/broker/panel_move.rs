use super::operations::{storage_error, UiMutation};
use super::*;

pub(super) fn identities(state: &State, workspace: &str) -> Vec<PanelMoveIdentity> {
    projection_identities(&state.projection, workspace)
}
pub(super) fn projection_identities(
    projection: &Projection,
    workspace: &str,
) -> Vec<PanelMoveIdentity> {
    let mut panels: Vec<_> = projection
        .panels
        .iter()
        .filter(|p| p.workspace_id == workspace)
        .map(|p| PanelMoveIdentity {
            panel_id: p.id.clone(),
            tab_id: p.tab_id.clone(),
            kind: p.kind.clone(),
            terminal_session_id: p.terminal_session_id.clone(),
            browser_generation: p.browser_generation.clone(),
            android_device_id: p.android_device_id.clone(),
        })
        .collect();
    panels.sort_by(|a, b| a.panel_id.cmp(&b.panel_id));
    panels
}
fn tab_order(state: &State, workspace: &str) -> Vec<String> {
    projection_tab_order(&state.projection, workspace)
}
pub(super) fn projection_tab_order(projection: &Projection, workspace: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for p in projection
        .panels
        .iter()
        .filter(|p| p.workspace_id == workspace)
    {
        if !ids.contains(&p.tab_id) {
            ids.push(p.tab_id.clone());
        }
    }
    ids
}
impl Broker {
    pub(super) fn validate_panel_move(
        state: &State,
        owner: &str,
        command: &PanelMoveCommand,
    ) -> Result<(), ErrorCode> {
        if identities(state, &command.workspace_id) != command.panels
            || tab_order(state, &command.workspace_id) != command.tab_order
            || state.projection.focused_panel_id != command.focused_panel_id
        {
            return Err(ErrorCode::RevisionConflict);
        }
        let panels = &command.panels;
        let focus = |panel: &PanelMoveIdentity| {
            Self::validate_panel_action(
                state,
                owner,
                &UiAction::FocusPanel {
                    workspace_id: command.workspace_id.clone(),
                    panel_id: panel.panel_id.clone(),
                    tab_id: panel.tab_id.clone(),
                    terminal_session_id: panel.terminal_session_id.clone(),
                    browser_generation: panel.browser_generation.clone(),
                },
            )
        };
        let visible = |tab: &str| {
            command
                .focused_panel_id
                .as_ref()
                .is_some_and(|id| panels.iter().any(|p| p.tab_id == tab && &p.panel_id == id))
        };
        match &command.movement {
            PanelMove::TransferTab { .. } => Self::validate_panel_transfer(state, owner, command)?,
            PanelMove::ReorderTab {
                tab_id,
                before_tab_id,
            } => {
                if !command.tab_order.contains(tab_id)
                    || before_tab_id
                        .as_ref()
                        .is_some_and(|id| !command.tab_order.contains(id) || id == tab_id)
                {
                    return Err(ErrorCode::TargetNotFound);
                }
            }
            PanelMove::DockTab {
                tab_id,
                target_tab_id,
                ..
            } => {
                if tab_id == target_tab_id {
                    return Err(ErrorCode::TargetNotFound);
                }
                let source = panels
                    .iter()
                    .find(|p| p.tab_id == *tab_id)
                    .ok_or(ErrorCode::TargetNotFound)?;
                let target = panels
                    .iter()
                    .find(|p| p.tab_id == *target_tab_id)
                    .ok_or(ErrorCode::TargetNotFound)?;
                if !visible(target_tab_id) {
                    return Err(ErrorCode::PanelNotRenderable);
                }
                if matches!(source.kind.as_str(), "diff" | "commit") {
                    return Err(ErrorCode::UnsupportedCapability);
                }
                focus(source)?;
                focus(target)?;
            }
            PanelMove::MovePane {
                panel_id,
                target_panel_id,
                ..
            } => {
                let source = panels
                    .iter()
                    .find(|p| p.panel_id == *panel_id)
                    .ok_or(ErrorCode::TargetNotFound)?;
                let target = panels
                    .iter()
                    .find(|p| p.panel_id == *target_panel_id)
                    .ok_or(ErrorCode::TargetNotFound)?;
                if source.panel_id == target.panel_id || source.tab_id != target.tab_id {
                    return Err(ErrorCode::TargetNotFound);
                }
                if !visible(&target.tab_id) {
                    return Err(ErrorCode::PanelNotRenderable);
                }
                focus(target)?;
            }
        }
        Ok(())
    }
    pub(super) fn panel_move_completed(
        state: &State,
        command: &PanelMoveCommand,
        result: &PanelMoved,
    ) -> bool {
        if matches!(command.movement, PanelMove::TransferTab { .. }) {
            return Self::panel_transfer_completed(&state.projection, command, result);
        }
        if command.destination.is_some() || result.destination.is_some() {
            return false;
        }
        if result.workspace_id != command.workspace_id || result.movement != command.movement {
            return false;
        }
        let mut expected = command.panels.clone();
        let mut order = command.tab_order.clone();
        let focused = match &command.movement {
            PanelMove::ReorderTab {
                tab_id,
                before_tab_id,
            } => {
                order.retain(|id| id != tab_id);
                let at = before_tab_id
                    .as_ref()
                    .and_then(|id| order.iter().position(|i| i == id))
                    .unwrap_or(order.len());
                order.insert(at, tab_id.clone());
                command.focused_panel_id.clone()
            }
            PanelMove::DockTab {
                tab_id,
                target_tab_id,
                ..
            } => {
                order.retain(|id| id != tab_id);
                for p in &mut expected {
                    if p.tab_id == *tab_id {
                        p.tab_id = target_tab_id.clone();
                    }
                }
                // The domain selects the moved tab's active pane. Any original source
                // pane is valid here; its exact identity is checked against publication.
                let actual = state.projection.focused_panel_id.clone();
                if !command
                    .panels
                    .iter()
                    .any(|p| p.tab_id == *tab_id && Some(&p.panel_id) == actual.as_ref())
                {
                    return false;
                }
                actual
            }
            PanelMove::MovePane { panel_id, .. } => Some(panel_id.clone()),
            PanelMove::TransferTab { .. } => return false,
        };
        result.panels == expected
            && identities(state, &command.workspace_id) == expected
            && tab_order(state, &command.workspace_id) == order
            && state.projection.focused_panel_id == focused
    }
    pub(super) fn move_panel(self: &Arc<Self>, id: &str, input: PanelMoveInput) -> Reply {
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
            || !session.grant.scopes.contains("panel.move")
            || (!matches!(input.movement, PanelMove::ReorderTab { .. })
                && !session.grant.scopes.contains("panel.focus"))
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
            tool: "lomi_panel_move",
        };
        match self.replay(&state, id, &key, hash) {
            Ok(Some(reply)) => return reply,
            Err(reply) => return *reply,
            Ok(None) => {}
        }
        let destination = if let PanelMove::TransferTab {
            target_workspace_id,
            ..
        } = &input.movement
        {
            if let Err(code) = Self::panel_transfer_access(&state, id, target_workspace_id) {
                return error(code);
            }
            Some(PanelMoveDestination {
                workspace_id: target_workspace_id.clone(),
                tab_order: tab_order(&state, target_workspace_id),
                panels: identities(&state, target_workspace_id),
            })
        } else {
            None
        };
        let command = PanelMoveCommand {
            workspace_id: input.workspace_id.clone(),
            movement: input.movement,
            panels: identities(&state, &input.workspace_id),
            tab_order: tab_order(&state, &input.workspace_id),
            focused_panel_id: state.projection.focused_panel_id.clone(),
            destination,
        };
        if command.panels.len() + command.destination.as_ref().map_or(0, |d| d.panels.len()) > 128
            || serde_json::to_vec(&command).map_or(true, |b| b.len() > 48 * 1024)
        {
            return error(ErrorCode::ResourceExhausted);
        }
        if let Err(e) = Self::validate_panel_move(&state, id, &command) {
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
                tool: "lomi_panel_move",
                hash,
                action: UiAction::MovePanel(command),
            },
        )
    }
}
