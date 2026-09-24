use super::{
    operations::{storage_error, UiMutation, Work},
    *,
};

pub type ChatSendPrepareDispatch = Arc<
    dyn Fn(
            &str,
            &ChatSendCommand,
            &dyn Fn() -> Result<(), ErrorCode>,
        ) -> Result<ChatSendPlan, ErrorCode>
        + Send
        + Sync,
>;
pub(super) enum Approval {
    Preparing,
    Ready {
        plan: Box<ChatSendPlan>,
        approved: bool,
    },
}
pub struct ChatSendAuthorization {
    pub command: ChatSendCommand,
    pub plan: ChatSendPlan,
}

impl Broker {
    pub fn set_chat_send_prepare_dispatch(
        &self,
        dispatch: ChatSendPrepareDispatch,
    ) -> io::Result<()> {
        *self
            .chat_send_prepare_dispatch
            .lock()
            .map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    fn send_access(state: &State, owner: &str, input: &ChatSendInput) -> Result<String, ErrorCode> {
        let project = Self::chat_access(state, owner, &input.workspace_id)?;
        let grant = &state.sessions[owner].grant;
        if !grant.scopes.contains("chat.send")
            || !grant.chat_conversations.contains(&input.conversation_id)
        {
            return Err(ErrorCode::ScopeDenied);
        }
        if !state.projection.panels.iter().any(|p| {
            p.id == input.panel_id
                && p.workspace_id == input.workspace_id
                && p.kind == "chat"
                && p.chat_conversation_id.as_ref() == Some(&input.conversation_id)
        }) {
            return Err(ErrorCode::TargetNotFound);
        }
        Ok(project)
    }
    fn send_work<'a>(
        &self,
        state: &'a State,
        operation: &str,
        nonce: &str,
    ) -> Result<&'a Work, ErrorCode> {
        let work = state.work.get(operation).ok_or(ErrorCode::ControlRevoked)?;
        if !work.claimed
            || work.command.nonce != nonce
            || work.command.ui_epoch != state.projection.ui_epoch
            || work.command.domain_revision != state.projection.revision
        {
            return Err(ErrorCode::ControlRevoked);
        }
        let UiAction::SendChat(command) = &work.command.action else {
            return Err(ErrorCode::ScopeDenied);
        };
        work.native_permit.check()?;
        if Self::send_access(state, &work.pairing, &command.input)? != work.project {
            return Err(ErrorCode::ControlRevoked);
        }
        Ok(work)
    }
    pub(super) fn send_chat(self: &Arc<Self>, owner: &str, input: ChatSendInput) -> Reply {
        if !valid_id(&input.request_key)
            || !valid_id(&input.panel_id)
            || !valid_chat_id(&input.conversation_id)
            || !valid_chat_id(&input.connection_id)
            || input.model.is_empty()
            || input.model.len() > 200
            || input.model.chars().any(char::is_control)
            || [
                &input.expected_revision,
                &input.expected_conversation_revision,
                &input.expected_draft_revision,
            ]
            .iter()
            .any(|v| {
                v.is_empty()
                    || v.len() > 16
                    || v.parse::<u64>()
                        .ok()
                        .is_none_or(|n| n >= 9_007_199_254_740_991)
            })
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        let project = match Self::send_access(&state, owner, &input) {
            Ok(p) => p,
            Err(e) => return error(e),
        };
        if input.retry_epoch != state.sessions[owner].retry_epoch {
            return error(ErrorCode::RetryWindowExpired);
        }
        let root = state.sessions[owner]
            .grant
            .workspace(&input.workspace_id)
            .unwrap();
        let hash = match receipts::fingerprint(
            &(&input, &root.project_path),
            &receipts::Target {
                workspace_id: &input.workspace_id,
                resource_id: &input.conversation_id,
                generation: &state.projection.ui_epoch,
                revision: &input.expected_revision,
            },
        ) {
            Ok(h) => h,
            Err(e) => return storage_error(e),
        };
        let key = receipts::Key {
            pairing_id: owner,
            project_id: &project,
            retry_epoch: &input.retry_epoch,
            request_key: &input.request_key,
            tool: "lomi_chat_send",
        };
        match self.replay(&state, owner, &key, hash) {
            Ok(Some(r)) => return r,
            Err(e) => return *e,
            Ok(None) => {}
        }
        let ids = (new_id(), new_id(), new_id());
        let (Ok(request_id), Ok(user_id), Ok(assistant_id)) = ids else {
            return error(ErrorCode::ResourceExhausted);
        };
        self.enqueue_ui(
            &mut state,
            owner,
            UiMutation {
                workspace: input.workspace_id.clone(),
                project,
                revision: input.expected_revision.clone(),
                retry_epoch: input.retry_epoch.clone(),
                request_key: input.request_key.clone(),
                tool: "lomi_chat_send",
                hash,
                action: UiAction::SendChat(ChatSendCommand {
                    workspace_id: input.workspace_id.clone(),
                    input,
                    request_id,
                    user_id,
                    assistant_id,
                    not_after_millis: (std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                        + 120_000)
                        .to_string(),
                }),
            },
        )
    }
    pub(super) fn chat_send_reserved(command: &ChatSendCommand) -> ChatSent {
        ChatSent {
            workspace_id: command.workspace_id.clone(),
            panel_id: command.input.panel_id.clone(),
            conversation_id: command.input.conversation_id.clone(),
            request_id: command.request_id.clone(),
            user_id: command.user_id.clone(),
            assistant_id: command.assistant_id.clone(),
            connection_id: command.input.connection_id.clone(),
            model: command.input.model.clone(),
            draft_revision: None,
            rejection: None,
        }
    }
    pub fn prepare_chat_send(
        &self,
        operation: &str,
        nonce: &str,
    ) -> Result<ChatSendPlan, ErrorCode> {
        let _producer = self
            .file_reads
            .clone()
            .try_acquire_owned()
            .map_err(|_| ErrorCode::ResourceExhausted)?;
        let dispatch = self
            .chat_send_prepare_dispatch
            .lock()
            .ok()
            .and_then(|d| d.clone())
            .ok_or(ErrorCode::UiNotReady)?;
        let (project, command) = {
            let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            let work = self.send_work(&state, operation, nonce)?;
            if work.native_committed || work.chat_send.is_some() {
                return Err(ErrorCode::ControlRevoked);
            }
            let UiAction::SendChat(command) = &work.command.action else {
                unreachable!()
            };
            let result = (work.project.clone(), command.clone());
            state.work.get_mut(operation).unwrap().chat_send = Some(Approval::Preparing);
            result
        };
        let check = || {
            let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            self.send_work(&state, operation, nonce).map(|_| ())
        };
        check()?;
        let plan = dispatch(&project, &command, &check)?;
        check()?;
        if plan.plan_hash.len() != 64
            || !plan.plan_hash.bytes().all(|b| b.is_ascii_hexdigit())
            || plan.connection_id != command.input.connection_id
            || plan.model != command.input.model
            || plan.connection_name.len() > 200
            || plan.conversation_title.len() > 256
            || plan.provider.len() > 64
            || !(1..=32768).contains(&plan.max_output_tokens)
            || plan.draft_text.len() > 128 * 1024
            || plan.system.len() > 128 * 1024
            || plan.attachments.len() > 100
            || !(1..=2000).contains(&plan.message_count)
            || plan.context_bytes > 40 * 1024 * 1024
            || plan
                .temperature
                .is_some_and(|t| !t.is_finite() || !(0.0..=2.0).contains(&t))
            || plan.attachments.iter().any(|a| {
                a.name.len() > 255 || a.mime.len() > 100 || a.byte_length > 10 * 1024 * 1024
            })
        {
            return Err(ErrorCode::ResourceExhausted);
        }
        let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        let work = self.send_work(&state, operation, nonce)?;
        self.store
            .lock()
            .map_err(|_| ErrorCode::StorageUnavailable)?
            .transition(
                &work.pairing,
                &work.project,
                operation,
                receipts::State::AwaitingUser,
                receipts::Effect::None,
                now(),
            )
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        state.work.get_mut(operation).unwrap().chat_send = Some(Approval::Ready {
            plan: Box::new(plan.clone()),
            approved: false,
        });
        Ok(plan)
    }
    pub fn chat_send_pending(&self, operation: &str, nonce: &str, hash: &str) -> bool {
        let Ok(state) = self.lock_state() else {
            return false;
        };
        self.send_work(&state, operation, nonce).is_ok_and(|w| !w.native_committed && matches!(&w.chat_send, Some(Approval::Ready { plan, approved: false }) if plan.plan_hash == hash))
    }
    pub fn decide_chat_send(
        &self,
        operation: &str,
        nonce: &str,
        hash: &str,
        approved: bool,
    ) -> Result<(), ErrorCode> {
        let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        let work = self.send_work(&state, operation, nonce)?;
        if work.native_committed
            || !matches!(&work.chat_send, Some(Approval::Ready { plan, approved: false }) if plan.plan_hash == hash)
        {
            return Err(ErrorCode::ControlRevoked);
        }
        let mut store = self
            .store
            .lock()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        if !approved {
            let UiAction::SendChat(command) = &work.command.action else {
                unreachable!()
            };
            let mut result = Self::chat_send_reserved(command);
            result.rejection = Some(ErrorCode::ScopeDenied);
            store
                .finish_chat_send(&work.pairing, &work.project, operation, &result)
                .map_err(|_| ErrorCode::StorageUnavailable)?;
        }
        store
            .transition(
                &work.pairing,
                &work.project,
                operation,
                if approved {
                    receipts::State::Queued
                } else {
                    receipts::State::Cancelled
                },
                receipts::Effect::None,
                now(),
            )
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        if approved {
            let Some(Approval::Ready { approved, .. }) =
                &mut state.work.get_mut(operation).unwrap().chat_send
            else {
                unreachable!()
            };
            *approved = true;
        } else {
            state.work.remove(operation);
        }
        Ok(())
    }
    pub fn authorize_chat_send(
        &self,
        operation: &str,
        nonce: &str,
        hash: &str,
    ) -> Result<ChatSendAuthorization, ErrorCode> {
        let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        let work = self.send_work(&state, operation, nonce)?;
        let Some(Approval::Ready {
            plan,
            approved: true,
        }) = &work.chat_send
        else {
            return Err(ErrorCode::ScopeDenied);
        };
        if work.native_committed || plan.plan_hash != hash {
            return Err(ErrorCode::ControlRevoked);
        }
        let UiAction::SendChat(command) = &work.command.action else {
            unreachable!()
        };
        let result = ChatSendAuthorization {
            command: command.clone(),
            plan: *plan.clone(),
        };
        self.store
            .lock()
            .map_err(|_| ErrorCode::StorageUnavailable)?
            .transition(
                &work.pairing,
                &work.project,
                operation,
                receipts::State::Running,
                receipts::Effect::None,
                now(),
            )
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        state.work.get_mut(operation).unwrap().native_committed = true;
        Ok(result)
    }
    pub fn check_chat_send(
        &self,
        operation: &str,
        nonce: &str,
        hash: &str,
    ) -> Result<(), ErrorCode> {
        let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        let work = self.send_work(&state, operation, nonce)?;
        if !work.native_committed
            || !matches!(&work.chat_send, Some(Approval::Ready { plan, approved: true }) if plan.plan_hash == hash)
        {
            return Err(ErrorCode::ControlRevoked);
        }
        Ok(())
    }
    pub fn record_chat_send(
        &self,
        operation: &str,
        nonce: &str,
        result: ChatSent,
    ) -> Result<(), ErrorCode> {
        let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        let work = self.send_work(&state, operation, nonce)?;
        let UiAction::SendChat(command) = &work.command.action else {
            unreachable!()
        };
        let mut expected = Self::chat_send_reserved(command);
        expected.draft_revision = Some(
            (command
                .input
                .expected_draft_revision
                .parse::<u64>()
                .map_err(|_| ErrorCode::OutcomeUnknown)?
                + 1)
            .to_string(),
        );
        if !work.native_committed || result != expected {
            return Err(ErrorCode::OutcomeUnknown);
        }
        self.store
            .lock()
            .map_err(|_| ErrorCode::StorageUnavailable)?
            .finish_chat_send(&work.pairing, &work.project, operation, &result)
            .map_err(|_| ErrorCode::OutcomeUnknown)
    }

    /// Native preflight alone may attest that no provider request was committed.
    pub fn reject_chat_send(
        &self,
        operation: &str,
        nonce: &str,
        code: ErrorCode,
    ) -> Result<(), ErrorCode> {
        let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        let work = self.send_work(&state, operation, nonce)?;
        if !work.native_committed {
            return Err(ErrorCode::ControlRevoked);
        }
        let UiAction::SendChat(command) = &work.command.action else {
            unreachable!()
        };
        let mut result = Self::chat_send_reserved(command);
        result.rejection = Some(code);
        self.store
            .lock()
            .map_err(|_| ErrorCode::StorageUnavailable)?
            .finish_chat_send(&work.pairing, &work.project, operation, &result)
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        state.work.get_mut(operation).unwrap().chat_send_rejection = Some(code);
        Ok(())
    }
}
