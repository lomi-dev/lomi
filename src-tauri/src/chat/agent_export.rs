use super::{agent, store::Store};
use lomi_control_protocol::{
    chat::{ChatExport, ChatExportFormat, ChatExportInput, ChatReadPart, ChatSummary},
    ErrorCode,
};
use rusqlite::params;
use serde::Serialize;
use sha2::{Digest, Sha256};

const MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Message {
    id: String,
    parent_id: Option<String>,
    role: String,
    status: String,
    text: String,
    attachments_omitted: bool,
    non_text_parts_omitted: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Document {
    version: u8,
    source: &'static str,
    coverage: &'static str,
    omitted: [&'static str; 4],
    conversation: ChatSummary,
    messages: Vec<Message>,
    draft_text: String,
    draft_revision: String,
    draft_attachments_omitted: bool,
}

pub(crate) fn export(
    app: &tauri::AppHandle,
    project: &str,
    input: &ChatExportInput,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<ChatExport, ErrorCode> {
    check()?;
    agent::with_store(app, |store| export_store(store, project, input, check))
}

fn export_store(
    store: &Store,
    project: &str,
    input: &ChatExportInput,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<ChatExport, ErrorCode> {
    check()?;
    let conversation = agent::summary(store, project, &input.conversation_id)?;
    let ids = {
        let mut query = store.connection.prepare("SELECT CASE WHEN length(CAST(id AS BLOB))<=100 THEN id END FROM messages WHERE conversation_id=?1 ORDER BY rowid LIMIT 513").map_err(|_| ErrorCode::StorageUnavailable)?;
        let ids = query
            .query_map(params![input.conversation_id], |r| r.get::<_, String>(0))
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        ids.collect::<Result<Vec<_>, _>>()
            .map_err(|_| ErrorCode::StorageUnavailable)?
    };
    if ids.len() > 512 {
        return Err(ErrorCode::ResourceExhausted);
    }
    let draft = agent::part(store, &conversation, &ChatReadPart::Draft)?;
    let mut size = draft.raw.len();
    let mut messages = Vec::with_capacity(ids.len());
    let mut attachments_omitted = draft.attachments;
    let mut non_text_parts_omitted = false;
    for id in ids {
        check()?;
        let part = agent::part(
            store,
            &conversation,
            &ChatReadPart::Message {
                message_id: Some(id.clone()),
            },
        )?;
        size = size.saturating_add(part.raw.len());
        if size > MAX_BYTES {
            return Err(ErrorCode::ResourceExhausted);
        }
        attachments_omitted |= part.attachments;
        non_text_parts_omitted |= part.non_text;
        messages.push(Message {
            id,
            parent_id: part.parent,
            role: part.role,
            status: part.status,
            text: part.content,
            attachments_omitted: part.attachments,
            non_text_parts_omitted: part.non_text,
        });
    }
    let count = messages.len() as u16;
    let document = Document {
        version: 1,
        source: "persisted_checkpoint",
        coverage: "all_saved_message_variants_and_draft",
        omitted: [
            "system_instructions",
            "provider_metadata",
            "non_text_parts",
            "attachments",
        ],
        conversation,
        messages,
        draft_text: draft.content,
        draft_revision: draft.draft_revision.ok_or(ErrorCode::OutcomeUnknown)?,
        draft_attachments_omitted: draft.attachments,
    };
    let text = match input.format {
        ChatExportFormat::Json => {
            struct Bounded(Vec<u8>);
            impl std::io::Write for Bounded {
                fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                    if bytes.len() > MAX_BYTES.saturating_sub(self.0.len()) {
                        return Err(std::io::Error::other("Export limit"));
                    }
                    self.0.extend_from_slice(bytes);
                    Ok(bytes.len())
                }
                fn flush(&mut self) -> std::io::Result<()> {
                    Ok(())
                }
            }
            let mut bytes = Bounded(Vec::new());
            serde_json::to_writer_pretty(&mut bytes, &document)
                .map_err(|_| ErrorCode::ResourceExhausted)?;
            String::from_utf8(bytes.0).map_err(|_| ErrorCode::OutcomeUnknown)?
        }
        ChatExportFormat::Markdown => {
            let mut text = format!("# {}\n\nSaved text from all message variants and the persisted draft. System instructions, provider metadata, non-text parts and attachments are omitted.\n\n", document.conversation.title);
            for message in &document.messages {
                check()?;
                text.push_str(&format!(
                    "## {} · {}\n\nMessage: {} · Parent: {}\n\n{}\n\n",
                    message.role,
                    message.status,
                    message.id,
                    message.parent_id.as_deref().unwrap_or("none"),
                    message.text
                ));
                if text.len() > MAX_BYTES {
                    return Err(ErrorCode::ResourceExhausted);
                }
            }
            text.push_str(&format!("## Saved draft\n\n{}\n", document.draft_text));
            if text.len() > MAX_BYTES {
                return Err(ErrorCode::ResourceExhausted);
            }
            text
        }
    };
    check()?;
    let revision = format!("{:x}", Sha256::digest(text.as_bytes()));
    if input
        .expected_revision
        .as_ref()
        .is_some_and(|r| r != &revision)
    {
        return Err(ErrorCode::RevisionConflict);
    }
    let (content, total_utf16, next_utf16) = page(&text, input.start_utf16, input.max_chars)?;
    check()?;
    Ok(ChatExport {
        workspace_id: input.workspace_id.clone(),
        conversation_id: input.conversation_id.clone(),
        format: input.format.clone(),
        revision,
        total_bytes: text.len() as u32,
        message_count: count,
        content,
        start_utf16: input.start_utf16,
        total_utf16,
        next_utf16,
        attachments_omitted,
        non_text_parts_omitted,
    })
}

fn page(text: &str, offset: u32, max: u16) -> Result<(String, u32, Option<u32>), ErrorCode> {
    if !(2..=8192).contains(&max) {
        return Err(ErrorCode::ResourceExhausted);
    }
    let total = text.encode_utf16().count() as u32;
    let mut units = 0;
    let mut start = None;
    let mut end = (text.len(), total);
    for (byte, c) in text.char_indices() {
        if units == offset {
            start = Some(byte);
        }
        if units <= offset.saturating_add(u32::from(max)) {
            end = (byte, units);
        }
        units += c.len_utf16() as u32;
        if units > offset.saturating_add(u32::from(max)) {
            break;
        }
    }
    if offset == total {
        start = Some(text.len());
    }
    if total <= offset.saturating_add(u32::from(max)) {
        end = (text.len(), total);
    }
    let start = start.ok_or(ErrorCode::ResourceExhausted)?;
    Ok((
        text[start..end.0].to_owned(),
        total,
        (end.1 < total).then_some(end.1),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::store::{Config, Origin};
    #[test]
    fn export_covers_variants_without_private_fields_and_rejects_changed_or_split_pages() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path()).unwrap();
        let config = Config {
            system: "PRIVATE_SYSTEM".into(),
            connection_id: Some("PRIVATE_CONNECTION".into()),
            ..Default::default()
        };
        store
            .create(
                "c",
                &Origin {
                    project_id: "p".into(),
                    project_name: "P".into(),
                    workspace_id: "w".into(),
                    workspace_name: "W".into(),
                },
                &config,
            )
            .unwrap();
        store.save_draft("c", "Draft 日本語 🙂", 0).unwrap();
        for id in ["first", "branch"] {
            store.connection.execute("INSERT INTO messages(id,conversation_id,role,parts,status,metadata) VALUES(?1,'c','assistant',?2,'completed',?3)", params![id, r#"[{"type":"text","text":"Answer 🙂"},{"type":"reasoning","text":"PRIVATE_REASONING"}]"#, r#"{"key":"PRIVATE_KEY"}"#]).unwrap();
        }
        store
            .connection
            .execute(
                "UPDATE conversations SET active_leaf='first' WHERE id='c'",
                [],
            )
            .unwrap();
        let mut input = ChatExportInput {
            workspace_id: "w".into(),
            conversation_id: "c".into(),
            format: ChatExportFormat::Json,
            start_utf16: 0,
            max_chars: 8192,
            expected_revision: None,
        };
        let before = store.connection.total_changes();
        let whole = export_store(&store, "p", &input, &|| Ok(())).unwrap();
        assert_eq!(whole.message_count, 2);
        assert!(whole.non_text_parts_omitted);
        assert!(!whole.content.contains("PRIVATE_"));
        let value: serde_json::Value = serde_json::from_str(&whole.content).unwrap();
        assert_eq!(value["messages"][1]["id"], "branch");
        assert_eq!(value["draftText"], "Draft 日本語 🙂");
        assert_eq!(store.connection.total_changes(), before);
        assert_eq!(
            export_store(&store, "foreign", &input, &|| Ok(())).unwrap_err(),
            ErrorCode::TargetNotFound
        );
        input.max_chars = 41;
        input.expected_revision = Some(whole.revision.clone());
        let mut combined = String::new();
        loop {
            let part = export_store(&store, "p", &input, &|| Ok(())).unwrap();
            combined.push_str(&part.content);
            let Some(next) = part.next_utf16 else {
                break;
            };
            input.start_utf16 = next;
        }
        assert_eq!(combined, whole.content);
        assert!(page("A🙂B", 2, 2).is_err());
        assert_eq!(page("A🙂B", 1, 2).unwrap(), ("🙂".into(), 4, Some(3)));
        store.save_draft("c", "Changed", 1).unwrap();
        assert_eq!(
            export_store(&store, "p", &input, &|| Ok(())).unwrap_err(),
            ErrorCode::RevisionConflict
        );
        input.expected_revision = None;
        input.start_utf16 = 0;
        input.max_chars = 8192;
        input.format = ChatExportFormat::Markdown;
        let markdown = export_store(&store, "p", &input, &|| Ok(())).unwrap();
        assert!(markdown.content.contains("Message: branch"));
        assert!(!markdown.content.contains("PRIVATE_"));
        assert_eq!(
            export_store(&store, "p", &input, &|| Err(ErrorCode::ControlRevoked)).unwrap_err(),
            ErrorCode::ControlRevoked
        );
        store.connection.execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<511) INSERT INTO messages(id,conversation_id,role,parts,status) SELECT 'limit-'||x,'c','user','[]','completed' FROM n").unwrap();
        assert_eq!(
            export_store(&store, "p", &input, &|| Ok(())).unwrap_err(),
            ErrorCode::ResourceExhausted
        );
    }
}
