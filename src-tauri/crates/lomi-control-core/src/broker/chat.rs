use super::*;

pub type ChatListDispatch = Arc<
    dyn Fn(
            &str,
            ChatConversationSelection,
            &dyn Fn() -> Result<(), ErrorCode>,
        ) -> Result<Vec<ChatSummary>, ErrorCode>
        + Send
        + Sync,
>;
pub type ChatReadDispatch = Arc<
    dyn Fn(&str, &ChatReadInput, &dyn Fn() -> Result<(), ErrorCode>) -> Result<ChatRead, ErrorCode>
        + Send
        + Sync,
>;
pub type ChatOpenDispatch = Arc<
    dyn Fn(
            &str,
            &ChatOpenCommand,
            &dyn Fn() -> Result<(), ErrorCode>,
        ) -> Result<ChatSummary, ErrorCode>
        + Send
        + Sync,
>;

fn revision(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn counter(value: &str) -> bool {
    !value.is_empty() && value.len() <= 19 && value.parse::<u64>().is_ok()
}
fn valid_summary(value: &ChatSummary) -> bool {
    valid_chat_id(&value.conversation_id)
        && value.title.len() <= 256
        && counter(&value.conversation_revision)
        && counter(&value.updated_at_millis)
        && value
            .latest_message_id
            .as_ref()
            .is_none_or(|id| valid_chat_id(id))
}

impl Broker {
    pub fn set_chat_open_dispatch(&self, dispatch: ChatOpenDispatch) -> io::Result<()> {
        *self.chat_open_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    pub(super) fn open_chat(self: &Arc<Self>, owner: &str, input: ChatOpenInput) -> Reply {
        use super::operations::{storage_error, UiMutation};
        if !valid_id(&input.request_key)
            || input.expected_revision.parse::<u64>().is_err()
            || matches!(&input.target,ChatOpenTarget::Existing {conversation_id} if !valid_chat_id(conversation_id))
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        let project = match Self::chat_access(&state, owner, &input.workspace_id) {
            Ok(p) => p,
            Err(e) => return error(e),
        };
        let session = &state.sessions[owner];
        if !["chat.open", "panel.create", "panel.focus"]
            .iter()
            .all(|s| session.grant.scopes.contains(*s))
            || match &input.target {
                ChatOpenTarget::Existing { conversation_id } => {
                    !session.grant.permits_chat_conversation(conversation_id)
                }
                ChatOpenTarget::New => !session.grant.scopes.contains("chat.create"),
            }
        {
            return error(ErrorCode::ScopeDenied);
        }
        if input.retry_epoch != session.retry_epoch {
            return error(ErrorCode::RetryWindowExpired);
        }
        let workspace = state
            .projection
            .workspaces
            .iter()
            .find(|w| w.id == input.workspace_id)
            .unwrap()
            .clone();
        let hash = match receipts::fingerprint(
            &(&input, &workspace.project_path),
            &receipts::Target {
                workspace_id: &input.workspace_id,
                resource_id: match &input.target {
                    ChatOpenTarget::Existing { conversation_id } => conversation_id,
                    ChatOpenTarget::New => "new-chat",
                },
                generation: &state.projection.ui_epoch,
                revision: &input.expected_revision,
            },
        ) {
            Ok(hash) => hash,
            Err(e) => return storage_error(e),
        };
        let key = receipts::Key {
            pairing_id: owner,
            project_id: &project,
            retry_epoch: &input.retry_epoch,
            request_key: &input.request_key,
            tool: "lomi_chat_open",
        };
        match self.replay(&state, owner, &key, hash) {
            Ok(Some(reply)) => return reply,
            Err(e) => return *e,
            Ok(None) => {}
        }
        let create = matches!(input.target, ChatOpenTarget::New);
        if create && !session.grant.yolo && session.grant.chat_conversations.len() >= 64 {
            return error(ErrorCode::ResourceExhausted);
        }
        let conversation_id = match input.target {
            ChatOpenTarget::Existing { conversation_id } => conversation_id,
            ChatOpenTarget::New => match new_id() {
                Ok(id) => id,
                Err(_) => return error(ErrorCode::ResourceExhausted),
            },
        };
        let panel_id = match state
            .projection
            .panels
            .iter()
            .find(|p| {
                p.workspace_id == input.workspace_id
                    && p.kind == "chat"
                    && p.chat_conversation_id.as_ref() == Some(&conversation_id)
            })
            .map(|p| p.id.clone())
            .map(Ok)
            .unwrap_or_else(new_id)
        {
            Ok(id) => id,
            Err(_) => return error(ErrorCode::ResourceExhausted),
        };
        let action = UiAction::OpenChat(ChatOpenCommand {
            workspace_id: input.workspace_id.clone(),
            project_name: workspace.project_name,
            workspace_name: workspace.name,
            conversation_id,
            panel_id,
            create,
            not_after_millis: (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                + 30_000)
                .to_string(),
        });
        if let Err(code) = Self::validate_panel_action(&state, owner, &action) {
            return error(code);
        }
        self.enqueue_ui(
            &mut state,
            owner,
            UiMutation {
                workspace: input.workspace_id.clone(),
                project,
                revision: input.expected_revision,
                retry_epoch: input.retry_epoch,
                request_key: input.request_key,
                tool: "lomi_chat_open",
                hash,
                action,
            },
        )
    }
    pub fn prepare_chat_open(
        &self,
        operation: &str,
        nonce: &str,
    ) -> Result<ChatSummary, ErrorCode> {
        let _producer = self
            .file_reads
            .clone()
            .try_acquire_owned()
            .map_err(|_| ErrorCode::ResourceExhausted)?;
        let dispatch = self
            .chat_open_dispatch
            .lock()
            .ok()
            .and_then(|d| d.clone())
            .ok_or(ErrorCode::UiNotReady)?;
        let (owner, project, command, permit, policy) = {
            let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let work = state.work.get(operation).ok_or(ErrorCode::ControlRevoked)?;
            if !work.claimed
                || work.native_committed
                || work.command.nonce != nonce
                || work.command.ui_epoch != state.projection.ui_epoch
                || work.command.domain_revision != state.projection.revision
            {
                return Err(ErrorCode::ControlRevoked);
            }
            let UiAction::OpenChat(command) = &work.command.action else {
                return Err(ErrorCode::ScopeDenied);
            };
            Self::validate_panel_action(&state, &work.pairing, &work.command.action)?;
            let project = Self::chat_access(&state, &work.pairing, &command.workspace_id)?;
            let session = &state.sessions[&work.pairing];
            if !Self::action_scopes(&work.command.action)
                .iter()
                .all(|s| session.grant.scopes.contains(*s))
                || (!command.create
                    && !session
                        .grant
                        .permits_chat_conversation(&command.conversation_id))
            {
                return Err(ErrorCode::ScopeDenied);
            }
            let pending_creates = state
                .work
                .values()
                .filter(|w| {
                    w.pairing == work.pairing
                        && w.native_committed
                        && w.chat_open.is_none()
                        && matches!(&w.command.action,UiAction::OpenChat(c) if c.create)
                })
                .count();
            if command.create
                && !session.grant.yolo
                && session.grant.chat_conversations.len() + pending_creates >= 64
            {
                return Err(ErrorCode::ResourceExhausted);
            }
            work.native_permit.check()?;
            let result = (
                work.pairing.clone(),
                project,
                command.clone(),
                work.native_permit.clone(),
                state.policy_revision,
            );
            state.work.get_mut(operation).unwrap().native_committed = true;
            result
        };
        let check = || {
            permit.check()?;
            if self.authorization.load(Ordering::SeqCst) != policy {
                return Err(ErrorCode::ControlRevoked);
            }
            let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            if Self::chat_access(&state, &owner, &command.workspace_id)? != project {
                return Err(ErrorCode::ControlRevoked);
            }
            Ok(())
        };
        check()?;
        let summary = dispatch(&project, &command, &check)?;
        check()?;
        if !valid_summary(&summary) || summary.conversation_id != command.conversation_id {
            return Err(ErrorCode::OutcomeUnknown);
        }
        let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        if self.authorization.load(Ordering::SeqCst) != policy
            || Self::chat_access(&state, &owner, &command.workspace_id)? != project
        {
            return Err(ErrorCode::ControlRevoked);
        }
        let work = state
            .work
            .get_mut(operation)
            .ok_or(ErrorCode::ControlRevoked)?;
        if work.command.nonce != nonce || work.chat_open.is_some() {
            return Err(ErrorCode::ControlRevoked);
        }
        permit.check()?;
        work.chat_open = Some(summary.clone());
        if command.create {
            let session = state
                .sessions
                .get_mut(&owner)
                .ok_or(ErrorCode::ControlRevoked)?;
            if !session.grant.yolo && session.grant.chat_conversations.len() >= 64 {
                return Err(ErrorCode::ResourceExhausted);
            }
            if session.grant.chat_conversations.len() < 64 {
                session
                    .grant
                    .chat_conversations
                    .insert(command.conversation_id.clone());
                session
                    .view
                    .chat_conversations
                    .push(command.conversation_id);
            }
        }
        Ok(summary)
    }
    pub fn set_chat_list_dispatch(&self, dispatch: ChatListDispatch) -> io::Result<()> {
        *self.chat_list_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    pub fn set_chat_read_dispatch(&self, dispatch: ChatReadDispatch) -> io::Result<()> {
        *self.chat_read_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    pub(super) fn chat_access(
        state: &State,
        owner: &str,
        workspace: &str,
    ) -> Result<String, ErrorCode> {
        let session = state
            .sessions
            .get(owner)
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .ok_or(ErrorCode::ControlRevoked)?;
        if !session.grant.scopes.contains("chat.read") {
            return Err(ErrorCode::ScopeDenied);
        }
        state
            .projection
            .workspaces
            .iter()
            .find(|w| w.id == workspace && session.grant.permits(w))
            .map(|w| w.project_id.clone())
            .ok_or(ErrorCode::TargetNotFound)
    }
    pub(super) fn list_chats(&self, owner: &str, input: ChatListInput) -> Reply {
        if !(1..=64).contains(&input.limit)
            || input.offset > 64
            || (input.offset > 0 && input.expected_revision.is_none())
            || input
                .expected_revision
                .as_ref()
                .is_some_and(|r| !revision(r))
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let (project, selection, policy) = {
            let Ok(state) = self.lock_state() else {
                return error(ErrorCode::ControlRevoked);
            };
            let project = match Self::chat_access(&state, owner, &input.workspace_id) {
                Ok(p) => p,
                Err(e) => return error(e),
            };
            let grant = &state.sessions[owner].grant;
            let selection = if grant.yolo {
                ChatConversationSelection::All
            } else {
                let mut ids: Vec<_> = grant.chat_conversations.iter().cloned().collect();
                ids.sort();
                ChatConversationSelection::Exact(ids)
            };
            (project, selection, state.policy_revision)
        };
        let _producer = match self.file_reads.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => return error(ErrorCode::ResourceExhausted),
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        let check = || {
            if self.authorization.load(Ordering::SeqCst) != policy {
                return Err(ErrorCode::ControlRevoked);
            }
            let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            if Self::chat_access(&state, owner, &input.workspace_id)? != project {
                return Err(ErrorCode::ControlRevoked);
            }
            let grant = &state
                .sessions
                .get(owner)
                .filter(|session| session.alive.load(Ordering::SeqCst))
                .ok_or(ErrorCode::ControlRevoked)?
                .grant;
            let selection_authorized = match &selection {
                ChatConversationSelection::Exact(ids) => {
                    ids.iter().all(|id| grant.permits_chat_conversation(id))
                }
                ChatConversationSelection::All => grant.yolo,
            };
            if !selection_authorized {
                return Err(ErrorCode::ControlRevoked);
            }
            if Instant::now() >= deadline {
                return Err(ErrorCode::TargetBusy);
            }
            Ok(())
        };
        let read = || -> Result<ChatList, ErrorCode> {
            check()?;
            // An empty exact grant reveals nothing; `All` is available only to
            // a YOLO session and remains bounded by the native history adapter.
            let mut items = match &selection {
                ChatConversationSelection::Exact(ids) if ids.is_empty() => Vec::new(),
                _ => {
                    let dispatch = self
                        .chat_list_dispatch
                        .lock()
                        .ok()
                        .and_then(|d| d.clone())
                        .ok_or(ErrorCode::UiNotReady)?;
                    dispatch(&project, selection.clone(), &check)?
                }
            };
            check()?;
            let valid_items = items.iter().all(|summary| {
                valid_summary(summary)
                    && match &selection {
                        ChatConversationSelection::Exact(ids) => {
                            ids.contains(&summary.conversation_id)
                        }
                        ChatConversationSelection::All => true,
                    }
            });
            if items.len() > 64 || !valid_items {
                return Err(ErrorCode::OutcomeUnknown);
            }
            items.sort_by(|a, b| a.conversation_id.cmp(&b.conversation_id));
            if items
                .windows(2)
                .any(|p| p[0].conversation_id == p[1].conversation_id)
            {
                return Err(ErrorCode::OutcomeUnknown);
            }
            let bytes = serde_json::to_vec(&(&project, &selection, &items))
                .map_err(|_| ErrorCode::ResourceExhausted)?;
            if bytes.len() > 48 * 1024 {
                return Err(ErrorCode::ResourceExhausted);
            }
            let revision = format!("{:x}", Sha256::digest(&bytes));
            if input
                .expected_revision
                .as_ref()
                .is_some_and(|r| r != &revision)
            {
                return Err(ErrorCode::RevisionConflict);
            }
            let total = items.len() as u16;
            if input.offset > total {
                return Err(ErrorCode::ResourceExhausted);
            }
            let items: Vec<_> = items
                .into_iter()
                .skip(input.offset as usize)
                .take(input.limit as usize)
                .collect();
            let end = input.offset + items.len() as u16;
            check()?;
            Ok(ChatList {
                workspace_id: input.workspace_id.clone(),
                revision,
                items,
                total,
                offset: input.offset,
                next_offset: (end < total).then_some(end),
            })
        };
        match read() {
            Ok(value) => Reply::ok(Data::ChatList(Box::new(value))),
            Err(e) => error(e),
        }
    }
    pub(super) fn read_chat(&self, owner: &str, input: ChatReadInput) -> Reply {
        if !valid_chat_id(&input.conversation_id)
            || !(2..=8192).contains(&input.max_chars)
            || input.start_utf16 > 4 * 1024 * 1024
            || (input.start_utf16 > 0 && input.expected_revision.is_none())
            || input
                .expected_revision
                .as_ref()
                .is_some_and(|r| !revision(r))
            || matches!(&input.part, ChatReadPart::Message {message_id: Some(id)} if !valid_chat_id(id))
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let access = |state: &State| {
            let project = Self::chat_access(state, owner, &input.workspace_id)?;
            if !state.sessions[owner]
                .grant
                .permits_chat_conversation(&input.conversation_id)
            {
                return Err(ErrorCode::ScopeDenied);
            }
            if input.include_send_target
                && !state.sessions[owner].grant.scopes.contains("chat.send")
            {
                return Err(ErrorCode::ScopeDenied);
            }
            Ok(project)
        };
        let (project, policy) = {
            let Ok(state) = self.lock_state() else {
                return error(ErrorCode::ControlRevoked);
            };
            let project = match access(&state) {
                Ok(p) => p,
                Err(e) => return error(e),
            };
            (project, state.policy_revision)
        };
        let _producer = match self.file_reads.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => return error(ErrorCode::ResourceExhausted),
        };
        let Some(dispatch) = self.chat_read_dispatch.lock().ok().and_then(|d| d.clone()) else {
            return error(ErrorCode::UiNotReady);
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        let check = || {
            if self.authorization.load(Ordering::SeqCst) != policy {
                return Err(ErrorCode::ControlRevoked);
            }
            let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            if access(&state)? != project {
                return Err(ErrorCode::ControlRevoked);
            }
            if Instant::now() >= deadline {
                return Err(ErrorCode::TargetBusy);
            }
            Ok(())
        };
        let read = || -> Result<ChatRead, ErrorCode> {
            check()?;
            let value = dispatch(&project, &input, &check)?;
            check()?;
            let count = value.content.encode_utf16().count() as u32;
            let end = input.start_utf16.saturating_add(count);
            let part_matches = match (&input.part, &value.part) {
                (ChatReadPart::Draft, ChatReadPart::Draft) => {
                    value.draft_revision.as_ref().is_some_and(|r| counter(r))
                }
                (
                    ChatReadPart::Message {
                        message_id: requested,
                    },
                    ChatReadPart::Message {
                        message_id: Some(id),
                    },
                ) => {
                    valid_chat_id(id)
                        && requested
                            .as_ref()
                            .or(value.conversation.latest_message_id.as_ref())
                            == Some(id)
                        && value.draft_revision.is_none()
                }
                _ => false,
            };
            if value.request.as_ref().is_some_and(|r| {
                !valid_chat_id(&r.request_id)
                    || !valid_chat_id(&r.assistant_id)
                    || !matches!(
                        r.status.as_str(),
                        "active" | "completed" | "cancelled" | "failed" | "interrupted"
                    )
            }) {
                return Err(ErrorCode::OutcomeUnknown);
            }
            if value.send_target.as_ref().is_some_and(|t| {
                !input.include_send_target
                    || !valid_chat_id(&t.connection_id)
                    || t.model.is_empty()
                    || t.model.len() > 200
                    || !(1..=32768).contains(&t.max_output_tokens)
            }) || !valid_summary(&value.conversation)
                || value.conversation.conversation_id != input.conversation_id
                || value.workspace_id != input.workspace_id
                || !part_matches
                || !revision(&value.revision)
                || value.start_utf16 != input.start_utf16
                || value.total_utf16 > 4 * 1024 * 1024
                || count > u32::from(input.max_chars)
                || end > value.total_utf16
                || value.next_utf16 != (end < value.total_utf16).then_some(end)
                || (end < value.total_utf16 && count == 0)
                || !matches!(value.role.as_str(), "user" | "assistant")
                || value.status.len() > 64
                || value
                    .parent_message_id
                    .as_ref()
                    .is_some_and(|id| !valid_chat_id(id))
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
            check()?;
            Ok(value)
        };
        match read() {
            Ok(value) => Reply::ok(Data::ChatRead(Box::new(value))),
            Err(e) => error(e),
        }
    }
}
