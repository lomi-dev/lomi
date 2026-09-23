use super::*;
use crate::android::AndroidControl;

pub struct AndroidLogBatch {
    pub process_id: u32,
    pub lines: Vec<String>,
    pub truncated: bool,
}
pub type AndroidLogcatDispatch = Arc<
    dyn Fn(Arc<AndroidControl>, AndroidLogcatInput, Instant) -> Result<AndroidLogBatch, ErrorCode>
        + Send
        + Sync,
>;
pub(super) struct CachedLogs {
    pub owner: String,
    input: AndroidLogcatInput,
    control: Arc<AndroidControl>,
    batch: AndroidLogBatch,
    expires: Instant,
}
impl Broker {
    pub fn set_android_logcat_dispatch(&self, dispatch: AndroidLogcatDispatch) -> io::Result<()> {
        *self.android_logcat_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    pub(super) fn android_logcat(&self, owner: &str, input: AndroidLogcatInput) -> Reply {
        if !(1..=64).contains(&input.limit)
            || !lomi_control_protocol::android::valid_package(&input.package_name)
            || input.cursor.as_ref().is_some_and(|c| c.len() > 80)
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let access = |state: &State| {
            Self::android_app_access(
                state,
                owner,
                &input.workspace_id,
                &input.panel_id,
                &input.device_id,
                &input.generation,
                &input.package_name,
                "android.logs",
            )
        };
        let control = {
            let Ok(mut state) = self.lock_state() else {
                return error(ErrorCode::ControlRevoked);
            };
            state
                .android_logs
                .retain(|_, b| b.expires > Instant::now() && b.control.check().is_ok());
            let control = match access(&state) {
                Ok(c) => c,
                Err(e) => return error(e),
            };
            if let Some(cursor) = &input.cursor {
                let Some((id, offset)) = cursor.split_once(':') else {
                    return error(ErrorCode::CursorExpired);
                };
                let Some(batch) = state.android_logs.get(id).filter(|b| {
                    b.owner == owner
                        && Arc::ptr_eq(&b.control, &control)
                        && b.input.workspace_id == input.workspace_id
                        && b.input.panel_id == input.panel_id
                        && b.input.device_id == input.device_id
                        && b.input.generation == input.generation
                        && b.input.package_name == input.package_name
                        && b.input.min_priority == input.min_priority
                }) else {
                    return error(ErrorCode::CursorExpired);
                };
                let Some(offset) = offset
                    .parse::<usize>()
                    .ok()
                    .filter(|n| n.to_string() == offset && *n < batch.batch.lines.len())
                else {
                    return error(ErrorCode::CursorExpired);
                };
                return page(id, batch, offset, input.limit);
            }
            if state.android_logs.len() >= 8 {
                return error(ErrorCode::ResourceExhausted);
            }
            control
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
            .android_logcat_dispatch
            .lock()
            .ok()
            .and_then(|d| d.clone())
        else {
            return error(ErrorCode::UnsupportedCapability);
        };
        let deadline = Instant::now() + Duration::from_secs(8);
        let batch = match dispatch(control.clone(), input.clone(), deadline) {
            Ok(b) => b,
            Err(e) => return error(e),
        };
        if batch.process_id == 0
            || batch.lines.len() > 256
            || batch.lines.iter().any(|l| l.len() > 2048)
            || batch.lines.iter().map(String::len).sum::<usize>() > 65536
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        let current = match access(&state) {
            Ok(c) => c,
            Err(e) => return error(e),
        };
        if !Arc::ptr_eq(&current, &control) {
            return error(ErrorCode::StaleGeneration);
        }
        if Instant::now() >= deadline {
            return error(ErrorCode::DeadlineExceeded);
        }
        if state.android_logs.len() >= 8 {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(id) = new_id() else {
            return error(ErrorCode::ResourceExhausted);
        };
        let cached = CachedLogs {
            owner: owner.into(),
            input,
            control,
            batch,
            expires: Instant::now() + Duration::from_secs(60),
        };
        let reply = page(&id, &cached, 0, cached.input.limit);
        state.android_logs.insert(id, cached);
        reply
    }
}
fn page(id: &str, cached: &CachedLogs, offset: usize, limit: u16) -> Reply {
    let end = (offset + usize::from(limit)).min(cached.batch.lines.len());
    Reply::ok(Data::AndroidLogcat(Box::new(AndroidLogcat {
        workspace_id: cached.input.workspace_id.clone(),
        device_id: cached.input.device_id.clone(),
        generation: cached.input.generation.clone(),
        package_name: cached.input.package_name.clone(),
        process_id: cached.batch.process_id,
        lines: cached.batch.lines[offset..end].to_vec(),
        next_cursor: (end < cached.batch.lines.len()).then(|| format!("{id}:{end}")),
        truncated: cached.batch.truncated,
        complete: false,
        scope: "main_process_main_buffer_recent_snapshot".into(),
        gap: "unknown".into(),
    })))
}
