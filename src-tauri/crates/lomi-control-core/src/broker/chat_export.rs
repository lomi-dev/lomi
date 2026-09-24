use super::*;

pub type ChatExportDispatch = Arc<
    dyn Fn(
            &str,
            &ChatExportInput,
            &dyn Fn() -> Result<(), ErrorCode>,
        ) -> Result<ChatExport, ErrorCode>
        + Send
        + Sync,
>;

impl Broker {
    pub fn set_chat_export_dispatch(&self, dispatch: ChatExportDispatch) -> io::Result<()> {
        *self.chat_export_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    pub(super) fn export_chat(&self, owner: &str, input: ChatExportInput) -> Reply {
        if !valid_chat_id(&input.conversation_id)
            || !(2..=8192).contains(&input.max_chars)
            || input.start_utf16 > 4 * 1024 * 1024
            || (input.start_utf16 > 0 && input.expected_revision.is_none())
            || input
                .expected_revision
                .as_ref()
                .is_some_and(|r| r.len() != 64 || !r.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let _producer = match self.file_reads.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => return error(ErrorCode::ResourceExhausted),
        };
        let access = |state: &State| {
            let project = Self::chat_access(state, owner, &input.workspace_id)?;
            let grant = &state.sessions[owner].grant;
            if !grant.scopes.contains("chat.export")
                || !grant.chat_conversations.contains(&input.conversation_id)
            {
                return Err(ErrorCode::ScopeDenied);
            }
            Ok(project)
        };
        let (project, policy) = {
            let state = match self.lock_state() {
                Ok(s) => s,
                Err(_) => return error(ErrorCode::ControlRevoked),
            };
            let project = match access(&state) {
                Ok(p) => p,
                Err(code) => return error(code),
            };
            (project, state.policy_revision)
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        let check = || {
            if Instant::now() >= deadline {
                return Err(ErrorCode::DeadlineExceeded);
            }
            let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            if state.policy_revision != policy || access(&state)? != project {
                return Err(ErrorCode::ControlRevoked);
            }
            Ok(())
        };
        let export = || {
            check()?;
            let dispatch = self
                .chat_export_dispatch
                .lock()
                .ok()
                .and_then(|d| d.clone())
                .ok_or(ErrorCode::UiNotReady)?;
            let value = dispatch(&project, &input, &check)?;
            check()?;
            let count = value.content.encode_utf16().count() as u32;
            let end = input.start_utf16.saturating_add(count);
            if value.workspace_id != input.workspace_id
                || value.conversation_id != input.conversation_id
                || value.format != input.format
                || value.revision.len() != 64
                || !value.revision.bytes().all(|b| b.is_ascii_hexdigit())
                || value.total_bytes > 4 * 1024 * 1024
                || value.total_utf16 > value.total_bytes
                || value.message_count > 512
                || count > u32::from(input.max_chars)
                || end > value.total_utf16
                || value.start_utf16 != input.start_utf16
                || value.next_utf16 != (end < value.total_utf16).then_some(end)
                || (end < value.total_utf16 && count == 0)
            {
                return Err(ErrorCode::OutcomeUnknown);
            }
            if input
                .expected_revision
                .as_ref()
                .is_some_and(|r| r != &value.revision)
            {
                return Err(ErrorCode::RevisionConflict);
            }
            if serde_json::to_vec(&value).map_or(true, |b| b.len() > 48 * 1024) {
                return Err(ErrorCode::ResourceExhausted);
            }
            Ok(value)
        };
        match export() {
            Ok(value) => Reply::ok(Data::ChatExport(Box::new(value))),
            Err(code) => error(code),
        }
    }
}
