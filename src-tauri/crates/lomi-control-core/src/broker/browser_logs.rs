use super::*;
use crate::browser::BrowserControl;

pub type BrowserLogsDispatch = Arc<
    dyn Fn(
            Arc<BrowserControl>,
            BrowserLogsInput,
            String,
            u64,
            Instant,
        ) -> Result<BrowserLogs, ErrorCode>
        + Send
        + Sync,
>;

impl Broker {
    pub fn set_browser_logs_dispatch(&self, dispatch: BrowserLogsDispatch) -> io::Result<()> {
        *self.browser_logs_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    pub(super) fn browser_logs(&self, owner: &str, input: BrowserLogsInput) -> Reply {
        if !(1..=64).contains(&input.limit) || input.cursor.as_ref().is_some_and(|s| s.len() > 160)
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let target = || -> Result<Arc<BrowserControl>, ErrorCode> {
            let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            Ok(Self::browser_target(
                &state,
                owner,
                &input.workspace_id,
                &input.panel_id,
                &input.browser_generation,
                "browser.read",
            )?
            .control
            .clone())
        };
        let control = match target() {
            Ok(c) => c,
            Err(e) => return error(e),
        };
        let _guard = match control.begin_dom() {
            Ok(g) => g,
            Err(e) => return error(e),
        };
        let navigation = control.navigation_id();
        let prefix = input
            .log_kind
            .cursor_prefix(&input.browser_generation, &navigation);
        let after = match input.cursor.as_ref() {
            None => 0,
            Some(cursor) => match cursor
                .strip_prefix(&prefix)
                .and_then(|s| s.parse::<u64>().ok())
                .filter(|n| *n < 9_007_199_254_740_991)
            {
                Some(n) => n,
                None => return error(ErrorCode::CursorExpired),
            },
        };
        let Some(dispatch) = self
            .browser_logs_dispatch
            .lock()
            .ok()
            .and_then(|d| d.clone())
        else {
            return error(ErrorCode::UnsupportedCapability);
        };
        let result = match dispatch(
            control.clone(),
            input.clone(),
            navigation.clone(),
            after,
            Instant::now() + Duration::from_secs(3),
        ) {
            Ok(r) => r,
            Err(e) => return error(e),
        };
        let current = match target() {
            Ok(c) => c,
            Err(e) => return error(e),
        };
        if !Arc::ptr_eq(&current, &control) {
            return error(ErrorCode::StaleGeneration);
        }
        if let Err(e) = current.check_document(&navigation) {
            return error(e);
        }
        let url = match control
            .document_url()
            .ok()
            .and_then(|s| lomi_control_protocol::browser::address(&s).ok())
        {
            Some(url) => url,
            None => return error(ErrorCode::StaleSnapshot),
        };
        if result.workspace_id != input.workspace_id
            || result.panel_id != input.panel_id
            || result.browser_generation != input.browser_generation
            || result.navigation_id != navigation
            || result.frame_id != "main"
            || result.log_kind != input.log_kind
            || result.origin != url.origin().ascii_serialization()
            || !control.permits(url.as_str())
        {
            return error(ErrorCode::StaleSnapshot);
        }
        let mut through = after;
        let ordered = result.entries.iter().all(|entry| {
            let valid = entry.sequence > through
                && entry.sequence < 9_007_199_254_740_991
                && entry.kind == input.log_kind
                && if input.log_kind == BrowserLogKind::PromiseRejection {
                    entry.event_trusted.is_some()
                } else {
                    entry.event_trusted.is_none()
                }
                && match input.log_kind {
                    BrowserLogKind::Console => entry.level.as_deref().is_some_and(|level| {
                        matches!(level, "log" | "info" | "warn" | "error" | "debug")
                    }),
                    _ => entry.level.is_none(),
                };
            through = entry.sequence;
            valid
        });
        if !ordered
            || result.next_cursor != format!("{prefix}{through}")
            || result.entries.len() > input.limit as usize
            || result.entries.iter().any(|e| e.message.len() > 1024)
            || serde_json::to_vec(&result).map_or(true, |v| v.len() > 65536)
        {
            return error(ErrorCode::ResourceExhausted);
        }
        Reply::ok(Data::BrowserLogs(Box::new(result)))
    }
}
