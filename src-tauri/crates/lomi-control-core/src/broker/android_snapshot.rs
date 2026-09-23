use super::*;
use crate::android::AndroidControl;

pub type AndroidSnapshotDispatch = Arc<
    dyn Fn(
            Arc<AndroidControl>,
            AndroidSnapshotInput,
            String,
            Instant,
        ) -> Result<AndroidSnapshot, ErrorCode>
        + Send
        + Sync,
>;

impl Broker {
    pub fn set_android_snapshot_dispatch(
        &self,
        dispatch: AndroidSnapshotDispatch,
    ) -> io::Result<()> {
        *self
            .android_snapshot_dispatch
            .lock()
            .map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    fn android_observation_access(
        state: &State,
        owner: &str,
        input: &AndroidSnapshotInput,
    ) -> Result<Arc<AndroidControl>, ErrorCode> {
        Self::android_runtime_access(
            state,
            owner,
            &input.workspace_id,
            &input.panel_id,
            &input.device_id,
            Some(&input.generation),
        )?;
        if !state.sessions[owner]
            .grant
            .scopes
            .contains("android.observe")
        {
            return Err(ErrorCode::ScopeDenied);
        }
        let control = state.android[&input.device_id].control.clone();
        control.check_generation(&input.generation)?;
        Ok(control)
    }
    pub(super) fn android_snapshot(&self, owner: &str, input: AndroidSnapshotInput) -> Reply {
        if !(1..=500).contains(&input.max_nodes) || !(1024..=49152).contains(&input.max_bytes) {
            return error(ErrorCode::ResourceExhausted);
        }
        let control = {
            let Ok(state) = self.lock_state() else {
                return error(ErrorCode::ControlRevoked);
            };
            match Self::android_observation_access(&state, owner, &input) {
                Ok(c) => c,
                Err(e) => return error(e),
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
            .android_snapshot_dispatch
            .lock()
            .ok()
            .and_then(|d| d.clone())
        else {
            return error(ErrorCode::UnsupportedCapability);
        };
        let Ok(snapshot_id) = new_id() else {
            return error(ErrorCode::ResourceExhausted);
        };
        let deadline = Instant::now() + Duration::from_secs(12);
        let snapshot = match dispatch(
            control.clone(),
            input.clone(),
            snapshot_id.clone(),
            deadline,
        ) {
            Ok(s) => s,
            Err(e) => return error(e),
        };
        let Ok(state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        let current = match Self::android_observation_access(&state, owner, &input) {
            Ok(c) => c,
            Err(e) => return error(e),
        };
        if !Arc::ptr_eq(&current, &control) {
            return error(ErrorCode::StaleGeneration);
        }
        if Instant::now() >= deadline {
            return error(ErrorCode::DeadlineExceeded);
        }
        if snapshot.workspace_id != input.workspace_id
            || snapshot.panel_id != input.panel_id
            || snapshot.device_id != input.device_id
            || snapshot.generation != input.generation
            || snapshot.snapshot_id != snapshot_id
            || snapshot.rotation > 3
            || snapshot.coordinate_space != "rotated_display"
            || snapshot.nodes.len() > usize::from(input.max_nodes)
            || serde_json::to_vec(&snapshot)
                .map_or(true, |bytes| bytes.len() > input.max_bytes as usize)
        {
            return error(ErrorCode::OutcomeUnknown);
        }
        Reply::ok(Data::AndroidSnapshot(Box::new(snapshot)))
    }
}
