use super::{process::valid_id, storage};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_TEXT: usize = 128 * 1024;
const MAX_CONTEXT: usize = 40 * 1024 * 1024;
fn db<T>(result: rusqlite::Result<T>) -> Result<T, String> {
    result.map_err(|_| {
        "storage: Chat history could not be saved or read. Your existing data was preserved.".into()
    })
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
fn check_id(id: &str) -> Result<(), String> {
    if valid_id(id) {
        Ok(())
    } else {
        Err("Invalid chat ID.".into())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Origin {
    pub project_id: String,
    pub project_name: String,
    pub workspace_id: String,
    pub workspace_name: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub connection_id: Option<String>,
    pub model: String,
    pub system: String,
    pub max_output_tokens: u32,
    pub temperature: Option<f64>,
    pub configured: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            connection_id: None,
            model: String::new(),
            system: String::new(),
            max_output_tokens: 4096,
            temperature: None,
            configured: false,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .connection_id
            .as_deref()
            .is_some_and(|id| !valid_id(id))
            || self.model.len() > 200
            || self.system.len() > MAX_TEXT
            || !(1..=32768).contains(&self.max_output_tokens)
            || self
                .temperature
                .is_some_and(|n| !n.is_finite() || !(0.0..=2.0).contains(&n))
        {
            return Err("Invalid conversation settings.".into());
        }
        if !self
            .model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':' | b'/'))
        {
            return Err("Invalid model ID.".into());
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub origin: Origin,
    pub revision: i64,
    pub active_leaf_id: Option<String>,
    pub config: Config,
    pub updated_at: i64,
    #[serde(default)]
    pub pinned: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub parent_id: Option<String>,
    pub role: String,
    pub parts_version: u32,
    pub parts: Value,
    pub status: String,
    pub metadata: Value,
    pub previous_variant: Option<String>,
    pub next_variant: Option<String>,
    pub attachments: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub text: String,
    pub revision: i64,
    pub attachments: Vec<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Loaded {
    pub conversation: Conversation,
    pub draft: Draft,
    pub messages: Vec<Message>,
    pub has_older: bool,
    pub request: Option<Value>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Start {
    pub request_id: String,
    pub conversation_id: String,
    pub assistant_id: String,
    pub user_id: String,
    pub expected_revision: i64,
    pub draft_revision: i64,
    pub action: String,
    pub target_id: Option<String>,
    pub text: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Accepted {
    pub request_id: String,
    pub assistant_id: String,
    pub user_id: String,
    pub draft_revision: i64,
    pub repeated: bool,
}

pub struct Store {
    pub connection: Connection,
}
impl Store {
    pub fn open(root: &Path) -> Result<Self, String> {
        let path = root.join("history.sqlite3");
        storage::reject_link(&path)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|_| "storage: Cannot open chat history.")?;
        storage::private(&path, false)?;
        drop(file);
        let connection = db(Connection::open(&path))?;
        db(connection.busy_timeout(Duration::from_secs(2)))?;
        let version: i64 = db(connection.query_row("PRAGMA user_version", [], |r| r.get(0)))?;
        if version > 2 {
            return Err(
                "The chat history was created by a newer version. It was preserved.".into(),
            );
        }
        if version == 0 {
            let count: i64 = db(connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",[],|r|r.get(0)))?;
            if count != 0 {
                return Err("The chat history has an unknown format. It was preserved.".into());
            }
        }
        db(connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA temp_store=MEMORY;"))?;
        let mode: String = db(connection.query_row("PRAGMA journal_mode", [], |r| r.get(0)))?;
        let sync: i64 = db(connection.query_row("PRAGMA synchronous", [], |r| r.get(0)))?;
        if mode != "wal" || sync != 2 {
            return Err("storage: Durable chat history is unavailable.".into());
        }
        if version == 0 {
            db(connection.execute_batch(include_str!("schema.sql")))?;
        }
        if version == 1 {
            db(connection.execute_batch("BEGIN IMMEDIATE; ALTER TABLE conversations ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0,1)); CREATE INDEX conversation_history_order ON conversations(pinned DESC,updated_at DESC,id); PRAGMA user_version=2; COMMIT;"))?;
        }
        db(connection.execute_batch("BEGIN IMMEDIATE; UPDATE requests SET status='interrupted' WHERE status='active'; UPDATE messages SET status='interrupted' WHERE status='active'; COMMIT;"))?;
        Ok(Self { connection })
    }
    pub fn create(
        &mut self,
        id: &str,
        origin: &Origin,
        config: &Config,
    ) -> Result<Conversation, String> {
        check_id(id)?;
        config.validate()?;
        if serde_json::to_vec(origin)
            .map_err(|_| "Invalid origin.")?
            .len()
            > 4096
        {
            return Err("Workspace names are too long.".into());
        }
        let tx = db(self.connection.transaction())?;
        db(tx.execute("INSERT INTO conversations(id,title,origin,config,updated_at) VALUES (?1,'Chat AI',?2,?3,?4)",params![id,serde_json::to_string(origin).unwrap(),serde_json::to_string(config).unwrap(),now()]))?;
        db(tx.execute("INSERT INTO drafts(conversation_id) VALUES (?1)", [id]))?;
        db(tx.execute(
            "INSERT INTO chat_search VALUES ('Chat AI',?1,'title')",
            [id],
        ))?;
        db(tx.commit())?;
        self.conversation(id)
    }
    pub fn conversation(&self, id: &str) -> Result<Conversation, String> {
        let values = db(self.connection.query_row("SELECT title,origin,config,revision,active_leaf,updated_at,pinned FROM conversations WHERE id=?1",[id],|r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional())?.ok_or("missing: This conversation is unavailable. Open history to choose another conversation.")?;
        Ok(Conversation {
            id: id.into(),
            title: values.0,
            origin: serde_json::from_str(&values.1).map_err(|_| "Invalid stored origin.")?,
            config: serde_json::from_str(&values.2)
                .map_err(|_| "Unknown stored conversation settings.")?,
            revision: values.3,
            active_leaf_id: values.4,
            updated_at: values.5,
            pinned: values.6,
        })
    }
    pub fn draft(&self, id: &str) -> Result<Draft, String> {
        let (text, revision) = db(self.connection.query_row(
            "SELECT text,revision FROM drafts WHERE conversation_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ))?;
        let mut query = db(self.connection.prepare(
            "SELECT attachment_id FROM draft_attachments WHERE conversation_id=?1 ORDER BY ordinal",
        ))?;
        let attachments = db(
            db(query.query_map([id], |r| r.get(0)))?.collect::<rusqlite::Result<Vec<String>>>()
        )?;
        Ok(Draft {
            text,
            revision,
            attachments,
        })
    }
    pub fn save_draft(&mut self, id: &str, text: &str, expected: i64) -> Result<Draft, String> {
        if text.len() > MAX_TEXT {
            return Err("The message exceeds 128 KiB of UTF-8 text.".into());
        }
        if db(self.connection.execute("UPDATE drafts SET text=?1,revision=revision+1 WHERE conversation_id=?2 AND revision=?3",params![text,id,expected]))? != 1 { return Err("conflict: The shared draft changed. Reload it before saving.".into()); }
        self.draft(id)
    }
    pub fn configure(
        &mut self,
        id: &str,
        config: &Config,
        expected: i64,
    ) -> Result<Conversation, String> {
        config.validate()?;
        self.idle(id)?;
        if db(self.connection.execute(
            "UPDATE conversations SET config=?1,revision=revision+1 WHERE id=?2 AND revision=?3",
            params![serde_json::to_string(config).unwrap(), id, expected],
        ))? != 1
        {
            return Err("conflict: Conversation settings changed.".into());
        }
        self.conversation(id)
    }
    pub fn idle(&self, id: &str) -> Result<(), String> {
        let active: bool = db(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM requests WHERE conversation_id=?1 AND status='active')",
            [id],
            |r| r.get(0),
        ))?;
        if active {
            Err("This conversation is already generating a response.".into())
        } else {
            Ok(())
        }
    }
    pub fn message(&self, conversation: &str, id: &str) -> Result<Message, String> {
        let (parent,role,parts,status,metadata,version): (Option<String>,String,String,String,String,u32) = db(self.connection.query_row("SELECT parent_id,role,parts,status,metadata,parts_version FROM messages WHERE conversation_id=?1 AND id=?2",params![conversation,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))))?;
        let neighbor = |direction: &str| -> Result<Option<String>, String> {
            let sql = if direction == "previous" {
                "SELECT id FROM messages WHERE conversation_id=?1 AND parent_id IS ?2 AND rowid<(SELECT rowid FROM messages WHERE id=?3) ORDER BY rowid DESC LIMIT 1"
            } else {
                "SELECT id FROM messages WHERE conversation_id=?1 AND parent_id IS ?2 AND rowid>(SELECT rowid FROM messages WHERE id=?3) ORDER BY rowid LIMIT 1"
            };
            db(self
                .connection
                .query_row(sql, params![conversation, parent, id], |r| r.get(0))
                .optional())
        };
        Ok(Message {
            id: id.into(),
            parent_id: parent.clone(),
            role,
            parts_version: version,
            parts: serde_json::from_str(&parts).map_err(|_| "Invalid stored message.")?,
            status,
            metadata: serde_json::from_str(&metadata).map_err(|_| "Invalid message metadata.")?,
            previous_variant: neighbor("previous")?,
            next_variant: neighbor("next")?,
            attachments: self.attachment_ids(id)?,
        })
    }
    pub fn path(
        &self,
        conversation: &str,
        leaf: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<(Vec<Message>, bool), String> {
        let mut id = leaf.map(str::to_owned);
        let mut messages = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut bytes = 0;
        let mut skipped = 0;
        while let Some(current) = id {
            if !visited.insert(current.clone()) || visited.len() > 100_000 {
                return Err("The conversation exceeds the supported context depth.".into());
            }
            let message = self.message(conversation, &current)?;
            id = message.parent_id.clone();
            if skipped < offset {
                skipped += 1;
                continue;
            }
            bytes += serde_json::to_vec(&message)
                .map_err(|_| "Invalid message.")?
                .len();
            if bytes > MAX_CONTEXT {
                return Err(
                    "The conversation exceeds the 40 MiB context limit. Start a new conversation."
                        .into(),
                );
            }
            messages.push(message);
            if messages.len() >= limit {
                messages.reverse();
                return Ok((messages, id.is_some()));
            }
        }
        messages.reverse();
        Ok((messages, false))
    }
    pub fn load(&self, id: &str, offset: usize) -> Result<Loaded, String> {
        let conversation = self.conversation(id)?;
        let (messages, has_older) = self.path(
            id,
            conversation.active_leaf_id.as_deref(),
            50,
            offset.min(100_000),
        )?;
        let request = db(self.connection.query_row("SELECT id,assistant_id,status FROM requests WHERE conversation_id=?1 ORDER BY rowid DESC LIMIT 1",[id],|r| Ok(json!({"id":r.get::<_,String>(0)?,"assistantId":r.get::<_,String>(1)?,"status":r.get::<_,String>(2)?}))).optional())?;
        Ok(Loaded {
            conversation,
            draft: self.draft(id)?,
            messages,
            has_older,
            request,
        })
    }
    pub fn replay(&self, input: &Start) -> Result<Option<Accepted>, String> {
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(input).map_err(|_| "Invalid request.")?)
        );
        let previous: Option<(String, String, String, i64)> = db(self
            .connection
            .query_row(
                "SELECT fingerprint,assistant_id,user_id,draft_revision FROM requests WHERE id=?1",
                [&input.request_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional())?;
        if let Some((hash, assistant_id, user_id, draft_revision)) = previous {
            if hash != fingerprint {
                return Err("conflict: The request ID belongs to a different message.".into());
            }
            return Ok(Some(Accepted {
                request_id: input.request_id.clone(),
                assistant_id,
                user_id,
                draft_revision,
                repeated: true,
            }));
        }
        Ok(None)
    }
    pub fn begin(&mut self, input: &Start, metadata: &Value) -> Result<Accepted, String> {
        for id in [
            &input.request_id,
            &input.conversation_id,
            &input.user_id,
            &input.assistant_id,
        ] {
            check_id(id)?;
        }
        if input.text.len() > MAX_TEXT {
            return Err("The message exceeds 128 KiB.".into());
        }
        if let Some(accepted) = self.replay(input)? {
            return Ok(accepted);
        }
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(input).map_err(|_| "Invalid request.")?)
        );
        self.idle(&input.conversation_id)?;
        let conversation = self.conversation(&input.conversation_id)?;
        if conversation.revision != input.expected_revision {
            return Err("conflict: The active conversation changed.".into());
        }
        let draft = self.draft(&input.conversation_id)?;
        let mut user_id = input.user_id.clone();
        let (parent, create_user) = match input.action.as_str() {
            "send" => {
                if draft.revision != input.draft_revision || draft.text != input.text {
                    return Err("conflict: The draft changed before Send.".into());
                }
                if input.text.trim().is_empty() && draft.attachments.is_empty() {
                    return Err("Write a message or attach a file.".into());
                }
                (conversation.active_leaf_id, true)
            }
            "edit" => {
                let target = self.message(
                    &input.conversation_id,
                    input.target_id.as_deref().ok_or("Missing edit target.")?,
                )?;
                if target.role != "user" || input.text.trim().is_empty() {
                    return Err("Select a user message to edit.".into());
                }
                (target.parent_id, true)
            }
            "retry" => {
                let target = self.message(
                    &input.conversation_id,
                    input.target_id.as_deref().ok_or("Missing retry target.")?,
                )?;
                user_id = if target.role == "user" {
                    target.id
                } else {
                    target.parent_id.ok_or("Missing user message.")?
                };
                if self.message(&input.conversation_id, &user_id)?.role != "user" {
                    return Err("Invalid retry target.".into());
                }
                (None, false)
            }
            _ => return Err("Unknown generation action.".into()),
        };
        let tx = db(self.connection.transaction())?;
        if create_user {
            db(tx.execute("INSERT INTO messages(id,conversation_id,parent_id,role,parts,status) VALUES (?1,?2,?3,'user',?4,'completed')",params![user_id,input.conversation_id,parent,json!([{ "type":"text","text":input.text }]).to_string()]))?;
            db(tx.execute(
                "INSERT INTO chat_search VALUES (?1,?2,?3)",
                params![input.text, input.conversation_id, user_id],
            ))?;
        }
        let mut draft_revision = draft.revision;
        if input.action == "send" {
            db(tx.execute("INSERT INTO message_attachments SELECT ?1,attachment_id,ordinal FROM draft_attachments WHERE conversation_id=?2",params![user_id,input.conversation_id]))?;
            db(tx.execute(
                "DELETE FROM draft_attachments WHERE conversation_id=?1",
                [&input.conversation_id],
            ))?;
            db(tx.execute("UPDATE drafts SET text='',revision=revision+1 WHERE conversation_id=?1 AND revision=?2",params![input.conversation_id,draft.revision]))?;
            draft_revision += 1;
        } else if input.action == "edit" {
            db(tx.execute("INSERT INTO message_attachments SELECT ?1,attachment_id,ordinal FROM message_attachments WHERE message_id=?2",params![user_id,input.target_id]))?;
        }
        db(tx.execute("INSERT INTO messages(id,conversation_id,parent_id,role,parts,status,metadata) VALUES (?1,?2,?3,'assistant','[]','active',?4)",params![input.assistant_id,input.conversation_id,user_id,metadata.to_string()]))?;
        db(tx.execute("INSERT INTO requests(id,conversation_id,user_id,assistant_id,fingerprint,status,draft_revision) VALUES (?1,?2,?3,?4,?5,'active',?6)",params![input.request_id,input.conversation_id,user_id,input.assistant_id,fingerprint,draft_revision]))?;
        db(tx.execute(
            "UPDATE conversations SET active_leaf=?1,revision=revision+1,updated_at=?2 WHERE id=?3",
            params![input.assistant_id, now(), input.conversation_id],
        ))?;
        if conversation.title == "Chat AI" && input.action == "send" {
            let title = input
                .text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(70)
                .collect::<String>();
            if !title.is_empty() {
                db(tx.execute(
                    "UPDATE conversations SET title=?1 WHERE id=?2",
                    params![title, input.conversation_id],
                ))?;
                db(tx.execute(
                    "UPDATE chat_search SET content=?1 WHERE conversation=?2 AND message='title'",
                    params![title, input.conversation_id],
                ))?;
            }
        }
        db(tx.commit())?;
        Ok(Accepted {
            request_id: input.request_id.clone(),
            assistant_id: input.assistant_id.clone(),
            user_id,
            draft_revision,
            repeated: false,
        })
    }
    pub fn checkpoint(
        &mut self,
        request: &str,
        sequence: i64,
        parts: &Value,
        status: &str,
        result: &Value,
    ) -> Result<(), String> {
        if !matches!(
            status,
            "active" | "completed" | "cancelled" | "failed" | "interrupted"
        ) || !parts.is_array()
            || parts.to_string().len() > 16 * 1024 * 1024
        {
            return Err("Invalid response checkpoint.".into());
        }
        let tx = db(self.connection.transaction())?;
        let changed=db(tx.execute("UPDATE requests SET sequence=?1,status=?2,result=?3 WHERE id=?4 AND status='active' AND sequence<?1",params![sequence,status,result.to_string(),request]))?;
        if changed != 1 {
            return Err("Stale response checkpoint.".into());
        }
        db(tx.execute("UPDATE messages SET parts=?1,status=?2 WHERE id=(SELECT assistant_id FROM requests WHERE id=?3)",params![parts.to_string(),status,request]))?;
        if status != "active" {
            db(tx.execute("UPDATE messages SET metadata=json_set(metadata,'$.result',json(?1)) WHERE id=(SELECT assistant_id FROM requests WHERE id=?2)", params![result.to_string(), request]))?;
            let text = parts
                .as_array()
                .unwrap()
                .iter()
                .filter(|p| p["type"] == "text")
                .filter_map(|p| p["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            db(tx.execute("INSERT INTO chat_search SELECT ?1,conversation_id,assistant_id FROM requests WHERE id=?2",params![text,request]))?;
            db(tx.execute("UPDATE conversations SET updated_at=?1 WHERE id=(SELECT conversation_id FROM requests WHERE id=?2)",params![now(),request]))?;
        }
        db(tx.commit())
    }
    pub fn select(
        &mut self,
        id: &str,
        target: &str,
        expected: i64,
    ) -> Result<Conversation, String> {
        self.idle(id)?;
        self.message(id, target)?;
        let mut leaf = target.to_owned();
        for _ in 0..2000 {
            let child:Option<String>=db(self.connection.query_row("SELECT id FROM messages WHERE conversation_id=?1 AND parent_id=?2 ORDER BY rowid DESC LIMIT 1",params![id,leaf],|r|r.get(0)).optional())?;
            if let Some(child) = child {
                leaf = child;
            } else {
                break;
            }
        }
        if db(self.connection.execute("UPDATE conversations SET active_leaf=?1,revision=revision+1 WHERE id=?2 AND revision=?3",params![leaf,id,expected]))?!=1 {return Err("conflict: The selected variant changed.".into());}
        self.conversation(id)
    }
}

impl Store {
    pub fn list(
        &self,
        query: &str,
        workspace: Option<&str>,
        project: Option<&str>,
        offset: usize,
    ) -> Result<Vec<Conversation>, String> {
        if query.len() > 1024 {
            return Err("Search text is too long.".into());
        }
        let search = if query.trim().is_empty() {
            None
        } else {
            Some(format!("\"{}\"", query.replace('"', "\"\"")))
        };
        let mut statement=db(self.connection.prepare("SELECT id FROM conversations WHERE (?1 IS NULL OR id IN (SELECT conversation FROM chat_search WHERE chat_search MATCH ?1)) AND (?2 IS NULL OR json_extract(origin,'$.workspaceId')=?2) AND (?3 IS NULL OR json_extract(origin,'$.projectId')=?3) ORDER BY pinned DESC,updated_at DESC,id LIMIT 50 OFFSET ?4"))?;
        let ids = db(db(statement.query_map(
            params![search, workspace, project, offset.min(1000000) as i64],
            |row| row.get(0),
        ))?
        .collect::<rusqlite::Result<Vec<String>>>())?;
        ids.iter().map(|id| self.conversation(id)).collect()
    }
    pub fn rename(&mut self, id: &str, title: &str) -> Result<Conversation, String> {
        if title.trim().is_empty() || title.len() > 256 {
            return Err("Conversation names must contain 1–256 bytes.".into());
        }
        let tx = db(self.connection.transaction())?;
        db(tx.execute(
            "UPDATE conversations SET title=?1,updated_at=?2 WHERE id=?3",
            params![title, now(), id],
        ))?;
        db(tx.execute(
            "UPDATE chat_search SET content=?1 WHERE conversation=?2 AND message='title'",
            params![title, id],
        ))?;
        db(tx.commit())?;
        self.conversation(id)
    }
    pub fn pin(&mut self, id: &str, pinned: bool) -> Result<Conversation, String> {
        db(self.connection.execute(
            "UPDATE conversations SET pinned=?1 WHERE id=?2",
            params![pinned, id],
        ))?;
        self.conversation(id)
    }
    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        self.idle(id)?;
        let tx = db(self.connection.transaction())?;
        db(tx.execute("DELETE FROM requests WHERE conversation_id=?1", [id]))?;
        db(tx.execute("DELETE FROM chat_search WHERE conversation=?1", [id]))?;
        db(tx.execute("DELETE FROM conversations WHERE id=?1", [id]))?;
        db(tx.commit())
    }
    pub fn interrupt_orphan(&mut self, request: &str) -> Result<(), String> {
        let tx = db(self.connection.transaction())?;
        db(tx.execute("UPDATE messages SET status='interrupted' WHERE id=(SELECT assistant_id FROM requests WHERE id=?1 AND status='active')",[request]))?;
        db(tx.execute(
            "UPDATE requests SET status='interrupted' WHERE id=?1 AND status='active'",
            [request],
        ))?;
        db(tx.commit())
    }
    pub fn attachment_meta(&self, id: &str) -> Result<Value, String> {
        db(self.connection.query_row("SELECT name,mime,size FROM attachments WHERE id=?1",[id],|row| Ok(json!({"id":id,"name":row.get::<_,String>(0)?,"mime":row.get::<_,String>(1)?,"size":row.get::<_,i64>(2)?}))))
    }
    pub fn attachment_ids(&self, message: &str) -> Result<Vec<String>, String> {
        let mut statement = db(self.connection.prepare(
            "SELECT attachment_id FROM message_attachments WHERE message_id=?1 ORDER BY ordinal",
        ))?;
        let result = db(db(statement.query_map([message], |r| r.get(0)))?.collect());
        result
    }
    pub fn remove_attachment(
        &mut self,
        conversation: &str,
        id: &str,
        expected: i64,
    ) -> Result<Draft, String> {
        let tx = db(self.connection.transaction())?;
        if db(tx.execute(
            "UPDATE drafts SET revision=revision+1 WHERE conversation_id=?1 AND revision=?2",
            params![conversation, expected],
        ))? != 1
        {
            return Err("conflict: The shared draft changed.".into());
        }
        db(tx.execute(
            "DELETE FROM draft_attachments WHERE conversation_id=?1 AND attachment_id=?2",
            params![conversation, id],
        ))?;
        db(tx.commit())?;
        self.draft(conversation)
    }
}

impl Store {
    pub fn preview(&self, input: &Start) -> Result<Vec<(Message, Vec<String>)>, String> {
        let conversation = self.conversation(&input.conversation_id)?;
        if conversation.revision != input.expected_revision {
            return Err("conflict: The active conversation changed.".into());
        }
        let draft = self.draft(&input.conversation_id)?;
        let (leaf, new_user, attachments) = match input.action.as_str() {
            "send" => {
                if draft.revision != input.draft_revision || draft.text != input.text {
                    return Err("conflict: The shared draft changed before Send.".into());
                }
                (conversation.active_leaf_id, true, draft.attachments)
            }
            "edit" => {
                let target = self.message(
                    &input.conversation_id,
                    input.target_id.as_deref().ok_or("Missing edit target.")?,
                )?;
                if target.role != "user" {
                    return Err("Choose a user message to edit.".into());
                }
                (target.parent_id, true, self.attachment_ids(&target.id)?)
            }
            "retry" => {
                let target = self.message(
                    &input.conversation_id,
                    input.target_id.as_deref().ok_or("Missing retry target.")?,
                )?;
                let user = if target.role == "user" {
                    target.id
                } else {
                    target.parent_id.ok_or("Missing user message.")?
                };
                (Some(user), false, vec![])
            }
            _ => return Err("Unknown generation action.".into()),
        };
        let (path, has_older) = self.path(&input.conversation_id, leaf.as_deref(), 2000, 0)?;
        if has_older {
            return Err(
                "This conversation exceeds the supported context depth. Start a new conversation."
                    .into(),
            );
        }
        let mut path = path
            .into_iter()
            .map(|m| self.attachment_ids(&m.id).map(|ids| (m, ids)))
            .collect::<Result<Vec<_>, _>>()?;
        if new_user {
            path.push((
                Message {
                    id: input.user_id.clone(),
                    parent_id: leaf,
                    role: "user".into(),
                    parts_version: 1,
                    parts: json!([{"type":"text","text":input.text}]),
                    status: "completed".into(),
                    metadata: json!({}),
                    previous_variant: None,
                    next_variant: None,
                    attachments: attachments.clone(),
                },
                attachments,
            ));
        }
        Ok(path)
    }
}

impl Store {
    pub fn export(&self, id: &str, format: &str) -> Result<Vec<u8>, String> {
        let conversation = self.conversation(id)?;
        let mut messages = if format == "markdown" {
            let (messages, more) =
                self.path(id, conversation.active_leaf_id.as_deref(), 100_000, 0)?;
            if more {
                return Err("This export exceeds the supported conversation depth.".into());
            }
            messages
        } else {
            let mut statement = db(self
                .connection
                .prepare("SELECT id FROM messages WHERE conversation_id=?1 ORDER BY rowid"))?;
            let mut rows = db(statement.query([id]))?;
            let mut messages = Vec::new();
            let mut size = 0;
            while let Some(row) = db(rows.next())? {
                let message = self.message(id, &db(row.get::<_, String>(0))?)?;
                size += serde_json::to_vec(&message)
                    .map_err(|_| "Invalid message.")?
                    .len();
                if size > MAX_CONTEXT {
                    return Err("The export exceeds the 40 MiB limit. Export the active variant as Markdown.".into());
                }
                messages.push(message);
            }
            messages
        };
        let mut attachments = Vec::new();
        let draft = self.draft(id)?;
        let mut ids = messages
            .iter()
            .flat_map(|m| m.attachments.clone())
            .chain(draft.attachments.iter().cloned())
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        for id in ids {
            attachments.push(db(self.connection.query_row("SELECT name,mime,size FROM attachments WHERE id=?1",[&id],|row|Ok(json!({"id":id,"name":row.get::<_,String>(0)?,"mime":row.get::<_,String>(1)?,"size":row.get::<_,i64>(2)?}))))?);
        }
        // Export provider/model metadata, not local credential identities or origin paths.
        for message in &mut messages {
            if let Some(meta) = message.metadata.as_object_mut() {
                meta.remove("connectionId");
                meta.remove("credentialRevision");
            }
        }
        if format == "markdown" {
            let mut text = format!(
                "# {}\n\nAttachment descriptions are included; binary files are not embedded.\n\n",
                conversation.title
            );
            for message in messages {
                text.push_str(&format!(
                    "## {}\n\n",
                    if message.role == "user" {
                        "You"
                    } else {
                        "Assistant"
                    }
                ));
                if message.role == "assistant" {
                    text.push_str(&format!(
                        "Model: {} · Status: {}\n\n",
                        message.metadata["model"].as_str().unwrap_or("Unknown"),
                        message.status
                    ));
                }
                for part in message.parts.as_array().ok_or("Invalid message parts.")? {
                    if matches!(part["type"].as_str(), Some("text" | "reasoning")) {
                        text.push_str(part["text"].as_str().unwrap_or(""));
                        text.push_str("\n\n");
                    }
                }
                for id in message.attachments {
                    if let Some(meta) = attachments.iter().find(|a| a["id"] == id) {
                        text.push_str(&format!(
                            "Attachment: {} ({}, {} bytes)\n\n",
                            meta["name"].as_str().unwrap_or("File"),
                            meta["mime"].as_str().unwrap_or(""),
                            meta["size"]
                        ));
                    }
                }
            }
            Ok(text.into_bytes())
        } else {
            serde_json::to_vec_pretty(&json!({"version":1,"conversation":{"id":id,"title":conversation.title,"activeLeafId":conversation.active_leaf_id,"system":conversation.config.system,"model":conversation.config.model},"messages":messages,"draft":draft,"attachments":attachments})).map_err(|_|"Cannot encode chat export.".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (tempfile::TempDir, Store) {
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
        (root, store)
    }
    fn start() -> Start {
        Start {
            request_id: "request".into(),
            conversation_id: "conversation".into(),
            assistant_id: "assistant".into(),
            user_id: "user".into(),
            expected_revision: 0,
            draft_revision: 1,
            action: "send".into(),
            target_id: None,
            text: "first".into(),
        }
    }
    #[test]
    fn send_consumes_exact_draft_and_retry_preserves_branch() {
        let (_root, mut store) = setup();
        store.save_draft("conversation", "first", 0).unwrap();
        let input = start();
        let accepted = store.begin(&input, &json!({"model":"fixture"})).unwrap();
        assert_eq!(accepted.draft_revision, 2);
        assert!(store.begin(&input, &json!({})).unwrap().repeated);
        let mut conflict = input.clone();
        conflict.text = "different".into();
        assert!(store.begin(&conflict, &json!({})).is_err());
        store.save_draft("conversation", "next draft", 2).unwrap();
        assert!(store.save_draft("conversation", "first", 1).is_err());
        assert_eq!(store.draft("conversation").unwrap().text, "next draft");
        store
            .checkpoint(
                "request",
                1,
                &json!([{"type":"text","text":"partial"}]),
                "cancelled",
                &json!({}),
            )
            .unwrap();
        let retry = Start {
            request_id: "retry".into(),
            assistant_id: "assistant-2".into(),
            expected_revision: 1,
            action: "retry".into(),
            target_id: Some("assistant".into()),
            ..input
        };
        let accepted = store.begin(&retry, &json!({})).unwrap();
        assert_eq!(accepted.user_id, "user");
        let context = store
            .path("conversation", Some(&accepted.user_id), 2000, 0)
            .unwrap()
            .0;
        assert_eq!(context.len(), 1);
        assert_eq!(context[0].id, "user");
        assert_eq!(
            store.message("conversation", "assistant").unwrap().parts[0]["text"],
            "partial"
        );
        store
            .checkpoint(
                "retry",
                1,
                &json!([{"type":"text","text":"new"}]),
                "completed",
                &json!({}),
            )
            .unwrap();
        store.select("conversation", "assistant", 2).unwrap();
        assert_eq!(
            store
                .load("conversation", 0)
                .unwrap()
                .messages
                .last()
                .unwrap()
                .id,
            "assistant"
        );
        assert_eq!(store.draft("conversation").unwrap().text, "next draft");
    }
    #[test]
    fn edit_keeps_old_descendants_and_recovery_never_resends() {
        let (root, mut store) = setup();
        store.save_draft("conversation", "first", 0).unwrap();
        let input = start();
        store.begin(&input, &json!({})).unwrap();
        store
            .checkpoint(
                "request",
                1,
                &json!([{"type":"text","text":"answer"}]),
                "completed",
                &json!({}),
            )
            .unwrap();
        let edit = Start {
            request_id: "edit-request".into(),
            assistant_id: "edited-answer".into(),
            user_id: "edited-user".into(),
            expected_revision: 1,
            action: "edit".into(),
            target_id: Some("user".into()),
            text: "edited".into(),
            ..input
        };
        store.begin(&edit, &json!({})).unwrap();
        store
            .checkpoint(
                "edit-request",
                1,
                &json!([{"type":"text","text":"checkpoint"}]),
                "active",
                &json!({}),
            )
            .unwrap();
        drop(store);
        let mut store = Store::open(root.path()).unwrap();
        let loaded = store.load("conversation", 0).unwrap();
        assert_eq!(loaded.messages[0].id, "edited-user");
        assert_eq!(loaded.messages[1].status, "interrupted");
        assert_eq!(loaded.messages[1].parts[0]["text"], "checkpoint");
        store.select("conversation", "user", 2).unwrap();
        assert_eq!(
            store
                .load("conversation", 0)
                .unwrap()
                .messages
                .last()
                .unwrap()
                .id,
            "assistant"
        );
    }
    #[test]
    fn history_v1_migration_preserves_messages_draft_and_search() {
        let (root, mut store) = setup();
        store.save_draft("conversation", "first", 0).unwrap();
        store.begin(&start(), &json!({})).unwrap();
        store
            .checkpoint(
                "request",
                1,
                &json!([{ "type": "text", "text": "answer" }]),
                "completed",
                &json!({}),
            )
            .unwrap();
        store.save_draft("conversation", "unsent draft", 2).unwrap();
        store.rename("conversation", "Saved conversation").unwrap();
        let before = serde_json::to_value(store.load("conversation", 0).unwrap()).unwrap();
        store.connection.execute_batch(
            "DROP INDEX conversation_history_order; ALTER TABLE conversations DROP COLUMN pinned; PRAGMA user_version=1;"
        ).unwrap();
        drop(store);
        let mut store = Store::open(root.path()).unwrap();
        assert_eq!(
            serde_json::to_value(store.load("conversation", 0).unwrap()).unwrap(),
            before
        );
        assert_eq!(
            store.list("Saved", None, None, 0).unwrap()[0].id,
            "conversation"
        );
        assert!(!store.conversation("conversation").unwrap().pinned);
        store.pin("conversation", true).unwrap();
        drop(store);
        assert!(
            Store::open(root.path())
                .unwrap()
                .conversation("conversation")
                .unwrap()
                .pinned
        );
    }
    #[test]
    fn pins_persist_and_sort_before_paginated_history_without_changing_revisions() {
        let (root, mut store) = setup();
        let initial = store.conversation("conversation").unwrap();
        store.save_draft("conversation", "first", 0).unwrap();
        for index in 0..60 {
            let id = format!("history-{index:02}");
            store
                .create(&id, &initial.origin, &Config::default())
                .unwrap();
            store
                .connection
                .execute(
                    "UPDATE conversations SET updated_at=?1 WHERE id=?2",
                    params![index, id],
                )
                .unwrap();
        }
        store.pin("history-00", true).unwrap();
        let first = store.list("", None, None, 0).unwrap();
        let second = store.list("", None, None, 50).unwrap();
        assert_eq!(first[0].id, "history-00");
        assert_eq!(first.len(), 50);
        assert_eq!(second.len(), 11);
        let ids: std::collections::HashSet<_> =
            first.iter().chain(&second).map(|item| &item.id).collect();
        assert_eq!(ids.len(), 61);
        assert!(store
            .list("", Some("other-workspace"), None, 0)
            .unwrap()
            .is_empty());
        assert!(store
            .list("", None, Some("other-project"), 0)
            .unwrap()
            .is_empty());
        store.rename("history-00", "Pinned search result").unwrap();
        assert_eq!(
            store
                .list("Pinned", Some("workspace"), Some("project"), 0)
                .unwrap()[0]
                .id,
            "history-00"
        );
        let pinned = store.pin("conversation", true).unwrap();
        assert_eq!(pinned.revision, initial.revision);
        assert_eq!(pinned.updated_at, initial.updated_at);
        assert_eq!(store.draft("conversation").unwrap().revision, 1);
        store.begin(&start(), &json!({})).unwrap();
        store.pin("conversation", false).unwrap();
        assert_eq!(store.conversation("conversation").unwrap().revision, 1);
        assert_eq!(
            store.load("conversation", 0).unwrap().request.unwrap()["status"],
            "active"
        );
        drop(store);
        let mut store = Store::open(root.path()).unwrap();
        assert!(store.conversation("history-00").unwrap().pinned);
        assert!(!store.conversation("conversation").unwrap().pinned);
        store.pin("history-00", false).unwrap();
        assert!(store
            .list("", None, None, 0)
            .unwrap()
            .iter()
            .all(|item| !item.pinned));
        store.pin("history-00", true).unwrap();
        store.delete("history-00").unwrap();
        assert!(store.list("Pinned", None, None, 0).unwrap().is_empty());
    }
    #[test]
    fn newer_database_and_cross_conversation_parent_are_rejected() {
        let (root, mut store) = setup();
        store
            .connection
            .execute_batch("PRAGMA user_version=99")
            .unwrap();
        assert!(Store::open(root.path()).is_err());
        store
            .connection
            .execute_batch("PRAGMA user_version=2")
            .unwrap();
        store.save_draft("conversation", "first", 0).unwrap();
        store.begin(&start(), &json!({})).unwrap();
        store
            .create(
                "other",
                &store.conversation("conversation").unwrap().origin,
                &Config::default(),
            )
            .unwrap();
        assert!(store.connection.execute("INSERT INTO messages(id,conversation_id,parent_id,role,parts,status) VALUES ('bad','other','user','user','[]','completed')",[]).is_err());
    }
}
