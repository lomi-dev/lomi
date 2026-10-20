//! Bounded in-memory output timing, compiled only with native-smoke.
use serde::Serialize;
use std::{
    collections::VecDeque,
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex, OnceLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};
const LIMIT: usize = 16_384;
struct Trace {
    enabled: bool,
    records: Mutex<VecDeque<ReadTrace>>,
    dropped: AtomicUsize,
}
static TRACE: OnceLock<Trace> = OnceLock::new();

pub fn initialize(app: &tauri::AppHandle) {
    TRACE.get_or_init(|| {
        let expected = std::env::var("LOMI_TERMINAL_REGRESSION_IDENTIFIER").unwrap_or_default();
        let enabled = expected.starts_with("dev.lomi.terminal-regression-")
            && app.config().identifier == expected
            && std::env::var("LOMI_TERMINAL_REGRESSION_DIRECTORY")
                .ok()
                .is_some_and(|root| Path::new(&root).is_absolute());
        Trace {
            enabled,
            records: Mutex::new(VecDeque::new()),
            dropped: AtomicUsize::new(0),
        }
    });
}
fn unix_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadTrace {
    session: String,
    sequence: u64,
    bytes: usize,
    prefix: String,
    read_end_unix_us: u64,
    remote_begin_unix_us: Option<u64>,
    remote_end_unix_us: Option<u64>,
    local_begin_unix_us: Option<u64>,
    local_end_unix_us: Option<u64>,
    channel_succeeded: bool,
}
impl ReadTrace {
    pub fn begin(session: &str, sequence: u64, data: &[u8]) -> Option<Self> {
        if !TRACE.get().is_some_and(|trace| trace.enabled) {
            return None;
        }
        let read_end_unix_us = unix_us();
        Some(Self {
            session: session.into(),
            sequence,
            bytes: data.len(),
            prefix: String::from_utf8_lossy(&data[..data.len().min(512)]).into_owned(),
            read_end_unix_us,
            remote_begin_unix_us: None,
            remote_end_unix_us: None,
            local_begin_unix_us: None,
            local_end_unix_us: None,
            channel_succeeded: false,
        })
    }
    pub fn remote_begin(&mut self) {
        self.remote_begin_unix_us = Some(unix_us());
    }
    pub fn remote_end(&mut self) {
        self.remote_end_unix_us = Some(unix_us());
    }
    pub fn local_begin(&mut self) {
        self.local_begin_unix_us = Some(unix_us());
    }
    pub fn finish(mut self, succeeded: bool) {
        self.local_end_unix_us = Some(unix_us());
        self.channel_succeeded = succeeded;
        let Some(trace) = TRACE.get().filter(|trace| trace.enabled) else {
            return;
        };
        // Trace export must never hold up the native reader. A contending export
        // drops diagnostic records rather than changing production scheduling.
        let Ok(mut records) = trace.records.try_lock() else {
            trace.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if records.len() == LIMIT {
            records.pop_front();
            trace.dropped.fetch_add(1, Ordering::Relaxed);
        }
        records.push_back(self);
    }
}
pub fn snapshot() -> serde_json::Value {
    let Some(trace) = TRACE.get().filter(|trace| trace.enabled) else {
        return serde_json::json!({"enabled":false});
    };
    match trace.records.lock() {
        Ok(records) => {
            serde_json::json!({"enabled":true,"limit":LIMIT,"dropped":trace.dropped.load(Ordering::Relaxed),"records":&*records})
        }
        Err(_) => serde_json::json!({"enabled":true,"error":"Output timing trace lock poisoned"}),
    }
}
pub fn write(root: &Path, phase: &str) -> Result<(), String> {
    if !matches!(phase, "exercise" | "restore") {
        return Err("Invalid output trace phase".into());
    }
    std::fs::write(
        root.join(format!("{phase}-native-output.json")),
        serde_json::to_vec(&snapshot()).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}
