use super::{operations::storage_error, *};

pub type ChatStopDispatch = Arc<
    dyn Fn(
            &str,
            &ChatStopInput,
            &dyn Fn() -> Result<(), ErrorCode>,
        ) -> Result<ChatStopped, ErrorCode>
        + Send
        + Sync,
>;

impl Broker {
    pub fn set_chat_stop_dispatch(&self, dispatch: ChatStopDispatch) -> io::Result<()> {
        *self.chat_stop_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    fn stop_access(state: &State, owner: &str, input: &ChatStopInput) -> Result<String, ErrorCode> {
        let project = Self::chat_access(state, owner, &input.workspace_id)?;
        let grant = &state.sessions[owner].grant;
        if !grant.scopes.contains("chat.stop")
            || !grant.chat_conversations.contains(&input.conversation_id)
        {
            return Err(ErrorCode::ScopeDenied);
        }
        Ok(project)
    }
    pub(super) fn stop_chat(&self, owner: &str, input: ChatStopInput) -> Reply {
        if !valid_chat_id(&input.conversation_id)
            || !valid_chat_id(&input.request_id)
            || !valid_id(&input.request_key)
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let _producer = match self.file_reads.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => return error(ErrorCode::ResourceExhausted),
        };
        let Some(dispatch) = self.chat_stop_dispatch.lock().ok().and_then(|d| d.clone()) else {
            return error(ErrorCode::UiNotReady);
        };
        let prepared = (|| -> Result<_, Box<Reply>> {
            let state = self
                .lock_state()
                .map_err(|_| error(ErrorCode::ControlRevoked))?;
            let project = Self::stop_access(&state, owner, &input).map_err(error)?;
            if input.retry_epoch != state.sessions[owner].retry_epoch {
                return Err(error(ErrorCode::RetryWindowExpired).into());
            }
            let root = state.sessions[owner]
                .grant
                .workspace(&input.workspace_id)
                .unwrap();
            let hash = receipts::fingerprint(
                &(&input, &root.project_path),
                &receipts::Target {
                    workspace_id: &input.workspace_id,
                    resource_id: &input.conversation_id,
                    generation: &input.request_id,
                    revision: "",
                },
            )
            .map_err(storage_error)?;
            let key = receipts::Key {
                pairing_id: owner,
                project_id: &project,
                retry_epoch: &input.retry_epoch,
                request_key: &input.request_key,
                tool: "lomi_chat_stop",
            };
            if let Some(reply) = self.replay(&state, owner, &key, hash).map_err(|r| *r)? {
                return Err(Box::new(reply));
            }
            let mut store = self
                .store
                .lock()
                .map_err(|_| error(ErrorCode::StorageUnavailable))?;
            let reservation = store.reserve(&key, hash, now()).map_err(storage_error)?;
            if !reservation.created {
                return Err(Box::new(Self::operation_reply(reservation.receipt)));
            }
            let id = reservation.receipt.operation_id;
            store
                .bind_workspace(owner, &project, &id, &input.workspace_id)
                .map_err(storage_error)?;
            self.check_policy(&state)
                .map_err(|_| error(ErrorCode::ControlRevoked))?;
            store
                .transition(
                    owner,
                    &project,
                    &id,
                    receipts::State::Running,
                    receipts::Effect::None,
                    now(),
                )
                .map_err(storage_error)?;
            Ok((project, id, state.policy_revision))
        })();
        let (project, operation, policy) = match prepared {
            Ok(p) => p,
            Err(reply) => return *reply,
        };
        let deadline = Instant::now() + Duration::from_secs(6);
        let check = || {
            if Instant::now() >= deadline {
                return Err(ErrorCode::DeadlineExceeded);
            }
            let state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
            if state.policy_revision != policy
                || Self::stop_access(&state, owner, &input)? != project
            {
                return Err(ErrorCode::ControlRevoked);
            }
            Ok(())
        };
        let result = check()
            .and_then(|()| dispatch(&project, &input, &check))
            .and_then(|result| {
                check().map_err(|_| ErrorCode::OutcomeUnknown)?;
                if result.workspace_id != input.workspace_id
                    || result.conversation_id != input.conversation_id
                    || result.request.request_id != input.request_id
                    || !valid_chat_id(&result.request.assistant_id)
                    || !matches!(
                        result.request.status.as_str(),
                        "completed" | "cancelled" | "failed" | "interrupted"
                    )
                {
                    return Err(ErrorCode::OutcomeUnknown);
                }
                Ok(result)
            });
        let (result, next, effect) = match result {
            Ok(r) => (
                OperationResult::ChatStopped(Box::new(r)),
                receipts::State::Succeeded,
                receipts::Effect::Complete,
            ),
            Err(ErrorCode::OutcomeUnknown) => (
                OperationResult::Failure {
                    code: ErrorCode::OutcomeUnknown,
                },
                receipts::State::OutcomeUnknown,
                receipts::Effect::Unknown,
            ),
            Err(code) => (
                OperationResult::Failure { code },
                receipts::State::Failed,
                receipts::Effect::None,
            ),
        };
        let finalized = (|| {
            let mut store = self
                .store
                .lock()
                .map_err(|_| ErrorCode::StorageUnavailable)?;
            store
                .record_result(owner, &project, &operation, &result)
                .map_err(|_| ErrorCode::OutcomeUnknown)?;
            store
                .transition(owner, &project, &operation, next, effect, now())
                .map_err(|_| ErrorCode::OutcomeUnknown)
        })();
        if let Err(code) = check() {
            return error(code);
        }
        match finalized {
            Ok(receipt) => Self::operation_reply(receipt),
            Err(code) => error(code),
        }
    }
}
