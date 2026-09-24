use super::{
    operations::{storage_error, NativePermit, UiMutation},
    *,
};

pub struct SettingsOpenPermit {
    operation: NativePermit,
    authorization: Arc<AtomicU64>,
    policy: u64,
    connected: Arc<AtomicBool>,
    automatically_approved: bool,
}
impl SettingsOpenPermit {
    pub fn automatically_approved(&self) -> bool {
        self.automatically_approved
    }
    pub(super) fn mark_automatically_approved(&mut self) {
        self.automatically_approved = true;
    }
    pub fn check(&self) -> Result<(), ErrorCode> {
        if self.authorization.load(Ordering::SeqCst) != self.policy
            || !self.connected.load(Ordering::SeqCst)
        {
            return Err(ErrorCode::ControlRevoked);
        }
        self.operation.check()
    }
}

impl Broker {
    pub(super) fn revoke_missing_settings_targets(state: &State, projection: &Projection) {
        for work in state.work.values().filter(|w| {
            matches!(
                w.command.action,
                UiAction::OpenSettings(_) | UiAction::UpdateSettings(_)
            )
        }) {
            let live = state.sessions.get(&work.pairing).is_some_and(|session| {
                projection
                    .workspaces
                    .iter()
                    .any(|w| w.id == work.workspace && session.grant.permits(w))
            });
            if !live {
                work.native_permit.revoke();
            }
        }
    }
    pub(super) fn settings_permit(
        &self,
        state: &State,
        work: &super::operations::Work,
    ) -> SettingsOpenPermit {
        SettingsOpenPermit {
            operation: work.native_permit.clone(),
            authorization: self.authorization.clone(),
            policy: state.policy_revision,
            connected: state.sessions[&work.pairing].alive.clone(),
            automatically_approved: false,
        }
    }
    fn settings_access(state: &State, owner: &str, workspace: &str) -> Result<(), ErrorCode> {
        let session = state
            .sessions
            .get(owner)
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .ok_or(ErrorCode::ControlRevoked)?;
        if !session.grant.scopes.contains("settings.open") {
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
    pub(super) fn open_settings(self: &Arc<Self>, owner: &str, input: SettingsOpenInput) -> Reply {
        if !valid_id(&input.request_key) || input.expected_revision.parse::<u64>().is_err() {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        if let Err(code) = Self::settings_access(&state, owner, &input.workspace_id) {
            return error(code);
        }
        let session = &state.sessions[owner];
        if session.retry_epoch != input.retry_epoch {
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
            Ok(hash) => hash,
            Err(e) => return storage_error(e),
        };
        self.enqueue_ui(
            &mut state,
            owner,
            UiMutation {
                workspace: input.workspace_id.clone(),
                project,
                revision: input.expected_revision,
                retry_epoch: input.retry_epoch,
                request_key: input.request_key,
                tool: "lomi_settings_open",
                hash,
                action: UiAction::OpenSettings(SettingsOpenCommand {
                    workspace_id: input.workspace_id,
                    page: input.page,
                }),
            },
        )
    }
    pub fn begin_settings_open(
        &self,
        operation: &str,
        nonce: &str,
    ) -> Result<(UiCommand, SettingsOpenPermit), ErrorCode> {
        let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        let work = state.work.get(operation).ok_or(ErrorCode::TargetNotFound)?;
        if !matches!(work.command.action, UiAction::OpenSettings(_))
            || !work.claimed
            || work.native_committed
            || work.command.nonce != nonce
            || work.command.ui_epoch != state.projection.ui_epoch
        {
            return Err(ErrorCode::ControlRevoked);
        }
        work.native_permit.check()?;
        Self::settings_access(&state, &work.pairing, &work.workspace)?;
        if work.command.domain_revision != state.projection.revision {
            return Err(ErrorCode::RevisionConflict);
        }
        // claim_ui already durably moved the receipt to Running before this permit.
        let result = (
            work.command.clone(),
            SettingsOpenPermit {
                operation: work.native_permit.clone(),
                authorization: self.authorization.clone(),
                policy: state.policy_revision,
                connected: state.sessions[&work.pairing].alive.clone(),
                automatically_approved: false,
            },
        );
        state.work.get_mut(operation).unwrap().native_committed = true;
        Ok(result)
    }
    pub fn complete_settings_open(&self, operation: &str, nonce: &str) -> io::Result<()> {
        let ack = {
            let state = self.lock_state().map_err(|_| failure())?;
            let work = state.work.get(operation).ok_or_else(failure)?;
            let UiAction::OpenSettings(command) = &work.command.action else {
                return Err(failure());
            };
            if !work.native_committed || work.command.nonce != nonce {
                return Err(failure());
            }
            work.native_permit.check().map_err(|_| failure())?;
            Self::settings_access(&state, &work.pairing, &work.workspace).map_err(|_| failure())?;
            let result = OperationResult::SettingsOpened(SettingsOpened {
                workspace_id: command.workspace_id.clone(),
                page: command.page,
                requested: true,
            });
            self.store
                .lock()
                .map_err(|_| failure())?
                .record_result(&work.pairing, &work.project, operation, &result)
                .map_err(|_| failure())?;
            UiAck {
                operation_id: operation.into(),
                nonce: nonce.into(),
                ui_epoch: work.command.ui_epoch.clone(),
                result,
            }
        };
        self.acknowledge_ui(ack)
    }
}
