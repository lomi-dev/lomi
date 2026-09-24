use super::{operations::UiMutation, *};
use crate::browser::BrowserControl;

pub struct BrowserDownloadFailure {
    pub code: ErrorCode,
    pub no_effect: bool,
}

impl Broker {
    pub(super) fn download_access(
        state: &State,
        owner: &str,
        input: &BrowserDownloadInput,
    ) -> Result<Arc<BrowserControl>, ErrorCode> {
        let target = Self::browser_target(
            state,
            owner,
            &input.workspace_id,
            &input.panel_id,
            &input.browser_generation,
            "browser.download",
        )?;
        if !state.sessions[owner].grant.scopes.contains("browser.read") {
            return Err(ErrorCode::ScopeDenied);
        }
        target.control.check_document(&input.navigation_id)?;
        let current = lomi_control_protocol::browser::address(&target.control.document_url()?)
            .map_err(|_| ErrorCode::ScopeDenied)?;
        let download = lomi_control_protocol::browser::address(&input.url)
            .map_err(|_| ErrorCode::ScopeDenied)?;
        if !matches!(download.scheme(), "http" | "https")
            || download.fragment().is_some()
            || download.origin() != current.origin()
            || !target.control.permits(download.as_str())
        {
            return Err(ErrorCode::ScopeDenied);
        }
        Ok(target.control.clone())
    }
    pub(super) fn download_browser(
        self: &Arc<Self>,
        owner: &str,
        input: BrowserDownloadInput,
    ) -> Reply {
        if !(1..=4 * 1024 * 1024).contains(&input.max_bytes)
            || !valid_id(&input.request_key)
            || input.expected_revision.parse::<u64>().is_err()
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        // Replay is source-authorized by its stored artifact; a later navigation
        // cannot cause the already completed GET to run again.
        let target = match Self::browser_target(
            &state,
            owner,
            &input.workspace_id,
            &input.panel_id,
            &input.browser_generation,
            "browser.download",
        ) {
            Ok(target) => target,
            Err(code) => return error(code),
        };
        let session = &state.sessions[owner];
        if !session.grant.scopes.contains("browser.read") {
            return error(ErrorCode::ScopeDenied);
        }
        if session.retry_epoch != input.retry_epoch {
            return error(ErrorCode::RetryWindowExpired);
        }
        let project = session
            .grant
            .workspace(&input.workspace_id)
            .unwrap()
            .project_id
            .clone();
        let hash = match receipts::fingerprint(
            &input,
            &receipts::Target {
                workspace_id: &input.workspace_id,
                resource_id: &input.panel_id,
                generation: &target.control.generation,
                revision: &input.navigation_id,
            },
        ) {
            Ok(hash) => hash,
            Err(e) => return operations::storage_error(e),
        };
        let key = receipts::Key {
            pairing_id: owner,
            retry_epoch: &input.retry_epoch,
            project_id: &project,
            tool: "lomi_browser_download",
            request_key: &input.request_key,
        };
        match self.replay(&state, owner, &key, hash) {
            Ok(Some(reply)) => return reply,
            Err(reply) => return *reply,
            Ok(None) => {}
        }
        if let Err(code) = Self::download_access(&state, owner, &input) {
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
                tool: "lomi_browser_download",
                hash,
                action: UiAction::DownloadBrowser(input),
            },
        )
    }
    pub fn execute_browser_download(
        &self,
        operation: &str,
        nonce: &str,
        download: impl FnOnce(
            Arc<BrowserControl>,
            BrowserDownloadInput,
            NativePermit,
        ) -> Result<Vec<u8>, BrowserDownloadFailure>,
    ) -> io::Result<()> {
        let (input, owner, project, permit) = {
            let mut state = self.lock_state().map_err(|_| failure())?;
            let work = state.work.get(operation).ok_or_else(failure)?;
            if !work.claimed
                || work.native_committed
                || work.command.nonce != nonce
                || work.command.ui_epoch != state.projection.ui_epoch
            {
                return Err(failure());
            }
            let UiAction::DownloadBrowser(input) = &work.command.action else {
                return Err(failure());
            };
            let tuple = (
                input.clone(),
                work.pairing.clone(),
                work.project.clone(),
                work.native_permit.clone(),
            );
            state.work.get_mut(operation).unwrap().native_committed = true;
            tuple
        };
        let mut may_have_started = false;
        let mut reservation = None;
        let result = (|| -> Result<Artifact, ErrorCode> {
            permit.check()?;
            let (control, source) = {
                let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
                let control = Self::download_access(&state, &owner, &input)?;
                let url = lomi_control_protocol::browser::address(&input.url)
                    .map_err(|_| ErrorCode::ScopeDenied)?;
                let source = ArtifactSource::Browser(BrowserArtifactSource {
                    workspace_id: input.workspace_id.clone(),
                    panel_id: input.panel_id.clone(),
                    browser_generation: input.browser_generation.clone(),
                    profile_id: control.profile_id.clone(),
                    navigation_id: input.navigation_id.clone(),
                    origin: url.origin().ascii_serialization(),
                    required_scope: "browser.download".into(),
                });
                (control, source)
            };
            reservation = Some(
                self.store
                    .lock()
                    .map_err(|_| ErrorCode::StorageUnavailable)?
                    .reserve_artifact(&owner, &project, &source, input.max_bytes as usize, now())
                    .map_err(|e| {
                        if e == receipts::Error::ResourceExhausted {
                            ErrorCode::ResourceExhausted
                        } else {
                            ErrorCode::StorageUnavailable
                        }
                    })?,
            );
            permit.check()?;
            may_have_started = true;
            let bytes = download(control.clone(), input.clone(), permit.clone()).map_err(|e| {
                may_have_started = !e.no_effect;
                e.code
            })?;
            if bytes.len() > input.max_bytes as usize {
                return Err(ErrorCode::ArtifactTooLarge);
            }
            permit.check()?;
            let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let current = Self::download_access(&state, &owner, &input)?;
            if !Arc::ptr_eq(&control, &current) {
                return Err(ErrorCode::StaleGeneration);
            }
            let artifact = self
                .store
                .lock()
                .map_err(|_| ErrorCode::StorageUnavailable)?
                .commit_browser_download(reservation.as_ref().unwrap(), &bytes, now())
                .map_err(|_| ErrorCode::StorageUnavailable)?;
            permit.check()?;
            Self::artifact_source_access(&state, &owner, &artifact)?;
            Ok(artifact)
        })();
        if result.is_err() {
            if let Some(reservation) = &reservation {
                if let Ok(mut store) = self.store.lock() {
                    let _ = store.abandon_artifact(reservation);
                }
            }
        }
        use receipts::{Effect, State as OperationState};
        let mut state = self.lock_state().map_err(|_| failure())?;
        let work = state
            .work
            .get(operation)
            .filter(|w| w.command.nonce == nonce)
            .ok_or_else(failure)?;
        let result = if permit.check().is_err() {
            Err(ErrorCode::ControlRevoked)
        } else {
            result
        };
        let (mut next, effect, output) = match result {
            Ok(artifact) => (
                OperationState::Succeeded,
                Effect::Complete,
                OperationResult::BrowserDownloaded(Box::new(artifact)),
            ),
            Err(code) => (
                if may_have_started {
                    OperationState::OutcomeUnknown
                } else {
                    OperationState::Failed
                },
                if may_have_started {
                    Effect::Unknown
                } else {
                    Effect::None
                },
                OperationResult::Failure { code },
            ),
        };
        let mut store = self.store.lock().map_err(|_| failure())?;
        if next == OperationState::Failed
            && store
                .get(&owner, &project, operation)
                .map_err(|_| failure())?
                .state
                == OperationState::Cancelling
        {
            next = OperationState::Cancelled;
        }
        store
            .record_result(&work.pairing, &work.project, operation, &output)
            .map_err(|_| failure())?;
        store
            .transition(&work.pairing, &work.project, operation, next, effect, now())
            .map_err(|_| failure())?;
        drop(store);
        state.work.remove(operation);
        Ok(())
    }
}
