use super::{
    operations::{storage_error, UiMutation},
    *,
};
use crate::{
    android::AndroidControl,
    android_input::{InputLease, Reservation},
};

pub enum AndroidInputDispatchAction {
    Claim,
    Release,
    Send {
        lease: Arc<InputLease>,
        sequence: u64,
        event: AndroidInputEvent,
    },
}
pub struct AndroidInputDispatch {
    pub control: Arc<AndroidControl>,
    pub permit: NativePermit,
    pub workspace: String,
    pub panel: String,
    pub generation: String,
    pub action: AndroidInputDispatchAction,
}
impl Broker {
    pub(super) fn android_input_access(
        state: &State,
        owner: &str,
        workspace: &str,
        panel: &str,
        device: &str,
        generation: &str,
        selected: bool,
    ) -> Result<Arc<AndroidControl>, ErrorCode> {
        Self::android_runtime_access(state, owner, workspace, panel, device, Some(generation))?;
        if !state.sessions[owner]
            .grant
            .scopes
            .contains("android.interact")
        {
            return Err(ErrorCode::ScopeDenied);
        }
        let control = state.android[device].control.clone();
        if selected {
            control.check_selected(panel)?;
        }
        Ok(control)
    }
    fn input_fingerprint(input: &AndroidInput) -> Result<[u8; 32], ErrorCode> {
        receipts::fingerprint(
            &(&input.event, &input.lease_id, &input.input_sequence),
            &receipts::Target {
                workspace_id: &input.workspace_id,
                resource_id: &input.device_id,
                generation: &input.generation,
                revision: &input.panel_id,
            },
        )
        .map_err(|_| ErrorCode::StorageUnavailable)
    }
    fn input_sequence(input: &AndroidInput) -> Result<u64, ErrorCode> {
        let sequence = input
            .input_sequence
            .parse::<u64>()
            .map_err(|_| ErrorCode::RevisionConflict)?;
        if sequence == 0 || sequence >= 1 << 53 || sequence.to_string() != input.input_sequence {
            return Err(ErrorCode::RevisionConflict);
        }
        Ok(sequence)
    }
    pub(super) fn android_work_lease(state: &State, action: &UiAction) -> Option<Arc<InputLease>> {
        let UiAction::AndroidInput(input) = action else {
            return None;
        };
        state.android.get(&input.device_id)?.control.input()
    }
    pub(super) fn reserve_android_input(
        state: &State,
        action: &UiAction,
        operation: &str,
    ) -> Result<(), ErrorCode> {
        let UiAction::AndroidInput(input) = action else {
            return Ok(());
        };
        let lease = Self::android_work_lease(state, action)
            .filter(|l| l.id == input.lease_id)
            .ok_or(ErrorCode::ControlRevoked)?;
        match lease.reserve(
            Self::input_sequence(input)?,
            Self::input_fingerprint(input)?,
            operation,
        )? {
            Reservation::New => Ok(()),
            Reservation::Replay(_) => Err(ErrorCode::IdempotencyConflict),
        }
    }
    pub(super) fn android_input_control(
        self: &Arc<Self>,
        owner: &str,
        input: AndroidControlInput,
    ) -> Reply {
        if !valid_id(&input.request_key) || input.expected_revision.parse::<u64>().is_err() {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        if let Err(code) = Self::android_input_access(
            &state,
            owner,
            &input.workspace_id,
            &input.panel_id,
            &input.device_id,
            &input.generation,
            input.action == PanelControlAction::Claim,
        ) {
            return error(code);
        }
        let session = &state.sessions[owner];
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
                resource_id: &input.device_id,
                generation: &input.generation,
                revision: &input.expected_revision,
            },
        ) {
            Ok(h) => h,
            Err(e) => return storage_error(e),
        };
        self.enqueue_ui(
            &mut state,
            owner,
            UiMutation {
                workspace: input.workspace_id.clone(),
                project,
                revision: input.expected_revision.clone(),
                retry_epoch: input.retry_epoch.clone(),
                request_key: input.request_key.clone(),
                tool: "lomi_panel_control",
                hash,
                action: UiAction::AndroidInputControl(input),
            },
        )
    }
    pub(super) fn android_input(self: &Arc<Self>, owner: &str, input: AndroidInput) -> Reply {
        let request_key = certificate_hash(
            format!("android-input:{}:{}", input.lease_id, input.input_sequence).as_bytes(),
        );
        let sequence = match Self::input_sequence(&input) {
            Ok(n) => n,
            Err(code) => return error(code),
        };
        if match &input.event {
            AndroidInputEvent::Rotate { quarter_turns } => *quarter_turns > 3,
            AndroidInputEvent::Text { text } => text.len() > 16384 || text.contains('\0'),
            AndroidInputEvent::Key { key, .. } => key.len() > 32 || key.contains('\0'),
            AndroidInputEvent::Touch {
                identifier, x, y, ..
            } => *identifier > 1 || *x > 32767 || *y > 32767,
            AndroidInputEvent::Navigation { .. } => false,
        } {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        let control = match Self::android_input_access(
            &state,
            owner,
            &input.workspace_id,
            &input.panel_id,
            &input.device_id,
            &input.generation,
            true,
        ) {
            Ok(c) => c,
            Err(code) => return error(code),
        };
        let Some(lease) = control
            .input()
            .filter(|l| l.id == input.lease_id && l.view == input.panel_id)
        else {
            return error(ErrorCode::ControlRevoked);
        };
        let session = &state.sessions[owner];
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
                resource_id: &input.device_id,
                generation: &input.generation,
                revision: "android-input-v1",
            },
        ) {
            Ok(h) => h,
            Err(e) => return storage_error(e),
        };
        let key = receipts::Key {
            pairing_id: owner,
            project_id: &project,
            retry_epoch: &input.retry_epoch,
            request_key: &request_key,
            tool: "lomi_android_input",
        };
        match self.replay(&state, owner, &key, hash) {
            Ok(Some(reply)) => return reply,
            Err(reply) => return *reply,
            Ok(None) => {}
        }
        let payload = match Self::input_fingerprint(&input) {
            Ok(h) => h,
            Err(code) => return error(code),
        };
        match lease.lookup(sequence, payload) {
            Ok(Some(operation)) => {
                return match self
                    .store
                    .lock()
                    .map_err(|_| ErrorCode::StorageUnavailable)
                    .and_then(|store| {
                        store
                            .get(owner, &project, &operation)
                            .map_err(|_| ErrorCode::StorageUnavailable)
                    }) {
                    Ok(receipt) => Self::terminal_receipt(&state, owner, receipt),
                    Err(code) => error(code),
                };
            }
            Ok(None) => {}
            Err(code) => return error(code),
        }
        let revision = state.projection.revision.clone();
        self.enqueue_ui(
            &mut state,
            owner,
            UiMutation {
                workspace: input.workspace_id.clone(),
                project,
                revision,
                retry_epoch: input.retry_epoch.clone(),
                request_key: request_key.clone(),
                tool: "lomi_android_input",
                hash,
                action: UiAction::AndroidInput(input),
            },
        )
    }
    pub fn authorize_android_input(
        &self,
        operation: &str,
        nonce: &str,
    ) -> Result<AndroidInputDispatch, ErrorCode> {
        let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        let work = state.work.get(operation).ok_or(ErrorCode::TargetNotFound)?;
        if !work.claimed
            || work.native_committed
            || work.command.nonce != nonce
            || work.command.ui_epoch != state.projection.ui_epoch
        {
            return Err(ErrorCode::ControlRevoked);
        }
        work.native_permit.check()?;
        let (workspace, panel, device, generation, selected) = match &work.command.action {
            UiAction::AndroidInputControl(i) => (
                &i.workspace_id,
                &i.panel_id,
                &i.device_id,
                &i.generation,
                i.action == PanelControlAction::Claim,
            ),
            UiAction::AndroidInput(i) => (
                &i.workspace_id,
                &i.panel_id,
                &i.device_id,
                &i.generation,
                true,
            ),
            _ => return Err(ErrorCode::ScopeDenied),
        };
        let control = Self::android_input_access(
            &state,
            &work.pairing,
            workspace,
            panel,
            device,
            generation,
            selected,
        )?;
        let action = match &work.command.action {
            UiAction::AndroidInputControl(i) => {
                if i.action == PanelControlAction::Claim {
                    AndroidInputDispatchAction::Claim
                } else {
                    AndroidInputDispatchAction::Release
                }
            }
            UiAction::AndroidInput(i) => AndroidInputDispatchAction::Send {
                lease: control
                    .input()
                    .filter(|l| l.id == i.lease_id)
                    .ok_or(ErrorCode::ControlRevoked)?,
                sequence: Self::input_sequence(i)?,
                event: i.event.clone(),
            },
            _ => unreachable!(),
        };
        let request = AndroidInputDispatch {
            control,
            permit: work.native_permit.clone(),
            workspace: workspace.clone(),
            panel: panel.clone(),
            generation: generation.clone(),
            action,
        };
        state.work.get_mut(operation).unwrap().native_committed = true;
        Ok(request)
    }
}

impl Broker {
    pub fn finish_android_input(
        &self,
        operation: &str,
        nonce: &str,
        result: Result<OperationResult, ErrorCode>,
    ) -> io::Result<()> {
        use receipts::{Effect, State as OperationState};
        let mut state = self.lock_state().map_err(|_| failure())?;
        let work = state.work.get(operation).ok_or_else(failure)?;
        if !work.claimed
            || work.command.nonce != nonce
            || work.command.ui_epoch != state.projection.ui_epoch
        {
            return Err(failure());
        }
        let (workspace, panel, device, generation, selected) = match &work.command.action {
            UiAction::AndroidInputControl(i) => (
                &i.workspace_id,
                &i.panel_id,
                &i.device_id,
                &i.generation,
                i.action == PanelControlAction::Claim,
            ),
            UiAction::AndroidInput(i) => (
                &i.workspace_id,
                &i.panel_id,
                &i.device_id,
                &i.generation,
                true,
            ),
            _ => return Err(failure()),
        };
        let result = if work.native_permit.check().is_err() {
            Err(ErrorCode::OutcomeUnknown)
        } else {
            result
        };
        if let Ok(output) = &result {
            let control = Self::android_input_access(
                &state,
                &work.pairing,
                workspace,
                panel,
                device,
                generation,
                selected,
            )
            .map_err(|_| failure())?;
            if !work.native_committed {
                return Err(failure());
            }
            match (&work.command.action, output) {
                (UiAction::AndroidInputControl(i), OperationResult::AndroidControl(r)) => {
                    if r.workspace_id != *workspace
                        || r.device_id != *device
                        || r.generation != *generation
                        || r.controlled != (i.action == PanelControlAction::Claim)
                    {
                        return Err(failure());
                    }
                    if r.controlled {
                        if control
                            .input()
                            .is_none_or(|l| Some(&l.id) != r.lease_id.as_ref())
                        {
                            return Err(failure());
                        }
                    } else if r.lease_id.is_some() || control.input().is_some() {
                        return Err(failure());
                    }
                }
                (UiAction::AndroidInput(i), OperationResult::AndroidInput(r)) => {
                    if r.workspace_id != *workspace
                        || r.device_id != *device
                        || r.generation != *generation
                        || r.input_sequence != i.input_sequence
                        || work
                            .android_input
                            .as_ref()
                            .is_none_or(|l| l.check().is_err())
                    {
                        return Err(failure());
                    }
                }
                _ => return Err(failure()),
            }
        }
        let (next, effect, mut output) = match result {
            Ok(r) => (OperationState::Succeeded, Effect::Complete, r),
            Err(code) => (
                if work.native_committed {
                    OperationState::OutcomeUnknown
                } else {
                    OperationState::Failed
                },
                if work.native_committed {
                    Effect::Unknown
                } else {
                    Effect::None
                },
                OperationResult::Failure { code },
            ),
        };
        if let OperationResult::AndroidControl(result) = &mut output {
            result.lease_id = None;
        }
        let mut store = self.store.lock().map_err(|_| failure())?;
        store
            .record_result(&work.pairing, &work.project, operation, &output)
            .map_err(|_| failure())?;
        store
            .transition(&work.pairing, &work.project, operation, next, effect, now())
            .map_err(|_| failure())?;
        drop(store);
        let mut work = state.work.remove(operation).unwrap();
        if let (Some(lease), OperationResult::AndroidInput(result)) = (&work.android_input, &output)
        {
            if lease
                .complete(result.input_sequence.parse().map_err(|_| failure())?)
                .is_ok()
            {
                work.android_input = None;
            }
        }
        Ok(())
    }
}
