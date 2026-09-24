use super::{commands::Chats, store::Store};
use lomi_control_core::broker::ChatConversationSelection;
use lomi_control_protocol::{
    chat::{
        valid_chat_id, ChatDraftInput, ChatDraftUpdated, ChatOpenCommand, ChatRead, ChatReadInput,
        ChatReadPart, ChatReadSource, ChatRequest, ChatSendTarget, ChatSummary,
    },
    ErrorCode,
};
use rusqlite::{params, OptionalExtension};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use tauri::Manager;

const MAX_PART_BYTES: usize = 4 * 1024 * 1024;

pub(super) fn with_store<T>(
    app: &tauri::AppHandle,
    read: impl FnOnce(&Store) -> Result<T, ErrorCode>,
) -> Result<T, ErrorCode> {
    with_store_wait(app, Duration::ZERO, read)
}

fn with_store_wait<T>(
    app: &tauri::AppHandle,
    wait: Duration,
    read: impl FnOnce(&Store) -> Result<T, ErrorCode>,
) -> Result<T, ErrorCode> {
    // A broker read must not initialize/migrate history or recover credentials.
    let chats = app.state::<Chats>();
    let backend = chats
        .0
        .try_lock()
        .map_err(|_| ErrorCode::TargetBusy)?
        .clone()
        .ok_or(ErrorCode::UiNotReady)?;
    let deadline = Instant::now() + wait;
    let services = loop {
        match backend.services.try_lock() {
            Ok(services) => break services,
            Err(std::sync::TryLockError::Poisoned(_)) => return Err(ErrorCode::StorageUnavailable),
            Err(std::sync::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return Err(ErrorCode::TargetBusy),
        }
    };
    read(
        services
            .store
            .as_ref()
            .map_err(|_| ErrorCode::StorageUnavailable)?,
    )
}

pub(super) fn summary(store: &Store, project: &str, id: &str) -> Result<ChatSummary, ErrorCode> {
    store.connection.query_row(
        "SELECT id,title,revision,active_leaf,updated_at FROM conversations WHERE id=?1 AND json_extract(origin,'$.projectId')=?2 AND length(CAST(title AS BLOB))<=256 AND length(CAST(origin AS BLOB))<=4096",
        params![id, project],
        |row| Ok(ChatSummary {
            conversation_id: row.get(0)?,
            title: row.get(1)?,
            conversation_revision: row.get::<_,i64>(2)?.to_string(),
            latest_message_id: row.get(3)?,
            updated_at_millis: row.get::<_,i64>(4)?.to_string(),
        }),
    ).optional().map_err(|_|ErrorCode::StorageUnavailable)?.ok_or(ErrorCode::TargetNotFound)
}

pub(crate) fn list(
    app: &tauri::AppHandle,
    project: &str,
    selection: ChatConversationSelection,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<Vec<ChatSummary>, ErrorCode> {
    check()?;
    with_store(app, |store| list_store(store, project, selection, check))
}

fn list_store(
    store: &Store,
    project: &str,
    selection: ChatConversationSelection,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<Vec<ChatSummary>, ErrorCode> {
    check()?;
    let ids = match selection {
        ChatConversationSelection::Exact(ids) => {
            if ids.len() > 64 || ids.iter().any(|id| !valid_chat_id(id)) {
                return Err(ErrorCode::ResourceExhausted);
            }
            ids
        }
        ChatConversationSelection::All => {
            let mut query = store
                .connection
                .prepare(
                    "SELECT id FROM conversations WHERE length(CAST(id AS BLOB))<=100 AND length(CAST(origin AS BLOB))<=4096 AND json_extract(origin,'$.projectId')=?1 ORDER BY updated_at DESC,id LIMIT 64",
                )
                .map_err(|_| ErrorCode::StorageUnavailable)?;
            let rows = query
                .query_map(params![project], |row| row.get::<_, String>(0))
                .map_err(|_| ErrorCode::StorageUnavailable)?;
            let mut ids = Vec::new();
            for row in rows {
                check()?;
                let id = row.map_err(|_| ErrorCode::StorageUnavailable)?;
                if valid_chat_id(&id) {
                    ids.push(id);
                }
            }
            ids
        }
    };
    let mut result = Vec::new();
    for id in &ids {
        check()?;
        match summary(store, project, id) {
            Ok(value) => result.push(value),
            Err(ErrorCode::TargetNotFound) => {}
            Err(error) => return Err(error),
        }
    }
    result.sort_by(|a, b| a.conversation_id.cmp(&b.conversation_id));
    check()?;
    Ok(result)
}

pub(crate) fn read(
    app: &tauri::AppHandle,
    project: &str,
    input: &ChatReadInput,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<ChatRead, ErrorCode> {
    check()?;
    with_store(app, |store| read_store(store, project, input, check))
}

pub(crate) fn open(
    app: &tauri::AppHandle,
    project: &str,
    command: &ChatOpenCommand,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<ChatSummary, ErrorCode> {
    check()?;
    if !valid_chat_id(&command.conversation_id) {
        return Err(ErrorCode::ResourceExhausted);
    }
    if !command.create {
        return with_store(app, |store| {
            let value = summary(store, project, &command.conversation_id)?;
            check()?;
            Ok(value)
        });
    }
    let backend = app
        .state::<Chats>()
        .backend(app)
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let mut services = backend
        .services
        .try_lock()
        .map_err(|_| ErrorCode::TargetBusy)?;
    let defaults = services
        .settings
        .as_ref()
        .map_err(|_| ErrorCode::StorageUnavailable)?
        .data
        .defaults
        .clone();
    let store = services
        .store
        .as_mut()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    check()?;
    store
        .create(
            &command.conversation_id,
            &super::store::Origin {
                project_id: project.into(),
                project_name: command.project_name.clone(),
                workspace_id: command.workspace_id.clone(),
                workspace_name: command.workspace_name.clone(),
            },
            &defaults,
        )
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    check()?;
    summary(store, project, &command.conversation_id)
}

pub(crate) fn draft(
    app: &tauri::AppHandle,
    project: &str,
    input: &ChatDraftInput,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<ChatDraftUpdated, ErrorCode> {
    check()?;
    let backend = app
        .state::<Chats>()
        .0
        .try_lock()
        .map_err(|_| ErrorCode::TargetBusy)?
        .clone()
        .ok_or(ErrorCode::UiNotReady)?;
    let mut services = backend
        .services
        .try_lock()
        .map_err(|_| ErrorCode::TargetBusy)?;
    let store = services
        .store
        .as_mut()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    draft_store(store, project, input, check)
}

fn draft_store(
    store: &mut Store,
    project: &str,
    input: &ChatDraftInput,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<ChatDraftUpdated, ErrorCode> {
    if !valid_chat_id(&input.conversation_id) || input.text.len() > 32 * 1024 {
        return Err(ErrorCode::ResourceExhausted);
    }
    let expected = input
        .expected_draft_revision
        .parse::<i64>()
        .ok()
        .filter(|n| (0..9_007_199_254_740_991).contains(n))
        .ok_or(ErrorCode::ResourceExhausted)?;
    check()?;
    // Settings, human drafts and sends share this services lock. Both revisions
    // are checked before using the existing store's atomic draft CAS.
    let conversation = summary(store, project, &input.conversation_id)?;
    if conversation.conversation_revision != input.expected_conversation_revision
        || store
            .draft(&input.conversation_id)
            .map_err(|_| ErrorCode::StorageUnavailable)?
            .revision
            != expected
    {
        return Err(ErrorCode::RevisionConflict);
    }
    check()?;
    let draft = store
        .save_draft(&input.conversation_id, &input.text, expected)
        .map_err(|e| {
            if e.starts_with("conflict:") {
                ErrorCode::RevisionConflict
            } else {
                ErrorCode::StorageUnavailable
            }
        })?;
    check().map_err(|_| ErrorCode::OutcomeUnknown)?;
    Ok(ChatDraftUpdated {
        workspace_id: input.workspace_id.clone(),
        panel_id: input.panel_id.clone(),
        conversation_id: input.conversation_id.clone(),
        draft_revision: draft.revision.to_string(),
        conversation_revision: conversation.conversation_revision,
        text_sha256: format!("{:x}", Sha256::digest(input.text.as_bytes())),
        total_utf16: input.text.encode_utf16().count() as u32,
    })
}

pub(crate) fn catalog(
    app: &tauri::AppHandle,
    project: &str,
    after: Option<&str>,
) -> Result<Value, ErrorCode> {
    // The Settings-only picker is an explicit human history action, like opening
    // Chat History. MCP list/read never take this initialization path.
    app.state::<Chats>()
        .backend(app)
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    with_store_wait(app, Duration::from_millis(500), |store| {
        let mut query = store.connection.prepare(
            "SELECT id FROM conversations WHERE id>?1 AND length(CAST(id AS BLOB))<=100 AND length(CAST(origin AS BLOB))<=4096 AND json_extract(origin,'$.projectId')=?2 ORDER BY id LIMIT 51"
        ).map_err(|_| ErrorCode::StorageUnavailable)?;
        let ids: Vec<String> = query
            .query_map(params![after.unwrap_or(""), project], |r| r.get(0))
            .map_err(|_| ErrorCode::StorageUnavailable)?
            .collect::<Result<_, _>>()
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        let next = if ids.len() > 50 {
            ids.get(49).cloned()
        } else {
            None
        };
        let mut items = Vec::new();
        for id in ids.iter().take(50) {
            if !valid_chat_id(id) {
                return Err(ErrorCode::UnsupportedCapability);
            }
            items.push(summary(store, project, id)?);
        }
        Ok(serde_json::json!({"items":items,"next":next}))
    })
}

pub(super) struct Part {
    pub(super) selector: ChatReadPart,
    pub(super) raw: String,
    pub(super) content: String,
    pub(super) draft_revision: Option<String>,
    pub(super) parent: Option<String>,
    pub(super) role: String,
    pub(super) status: String,
    pub(super) attachments: bool,
    pub(super) non_text: bool,
}

pub(super) fn part(
    store: &Store,
    conversation: &ChatSummary,
    selector: &ChatReadPart,
) -> Result<Part, ErrorCode> {
    match selector {
        ChatReadPart::Draft => {
            let (text, revision, attachments): (Option<String>, i64, bool) = store.connection.query_row(
                "SELECT CASE WHEN length(CAST(text AS BLOB))<=131072 THEN text END,revision,EXISTS(SELECT 1 FROM draft_attachments WHERE conversation_id=?1) FROM drafts WHERE conversation_id=?1",
                [&conversation.conversation_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
            ).optional().map_err(|_|ErrorCode::StorageUnavailable)?.ok_or(ErrorCode::TargetNotFound)?;
            let text = text.ok_or(ErrorCode::ResourceExhausted)?;
            Ok(Part {
                selector: ChatReadPart::Draft,
                raw: text.clone(),
                content: text,
                draft_revision: Some(revision.to_string()),
                parent: None,
                role: "user".into(),
                status: "draft".into(),
                attachments,
                non_text: false,
            })
        }
        ChatReadPart::Message { message_id } => {
            let id = message_id
                .as_ref()
                .or(conversation.latest_message_id.as_ref())
                .ok_or(ErrorCode::TargetNotFound)?;
            if !valid_chat_id(id) {
                return Err(ErrorCode::ResourceExhausted);
            }
            let (parent, role, status, raw, version, attachments): (Option<String>, String, String, Option<String>, u32, bool) = store.connection.query_row(
                "SELECT parent_id,role,status,CASE WHEN length(CAST(parts AS BLOB))<=4194304 THEN parts END,parts_version,EXISTS(SELECT 1 FROM message_attachments WHERE message_id=?2) FROM messages WHERE conversation_id=?1 AND id=?2",
                params![conversation.conversation_id,id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
            ).optional().map_err(|_|ErrorCode::StorageUnavailable)?.ok_or(ErrorCode::TargetNotFound)?;
            if version != 1 || !matches!(role.as_str(), "user" | "assistant") || status.len() > 64 {
                return Err(ErrorCode::UnsupportedCapability);
            }
            let raw = raw.ok_or(ErrorCode::ResourceExhausted)?;
            let value: Value =
                serde_json::from_str(&raw).map_err(|_| ErrorCode::UnsupportedCapability)?;
            let parts = value
                .as_array()
                .filter(|parts| parts.len() <= 4096)
                .ok_or(ErrorCode::ResourceExhausted)?;
            let mut content = String::new();
            let mut non_text = false;
            for part in parts {
                if part["type"] != "text" {
                    non_text = true;
                    continue;
                }
                let text = part["text"]
                    .as_str()
                    .ok_or(ErrorCode::UnsupportedCapability)?;
                if content.len().saturating_add(text.len()) > MAX_PART_BYTES {
                    return Err(ErrorCode::ResourceExhausted);
                }
                content.push_str(text);
            }
            Ok(Part {
                selector: ChatReadPart::Message {
                    message_id: Some(id.clone()),
                },
                raw,
                content,
                draft_revision: None,
                parent,
                role,
                status,
                attachments,
                non_text,
            })
        }
    }
}

fn read_store(
    store: &Store,
    project: &str,
    input: &ChatReadInput,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<ChatRead, ErrorCode> {
    if !valid_chat_id(&input.conversation_id)
        || !(2..=8192).contains(&input.max_chars)
        || input.start_utf16 > MAX_PART_BYTES as u32
        || (input.start_utf16 > 0 && input.expected_revision.is_none())
        || input
            .expected_revision
            .as_ref()
            .is_some_and(|r| r.len() != 64 || !r.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(ErrorCode::ResourceExhausted);
    }
    check()?;
    let conversation = summary(store, project, &input.conversation_id)?;
    let part = part(store, &conversation, &input.part)?;
    let request = request(store, &input.conversation_id, None)?;
    let send_target = if input.include_send_target {
        let (connection, model, max_output): (Option<String>, String, u32) = store.connection.query_row(
            "SELECT json_extract(config,'$.connectionId'),json_extract(config,'$.model'),json_extract(config,'$.maxOutputTokens') FROM conversations WHERE id=?1 AND length(CAST(config AS BLOB))<=1048576",
            [&input.conversation_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).map_err(|_| ErrorCode::StorageUnavailable)?;
        connection
            .filter(|_| !model.is_empty())
            .map(|connection_id| ChatSendTarget {
                connection_id,
                model,
                max_output_tokens: max_output,
            })
    } else {
        None
    };
    let metadata = serde_json::to_vec(&(
        &conversation,
        &send_target,
        &request,
        &part.selector,
        &part.draft_revision,
        &part.parent,
        &part.role,
        &part.status,
        part.attachments,
        part.non_text,
    ))
    .map_err(|_| ErrorCode::ResourceExhausted)?;
    let mut hash = Sha256::new();
    hash.update((metadata.len() as u64).to_le_bytes());
    hash.update(metadata);
    hash.update(part.raw.as_bytes());
    let revision = format!("{:x}", hash.finalize());
    if input
        .expected_revision
        .as_ref()
        .is_some_and(|r| r != &revision)
    {
        return Err(ErrorCode::RevisionConflict);
    }
    let mut units = 0;
    let mut start = None;
    let mut end = (0, 0);
    let limit = input.start_utf16 + u32::from(input.max_chars);
    for (count, (index, ch)) in part.content.char_indices().enumerate() {
        if count % 4096 == 0 {
            check()?;
        }
        if units == input.start_utf16 {
            start = Some(index);
        }
        if units <= limit {
            end = (index, units);
        }
        units += ch.len_utf16() as u32;
    }
    if units == input.start_utf16 {
        start = Some(part.content.len());
    }
    if units <= limit {
        end = (part.content.len(), units);
    }
    let start = start.ok_or(ErrorCode::ResourceExhausted)?;
    check()?;
    Ok(ChatRead {
        workspace_id: input.workspace_id.clone(),
        conversation,
        source: ChatReadSource::PersistedCheckpoint,
        part: part.selector,
        revision,
        draft_revision: part.draft_revision,
        parent_message_id: part.parent,
        role: part.role,
        status: part.status,
        content: part.content[start..end.0].into(),
        start_utf16: input.start_utf16,
        total_utf16: units,
        next_utf16: (end.1 < units).then_some(end.1),
        attachments_omitted: part.attachments,
        non_text_parts_omitted: part.non_text,
        send_target,
        request,
    })
}

// Read only identity and terminal status; provider settings and request fingerprints stay private.
pub(super) fn request(
    store: &Store,
    conversation: &str,
    request: Option<&str>,
) -> Result<Option<ChatRequest>, ErrorCode> {
    store.connection.query_row("SELECT id,assistant_id,status FROM requests WHERE conversation_id=?1 AND (?2 IS NULL OR id=?2) ORDER BY rowid DESC LIMIT 1", params![conversation,request], |r| Ok(ChatRequest { request_id:r.get(0)?, assistant_id:r.get(1)?, status:r.get(2)? })).optional().map_err(|_| ErrorCode::StorageUnavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::store::{Config, Origin};
    fn input() -> ChatReadInput {
        ChatReadInput {
            workspace_id: "workspace".into(),
            conversation_id: "conversation".into(),
            part: ChatReadPart::Draft,
            start_utf16: 0,
            max_chars: 3,
            expected_revision: None,
            include_send_target: false,
        }
    }
    #[test]
    fn all_yolo_conversations_are_project_scoped_and_capped() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(temp.path()).unwrap();
        let config = Config::default();
        for (project, count) in [("current", 70), ("other", 2)] {
            for index in 0..count {
                let origin = Origin {
                    project_id: project.into(),
                    project_name: project.into(),
                    workspace_id: format!("workspace-{project}"),
                    workspace_name: project.into(),
                };
                store
                    .create(
                        &format!("conversation-{project}-{index:03}"),
                        &origin,
                        &config,
                    )
                    .unwrap();
            }
        }
        let items = list_store(
            &store,
            "current",
            ChatConversationSelection::All,
            &|| Ok(()),
        )
        .unwrap();
        assert_eq!(items.len(), 64);
        assert!(items
            .iter()
            .all(|item| item.conversation_id.starts_with("conversation-current-")));

        let denied = list_store(&store, "current", ChatConversationSelection::All, &|| {
            Err(ErrorCode::ControlRevoked)
        });
        assert_eq!(denied.unwrap_err(), ErrorCode::ControlRevoked);
    }

    #[test]
    fn drafts_check_project_and_both_revisions_before_preserving_the_existing_store() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path()).unwrap();
        store
            .create(
                "conversation",
                &Origin {
                    project_id: "project".into(),
                    project_name: "Project".into(),
                    workspace_id: "workspace".into(),
                    workspace_name: "Workspace".into(),
                },
                &Config::default(),
            )
            .unwrap();
        let mut input = ChatDraftInput {
            workspace_id: "workspace".into(),
            panel_id: "panel".into(),
            conversation_id: "conversation".into(),
            text: "Agent 日本語 🙂".into(),
            expected_draft_revision: "0".into(),
            expected_conversation_revision: "0".into(),
            expected_revision: "1".into(),
            retry_epoch: "epoch".into(),
            request_key: "draft".into(),
        };
        let changes = store.connection.total_changes();
        assert_eq!(
            draft_store(&mut store, "foreign", &input, &|| Ok(())).unwrap_err(),
            ErrorCode::TargetNotFound
        );
        input.expected_conversation_revision = "99".into();
        assert_eq!(
            draft_store(&mut store, "project", &input, &|| Ok(())).unwrap_err(),
            ErrorCode::RevisionConflict
        );
        input.expected_conversation_revision = "0".into();
        assert_eq!(
            draft_store(&mut store, "project", &input, &|| Err(
                ErrorCode::ControlRevoked
            ))
            .unwrap_err(),
            ErrorCode::ControlRevoked
        );
        assert_eq!(store.connection.total_changes(), changes);
        let result = draft_store(&mut store, "project", &input, &|| Ok(())).unwrap();
        assert_eq!(result.draft_revision, "1");
        assert_eq!(store.draft("conversation").unwrap().text, input.text);
        assert_eq!(
            draft_store(&mut store, "project", &input, &|| Ok(())).unwrap_err(),
            ErrorCode::RevisionConflict
        );
        assert_eq!(store.draft("conversation").unwrap().revision, 1);
        assert_eq!(
            store
                .connection
                .query_row("SELECT COUNT(*) FROM requests", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn reads_only_the_selected_project_and_preserves_unicode_revision_and_private_fields() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path()).unwrap();
        let origin = Origin {
            project_id: "project".into(),
            project_name: "Project".into(),
            workspace_id: "workspace".into(),
            workspace_name: "Workspace".into(),
        };
        store
            .create(
                "conversation",
                &origin,
                &Config {
                    system: "PRIVATE_SYSTEM".into(),
                    connection_id: Some("PRIVATE_CONNECTION".into()),
                    ..Config::default()
                },
            )
            .unwrap();
        store.save_draft("conversation", "A🙂B", 0).unwrap();
        let mut request = input();
        let first = read_store(&store, "project", &request, &|| Ok(())).unwrap();
        assert_eq!(first.content, "A🙂");
        assert_eq!(first.next_utf16, Some(3));
        assert_eq!(first.total_utf16, 4);
        assert!(!serde_json::to_string(&first).unwrap().contains("PRIVATE_"));
        assert!(matches!(
            read_store(&store, "foreign", &request, &|| Ok(())),
            Err(ErrorCode::TargetNotFound)
        ));
        request.expected_revision = Some(first.revision);
        request.start_utf16 = 2;
        assert!(matches!(
            read_store(&store, "project", &request, &|| Ok(())),
            Err(ErrorCode::ResourceExhausted)
        ));
        request.start_utf16 = 3;
        assert_eq!(
            read_store(&store, "project", &request, &|| Ok(()))
                .unwrap()
                .content,
            "B"
        );
        store.save_draft("conversation", "changed", 1).unwrap();
        assert!(matches!(
            read_store(&store, "project", &request, &|| Ok(())),
            Err(ErrorCode::RevisionConflict)
        ));
    }
    #[test]
    fn message_reads_omit_non_text_and_never_accept_a_foreign_message_id() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path()).unwrap();
        let origin = Origin {
            project_id: "project".into(),
            project_name: "Project".into(),
            workspace_id: "workspace".into(),
            workspace_name: "Workspace".into(),
        };
        for id in ["conversation", "foreign"] {
            store.create(id, &origin, &Config::default()).unwrap();
        }
        store.connection.execute("INSERT INTO messages(id,conversation_id,role,parts,status,metadata) VALUES ('message','conversation','assistant',?1,'active',?2)",params![r#"[{"type":"text","text":"answer"},{"type":"reasoning","text":"PRIVATE_REASONING"}]"#,r#"{"credentialRevision":"PRIVATE_CREDENTIAL"}"#]).unwrap();
        store.connection.execute("INSERT INTO messages(id,conversation_id,role,parts,status) VALUES ('foreign-message','foreign','user','[]','completed')",[]).unwrap();
        let mut request = input();
        request.max_chars = 8192;
        request.part = ChatReadPart::Message {
            message_id: Some("message".into()),
        };
        let result = read_store(&store, "project", &request, &|| Ok(())).unwrap();
        assert_eq!(result.content, "answer");
        assert!(result.non_text_parts_omitted);
        assert!(!serde_json::to_string(&result).unwrap().contains("PRIVATE_"));
        request.part = ChatReadPart::Message {
            message_id: Some("foreign-message".into()),
        };
        assert!(matches!(
            read_store(&store, "project", &request, &|| Ok(())),
            Err(ErrorCode::TargetNotFound)
        ));
        assert!(matches!(
            read_store(&store, "project", &input(), &|| Err(
                ErrorCode::ControlRevoked
            )),
            Err(ErrorCode::ControlRevoked)
        ));
    }
}
