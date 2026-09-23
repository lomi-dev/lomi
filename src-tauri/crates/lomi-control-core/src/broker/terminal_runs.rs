use super::operations::storage_error;
use super::*;
use crate::{
    terminal::{Prompt, TerminalControl},
    terminal_io::WriteFailure,
};
use receipts::{Effect, State as OperationState};

pub type TerminalDispatch =
    Arc<dyn Fn(TerminalDispatchRequest) -> Result<(), WriteFailure> + Send + Sync>;
pub struct TerminalDispatchRequest {
    pub generation: String,
    pub lease: String,
    pub operation: String,
    pub command: String,
    pub interrupt: Option<String>,
    pub control: Arc<Mutex<TerminalControl>>,
    /// Resolve current authenticated peer PIDs and policy at each write boundary.
    pub permit: Arc<dyn Fn() -> Option<Vec<u32>> + Send + Sync>,
}
#[derive(Clone)]
pub(super) struct Run {
    pairing: String,
    project: String,
    workspace: String,
    panel: String,
    generation: String,
    control: Arc<Mutex<TerminalControl>>,
    deadline: Instant,
    pub(super) cancelling: bool,
    interrupt: Option<String>,
}
impl Broker {
    pub(super) fn transfer_terminal_runs(
        state: &mut State,
        owner: &str,
        source: &str,
        destination: &str,
        panels: &[PanelMoveIdentity],
    ) {
        for run in state.runs.values_mut() {
            if run.pairing == owner
                && run.workspace == source
                && panels.iter().any(|p| {
                    p.panel_id == run.panel
                        && p.terminal_session_id.as_ref() == Some(&run.generation)
                })
            {
                run.workspace = destination.into();
            }
        }
    }
    pub fn set_terminal_dispatch(&self, dispatch: TerminalDispatch) -> io::Result<()> {
        *self.terminal_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    pub(super) fn terminal_target<'a>(
        state: &'a State,
        id: &str,
        workspace: &str,
        panel: &str,
        generation: &str,
        scope: &str,
    ) -> Result<&'a terminals::OwnedTerminal, ErrorCode> {
        let session = state
            .sessions
            .get(id)
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .ok_or(ErrorCode::ControlRevoked)?;
        // Every identifier participates in visibility before capability checks.
        if session.grant.workspace(workspace).is_none()
            || !state
                .projection
                .workspaces
                .iter()
                .any(|w| w.id == workspace && session.grant.permits(w))
            || !state.projection.panels.iter().any(|p| {
                p.id == panel
                    && p.workspace_id == workspace
                    && p.terminal_session_id.as_deref() == Some(generation)
            })
        {
            return Err(ErrorCode::TargetNotFound);
        }
        let target = state
            .terminals
            .get(generation)
            .filter(|t| t.started && t.owner == id && t.workspace == workspace && t.panel == panel)
            .ok_or(ErrorCode::TargetNotFound)?;
        if !session.grant.scopes.contains(scope) {
            return Err(ErrorCode::ScopeDenied);
        }
        Ok(target)
    }
    pub(super) fn read_terminal(&self, id: &str, input: TerminalReadInput) -> Reply {
        if input.mode == TerminalReadMode::Screen {
            return self.read_screen(id, input);
        }
        if input.minimum_parsed_sequence.is_some() {
            return error(ErrorCode::RevisionConflict);
        }
        let Ok(state) = self.lock_state() else {
            return error(ErrorCode::AppUnavailable);
        };
        let target = match Self::terminal_target(
            &state,
            id,
            &input.workspace_id,
            &input.panel_id,
            &input.terminal_session_id,
            "terminal.read",
        ) {
            Ok(t) => t,
            Err(e) => return error(e),
        };
        let cursor = match input.cursor.map(|v| v.parse::<u64>()).transpose() {
            Ok(v) => v,
            Err(_) => return error(ErrorCode::CursorExpired),
        };
        let Ok(control) = target.control.lock() else {
            return error(ErrorCode::AppUnavailable);
        };
        if input.mode == TerminalReadMode::Raw {
            if input.operation_id.is_some() {
                return error(ErrorCode::RevisionConflict);
            }
            let (bytes, next, start, gap) =
                match control.read_raw(cursor, input.max_bytes.unwrap_or(16 * 1024) as usize) {
                    Ok(r) => r,
                    Err(e) => return error(e),
                };
            use base64::Engine;
            return Reply::ok(Data::TerminalRaw {
                workspace_id: input.workspace_id,
                panel_id: input.panel_id,
                terminal_session_id: input.terminal_session_id,
                base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                next_cursor: next.to_string(),
                available_from: start.to_string(),
                gap,
                stream_sequence: control.watermarks().0.to_string(),
            });
        }
        let command = if let Some(operation) = &input.operation_id {
            match control.command(operation) {
                Some(c) => Some(c.into()),
                None => return error(ErrorCode::TargetNotFound),
            }
        } else {
            None
        };
        let read = if input.mode == TerminalReadMode::Command {
            let Some(operation) = &input.operation_id else {
                return error(ErrorCode::TargetNotFound);
            };
            control.read_command(
                operation,
                cursor,
                input.max_bytes.unwrap_or(16 * 1024) as usize,
            )
        } else {
            control.read(cursor, input.max_bytes.unwrap_or(16 * 1024) as usize)
        };
        let output = match read {
            Ok(v) => v,
            Err(e) => return error(e),
        };
        Reply::ok(Data::TerminalOutput(Box::new(TerminalOutput {
            mode: input.mode,
            workspace_id: input.workspace_id,
            panel_id: input.panel_id,
            terminal_session_id: input.terminal_session_id,
            text: output.text,
            next_cursor: output.next_cursor.to_string(),
            available_from: output.available_from.to_string(),
            gap: output.gap,
            prompt: match control.prompt() {
                Prompt::Unknown => "unknown",
                Prompt::Ready => "ready",
                Prompt::Editing => "editing",
                Prompt::Running => "running",
            }
            .into(),
            lease_id: control.lease().map(str::to_owned),
            command,
            exited: control.exited,
            shell_exit_code: control.shell_exit_code,
            next_input_sequence: control.input_sequence().saturating_add(1).to_string(),
            stream_sequence: control.watermarks().0.to_string(),
            parsed_sequence: control.watermarks().1.to_string(),
        })))
    }
    pub(super) fn interrupt_terminal(
        self: &Arc<Self>,
        id: &str,
        input: TerminalInterruptInput,
    ) -> Reply {
        let target = input.operation_id;
        if !valid_id(&target) {
            return error(ErrorCode::TargetNotFound);
        }
        self.terminal_action(
            id,
            TerminalRunInput {
                workspace_id: input.workspace_id,
                panel_id: input.panel_id,
                terminal_session_id: input.terminal_session_id,
                lease_id: input.lease_id,
                command: String::new(),
                retry_epoch: input.retry_epoch,
                request_key: input.request_key,
            },
            Some(target),
        )
    }
    pub(super) fn run_terminal(self: &Arc<Self>, id: &str, input: TerminalRunInput) -> Reply {
        self.terminal_action(id, input, None)
    }
    fn terminal_action(
        self: &Arc<Self>,
        id: &str,
        input: TerminalRunInput,
        interrupt: Option<String>,
    ) -> Reply {
        if (interrupt.is_none() && input.command.trim().is_empty())
            || input.command.len() > 16 * 1024
            || input.command.chars().any(char::is_control)
            || !valid_id(&input.request_key)
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
        if session.grant.workspace(&input.workspace_id).is_none()
            || !state
                .projection
                .workspaces
                .iter()
                .any(|w| w.id == input.workspace_id && session.grant.permits(w))
        {
            return error(ErrorCode::TargetNotFound);
        }
        if !session.grant.scopes.contains("terminal.execute") {
            return error(ErrorCode::ScopeDenied);
        }
        if session.retry_epoch != input.retry_epoch {
            return error(ErrorCode::RetryWindowExpired);
        }
        if session.peer_pid.is_none() {
            return error(ErrorCode::ProtectedOriginTerminal);
        }
        let Some(root) = session.grant.workspace(&input.workspace_id) else {
            return error(ErrorCode::TargetNotFound);
        };
        let project = root.project_id.clone();
        let key = receipts::Key {
            pairing_id: id,
            project_id: &project,
            tool: if interrupt.is_some() {
                "lomi_terminal_interrupt"
            } else {
                "lomi_terminal_run"
            },
            retry_epoch: &input.retry_epoch,
            request_key: &input.request_key,
        };
        let hash = match receipts::fingerprint(
            &(&input, &interrupt),
            &receipts::Target {
                workspace_id: &input.workspace_id,
                resource_id: &input.panel_id,
                generation: &input.terminal_session_id,
                revision: &input.lease_id,
            },
        ) {
            Ok(h) => h,
            Err(e) => return storage_error(e),
        };
        // A retained terminal may have moved since dispatch. Receipt replay is
        // authorized by the original workspace; it cannot grant target access.
        match self.replay(&state, id, &key, hash) {
            Ok(Some(reply)) => return reply,
            Err(reply) => return *reply,
            Ok(None) => {}
        }
        let target = match Self::terminal_target(
            &state,
            id,
            &input.workspace_id,
            &input.panel_id,
            &input.terminal_session_id,
            "terminal.execute",
        ) {
            Ok(t) => t.clone(),
            Err(e) => return error(e),
        };
        let Ok(mut store) = self.store.lock() else {
            return error(ErrorCode::StorageUnavailable);
        };
        let reservation = match store.reserve(&key, hash, now()) {
            Ok(r) => r,
            Err(e) => return storage_error(e),
        };
        if !reservation.created {
            return Self::operation_reply(reservation.receipt);
        }
        let op = reservation.receipt.operation_id.clone();
        if let Err(e) = store.bind_workspace(id, &project, &op, &input.workspace_id) {
            return storage_error(e);
        }
        let dispatch = self.terminal_dispatch.lock().ok().and_then(|d| d.clone());
        let denied = if state.runs.len() >= 32 {
            Some(ErrorCode::ResourceExhausted)
        } else if dispatch.is_none() {
            Some(ErrorCode::HostUnqualified)
        } else if target
            .control
            .lock()
            .ok()
            .is_none_or(|c| c.lease() != Some(&input.lease_id))
        {
            Some(ErrorCode::ControlRevoked)
        } else {
            None
        };
        if let Some(code) = denied {
            if let Err(e) =
                store.record_result(id, &project, &op, &OperationResult::Failure { code })
            {
                return storage_error(e);
            }
            return match store.transition(
                id,
                &project,
                &op,
                OperationState::Failed,
                Effect::None,
                now(),
            ) {
                Ok(r) => Self::operation_reply(r),
                Err(e) => storage_error(e),
            };
        }
        let run = Run {
            pairing: id.into(),
            project,
            workspace: input.workspace_id.clone(),
            panel: input.panel_id.clone(),
            generation: input.terminal_session_id.clone(),
            control: target.control.clone(),
            deadline: Instant::now() + Duration::from_secs(30),
            cancelling: false,
            interrupt: interrupt.clone(),
        };
        state.runs.insert(op.clone(), run.clone());
        let receipt = match store.get(id, &run.project, &op) {
            Ok(r) => r,
            Err(e) => return storage_error(e),
        };
        drop(store);
        drop(state);
        let weak = Arc::downgrade(self);
        let permit_weak = weak.clone();
        let owner = id.to_string();
        let target_input = input.clone();
        let permit_op = op.clone();
        let permit: Arc<dyn Fn() -> Option<Vec<u32>> + Send + Sync> = Arc::new(move || {
            let broker = permit_weak.upgrade()?;
            let state = broker.writer_state()?;
            let active = state.runs.get(&permit_op)?;
            if active.cancelling || active.deadline <= Instant::now() {
                return None;
            }
            let target = Self::terminal_target(
                &state,
                &owner,
                &target_input.workspace_id,
                &target_input.panel_id,
                &target_input.terminal_session_id,
                "terminal.execute",
            )
            .ok()?;
            if bounded_lock(&target.control, || broker.check_policy(&state).is_ok())?.lease()
                != Some(&target_input.lease_id)
            {
                return None;
            }
            state.sessions.values().map(|s| s.peer_pid).collect()
        });
        let dispatch = dispatch.unwrap();
        self.spawn_background(async move {
            let Some(broker) = weak.upgrade() else { return };
            let start = {
                let Ok(state) = broker.lock_state() else {
                    return;
                };
                if !state.runs.contains_key(&op) || run.deadline <= Instant::now() {
                    return;
                }
                let Ok(mut store) = broker.store.lock() else {
                    return;
                };
                store.transition(
                    &run.pairing,
                    &run.project,
                    &op,
                    OperationState::Running,
                    Effect::None,
                    now(),
                )
            };
            if start.is_err() {
                broker.finish_run_failure(&op, ErrorCode::StorageUnavailable, 0);
                return;
            }
            let request = TerminalDispatchRequest {
                generation: input.terminal_session_id,
                lease: input.lease_id,
                operation: op.clone(),
                command: input.command,
                interrupt,
                control: target.control,
                permit,
            };
            let result = broker.spawn_worker(move || dispatch(request)).await;
            match result {
                Ok(Ok(())) => {
                    if run.interrupt.is_some() {
                        broker.finish_interrupt(&op);
                        return;
                    }
                }
                Ok(Err(e)) => {
                    broker.finish_run_failure(&op, e.code, e.written);
                    return;
                }
                Err(_) => {
                    broker.finish_run_failure(&op, ErrorCode::OutcomeUnknown, 1);
                    return;
                }
            }
            drop(broker);
            // Only active commands own an observation task; idle/off mode has no timer.
            let deadline = Instant::now() + Duration::from_secs(24 * 60 * 60);
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let Some(broker) = weak.upgrade() else { break };
                if broker.settle_run(&op) {
                    break;
                }
                if Instant::now() >= deadline {
                    broker.finish_run_failure(&op, ErrorCode::DeadlineExceeded, 1);
                    break;
                }
            }
        });
        Self::operation_reply(receipt)
    }
    fn finish_interrupt(&self, op: &str) {
        let Ok(mut state) = self.lock_state() else {
            return;
        };
        let Some(run) = state.runs.remove(op) else {
            return;
        };
        let Some(target_operation_id) = run.interrupt else {
            return;
        };
        let Ok(mut store) = self.store.lock() else {
            return;
        };
        let result = OperationResult::TerminalInterrupt {
            workspace_id: run.workspace,
            panel_id: run.panel,
            terminal_session_id: run.generation,
            target_operation_id,
            dispatched: true,
        };
        if store
            .record_result(&run.pairing, &run.project, op, &result)
            .is_ok()
        {
            let _ = store.transition(
                &run.pairing,
                &run.project,
                op,
                OperationState::Succeeded,
                Effect::Complete,
                now(),
            );
        }
    }
    fn finish_run_failure(&self, op: &str, code: ErrorCode, written: usize) {
        let Ok(mut state) = self.lock_state() else {
            return;
        };
        let Some(run) = state.runs.remove(op) else {
            return;
        };
        let Ok(mut store) = self.store.lock() else {
            return;
        };
        if store
            .record_result(
                &run.pairing,
                &run.project,
                op,
                &OperationResult::Failure { code },
            )
            .is_err()
        {
            return;
        }
        let (next, effect) = if written == 0 {
            (OperationState::Failed, Effect::None)
        } else {
            (OperationState::OutcomeUnknown, Effect::Unknown)
        };
        let _ = store.transition(&run.pairing, &run.project, op, next, effect, now());
    }
    fn settle_run(&self, op: &str) -> bool {
        let Ok(mut state) = self.lock_state() else {
            return true;
        };
        let Some(run) = state.runs.get(op) else {
            return true;
        };
        let Ok(control) = run.control.lock() else {
            return true;
        };
        let command = control.command(op).cloned();
        let completed = command.as_ref().is_some_and(|c| c.completed);
        let exit_code = command.as_ref().and_then(|c| c.exit_code);
        if !completed && !control.exited && control.lease().is_some() {
            return false;
        }
        let result = command.map(|c| OperationResult::TerminalCommand {
            workspace_id: run.workspace.clone(),
            panel_id: run.panel.clone(),
            terminal_session_id: run.generation.clone(),
            observation: Box::new((&c).into()),
        });
        drop(control);
        let Ok(mut store) = self.store.lock() else {
            return true;
        };
        if let Some(result) = result {
            if store
                .record_result(&run.pairing, &run.project, op, &result)
                .is_err()
            {
                return true;
            }
        }
        let (next, effect) = if completed && exit_code.is_some() {
            (
                if run.cancelling && exit_code == Some(130) {
                    OperationState::Cancelled
                } else if exit_code == Some(0) {
                    OperationState::Succeeded
                } else {
                    OperationState::Failed
                },
                Effect::Complete,
            )
        } else {
            (OperationState::OutcomeUnknown, Effect::Unknown)
        };
        if store
            .transition(&run.pairing, &run.project, op, next, effect, now())
            .is_err()
        {
            return true;
        }
        state.runs.remove(op);
        true
    }
    pub(super) fn queue_terminal_cancel(self: &Arc<Self>, state: &State, operation: &str) {
        let Some(run) = state
            .runs
            .get(operation)
            .filter(|r| r.interrupt.is_none())
            .cloned()
        else {
            return;
        };
        let Some(dispatch) = self.terminal_dispatch.lock().ok().and_then(|d| d.clone()) else {
            return;
        };
        let Some(lease) = run
            .control
            .lock()
            .ok()
            .and_then(|c| c.lease().map(str::to_owned))
        else {
            return;
        };
        let weak = Arc::downgrade(self);
        let op = operation.to_string();
        let owner = run.pairing.clone();
        let workspace = run.workspace.clone();
        let panel = run.panel.clone();
        let generation = run.generation.clone();
        let expected_lease = lease.clone();
        let deadline = Instant::now() + Duration::from_secs(5);
        let permit = Arc::new(move || {
            if Instant::now() >= deadline {
                return None;
            }
            let broker = weak.upgrade()?;
            let state = broker.writer_state()?;
            if !state.runs.get(&op)?.cancelling {
                return None;
            }
            let target = Self::terminal_target(
                &state,
                &owner,
                &workspace,
                &panel,
                &generation,
                "terminal.execute",
            )
            .ok()?;
            if bounded_lock(&target.control, || broker.check_policy(&state).is_ok())?.lease()
                != Some(&expected_lease)
            {
                return None;
            }
            state.sessions.values().map(|s| s.peer_pid).collect()
        });
        let request = TerminalDispatchRequest {
            generation: run.generation,
            lease,
            operation: operation.into(),
            command: String::new(),
            interrupt: Some(operation.into()),
            control: run.control,
            permit,
        };
        // The durable cancelling transition admits exactly one interrupt.
        drop(self.spawn_worker(move || {
            let _ = dispatch(request);
        }));
    }
    pub(super) fn end_runs(&self, state: &mut State, pairing: Option<&str>) {
        let ids: Vec<_> = state
            .runs
            .iter()
            .filter(|(_, r)| pairing.is_none_or(|p| p == r.pairing))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if let Some(run) = state.runs.remove(&id) {
                if let Ok(mut store) = self.store.lock() {
                    if let Ok(receipt) = store.get(&run.pairing, &run.project, &id) {
                        let (next, effect) = if receipt.state == OperationState::Queued {
                            (OperationState::Cancelled, Effect::None)
                        } else {
                            (OperationState::OutcomeUnknown, Effect::Unknown)
                        };
                        let _ =
                            store.transition(&run.pairing, &run.project, &id, next, effect, now());
                    }
                }
            }
        }
    }
}
