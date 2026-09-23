use super::*;
use crate::{android::AndroidControl, artifacts::ArtifactFile, project_files::ProjectDirectory};
use receipts::{Effect, State as OperationState};

pub type AndroidInstallDispatch =
    Arc<dyn Fn(AndroidInstallRequest) -> Result<AndroidInstallResult, ErrorCode> + Send + Sync>;
pub struct AndroidInstallRequest {
    pub input: AndroidInstallInput,
    pub file: ArtifactFile,
    pub control: Arc<AndroidControl>,
    pub permit: NativePermit,
    pub project_directory: Arc<ProjectDirectory>,
    started: Arc<AtomicBool>,
}
impl AndroidInstallRequest {
    pub fn check(&self) -> Result<(), ErrorCode> {
        self.permit.check()?;
        self.control.check_generation(&self.input.generation)
    }
    /// Native dispatch calls this after verification and before submitting to
    /// the generation-checked installer. An uncertain result never authorizes replay.
    pub fn mark_dispatching(&self) -> Result<(), ErrorCode> {
        self.check()?;
        self.started.store(true, Ordering::SeqCst);
        self.check()
    }
}
pub(super) struct InstallJob {
    pairing: String,
    project: String,
    input: AndroidInstallInput,
    title: String,
    relative_path: String,
    byte_length: u32,
    file: Option<ArtifactFile>,
    control: Arc<AndroidControl>,
    pub(super) permit: NativePermit,
    started: Arc<AtomicBool>,
    pub(super) deadline: Instant,
    pub(super) running: bool,
}
impl Drop for InstallJob {
    fn drop(&mut self) {
        self.permit.revoke();
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingInstallView {
    operation_id: String,
    client_label: String,
    workspace_id: String,
    device_id: String,
    generation: String,
    title: String,
    relative_path: String,
    artifact_id: String,
    sha256: String,
    byte_length: u32,
    seconds_remaining: u64,
}
impl InstallJob {
    pub(super) fn view(&self, operation: &str, state: &State) -> PendingInstallView {
        PendingInstallView {
            operation_id: operation.into(),
            client_label: state
                .sessions
                .get(&self.pairing)
                .map(|s| s.view.client_label.clone())
                .unwrap_or_default(),
            workspace_id: self.input.workspace_id.clone(),
            device_id: self.input.device_id.clone(),
            generation: self.input.generation.clone(),
            title: self.title.clone(),
            relative_path: self.relative_path.clone(),
            artifact_id: self.input.artifact_id.clone(),
            sha256: self.input.sha256.clone(),
            byte_length: self.byte_length,
            seconds_remaining: self
                .deadline
                .saturating_duration_since(Instant::now())
                .as_secs(),
        }
    }
}
impl Broker {
    pub fn set_android_install_dispatch(&self, dispatch: AndroidInstallDispatch) -> io::Result<()> {
        *self
            .android_install_dispatch
            .lock()
            .map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    fn install_access(
        state: &State,
        owner: &str,
        input: &AndroidInstallInput,
    ) -> Result<Arc<ProjectDirectory>, ErrorCode> {
        Self::android_runtime_access(
            state,
            owner,
            &input.workspace_id,
            &input.panel_id,
            &input.device_id,
            Some(&input.generation),
        )?;
        let root = Self::project_file_access(state, owner, &input.workspace_id)?;
        if !state.sessions[owner]
            .grant
            .scopes
            .contains("android.install")
        {
            return Err(ErrorCode::ScopeDenied);
        }
        Ok(root)
    }
    pub(super) fn android_install(
        self: &Arc<Self>,
        owner: &str,
        input: AndroidInstallInput,
    ) -> Reply {
        use operations::storage_error;
        if !valid_id(&input.request_key)
            || !valid_id(&input.artifact_id)
            || !lomi_control_protocol::android::valid_device_id(&input.device_id)
            || !lomi_control_protocol::android::valid_device_id(&input.generation)
            || input.sha256.len() != 64
            || !input
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        if let Err(code) = Self::android_access(&state, owner, &input.workspace_id) {
            return error(code);
        }
        if let Err(code) = Self::project_file_access(&state, owner, &input.workspace_id) {
            return error(code);
        }
        let session = &state.sessions[owner];
        if !session.grant.scopes.contains("android.install")
            || !session.grant.android_devices.contains(&input.device_id)
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
            &input,
            &receipts::Target {
                workspace_id: &input.workspace_id,
                resource_id: &input.device_id,
                generation: &input.generation,
                revision: &input.sha256,
            },
        ) {
            Ok(h) => h,
            Err(e) => return storage_error(e),
        };
        let key = receipts::Key {
            pairing_id: owner,
            project_id: &project,
            retry_epoch: &input.retry_epoch,
            tool: "lomi_android_install_apk",
            request_key: &input.request_key,
        };
        match self.replay(&state, owner, &key, hash) {
            Ok(Some(reply)) => return reply,
            Err(reply) => return *reply,
            Ok(None) => {}
        }
        if let Err(code) = Self::install_access(&state, owner, &input) {
            return error(code);
        }
        if state.installs.len() >= 8
            || state
                .installs
                .values()
                .any(|j| j.input.device_id == input.device_id)
        {
            return error(ErrorCode::TargetBusy);
        }
        if self
            .android_install_dispatch
            .lock()
            .ok()
            .is_none_or(|d| d.is_none())
        {
            return error(ErrorCode::UnsupportedCapability);
        }
        let Ok(mut store) = self.store.lock() else {
            return error(ErrorCode::StorageUnavailable);
        };
        let file = match store.lease_artifact(owner, &project, &input.artifact_id, now()) {
            Ok(f) => f,
            Err(e) => return storage_error(e),
        };
        let ArtifactSource::Project(source) = &file.artifact.source else {
            return error(ErrorCode::ScopeDenied);
        };
        if source.workspace_id != input.workspace_id {
            return error(ErrorCode::TargetNotFound);
        }
        if file.artifact.sha256 != input.sha256
            || file.artifact.media_type != "application/vnd.android.package-archive"
        {
            return error(ErrorCode::RevisionConflict);
        }
        let reserved = match store.reserve(&key, hash, now()) {
            Ok(r) => r,
            Err(e) => return storage_error(e),
        };
        if !reserved.created {
            return Self::terminal_receipt(&state, owner, reserved.receipt);
        }
        let operation = reserved.receipt.operation_id;
        if let Err(e) = store.bind_workspace(owner, &project, &operation, &input.workspace_id) {
            return storage_error(e);
        }
        let receipt = match store.transition(
            owner,
            &project,
            &operation,
            OperationState::AwaitingUser,
            Effect::None,
            now(),
        ) {
            Ok(r) => r,
            Err(e) => return storage_error(e),
        };
        let deadline = Instant::now() + Duration::from_secs(120);
        let job = InstallJob {
            pairing: owner.into(),
            project,
            input: input.clone(),
            title: state
                .projection
                .panels
                .iter()
                .find(|p| p.id == input.panel_id)
                .map(|p| p.title.clone())
                .unwrap_or_default(),
            relative_path: source.relative_path.clone(),
            byte_length: file.artifact.byte_length,
            file: Some(file),
            control: state.android[&input.device_id].control.clone(),
            permit: NativePermit::until(deadline),
            started: Arc::new(AtomicBool::new(false)),
            deadline,
            running: false,
        };
        state.installs.insert(operation.clone(), job);
        self.schedule_install_expiry(operation, Duration::from_secs(120));
        Self::operation_reply(receipt)
    }
    fn schedule_install_expiry(self: &Arc<Self>, operation: String, duration: Duration) {
        let weak = Arc::downgrade(self);
        self.spawn_background(async move {
            tokio::time::sleep(duration).await;
            if let Some(broker) = weak.upgrade() {
                let worker = broker.clone();
                let _ = broker
                    .spawn_worker(move || {
                        let broker = worker;
                        if let Ok(mut state) = broker.lock_state() {
                            if state
                                .installs
                                .get(&operation)
                                .is_some_and(|j| j.deadline <= Instant::now())
                            {
                                if let Some(job) = state.installs.remove(&operation) {
                                    broker.settle_install(&operation, &job);
                                }
                            }
                        };
                    })
                    .await;
            }
        });
    }
    fn settle_install(&self, operation: &str, job: &InstallJob) {
        job.permit.revoke();
        if let Ok(mut store) = self.store.lock() {
            let _ = store.transition(
                &job.pairing,
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
    pub(super) fn end_installs(&self, state: &mut State, pairing: Option<&str>) {
        let ids: Vec<_> = state
            .installs
            .iter()
            .filter(|(_, j)| pairing.is_none_or(|id| j.pairing == id))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if let Some(job) = state.installs.remove(&id) {
                self.settle_install(&id, &job);
            }
        }
    }
    /// Only native Settings may decide this one-use approval. Its identity binds
    /// owner/project, target generation, immutable artifact hash and deadline.
    pub fn decide_install(self: &Arc<Self>, operation: &str, approve: bool) -> io::Result<()> {
        let mut state = self.lock_state().map_err(|_| failure())?;
        let job = state.installs.get(operation).ok_or_else(failure)?;
        if job.running {
            return Err(failure());
        }
        let valid = job.deadline > Instant::now()
            && job.permit.check().is_ok()
            && job.file.as_ref().is_some_and(|f| {
                f.artifact
                    .expires_at_seconds
                    .parse::<i64>()
                    .is_ok_and(|expires| expires > now())
            });
        let access = Self::install_access(&state, &job.pairing, &job.input);
        if !approve || !valid || access.is_err() {
            let job = state.installs.remove(operation).unwrap();
            self.settle_install(operation, &job);
            return Ok(());
        }
        let root = access.unwrap();
        root.check().map_err(|_| failure())?;
        if state.installs.values().filter(|j| j.running).count() >= 2 {
            return Err(failure());
        }
        let dispatch = self
            .android_install_dispatch
            .lock()
            .map_err(|_| failure())?
            .clone()
            .ok_or_else(failure)?;
        let mut store = self.store.lock().map_err(|_| failure())?;
        let receipt = store
            .get(&job.pairing, &job.project, operation)
            .map_err(|_| failure())?;
        if receipt.state != OperationState::AwaitingUser {
            return Err(failure());
        }
        store
            .transition(
                &job.pairing,
                &job.project,
                operation,
                OperationState::Queued,
                Effect::None,
                now(),
            )
            .map_err(|_| failure())?;
        store
            .transition(
                &job.pairing,
                &job.project,
                operation,
                OperationState::Running,
                Effect::None,
                now(),
            )
            .map_err(|_| failure())?;
        let job = state.installs.get_mut(operation).unwrap();
        job.deadline = Instant::now() + Duration::from_secs(180);
        job.permit = NativePermit::until(job.deadline);
        job.running = true;
        let request = AndroidInstallRequest {
            input: job.input.clone(),
            file: job.file.take().ok_or_else(failure)?,
            control: job.control.clone(),
            permit: job.permit.clone(),
            project_directory: root,
            started: job.started.clone(),
        };
        drop(store);
        drop(state);
        let op = operation.to_owned();
        self.schedule_install_expiry(op.clone(), Duration::from_secs(180));
        let broker = self.clone();
        drop(self.spawn_worker(move || {
            let result = dispatch(request);
            let _ = broker.finish_install(&op, result);
        }));
        Ok(())
    }
    fn finish_install(
        &self,
        operation: &str,
        result: Result<AndroidInstallResult, ErrorCode>,
    ) -> io::Result<()> {
        let mut state = self.lock_state().map_err(|_| failure())?;
        let job = state.installs.get(operation).ok_or_else(failure)?;
        if !job.running {
            return Err(failure());
        }
        let authorized = job.permit.check().is_ok()
            && Self::install_access(&state, &job.pairing, &job.input).is_ok();
        let result = result.and_then(|result| {
            if !authorized
                || !job.started.load(Ordering::SeqCst)
                || result.workspace_id != job.input.workspace_id
                || result.device_id != job.input.device_id
                || result.generation != job.input.generation
                || result.artifact_id != job.input.artifact_id
                || result.sha256 != job.input.sha256
                || result.installed == result.installer_failure.is_some()
                || result.package_name.as_ref().is_some_and(|p| p.len() > 255)
                || result
                    .previous_version
                    .as_ref()
                    .is_some_and(|v| v.len() > 128)
                || result.new_version.as_ref().is_some_and(|v| v.len() > 128)
                || result.installer_failure.as_ref().is_some_and(|e| {
                    e.len() > 128
                        || !e
                            .bytes()
                            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                })
            {
                return Err(ErrorCode::OutcomeUnknown);
            }
            Ok(result)
        });
        let (mut next, effect, output) = match result {
            Ok(result) => (
                if result.installed {
                    OperationState::Succeeded
                } else {
                    OperationState::Failed
                },
                if result.installed {
                    Effect::Complete
                } else {
                    Effect::None
                },
                OperationResult::AndroidInstall(Box::new(result)),
            ),
            Err(code) if !job.started.load(Ordering::SeqCst) => (
                OperationState::Failed,
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
        if next == OperationState::Failed
            && !authorized
            && store
                .get(&job.pairing, &job.project, operation)
                .map_err(|_| failure())?
                .state
                == OperationState::Cancelling
        {
            next = OperationState::Cancelled;
        }
        store
            .record_result(&job.pairing, &job.project, operation, &output)
            .map_err(|_| failure())?;
        store
            .transition(&job.pairing, &job.project, operation, next, effect, now())
            .map_err(|_| failure())?;
        drop(store);
        state.installs.remove(operation);
        Ok(())
    }
}
