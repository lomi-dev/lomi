use super::operations::{storage_error, UiMutation};
use super::*;

pub struct AndroidLaunchDispatch {
    pub control: Arc<crate::android::AndroidControl>,
    pub permit: NativePermit,
    pub input: AndroidLaunchInput,
}
impl Broker {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn android_app_access(
        state: &State,
        owner: &str,
        workspace: &str,
        panel: &str,
        device: &str,
        generation: &str,
        package: &str,
        scope: &str,
    ) -> Result<Arc<crate::android::AndroidControl>, ErrorCode> {
        Self::android_runtime_access(state, owner, workspace, panel, device, Some(generation))?;
        let grant = &state.sessions[owner].grant;
        if !grant.scopes.contains(scope) || !grant.permits_android_package(package) {
            return Err(ErrorCode::ScopeDenied);
        }
        Ok(state.android[device].control.clone())
    }
    pub(super) fn android_launch(
        self: &Arc<Self>,
        owner: &str,
        input: AndroidLaunchInput,
    ) -> Reply {
        if !valid_id(&input.request_key)
            || input.expected_revision.parse::<u64>().is_err()
            || !lomi_control_protocol::android::valid_package(&input.package_name)
            || input
                .activity
                .as_ref()
                .is_some_and(|s| !lomi_control_protocol::android::valid_activity(s))
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        if let Err(code) = Self::android_access(&state, owner, &input.workspace_id) {
            return error(code);
        }
        let session = &state.sessions[owner];
        if !session.grant.scopes.contains("android.launch")
            || !session.grant.permits_android_package(&input.package_name)
        {
            return error(ErrorCode::ScopeDenied);
        }
        if session.retry_epoch != input.retry_epoch {
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
        let key = receipts::Key {
            pairing_id: owner,
            project_id: &project,
            retry_epoch: &input.retry_epoch,
            request_key: &input.request_key,
            tool: "lomi_android_launch",
        };
        match self.replay(&state, owner, &key, hash) {
            Ok(Some(r)) => return r,
            Err(r) => return *r,
            Ok(None) => {}
        }
        if let Err(code) = Self::android_app_access(
            &state,
            owner,
            &input.workspace_id,
            &input.panel_id,
            &input.device_id,
            &input.generation,
            &input.package_name,
            "android.launch",
        ) {
            return error(code);
        }
        self.enqueue_ui(
            &mut state,
            owner,
            UiMutation {
                workspace: input.workspace_id.clone(),
                project,
                revision: input.expected_revision.clone(),
                retry_epoch: input.retry_epoch.clone(),
                request_key: input.request_key.clone(),
                tool: "lomi_android_launch",
                hash,
                action: UiAction::AndroidLaunch(input),
            },
        )
    }
    pub fn authorize_android_launch(
        &self,
        operation: &str,
        nonce: &str,
    ) -> Result<AndroidLaunchDispatch, ErrorCode> {
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
        let UiAction::AndroidLaunch(input) = &work.command.action else {
            return Err(ErrorCode::ScopeDenied);
        };
        let control = Self::android_app_access(
            &state,
            &work.pairing,
            &input.workspace_id,
            &input.panel_id,
            &input.device_id,
            &input.generation,
            &input.package_name,
            "android.launch",
        )?;
        let request = AndroidLaunchDispatch {
            control,
            permit: work.native_permit.clone(),
            input: input.clone(),
        };
        state.work.get_mut(operation).unwrap().native_committed = true;
        Ok(request)
    }
    pub fn finish_android_launch(
        &self,
        operation: &str,
        nonce: &str,
        result: Result<AndroidLaunchResult, ErrorCode>,
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
        let UiAction::AndroidLaunch(input) = &work.command.action else {
            return Err(failure());
        };
        let result = if work.native_permit.check().is_err() {
            Err(ErrorCode::OutcomeUnknown)
        } else {
            result
        };
        if let Ok(r) = &result {
            Self::android_app_access(
                &state,
                &work.pairing,
                &input.workspace_id,
                &input.panel_id,
                &input.device_id,
                &input.generation,
                &input.package_name,
                "android.launch",
            )
            .map_err(|_| failure())?;
            if !work.native_committed
                || r.workspace_id != input.workspace_id
                || r.device_id != input.device_id
                || r.generation != input.generation
                || r.package_name != input.package_name
                || !r.intent_delivered
                || !lomi_control_protocol::android::valid_activity(&r.activity)
                || input.activity.as_ref().is_some_and(|a| a != &r.activity)
            {
                return Err(failure());
            }
        }
        let (next, effect, result) = match result {
            Ok(r) => (
                OperationState::Succeeded,
                Effect::Complete,
                OperationResult::AndroidLaunch(r),
            ),
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
        let mut store = self.store.lock().map_err(|_| failure())?;
        store
            .record_result(&work.pairing, &work.project, operation, &result)
            .map_err(|_| failure())?;
        store
            .transition(&work.pairing, &work.project, operation, next, effect, now())
            .map_err(|_| failure())?;
        drop(store);
        state.work.remove(operation);
        Ok(())
    }
}
