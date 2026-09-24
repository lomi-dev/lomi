use super::{agent, backend::Backend, commands::Chats};
use lomi_control_core::broker::NativePermit;
use lomi_control_protocol::ErrorCode;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tauri::Manager;

pub(crate) fn close(
    app: &tauri::AppHandle,
    project: &str,
    conversations: &[String],
    permit: Option<NativePermit>,
) -> Result<(), ErrorCode> {
    let backend = app
        .state::<Chats>()
        .0
        .try_lock()
        .map_err(|_| ErrorCode::TargetBusy)?
        .clone()
        .ok_or(ErrorCode::UiNotReady)?;
    close_backend(&backend, project, conversations, permit)
}

pub(super) fn close_backend(
    backend: &Arc<Backend>,
    project: &str,
    conversations: &[String],
    permit: Option<NativePermit>,
) -> Result<(), ErrorCode> {
    if conversations.len() > 64 {
        return Err(ErrorCode::ResourceExhausted);
    }
    let requests = {
        let services = backend
            .services
            .try_lock()
            .map_err(|_| ErrorCode::TargetBusy)?;
        let store = services
            .store
            .as_ref()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        let requests = backend
            .requests
            .try_lock()
            .map_err(|_| ErrorCode::TargetBusy)?;
        for conversation in conversations {
            agent::summary(store, project, conversation)?;
            if let Some(request) = agent::request(store, conversation, None)? {
                if request.status == "active" && !requests.contains_key(&request.request_id) {
                    return Err(ErrorCode::UiNotReady);
                }
            }
        }
        let targets = requests
            .iter()
            .filter(|(_, r)| {
                r.conversation
                    .as_ref()
                    .is_some_and(|id| conversations.contains(id))
            })
            .map(|(id, r)| (id.clone(), r.clone()))
            .collect::<Vec<_>>();
        for (_, request) in &targets {
            if request
                .done
                .0
                .try_lock()
                .map_err(|_| ErrorCode::TargetBusy)?
                .as_ref()
                .is_some_and(|result| result.is_err())
            {
                return Err(ErrorCode::StorageUnavailable);
            }
        }
        let mut closing = backend.closing.lock().map_err(|_| ErrorCode::TargetBusy)?;
        closing.retain(|_, active| active());
        if conversations.iter().any(|id| closing.contains_key(id)) {
            return Err(ErrorCode::TargetBusy);
        }
        let Some(permit) = permit.as_ref() else {
            return Ok(());
        };
        permit.check()?;
        // The services lock also covers generation admission. Hold this barrier
        // until the broker settles the layout ACK, including failed/late ACKs.
        for id in conversations {
            let permit = permit.clone();
            closing.insert(id.clone(), Box::new(move || permit.check().is_ok()));
        }
        targets
    };
    let permit = permit.unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    for (id, request) in &requests {
        permit.check().map_err(|_| ErrorCode::OutcomeUnknown)?;
        if request
            .done
            .0
            .lock()
            .map_err(|_| ErrorCode::OutcomeUnknown)?
            .is_none()
        {
            backend
                .cancel_known(id, request)
                .map_err(|_| ErrorCode::OutcomeUnknown)?;
        }
    }
    for (_, request) in requests {
        let mut done = request
            .done
            .0
            .lock()
            .map_err(|_| ErrorCode::OutcomeUnknown)?;
        while done.is_none() {
            permit.check().map_err(|_| ErrorCode::OutcomeUnknown)?;
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
        done.as_ref()
            .unwrap()
            .as_ref()
            .map_err(|_| ErrorCode::OutcomeUnknown)?;
    }
    permit.check().map_err(|_| ErrorCode::OutcomeUnknown)
}
