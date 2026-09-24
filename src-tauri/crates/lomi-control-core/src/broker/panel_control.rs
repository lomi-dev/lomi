use super::operations::storage_error;
use super::*;
use crate::terminal::TerminalControl;
use receipts::{Effect, State as OperationState};

pub type TerminalAttachDispatch = Arc<
    dyn Fn(&str, &Arc<Mutex<TerminalControl>>, &TerminalProfile, &[u32]) -> Result<(), ErrorCode>
        + Send
        + Sync,
>;
#[derive(Clone)]
pub(super) struct Claim {
    pairing: String,
    project: String,
    workspace: String,
    panel: String,
    generation: String,
    profile: TerminalProfile,
    title: String,
    pub(super) deadline: Instant,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingControlView {
    operation_id: String,
    client_label: String,
    workspace_id: String,
    panel_id: String,
    terminal_session_id: String,
    title: String,
    seconds_remaining: u64,
}
impl Claim {
    pub(super) fn view(&self, operation: &str, state: &State) -> PendingControlView {
        PendingControlView {
            operation_id: operation.into(),
            client_label: state
                .sessions
                .get(&self.pairing)
                .map(|s| s.view.client_label.clone())
                .unwrap_or_default(),
            workspace_id: self.workspace.clone(),
            panel_id: self.panel.clone(),
            terminal_session_id: self.generation.clone(),
            title: self.title.clone(),
            seconds_remaining: self
                .deadline
                .saturating_duration_since(Instant::now())
                .as_secs(),
        }
    }
}
impl Broker {
    pub fn set_terminal_attach_dispatch(&self, dispatch: TerminalAttachDispatch) -> io::Result<()> {
        *self
            .terminal_attach_dispatch
            .lock()
            .map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    pub(super) fn control_panel(self: &Arc<Self>, id: &str, input: PanelControlInput) -> Reply {
        if !valid_id(&input.request_key) {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
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
        if !session.grant.scopes.contains("terminal.execute") {
            return error(ErrorCode::ScopeDenied);
        }
        if session.retry_epoch != input.retry_epoch {
            return error(ErrorCode::RetryWindowExpired);
        }
        let Some(profile) = session.grant.terminal_profile.clone() else {
            return error(ErrorCode::HostUnqualified);
        };
        let project = workspace.project_id.clone();
        let hash = match receipts::fingerprint(
            &(&input, &workspace.project_path),
            &receipts::Target {
                workspace_id: &input.workspace_id,
                resource_id: &input.panel_id,
                generation: &input.terminal_session_id,
                revision: &profile.revision,
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
            tool: "lomi_panel_control",
        };
        match self.replay(&state, id, &key, hash) {
            Ok(Some(reply)) => return reply,
            Err(reply) => return *reply,
            _ => {}
        }
        let Some(panel) = state
            .projection
            .panels
            .iter()
            .find(|p| p.id == input.panel_id && p.workspace_id == input.workspace_id)
        else {
            return error(ErrorCode::TargetNotFound);
        };
        if panel.kind != "terminal" {
            return error(ErrorCode::UnsupportedCapability);
        }
        if panel.terminal_session_id.as_deref() != Some(&input.terminal_session_id) {
            return error(ErrorCode::StaleGeneration);
        }
        let owned = state
            .terminals
            .get(&input.terminal_session_id)
            .filter(|t| t.owner == id)
            .cloned();
        if input.action == PanelControlAction::Release && owned.is_none() {
            return error(ErrorCode::TargetNotFound);
        }
        let needs_approval = input.action == PanelControlAction::Claim
            && owned
                .as_ref()
                .is_none_or(|t| t.control.lock().ok().is_none_or(|c| c.lease().is_none()));
        if needs_approval && state.claims.len() >= 8 {
            return error(ErrorCode::ResourceExhausted);
        }
        let claim = Claim {
            pairing: id.into(),
            project: project.clone(),
            workspace: input.workspace_id.clone(),
            panel: input.panel_id.clone(),
            generation: input.terminal_session_id.clone(),
            profile,
            title: panel.title.clone(),
            deadline: Instant::now() + Duration::from_secs(120),
        };
        let Ok(mut store) = self.store.lock() else {
            return error(ErrorCode::StorageUnavailable);
        };
        let receipt = match store.reserve(&key, hash, now()) {
            Ok(r) => r,
            Err(e) => return storage_error(e),
        };
        if !receipt.created {
            return Self::terminal_receipt(&state, id, receipt.receipt);
        }
        let operation = receipt.receipt.operation_id;
        if let Err(e) = store.bind_workspace(id, &project, &operation, &input.workspace_id) {
            return storage_error(e);
        }
        if needs_approval {
            let receipt = match store.transition(
                id,
                &project,
                &operation,
                OperationState::AwaitingUser,
                Effect::None,
                now(),
            ) {
                Ok(r) => r,
                Err(e) => return storage_error(e),
            };
            state.claims.insert(operation.clone(), claim);
            let weak = Arc::downgrade(self);
            self.spawn_background(async move {
                tokio::time::sleep(Duration::from_secs(120)).await;
                if let Some(broker) = weak.upgrade() {
                    let worker = broker.clone();
                    drop(broker.spawn_worker(move || {
                        let _ = worker.decide_control(&operation, false);
                    }));
                }
            });
            return Self::operation_reply(receipt);
        }
        if let Err(e) = store.transition(
            id,
            &project,
            &operation,
            OperationState::Running,
            Effect::None,
            now(),
        ) {
            return storage_error(e);
        }
        if self.check_policy(&state).is_err() || !session.alive.load(Ordering::SeqCst) {
            return error(ErrorCode::ControlRevoked);
        }
        let target = owned.unwrap();
        if input.action == PanelControlAction::Release {
            if let Ok(mut control) = target.control.lock() {
                control.revoke();
            } else {
                return error(ErrorCode::AppUnavailable);
            }
        }
        let result = OperationResult::TerminalControl {
            workspace_id: input.workspace_id,
            panel_id: input.panel_id,
            terminal_session_id: input.terminal_session_id,
            lease_id: None,
            controlled: input.action == PanelControlAction::Claim,
        };
        if let Err(e) = store.record_result(id, &project, &operation, &result) {
            return storage_error(e);
        }
        match store.transition(
            id,
            &project,
            &operation,
            OperationState::Succeeded,
            Effect::Complete,
            now(),
        ) {
            Ok(r) => Self::terminal_receipt(&state, id, r),
            Err(e) => storage_error(e),
        }
    }
    pub fn decide_control(&self, operation: &str, approve: bool) -> io::Result<()> {
        let mut state = self.lock_state()?;
        let claim = state.claims.get(operation).cloned().ok_or_else(failure)?;
        let session = state
            .sessions
            .get(&claim.pairing)
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .ok_or_else(failure)?;
        let valid = claim.deadline > Instant::now()
            && session.grant.scopes.contains("terminal.execute")
            && session.grant.workspace(&claim.workspace).is_some()
            && state
                .projection
                .workspaces
                .iter()
                .any(|w| w.id == claim.workspace && session.grant.permits(w))
            && state.projection.panels.iter().any(|p| {
                p.id == claim.panel
                    && p.workspace_id == claim.workspace
                    && p.terminal_session_id.as_ref() == Some(&claim.generation)
            })
            && state
                .projection
                .qualified_terminal(&claim.profile.id)
                .is_some_and(|p| p.id == claim.profile.id && p.revision == claim.profile.revision);
        let mut store = self.store.lock().map_err(|_| failure())?;
        let current = store
            .get(&claim.pairing, &claim.project, operation)
            .map_err(|_| failure())?;
        if current.state != OperationState::AwaitingUser {
            state.claims.remove(operation);
            return Err(failure());
        }
        if !approve || !valid {
            store
                .transition(
                    &claim.pairing,
                    &claim.project,
                    operation,
                    OperationState::Cancelled,
                    Effect::None,
                    now(),
                )
                .map_err(|_| failure())?;
            state.claims.remove(operation);
            return Ok(());
        }
        let dispatch = self
            .terminal_attach_dispatch
            .lock()
            .ok()
            .and_then(|d| d.clone())
            .ok_or_else(failure)?;
        let peers = state
            .sessions
            .values()
            .map(|s| s.peer_pid)
            .collect::<Option<Vec<_>>>()
            .ok_or_else(failure)?;
        store
            .transition(
                &claim.pairing,
                &claim.project,
                operation,
                OperationState::Queued,
                Effect::None,
                now(),
            )
            .map_err(|_| failure())?;
        store
            .transition(
                &claim.pairing,
                &claim.project,
                operation,
                OperationState::Running,
                Effect::None,
                now(),
            )
            .map_err(|_| failure())?;
        self.check_policy(&state)?;
        let result = (|| {
            let mut monitor =
                TerminalControl::new(claim.pairing.clone(), claim.generation.clone())?;
            monitor.bind_authorization(self.authorization.clone(), state.policy_revision);
            monitor.bind_connection(session.alive.clone());
            let control = Arc::new(Mutex::new(monitor));
            dispatch(&claim.generation, &control, &claim.profile, &peers)?;
            Ok::<_, ErrorCode>(control)
        })();
        let (result, next, effect) = match result {
            Ok(control) => {
                state.terminals.insert(
                    claim.generation.clone(),
                    terminals::OwnedTerminal {
                        owner: claim.pairing.clone(),
                        workspace: claim.workspace.clone(),
                        panel: claim.panel.clone(),
                        control,
                        started: true,
                    },
                );
                (
                    OperationResult::TerminalControl {
                        workspace_id: claim.workspace,
                        panel_id: claim.panel,
                        terminal_session_id: claim.generation,
                        lease_id: None,
                        controlled: true,
                    },
                    OperationState::Succeeded,
                    Effect::Complete,
                )
            }
            Err(code) => (
                OperationResult::Failure { code },
                OperationState::Failed,
                Effect::None,
            ),
        };
        store
            .record_result(&claim.pairing, &claim.project, operation, &result)
            .map_err(|_| failure())?;
        store
            .transition(
                &claim.pairing,
                &claim.project,
                operation,
                next,
                effect,
                now(),
            )
            .map_err(|_| failure())?;
        state.claims.remove(operation);
        Ok(())
    }
    pub(super) fn end_claims(&self, state: &mut State, pairing: Option<&str>) {
        let ids: Vec<_> = state
            .claims
            .iter()
            .filter(|(_, c)| pairing.is_none_or(|p| p == c.pairing))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if let Some(c) = state.claims.remove(&id) {
                if let Ok(mut store) = self.store.lock() {
                    let _ = store.transition(
                        &c.pairing,
                        &c.project,
                        &id,
                        OperationState::Cancelled,
                        Effect::None,
                        now(),
                    );
                }
            }
        }
    }
}
