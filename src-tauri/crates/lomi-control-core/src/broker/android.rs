use super::*;

pub type AndroidListDispatch =
    Arc<dyn Fn(AndroidListRequest) -> Result<AndroidDevices, ErrorCode> + Send + Sync>;
pub struct AndroidListRequest {
    pub devices: Vec<String>,
    pub all_devices: bool,
    authorization: Arc<AtomicU64>,
    policy_revision: u64,
    connected: Arc<AtomicBool>,
    deadline: Instant,
}
impl AndroidListRequest {
    pub fn check(&self) -> Result<(), ErrorCode> {
        if self.authorization.load(Ordering::SeqCst) != self.policy_revision
            || !self.connected.load(Ordering::SeqCst)
        {
            return Err(ErrorCode::ControlRevoked);
        }
        if Instant::now() >= self.deadline {
            return Err(ErrorCode::DeadlineExceeded);
        }
        Ok(())
    }
}

impl Broker {
    pub fn set_android_list_dispatch(&self, dispatch: AndroidListDispatch) -> io::Result<()> {
        *self.android_list_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }

    pub(super) fn android_access(
        state: &State,
        owner: &str,
        workspace: &str,
    ) -> Result<Vec<String>, ErrorCode> {
        let session = state
            .sessions
            .get(owner)
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .ok_or(ErrorCode::ControlRevoked)?;
        if session.grant.workspace(workspace).is_none()
            || !state
                .projection
                .workspaces
                .iter()
                .any(|w| w.id == workspace && session.grant.permits(w))
        {
            return Err(ErrorCode::TargetNotFound);
        }
        if !session.grant.scopes.contains("android.read") {
            return Err(ErrorCode::ScopeDenied);
        }
        let mut devices: Vec<_> = session.grant.android_devices.iter().cloned().collect();
        devices.sort();
        Ok(devices)
    }

    pub(super) fn android_list(&self, owner: &str, input: AndroidListInput) -> Reply {
        let request = {
            let Ok(state) = self.lock_state() else {
                return error(ErrorCode::ControlRevoked);
            };
            let devices = match Self::android_access(&state, owner, &input.workspace_id) {
                Ok(devices) => devices,
                Err(code) => return error(code),
            };
            AndroidListRequest {
                devices,
                all_devices: state.sessions[owner].grant.yolo,
                authorization: self.authorization.clone(),
                policy_revision: state.policy_revision,
                connected: state.sessions[owner].alive.clone(),
                deadline: Instant::now() + Duration::from_secs(5),
            }
        };
        let Some(dispatch) = self
            .android_list_dispatch
            .lock()
            .ok()
            .and_then(|d| d.clone())
        else {
            return error(ErrorCode::HostUnqualified);
        };
        let devices = request.devices.clone();
        if let Err(code) = request.check() {
            return error(code);
        }
        let result = match dispatch(request) {
            Ok(result) => result,
            Err(code) => return error(code),
        };
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        let current = match Self::android_access(&state, owner, &input.workspace_id) {
            Ok(devices) => devices,
            Err(code) => return error(code),
        };
        if current != devices {
            return error(ErrorCode::ControlRevoked);
        }
        let mut seen = HashSet::new();
        let yolo = state.sessions[owner].grant.yolo;
        if result.items.len() > 16
            || result.devices_revision.parse::<u64>().is_err()
            || result.items.iter().any(|d| {
                (!yolo && !devices.contains(&d.device_id))
                    || !lomi_control_protocol::android::valid_device_id(&d.device_id)
                    || !seen.insert(&d.device_id)
                    || d.name.len() > 256
                    || d.name.chars().any(char::is_control)
                    || d.generation.as_ref().is_some_and(|g| !valid_id(g))
            })
        {
            return error(ErrorCode::OutcomeUnknown);
        }
        if yolo {
            let mut ids: Vec<_> = result.items.iter().map(|d| d.device_id.clone()).collect();
            ids.sort();
            state.sessions.get_mut(owner).unwrap().grant.android_devices =
                ids.iter().cloned().collect();
            state
                .sessions
                .get_mut(owner)
                .unwrap()
                .view
                .android_device_ids = ids;
        }
        Reply::ok(Data::AndroidDevices {
            workspace_id: input.workspace_id,
            devices: result,
        })
    }
}

impl Broker {
    pub(super) fn android_open(self: &Arc<Self>, owner: &str, input: AndroidOpenInput) -> Reply {
        use super::operations::{storage_error, UiMutation};
        if !lomi_control_protocol::android::valid_device_id(&input.device_id)
            || !valid_id(&input.request_key)
            || input.expected_revision.parse::<u64>().is_err()
        {
            return error(ErrorCode::ResourceExhausted);
        }
        // Resolve the durable identity and deduplicate before asking the native manager.
        let (project, hash) = {
            let Ok(state) = self.lock_state() else {
                return error(ErrorCode::ControlRevoked);
            };
            if let Err(code) = Self::android_access(&state, owner, &input.workspace_id) {
                return error(code);
            }
            let session = &state.sessions[owner];
            if !session.grant.permits_android_device(&input.device_id)
                || !session.grant.scopes.contains("panel.create")
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
                    generation: &state.projection.ui_epoch,
                    revision: &input.expected_revision,
                },
            ) {
                Ok(hash) => hash,
                Err(e) => return storage_error(e),
            };
            let key = receipts::Key {
                pairing_id: owner,
                project_id: &project,
                retry_epoch: &input.retry_epoch,
                request_key: &input.request_key,
                tool: "lomi_android_open",
            };
            match self.replay(&state, owner, &key, hash) {
                Ok(Some(reply)) => return reply,
                Err(reply) => return *reply,
                Ok(None) => {}
            }
            (project, hash)
        };
        let devices = match self.android_list(
            owner,
            AndroidListInput {
                workspace_id: input.workspace_id.clone(),
            },
        ) {
            Reply::Ok {
                data: Data::AndroidDevices { devices, .. },
                ..
            } => devices,
            reply => return reply,
        };
        let Some(device) = devices
            .items
            .into_iter()
            .find(|d| d.device_id == input.device_id)
        else {
            return error(ErrorCode::TargetNotFound);
        };
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        let devices = match Self::android_access(&state, owner, &input.workspace_id) {
            Ok(devices) => devices,
            Err(code) => return error(code),
        };
        if !devices.contains(&input.device_id) {
            return error(ErrorCode::ScopeDenied);
        }
        let Ok(panel) = new_id() else {
            return error(ErrorCode::ResourceExhausted);
        };
        let panel = format!(
            "{}-{}-{}-{}-{}",
            &panel[..8],
            &panel[8..12],
            &panel[12..16],
            &panel[16..20],
            &panel[20..]
        );
        let action = UiAction::CreateAndroid {
            workspace_id: input.workspace_id.clone(),
            panel_id: panel,
            device_id: input.device_id,
            title: device.name,
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
                tool: "lomi_android_open",
                hash,
                action,
            },
        )
    }
}

pub(super) struct OwnedAndroid {
    pub owner: String,
    pub workspaces: HashSet<String>,
    pub control: Arc<crate::android::AndroidControl>,
}

pub struct AndroidRuntimeDispatch {
    pub control: Arc<crate::android::AndroidControl>,
    pub permit: NativePermit,
    pub workspace: String,
    pub stop_generation: Option<String>,
}

impl Broker {
    pub(super) fn android_runtime_access(
        state: &State,
        owner: &str,
        workspace: &str,
        panel: &str,
        device: &str,
        generation: Option<&str>,
    ) -> Result<(), ErrorCode> {
        Self::android_access(state, owner, workspace)?;
        let session = &state.sessions[owner];
        if !session.grant.scopes.contains("android.control")
            || !session.grant.permits_android_device(device)
        {
            return Err(ErrorCode::ScopeDenied);
        }
        if !state.projection.panels.iter().any(|p| {
            p.id == panel
                && p.workspace_id == workspace
                && p.kind == "android"
                && p.android_device_id.as_deref() == Some(device)
        }) {
            return Err(ErrorCode::TargetNotFound);
        }
        if let Some(target) = state
            .android
            .get(device)
            .filter(|t| t.control.check().is_ok())
        {
            if target.owner != owner || !target.workspaces.contains(workspace) {
                return Err(ErrorCode::TargetBusy);
            }
        }
        if let Some(generation) = generation {
            let target = state
                .android
                .get(device)
                .filter(|t| t.owner == owner && t.workspaces.contains(workspace))
                .ok_or(ErrorCode::TargetNotFound)?;
            target.control.check_generation(generation)?;
        }
        Ok(())
    }

    pub(super) fn android_runtime(
        self: &Arc<Self>,
        owner: &str,
        input: AndroidStartInput,
        generation: Option<String>,
    ) -> Reply {
        use super::operations::{storage_error, UiMutation};
        if !valid_id(&input.request_key)
            || input.expected_revision.parse::<u64>().is_err()
            || !lomi_control_protocol::android::valid_device_id(&input.device_id)
            || generation
                .as_ref()
                .is_some_and(|g| !lomi_control_protocol::android::valid_device_id(g))
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
        if !session.grant.scopes.contains("android.control")
            || !session.grant.permits_android_device(&input.device_id)
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
            &(&input, &generation, &root.project_path),
            &receipts::Target {
                workspace_id: &input.workspace_id,
                resource_id: &input.device_id,
                generation: generation.as_deref().unwrap_or(&state.projection.ui_epoch),
                revision: &input.expected_revision,
            },
        ) {
            Ok(h) => h,
            Err(e) => return storage_error(e),
        };
        let tool = if generation.is_some() {
            "lomi_android_stop"
        } else {
            "lomi_android_start"
        };
        let key = receipts::Key {
            pairing_id: owner,
            project_id: &project,
            retry_epoch: &input.retry_epoch,
            request_key: &input.request_key,
            tool,
        };
        match self.replay(&state, owner, &key, hash) {
            Ok(Some(reply)) => return reply,
            Err(reply) => return *reply,
            Ok(None) => {}
        }
        if let Err(code) = Self::android_runtime_access(
            &state,
            owner,
            &input.workspace_id,
            &input.panel_id,
            &input.device_id,
            generation.as_deref(),
        ) {
            return error(code);
        }
        let action = UiAction::AndroidRuntime {
            workspace_id: input.workspace_id.clone(),
            panel_id: input.panel_id,
            device_id: input.device_id,
            generation,
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
                tool,
                hash,
                action,
            },
        )
    }

    pub fn authorize_android_runtime(
        &self,
        operation: &str,
        nonce: &str,
    ) -> Result<AndroidRuntimeDispatch, ErrorCode> {
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
        let UiAction::AndroidRuntime {
            workspace_id,
            panel_id,
            device_id,
            generation,
        } = &work.command.action
        else {
            return Err(ErrorCode::ScopeDenied);
        };
        Self::android_runtime_access(
            &state,
            &work.pairing,
            workspace_id,
            panel_id,
            device_id,
            generation.as_deref(),
        )?;
        let (owner, workspace, device, stop_generation, permit) = (
            work.pairing.clone(),
            workspace_id.clone(),
            device_id.clone(),
            generation.clone(),
            work.native_permit.clone(),
        );
        let control = match state
            .android
            .get(&device)
            .filter(|target| target.control.check().is_ok())
        {
            Some(target) => target.control.clone(),
            None => {
                state.android.retain(|_, t| t.control.check().is_ok());
                if state.android.len() >= 16 {
                    return Err(ErrorCode::ResourceExhausted);
                }
                let control = Arc::new(crate::android::AndroidControl::new(
                    device.clone(),
                    self.authorization.clone(),
                    state.policy_revision,
                    state.sessions[&owner].alive.clone(),
                ));
                state.android.insert(
                    device,
                    OwnedAndroid {
                        owner,
                        workspaces: HashSet::from([workspace.clone()]),
                        control: control.clone(),
                    },
                );
                control
            }
        };
        let selected = state.projection.panels.iter().find(|p| {
            Some(&p.id) == state.projection.focused_panel_id.as_ref()
                && p.workspace_id == workspace
                && p.android_device_id.as_ref() == Some(&control.device)
        });
        control.select(selected.map(|p| p.id.clone()));
        state.work.get_mut(operation).unwrap().native_committed = true;
        Ok(AndroidRuntimeDispatch {
            control,
            permit,
            workspace,
            stop_generation,
        })
    }

    /// Only the native manager calls this method. Main-renderer ACKs cannot attest a boot or stop.
    pub fn finish_android_runtime(
        &self,
        operation: &str,
        nonce: &str,
        result: Result<AndroidRuntimeResult, ErrorCode>,
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
        let UiAction::AndroidRuntime {
            workspace_id,
            panel_id,
            device_id,
            generation,
        } = &work.command.action
        else {
            return Err(failure());
        };
        let mut result = result;
        if work.native_permit.check().is_err() {
            result = Err(ErrorCode::OutcomeUnknown);
        }
        if let Ok(output) = &result {
            Self::android_runtime_access(
                &state,
                &work.pairing,
                workspace_id,
                panel_id,
                device_id,
                generation.as_deref(),
            )
            .map_err(|_| failure())?;
            let target = state.android.get(device_id).ok_or_else(failure)?;
            if !work.native_committed
                || output.workspace_id != *workspace_id
                || output.device_id != *device_id
                || output.stopped != generation.is_some()
                || output.ready == output.stopped
                || target.control.check_generation(&output.generation).is_err()
            {
                return Err(failure());
            }
        }
        let (next, effect, output) = match result {
            Ok(result) => (
                OperationState::Succeeded,
                Effect::Complete,
                OperationResult::AndroidRuntime(result),
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
        let stopped = matches!(&output, OperationResult::AndroidRuntime(r) if r.stopped);
        let device = device_id.clone();
        let mut store = self.store.lock().map_err(|_| failure())?;
        store
            .record_result(&work.pairing, &work.project, operation, &output)
            .map_err(|_| failure())?;
        store
            .transition(&work.pairing, &work.project, operation, next, effect, now())
            .map_err(|_| failure())?;
        drop(store);
        state.work.remove(operation);
        if stopped {
            if let Some(target) = state.android.remove(&device) {
                target.control.revoke();
            }
        }
        Ok(())
    }
}
