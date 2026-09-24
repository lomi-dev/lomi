use super::{
    commands::Chats,
    generation::{self, Context},
    preferences::Connection,
    store::{Conversation, Start},
};
use lomi_control_protocol::{
    chat::{ChatSendAttachment, ChatSendCommand, ChatSendPlan},
    ErrorCode,
};
use sha2::{Digest, Sha256};
use tauri::Manager;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SendReply {
    pub accepted: super::store::Accepted,
    pub result: lomi_control_protocol::chat::ChatSent,
}

pub(crate) fn commit(
    app: &tauri::AppHandle,
    broker: &lomi_control_core::broker::Broker,
    operation: &str,
    nonce: &str,
    hash: &str,
    channel: tauri::ipc::Channel<serde_json::Value>,
) -> Result<SendReply, ErrorCode> {
    let authorized = broker.authorize_chat_send(operation, nonce, hash)?;
    let command = authorized.command;
    let start = start_input(&command, authorized.plan.draft_text)?;
    let backend = app
        .state::<Chats>()
        .0
        .try_lock()
        .map_err(|_| ErrorCode::TargetBusy)?
        .clone()
        .ok_or(ErrorCode::UiNotReady)?;
    let rejected = std::cell::Cell::new(None);
    let outcome = backend.start_checked(
        start,
        channel,
        |start, conversation, connection, context| {
            let validation =
                fingerprint(start, conversation, connection, context).and_then(|current| {
                    if current != hash {
                        return Err(ErrorCode::RevisionConflict);
                    }
                    broker.check_chat_send(operation, nonce, hash)
                });
            validation.map_err(|code| {
                rejected.set(Some(code));
                error_string(code)
            })
        },
        || {
            broker
                .check_chat_send(operation, nonce, hash)
                .map_err(error_string)
        },
    );
    let value = match outcome {
        Ok(value) => value,
        Err(error) => {
            if error.starts_with("conflict:") {
                rejected.set(Some(ErrorCode::RevisionConflict));
            }
            if let Some(code) = rejected.get() {
                broker.reject_chat_send(operation, nonce, code)?;
                return Err(code);
            }
            return Err(ErrorCode::OutcomeUnknown);
        }
    };
    let accepted: super::store::Accepted =
        serde_json::from_value(value).map_err(|_| ErrorCode::OutcomeUnknown)?;
    if accepted.request_id != command.request_id
        || accepted.user_id != command.user_id
        || accepted.assistant_id != command.assistant_id
        || accepted.repeated
    {
        return Err(ErrorCode::OutcomeUnknown);
    }
    let result = lomi_control_protocol::chat::ChatSent {
        workspace_id: command.workspace_id,
        panel_id: command.input.panel_id,
        conversation_id: command.input.conversation_id,
        request_id: command.request_id,
        user_id: command.user_id,
        assistant_id: command.assistant_id,
        connection_id: command.input.connection_id,
        model: command.input.model,
        draft_revision: Some(accepted.draft_revision.to_string()),
        rejection: None,
    };
    broker.record_chat_send(operation, nonce, result.clone())?;
    Ok(SendReply { accepted, result })
}

fn failure(error: String) -> ErrorCode {
    if error.starts_with("conflict:") {
        ErrorCode::RevisionConflict
    } else if let Ok(code) = serde_json::from_value(serde_json::Value::String(error)) {
        code
    } else {
        ErrorCode::UiNotReady
    }
}

fn error_string(error: ErrorCode) -> String {
    serde_json::to_value(error)
        .unwrap()
        .as_str()
        .unwrap()
        .into()
}

pub(super) fn start_input(command: &ChatSendCommand, text: String) -> Result<Start, ErrorCode> {
    let revision = |s: &str| {
        s.parse::<i64>()
            .ok()
            .filter(|n| (0..9_007_199_254_740_991).contains(n))
            .ok_or(ErrorCode::ResourceExhausted)
    };
    if [
        &command.input.conversation_id,
        &command.request_id,
        &command.user_id,
        &command.assistant_id,
    ]
    .iter()
    .any(|id| !lomi_control_protocol::chat::valid_chat_id(id))
        || text.len() > 128 * 1024
    {
        return Err(ErrorCode::ResourceExhausted);
    }
    Ok(Start {
        request_id: command.request_id.clone(),
        conversation_id: command.input.conversation_id.clone(),
        assistant_id: command.assistant_id.clone(),
        user_id: command.user_id.clone(),
        expected_revision: revision(&command.input.expected_conversation_revision)?,
        draft_revision: revision(&command.input.expected_draft_revision)?,
        action: "send".into(),
        target_id: None,
        text,
    })
}

pub(super) fn fingerprint(
    start: &Start,
    conversation: &Conversation,
    connection: &Connection,
    context: &Context,
) -> Result<String, ErrorCode> {
    // Includes exact attachment bytes in the payload, provider/credential
    // revision, config and project origin. Credentials themselves are absent.
    let bytes = serde_json::to_vec(&(
        start,
        conversation,
        connection,
        &context.payload,
        &context.attachments,
    ))
    .map_err(|_| ErrorCode::StorageUnavailable)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub(crate) fn prepare(
    app: &tauri::AppHandle,
    project: &str,
    command: &ChatSendCommand,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<ChatSendPlan, ErrorCode> {
    check()?;
    let backend = app
        .state::<Chats>()
        .0
        .try_lock()
        .map_err(|_| ErrorCode::TargetBusy)?
        .clone()
        .ok_or(ErrorCode::UiNotReady)?;
    let services = backend
        .services
        .try_lock()
        .map_err(|_| ErrorCode::TargetBusy)?;
    let store = services
        .store
        .as_ref()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let conversation = store
        .conversation(&command.input.conversation_id)
        .map_err(|_| ErrorCode::TargetNotFound)?;
    if conversation.origin.project_id != project {
        return Err(ErrorCode::TargetNotFound);
    }
    if conversation.config.connection_id.as_ref() != Some(&command.input.connection_id)
        || conversation.config.model != command.input.model
    {
        return Err(ErrorCode::RevisionConflict);
    }
    store
        .idle(&conversation.id)
        .map_err(|_| ErrorCode::TargetBusy)?;
    let settings = services
        .settings
        .as_ref()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let connection = settings
        .data
        .connections
        .iter()
        .find(|c| c.id == command.input.connection_id && c.enabled)
        .ok_or(ErrorCode::UiNotReady)?;
    if backend
        .changing
        .lock()
        .map_err(|_| ErrorCode::TargetBusy)?
        .contains(&connection.id)
    {
        return Err(ErrorCode::TargetBusy);
    }
    let draft = store
        .draft(&conversation.id)
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let start = start_input(command, draft.text)?;
    let context = generation::context(
        store,
        &backend.owner.root,
        &start,
        &conversation,
        connection,
        &|| check().map_err(error_string),
    )
    .map_err(failure)?;
    // Approval renders every attachment name. Larger sets need an explicit
    // paginated review before this path can support them.
    if context.attachments.len() > 100 {
        return Err(ErrorCode::ResourceExhausted);
    }
    let plan_hash = fingerprint(&start, &conversation, connection, &context)?;
    let context_bytes = serde_json::to_vec(&context.payload)
        .map_err(|_| ErrorCode::StorageUnavailable)?
        .len() as u32;
    let attachments = context
        .attachments
        .into_iter()
        .map(|a| ChatSendAttachment {
            name: a.name,
            mime: a.mime,
            byte_length: a.size as u32,
        })
        .collect();
    check()?;
    Ok(ChatSendPlan {
        plan_hash,
        conversation_title: conversation.title,
        connection_id: connection.id.clone(),
        connection_name: connection.name.clone(),
        provider: connection.provider.clone(),
        model: conversation.config.model,
        max_output_tokens: conversation.config.max_output_tokens,
        temperature: conversation.config.temperature,
        draft_text: start.text,
        system: conversation.config.system,
        message_count: context.payload["messages"]
            .as_array()
            .ok_or(ErrorCode::StorageUnavailable)?
            .len() as u32,
        context_bytes,
        attachments,
    })
}
