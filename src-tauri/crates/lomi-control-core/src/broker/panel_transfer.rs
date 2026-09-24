use super::panel_move::{identities, projection_identities, projection_tab_order};
use super::*;

impl Broker {
    pub(super) fn panel_transfer_access(
        state: &State,
        owner: &str,
        workspace: &str,
    ) -> Result<(), ErrorCode> {
        let session = state
            .sessions
            .get(owner)
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .ok_or(ErrorCode::ControlRevoked)?;
        if session.grant.workspace(workspace).is_none()
            || !state
                .projection
                .workspaces
                .iter()
                .any(|w| w.id == workspace && session.grant.permits(w))
        {
            return Err(ErrorCode::TargetNotFound);
        }
        if !["workspace.write", "panel.move", "panel.focus"]
            .iter()
            .all(|s| session.grant.scopes.contains(*s))
        {
            return Err(ErrorCode::ScopeDenied);
        }
        Ok(())
    }
    pub(super) fn validate_panel_transfer(
        state: &State,
        owner: &str,
        command: &PanelMoveCommand,
    ) -> Result<(), ErrorCode> {
        let PanelMove::TransferTab {
            tab_id,
            target_workspace_id,
            before_tab_id,
        } = &command.movement
        else {
            return Err(ErrorCode::ScopeDenied);
        };
        Self::panel_transfer_access(state, owner, &command.workspace_id)?;
        Self::panel_transfer_access(state, owner, target_workspace_id)?;
        let grant = &state.sessions[owner].grant;
        if grant
            .workspace(&command.workspace_id)
            .map(|p| &p.project_id)
            != grant.workspace(target_workspace_id).map(|p| &p.project_id)
        {
            return Err(ErrorCode::TargetNotFound);
        }
        let destination = command
            .destination
            .as_ref()
            .ok_or(ErrorCode::TargetNotFound)?;
        if command.workspace_id == *target_workspace_id
            || destination.workspace_id != *target_workspace_id
            || !command.tab_order.contains(tab_id)
            || destination.tab_order.contains(tab_id)
            || before_tab_id
                .as_ref()
                .is_some_and(|id| !destination.tab_order.contains(id))
        {
            return Err(ErrorCode::TargetNotFound);
        }
        if identities(state, target_workspace_id) != destination.panels
            || projection_tab_order(&state.projection, target_workspace_id) != destination.tab_order
        {
            return Err(ErrorCode::RevisionConflict);
        }
        for panel in command.panels.iter().filter(|p| p.tab_id == *tab_id) {
            match panel.kind.as_str() {
                "chat" => Self::chat_panel_scope(state, owner, &panel.panel_id, "chat.open")?,
                "file" => {}
                "android" => Self::android_panel_access(state, owner, &panel.panel_id, true)?,
                "terminal" => {
                    if let Some(generation) = &panel.terminal_session_id {
                        let target = Self::terminal_target(
                            state,
                            owner,
                            &command.workspace_id,
                            &panel.panel_id,
                            generation,
                            "panel.move",
                        )?;
                        let control = target
                            .control
                            .try_lock()
                            .map_err(|_| ErrorCode::TargetBusy)?;
                        if !control.authorized() || control.human_owned {
                            return Err(ErrorCode::ControlRevoked);
                        }
                    }
                }
                "browser" => {
                    let generation = panel
                        .browser_generation
                        .as_deref()
                        .ok_or(ErrorCode::UnsupportedCapability)?;
                    let target = Self::browser_target(
                        state,
                        owner,
                        &command.workspace_id,
                        &panel.panel_id,
                        generation,
                        "panel.move",
                    )?;
                    if !target.control.authorized() || !target.control.started() {
                        return Err(ErrorCode::ControlRevoked);
                    }
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
                _ => return Err(ErrorCode::UnsupportedCapability),
            }
        }
        Ok(())
    }
    fn transfer_result(command: &PanelMoveCommand) -> Option<PanelMoved> {
        let PanelMove::TransferTab {
            tab_id,
            before_tab_id,
            ..
        } = &command.movement
        else {
            return None;
        };
        let mut destination = command.destination.clone()?;
        let at = before_tab_id
            .as_ref()
            .and_then(|id| destination.tab_order.iter().position(|i| i == id))
            .unwrap_or(destination.tab_order.len());
        destination.tab_order.insert(at, tab_id.clone());
        destination.panels.extend(
            command
                .panels
                .iter()
                .filter(|p| p.tab_id == *tab_id)
                .cloned(),
        );
        destination
            .panels
            .sort_by(|a, b| a.panel_id.cmp(&b.panel_id));
        Some(PanelMoved {
            workspace_id: command.workspace_id.clone(),
            movement: command.movement.clone(),
            panels: command
                .panels
                .iter()
                .filter(|p| p.tab_id != *tab_id)
                .cloned()
                .collect(),
            destination: Some(destination),
        })
    }
    pub(super) fn panel_transfer_completed(
        projection: &Projection,
        command: &PanelMoveCommand,
        result: &PanelMoved,
    ) -> bool {
        if super::panel_move::chat_bindings(
            projection,
            &command.workspace_id,
            command
                .destination
                .as_ref()
                .map(|d| d.workspace_id.as_str()),
        ) != command.chat_bindings
        {
            return false;
        }
        let Some(expected) = Self::transfer_result(command) else {
            return false;
        };
        if *result != expected {
            return false;
        }
        let PanelMove::TransferTab { tab_id, .. } = &command.movement else {
            return false;
        };
        let destination = expected.destination.as_ref().unwrap();
        let source_order: Vec<_> = command
            .tab_order
            .iter()
            .filter(|id| *id != tab_id)
            .cloned()
            .collect();
        let focused =
            if command.panels.iter().any(|p| {
                p.tab_id == *tab_id && Some(&p.panel_id) == command.focused_panel_id.as_ref()
            }) {
                None
            } else {
                command.focused_panel_id.clone()
            };
        projection_identities(projection, &command.workspace_id) == expected.panels
            && projection_tab_order(projection, &command.workspace_id) == source_order
            && projection_identities(projection, &destination.workspace_id) == destination.panels
            && projection_tab_order(projection, &destination.workspace_id) == destination.tab_order
            && projection.focused_panel_id == focused
    }
    pub(super) fn apply_panel_transfers(state: &mut State, projection: &Projection) {
        // Migration is admitted only by a live claimed command and its exact publication.
        // Ordinary human/domain moves keep the existing detach/revoke behavior.
        let transfers: Vec<_> = state
            .work
            .iter()
            .filter_map(|(id, work)| {
                let UiAction::MovePanel(command) = &work.command.action else {
                    return None;
                };
                let PanelMove::TransferTab {
                    tab_id,
                    target_workspace_id,
                    ..
                } = &command.movement
                else {
                    return None;
                };
                if !work.claimed
                    || work.native_committed
                    || work.command.domain_revision != state.projection.revision
                    || work.native_permit.check().is_err()
                    || Self::validate_android_layout(state, work).is_err()
                    || Self::validate_panel_move(state, &work.pairing, command).is_err()
                {
                    return None;
                }
                let session = state.sessions.get(&work.pairing)?;
                for workspace in [&command.workspace_id, target_workspace_id] {
                    if !projection
                        .workspaces
                        .iter()
                        .any(|w| &w.id == workspace && session.grant.permits(w))
                    {
                        return None;
                    }
                }
                let expected = Self::transfer_result(command)?;
                if !Self::panel_transfer_completed(projection, command, &expected) {
                    return None;
                }
                Some((
                    id.clone(),
                    work.pairing.clone(),
                    command.workspace_id.clone(),
                    target_workspace_id.clone(),
                    command
                        .panels
                        .iter()
                        .filter(|p| &p.tab_id == tab_id)
                        .cloned()
                        .collect::<Vec<_>>(),
                ))
            })
            .collect();
        for (id, owner, source, destination, panels) in transfers {
            for panel in &panels {
                if let Some(target) = panel
                    .android_device_id
                    .as_ref()
                    .and_then(|id| state.android.get_mut(id))
                {
                    target.control.revoke_input();
                    target.workspaces.insert(destination.clone());
                    if !projection.panels.iter().any(|p| {
                        p.workspace_id == source
                            && p.android_device_id.as_ref() == Some(&target.control.device)
                    }) {
                        target.workspaces.remove(&source);
                    }
                }
                if let Some(target) = panel
                    .terminal_session_id
                    .as_ref()
                    .and_then(|id| state.terminals.get_mut(id))
                {
                    target.workspace.clone_from(&destination);
                }
                if let Some(target) = panel
                    .browser_generation
                    .as_ref()
                    .and_then(|id| state.browsers.get_mut(id))
                {
                    target.workspace.clone_from(&destination);
                }
                if let Some((pairing, workspace)) = state.git_panels.get_mut(&panel.panel_id) {
                    if pairing == &owner && workspace == &source {
                        workspace.clone_from(&destination);
                    }
                }
            }
            Self::transfer_terminal_runs(state, &owner, &source, &destination, &panels);
            if let Some(work) = state.work.get_mut(&id) {
                work.native_committed = true;
            }
        }
    }
}
