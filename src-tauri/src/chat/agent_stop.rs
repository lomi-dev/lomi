use super::{agent, backend::Backend, commands::Chats};
use lomi_control_protocol::{
    chat::{ChatStopInput, ChatStopped},
    ErrorCode,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tauri::Manager;

pub(crate) fn stop(
    app: &tauri::AppHandle,
    project: &str,
    input: &ChatStopInput,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<ChatStopped, ErrorCode> {
    check()?;
    let backend = app
        .state::<Chats>()
        .0
        .try_lock()
        .map_err(|_| ErrorCode::TargetBusy)?
        .clone()
        .ok_or(ErrorCode::UiNotReady)?;
    stop_backend(&backend, project, input, check)
}

pub(super) fn stop_backend(
    backend: &Arc<Backend>,
    project: &str,
    input: &ChatStopInput,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<ChatStopped, ErrorCode> {
    check()?;
    let before = {
        let services = backend
            .services
            .try_lock()
            .map_err(|_| ErrorCode::TargetBusy)?;
        let store = services
            .store
            .as_ref()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        agent::summary(store, project, &input.conversation_id)?;
        agent::request(store, &input.conversation_id, Some(&input.request_id))?
            .ok_or(ErrorCode::TargetNotFound)?
    };
    if before.status != "active" {
        check()?;
        return Ok(ChatStopped {
            workspace_id: input.workspace_id.clone(),
            conversation_id: input.conversation_id.clone(),
            request: before,
            already_finished: true,
        });
    }
    let request = backend
        .requests
        .try_lock()
        .map_err(|_| ErrorCode::TargetBusy)?
        .get(&input.request_id)
        .cloned()
        .ok_or(ErrorCode::UiNotReady)?;
    if request.conversation.as_deref() != Some(&input.conversation_id) {
        return Err(ErrorCode::TargetNotFound);
    }
    check()?;
    // Only the verified live request may be cancelled. Never register a pending
    // cancellation for an unknown ID or choose a conversation's replacement run.
    backend
        .cancel_known(&input.request_id, &request)
        .map_err(|_| ErrorCode::OutcomeUnknown)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut done = request
        .done
        .0
        .lock()
        .map_err(|_| ErrorCode::OutcomeUnknown)?;
    while done.is_none() {
        check().map_err(|_| ErrorCode::OutcomeUnknown)?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ErrorCode::OutcomeUnknown);
        }
        done = request
            .done
            .1
            .wait_timeout(done, remaining.min(Duration::from_millis(50)))
            .map_err(|_| ErrorCode::OutcomeUnknown)?
            .0;
    }
    if done.as_ref().unwrap().is_err() {
        return Err(ErrorCode::OutcomeUnknown);
    }
    drop(done);
    let services = backend
        .services
        .try_lock()
        .map_err(|_| ErrorCode::OutcomeUnknown)?;
    let store = services
        .store
        .as_ref()
        .map_err(|_| ErrorCode::OutcomeUnknown)?;
    let saved = agent::request(store, &input.conversation_id, Some(&input.request_id))
        .map_err(|_| ErrorCode::OutcomeUnknown)?
        .ok_or(ErrorCode::OutcomeUnknown)?;
    if saved.assistant_id != before.assistant_id || saved.status == "active" {
        return Err(ErrorCode::OutcomeUnknown);
    }
    check().map_err(|_| ErrorCode::OutcomeUnknown)?;
    Ok(ChatStopped {
        workspace_id: input.workspace_id.clone(),
        conversation_id: input.conversation_id.clone(),
        request: saved,
        already_finished: false,
    })
}
