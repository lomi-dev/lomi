use super::{
    operations::{UiMutation, Work},
    *,
};
use crate::{artifacts::ArtifactFile, browser::BrowserControl};

pub struct BrowserUploadApproval {
    pub input: BrowserUploadInput,
    pub target: BrowserUploadTarget,
    pub file: ArtifactFile,
    pub control: Arc<BrowserControl>,
    pub permit: NativePermit,
    pub check: Arc<dyn Fn() -> Result<(), ErrorCode> + Send + Sync>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingBrowserUpload {
    operation_id: String,
    client_label: String,
    workspace_id: String,
    panel_id: String,
    element_ref: String,
    target: BrowserUploadTarget,
    file_name: String,
    byte_length: u32,
    sha256: String,
    seconds_remaining: u64,
}
impl Broker {
    pub(super) fn upload_access(
        state: &State,
        owner: &str,
        input: &BrowserUploadInput,
    ) -> Result<Arc<BrowserControl>, ErrorCode> {
        let target = Self::browser_target(
            state,
            owner,
            &input.workspace_id,
            &input.panel_id,
            &input.browser_generation,
            "browser.upload",
        )?;
        if !["browser.read", "browser.interact"]
            .iter()
            .all(|scope| state.sessions[owner].grant.scopes.contains(*scope))
        {
            return Err(ErrorCode::ScopeDenied);
        }
        if target.control.lease() != Some(input.lease_id.as_str()) {
            return Err(ErrorCode::ControlRevoked);
        }
        target.control.require_renderable()?;
        target
            .control
            .check_snapshot(&input.snapshot_id, &input.navigation_id)?;
        Ok(target.control.clone())
    }
    fn upload_work<'a>(
        state: &'a State,
        operation: &str,
        nonce: Option<&str>,
    ) -> Result<&'a Work, ErrorCode> {
        let work = state.work.get(operation).ok_or(ErrorCode::ControlRevoked)?;
        if !work.claimed
            || work.native_committed
            || work.command.ui_epoch != state.projection.ui_epoch
            || nonce.is_some_and(|n| n != work.command.nonce)
            || !matches!(work.command.action, UiAction::UploadBrowser(_))
        {
            return Err(ErrorCode::ControlRevoked);
        }
        work.native_permit.check()?;
        Ok(work)
    }
    fn upload_artifact_access(
        state: &State,
        owner: &str,
        input: &BrowserUploadInput,
        artifact: &Artifact,
    ) -> Result<(), ErrorCode> {
        if artifact.source.workspace() != input.workspace_id {
            return Err(ErrorCode::TargetNotFound);
        }
        Self::artifact_source_access(state, owner, artifact)?;
        if artifact.sha256 != input.expected_sha256 || artifact.id != input.artifact_id {
            return Err(ErrorCode::RevisionConflict);
        }
        if artifact.byte_length > 4 * 1024 * 1024 {
            return Err(ErrorCode::ArtifactTooLarge);
        }
        if artifact
            .expires_at_seconds
            .parse::<i64>()
            .map_or(true, |expires| expires <= now())
        {
            return Err(ErrorCode::TargetNotFound);
        }
        Ok(())
    }
    pub(super) fn upload_browser(
        self: &Arc<Self>,
        owner: &str,
        input: BrowserUploadInput,
    ) -> Reply {
        if ![
            &input.request_key,
            &input.snapshot_id,
            &input.element_ref,
            &input.frame_id,
            &input.artifact_id,
        ]
        .iter()
        .all(|id| valid_id(id))
            || input.expected_revision.parse::<u64>().is_err()
            || input.expected_sha256.len() != 64
            || !input
                .expected_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || input.file_name.is_empty()
            || input.file_name.len() > 255
            || matches!(input.file_name.as_str(), "." | "..")
            || input
                .file_name
                .chars()
                .any(|c| c.is_control() || c == '/' || c == '\\')
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        let session = &state.sessions[owner];
        let Some(root) = session.grant.workspace(&input.workspace_id) else {
            return error(ErrorCode::TargetNotFound);
        };
        if session.retry_epoch != input.retry_epoch {
            return error(ErrorCode::RetryWindowExpired);
        }
        if !["browser.read", "browser.interact", "browser.upload"]
            .iter()
            .all(|s| session.grant.scopes.contains(*s))
        {
            return error(ErrorCode::ScopeDenied);
        }
        let project = root.project_id.clone();
        let hash = match receipts::fingerprint(
            &input,
            &receipts::Target {
                workspace_id: &input.workspace_id,
                resource_id: &input.panel_id,
                generation: &input.browser_generation,
                revision: &input.navigation_id,
            },
        ) {
            Ok(h) => h,
            Err(e) => return operations::storage_error(e),
        };
        let key = receipts::Key {
            pairing_id: owner,
            project_id: &project,
            retry_epoch: &input.retry_epoch,
            tool: "lomi_browser_upload",
            request_key: &input.request_key,
        };
        match self.replay(&state, owner, &key, hash) {
            Ok(Some(reply)) => return reply,
            Err(reply) => return *reply,
            Ok(None) => {}
        }
        if let Err(code) = Self::upload_access(&state, owner, &input) {
            return error(code);
        }
        let artifact = match self.store.lock().ok().and_then(|s| {
            s.artifact_metadata(owner, &project, &input.artifact_id, now())
                .ok()
        }) {
            Some(a) => a,
            None => return error(ErrorCode::TargetNotFound),
        };
        if let Err(code) = Self::upload_artifact_access(&state, owner, &input, &artifact) {
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
                tool: "lomi_browser_upload",
                hash,
                action: UiAction::UploadBrowser(input),
            },
        )
    }
    pub fn prepare_browser_upload(
        self: &Arc<Self>,
        operation: &str,
        nonce: &str,
        prepare: impl FnOnce(
            Arc<BrowserControl>,
            &BrowserUploadInput,
            NativePermit,
        ) -> Result<BrowserUploadTarget, ErrorCode>,
    ) -> Result<NativePermit, ErrorCode> {
        let (input, control, owner, file, permit) = {
            let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let work = Self::upload_work(&state, operation, Some(nonce))?;
            if work.browser_upload.is_some() {
                return Err(ErrorCode::ControlRevoked);
            }
            let UiAction::UploadBrowser(input) = &work.command.action else {
                unreachable!()
            };
            let control = Self::upload_access(&state, &work.pairing, input)?;
            let file = self
                .store
                .lock()
                .map_err(|_| ErrorCode::StorageUnavailable)?
                .lease_artifact(&work.pairing, &work.project, &input.artifact_id, now())
                .map_err(|_| ErrorCode::TargetNotFound)?;
            Self::upload_artifact_access(&state, &work.pairing, input, &file.artifact)?;
            (
                input.clone(),
                control,
                work.pairing.clone(),
                file,
                work.native_permit.clone(),
            )
        };
        let target = prepare(control.clone(), &input, permit.clone())?;
        let current = lomi_control_protocol::browser::address(&control.document_url()?)
            .map_err(|_| ErrorCode::ScopeDenied)?;
        let document = lomi_control_protocol::browser::address(&target.document_url)
            .map_err(|_| ErrorCode::ScopeDenied)?;
        if target.frame_id != input.frame_id
            || target.origin != current.origin().ascii_serialization()
            || document.origin() != current.origin()
            || target.document_url.len() > 8192
            || target.label.len() > 2048
        {
            return Err(ErrorCode::ScopeDenied);
        }
        let weak = Arc::downgrade(self);
        let expected_input = input.clone();
        let expected_artifact = file.artifact.clone();
        let expected_control = control.clone();
        let live = permit.clone();
        let check: Arc<dyn Fn() -> Result<(), ErrorCode> + Send + Sync> = Arc::new(move || {
            live.check()?;
            let broker = weak.upgrade().ok_or(ErrorCode::ControlRevoked)?;
            let state = broker.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let current = Self::upload_access(&state, &owner, &expected_input)?;
            if !Arc::ptr_eq(&current, &expected_control) {
                return Err(ErrorCode::StaleGeneration);
            }
            Self::upload_artifact_access(&state, &owner, &expected_input, &expected_artifact)
        });
        check()?;
        let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        let work = Self::upload_work(&state, operation, Some(nonce))?;
        if work.browser_upload.is_some() {
            return Err(ErrorCode::ControlRevoked);
        }
        state.work.get_mut(operation).unwrap().browser_upload = Some(BrowserUploadApproval {
            input,
            target,
            file,
            control,
            permit: permit.clone(),
            check,
        });
        Ok(permit)
    }
    pub(super) fn pending_browser_uploads(
        state: &State,
        authorized: bool,
    ) -> Vec<PendingBrowserUpload> {
        state
            .work
            .iter()
            .filter_map(|(id, work)| {
                if !authorized
                    || !work.claimed
                    || work.native_committed
                    || work.native_permit.check().is_err()
                {
                    return None;
                }
                let approval = work.browser_upload.as_ref()?;
                if Self::upload_access(state, &work.pairing, &approval.input).is_err()
                    || Self::upload_artifact_access(
                        state,
                        &work.pairing,
                        &approval.input,
                        &approval.file.artifact,
                    )
                    .is_err()
                {
                    return None;
                }
                Some(PendingBrowserUpload {
                    operation_id: id.clone(),
                    client_label: state.sessions[&work.pairing].view.client_label.clone(),
                    workspace_id: work.workspace.clone(),
                    panel_id: approval.input.panel_id.clone(),
                    element_ref: approval.input.element_ref.clone(),
                    target: approval.target.clone(),
                    file_name: approval.input.file_name.clone(),
                    byte_length: approval.file.artifact.byte_length,
                    sha256: approval.file.artifact.sha256.clone(),
                    seconds_remaining: work
                        .deadline
                        .saturating_duration_since(Instant::now())
                        .as_secs(),
                })
            })
            .collect()
    }
    /// Settings alone consumes the prepared approval. Renderer acknowledgements
    /// cannot publish an upload result or provide its bytes/target/hash.
    pub fn decide_browser_upload(
        &self,
        operation: &str,
        approve: bool,
        dispatch: impl FnOnce(BrowserUploadApproval) -> Result<(), BrowserDownloadFailure>,
    ) -> Result<(), ErrorCode> {
        let (approval, nonce, artifact, input, target, permit, check) = {
            let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let work = Self::upload_work(&state, operation, None)?;
            if work.browser_upload.is_none() {
                return Err(ErrorCode::UiNotReady);
            }
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
            if store
                .get(&work.pairing, &work.project, operation)
                .map_err(|_| ErrorCode::StorageUnavailable)?
                .state
                != receipts::State::AwaitingUser
            {
                return Err(ErrorCode::ControlRevoked);
            }
            for next in [receipts::State::Queued, receipts::State::Running] {
                store
                    .transition(
                        &work.pairing,
                        &work.project,
                        operation,
                        next,
                        receipts::Effect::None,
                        now(),
                    )
                    .map_err(|_| ErrorCode::StorageUnavailable)?;
            }
            drop(store);
            let work = state.work.get_mut(operation).unwrap();
            work.native_committed = true;
            let approval = work.browser_upload.take().unwrap();
            let tuple = (
                work.command.nonce.clone(),
                approval.file.artifact.clone(),
                approval.input.clone(),
                approval.target.clone(),
                approval.permit.clone(),
                approval.check.clone(),
            );
            (
                approval, tuple.0, tuple.1, tuple.2, tuple.3, tuple.4, tuple.5,
            )
        };
        let result = check()
            .map_err(|code| BrowserDownloadFailure {
                code,
                no_effect: true,
            })
            .and_then(|()| dispatch(approval));
        let result = result.and_then(|()| {
            check().map_err(|code| BrowserDownloadFailure {
                code,
                no_effect: false,
            })
        });
        let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        let work = state
            .work
            .get(operation)
            .filter(|w| w.command.nonce == nonce)
            .ok_or(ErrorCode::ControlRevoked)?;
        let result = result.and_then(|()| {
            permit.check().map_err(|code| BrowserDownloadFailure {
                code,
                no_effect: false,
            })
        });
        use receipts::{Effect, State as OperationState};
        let (next, effect, output) = match result {
            Ok(()) => (
                OperationState::Succeeded,
                Effect::Complete,
                OperationResult::BrowserUploaded(Box::new(BrowserUploaded {
                    workspace_id: input.workspace_id,
                    panel_id: input.panel_id,
                    browser_generation: input.browser_generation,
                    navigation_id: input.navigation_id,
                    snapshot_id: input.snapshot_id,
                    element_ref: input.element_ref,
                    target,
                    file_name: input.file_name,
                    artifact: Box::new(artifact),
                    input_mode: "synthetic_dom".into(),
                    dispatched: true,
                })),
            ),
            Err(failure) => (
                if failure.no_effect {
                    OperationState::Failed
                } else {
                    OperationState::OutcomeUnknown
                },
                if failure.no_effect {
                    Effect::None
                } else {
                    Effect::Unknown
                },
                OperationResult::Failure { code: failure.code },
            ),
        };
        let mut store = self
            .store
            .lock()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        let next = if next == OperationState::Failed
            && store
                .get(&work.pairing, &work.project, operation)
                .map_err(|_| ErrorCode::StorageUnavailable)?
                .state
                == OperationState::Cancelling
        {
            OperationState::Cancelled
        } else {
            next
        };
        store
            .record_result(&work.pairing, &work.project, operation, &output)
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        store
            .transition(&work.pairing, &work.project, operation, next, effect, now())
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        drop(store);
        state.work.remove(operation);
        Ok(())
    }
}
