use super::{
    operations::{storage_error, UiMutation, Work},
    *,
};
use crate::atomic_file::ReplaceError;

pub type SettingsApply =
    Arc<dyn Fn(&dyn Fn() -> Result<(), ErrorCode>) -> Result<String, ReplaceError> + Send + Sync>;
#[derive(Clone)]
pub struct SettingsPlan {
    pub before: SettingsUpdateValues,
    pub after: SettingsUpdateValues,
    pub source_revision: Option<String>,
    pub apply: SettingsApply,
}
pub type SettingsPrepareDispatch = Arc<
    dyn Fn(
            &SettingsPatch,
            &SettingsUpdateValues,
            &dyn Fn() -> Result<(), ErrorCode>,
        ) -> Result<SettingsPlan, ErrorCode>
        + Send
        + Sync,
>;
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingSettingsUpdate {
    operation_id: String,
    client_label: String,
    request_key: String,
    before: SettingsUpdateValues,
    after: SettingsUpdateValues,
    section: SettingsSection,
    patch: SettingsPatch,
    seconds_remaining: u64,
}
impl Broker {
    pub fn set_settings_prepare_dispatch(
        &self,
        dispatch: SettingsPrepareDispatch,
    ) -> io::Result<()> {
        *self
            .settings_prepare_dispatch
            .lock()
            .map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    fn settings_update_access(
        state: &State,
        owner: &str,
        workspace: &str,
    ) -> Result<(), ErrorCode> {
        let session = state
            .sessions
            .get(owner)
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .ok_or(ErrorCode::ControlRevoked)?;
        if !["settings.read", "settings.write"]
            .iter()
            .all(|s| session.grant.scopes.contains(*s))
        {
            return Err(ErrorCode::ScopeDenied);
        }
        if !state
            .projection
            .workspaces
            .iter()
            .any(|w| w.id == workspace && session.grant.permits(w))
        {
            return Err(ErrorCode::TargetNotFound);
        }
        Ok(())
    }
    fn settings_update_work<'a>(
        &self,
        state: &'a State,
        operation: &str,
        nonce: Option<&str>,
    ) -> Result<&'a Work, ErrorCode> {
        let work = state.work.get(operation).ok_or(ErrorCode::ControlRevoked)?;
        if !work.claimed
            || work.native_committed
            || nonce.is_some_and(|n| n != work.command.nonce)
            || work.command.ui_epoch != state.projection.ui_epoch
            || !matches!(work.command.action, UiAction::UpdateSettings(_))
        {
            return Err(ErrorCode::ControlRevoked);
        }
        work.native_permit.check()?;
        Self::settings_update_access(state, &work.pairing, &work.workspace)?;
        Ok(work)
    }
    pub(super) fn update_settings(
        self: &Arc<Self>,
        owner: &str,
        input: SettingsUpdateInput,
    ) -> Reply {
        if !valid_id(&input.request_key)
            || input.expected_revision.parse::<u64>().is_err()
            || input.expected_settings_revision.len() != 64
            || !input
                .expected_settings_revision
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
            || matches!(input.patch,SettingsPatch::EditorTabSize{value} if !(1..=16).contains(&value))
            || match &input.patch {
                SettingsPatch::KeybindingSet { action, shortcut } => {
                    action.is_empty()
                        || action.len() > 160
                        || shortcut.as_ref().is_some_and(|s| s.len() > 64)
                }
                SettingsPatch::KeybindingReset { action } => {
                    action.is_empty() || action.len() > 160
                }
                _ => false,
            }
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        if let Err(e) = Self::settings_update_access(&state, owner, &input.workspace_id) {
            return error(e);
        }
        let session = &state.sessions[owner];
        if input.retry_epoch != session.retry_epoch {
            return error(ErrorCode::RetryWindowExpired);
        }
        let root = session.grant.workspace(&input.workspace_id).unwrap();
        let project = root.project_id.clone();
        let hash = match receipts::fingerprint(
            &(&input, &root.project_path),
            &receipts::Target {
                workspace_id: &input.workspace_id,
                resource_id: "settings",
                generation: &state.projection.ui_epoch,
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
                tool: "lomi_settings_update",
                hash,
                action: UiAction::UpdateSettings(Box::new(SettingsUpdateCommand {
                    workspace_id: input.workspace_id.clone(),
                    input,
                    not_after_millis: (now() * 1000 + 120_000).to_string(),
                })),
            },
        )
    }
    pub fn prepare_settings_update(
        &self,
        operation: &str,
        nonce: &str,
        revision: &str,
        current: SettingsUpdateValues,
    ) -> Result<super::settings::SettingsOpenPermit, ErrorCode> {
        let (patch, permit) = {
            let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let work = self.settings_update_work(&state, operation, Some(nonce))?;
            let UiAction::UpdateSettings(command) = &work.command.action else {
                unreachable!()
            };
            if work.settings_update.is_some()
                || command.input.expected_settings_revision != revision
                || state.projection.revision != work.command.domain_revision
            {
                return Err(ErrorCode::RevisionConflict);
            }
            (
                command.input.patch.clone(),
                self.settings_permit(&state, work),
            )
        };
        let dispatch = self
            .settings_prepare_dispatch
            .lock()
            .ok()
            .and_then(|d| d.clone())
            .ok_or(ErrorCode::UnsupportedCapability)?;
        let plan = dispatch(&patch, &current, &|| permit.check())?;
        let mut expected = current.clone();
        patch.apply_values(&mut expected)?;
        if plan.before != current
            || plan.after != expected
            || plan
                .source_revision
                .as_ref()
                .is_some_and(|r| r.len() != 64 || !r.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(ErrorCode::RevisionConflict);
        }
        permit.check()?;
        let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        let work = self.settings_update_work(&state, operation, Some(nonce))?;
        if work.settings_update.is_some() {
            return Err(ErrorCode::ControlRevoked);
        }
        state.work.get_mut(operation).unwrap().settings_update = Some(plan);
        Ok(permit)
    }
    pub fn settings_update_source_permit(
        &self,
        operation: &str,
        nonce: &str,
        revision: &str,
    ) -> Result<super::settings::SettingsOpenPermit, ErrorCode> {
        let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        let work = self.settings_update_work(&state, operation, Some(nonce))?;
        let UiAction::UpdateSettings(command) = &work.command.action else {
            unreachable!()
        };
        if work.settings_update.is_some()
            || command.input.patch.section() != SettingsSection::Keybinds
            || command.input.expected_settings_revision != revision
            || state.projection.revision != work.command.domain_revision
        {
            return Err(ErrorCode::RevisionConflict);
        }
        Ok(self.settings_permit(&state, work))
    }
    pub(super) fn pending_settings_updates(
        state: &State,
        authorized: bool,
    ) -> Vec<PendingSettingsUpdate> {
        state
            .work
            .iter()
            .filter_map(|(id, w)| {
                if !authorized
                    || !w.claimed
                    || w.native_committed
                    || w.deadline <= Instant::now()
                    || w.native_permit.check().is_err()
                    || Self::settings_update_access(state, &w.pairing, &w.workspace).is_err()
                {
                    return None;
                }
                let UiAction::UpdateSettings(command) = &w.command.action else {
                    return None;
                };
                let plan = w.settings_update.as_ref()?;
                Some(PendingSettingsUpdate {
                    operation_id: id.clone(),
                    client_label: state.sessions[&w.pairing].view.client_label.clone(),
                    request_key: command.input.request_key.clone(),
                    before: plan.before.clone(),
                    after: plan.after.clone(),
                    section: command.input.patch.section(),
                    patch: command.input.patch.clone(),
                    seconds_remaining: w
                        .deadline
                        .saturating_duration_since(Instant::now())
                        .as_secs(),
                })
            })
            .collect()
    }
    pub fn decide_settings_update(&self, operation: &str, approve: bool) -> Result<(), ErrorCode> {
        let (plan, permit, command, owner) = {
            let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let work = self.settings_update_work(&state, operation, None)?;
            let plan = work.settings_update.clone().ok_or(ErrorCode::UiNotReady)?;
            let mut store = self
                .store
                .lock()
                .map_err(|_| ErrorCode::StorageUnavailable)?;
            if !approve {
                store
                    .transition(
                        &work.pairing,
                        &work.project,
                        operation,
                        receipts::State::Cancelled,
                        receipts::Effect::None,
                        now(),
                    )
                    .map_err(|_| ErrorCode::StorageUnavailable)?;
                drop(store);
                state.work.remove(operation);
                return Ok(());
            }
            store
                .transition(
                    &work.pairing,
                    &work.project,
                    operation,
                    receipts::State::Queued,
                    receipts::Effect::None,
                    now(),
                )
                .map_err(|_| ErrorCode::StorageUnavailable)?;
            if store
                .transition(
                    &work.pairing,
                    &work.project,
                    operation,
                    receipts::State::Running,
                    receipts::Effect::None,
                    now(),
                )
                .is_err()
            {
                drop(store);
                if let Some(work) = state.work.remove(operation) {
                    self.settle_lost(&work)
                }
                return Err(ErrorCode::StorageUnavailable);
            }
            let result = (
                plan,
                self.settings_permit(&state, work),
                work.command.clone(),
                work.pairing.clone(),
            );
            drop(store);
            state.work.get_mut(operation).unwrap().native_committed = true;
            result
        };
        let result = (plan.apply)(&|| permit.check()).map(|revision| {
            OperationResult::SettingsUpdated(SettingsUpdated {
                workspace_id: match &command.action {
                    UiAction::UpdateSettings(c) => c.workspace_id.clone(),
                    _ => unreachable!(),
                },
                section: match &command.action {
                    UiAction::UpdateSettings(c) => c.input.patch.section(),
                    _ => unreachable!(),
                },
                previous_stored_revision: plan.source_revision,
                stored_revision: revision,
                applied: true,
            })
        });
        let saved = result.as_ref().ok().cloned();
        self.finish_native_file_write(operation, &command.nonce, &owner, result)?;
        self.acknowledge_ui(UiAck {
            operation_id: operation.into(),
            nonce: command.nonce,
            ui_epoch: command.ui_epoch,
            result: saved.ok_or(ErrorCode::OutcomeUnknown)?,
        })
        .map_err(|_| ErrorCode::OutcomeUnknown)
    }
}
