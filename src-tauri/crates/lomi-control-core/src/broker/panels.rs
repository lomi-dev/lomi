use super::operations::{storage_error, UiMutation};
use super::*;
use crate::terminal::TerminalControl;
pub type TerminalCloseDispatch = Arc<
    dyn Fn(&str, &Arc<Mutex<TerminalControl>>, &[u32], bool) -> Result<(), ErrorCode> + Send + Sync,
>;
pub type BrowserCloseDispatch =
    Arc<dyn Fn(Arc<crate::browser::BrowserControl>) -> Result<(), ErrorCode> + Send + Sync>;
impl Broker {
    pub(super) fn chat_panel_scope(
        state: &State,
        owner: &str,
        panel: &str,
        scope: &str,
    ) -> Result<(), ErrorCode> {
        let panel = state
            .projection
            .panels
            .iter()
            .find(|p| p.id == panel && p.kind == "chat")
            .ok_or(ErrorCode::TargetNotFound)?;
        Self::chat_access(state, owner, &panel.workspace_id)?;
        let grant = &state.sessions[owner].grant;
        if !grant.scopes.contains(scope)
            || panel
                .chat_conversation_id
                .as_ref()
                .is_none_or(|id| !grant.chat_conversations.contains(id))
        {
            return Err(ErrorCode::ScopeDenied);
        }
        Ok(())
    }
    pub(super) fn validate_panel_action(
        state: &State,
        owner: &str,
        action: &UiAction,
    ) -> Result<(), ErrorCode> {
        match action {
            UiAction::OpenChat(command) => {
                if let Some(panel) = state.projection.panels.iter().find(|p| {
                    p.workspace_id == command.workspace_id
                        && p.kind == "chat"
                        && p.chat_conversation_id.as_ref() == Some(&command.conversation_id)
                }) {
                    if panel.id != command.panel_id {
                        return Err(ErrorCode::RevisionConflict);
                    }
                    Self::validate_panel_action(
                        state,
                        owner,
                        &UiAction::FocusPanel {
                            workspace_id: command.workspace_id.clone(),
                            panel_id: panel.id.clone(),
                            tab_id: panel.tab_id.clone(),
                            terminal_session_id: None,
                            browser_generation: None,
                        },
                    )?;
                }
            }
            UiAction::OpenProject(command) => Self::validate_project_open(state, owner, command)?,
            UiAction::CloseProject(command) => Self::validate_project_close(state, owner, command)?,
            UiAction::CloseWorkspace(command) => {
                Self::validate_workspace_close(state, owner, command)?
            }
            UiAction::MovePanel(command) => Self::validate_panel_move(state, owner, command)?,
            UiAction::UploadBrowser(input) => {
                Self::upload_access(state, owner, input)?;
            }
            UiAction::DownloadBrowser(input) => {
                Self::download_access(state, owner, input)?;
            }
            UiAction::ExportArtifact(command) => {
                Self::validate_artifact_export(state, owner, &command.input)?;
            }
            UiAction::FilesMutate(command) => {
                Self::validate_files_mutate(state, owner, &command.input)?;
            }
            UiAction::EditorSave(command) => {
                Self::validate_editor_save(state, owner, &command.input)?;
            }
            UiAction::EditorOpen(command) => {
                Self::project_file_access(state, owner, &command.workspace_id)?
                    .open_file(&command.relative_path, 4 * 1024 * 1024)?;
            }
            UiAction::EditorEdits(command) => {
                Self::validate_editor_edits(state, owner, &command.input)?;
            }
            UiAction::AndroidLaunch(input) => {
                Self::android_app_access(
                    state,
                    owner,
                    &input.workspace_id,
                    &input.panel_id,
                    &input.device_id,
                    &input.generation,
                    &input.package_name,
                    "android.launch",
                )?;
            }
            UiAction::AndroidInput(input) => {
                Self::android_input_access(
                    state,
                    owner,
                    &input.workspace_id,
                    &input.panel_id,
                    &input.device_id,
                    &input.generation,
                    true,
                )?;
            }
            UiAction::AndroidInputControl(input) => {
                Self::android_input_access(
                    state,
                    owner,
                    &input.workspace_id,
                    &input.panel_id,
                    &input.device_id,
                    &input.generation,
                    input.action == PanelControlAction::Claim,
                )?;
            }
            _ => {}
        }
        if let UiAction::AndroidRuntime {
            workspace_id,
            panel_id,
            device_id,
            generation,
        } = action
        {
            Self::android_runtime_access(
                state,
                owner,
                workspace_id,
                panel_id,
                device_id,
                generation.as_deref(),
            )?;
        }
        if let UiAction::CreateAndroid { device_id, .. } = action {
            if !state
                .sessions
                .get(owner)
                .is_some_and(|s| s.grant.android_devices.contains(device_id))
            {
                return Err(ErrorCode::ScopeDenied);
            }
        }
        if let UiAction::SelectWorkspace {
            workspace_id,
            panel_id,
            tab_id,
            terminal_session_id,
            ..
        }
        | UiAction::FocusPanel {
            workspace_id,
            panel_id,
            tab_id,
            terminal_session_id,
            ..
        }
        | UiAction::ClosePanel {
            workspace_id,
            panel_id,
            tab_id,
            terminal_session_id,
            ..
        } = action
        {
            let panel = state
                .projection
                .panels
                .iter()
                .find(|p| {
                    p.id == *panel_id && p.workspace_id == *workspace_id && p.tab_id == *tab_id
                })
                .ok_or(ErrorCode::TargetNotFound)?;
            if panel.terminal_session_id != *terminal_session_id {
                return Err(ErrorCode::StaleGeneration);
            }
            let focus = matches!(
                action,
                UiAction::FocusPanel { .. } | UiAction::SelectWorkspace { .. }
            );
            if let UiAction::SelectWorkspace {
                browser_generation, ..
            }
            | UiAction::FocusPanel {
                browser_generation, ..
            }
            | UiAction::ClosePanel {
                browser_generation, ..
            } = action
            {
                if panel.browser_generation != *browser_generation {
                    return Err(ErrorCode::StaleGeneration);
                }
            }
            if matches!(action, UiAction::SelectWorkspace { .. })
                && !state
                    .projection
                    .workspaces
                    .iter()
                    .any(|w| w.id == *workspace_id && w.active_panel_id.as_ref() == Some(panel_id))
            {
                return Err(ErrorCode::RevisionConflict);
            }
            // Lazy shell/device/navigation effects need their native start tickets.
            // Focus reuses native owned browsers and never starts a lazy runtime.
            for p in state
                .projection
                .panels
                .iter()
                .filter(|p| p.tab_id == *tab_id)
            {
                if p.kind == "browser" && (focus || p.id == *panel_id) {
                    let generation = p
                        .browser_generation
                        .as_deref()
                        .ok_or(ErrorCode::UnsupportedCapability)?;
                    let target = Self::browser_target(
                        state,
                        owner,
                        workspace_id,
                        &p.id,
                        generation,
                        if focus { "panel.focus" } else { "panel.close" },
                    )?;
                    if !target.control.authorized() {
                        return Err(ErrorCode::ControlRevoked);
                    }
                } else if p.kind == "android" && (focus || p.id == *panel_id) {
                    Self::android_panel_access(state, owner, &p.id, focus)?;
                } else if p.kind == "android" {
                    continue;
                } else if p.kind == "chat" {
                    if focus {
                        Self::chat_panel_scope(state, owner, &p.id, "chat.open")?;
                    } else if p.id == *panel_id {
                        Self::closing_chats(state, owner, std::slice::from_ref(panel_id))?;
                    }
                } else if matches!(p.kind.as_str(), "diff" | "commit") {
                    if !state
                        .git_panels
                        .get(&p.id)
                        .is_some_and(|(pairing, workspace)| {
                            pairing == owner && workspace == workspace_id
                        })
                    {
                        return Err(ErrorCode::ScopeDenied);
                    }
                } else if !matches!(p.kind.as_str(), "file" | "terminal") {
                    return Err(ErrorCode::UnsupportedCapability);
                }
                if p.kind == "terminal" && p.terminal_session_id.is_none() {
                    return Err(ErrorCode::ScopeDenied);
                }
            }
        }
        Ok(())
    }
    pub(super) fn focus_panel(self: &Arc<Self>, id: &str, input: PanelMutationInput) -> Reply {
        self.panel_change(id, input, false)
    }
    pub(super) fn close_panel(self: &Arc<Self>, id: &str, input: PanelMutationInput) -> Reply {
        self.panel_change(id, input, true)
    }
    fn panel_change(self: &Arc<Self>, id: &str, input: PanelMutationInput, close: bool) -> Reply {
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
        let Some(workspace) = state
            .projection
            .workspaces
            .iter()
            .find(|w| w.id == input.workspace_id && session.grant.permits(w))
        else {
            return error(ErrorCode::TargetNotFound);
        };
        let project = workspace.project_id.clone();
        let hash = match receipts::fingerprint(
            &(&input, &workspace.project_path),
            &receipts::Target {
                workspace_id: &input.workspace_id,
                resource_id: &input.panel_id,
                generation: input
                    .terminal_session_id
                    .as_deref()
                    .unwrap_or(&state.projection.ui_epoch),
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
            tool: if close {
                "lomi_panel_close"
            } else {
                "lomi_panel_focus"
            },
        };
        match self.replay(&state, id, &key, hash) {
            Ok(Some(reply)) => return reply,
            Err(reply) => return *reply,
            Ok(None) => {}
        }
        let Some(panel) = state
            .projection
            .panels
            .iter()
            .find(|p| p.id == input.panel_id && p.workspace_id == input.workspace_id)
        else {
            return error(ErrorCode::TargetNotFound);
        };
        if !session
            .grant
            .scopes
            .contains(if close { "panel.close" } else { "panel.focus" })
        {
            return error(ErrorCode::ScopeDenied);
        }
        if input.retry_epoch != session.retry_epoch {
            return error(ErrorCode::RetryWindowExpired);
        }
        if input.terminal_session_id != panel.terminal_session_id
            || input.browser_generation != panel.browser_generation
        {
            return error(ErrorCode::StaleGeneration);
        }
        let action = if close {
            if !session.grant.scopes.contains("panel.create") {
                return error(ErrorCode::ScopeDenied);
            }
            if panel.kind == "terminal" {
                if !session.grant.scopes.contains("terminal.execute") {
                    return error(ErrorCode::ScopeDenied);
                }
                let Some(target) = panel
                    .terminal_session_id
                    .as_ref()
                    .and_then(|g| state.terminals.get(g))
                    .filter(|t| t.owner == id)
                else {
                    return error(ErrorCode::ProtectedOriginTerminal);
                };
                if target.control.lock().ok().is_none_or(|c| c.human_owned) {
                    return error(ErrorCode::ControlRevoked);
                }
                let Some(peers) = state
                    .sessions
                    .values()
                    .map(|s| s.peer_pid)
                    .collect::<Option<Vec<_>>>()
                else {
                    return error(ErrorCode::ProtectedOriginTerminal);
                };
                let Some(dispatch) = self
                    .terminal_close_dispatch
                    .lock()
                    .ok()
                    .and_then(|d| d.clone())
                else {
                    return error(ErrorCode::HostUnqualified);
                };
                if let Err(code) = dispatch(
                    panel.terminal_session_id.as_deref().unwrap(),
                    &target.control,
                    &peers,
                    false,
                ) {
                    return error(code);
                }
            }
            UiAction::ClosePanel {
                workspace_id: input.workspace_id.clone(),
                panel_id: input.panel_id.clone(),
                tab_id: panel.tab_id.clone(),
                terminal_session_id: panel.terminal_session_id.clone(),
                browser_generation: panel.browser_generation.clone(),
                replacement_tab_id: match new_id() {
                    Ok(id) => id,
                    Err(_) => return error(ErrorCode::ResourceExhausted),
                },
            }
        } else {
            UiAction::FocusPanel {
                workspace_id: input.workspace_id.clone(),
                panel_id: input.panel_id.clone(),
                tab_id: panel.tab_id.clone(),
                terminal_session_id: panel.terminal_session_id.clone(),
                browser_generation: panel.browser_generation.clone(),
            }
        };
        if let Err(e) = Self::validate_panel_action(&state, id, &action) {
            return error(e);
        }
        if close {
            if let Err(code) =
                self.preflight_android_close(&state, id, std::slice::from_ref(&input.panel_id))
            {
                return error(code);
            }
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
                tool: if close {
                    "lomi_panel_close"
                } else {
                    "lomi_panel_focus"
                },
                hash,
                action,
            },
        )
    }
    pub fn set_terminal_close_dispatch(&self, dispatch: TerminalCloseDispatch) -> io::Result<()> {
        *self.terminal_close_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    pub fn set_browser_close_dispatch(&self, dispatch: BrowserCloseDispatch) -> io::Result<()> {
        *self.browser_close_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    pub fn commit_panel_close(
        &self,
        operation: &str,
        nonce: &str,
        epoch: &str,
    ) -> Result<(), ErrorCode> {
        let ancestor_close = {
            let state = self.lock_state().map_err(|_| ErrorCode::AppUnavailable)?;
            state
                .work
                .get(operation)
                .and_then(|work| match work.command.action {
                    UiAction::CloseWorkspace(_) => Some(false),
                    UiAction::CloseProject(_) => Some(true),
                    _ => None,
                })
        };
        if let Some(project) = ancestor_close {
            return if project {
                self.commit_project_close(operation, nonce, epoch)
            } else {
                self.commit_workspace_close(operation, nonce, epoch)
            };
        }
        let mut state = self.lock_state().map_err(|_| ErrorCode::AppUnavailable)?;
        let work = state.work.get(operation).ok_or(ErrorCode::TargetNotFound)?;
        if !work.claimed
            || work.native_committed
            || work.deadline <= Instant::now()
            || work.command.nonce != nonce
            || work.command.ui_epoch != epoch
            || state.projection.ui_epoch != epoch
            || state.projection.revision != work.command.domain_revision
        {
            return Err(ErrorCode::ControlRevoked);
        }
        let UiAction::ClosePanel {
            workspace_id,
            panel_id,
            ..
        } = &work.command.action
        else {
            return Err(ErrorCode::ScopeDenied);
        };
        let session = state
            .sessions
            .get(&work.pairing)
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .ok_or(ErrorCode::ControlRevoked)?;
        if !Self::action_scopes(&work.command.action)
            .iter()
            .all(|s| session.grant.scopes.contains(*s))
            || session.grant.workspace(workspace_id).is_none()
            || !state
                .projection
                .workspaces
                .iter()
                .any(|w| w.id == *workspace_id && session.grant.permits(w))
        {
            return Err(ErrorCode::ScopeDenied);
        }
        Self::validate_android_layout(&state, work)?;
        Self::validate_panel_action(&state, &work.pairing, &work.command.action)?;
        let receipt = self
            .store
            .lock()
            .map_err(|_| ErrorCode::StorageUnavailable)?
            .get(&work.pairing, &work.project, operation)
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        if receipt.state != receipts::State::Running {
            return Err(ErrorCode::ControlRevoked);
        }
        self.check_policy(&state)
            .map_err(|_| ErrorCode::ControlRevoked)?;
        let panels = super::panel_move::identities(&state, workspace_id)
            .into_iter()
            .filter(|p| p.panel_id == *panel_id)
            .collect::<Vec<_>>();
        let steps = self.prepare_close_resources(&state, operation, &panels)?;
        state.work.get_mut(operation).unwrap().native_committed = true;
        drop(state);
        self.finish_close_resources(operation, steps)
    }
}
