use super::*;
use crate::project_files::ProjectDirectory;
use std::sync::mpsc::{sync_channel, SyncSender};

pub type EditorReadDispatch = Arc<dyn Fn(EditorReadRequest) -> io::Result<()> + Send + Sync>;
pub(super) struct PendingRead {
    epoch: String,
    answer: SyncSender<EditorReadReply>,
}
impl Broker {
    pub fn set_editor_read_dispatch(&self, dispatch: EditorReadDispatch) -> io::Result<()> {
        *self.editor_read_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    pub fn editor_read_reply(&self, reply: EditorReadReply) -> io::Result<()> {
        if serde_json::to_vec(&reply).map_or(true, |v| v.len() > MAX_METADATA_BYTES) {
            return Err(failure());
        }
        let mut pending = self.editor_reads.lock().map_err(|_| failure())?;
        let request = pending.get(&reply.request_id).ok_or_else(failure)?;
        if request.epoch != reply.ui_epoch {
            return Err(failure());
        }
        pending
            .remove(&reply.request_id)
            .unwrap()
            .answer
            .try_send(reply)
            .map_err(|_| failure())
    }
    pub(super) fn editor_read_access(
        state: &State,
        owner: &str,
        input: &EditorReadInput,
    ) -> Result<Arc<ProjectDirectory>, ErrorCode> {
        let directory = Self::project_file_access(state, owner, &input.workspace_id)?;
        if !state.sessions[owner].grant.scopes.contains("editor.read") {
            return Err(ErrorCode::ScopeDenied);
        }
        if !state.projection.panels.iter().any(|p| {
            p.id == input.panel_id && p.workspace_id == input.workspace_id && p.kind == "file"
        }) {
            return Err(ErrorCode::TargetNotFound);
        }
        Ok(directory)
    }
    pub(super) fn editor_read(&self, owner: &str, input: EditorReadInput) -> Reply {
        if !(2..=8192).contains(&input.max_chars)
            || input.start_utf16 > 16 * 1024 * 1024
            || (input.start_utf16 > 0
                && (input.document_id.is_none() || input.expected_buffer_revision.is_none()))
            || input.document_id.as_ref().is_some_and(|s| !valid_id(s))
            || input
                .expected_buffer_revision
                .as_ref()
                .is_some_and(|s| s.is_empty() || s.len() > 96)
        {
            return error(ErrorCode::ResourceExhausted);
        }
        if let Err(e) = crate::project_files::validate_relative(&input.relative_path) {
            return error(e);
        }
        let (directory, root, project_id, epoch, connected, policy) = {
            let Ok(state) = self.lock_state() else {
                return error(ErrorCode::ControlRevoked);
            };
            let directory = match Self::editor_read_access(&state, owner, &input) {
                Ok(d) => d,
                Err(e) => return error(e),
            };
            let session = &state.sessions[owner];
            let Some(root) = session.grant.workspace(&input.workspace_id) else {
                return error(ErrorCode::TargetNotFound);
            };
            (
                directory,
                root.project_path.clone(),
                root.project_id.clone(),
                state.projection.ui_epoch.clone(),
                session.alive.clone(),
                state.policy_revision,
            )
        };
        let _producer = match self.file_reads.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => return error(ErrorCode::ResourceExhausted),
        };
        if let Err(e) = directory.open_file(&input.relative_path, 16 * 1024 * 1024) {
            return error(e);
        }
        let Some(dispatch) = self
            .editor_read_dispatch
            .lock()
            .ok()
            .and_then(|d| d.clone())
        else {
            return error(ErrorCode::UiNotReady);
        };
        let Ok(id) = new_id() else {
            return error(ErrorCode::ResourceExhausted);
        };
        let (answer, receive) = sync_channel(1);
        {
            let Ok(mut pending) = self.editor_reads.lock() else {
                return error(ErrorCode::AppUnavailable);
            };
            if pending.len() >= 16 {
                return error(ErrorCode::ResourceExhausted);
            }
            pending.insert(
                id.clone(),
                PendingRead {
                    epoch: epoch.clone(),
                    answer,
                },
            );
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        let check = || {
            if !connected.load(Ordering::SeqCst)
                || self.authorization.load(Ordering::SeqCst) != policy
            {
                return Err(ErrorCode::ControlRevoked);
            }
            if Instant::now() >= deadline {
                return Err(ErrorCode::UiNotReady);
            }
            Ok(())
        };
        let read = || -> Result<EditorReadReply, ErrorCode> {
            check()?;
            dispatch(EditorReadRequest {
                request_id: id.clone(),
                ui_epoch: epoch.clone(),
                project_path: root.clone(),
                project_id: project_id.clone(),
                input: input.clone(),
            })
            .map_err(|_| ErrorCode::UiNotReady)?;
            loop {
                check()?;
                match receive.recv_timeout(Duration::from_millis(25)) {
                    Ok(reply) => return Ok(reply),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => return Err(ErrorCode::UiNotReady),
                }
            }
        };
        let response = read();
        if let Ok(mut pending) = self.editor_reads.lock() {
            pending.remove(&id);
        }
        let reply = match response {
            Ok(r) => r,
            Err(e) => return error(e),
        };
        if let Err(e) = check() {
            return error(e);
        }
        if let Some(code) = reply.error {
            return error(code);
        }
        let Some(text) = reply.text else {
            return error(ErrorCode::UiNotReady);
        };
        let expected_path = Path::new(&root).join(&input.relative_path);
        if reply.source_path.as_deref() != expected_path.to_str() {
            return error(ErrorCode::ScopeDenied);
        }
        let units = text.content.encode_utf16().count() as u32;
        let Some(end) = input.start_utf16.checked_add(units) else {
            return error(ErrorCode::ResourceExhausted);
        };
        if text.workspace_id != input.workspace_id
            || text.panel_id != input.panel_id
            || text.relative_path != input.relative_path
            || !valid_id(&text.document_id)
            || text.buffer_revision.len() > 96
            || text.buffer_revision.is_empty()
            || text.disk_revision.len() != 64
            || !text.disk_revision.bytes().all(|b| b.is_ascii_hexdigit())
            || text.start_utf16 != input.start_utf16
            || units > u32::from(input.max_chars)
            || text.content.len() > 32768
            || text.total_utf16 > 16 * 1024 * 1024
            || end > text.total_utf16
            || text.next_utf16 != (end < text.total_utf16).then_some(end)
            || text.truncated != text.next_utf16.is_some()
            || (units == 0 && end < text.total_utf16)
        {
            return error(ErrorCode::OutcomeUnknown);
        }
        if input
            .document_id
            .as_ref()
            .is_some_and(|d| d != &text.document_id)
        {
            return error(ErrorCode::StaleGeneration);
        }
        if input
            .expected_buffer_revision
            .as_ref()
            .is_some_and(|r| r != &text.buffer_revision)
        {
            return error(ErrorCode::RevisionConflict);
        }
        if let Err(e) = directory.open_file(&input.relative_path, 16 * 1024 * 1024) {
            return error(e);
        }
        let Ok(state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        let current = match Self::editor_read_access(&state, owner, &input) {
            Ok(d) => d,
            Err(e) => return error(e),
        };
        if state.projection.ui_epoch != epoch || !Arc::ptr_eq(&directory, &current) {
            return error(ErrorCode::ControlRevoked);
        }
        if let Err(e) = check() {
            return error(e);
        }
        Reply::ok(Data::EditorText(Box::new(text)))
    }
}
