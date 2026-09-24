use super::operations::{storage_error, UiMutation};
use super::*;
use crate::terminal::TerminalControl;
use receipts::State as OperationState;

#[derive(Clone)]
pub(super) struct OwnedTerminal {
    pub owner: String,
    pub workspace: String,
    pub panel: String,
    pub control: Arc<Mutex<TerminalControl>>,
    pub started: bool,
}

impl Broker {
    pub(super) fn create_terminal(self: &Arc<Self>, id: &str, input: TerminalCreateInput) -> Reply {
        if input.title.trim().is_empty()
            || input.title.len() > 256
            || input.title.chars().any(char::is_control)
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
        if !["panel.create", "terminal.execute"]
            .iter()
            .all(|scope| session.grant.scopes.contains(*scope))
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
        let Some(profile) = session.grant.terminal_profile.clone() else {
            return error(ErrorCode::HostUnqualified);
        };
        if state
            .projection
            .qualified_terminal(&profile.id)
            .is_none_or(|p| p.revision != profile.revision)
        {
            return error(ErrorCode::ScopeDenied);
        }
        if input
            .profile_id
            .as_ref()
            .is_some_and(|id| id != &profile.id)
        {
            return error(ErrorCode::ScopeDenied);
        }
        let relative = Path::new(&input.cwd_relative);
        if relative.is_absolute()
            || input.cwd_relative.len() > 4096
            || input.cwd_relative.contains('\0')
            || relative.components().any(|c| {
                !matches!(
                    c,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )
            })
        {
            return error(ErrorCode::ScopeDenied);
        }
        let Ok(cwd) = fs::canonicalize(Path::new(&workspace.project_path).join(relative)) else {
            return error(ErrorCode::TargetNotFound);
        };
        if !cwd.starts_with(&workspace.project_path) || !cwd.is_dir() {
            return error(ErrorCode::ScopeDenied);
        }
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
        let hash = match receipts::fingerprint(&(&input, &project_path, &profile), &target) {
            Ok(hash) => hash,
            Err(e) => return storage_error(e),
        };
        let identity = || new_id().map_err(|_| ErrorCode::ResourceExhausted);
        let (Ok(panel_id), Ok(tab_id), Ok(terminal_session_id)) =
            (identity(), identity(), identity())
        else {
            return error(ErrorCode::ResourceExhausted);
        };
        let action = UiAction::CreateTerminal {
            workspace_id: input.workspace_id.clone(),
            panel_id,
            tab_id,
            terminal_session_id,
            profile_id: profile.id,
            cwd: cwd.to_string_lossy().into_owned(),
            title: input.title,
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
                tool: "lomi_terminal_create",
                hash,
                action,
            },
        )
    }
    /// Revalidate the native spawn ticket while holding the grant lock through
    /// process creation. No renderer can turn a stale ticket into a user start.
    pub fn start_terminal<T>(
        &self,
        operation: &str,
        nonce: &str,
        generation: &str,
        profile: &TerminalProfile,
        cwd: &str,
        start: impl FnOnce(Arc<Mutex<TerminalControl>>) -> Result<T, String>,
    ) -> Result<T, String> {
        let denied = || "Terminal authorization expired or changed.".to_string();
        let mut state = self.lock_state().map_err(|_| denied())?;
        let work = state.work.get(operation).ok_or_else(denied)?;
        let UiAction::CreateTerminal {
            panel_id,
            terminal_session_id,
            profile_id,
            cwd: expected_cwd,
            ..
        } = &work.command.action
        else {
            return Err(denied());
        };
        let session = state
            .sessions
            .get(&work.pairing)
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .ok_or_else(denied)?;
        let root = session
            .grant
            .workspace(&work.workspace)
            .ok_or_else(denied)?;
        if !work.claimed
            || work.deadline <= Instant::now()
            || work.command.nonce != nonce
            || work.command.ui_epoch != state.projection.ui_epoch
            || terminal_session_id != generation
            || profile_id != &profile.id
            || expected_cwd != cwd
            || fs::canonicalize(cwd).ok().is_none_or(|path| {
                path.to_string_lossy() != cwd || !path.starts_with(&root.project_path)
            })
            || session
                .grant
                .terminal_profile
                .as_ref()
                .is_none_or(|p| p.id != profile.id || p.revision != profile.revision)
            || state
                .projection
                .qualified_terminal(&profile.id)
                .is_none_or(|p| p.revision != profile.revision)
            || state.terminals.contains_key(generation)
            || !Self::action_scopes(&work.command.action)
                .iter()
                .all(|scope| session.grant.scopes.contains(*scope))
            || session.grant.workspace(&work.workspace).is_none()
            || !state
                .projection
                .workspaces
                .iter()
                .any(|w| w.id == work.workspace && session.grant.permits(w))
        {
            return Err(denied());
        }
        let receipt = self
            .store
            .lock()
            .map_err(|_| denied())?
            .get(&work.pairing, &work.project, operation)
            .map_err(|_| denied())?;
        if receipt.state != OperationState::Running {
            return Err(denied());
        }
        self.check_policy(&state).map_err(|_| denied())?;
        let mut monitor =
            TerminalControl::new(work.pairing.clone(), generation.into()).map_err(|_| denied())?;
        monitor.bind_authorization(self.authorization.clone(), state.policy_revision);
        monitor.bind_connection(session.alive.clone());
        let control = Arc::new(Mutex::new(monitor));
        let owned = OwnedTerminal {
            owner: work.pairing.clone(),
            workspace: work.workspace.clone(),
            panel: panel_id.clone(),
            control: control.clone(),
            started: false,
        };
        // Consuming the generation before spawning prevents retries after an
        // ambiguous native failure from starting a second process.
        state.terminals.insert(generation.into(), owned);
        state.work.get_mut(operation).unwrap().native_committed = true;
        let result = start(control);
        if result.is_ok() {
            state.terminals.get_mut(generation).unwrap().started = true;
        }
        result
    }
    pub fn reserved_terminal(&self, generation: &str) -> bool {
        self.lock_state().map(|state| state.terminals.contains_key(generation) || state.work.values().any(|w| matches!(&w.command.action, UiAction::CreateTerminal { terminal_session_id, .. } if terminal_session_id == generation))).unwrap_or(true)
    }
    pub(super) fn terminal_receipt(
        state: &State,
        owner: &str,
        mut receipt: receipts::Receipt,
    ) -> Reply {
        if let Some(
            OperationResult::WorkspaceClosure(_)
            | OperationResult::ProjectClosure(_)
            | OperationResult::ProjectOpened(_),
        ) = &receipt.result
        {
            if !Self::receipt_workspace_authorized(state, owner, &receipt) {
                return error(ErrorCode::ScopeDenied);
            }
        }
        if let Some(OperationResult::PanelMoved(result)) = &receipt.result {
            if let Some(destination) = &result.destination {
                if let Err(code) =
                    Self::panel_transfer_access(state, owner, &destination.workspace_id)
                {
                    return error(code);
                }
            }
            let scopes = if matches!(result.movement, PanelMove::ReorderTab { .. }) {
                &["workspace.write", "panel.move"][..]
            } else {
                &["workspace.write", "panel.move", "panel.focus"][..]
            };
            if !state.sessions.get(owner).is_some_and(|session| {
                scopes
                    .iter()
                    .all(|scope| session.grant.scopes.contains(*scope))
            }) {
                return error(ErrorCode::ScopeDenied);
            }
        }
        if let Some(OperationResult::BrowserUploaded(result)) = &receipt.result {
            if let Err(code) = Self::artifact_source_access(state, owner, &result.artifact) {
                return error(code);
            }
            match Self::browser_target(
                state,
                owner,
                &result.workspace_id,
                &result.panel_id,
                &result.browser_generation,
                "browser.upload",
            ) {
                Err(code) => return error(code),
                Ok(target) if !target.control.authorized() => {
                    return error(ErrorCode::ControlRevoked)
                }
                Ok(_) => {}
            }
        }
        if let Some(OperationResult::BrowserDownloaded(artifact)) = &receipt.result {
            if let Err(code) = Self::artifact_source_access(state, owner, artifact) {
                return error(code);
            }
        }
        if let Some(OperationResult::ArtifactExported(result)) = &receipt.result {
            if let Err(code) = Self::artifact_source_access(state, owner, &result.artifact) {
                return error(code);
            }
            if Self::project_file_access(state, owner, &result.workspace_id).is_err() {
                return error(ErrorCode::TargetNotFound);
            }
            if !["artifact.export", "files.mutate", "files.create"]
                .iter()
                .all(|scope| state.sessions[owner].grant.scopes.contains(*scope))
            {
                return error(ErrorCode::ScopeDenied);
            }
        }
        if let Some(OperationResult::FilesMutated(result)) = &receipt.result {
            if let Err(code) = Self::project_file_access(state, owner, &result.workspace_id) {
                return error(code);
            }
            if !state.sessions.get(owner).is_some_and(|s| {
                [
                    "files.mutate",
                    if result.new_path.is_none() {
                        "files.trash"
                    } else if result.old_path.is_some() {
                        "files.rename"
                    } else {
                        "files.create"
                    },
                ]
                .iter()
                .all(|scope| s.grant.scopes.contains(*scope))
            }) {
                return error(ErrorCode::ScopeDenied);
            }
        }
        if let Some(OperationResult::EditorSaved(result)) = &receipt.result {
            if let Err(code) = Self::project_file_access(state, owner, &result.workspace_id) {
                return error(code);
            }
            if !state.sessions.get(owner).is_some_and(|s| {
                ["files.mutate", "editor.read", "editor.write"]
                    .iter()
                    .all(|scope| s.grant.scopes.contains(*scope))
            }) {
                return error(ErrorCode::ScopeDenied);
            }
        }
        let editor_open_workspace = match &receipt.result {
            Some(OperationResult::EditorOpened(result)) => Some(&result.workspace_id),
            Some(OperationResult::EditorPreviewed(result)) => Some(&result.workspace_id),
            _ => None,
        };
        if let Some(workspace) = editor_open_workspace {
            if let Err(code) = Self::project_file_access(state, owner, workspace) {
                return error(code);
            }
            if !state
                .sessions
                .get(owner)
                .is_some_and(|s| s.grant.scopes.contains("editor.read"))
            {
                return error(ErrorCode::ScopeDenied);
            }
        }
        if let Some(OperationResult::EditorEdited(result)) = &receipt.result {
            if let Err(code) = Self::project_file_access(state, owner, &result.workspace_id) {
                return error(code);
            }
            if !state.sessions.get(owner).is_some_and(|s| {
                ["editor.read", "editor.write"]
                    .iter()
                    .all(|scope| s.grant.scopes.contains(*scope))
            }) {
                return error(ErrorCode::ScopeDenied);
            }
        }
        if let Some(OperationResult::ArtifactImported { workspace_id, .. }) = &receipt.result {
            if let Err(code) = Self::project_file_access(state, owner, workspace_id) {
                return error(code);
            }
        }
        if let Some(OperationResult::AndroidControl(result)) = &mut receipt.result {
            result.lease_id = if result.controlled {
                state
                    .android
                    .get(&result.device_id)
                    .filter(|t| t.owner == owner && t.workspaces.contains(&result.workspace_id))
                    .and_then(|t| t.control.input())
                    .filter(|l| l.generation == result.generation)
                    .map(|l| l.id.clone())
            } else {
                None
            };
        }
        if let Some(OperationResult::Browser(result)) = &mut receipt.result {
            let BrowserResult {
                workspace_id,
                panel_id,
                browser_generation,
                lease_id,
                ..
            } = result.as_mut();
            *lease_id = state
                .browsers
                .get(browser_generation)
                .filter(|b| {
                    b.owner == owner
                        && b.workspace == *workspace_id
                        && b.control.panel_id == *panel_id
                        && b.control.started()
                })
                .and_then(|b| b.control.lease().map(str::to_owned));
        }
        if let Some(
            OperationResult::Terminal {
                workspace_id,
                panel_id,
                terminal_session_id,
                lease_id,
                ..
            }
            | OperationResult::TerminalControl {
                workspace_id,
                panel_id,
                terminal_session_id,
                lease_id,
                controlled: true,
            },
        ) = &mut receipt.result
        {
            *lease_id = state
                .terminals
                .get(terminal_session_id)
                .filter(|t| {
                    t.started
                        && t.owner == owner
                        && t.workspace == *workspace_id
                        && t.panel == *panel_id
                })
                .and_then(|t| t.control.lock().ok()?.lease().map(str::to_owned));
        }
        Self::operation_reply(receipt)
    }
}
