use super::operations::{storage_error, UiMutation, Work};
use super::*;
use crate::project_files::ProjectDirectory;

pub(super) struct Approval {
    directory: Arc<ProjectDirectory>,
    scopes: HashSet<String>,
    approved: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingProjectOpenView {
    operation_id: String,
    client_label: String,
    project_path: String,
    workspace_name: String,
    scopes: Vec<String>,
    request_key: String,
    seconds_remaining: u64,
}
impl Broker {
    pub(super) fn project_opened(
        command: &ProjectOpenCommand,
        opened: Option<bool>,
    ) -> ProjectOpened {
        ProjectOpened {
            anchor_workspace_id: command.workspace_id.clone(),
            project_id: command.project_id.clone(),
            project_path: command.project_path.clone(),
            workspace_id: command.new_workspace_id.clone(),
            panel_id: command.tab_id.clone(),
            name: command.name.clone(),
            opened,
        }
    }
    pub(super) fn project_open_approval(
        state: &State,
        owner: &str,
        action: &UiAction,
    ) -> Result<Option<Approval>, ErrorCode> {
        let UiAction::OpenProject(command) = action else {
            return Ok(None);
        };
        Self::validate_project_open(state, owner, command)?;
        Ok(Some(Approval {
            directory: Arc::new(ProjectDirectory::open(Path::new(&command.project_path))?),
            scopes: state.sessions[owner].grant.scopes.clone(),
            approved: false,
        }))
    }
    pub(super) fn validate_project_open(
        state: &State,
        owner: &str,
        command: &ProjectOpenCommand,
    ) -> Result<(), ErrorCode> {
        let session = state
            .sessions
            .get(owner)
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .ok_or(ErrorCode::ControlRevoked)?;
        if session.grant.workspace(&command.workspace_id).is_none() {
            return Err(ErrorCode::TargetNotFound);
        }
        if !["project.open", "workspace.write", "panel.create"]
            .iter()
            .all(|s| session.grant.scopes.contains(*s))
        {
            return Err(ErrorCode::ScopeDenied);
        }
        let project_limit = if session.grant.yolo { 500 } else { 16 };
        if session.grant.projects.len() >= project_limit {
            return Err(ErrorCode::ResourceExhausted);
        }
        if session.grant.projects.contains_key(&command.project_id)
            || state.projection.workspaces.iter().any(|w| {
                w.project_id == command.project_id
                    || w.project_path == command.project_path
                    || w.id == command.new_workspace_id
            })
            || state
                .projection
                .panels
                .iter()
                .any(|p| p.id == command.tab_id || p.tab_id == command.tab_id)
        {
            return Err(ErrorCode::RevisionConflict);
        }
        Ok(())
    }
    pub(super) fn open_project(self: &Arc<Self>, owner: &str, input: ProjectOpenInput) -> Reply {
        if !valid_id(&input.request_key)
            || input.expected_revision.parse::<u64>().is_err()
            || input.name.trim().is_empty()
            || input.name.len() > 256
            || input.name.chars().any(char::is_control)
            || input.project_path.len() > 4096
            || !Path::new(&input.project_path).is_absolute()
            || input.project_path.chars().any(char::is_control)
        {
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
        if !["project.open", "workspace.write", "panel.create"]
            .iter()
            .all(|s| session.grant.scopes.contains(*s))
        {
            return error(ErrorCode::ScopeDenied);
        }
        let Some(anchor) = session.grant.workspace(&input.workspace_id) else {
            return error(ErrorCode::TargetNotFound);
        };
        if input.retry_epoch != session.retry_epoch {
            return error(ErrorCode::RetryWindowExpired);
        }
        let project = anchor.project_id.clone();
        // The original supplied path is part of retry identity. Canonicalization is
        // repeated only on cache miss, never to redirect an existing operation.
        let hash = match receipts::fingerprint(
            &(&input, &anchor.project_path),
            &receipts::Target {
                workspace_id: &input.workspace_id,
                resource_id: &format!("{:x}", Sha256::digest(input.project_path.as_bytes())),
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
            tool: "lomi_project_open",
            request_key: &input.request_key,
        };
        match self.replay(&state, owner, &key, hash) {
            Ok(Some(reply)) => return reply,
            Err(reply) => return *reply,
            Ok(None) => {}
        }
        let Ok(path) = fs::canonicalize(&input.project_path) else {
            return error(ErrorCode::TargetNotFound);
        };
        let Some(path) = path
            .to_str()
            .filter(|p| p.len() <= 4096 && !p.chars().any(char::is_control))
        else {
            return error(ErrorCode::TargetNotFound);
        };
        let (Ok(project_id), Ok(new_workspace_id), Ok(tab_id)) = (new_id(), new_id(), new_id())
        else {
            return error(ErrorCode::ResourceExhausted);
        };
        let command = ProjectOpenCommand {
            workspace_id: input.workspace_id.clone(),
            project_id,
            project_path: path.into(),
            new_workspace_id,
            tab_id,
            name: input.name,
            request_key: input.request_key.clone(),
            not_after_millis: (now().saturating_mul(1000) + 120_000).to_string(),
        };
        self.enqueue_ui(
            &mut state,
            owner,
            UiMutation {
                workspace: input.workspace_id,
                project,
                revision: input.expected_revision,
                retry_epoch: input.retry_epoch,
                request_key: input.request_key,
                tool: "lomi_project_open",
                hash,
                action: UiAction::OpenProject(command),
            },
        )
    }
    pub(super) fn pending_project_opens(
        state: &State,
        authorized: bool,
    ) -> Vec<PendingProjectOpenView> {
        state
            .work
            .values()
            .filter_map(|work| {
                let UiAction::OpenProject(command) = &work.command.action else {
                    return None;
                };
                let approval = work.project_open.as_ref()?;
                let session = state.sessions.get(&work.pairing)?;
                if !authorized
                    || !session.alive.load(Ordering::SeqCst)
                    || !work.claimed
                    || work.native_committed
                    || approval.approved
                    || work.native_permit.check().is_err()
                {
                    return None;
                }
                let mut scopes: Vec<_> = approval.scopes.iter().cloned().collect();
                scopes.sort();
                Some(PendingProjectOpenView {
                    operation_id: work.command.operation_id.clone(),
                    client_label: session.view.client_label.clone(),
                    project_path: command.project_path.clone(),
                    workspace_name: command.name.clone(),
                    scopes,
                    request_key: command.request_key.clone(),
                    seconds_remaining: work
                        .deadline
                        .saturating_duration_since(Instant::now())
                        .as_secs(),
                })
            })
            .collect()
    }
    fn project_open_work<'a>(
        state: &'a State,
        operation: &str,
        nonce: &str,
        epoch: &str,
    ) -> Result<&'a Work, ErrorCode> {
        let work = state.work.get(operation).ok_or(ErrorCode::TargetNotFound)?;
        if !work.claimed
            || work.native_committed
            || work.command.nonce != nonce
            || work.command.ui_epoch != epoch
            || state.projection.ui_epoch != epoch
        {
            return Err(ErrorCode::ControlRevoked);
        }
        work.native_permit.check()?;
        let UiAction::OpenProject(command) = &work.command.action else {
            return Err(ErrorCode::ScopeDenied);
        };
        Self::validate_project_open(state, &work.pairing, command)?;
        let approval = work.project_open.as_ref().ok_or(ErrorCode::ScopeDenied)?;
        if approval.scopes != state.sessions[&work.pairing].grant.scopes {
            return Err(ErrorCode::ControlRevoked);
        }
        approval.directory.check()?;
        Ok(work)
    }
    /// Called only by the trusted Settings adapter, never by the MCP client.
    pub fn decide_project_open(&self, operation: &str, approved: bool) -> Result<(), ErrorCode> {
        let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        self.check_policy(&state)
            .map_err(|_| ErrorCode::ControlRevoked)?;
        let work = state.work.get(operation).ok_or(ErrorCode::TargetNotFound)?;
        let work = Self::project_open_work(
            &state,
            operation,
            &work.command.nonce,
            &work.command.ui_epoch,
        )?;
        if work.project_open.as_ref().is_none_or(|a| a.approved) {
            return Err(ErrorCode::ControlRevoked);
        }
        if approved {
            if state.projection.revision != work.command.domain_revision {
                return Err(ErrorCode::RevisionConflict);
            }
            self.store
                .lock()
                .map_err(|_| ErrorCode::StorageUnavailable)?
                .transition(
                    &work.pairing,
                    &work.project,
                    operation,
                    receipts::State::Queued,
                    receipts::Effect::None,
                    now(),
                )
                .map_err(|_| ErrorCode::StorageUnavailable)?;
            state
                .work
                .get_mut(operation)
                .unwrap()
                .project_open
                .as_mut()
                .unwrap()
                .approved = true;
        } else {
            self.store
                .lock()
                .map_err(|_| ErrorCode::StorageUnavailable)?
                .transition(
                    &work.pairing,
                    &work.project,
                    operation,
                    receipts::State::Cancelled,
                    receipts::Effect::None,
                    now(),
                )
                .map_err(|_| ErrorCode::StorageUnavailable)?;
            state.work.remove(operation);
        }
        Ok(())
    }
    pub fn project_open_ready(
        &self,
        operation: &str,
        nonce: &str,
        epoch: &str,
    ) -> Result<bool, ErrorCode> {
        let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        self.check_policy(&state)
            .map_err(|_| ErrorCode::ControlRevoked)?;
        let work = Self::project_open_work(&state, operation, nonce, epoch)?;
        Ok(work.project_open.as_ref().is_some_and(|a| a.approved))
    }
    pub fn commit_project_open(
        &self,
        operation: &str,
        nonce: &str,
        epoch: &str,
    ) -> Result<(), ErrorCode> {
        let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        self.check_policy(&state)
            .map_err(|_| ErrorCode::ControlRevoked)?;
        let work = Self::project_open_work(&state, operation, nonce, epoch)?;
        if !work.project_open.as_ref().is_some_and(|a| a.approved) {
            return Err(ErrorCode::ScopeDenied);
        }
        if state.projection.revision != work.command.domain_revision {
            return Err(ErrorCode::RevisionConflict);
        }
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
        state.work.get_mut(operation).unwrap().native_committed = true;
        Ok(())
    }
    pub(super) fn project_open_completed(
        state: &State,
        work: &Work,
        result: &ProjectOpened,
    ) -> Result<ProjectGrant, ErrorCode> {
        let UiAction::OpenProject(command) = &work.command.action else {
            return Err(ErrorCode::ScopeDenied);
        };
        let approval = work.project_open.as_ref().ok_or(ErrorCode::ScopeDenied)?;
        work.native_permit.check()?;
        if !work.claimed
            || !work.native_committed
            || !approval.approved
            || *result != Self::project_opened(command, Some(true))
        {
            return Err(ErrorCode::ControlRevoked);
        }
        let session = state
            .sessions
            .get(&work.pairing)
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .ok_or(ErrorCode::ControlRevoked)?;
        let pregranted_yolo_project = session.grant.yolo
            && session
                .grant
                .projects
                .get(&command.project_id)
                .is_some_and(|project| {
                    project.project_path == command.project_path
                        && project.workspaces.contains(&command.new_workspace_id)
                });
        if (session.grant.yolo && session.grant.projects.len() >= 500 && !pregranted_yolo_project)
            || (!session.grant.yolo
                && (session.grant.projects.len() >= 16
                    || session.grant.projects.contains_key(&command.project_id)))
            || session.grant.scopes != approval.scopes
        {
            return Err(ErrorCode::ControlRevoked);
        }
        approval.directory.check()?;
        let workspaces: Vec<_> = state
            .projection
            .workspaces
            .iter()
            .filter(|w| {
                w.project_id == command.project_id
                    || w.project_path == command.project_path
                    || w.id == command.new_workspace_id
            })
            .collect();
        if workspaces.len() != 1
            || workspaces[0].id != command.new_workspace_id
            || workspaces[0].project_id != command.project_id
            || workspaces[0].project_path != command.project_path
            || workspaces[0].name != command.name
        {
            return Err(ErrorCode::RevisionConflict);
        }
        let panels: Vec<_> = state
            .projection
            .panels
            .iter()
            .filter(|p| {
                p.workspace_id == command.new_workspace_id
                    || p.id == command.tab_id
                    || p.tab_id == command.tab_id
            })
            .collect();
        if panels.len() != 1
            || panels[0].id != command.tab_id
            || panels[0].tab_id != command.tab_id
            || panels[0].workspace_id != command.new_workspace_id
            || panels[0].kind != "file"
            || panels[0].terminal_session_id.is_some()
            || panels[0].browser_generation.is_some()
            || panels[0].android_device_id.is_some()
        {
            return Err(ErrorCode::RevisionConflict);
        }
        Ok(ProjectGrant {
            project_id: command.project_id.clone(),
            project_path: command.project_path.clone(),
            project_directory: approval
                .scopes
                .contains("files.read")
                .then(|| approval.directory.clone()),
            workspaces: HashSet::from([command.new_workspace_id.clone()]),
        })
    }
}
