use super::operations::{storage_error, UiMutation};
use super::*;

pub type ChatDraftDispatch = Arc<
    dyn Fn(
            &str,
            &ChatDraftInput,
            &dyn Fn() -> Result<(), ErrorCode>,
        ) -> Result<ChatDraftUpdated, ErrorCode>
        + Send
        + Sync,
>;

impl Broker {
    pub fn set_chat_draft_dispatch(&self, dispatch: ChatDraftDispatch) -> io::Result<()> {
        *self.chat_draft_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }

    fn draft_access(
        state: &State,
        owner: &str,
        input: &ChatDraftInput,
    ) -> Result<String, ErrorCode> {
        let project = Self::chat_access(state, owner, &input.workspace_id)?;
        let session = &state.sessions[owner];
        if !session.grant.scopes.contains("chat.draft")
            || !session
                .grant
                .chat_conversations
                .contains(&input.conversation_id)
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

    pub(super) fn draft_chat(self: &Arc<Self>, owner: &str, input: ChatDraftInput) -> Reply {
        if !valid_id(&input.request_key)
            || !valid_id(&input.panel_id)
            || !valid_chat_id(&input.conversation_id)
            || input.text.len() > 32 * 1024
            || [
                &input.expected_revision,
                &input.expected_draft_revision,
                &input.expected_conversation_revision,
            ]
            .iter()
            .any(|s| {
                s.is_empty()
                    || s.len() > 16
                    || s.parse::<u64>()
                        .ok()
                        .is_none_or(|v| v >= 9_007_199_254_740_991)
            })
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        let project = match Self::draft_access(&state, owner, &input) {
            Ok(p) => p,
            Err(e) => return error(e),
        };
        if state.sessions[owner].retry_epoch != input.retry_epoch {
            return error(ErrorCode::RetryWindowExpired);
        }
        let workspace = state
            .projection
            .workspaces
            .iter()
            .find(|w| w.id == input.workspace_id)
            .unwrap();
        let hash = match receipts::fingerprint(
            &(&input, &workspace.project_path),
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
            tool: "lomi_chat_draft",
        };
        match self.replay(&state, owner, &key, hash) {
            Ok(Some(reply)) => return reply,
            Err(e) => return *e,
            Ok(None) => {}
        }
        self.enqueue_ui(
            &mut state,
            owner,
            UiMutation {
                workspace: input.workspace_id.clone(),
                project,
                revision: input.expected_revision.clone(),
                retry_epoch: input.retry_epoch.clone(),
                request_key: input.request_key.clone(),
                tool: "lomi_chat_draft",
                hash,
                action: UiAction::DraftChat(ChatDraftCommand {
                    workspace_id: input.workspace_id.clone(),
                    input,
                    not_after_millis: (std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                        + 30_000)
                        .to_string(),
                }),
            },
        )
    }

    pub fn commit_chat_draft(
        &self,
        operation: &str,
        nonce: &str,
    ) -> Result<ChatDraftUpdated, ErrorCode> {
        let _producer = self
            .file_reads
            .clone()
            .try_acquire_owned()
            .map_err(|_| ErrorCode::ResourceExhausted)?;
        let dispatch = self
            .chat_draft_dispatch
            .lock()
            .ok()
            .and_then(|d| d.clone())
            .ok_or(ErrorCode::UiNotReady)?;
        let (owner, project, input, permit, policy) = {
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
            let UiAction::DraftChat(command) = &work.command.action else {
                return Err(ErrorCode::ScopeDenied);
            };
            let project = Self::draft_access(&state, &work.pairing, &command.input)?;
            work.native_permit.check()?;
            let result = (
                work.pairing.clone(),
                project,
                command.input.clone(),
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
            if Self::draft_access(&state, &owner, &input)? != project {
                return Err(ErrorCode::ControlRevoked);
            }
            Ok(())
        };
        check()?;
        let result = match dispatch(&project, &input, &check) {
            Ok(result) => result,
            Err(code) => {
                // The native draft adapter returns these only before its CAS.
                // Storage, deadline and revoke failures may follow a write.
                if matches!(
                    code,
                    ErrorCode::RevisionConflict
                        | ErrorCode::TargetNotFound
                        | ErrorCode::TargetBusy
                        | ErrorCode::UiNotReady
                        | ErrorCode::ResourceExhausted
                ) {
                    let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
                    if let Some(work) = state
                        .work
                        .get_mut(operation)
                        .filter(|w| w.command.nonce == nonce)
                    {
                        work.chat_draft_rejection = Some(code);
                    }
                }
                return Err(code);
            }
        };
        check()?;
        if result.workspace_id != input.workspace_id
            || result.panel_id != input.panel_id
            || result.conversation_id != input.conversation_id
            || result.conversation_revision != input.expected_conversation_revision
            || result.draft_revision
                != (input
                    .expected_draft_revision
                    .parse::<u64>()
                    .map_err(|_| ErrorCode::OutcomeUnknown)?
                    + 1)
                .to_string()
            || result.text_sha256 != format!("{:x}", sha2::Sha256::digest(input.text.as_bytes()))
            || result.total_utf16 as usize != input.text.encode_utf16().count()
        {
            return Err(ErrorCode::OutcomeUnknown);
        }
        let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        permit.check()?;
        if self.authorization.load(Ordering::SeqCst) != policy
            || Self::draft_access(&state, &owner, &input)? != project
        {
            return Err(ErrorCode::ControlRevoked);
        }
        self.store
            .lock()
            .map_err(|_| ErrorCode::StorageUnavailable)?
            .record_result(
                &owner,
                &project,
                operation,
                &OperationResult::ChatDraftUpdated(Box::new(result.clone())),
            )
            .map_err(|_| ErrorCode::OutcomeUnknown)?;
        Ok(result)
    }
}
