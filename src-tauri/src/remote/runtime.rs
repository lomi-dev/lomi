use super::{json, uuid, Value};
use crate::terminal::RemoteTerminalEvent;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::Manager;

const MAX_LINE: usize = 12 * 1024 * 1024;

pub(super) struct Session {
    pub id: String,
    pub epoch: String,
    pub label: String,
    pub cols: u16,
    pub rows: u16,
    pub available: bool,
    pub unavailable_reason: Option<&'static str>,
    seq: u64,
    initial_cols: u16,
    initial_rows: u16,
    checkpoint: Option<Value>,
    journal: Vec<Value>,
    journal_bytes: usize,
    checkpoint_at: Instant,
}
#[derive(Default)]
pub(super) struct Runtime {
    pub sessions: HashMap<String, Session>,
    helper: Option<Helper>,
    pending: Option<RemoteTerminalEvent>,
    recovery_required: bool,
}

impl Runtime {
    #[cfg(feature = "remote-probe")]
    pub(super) fn probe_kill_helper(&mut self) -> Result<(), String> {
        let helper = self.helper.as_ref().ok_or("Terminal helper unavailable.")?;
        let mut child = helper.child.lock().map_err(|_| "Helper unavailable.")?;
        child.kill().map_err(|_| "Helper fault injection failed.")?;
        child.wait().map_err(|_| "Helper fault injection failed.")?;
        Ok(())
    }
    pub fn observe(
        &mut self,
        app: &tauri::AppHandle,
        event: RemoteTerminalEvent,
    ) -> Result<Option<Value>, String> {
        if self.pending.is_some() {
            return Err("Remote terminal recovery is still pending.".into());
        }
        match self.apply(event) {
            Ok(value) => Ok(value),
            Err(_) => self.recover(app),
        }
    }

    // The observer must retry this before dequeuing another PTY event. The pending
    // operation is already journaled and is replayed into a fresh model exactly once.
    pub fn needs_recovery(&self) -> bool {
        self.recovery_required
    }

    pub fn recover(&mut self, app: &tauri::AppHandle) -> Result<Option<Value>, String> {
        self.recovery_required = true;
        self.helper.take();
        let helper = Helper::start(app)?;
        self.restore_helper(helper)
    }

    fn restore_helper(&mut self, mut helper: Helper) -> Result<Option<Value>, String> {
        for s in self.sessions.values_mut().filter(|s| s.available) {
            let restored = if let Some(checkpoint) = &s.checkpoint {
                helper.request(json!({"op":"restore","sessionId":s.id,"epoch":s.epoch,
                    "seq":checkpoint["throughSeq"],"cols":checkpoint["cols"],"rows":checkpoint["rows"],
                    "state":checkpoint["state"],"suffix":checkpoint["suffix"]}))
            } else {
                helper.request(json!({"op":"create","sessionId":s.id,"epoch":s.epoch,
                    "seq":0,"cols":s.initial_cols,"rows":s.initial_rows}))
            };
            if let Err(error) = restored {
                if unavailable_message(&error).is_some() {
                    s.make_unavailable(&error);
                    continue;
                }
                return Err(error);
            }
            for operation in &s.journal {
                if let Err(error) = helper.request(operation.clone()) {
                    if unavailable_message(&error).is_some() {
                        s.available = false;
                        s.unavailable_reason = unavailable_message(&error);
                        // A model failure can advance its watermark; dropping is best
                        // effort. Healthy sessions remain usable on this helper.
                        let seq = operation["seq"].as_u64().unwrap();
                        for watermark in [seq, seq.saturating_sub(1)] {
                            let _ = helper.request(json!({"op":"drop","sessionId":s.id,"epoch":s.epoch,"seq":watermark}));
                        }
                        break;
                    }
                    return Err(error);
                }
            }
            if !s.available {
                s.checkpoint = None;
                s.journal.clear();
                s.journal_bytes = 0;
            }
        }
        self.helper = Some(helper);
        if let Some(RemoteTerminalEvent::Exit { id, .. }) = &self.pending {
            if let Some(s) = self.sessions.get(id).filter(|s| s.available) {
                if let Err(error) = self
                    .helper
                    .as_mut()
                    .unwrap()
                    .request(json!({"op":"drop","sessionId":id,"epoch":s.epoch,"seq":s.seq}))
                {
                    self.helper.take();
                    self.recovery_required = true;
                    return Err(error);
                }
            }
        }
        self.recovery_required = false;
        let result = self.finish_pending();
        // A new empty session also needs a baseline before its first unsafe write.
        self.refresh_checkpoints();
        Ok(result)
    }

    fn apply(&mut self, event: RemoteTerminalEvent) -> Result<Option<Value>, String> {
        if self.pending.is_some() {
            return Err("Remote terminal recovery is still pending.".into());
        }
        let request = match &event {
            RemoteTerminalEvent::Start { id, cols, rows } => {
                super::uuid_bytes(id)?;
                if self.sessions.get(id).is_some_and(|s| s.available) {
                    return Err("Remote terminal already exists.".into());
                }
                let epoch = uuid()?;
                self.sessions.insert(
                    id.clone(),
                    Session::new(
                        id.clone(),
                        epoch.clone(),
                        *cols,
                        *rows,
                        self.sessions.len() + 1,
                    ),
                );
                Some(
                    json!({"op":"create","sessionId":id,"epoch":epoch,"seq":0,"cols":cols,"rows":rows}),
                )
            }
            RemoteTerminalEvent::Output { id, data } => {
                let s = self
                    .sessions
                    .get_mut(id)
                    .ok_or("Remote terminal sequence unavailable.")?;
                if !s.available {
                    return Ok(None);
                }
                if data.len() > 16 * 1024 {
                    s.make_unavailable("JOURNAL_LIMIT");
                    let drop = json!({"op":"drop","sessionId":id,"epoch":s.epoch,"seq":s.seq});
                    self.discard_model(drop);
                    return Ok(Some(json!({"v":1,"type":"unavailable","sessionId":id})));
                }
                let seq = s.next_seq()?;
                let req = json!({"op":"write","sessionId":id,"epoch":s.epoch,"seq":seq,"data":STANDARD.encode(data)});
                if !s.record(&req) {
                    let drop = json!({"op":"drop","sessionId":id,"epoch":s.epoch,"seq":s.seq});
                    self.discard_model(drop);
                    return Ok(Some(json!({"v":1,"type":"unavailable","sessionId":id})));
                }
                Some(req)
            }
            RemoteTerminalEvent::Resize { id, cols, rows } => {
                let s = self
                    .sessions
                    .get_mut(id)
                    .ok_or("Remote terminal sequence unavailable.")?;
                if !s.available {
                    s.cols = *cols;
                    s.rows = *rows;
                    return Ok(None);
                }
                let seq = s.next_seq()?;
                let req = json!({"op":"resize","sessionId":id,"epoch":s.epoch,"seq":seq,"cols":cols,"rows":rows});
                if !s.record(&req) {
                    let drop = json!({"op":"drop","sessionId":id,"epoch":s.epoch,"seq":s.seq});
                    self.discard_model(drop);
                    return Ok(Some(json!({"v":1,"type":"unavailable","sessionId":id})));
                }
                Some(req)
            }
            RemoteTerminalEvent::Exit { id, .. } => self
                .sessions
                .get(id)
                .filter(|s| s.available)
                .map(|s| json!({"op":"drop","sessionId":id,"epoch":s.epoch,"seq":s.seq})),
        };
        self.pending = Some(event);
        if let Some(request) = request {
            let result = self
                .helper
                .as_mut()
                .ok_or("Remote terminal helper unavailable.")?
                .request(request.clone());
            if let Err(error) = result {
                if unavailable_message(&error).is_some() {
                    if let Some(s) = self
                        .sessions
                        .get_mut(request["sessionId"].as_str().unwrap())
                    {
                        s.make_unavailable(&error);
                    }
                    // Drop with either watermark: invalid dimensions is rejected
                    // before mutation, model limits can occur after mutation.
                    for seq in [
                        request["seq"].as_u64().unwrap(),
                        request["seq"].as_u64().unwrap().saturating_sub(1),
                    ] {
                        let _ = self.helper.as_mut().unwrap().request(json!({"op":"drop","sessionId":request["sessionId"],"epoch":request["epoch"],"seq":seq}));
                    }
                } else {
                    self.helper.take();
                    self.recovery_required = true;
                    return Err(error);
                }
            }
        }
        let result = self.finish_pending();
        self.refresh_checkpoints();
        Ok(result)
    }

    pub fn invalidate(&mut self, id: &str) -> Option<Value> {
        if self
            .pending
            .as_ref()
            .is_some_and(|event| event.session_id() == id)
        {
            self.pending = None;
        }
        let session = self.sessions.get_mut(id)?;
        if !session.available {
            return None;
        }
        let drop = json!({"op":"drop","sessionId":id,"epoch":session.epoch,"seq":session.seq});
        session.make_unavailable("OUTPUT_LOST");
        self.discard_model(drop);
        Some(json!({"v":1,"type":"unavailable","sessionId":id}))
    }

    // A loss marker can arrive before a queued Start. Register that terminal
    // honestly without dispatching any of its incomplete output to the helper.
    pub fn discard_lost_event(
        &mut self,
        event: RemoteTerminalEvent,
    ) -> Result<Option<Value>, String> {
        match event {
            RemoteTerminalEvent::Start { id, cols, rows } => {
                super::uuid_bytes(&id)?;
                self.invalidate(&id);
                let mut session =
                    Session::new(id.clone(), uuid()?, cols, rows, self.sessions.len() + 1);
                session.make_unavailable("OUTPUT_LOST");
                self.sessions.insert(id.clone(), session);
                Ok(Some(json!({"v":1,"type":"unavailable","sessionId":id})))
            }
            RemoteTerminalEvent::Resize { id, cols, rows } => {
                let delta = self.invalidate(&id);
                if let Some(session) = self.sessions.get_mut(&id) {
                    session.cols = cols;
                    session.rows = rows;
                }
                Ok(delta)
            }
            RemoteTerminalEvent::Output { id, .. } => Ok(self.invalidate(&id)),
            RemoteTerminalEvent::Exit { id, code } => {
                self.invalidate(&id);
                Ok(self.sessions.remove(&id).map(|session|
                    json!({"v":1,"type":"exit","sessionId":id,"seq":session.seq+1,"code":code})))
            }
        }
    }

    fn discard_model(&mut self, request: Value) {
        if self
            .helper
            .as_mut()
            .is_some_and(|h| h.request(request).is_err())
        {
            self.helper.take();
            self.recovery_required = true;
        }
    }

    fn finish_pending(&mut self) -> Option<Value> {
        match self.pending.take()? {
            RemoteTerminalEvent::Start { .. } => None,
            RemoteTerminalEvent::Output { id, data } => {
                let s = self.sessions.get_mut(&id)?;
                if !s.available {
                    return Some(json!({"v":1,"type":"unavailable","sessionId":id}));
                }
                s.seq += 1;
                Some(
                    json!({"v":1,"type":"output","sessionId":id,"seq":s.seq,"data":STANDARD.encode(data)}),
                )
            }
            RemoteTerminalEvent::Resize { id, cols, rows } => {
                let s = self.sessions.get_mut(&id)?;
                if !s.available {
                    return Some(json!({"v":1,"type":"unavailable","sessionId":id}));
                }
                s.seq += 1;
                s.cols = cols;
                s.rows = rows;
                Some(
                    json!({"v":1,"type":"resize","sessionId":id,"seq":s.seq,"cols":cols,"rows":rows}),
                )
            }
            RemoteTerminalEvent::Exit { id, code } => self
                .sessions
                .remove(&id)
                .map(|s| json!({"v":1,"type":"exit","sessionId":id,"seq":s.seq+1,"code":code})),
        }
    }

    fn refresh_checkpoints(&mut self) {
        let ids: Vec<_> = self
            .sessions
            .values()
            .filter(|s| {
                s.available
                    && (s.checkpoint.is_none()
                        || s.journal_bytes >= 64 * 1024
                        || s.checkpoint_at.elapsed() >= Duration::from_secs(1))
            })
            .map(|s| s.id.clone())
            .collect();
        for id in ids {
            // SNAPSHOT_PENDING preserves the previous exact checkpoint plus the
            // ordered journal, including resizes at incomplete parser boundaries.
            if self.snapshot(&id).is_err() && self.helper.is_none() {
                break;
            }
        }
    }

    pub fn snapshot(&mut self, id: &str) -> Result<Value, String> {
        if self.pending.is_some() {
            return Err("Remote terminal recovery is still pending.".into());
        }
        let s = self
            .sessions
            .get(id)
            .ok_or("Terminal is no longer running.")?;
        if !s.available {
            return Err("Remote model is unavailable for this terminal.".into());
        }
        let result = self
            .helper
            .as_mut()
            .ok_or("Remote terminal helper unavailable.")?
            .request(json!({"op":"snapshot","sessionId":id,"epoch":s.epoch,"seq":s.seq}));
        let reply = match result {
            Ok(reply) => reply,
            Err(error) if error == "SNAPSHOT_PENDING" => return Err(error),
            Err(error) if unavailable_message(&error).is_some() => {
                let request = json!({"op":"drop","sessionId":id,"epoch":s.epoch,"seq":s.seq});
                self.sessions.get_mut(id).unwrap().make_unavailable(&error);
                if self.helper.as_mut().unwrap().request(request).is_err() {
                    self.helper.take();
                    self.recovery_required = true;
                }
                return Err(error);
            }
            Err(error) => {
                self.helper.take();
                self.recovery_required = true;
                return Err(error);
            }
        };
        let s = self.sessions.get(id).unwrap();
        let (Some(suffix), Some(state)) = (
            reply.get("suffix").and_then(Value::as_str),
            reply.get("state"),
        ) else {
            self.helper.take();
            self.recovery_required = true;
            return Err("Invalid Remote snapshot.".into());
        };
        if reply.get("throughSeq").and_then(Value::as_u64) != Some(s.seq)
            || reply.get("cols").and_then(Value::as_u64) != Some(s.cols as u64)
            || reply.get("rows").and_then(Value::as_u64) != Some(s.rows as u64)
            || state.get("schema").and_then(Value::as_str) != Some("lomi-xterm-6-v1")
            || state.get("cols") != reply.get("cols")
            || state.get("rows") != reply.get("rows")
            || !serde_json::to_vec(state).is_ok_and(|bytes| bytes.len() <= 8 * 1024 * 1024)
            || !STANDARD
                .decode(suffix)
                .is_ok_and(|bytes| bytes.len() <= 512 * 1024 && STANDARD.encode(bytes) == suffix)
        {
            self.helper.take();
            self.recovery_required = true;
            return Err("Remote snapshot watermark or budget mismatch.".into());
        }
        let value = json!({"v":1,"type":"snapshot","seq":s.seq,"cols":s.cols,"rows":s.rows,"state":state,"suffix":suffix});
        let s = self.sessions.get_mut(id).unwrap();
        s.checkpoint = Some(reply);
        s.journal.clear();
        s.journal_bytes = 0;
        s.checkpoint_at = Instant::now();
        Ok(value)
    }
    pub fn stop(&mut self) {
        self.helper.take();
        self.sessions.clear();
        self.pending = None;
        self.recovery_required = false;
    }
}

impl Session {
    fn new(id: String, epoch: String, cols: u16, rows: u16, ordinal: usize) -> Self {
        Self {
            id,
            epoch,
            label: format!("Terminal {ordinal}"),
            cols,
            rows,
            initial_cols: cols,
            initial_rows: rows,
            seq: 0,
            available: true,
            unavailable_reason: None,
            checkpoint: None,
            journal: Vec::new(),
            journal_bytes: 0,
            checkpoint_at: Instant::now(),
        }
    }
    fn next_seq(&self) -> Result<u64, String> {
        self.seq
            .checked_add(1)
            .filter(|n| *n <= 9_007_199_254_740_991)
            .ok_or_else(|| "Remote sequence exhausted.".into())
    }
    fn record(&mut self, request: &Value) -> bool {
        let size = serde_json::to_vec(request)
            .map(|v| v.len())
            .unwrap_or(usize::MAX);
        if self.journal.len() >= 2048 || size > 768 * 1024 - self.journal_bytes {
            self.make_unavailable("JOURNAL_LIMIT");
            return false;
        }
        self.journal_bytes += size;
        self.journal.push(request.clone());
        true
    }
    fn make_unavailable(&mut self, error: &str) {
        self.available = false;
        self.unavailable_reason = unavailable_message(error);
        self.checkpoint = None;
        self.journal.clear();
        self.journal_bytes = 0;
    }
}

struct Helper {
    child: Arc<Mutex<Child>>,
    stdin: ChildStdin,
    responses: mpsc::Receiver<Value>,
    deadline: Arc<Mutex<Option<Instant>>>,
    next: u64,
}
impl Drop for Helper {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
impl Helper {
    fn start(app: &tauri::AppHandle) -> Result<Self, String> {
        let (node, bundle) = if cfg!(debug_assertions) {
            let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            (
                root.join(format!(
                    "binaries/lomi-node-{}{}",
                    env!("LOMI_AI_TARGET"),
                    if cfg!(windows) { ".exe" } else { "" }
                )),
                root.join("resources/remote-terminal/index.cjs"),
            )
        } else {
            (
                tauri::utils::platform::current_exe()
                    .map_err(|_| "Application path unavailable.")?
                    .parent()
                    .ok_or("Application path unavailable.")?
                    .join(if cfg!(windows) {
                        "lomi-node.exe"
                    } else {
                        "lomi-node"
                    }),
                app.path()
                    .resource_dir()
                    .map_err(|_| "Remote resources unavailable.")?
                    .join("remote-terminal/index.cjs"),
            )
        };
        Self::from_paths(node, bundle)
    }

    fn from_paths(node: std::path::PathBuf, bundle: std::path::PathBuf) -> Result<Self, String> {
        if !node.is_absolute() || !bundle.is_absolute() || !node.is_file() || !bundle.is_file() {
            return Err("Bundled Remote runtime is unavailable.".into());
        }
        use sha2::{Digest, Sha256};
        let bytes = std::fs::read(&bundle).map_err(|_| "Remote runtime unavailable.")?;
        let expected = std::fs::read_to_string(bundle.with_file_name("index.cjs.sha256"))
            .map_err(|_| "Remote runtime verification unavailable.")?;
        if super::hex(&Sha256::digest(&bytes)) != expected.trim() {
            return Err("Remote runtime integrity verification failed.".into());
        }
        let mut cmd = Command::new(node);
        cmd.arg("--no-warnings")
            .arg(bundle)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for name in ["SystemRoot", "WINDIR", "TEMP", "TMP", "TMPDIR", "LANG"] {
            if let Some(v) = std::env::var_os(name) {
                cmd.env(name, v);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        let mut child = cmd
            .spawn()
            .map_err(|_| "Cannot start trusted terminal helper.")?;
        let stdin = child
            .stdin
            .take()
            .ok_or("Terminal helper input unavailable.")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("Terminal helper output unavailable.")?;
        let child = Arc::new(Mutex::new(child));
        let (send, responses) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                match reader
                    .by_ref()
                    .take((MAX_LINE + 1) as u64)
                    .read_until(b'\n', &mut line)
                {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                };
                if line.len() > MAX_LINE || line.last() != Some(&b'\n') {
                    break;
                }
                let Ok(value) = serde_json::from_slice::<Value>(&line) else {
                    break;
                };
                if send.send(value).is_err() {
                    break;
                }
            }
        });
        let deadline = Arc::new(Mutex::new(None::<Instant>));
        let watch_deadline = deadline.clone();
        let weak = Arc::downgrade(&child);
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(100));
            let Some(child) = weak.upgrade() else {
                break;
            };
            if watch_deadline
                .lock()
                .map(|d| d.is_some_and(|d| Instant::now() > d))
                .unwrap_or(true)
            {
                if let Ok(mut child) = child.lock() {
                    let _ = child.kill();
                }
                break;
            }
        });
        Ok(Self {
            child,
            stdin,
            responses,
            deadline,
            next: 0,
        })
    }
    fn request(&mut self, mut request: Value) -> Result<Value, String> {
        self.next += 1;
        request["id"] = json!(self.next);
        let mut bytes =
            serde_json::to_vec(&request).map_err(|_| "Terminal helper request invalid.")?;
        if bytes.len() > MAX_LINE {
            return Err("Terminal helper request exceeds its budget.".into());
        }
        bytes.push(b'\n');
        let budget = Duration::from_secs(if self.next == 1 { 5 } else { 2 });
        *self
            .deadline
            .lock()
            .map_err(|_| "Terminal helper unavailable.")? = Some(Instant::now() + budget);
        self.stdin
            .write_all(&bytes)
            .and_then(|_| self.stdin.flush())
            .map_err(|_| "Terminal helper pipe closed.")?;
        let reply = self
            .responses
            .recv_timeout(budget)
            .map_err(|_| "Terminal helper deadline exceeded.")?;
        *self
            .deadline
            .lock()
            .map_err(|_| "Terminal helper unavailable.")? = None;
        if reply.get("id").and_then(Value::as_u64) != Some(self.next) {
            return Err("Terminal helper response mismatch.".into());
        }
        if reply.get("ok").and_then(Value::as_bool) != Some(true) {
            return Err(match reply.get("error").and_then(Value::as_str) {
                Some("SESSION_LIMIT") => "SESSION_LIMIT",
                Some("MODEL_LIMIT") => "MODEL_LIMIT",
                Some("INVALID_DIMENSIONS") => "INVALID_DIMENSIONS",
                Some("SNAPSHOT_PENDING") => "SNAPSHOT_PENDING",
                Some("SNAPSHOT_LIMIT") => "SNAPSHOT_LIMIT",
                _ => "Terminal helper rejected the request.",
            }
            .into());
        }
        let result = reply
            .get("result")
            .cloned()
            .ok_or("Terminal helper returned an invalid response.")?;
        if matches!(
            request["op"].as_str(),
            Some("create" | "restore" | "write" | "resize")
        ) && result.get("seq") != request.get("seq")
        {
            return Err("Terminal helper watermark mismatch.".into());
        }
        Ok(result)
    }
}

#[cfg(test)]
#[allow(
    clippy::items_after_test_module,
    reason = "Keep lifecycle fixtures beside the state transitions they exercise."
)]
mod tests {
    use super::*;
    fn runtime() -> Runtime {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        Runtime {
            sessions: HashMap::new(),
            pending: None,
            recovery_required: false,
            helper: Some(
                Helper::from_paths(
                    root.join(format!(
                        "binaries/lomi-node-{}{}",
                        env!("LOMI_AI_TARGET"),
                        if cfg!(windows) { ".exe" } else { "" }
                    )),
                    root.join("resources/remote-terminal/index.cjs"),
                )
                .unwrap(),
            ),
        }
    }
    fn fresh_helper() -> Helper {
        runtime().helper.take().unwrap()
    }

    #[test]
    fn remote_helper_output_loss_fences_only_affected_session_and_restart_gets_new_epoch() {
        let mut r = runtime();
        let id = uuid().unwrap();
        let survivor = uuid().unwrap();
        for session in [&id, &survivor] {
            r.apply(RemoteTerminalEvent::Start {
                id: session.clone(),
                cols: 20,
                rows: 5,
            })
            .unwrap();
            r.apply(RemoteTerminalEvent::Output {
                id: session.clone(),
                data: b"before".to_vec(),
            })
            .unwrap();
        }
        let epoch = r.sessions[&id].epoch.clone();
        let before = r.snapshot(&survivor).unwrap();
        r.helper.take();
        assert!(r
            .apply(RemoteTerminalEvent::Output {
                id: id.clone(),
                data: b"pending".to_vec()
            })
            .is_err());
        assert!(r.pending.is_some());
        assert_eq!(r.invalidate(&id).unwrap()["type"], "unavailable");
        assert!(r.pending.is_none());
        assert!(r.sessions[&id].checkpoint.is_none());
        assert!(r.sessions[&id].journal.is_empty());
        r.restore_helper(fresh_helper()).unwrap();
        assert!(!r.sessions[&id].available);
        assert!(r.sessions[&id]
            .unavailable_reason
            .unwrap()
            .contains("output queue"));
        assert_eq!(r.snapshot(&survivor).unwrap(), before);
        r.apply(RemoteTerminalEvent::Output {
            id: survivor.clone(),
            data: b"exact".to_vec(),
        })
        .unwrap();
        assert_eq!(r.snapshot(&survivor).unwrap()["seq"], 2);
        r.apply(RemoteTerminalEvent::Start {
            id: id.clone(),
            cols: 20,
            rows: 5,
        })
        .unwrap();
        assert_ne!(r.sessions[&id].epoch, epoch);
        assert!(r.sessions[&id].available);
        assert_eq!(r.snapshot(&id).unwrap()["seq"], 0);
    }

    #[test]
    fn remote_helper_registers_unavailable_lost_start_without_helper() {
        let mut r = Runtime::default();
        let id = uuid().unwrap();
        r.discard_lost_event(RemoteTerminalEvent::Start {
            id: id.clone(),
            cols: 120,
            rows: 40,
        })
        .unwrap();
        assert!(!r.sessions[&id].available);
        assert_eq!((r.sessions[&id].cols, r.sessions[&id].rows), (120, 40));
        assert!(r.helper.is_none());
        r.discard_lost_event(RemoteTerminalEvent::Exit {
            id: id.clone(),
            code: None,
        })
        .unwrap();
        assert!(!r.sessions.contains_key(&id));
    }

    #[test]
    fn remote_helper_retains_initial_start_and_detects_snapshot_fault() {
        let mut r = Runtime::default();
        let id = uuid().unwrap();
        assert!(!r.needs_recovery());
        assert!(r
            .apply(RemoteTerminalEvent::Start {
                id: id.clone(),
                cols: 20,
                rows: 5
            })
            .is_err());
        assert!(r.pending.is_some());
        let epoch = r.sessions[&id].epoch.clone();
        r.restore_helper(fresh_helper()).unwrap();
        assert!(r.pending.is_none());
        assert_eq!(r.sessions[&id].epoch, epoch);
        r.helper
            .as_mut()
            .unwrap()
            .child
            .lock()
            .unwrap()
            .kill()
            .unwrap();
        assert!(r.snapshot(&id).is_err());
        assert!(r.needs_recovery());
        r.restore_helper(fresh_helper()).unwrap();
        assert!(!r.needs_recovery());
        assert_eq!(r.snapshot(&id).unwrap()["seq"], 0);
    }

    #[test]
    fn remote_helper_recovers_ambiguous_write_once_and_preserves_survivor() {
        let mut r = runtime();
        let id = uuid().unwrap();
        let survivor = uuid().unwrap();
        for session in [&id, &survivor] {
            r.apply(RemoteTerminalEvent::Start {
                id: session.clone(),
                cols: 20,
                rows: 5,
            })
            .unwrap();
        }
        r.apply(RemoteTerminalEvent::Output {
            id: survivor.clone(),
            data: b"survivor".to_vec(),
        })
        .unwrap();
        let before = r.snapshot(&survivor).unwrap();
        let epoch = r.sessions[&id].epoch.clone();
        let request = json!({"op":"write","sessionId":id,"epoch":epoch,"seq":1,"data":STANDARD.encode(b"once")});
        assert!(r.sessions.get_mut(&id).unwrap().record(&request));
        r.pending = Some(RemoteTerminalEvent::Output {
            id: id.clone(),
            data: b"once".to_vec(),
        });
        // The real helper applied the bytes, but the owner has not committed the
        // acknowledgment when the process dies. Its old screen must be discarded.
        r.helper.as_mut().unwrap().request(request).unwrap();
        r.helper
            .as_mut()
            .unwrap()
            .child
            .lock()
            .unwrap()
            .kill()
            .unwrap();
        r.helper.take();
        let dead = fresh_helper();
        dead.child.lock().unwrap().kill().unwrap();
        assert!(r.restore_helper(dead).is_err());
        assert!(r.pending.is_some());
        assert_eq!(r.sessions[&id].seq, 0);
        let output = r.restore_helper(fresh_helper()).unwrap().unwrap();
        assert_eq!(output["seq"], 1);
        assert_eq!(r.sessions[&id].epoch, epoch);
        let once = r.snapshot(&id).unwrap();
        r.helper.take();
        assert!(r.restore_helper(fresh_helper()).unwrap().is_none());
        assert_eq!(r.snapshot(&id).unwrap(), once);
        assert_eq!(r.snapshot(&survivor).unwrap(), before);
        let cells = STANDARD
            .decode(
                once["state"]["normal"]["lines"][0]["cells"]
                    .as_str()
                    .unwrap(),
            )
            .unwrap();
        let text: String = cells
            .as_chunks::<12>()
            .0
            .iter()
            .take(8)
            .map(|cell| {
                char::from_u32(u32::from_le_bytes(cell[..4].try_into().unwrap()) & 0x1fffff)
                    .unwrap()
            })
            .collect();
        assert_eq!(text, "once\0\0\0\0");
    }

    #[test]
    fn remote_helper_recovers_incomplete_parser_and_resize_in_order() {
        for parts in [
            vec![vec![0xe7, 0x95], vec![0x8c]],
            vec![b"\x1b[3".to_vec(), b"1mRED".to_vec()],
        ] {
            let id = uuid().unwrap();
            let mut r = runtime();
            let mut expected = runtime();
            for runtime in [&mut r, &mut expected] {
                runtime
                    .apply(RemoteTerminalEvent::Start {
                        id: id.clone(),
                        cols: 20,
                        rows: 5,
                    })
                    .unwrap();
                runtime
                    .apply(RemoteTerminalEvent::Output {
                        id: id.clone(),
                        data: parts[0].clone(),
                    })
                    .unwrap();
                runtime
                    .apply(RemoteTerminalEvent::Resize {
                        id: id.clone(),
                        cols: 12,
                        rows: 6,
                    })
                    .unwrap();
            }
            assert_eq!(r.snapshot(&id).unwrap_err(), "SNAPSHOT_PENDING");
            let epoch = r.sessions[&id].epoch.clone();
            r.helper.take();
            r.restore_helper(fresh_helper()).unwrap();
            assert_eq!(r.sessions[&id].epoch, epoch);
            for runtime in [&mut r, &mut expected] {
                runtime
                    .apply(RemoteTerminalEvent::Output {
                        id: id.clone(),
                        data: parts[1].clone(),
                    })
                    .unwrap();
            }
            assert_eq!(r.snapshot(&id).unwrap(), expected.snapshot(&id).unwrap());
        }
    }

    #[test]
    fn remote_helper_journal_limit_isolates_incomplete_terminal() {
        let mut r = runtime();
        let id = uuid().unwrap();
        let survivor = uuid().unwrap();
        for session in [&id, &survivor] {
            r.apply(RemoteTerminalEvent::Start {
                id: session.clone(),
                cols: 20,
                rows: 5,
            })
            .unwrap();
        }
        r.apply(RemoteTerminalEvent::Output {
            id: id.clone(),
            data: b"\x1b]0;".to_vec(),
        })
        .unwrap();
        r.sessions.get_mut(&id).unwrap().journal_bytes = 768 * 1024;
        assert_eq!(
            r.apply(RemoteTerminalEvent::Output {
                id: id.clone(),
                data: b"x".to_vec()
            })
            .unwrap()
            .unwrap()["type"],
            "unavailable"
        );
        assert!(!r.sessions[&id].available);
        r.helper.take();
        r.restore_helper(fresh_helper()).unwrap();
        r.apply(RemoteTerminalEvent::Output {
            id: survivor.clone(),
            data: b"still alive".to_vec(),
        })
        .unwrap();
        assert_eq!(r.snapshot(&survivor).unwrap()["seq"], 1);
    }

    #[test]
    fn remote_helper_native_ipc_isolates_33rd_session_and_model_limit() {
        let mut r = runtime();
        let ids: Vec<_> = (0..33).map(|_| uuid().unwrap()).collect();
        for id in &ids {
            r.apply(RemoteTerminalEvent::Start {
                id: id.clone(),
                cols: 80,
                rows: 24,
            })
            .unwrap();
        }
        assert_eq!(r.sessions.len(), 33);
        assert!(!r.sessions[&ids[32]].available);
        assert!(r.sessions[&ids[32]]
            .unavailable_reason
            .unwrap()
            .contains("32 active desktop terminals"));
        let mut domain = super::super::workspace::Domain::default();
        let epoch = domain.begin().unwrap();
        let workspace_id = uuid().unwrap();
        domain
            .sync(
                &epoch,
                1,
                vec![super::super::workspace::WorkspaceProjection {
                    id: workspace_id.clone(),
                    name: "Refused workspace".into(),
                    tabs: None,
                    terminals: vec![super::super::workspace::TerminalProjection {
                        pane_id: "budget-pane".into(),
                        session_id: Some(ids[32].clone()),
                        title: "Terminal".into(),
                    }],
                }],
            )
            .unwrap();
        assert!(
            super::super::workspace::workspace_unavailable(&domain, &r, &workspace_id)
                .unwrap()
                .contains("32 active desktop terminals")
        );
        assert!(r.sessions[&ids[0]].available);
        r.apply(RemoteTerminalEvent::Output {
            id: ids[0].clone(),
            data: b"survivor".to_vec(),
        })
        .unwrap();
        assert_eq!(r.snapshot(&ids[0]).unwrap()["seq"], 1);
        r.apply(RemoteTerminalEvent::Output {
            id: ids[1].clone(),
            data: b"a".to_vec(),
        })
        .unwrap();
        let data = "\u{0301}".repeat(8000).into_bytes();
        let mut unavailable = false;
        for _ in 0..4 {
            if r.apply(RemoteTerminalEvent::Output {
                id: ids[1].clone(),
                data: data.clone(),
            })
            .unwrap()
            .is_some_and(|v| v["type"] == "unavailable")
            {
                unavailable = true;
                break;
            }
        }
        assert!(unavailable);
        assert!(!r.sessions[&ids[1]].available);
        r.apply(RemoteTerminalEvent::Output {
            id: ids[0].clone(),
            data: b" still alive".to_vec(),
        })
        .unwrap();
        assert_eq!(r.snapshot(&ids[0]).unwrap()["seq"], 2);
        r.apply(RemoteTerminalEvent::Exit {
            id: ids[32].clone(),
            code: Some(0),
        })
        .unwrap();
        r.apply(RemoteTerminalEvent::Exit {
            id: ids[1].clone(),
            code: Some(0),
        })
        .unwrap();
        assert!(r.snapshot(&ids[0]).is_ok());
    }
}

fn unavailable_message(error: &str) -> Option<&'static str> {
    match error {
        "SESSION_LIMIT"=>Some("Remote currently supports independent models for at most 32 active desktop terminals. This workspace cannot be shared completely."),
        "INVALID_DIMENSIONS"=>Some("A workspace terminal exceeds the Remote terminal dimensions limit."),
        "OUTPUT_LOST"=>Some("This terminal exceeded the bounded Remote output queue. Its exact Remote state is unavailable; restart that terminal before sharing."),
        "JOURNAL_LIMIT"=>Some("A workspace terminal exceeded the bounded Remote recovery journal while its parser was incomplete. Restart that terminal before sharing."),
        "MODEL_LIMIT"|"SNAPSHOT_LIMIT"=>Some("A workspace terminal exceeded the Remote model budget. Restart that terminal before sharing."),
        _=>None,
    }
}
