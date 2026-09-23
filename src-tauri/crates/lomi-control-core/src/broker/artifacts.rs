use super::*;
use crate::{artifacts::MAX_IMAGE_BYTES, browser::BrowserControl};
use base64::Engine;

pub struct BrowserCapture {
    pub bytes: Vec<u8>,
    pub geometry: ImageGeometry,
    pub url: String,
}
pub type BrowserCaptureDispatch = Arc<
    dyn Fn(
            Arc<BrowserControl>,
            BrowserScreenshotInput,
            Instant,
        ) -> Result<BrowserCapture, ErrorCode>
        + Send
        + Sync,
>;
impl Broker {
    pub fn set_browser_capture_dispatch(&self, dispatch: BrowserCaptureDispatch) -> io::Result<()> {
        *self
            .browser_capture_dispatch
            .lock()
            .map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    fn capture_source(
        state: &State,
        owner: &str,
        workspace: &str,
        panel: &str,
        generation: &str,
        navigation: &str,
    ) -> Result<(Arc<BrowserControl>, String, BrowserArtifactSource), ErrorCode> {
        let target = Self::browser_target(
            state,
            owner,
            workspace,
            panel,
            generation,
            "browser.capture_composite",
        )?;
        target.control.check_document(navigation)?;
        target.control.require_renderable()?;
        let url = target.control.document_url()?;
        let url =
            lomi_control_protocol::browser::address(&url).map_err(|_| ErrorCode::ScopeDenied)?;
        if !target.control.permits(url.as_str()) {
            return Err(ErrorCode::ScopeDenied);
        }
        let session = state.sessions.get(owner).ok_or(ErrorCode::ControlRevoked)?;
        if session.grant.browser_profile.as_deref() != Some(target.control.profile_id.as_str()) {
            return Err(ErrorCode::ScopeDenied);
        }
        Ok((
            target.control.clone(),
            session
                .grant
                .workspace(workspace)
                .ok_or(ErrorCode::TargetNotFound)?
                .project_id
                .clone(),
            BrowserArtifactSource {
                workspace_id: workspace.into(),
                panel_id: panel.into(),
                browser_generation: generation.into(),
                profile_id: target.control.profile_id.clone(),
                navigation_id: navigation.into(),
                origin: url.origin().ascii_serialization(),
                required_scope: "browser.capture_composite".into(),
            },
        ))
    }
    pub(super) fn screenshot_browser(&self, owner: &str, input: BrowserScreenshotInput) -> Reply {
        if !(64..=1600).contains(&input.max_width)
            || !(16384..=MAX_IMAGE_BYTES).contains(&(input.max_bytes as usize))
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let (control, project, source) = {
            let Ok(state) = self.lock_state() else {
                return error(ErrorCode::ControlRevoked);
            };
            match Self::capture_source(
                &state,
                owner,
                &input.workspace_id,
                &input.panel_id,
                &input.browser_generation,
                &input.navigation_id,
            ) {
                Ok(s) => s,
                Err(e) => return error(e),
            }
        };
        let _guard = match control.begin_dom() {
            Ok(g) => g,
            Err(e) => return error(e),
        };
        let Some(dispatch) = self
            .browser_capture_dispatch
            .lock()
            .ok()
            .and_then(|d| d.clone())
        else {
            return error(ErrorCode::UnsupportedCapability);
        };
        let reservation = {
            let Ok(mut store) = self.store.lock() else {
                return error(ErrorCode::StorageUnavailable);
            };
            match store.reserve_artifact(
                owner,
                &project,
                &ArtifactSource::Browser(source.clone()),
                input.max_bytes as usize,
                now(),
            ) {
                Ok(r) => r,
                Err(e) => return operations::storage_error(e),
            }
        };
        let capture = dispatch(
            control.clone(),
            input.clone(),
            Instant::now() + Duration::from_secs(5),
        );
        let result = (|| -> Result<Artifact, ErrorCode> {
            let capture = capture?;
            if capture.bytes.len() > input.max_bytes as usize
                || !control.permits(&capture.url)
                || lomi_control_protocol::browser::address(&capture.url)
                    .map_err(|_| ErrorCode::ScopeDenied)?
                    .origin()
                    .ascii_serialization()
                    != source.origin
            {
                return Err(ErrorCode::StaleSnapshot);
            }
            let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let (current, _, current_source) = Self::capture_source(
                &state,
                owner,
                &input.workspace_id,
                &input.panel_id,
                &input.browser_generation,
                &input.navigation_id,
            )?;
            if !Arc::ptr_eq(&control, &current) || source != current_source {
                return Err(ErrorCode::StaleGeneration);
            }
            self.store
                .lock()
                .map_err(|_| ErrorCode::StorageUnavailable)?
                .commit_artifact(&reservation, &capture.bytes, capture.geometry, now())
                .map_err(|_| ErrorCode::StorageUnavailable)
        })();
        match result {
            Ok(artifact) => self.disclose_artifact(owner, &input.workspace_id, artifact),
            Err(code) => {
                if let Ok(mut store) = self.store.lock() {
                    let _ = store.abandon_artifact(&reservation);
                }
                error(code)
            }
        }
    }
    pub(super) fn read_artifact(&self, owner: &str, input: ArtifactReadInput) -> Reply {
        let artifact = {
            let Ok(state) = self.lock_state() else {
                return error(ErrorCode::ControlRevoked);
            };
            let Some(session) = state
                .sessions
                .get(owner)
                .filter(|s| s.alive.load(Ordering::SeqCst))
            else {
                return error(ErrorCode::ControlRevoked);
            };
            let Some(root) = session.grant.workspace(&input.workspace_id) else {
                return error(ErrorCode::TargetNotFound);
            };
            let Ok(store) = self.store.lock() else {
                return error(ErrorCode::StorageUnavailable);
            };
            match store.artifact_metadata(owner, &root.project_id, &input.artifact_id, now()) {
                Ok(a) => a,
                Err(_) => return error(ErrorCode::TargetNotFound),
            }
        };
        self.disclose_artifact(owner, &input.workspace_id, artifact)
    }
    pub(super) fn disclose_artifact(
        &self,
        owner: &str,
        workspace: &str,
        artifact: Artifact,
    ) -> Reply {
        let Ok(state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        if artifact.source.workspace() != workspace {
            return error(ErrorCode::TargetNotFound);
        }
        if matches!(&artifact.source, ArtifactSource::Project(_)) {
            return self.disclose_project_artifact(&state, owner, workspace, artifact);
        }
        let ArtifactSource::Browser(source) = &artifact.source else {
            return self.disclose_android_artifact(&state, owner, workspace, artifact);
        };
        let target = match Self::browser_target(
            &state,
            owner,
            workspace,
            &source.panel_id,
            &source.browser_generation,
            "browser.capture_composite",
        ) {
            Ok(t) => t,
            Err(e) => return error(e),
        };
        let Some(session) = state.sessions.get(owner) else {
            return error(ErrorCode::ControlRevoked);
        };
        if !target.control.authorized()
            || session.grant.browser_profile.as_deref() != Some(source.profile_id.as_str())
            || !session
                .grant
                .browser_origins
                .iter()
                .any(|o| o.as_str() == source.origin)
        {
            return error(ErrorCode::ControlRevoked);
        }
        let Ok(store) = self.store.lock() else {
            return error(ErrorCode::StorageUnavailable);
        };
        let bytes = match store.artifact_bytes(&artifact) {
            Ok(b) => b,
            Err(_) => return error(ErrorCode::StorageUnavailable),
        };
        let image = base64::engine::general_purpose::STANDARD.encode(bytes);
        if !target.control.authorized() {
            return error(ErrorCode::ControlRevoked);
        }
        Reply::ok(Data::Artifact {
            artifact: Box::new(artifact),
            image: Some(image),
        })
    }
}
