use super::*;
use crate::{android::AndroidControl, artifacts::MAX_IMAGE_BYTES};
use base64::Engine;

pub struct AndroidCapture {
    pub bytes: Vec<u8>,
    pub geometry: AndroidImageGeometry,
}
pub type AndroidCaptureDispatch = Arc<
    dyn Fn(
            Arc<AndroidControl>,
            AndroidScreenshotInput,
            Instant,
        ) -> Result<AndroidCapture, ErrorCode>
        + Send
        + Sync,
>;

impl Broker {
    pub fn set_android_capture_dispatch(&self, dispatch: AndroidCaptureDispatch) -> io::Result<()> {
        *self
            .android_capture_dispatch
            .lock()
            .map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    pub(super) fn android_capture_access(
        state: &State,
        owner: &str,
        source: &AndroidArtifactSource,
    ) -> Result<Arc<AndroidControl>, ErrorCode> {
        Self::android_runtime_access(
            state,
            owner,
            &source.workspace_id,
            &source.panel_id,
            &source.device_id,
            Some(&source.generation),
        )?;
        if source.required_scope != "android.capture"
            || !state.sessions[owner]
                .grant
                .scopes
                .contains("android.capture")
        {
            return Err(ErrorCode::ScopeDenied);
        }
        Ok(state.android[&source.device_id].control.clone())
    }
    pub(super) fn android_screenshot(&self, owner: &str, input: AndroidScreenshotInput) -> Reply {
        if !(64..=1600).contains(&input.max_edge)
            || !(16384..=MAX_IMAGE_BYTES).contains(&(input.max_bytes as usize))
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let source = AndroidArtifactSource {
            workspace_id: input.workspace_id.clone(),
            panel_id: input.panel_id.clone(),
            device_id: input.device_id.clone(),
            generation: input.generation.clone(),
            required_scope: "android.capture".into(),
        };
        let (control, project) = {
            let Ok(state) = self.lock_state() else {
                return error(ErrorCode::ControlRevoked);
            };
            let control = match Self::android_capture_access(&state, owner, &source) {
                Ok(c) => c,
                Err(e) => return error(e),
            };
            {
                let Some(root) = state.sessions[owner].grant.workspace(&input.workspace_id) else {
                    return error(ErrorCode::TargetNotFound);
                };
                (control, root.project_id.clone())
            }
        };
        let _global = match self.android_observations.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => return error(ErrorCode::ResourceExhausted),
        };
        let _device = match control.begin_observation() {
            Ok(p) => p,
            Err(e) => return error(e),
        };
        let Some(dispatch) = self
            .android_capture_dispatch
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
                &ArtifactSource::Android(source.clone()),
                input.max_bytes as usize,
                now(),
            ) {
                Ok(r) => r,
                Err(e) => return operations::storage_error(e),
            }
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        let captured = dispatch(control.clone(), input.clone(), deadline);
        let result = (|| {
            let capture = captured?;
            if Instant::now() >= deadline {
                return Err(ErrorCode::DeadlineExceeded);
            }
            if capture.bytes.len() > input.max_bytes as usize
                || capture.geometry.pixel_width > u32::from(input.max_edge)
                || capture.geometry.pixel_height > u32::from(input.max_edge)
            {
                return Err(ErrorCode::ArtifactTooLarge);
            }
            let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let current = Self::android_capture_access(&state, owner, &source)?;
            if !Arc::ptr_eq(&current, &control) {
                return Err(ErrorCode::StaleGeneration);
            }
            self.store
                .lock()
                .map_err(|_| ErrorCode::StorageUnavailable)?
                .commit_artifact(
                    &reservation,
                    &capture.bytes,
                    ImageGeometry::Android(capture.geometry),
                    now(),
                )
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
    pub(super) fn disclose_android_artifact(
        &self,
        state: &State,
        owner: &str,
        workspace: &str,
        artifact: Artifact,
    ) -> Reply {
        let ArtifactSource::Android(source) = &artifact.source else {
            return error(ErrorCode::ScopeDenied);
        };
        if source.workspace_id != workspace {
            return error(ErrorCode::TargetNotFound);
        }
        let control = match Self::android_capture_access(state, owner, source) {
            Ok(c) => c,
            Err(e) => return error(e),
        };
        let Ok(store) = self.store.lock() else {
            return error(ErrorCode::StorageUnavailable);
        };
        let bytes = match store.artifact_bytes(&artifact) {
            Ok(b) => b,
            Err(_) => return error(ErrorCode::StorageUnavailable),
        };
        let image = base64::engine::general_purpose::STANDARD.encode(bytes);
        if let Err(code) = control.check_generation(&source.generation) {
            return error(code);
        }
        Reply::ok(Data::Artifact {
            artifact: Box::new(artifact),
            image: Some(image),
        })
    }
}
