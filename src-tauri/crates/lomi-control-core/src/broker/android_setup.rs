use super::*;
use operations::storage_error;
use receipts::{Effect, State as OperationState};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidTerms {
    pub id: String,
    pub digest: String,
    pub text: String,
}
pub type AndroidManagementApply = Arc<
    dyn Fn(AndroidManagementRequest) -> Result<AndroidManagementResult, ErrorCode> + Send + Sync,
>;
#[derive(Clone)]
pub struct AndroidManagementPlan {
    pub view: AndroidPreparedPlan,
    pub licenses: Vec<AndroidTerms>,
    pub apply: AndroidManagementApply,
    pub present: Arc<dyn Fn(NativePermit) + Send + Sync>,
}
pub struct AndroidSetupRead {
    pub view: AndroidSetupView,
    pub plan: Option<AndroidManagementPlan>,
}
pub enum AndroidPrepareInput {
    Setup(AndroidSetupQuery),
    Device(AndroidDeviceAction),
}
pub type AndroidSetupDispatch = Arc<
    dyn Fn(
            AndroidPrepareInput,
            &[String],
            &dyn Fn() -> Result<(), ErrorCode>,
        ) -> Result<AndroidSetupRead, ErrorCode>
        + Send
        + Sync,
>;
pub struct AndroidManagementRequest {
    pub workspace_id: String,
    pub accepted: Vec<String>,
    pub permit: NativePermit,
    started: Arc<AtomicBool>,
}
impl AndroidManagementRequest {
    pub fn mark_dispatching(&self) -> Result<(), ErrorCode> {
        self.permit.check()?;
        self.started.store(true, Ordering::SeqCst);
        self.permit.check()
    }
}
pub(super) struct SavedAndroidPlan {
    owner: String,
    workspace: String,
    deadline: Instant,
    plan: AndroidManagementPlan,
}
pub(super) struct AndroidManagementJob {
    owner: String,
    project: String,
    workspace: String,
    action: Option<AndroidDeviceAction>,
    plan: AndroidManagementPlan,
    pub(super) permit: NativePermit,
    started: Arc<AtomicBool>,
    deadline: Instant,
    running: bool,
}
impl Drop for AndroidManagementJob {
    fn drop(&mut self) {
        self.permit.revoke();
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingAndroidManagement {
    operation_id: String,
    client_label: String,
    workspace_id: String,
    plan: AndroidPreparedPlan,
    action: Option<AndroidDeviceAction>,
    licenses: Vec<AndroidTerms>,
    seconds_remaining: u64,
}
impl Broker {
    pub fn set_android_setup_dispatch(&self, dispatch: AndroidSetupDispatch) -> io::Result<()> {
        *self.android_setup_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    fn android_management_access(
        state: &State,
        owner: &str,
        workspace: &str,
        action: Option<&AndroidDeviceAction>,
    ) -> Result<(), ErrorCode> {
        Self::android_access(state, owner, workspace)?;
        let grant = &state.sessions[owner].grant;
        let permitted = action.map_or_else(
            || grant.scopes.contains("android.setup") || grant.scopes.contains("android.manage"),
            |a| grant.scopes.contains(a.scope()),
        );
        if !permitted
            || action
                .and_then(AndroidDeviceAction::device_id)
                .is_some_and(|id| !grant.android_devices.contains(id))
        {
            return Err(ErrorCode::ScopeDenied);
        }
        if matches!(action, Some(AndroidDeviceAction::Create { .. }))
            && grant.android_devices.len() >= 16
        {
            return Err(ErrorCode::ResourceExhausted);
        }
        Ok(())
    }
    fn prepare_android_management(
        &self,
        owner: &str,
        workspace: &str,
        input: AndroidPrepareInput,
    ) -> Result<AndroidSetupRead, ErrorCode> {
        if matches!(&input, AndroidPrepareInput::Device(action) if serde_json::to_vec(action).map_or(true, |bytes| bytes.len() > 8192))
        {
            return Err(ErrorCode::ResourceExhausted);
        }
        let (devices, authorization, connected) = {
            let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            if matches!(&input, AndroidPrepareInput::Setup(query) if !matches!(query, AndroidSetupQuery::Inventory))
                && !state
                    .sessions
                    .get(owner)
                    .is_some_and(|s| s.grant.scopes.contains("android.setup"))
            {
                return Err(ErrorCode::ScopeDenied);
            }
            Self::android_management_access(
                &state,
                owner,
                workspace,
                match &input {
                    AndroidPrepareInput::Device(a) => Some(a),
                    _ => None,
                },
            )?;
            (
                Self::android_access(&state, owner, workspace)?,
                state.policy_revision,
                state.sessions[owner].alive.clone(),
            )
        };
        let dispatch = self
            .android_setup_dispatch
            .lock()
            .ok()
            .and_then(|d| d.clone())
            .ok_or(ErrorCode::UnsupportedCapability)?;
        let deadline = Instant::now() + Duration::from_secs(45);
        let check = || {
            if !connected.load(Ordering::SeqCst)
                || self.authorization.load(Ordering::SeqCst) != authorization
            {
                return Err(ErrorCode::ControlRevoked);
            }
            if Instant::now() >= deadline {
                return Err(ErrorCode::DeadlineExceeded);
            }
            Ok(())
        };
        check()?;
        let result = dispatch(input, &devices, &check)?;
        check()?;
        if serde_json::to_vec(&result.view)
            .map_err(|_| ErrorCode::OutcomeUnknown)?
            .len()
            > 48 * 1024
        {
            return Err(ErrorCode::ResourceExhausted);
        }
        if let Some(plan) = &result.plan {
            if !valid_id(&plan.view.plan_id)
                || plan.view.revision.len() != 64
                || plan.licenses.len() > 16
                || plan.licenses.iter().map(|l| l.text.len()).sum::<usize>() > 1024 * 1024
                || plan.licenses.len() != plan.view.licenses.len()
                || plan
                    .licenses
                    .iter()
                    .zip(&plan.view.licenses)
                    .any(|(a, b)| a.id != b.id || a.digest != b.digest)
                || !matches!(&result.view, AndroidSetupView::Prepared(view) if view.plan_id == plan.view.plan_id && view.revision == plan.view.revision)
            {
                return Err(ErrorCode::OutcomeUnknown);
            }
        }
        Ok(result)
    }
    pub(super) fn android_setup_plan(&self, owner: &str, input: AndroidSetupPlanInput) -> Reply {
        let result = match self.prepare_android_management(
            owner,
            &input.workspace_id,
            AndroidPrepareInput::Setup(input.action),
        ) {
            Ok(r) => r,
            Err(e) => return error(e),
        };
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        if let Err(e) = Self::android_management_access(&state, owner, &input.workspace_id, None) {
            return error(e);
        }
        if let Some(plan) = result.plan {
            state
                .android_plans
                .retain(|_, p| p.deadline > Instant::now());
            if state.android_plans.len() >= 8 {
                return error(ErrorCode::TargetBusy);
            }
            state.android_plans.insert(
                plan.view.plan_id.clone(),
                SavedAndroidPlan {
                    owner: owner.into(),
                    workspace: input.workspace_id,
                    deadline: Instant::now() + Duration::from_secs(1800),
                    plan,
                },
            );
        }
        Reply::ok(Data::AndroidSetup(Box::new(result.view)))
    }
    pub(super) fn android_setup_apply(
        self: &Arc<Self>,
        owner: &str,
        input: AndroidSetupApplyInput,
    ) -> Reply {
        self.enqueue_android_management(
            owner,
            &input.workspace_id,
            &input.retry_epoch,
            &input.request_key,
            "lomi_android_setup_apply",
            &input,
            None,
            Some((&input.plan_id, &input.plan_revision)),
        )
    }
    pub(super) fn android_device_manage(
        self: &Arc<Self>,
        owner: &str,
        input: AndroidDeviceManageInput,
    ) -> Reply {
        self.enqueue_android_management(
            owner,
            &input.workspace_id,
            &input.retry_epoch,
            &input.request_key,
            "lomi_android_device_manage",
            &input,
            Some(input.action.clone()),
            None,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn enqueue_android_management(
        self: &Arc<Self>,
        owner: &str,
        workspace: &str,
        epoch: &str,
        request_key: &str,
        tool: &str,
        input: &impl Serialize,
        action: Option<AndroidDeviceAction>,
        saved: Option<(&str, &str)>,
    ) -> Reply {
        if !valid_id(request_key) {
            return error(ErrorCode::ResourceExhausted);
        }
        let (project, hash) = {
            let Ok(state) = self.lock_state() else {
                return error(ErrorCode::ControlRevoked);
            };
            if let Err(e) =
                Self::android_management_access(&state, owner, workspace, action.as_ref())
            {
                return error(e);
            }
            let session = &state.sessions[owner];
            if action.is_none() && !session.grant.scopes.contains("android.setup") {
                return error(ErrorCode::ScopeDenied);
            }
            if session.retry_epoch != epoch {
                return error(ErrorCode::RetryWindowExpired);
            }
            let root = session.grant.workspace(workspace).unwrap();
            let hash = match receipts::fingerprint(
                &(input, &root.project_path),
                &receipts::Target {
                    workspace_id: workspace,
                    resource_id: "android-management",
                    generation: "native",
                    revision: "1",
                },
            ) {
                Ok(h) => h,
                Err(e) => return storage_error(e),
            };
            let key = receipts::Key {
                pairing_id: owner,
                project_id: &root.project_id,
                retry_epoch: epoch,
                request_key,
                tool,
            };
            match self.replay(&state, owner, &key, hash) {
                Ok(Some(r)) => return r,
                Err(r) => return *r,
                _ => {}
            }
            (root.project_id.clone(), hash)
        };
        // Preparation has no device or installation effects. Durable replay is checked
        // before it, so a changed native revision cannot hide a completed receipt.
        let prepared = if let Some(action) = &action {
            match self.prepare_android_management(
                owner,
                workspace,
                AndroidPrepareInput::Device(action.clone()),
            ) {
                Ok(r) => r.plan,
                Err(e) => return error(e),
            }
        } else {
            None
        };
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        if let Err(e) = Self::android_management_access(&state, owner, workspace, action.as_ref()) {
            return error(e);
        }
        let key = receipts::Key {
            pairing_id: owner,
            project_id: &project,
            retry_epoch: epoch,
            request_key,
            tool,
        };
        match self.replay(&state, owner, &key, hash) {
            Ok(Some(r)) => return r,
            Err(r) => return *r,
            _ => {}
        }
        if !state.android_management.is_empty() {
            return error(ErrorCode::TargetBusy);
        }
        let plan = if let Some((id, revision)) = saved {
            let Some(saved) = state.android_plans.get(id).filter(|p| {
                p.owner == owner && p.workspace == workspace && p.deadline > Instant::now()
            }) else {
                return error(ErrorCode::TargetNotFound);
            };
            if saved.plan.view.revision != revision {
                return error(ErrorCode::RevisionConflict);
            }
            saved.plan.clone()
        } else {
            let Some(plan) = prepared else {
                return error(ErrorCode::OutcomeUnknown);
            };
            plan
        };
        let Ok(mut store) = self.store.lock() else {
            return error(ErrorCode::StorageUnavailable);
        };
        let reserved = match store.reserve(&key, hash, now()) {
            Ok(r) => r,
            Err(e) => return storage_error(e),
        };
        if !reserved.created {
            return Self::operation_reply(reserved.receipt);
        }
        let op = reserved.receipt.operation_id;
        if let Err(e) = store.bind_workspace(owner, &project, &op, workspace) {
            return storage_error(e);
        }
        let receipt = match store.transition(
            owner,
            &project,
            &op,
            OperationState::AwaitingUser,
            Effect::None,
            now(),
        ) {
            Ok(r) => r,
            Err(e) => return storage_error(e),
        };
        let deadline = Instant::now() + Duration::from_secs(600);
        if let Some((id, _)) = saved {
            state.android_plans.remove(id);
        }
        state.android_management.insert(
            op.clone(),
            AndroidManagementJob {
                owner: owner.into(),
                project,
                workspace: workspace.into(),
                action,
                plan,
                permit: NativePermit::until(deadline),
                started: Arc::new(AtomicBool::new(false)),
                deadline,
                running: false,
            },
        );
        let job = &state.android_management[&op];
        let present = job.plan.present.clone();
        let permit = job.permit.clone();
        drop(store);
        drop(state);
        present(permit);
        self.expire_android_management(op, Duration::from_secs(600));
        Self::operation_reply(receipt)
    }
    pub(super) fn pending_android_management(
        state: &State,
        authorized: bool,
    ) -> Vec<PendingAndroidManagement> {
        state
            .android_management
            .iter()
            .filter(|(_, j)| authorized && !j.running && j.permit.check().is_ok())
            .map(|(id, j)| PendingAndroidManagement {
                operation_id: id.clone(),
                client_label: state
                    .sessions
                    .get(&j.owner)
                    .map(|s| s.view.client_label.clone())
                    .unwrap_or_default(),
                workspace_id: j.workspace.clone(),
                plan: j.plan.view.clone(),
                action: j.action.clone(),
                licenses: j.plan.licenses.clone(),
                seconds_remaining: j
                    .deadline
                    .saturating_duration_since(Instant::now())
                    .as_secs(),
            })
            .collect()
    }
    fn settle_android_management(&self, operation: &str, job: &AndroidManagementJob) {
        job.permit.revoke();
        if let Ok(mut store) = self.store.lock() {
            let _ = store.transition(
                &job.owner,
                &job.project,
                operation,
                if job.running {
                    OperationState::OutcomeUnknown
                } else {
                    OperationState::Cancelled
                },
                if job.running {
                    Effect::Unknown
                } else {
                    Effect::None
                },
                now(),
            );
        }
    }
    pub(super) fn end_android_management(&self, state: &mut State, pairing: Option<&str>) {
        state
            .android_plans
            .retain(|_, p| pairing.is_some_and(|id| p.owner != id));
        let ids: Vec<_> = state
            .android_management
            .iter()
            .filter(|(_, j)| pairing.is_none_or(|id| j.owner == id))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if let Some(job) = state.android_management.remove(&id) {
                self.settle_android_management(&id, &job);
            }
        }
    }
    pub(super) fn reconcile_android_management(&self, state: &mut State, projection: &Projection) {
        let valid = |owner: &str, workspace: &str| {
            state.sessions.get(owner).is_some_and(|s| {
                projection
                    .workspaces
                    .iter()
                    .any(|w| w.id == workspace && s.grant.permits(w))
            })
        };
        let plans: Vec<_> = state
            .android_plans
            .iter()
            .filter(|(_, p)| !valid(&p.owner, &p.workspace))
            .map(|(id, _)| id.clone())
            .collect();
        let jobs: Vec<_> = state
            .android_management
            .iter()
            .filter(|(_, j)| !valid(&j.owner, &j.workspace))
            .map(|(id, _)| id.clone())
            .collect();
        for id in plans {
            state.android_plans.remove(&id);
        }
        for id in jobs {
            if let Some(job) = state.android_management.remove(&id) {
                self.settle_android_management(&id, &job);
            }
        }
    }
    fn expire_android_management(self: &Arc<Self>, operation: String, duration: Duration) {
        let weak = Arc::downgrade(self);
        self.spawn_background(async move {
            tokio::time::sleep(duration).await;
            if let Some(broker) = weak.upgrade() {
                let worker = broker.clone();
                let _ = broker
                    .spawn_worker(move || {
                        if let Ok(mut state) = worker.lock_state() {
                            if state
                                .android_management
                                .get(&operation)
                                .is_some_and(|j| j.deadline <= Instant::now())
                            {
                                if let Some(job) = state.android_management.remove(&operation) {
                                    worker.settle_android_management(&operation, &job);
                                }
                            }
                        };
                    })
                    .await;
            }
        });
    }
    /// Called only by the trusted native Settings command, never an MCP request.
    pub fn decide_android_management(
        self: &Arc<Self>,
        operation: &str,
        revision: &str,
        approve: bool,
        accepted: Vec<String>,
        confirmation: Option<String>,
    ) -> io::Result<()> {
        let mut state = self.lock_state().map_err(|_| failure())?;
        let job = state
            .android_management
            .get(operation)
            .ok_or_else(failure)?;
        if job.running || job.plan.view.revision != revision {
            return Err(failure());
        }
        if !approve
            || job.permit.check().is_err()
            || Self::android_management_access(
                &state,
                &job.owner,
                &job.workspace,
                job.action.as_ref(),
            )
            .is_err()
        {
            let job = state.android_management.remove(operation).unwrap();
            self.settle_android_management(operation, &job);
            return Ok(());
        }
        if let Some(name) = job
            .action
            .as_ref()
            .and_then(AndroidDeviceAction::destructive_confirmation)
        {
            if confirmation.as_deref() != Some(name) {
                return Err(failure());
            }
        }
        let expected: HashSet<_> = job
            .plan
            .licenses
            .iter()
            .map(|l| l.digest.as_str())
            .collect();
        if accepted.len() != expected.len()
            || accepted.iter().map(String::as_str).collect::<HashSet<_>>() != expected
        {
            return Err(failure());
        }
        let mut store = self.store.lock().map_err(|_| failure())?;
        if store
            .get(&job.owner, &job.project, operation)
            .map_err(|_| failure())?
            .state
            != OperationState::AwaitingUser
        {
            return Err(failure());
        }
        store
            .transition(
                &job.owner,
                &job.project,
                operation,
                OperationState::Queued,
                Effect::None,
                now(),
            )
            .map_err(|_| failure())?;
        store
            .transition(
                &job.owner,
                &job.project,
                operation,
                OperationState::Running,
                Effect::None,
                now(),
            )
            .map_err(|_| failure())?;
        let job = state.android_management.get_mut(operation).unwrap();
        job.deadline = Instant::now() + Duration::from_secs(7200);
        job.permit = NativePermit::until(job.deadline);
        job.running = true;
        let request = AndroidManagementRequest {
            workspace_id: job.workspace.clone(),
            accepted,
            permit: job.permit.clone(),
            started: job.started.clone(),
        };
        let apply = job.plan.apply.clone();
        drop(store);
        drop(state);
        let broker = self.clone();
        let op = operation.to_string();
        self.expire_android_management(op.clone(), Duration::from_secs(7200));
        drop(self.spawn_worker(move || {
            let result = apply(request);
            let _ = broker.finish_android_management(&op, result);
        }));
        Ok(())
    }
    fn finish_android_management(
        &self,
        operation: &str,
        result: Result<AndroidManagementResult, ErrorCode>,
    ) -> io::Result<()> {
        let mut state = self.lock_state().map_err(|_| failure())?;
        let job = state
            .android_management
            .get(operation)
            .filter(|j| j.running)
            .ok_or_else(failure)?;
        let authorized = job.permit.check().is_ok()
            && Self::android_management_access(
                &state,
                &job.owner,
                &job.workspace,
                job.action.as_ref(),
            )
            .is_ok();
        let result = result.and_then(|r| {
            if !authorized
                || !job.started.load(Ordering::SeqCst)
                || r.workspace_id != job.workspace
                || !lomi_control_protocol::android::valid_device_id(&r.native_operation_id)
                || r.device_id
                    .as_ref()
                    .is_some_and(|id| !lomi_control_protocol::android::valid_device_id(id))
                || match &job.action {
                    Some(AndroidDeviceAction::Create { .. }) => {
                        r.device_id.is_none()
                            || r.device_id.as_ref().is_some_and(|id| {
                                state.sessions[&job.owner]
                                    .grant
                                    .android_devices
                                    .contains(id)
                            })
                    }
                    Some(a) if a.device_id().is_some() => r.device_id.as_deref() != a.device_id(),
                    _ => r.device_id.is_some(),
                }
            {
                return Err(ErrorCode::OutcomeUnknown);
            }
            Ok(r)
        });
        let created = if matches!(&job.action, Some(AndroidDeviceAction::Create { .. })) {
            result.as_ref().ok().and_then(|r| r.device_id.clone())
        } else {
            None
        };
        let (next, effect, output) = match result {
            Ok(r) => (
                OperationState::Succeeded,
                Effect::Complete,
                OperationResult::AndroidManagement(Box::new(r)),
            ),
            Err(code) if !job.started.load(Ordering::SeqCst) => (
                if authorized {
                    OperationState::Failed
                } else {
                    OperationState::Cancelled
                },
                Effect::None,
                OperationResult::Failure { code },
            ),
            Err(code) => (
                OperationState::OutcomeUnknown,
                Effect::Unknown,
                OperationResult::Failure { code },
            ),
        };
        let mut store = self.store.lock().map_err(|_| failure())?;
        store
            .record_result(&job.owner, &job.project, operation, &output)
            .map_err(|_| failure())?;
        store
            .transition(&job.owner, &job.project, operation, next, effect, now())
            .map_err(|_| failure())?;
        let owner = job.owner.clone();
        drop(store);
        if let Some(id) = created {
            let session = state.sessions.get_mut(&owner).ok_or_else(failure)?;
            session.grant.android_devices.insert(id.clone());
            session.view.android_device_ids.push(id);
        }
        state.android_management.remove(operation);
        Ok(())
    }
}
