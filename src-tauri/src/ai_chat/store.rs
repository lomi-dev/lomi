//! threads.sqlite: thread index, API messages and idempotency receipts.
use super::attachments::{self, Attachment};
use super::settings::{Permissions, Workspace};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::{Path, PathBuf};

const APPLICATION_ID: i64 = 0x4C4D_4348;
pub(crate) const MAX_TEXT: usize = 256 * 1024;
pub(crate) const MAX_CONTEXT: usize = 2 * 1024 * 1024;
const REUSED: &str = "Operation ID was reused.";
const GONE: &str = "This thread was deleted.";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ThreadKind {
    Api,
    Cli,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Role {
    User,
    Assistant,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MessageState {
    Committed,
    Dispatching,
    Streaming,
    Completed,
    Stopped,
    Failed,
    Interrupted,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadSummary {
    pub thread_id: String,
    pub project: String,
    pub kind: ThreadKind,
    pub instance_id: String,
    pub model: String,
    pub title: String,
    /// Working directory of a CLI thread; `None` for API threads.
    pub cwd: Option<String>,
    /// Reasoning effort of a CLI thread; `None` uses the CLI's default.
    pub effort: Option<String>,
    pub permissions: Permissions,
    pub workspace: Workspace,
    /// Owning workspace; `None` for legacy threads, which every workspace of the project lists.
    pub workspace_id: Option<String>,
    pub revision: u64,
    pub created_ms: u64,
    pub updated_ms: u64,
    pub streaming: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadMessage {
    pub message_id: String,
    pub seq: u64,
    pub role: Role,
    pub content: String,
    pub state: MessageState,
    pub instance_id: String,
    pub model: String,
    pub stream_seq: u64,
    pub stop_reason: Option<String>,
    pub error: Option<String>,
    pub created_ms: u64,
    /// Files of a user message; empty otherwise.
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadSnapshot {
    pub thread: ThreadSummary,
    pub messages: Vec<ThreadMessage>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ThreadCreate {
    pub operation_id: String,
    pub project: String,
    pub instance_id: String,
    pub model: String,
    pub title: String,
    /// Working directory for CLI threads; ignored for API threads.
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    /// Copied from the settings defaults by the command, never taken from the client.
    #[serde(skip)]
    pub permissions: Permissions,
    #[serde(skip)]
    pub workspace: Workspace,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ThreadUpdate {
    pub thread_id: String,
    pub expected_revision: u64,
    pub title: Option<String>,
    pub instance_id: Option<String>,
    pub model: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ThreadDelete {
    pub thread_id: String,
    pub expected_revision: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ThreadSend {
    pub operation_id: String,
    pub thread_id: String,
    pub expected_revision: u64,
    pub instance_id: String,
    pub model: String,
    pub text: String,
    /// Unsent attachment ids, bound to the new user message. Omitted from the
    /// request hash when empty so older receipts keep matching.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<String>,
}

/// One history turn of an API request: role, text and attachments.
pub(crate) type Turn = (&'static str, String, Vec<Attachment>);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadSendAccepted {
    pub thread: ThreadSummary,
    pub user: ThreadMessage,
    pub assistant: ThreadMessage,
}

#[derive(Clone, Debug, Serialize)]
#[allow(clippy::large_enum_variant)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum ThreadStreamEvent {
    Delta {
        message_id: String,
        seq: u64,
        text: String,
    },
    Final {
        thread: ThreadSummary,
        message: ThreadMessage,
    },
}

#[derive(Debug)]
pub(crate) enum SendBegin {
    Replay(ThreadSendAccepted),
    New(ThreadSendAccepted, Vec<Turn>),
}

pub(crate) struct Finish<'a> {
    pub state: MessageState,
    pub content: &'a str,
    pub stream_seq: u64,
    pub stop_reason: Option<&'a str>,
    pub error: Option<&'a str>,
}

#[derive(Debug)]
pub(crate) struct Store {
    db: Connection,
    /// Chat storage directory; private thread folders live under `workspaces/`.
    dir: PathBuf,
}

fn db_err<E>(_: E) -> String {
    "The chat history database failed.".into()
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn json<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(db_err)
}

fn unjson<T: DeserializeOwned>(value: &str) -> Result<T, String> {
    serde_json::from_str(value).map_err(db_err)
}

fn enum_text<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn enum_from<T: DeserializeOwned>(text: String) -> rusqlite::Result<T> {
    serde_json::from_value(serde_json::Value::String(text)).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into())
    })
}

const THREAD_COLUMNS: &str = "id,project,kind,instance_id,model,title,cwd,revision,created_ms,updated_ms,\
    EXISTS(SELECT 1 FROM messages m WHERE m.thread_id=threads.id AND m.state IN('dispatching','streaming')),\
    effort,permissions,workspace,workspace_id";
const MESSAGE_COLUMNS: &str =
    "id,seq,role,content,state,instance_id,model,stream_seq,stop_reason,error,created_ms";

fn summary_row(r: &rusqlite::Row) -> rusqlite::Result<ThreadSummary> {
    Ok(ThreadSummary {
        thread_id: r.get(0)?,
        project: r.get(1)?,
        kind: enum_from(r.get(2)?)?,
        instance_id: r.get(3)?,
        model: r.get(4)?,
        title: r.get(5)?,
        cwd: r.get(6)?,
        revision: r.get::<_, i64>(7)? as u64,
        created_ms: r.get::<_, i64>(8)? as u64,
        updated_ms: r.get::<_, i64>(9)? as u64,
        streaming: r.get(10)?,
        effort: r.get(11)?,
        permissions: enum_from(r.get(12)?)?,
        workspace: enum_from(r.get(13)?)?,
        workspace_id: r.get(14)?,
    })
}

fn message_row(r: &rusqlite::Row) -> rusqlite::Result<ThreadMessage> {
    Ok(ThreadMessage {
        message_id: r.get(0)?,
        seq: r.get::<_, i64>(1)? as u64,
        role: enum_from(r.get(2)?)?,
        content: r.get(3)?,
        state: enum_from(r.get(4)?)?,
        instance_id: r.get(5)?,
        model: r.get(6)?,
        stream_seq: r.get::<_, i64>(7)? as u64,
        stop_reason: r.get(8)?,
        error: r.get(9)?,
        created_ms: r.get::<_, i64>(10)? as u64,
        attachments: Vec::new(),
    })
}

fn attachment_row(r: &rusqlite::Row) -> rusqlite::Result<Attachment> {
    Ok(Attachment {
        id: r.get(0)?,
        name: r.get(1)?,
        mime: r.get(2)?,
        kind: enum_from(r.get(3)?)?,
        size: r.get::<_, i64>(4)? as u64,
    })
}

const ATTACHMENT_COLUMNS: &str = "id,name,mime,kind,size";

fn message_attachments(db: &Connection, message_id: &str) -> Result<Vec<Attachment>, String> {
    let mut stmt = db
        .prepare(&format!(
            "SELECT {ATTACHMENT_COLUMNS} FROM attachments WHERE message_id=?1 ORDER BY created_ms, rowid"
        ))
        .map_err(db_err)?;
    let rows = stmt
        .query_map([message_id], attachment_row)
        .map_err(db_err)?;
    rows.collect::<rusqlite::Result<_>>().map_err(db_err)
}

pub(crate) fn valid_title(title: &str) -> Result<String, String> {
    let title = title.trim();
    let count = title.chars().count();
    if count == 0 || count > 200 || title.chars().any(char::is_control) {
        return Err("A thread title must be 1-200 characters without control characters.".into());
    }
    Ok(title.to_string())
}

fn get_thread(db: &Connection, id: &str) -> Result<ThreadSummary, String> {
    db.query_row(
        &format!("SELECT {THREAD_COLUMNS} FROM threads WHERE id=?1"),
        [id],
        summary_row,
    )
    .optional()
    .map_err(db_err)?
    .ok_or_else(|| GONE.to_string())
}

fn get_messages(db: &Connection, id: &str) -> Result<Vec<ThreadMessage>, String> {
    let mut stmt = db
        .prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages WHERE thread_id=?1 ORDER BY seq"
        ))
        .map_err(db_err)?;
    let rows = stmt.query_map([id], message_row).map_err(db_err)?;
    let mut out: Vec<ThreadMessage> = rows.collect::<rusqlite::Result<_>>().map_err(db_err)?;
    for m in out.iter_mut().filter(|m| m.role == Role::User) {
        m.attachments = message_attachments(db, &m.message_id)?;
    }
    Ok(out)
}

fn get_message(db: &Connection, id: &str) -> Result<ThreadMessage, String> {
    let mut m = db
        .query_row(
            &format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE id=?1"),
            [id],
            message_row,
        )
        .map_err(db_err)?;
    m.attachments = message_attachments(db, id)?;
    Ok(m)
}

fn request_hash(request: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(request.as_bytes()))
}

/// Receipts prove an operation was committed; they keep only a request hash and
/// row ids, never message text. Returns (thread id, message ids) for a replay.
fn replay(
    db: &Connection,
    operation: &str,
    request: &str,
) -> Result<Option<(String, Vec<String>)>, String> {
    let row: Option<(String, String, String)> = db
        .query_row(
            "SELECT request_hash,thread_id,result FROM receipts WHERE operation_id=?1",
            [operation],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(db_err)?;
    match row {
        None => Ok(None),
        Some((stored, thread, ids)) if stored == request_hash(request) => {
            Ok(Some((thread, unjson(&ids)?)))
        }
        Some(_) => Err(REUSED.into()),
    }
}

fn insert_receipt(
    db: &Connection,
    operation: &str,
    request: &str,
    thread_id: &str,
    message_ids: &[&str],
) -> Result<(), String> {
    db.execute(
        "INSERT INTO receipts(operation_id,request_hash,thread_id,result,created_ms) VALUES(?1,?2,?3,?4,?5)",
        params![
            operation,
            request_hash(request),
            thread_id,
            json(&message_ids)?,
            now_ms() as i64
        ],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Rebuilds a send result from live rows: the receipt only proves existence.
fn accepted_from_rows(
    db: &Connection,
    thread_id: &str,
    ids: &[String],
) -> Result<ThreadSendAccepted, String> {
    let [user, assistant] = ids else {
        return Err(db_err(()));
    };
    Ok(ThreadSendAccepted {
        thread: get_thread(db, thread_id)?,
        user: get_message(db, user)?,
        assistant: get_message(db, assistant)?,
    })
}

const RECEIPT_MAX_AGE_MS: u64 = 30 * 24 * 60 * 60 * 1000;
const RECEIPT_MAX_COUNT: i64 = 5000;

fn prune_receipts(
    db: &Connection,
    now: u64,
    max_age_ms: u64,
    max_count: i64,
) -> Result<(), String> {
    db.execute(
        "DELETE FROM receipts WHERE created_ms < ?1 OR rowid NOT IN
         (SELECT rowid FROM receipts ORDER BY created_ms DESC, rowid DESC LIMIT ?2)",
        params![now.saturating_sub(max_age_ms) as i64, max_count],
    )
    .map_err(db_err)?;
    Ok(())
}

const UNSENT_MAX_AGE_MS: u64 = 24 * 60 * 60 * 1000;

fn remove_files(dir: &Path, files: &[Attachment]) {
    for a in files {
        let _ = std::fs::remove_file(attachments::path(&attachments::dir(dir), a));
    }
}

/// Deletes unsent attachments older than 24 h, rows and files.
fn prune_unsent(db: &Connection, dir: &Path, now: u64) -> Result<(), String> {
    let cutoff = now.saturating_sub(UNSENT_MAX_AGE_MS) as i64;
    let mut stmt = db
        .prepare(&format!(
            "SELECT {ATTACHMENT_COLUMNS} FROM attachments WHERE message_id IS NULL AND created_ms < ?1"
        ))
        .map_err(db_err)?;
    let ids: Vec<Attachment> = stmt
        .query_map([cutoff], attachment_row)
        .map_err(db_err)?
        .collect::<rusqlite::Result<_>>()
        .map_err(db_err)?;
    drop(stmt);
    db.execute(
        "DELETE FROM attachments WHERE message_id IS NULL AND created_ms < ?1",
        [cutoff],
    )
    .map_err(db_err)?;
    remove_files(dir, &ids);
    Ok(())
}

/// Drops the oldest turns until history plus `tail` bytes fit the budget. The
/// history then starts with a user turn, as Anthropic requires.
fn fit_context(history: &mut Vec<Turn>, tail: usize) {
    let mut size = tail + history.iter().map(|(_, c, _)| c.len()).sum::<usize>();
    let mut drop = 0;
    while drop < history.len() && (size > MAX_CONTEXT || history[drop].0 != "user") {
        size -= history[drop].1.len();
        drop += 1;
    }
    history.drain(..drop);
}

fn new_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| "Cannot create an identifier.")?;
    Ok(bytes.iter().map(|v| format!("{v:02x}")).collect())
}

/// Additive migration of databases created before CLI threads; the schema
/// version stays 1 because older readers select explicit columns.
fn add_missing_columns(db: &Connection) -> Result<(), String> {
    let mut stmt = db
        .prepare("SELECT name FROM pragma_table_info('threads')")
        .map_err(db_err)?;
    let have: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .map_err(db_err)?
        .collect::<rusqlite::Result<_>>()
        .map_err(db_err)?;
    for (column, definition) in [
        ("cwd", "TEXT"),
        ("native_session", "TEXT"),
        ("effort", "TEXT"),
        ("permissions", "TEXT NOT NULL DEFAULT 'read-only'"),
        ("workspace", "TEXT NOT NULL DEFAULT 'project'"),
        ("workspace_id", "TEXT"),
    ] {
        if !have.iter().any(|c| c == column) {
            db.execute(
                &format!("ALTER TABLE threads ADD COLUMN {column} {definition}"),
                [],
            )
            .map_err(db_err)?;
        }
    }
    Ok(())
}

impl Store {
    /// Opens or creates the database and marks rows left in flight by a previous
    /// process as interrupted. Other schema versions are refused and preserved.
    pub(crate) fn open(dir: &Path) -> Result<Self, String> {
        let path = dir.join("threads.sqlite");
        for suffix in ["", "-wal", "-shm"] {
            crate::storage_guard::reject_link(&dir.join(format!("threads.sqlite{suffix}")))?;
        }
        let db = Connection::open(&path).map_err(db_err)?;
        // Inspect before changing anything (WAL mode, permissions) so a foreign
        // database stays byte-for-byte untouched.
        let version: i64 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(db_err)?;
        let app: i64 = db
            .pragma_query_value(None, "application_id", |r| r.get(0))
            .map_err(db_err)?;
        let tables: i64 = db
            .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))
            .map_err(db_err)?;
        let fresh = match (version, app, tables) {
            (0, 0, 0) => true,
            (1, a, _) if a == APPLICATION_ID => false,
            _ => {
                return Err(format!(
                    "Chat history uses an unsupported format and was preserved at {}.",
                    path.display()
                ))
            }
        };
        crate::storage_guard::private(&path, false)?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
        )
        .map_err(db_err)?;
        if fresh {
            db.execute_batch(&format!(
                "BEGIN IMMEDIATE;
                 CREATE TABLE threads(id TEXT PRIMARY KEY, project TEXT NOT NULL,
                   kind TEXT NOT NULL CHECK(kind IN('api','cli')), instance_id TEXT NOT NULL,
                   model TEXT NOT NULL, title TEXT NOT NULL, cwd TEXT, native_session TEXT,
                   effort TEXT, permissions TEXT NOT NULL DEFAULT 'read-only',
                   workspace TEXT NOT NULL DEFAULT 'project', workspace_id TEXT,
                   revision INTEGER NOT NULL, created_ms INTEGER NOT NULL, updated_ms INTEGER NOT NULL);
                 CREATE INDEX threads_project ON threads(project, updated_ms DESC);
                 CREATE TABLE messages(id TEXT PRIMARY KEY,
                   thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
                   seq INTEGER NOT NULL, role TEXT NOT NULL CHECK(role IN('user','assistant')),
                   content TEXT NOT NULL, state TEXT NOT NULL, instance_id TEXT NOT NULL,
                   model TEXT NOT NULL, stream_seq INTEGER NOT NULL DEFAULT 0, stop_reason TEXT,
                   error TEXT, operation_id TEXT, created_ms INTEGER NOT NULL,
                   UNIQUE(thread_id, seq));
                 CREATE TABLE receipts(operation_id TEXT PRIMARY KEY, request_hash TEXT NOT NULL,
                   thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
                   result TEXT NOT NULL, created_ms INTEGER NOT NULL);
                 CREATE INDEX receipts_thread ON receipts(thread_id);
                 PRAGMA application_id={APPLICATION_ID}; PRAGMA user_version=1; COMMIT;"
            ))
            .map_err(db_err)?;
        }
        if !fresh {
            add_missing_columns(&db)?;
        }
        // Created for every database: new installs and ones that predate attachments.
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS attachments(id TEXT PRIMARY KEY,
               message_id TEXT REFERENCES messages(id) ON DELETE CASCADE,
               name TEXT NOT NULL, mime TEXT NOT NULL, kind TEXT NOT NULL,
               size INTEGER NOT NULL, created_ms INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS attachments_message ON attachments(message_id);
             CREATE INDEX IF NOT EXISTS threads_workspace ON threads(project, workspace_id, updated_ms DESC);",
        )
        .map_err(db_err)?;
        prune_unsent(&db, dir, now_ms())?;
        prune_receipts(&db, now_ms(), RECEIPT_MAX_AGE_MS, RECEIPT_MAX_COUNT)?;
        db.execute(
            "UPDATE messages SET state='interrupted' WHERE state IN('dispatching','streaming')",
            [],
        )
        .map_err(db_err)?;
        Ok(Self {
            db,
            dir: dir.to_path_buf(),
        })
    }

    pub(crate) fn attachments_dir(&self) -> PathBuf {
        attachments::dir(&self.dir)
    }

    /// Validates and stores a file as an unsent attachment.
    pub(crate) fn attachment_import(&self, name: &str, data: &[u8]) -> Result<Attachment, String> {
        let (kind, mime) = attachments::classify(data)?;
        let a = Attachment {
            id: new_id()?,
            name: attachments::sanitize_name(name),
            mime: mime.into(),
            kind,
            size: data.len() as u64,
        };
        let dir = self.attachments_dir();
        crate::storage_guard::reject_link(&dir)?;
        std::fs::create_dir_all(&dir).map_err(|_| "Cannot create the attachments folder.")?;
        crate::storage_guard::private(&dir, true)?;
        let path = dir.join(attachments::file_name(&a));
        crate::storage_guard::atomic(&path, data)?;
        let inserted = self.db.execute(
            "INSERT INTO attachments(id,message_id,name,mime,kind,size,created_ms) VALUES(?1,NULL,?2,?3,?4,?5,?6)",
            params![a.id, a.name, a.mime, enum_text(&a.kind), a.size as i64, now_ms() as i64],
        );
        if inserted.is_err() {
            let _ = std::fs::remove_file(&path);
            return Err(db_err(()));
        }
        Ok(a)
    }

    pub(crate) fn attachment(&self, id: &str) -> Result<Attachment, String> {
        self.db
            .query_row(
                &format!("SELECT {ATTACHMENT_COLUMNS} FROM attachments WHERE id=?1"),
                [id],
                attachment_row,
            )
            .optional()
            .map_err(db_err)?
            .ok_or_else(|| "Unknown attachment.".to_string())
    }

    /// Deletes an attachment that no message uses yet.
    pub(crate) fn attachment_discard(&self, id: &str) -> Result<(), String> {
        let attachment = self.attachment(id)?;
        let n = self
            .db
            .execute(
                "DELETE FROM attachments WHERE id=?1 AND message_id IS NULL",
                [id],
            )
            .map_err(db_err)?;
        if n == 0 {
            return Err("This attachment was already sent.".into());
        }
        remove_files(&self.dir, &[attachment]);
        Ok(())
    }

    pub(crate) fn list(
        &self,
        project: &str,
        workspace_id: &str,
    ) -> Result<Vec<ThreadSummary>, String> {
        let mut stmt = self
            .db
            .prepare(&format!(
                "SELECT {THREAD_COLUMNS} FROM threads WHERE project=?1 AND (workspace_id=?2 OR workspace_id IS NULL) ORDER BY updated_ms DESC, id LIMIT 500"
            ))
            .map_err(db_err)?;
        let rows = stmt
            .query_map([project, workspace_id], summary_row)
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }

    #[cfg(test)]
    pub(crate) fn thread(&self, id: &str) -> Result<ThreadSummary, String> {
        get_thread(&self.db, id)
    }

    pub(crate) fn snapshot(&self, id: &str) -> Result<ThreadSnapshot, String> {
        Ok(ThreadSnapshot {
            thread: get_thread(&self.db, id)?,
            messages: get_messages(&self.db, id)?,
        })
    }

    pub(crate) fn create_replay(
        &self,
        req: &ThreadCreate,
    ) -> Result<Option<ThreadSummary>, String> {
        match replay(&self.db, &req.operation_id, &json(req)?)? {
            Some((thread, _)) => get_thread(&self.db, &thread).map(Some),
            None => Ok(None),
        }
    }

    pub(crate) fn create(
        &mut self,
        req: &ThreadCreate,
        title: &str,
        kind: ThreadKind,
    ) -> Result<ThreadSummary, String> {
        let request = json(req)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        if let Some((thread, _)) = replay(&tx, &req.operation_id, &request)? {
            return get_thread(&tx, &thread);
        }
        let id = new_id()?;
        let now = now_ms() as i64;
        // API threads ignore cwd, effort, permissions and workspace.
        let cli = kind == ThreadKind::Cli;
        let (workspace, private) = match (cli, req.workspace) {
            (true, Workspace::Private) => (
                Workspace::Private,
                Some(self.dir.join("workspaces").join(&id)),
            ),
            _ => (Workspace::Project, None),
        };
        let cwd = match &private {
            Some(path) => Some(path.to_string_lossy().into_owned()),
            None => req.cwd.clone().filter(|_| cli),
        };
        let permissions = if cli {
            req.permissions
        } else {
            Permissions::ReadOnly
        };
        tx.execute(
            "INSERT INTO threads(id,project,kind,instance_id,model,title,cwd,effort,permissions,workspace,workspace_id,revision,created_ms,updated_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,1,?12,?12)",
            params![
                id,
                req.project,
                enum_text(&kind),
                req.instance_id,
                req.model,
                title,
                cwd,
                req.effort.as_deref().filter(|_| cli),
                enum_text(&permissions),
                enum_text(&workspace),
                req.workspace_id,
                now
            ],
        )
        .map_err(db_err)?;
        if let Some(path) = private {
            std::fs::create_dir_all(&path).map_err(|_| "Cannot create the private folder.")?;
            for dir in [path.parent().unwrap_or(&path), &path] {
                crate::storage_guard::private(dir, true)?;
            }
        }
        let thread = get_thread(&tx, &id)?;
        insert_receipt(&tx, &req.operation_id, &request, &id, &[])?;
        tx.commit().map_err(db_err)?;
        Ok(thread)
    }

    /// `check` validates the requested instance/model against the provider registry.
    pub(crate) fn update(
        &mut self,
        req: &ThreadUpdate,
        check: impl FnOnce(&ThreadSummary) -> Result<(), String>,
    ) -> Result<ThreadSummary, String> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        let thread = get_thread(&tx, &req.thread_id)?;
        if thread.revision != req.expected_revision {
            return Err("This thread changed in another view. Reload and try again.".into());
        }
        let title = req.title.as_deref().map(valid_title).transpose()?;
        let changes_target = req.instance_id.is_some() || req.model.is_some();
        if changes_target {
            if thread.streaming {
                return Err("Wait for the response to finish first.".into());
            }
            if thread.kind == ThreadKind::Cli && req.model.is_some() {
                return Err("A CLI thread keeps the model it was created with.".into());
            }
            check(&thread)?;
        }
        tx.execute(
            "UPDATE threads SET title=COALESCE(?2,title), instance_id=COALESCE(?3,instance_id),
             model=COALESCE(?4,model), revision=revision+1, updated_ms=?5 WHERE id=?1",
            params![
                req.thread_id,
                title,
                req.instance_id,
                req.model,
                now_ms() as i64
            ],
        )
        .map_err(db_err)?;
        let thread = get_thread(&tx, &req.thread_id)?;
        tx.commit().map_err(db_err)?;
        Ok(thread)
    }

    pub(crate) fn delete(&mut self, req: &ThreadDelete) -> Result<(), String> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        let thread = get_thread(&tx, &req.thread_id)?;
        if thread.revision != req.expected_revision {
            return Err("This thread changed in another view. Reload and try again.".into());
        }
        if thread.streaming {
            return Err("Stop the response before deleting this thread.".into());
        }
        let ids: Vec<Attachment> = {
            let mut stmt = tx
                .prepare(
                    "SELECT a.id,a.name,a.mime,a.kind,a.size FROM attachments a JOIN messages m ON m.id=a.message_id WHERE m.thread_id=?1",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map([&req.thread_id], attachment_row)
                .map_err(db_err)?;
            rows.collect::<rusqlite::Result<_>>().map_err(db_err)?
        };
        tx.execute("DELETE FROM threads WHERE id=?1", [&req.thread_id])
            .map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        remove_files(&self.dir, &ids);
        Ok(())
    }

    /// Commits the user message, the dispatching placeholder and the receipt
    /// before any network I/O. `check` validates thread kind and provider.
    pub(crate) fn send_begin(
        &mut self,
        req: &ThreadSend,
        extra_bytes: usize,
        check: impl FnOnce(&ThreadSummary) -> Result<(), String>,
    ) -> Result<SendBegin, String> {
        let request = json(req)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        if let Some((thread, ids)) = replay(&tx, &req.operation_id, &request)? {
            return Ok(SendBegin::Replay(accepted_from_rows(&tx, &thread, &ids)?));
        }
        let thread = get_thread(&tx, &req.thread_id)?;
        if thread.revision != req.expected_revision {
            return Err("This thread changed in another view. Reload and try again.".into());
        }
        if (req.text.is_empty() && req.attachments.is_empty()) || req.text.len() > MAX_TEXT {
            return Err("A message must be between 1 byte and 256 KiB.".into());
        }
        let mut files = Vec::new();
        for id in &req.attachments {
            let a = tx
                .query_row(
                    &format!(
                        "SELECT {ATTACHMENT_COLUMNS} FROM attachments WHERE id=?1 AND message_id IS NULL"
                    ),
                    [id],
                    attachment_row,
                )
                .optional()
                .map_err(db_err)?
                .ok_or("An attachment is unknown or was already sent.")?;
            if files.iter().any(|f: &Attachment| f.id == a.id) {
                return Err("An attachment was listed twice.".into());
            }
            files.push(a);
        }
        if files.len() > attachments::MAX_COUNT
            || files.iter().map(|f| f.size).sum::<u64>() > attachments::MAX_TOTAL
        {
            return Err("A message takes at most 10 attachments and 40 MB in total.".into());
        }
        if thread.streaming {
            return Err("This thread is already waiting for a response.".into());
        }
        check(&thread)?;
        let tail = req.text.len() + extra_bytes;
        if tail > MAX_CONTEXT {
            return Err("This thread exceeds the context budget; start a new thread.".into());
        }
        let mut context: Vec<Turn> = Vec::new();
        let mut last_seq = 0u64;
        for m in get_messages(&tx, &req.thread_id)? {
            last_seq = m.seq;
            let keep = match (m.role, m.state) {
                (Role::User, MessageState::Committed) => Some("user"),
                (Role::Assistant, MessageState::Completed | MessageState::Stopped)
                    if !m.content.is_empty() =>
                {
                    Some("assistant")
                }
                _ => None,
            };
            if let Some(role) = keep {
                context.push((role, m.content, m.attachments));
            }
        }
        fit_context(&mut context, tail);
        context.push(("user", req.text.clone(), files.clone()));
        let now = now_ms() as i64;
        let (user_id, assistant_id) = (new_id()?, new_id()?);
        tx.execute(
            "INSERT INTO messages(id,thread_id,seq,role,content,state,instance_id,model,operation_id,created_ms)
             VALUES(?1,?2,?3,'user',?4,'committed',?5,?6,?7,?8)",
            params![user_id, req.thread_id, last_seq as i64 + 1, req.text, req.instance_id, req.model, req.operation_id, now],
        )
        .map_err(db_err)?;
        for f in &files {
            tx.execute(
                "UPDATE attachments SET message_id=?2 WHERE id=?1 AND message_id IS NULL",
                params![f.id, user_id],
            )
            .map_err(db_err)?;
        }
        tx.execute(
            "INSERT INTO messages(id,thread_id,seq,role,content,state,instance_id,model,operation_id,created_ms)
             VALUES(?1,?2,?3,'assistant','','dispatching',?4,?5,?6,?7)",
            params![assistant_id, req.thread_id, last_seq as i64 + 2, req.instance_id, req.model, req.operation_id, now],
        )
        .map_err(db_err)?;
        tx.execute(
            "UPDATE threads SET instance_id=?2, model=?3, revision=revision+1, updated_ms=?4 WHERE id=?1",
            params![req.thread_id, req.instance_id, req.model, now],
        )
        .map_err(db_err)?;
        let accepted = ThreadSendAccepted {
            thread: get_thread(&tx, &req.thread_id)?,
            user: get_message(&tx, &user_id)?,
            assistant: get_message(&tx, &assistant_id)?,
        };
        insert_receipt(
            &tx,
            &req.operation_id,
            &request,
            &req.thread_id,
            &[&user_id, &assistant_id],
        )?;
        tx.commit().map_err(db_err)?;
        Ok(SendBegin::New(accepted, context))
    }

    pub(crate) fn native_session(&self, thread_id: &str) -> Result<Option<String>, String> {
        self.db
            .query_row(
                "SELECT native_session FROM threads WHERE id=?1",
                [thread_id],
                |r| r.get(0),
            )
            .map_err(db_err)
    }

    /// `None` clears the stored session so the next send starts fresh.
    pub(crate) fn set_native_session(
        &self,
        thread_id: &str,
        session: Option<&str>,
    ) -> Result<(), String> {
        self.db
            .execute(
                "UPDATE threads SET native_session=?2 WHERE id=?1",
                params![thread_id, session],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub(crate) fn mark_streaming(&self, message_id: &str) -> Result<(), String> {
        self.db
            .execute(
                "UPDATE messages SET state='streaming' WHERE id=?1 AND state='dispatching'",
                [message_id],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub(crate) fn flush(
        &self,
        message_id: &str,
        content: &str,
        stream_seq: u64,
    ) -> Result<(), String> {
        self.db
            .execute(
                "UPDATE messages SET content=?2, stream_seq=?3 WHERE id=?1 AND state IN('dispatching','streaming')",
                params![message_id, content, stream_seq as i64],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Marks in-flight rows of a thread interrupted when no stream owns them
    /// (e.g. after a panic). Returns whether anything changed.
    pub(crate) fn interrupt_inflight(
        &mut self,
        thread_id: &str,
        bump: bool,
    ) -> Result<bool, String> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        let n = tx
            .execute(
                "UPDATE messages SET state='interrupted' WHERE thread_id=?1 AND state IN('dispatching','streaming')",
                [thread_id],
            )
            .map_err(db_err)?;
        if n > 0 && bump {
            tx.execute(
                "UPDATE threads SET revision=revision+1, updated_ms=?2 WHERE id=?1",
                params![thread_id, now_ms() as i64],
            )
            .map_err(db_err)?;
        }
        tx.commit().map_err(db_err)?;
        Ok(n > 0)
    }

    /// Last-resort completion: keeps stored partial content, only fixes the state.
    pub(crate) fn finish_minimal(
        &mut self,
        thread_id: &str,
        message_id: &str,
    ) -> Result<(ThreadSummary, ThreadMessage), String> {
        self.db
            .execute(
                "UPDATE messages SET state='interrupted', error=COALESCE(error,'The response could not be saved.') WHERE id=?1 AND state IN('dispatching','streaming')",
                [message_id],
            )
            .map_err(db_err)?;
        self.db
            .execute(
                "UPDATE threads SET revision=revision+1, updated_ms=?2 WHERE id=?1",
                params![thread_id, now_ms() as i64],
            )
            .map_err(db_err)?;
        Ok((
            get_thread(&self.db, thread_id)?,
            get_message(&self.db, message_id)?,
        ))
    }

    pub(crate) fn finish(
        &mut self,
        thread_id: &str,
        message_id: &str,
        f: &Finish,
    ) -> Result<(ThreadSummary, ThreadMessage), String> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        tx.execute(
            "UPDATE messages SET state=?2, content=?3, stream_seq=?4, stop_reason=?5, error=?6 WHERE id=?1",
            params![message_id, enum_text(&f.state), f.content, f.stream_seq as i64, f.stop_reason, f.error],
        )
        .map_err(db_err)?;
        tx.execute(
            "UPDATE threads SET revision=revision+1, updated_ms=?2 WHERE id=?1",
            params![thread_id, now_ms() as i64],
        )
        .map_err(db_err)?;
        let out = (get_thread(&tx, thread_id)?, get_message(&tx, message_id)?);
        tx.commit().map_err(db_err)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_req(op: &str) -> ThreadCreate {
        ThreadCreate {
            operation_id: op.into(),
            project: "/p".into(),
            instance_id: "openai".into(),
            model: "gpt".into(),
            title: "Hello".into(),
            cwd: None,
            effort: None,
            workspace_id: Some("w1".into()),
            permissions: Permissions::default(),
            workspace: Workspace::default(),
        }
    }

    fn send_req(op: &str, thread: &ThreadSummary, text: &str) -> ThreadSend {
        ThreadSend {
            operation_id: op.into(),
            thread_id: thread.thread_id.clone(),
            expected_revision: thread.revision,
            instance_id: "openai".into(),
            model: "gpt".into(),
            text: text.into(),
            attachments: Vec::new(),
        }
    }

    fn open() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        (dir, store)
    }

    #[test]
    fn create_replays_and_rejects_reuse() {
        let (_d, mut s) = open();
        let a = s
            .create(&create_req("op1"), "Hello", ThreadKind::Api)
            .unwrap();
        let b = s
            .create(&create_req("op1"), "Hello", ThreadKind::Api)
            .unwrap();
        assert_eq!(a, b);
        assert_eq!(s.list("/p", "w1").unwrap().len(), 1);
        let mut other = create_req("op1");
        other.model = "other".into();
        assert_eq!(
            s.create(&other, "Hello", ThreadKind::Api).unwrap_err(),
            REUSED
        );
        assert!(s.list("/q", "w1").unwrap().is_empty());
    }

    #[test]
    fn send_replay_and_reuse() {
        let (_d, mut s) = open();
        let t = s.create(&create_req("c"), "T", ThreadKind::Api).unwrap();
        let req = send_req("s1", &t, "hi");
        let SendBegin::New(first, ctx) = s.send_begin(&req, 0, |_| Ok(())).unwrap() else {
            panic!("expected new");
        };
        assert_eq!(ctx, vec![("user", "hi".to_string(), vec![])]);
        assert_eq!(first.assistant.state, MessageState::Dispatching);
        assert_eq!(first.thread.revision, 2);
        // Same operation replays even though the revision moved on.
        let SendBegin::Replay(again) = s.send_begin(&req, 0, |_| Ok(())).unwrap() else {
            panic!("expected replay");
        };
        assert_eq!(again.user.message_id, first.user.message_id);
        assert_eq!(s.snapshot(&t.thread_id).unwrap().messages.len(), 2);
        let mut changed = req.clone();
        changed.text = "different".into();
        assert!(matches!(
            s.send_begin(&changed, 0, |_| Ok(())),
            Err(e) if e == REUSED
        ));
        // A second send while the first is in flight is refused and writes nothing.
        let thread = s.thread(&t.thread_id).unwrap();
        assert!(thread.streaming);
        assert!(s
            .send_begin(&send_req("s2", &thread, "x"), 0, |_| Ok(()))
            .is_err());
        assert_eq!(s.snapshot(&t.thread_id).unwrap().messages.len(), 2);
        assert!(s
            .delete(&ThreadDelete {
                thread_id: t.thread_id.clone(),
                expected_revision: thread.revision
            })
            .is_err());
    }

    #[test]
    fn finish_builds_context_and_enforces_limits() {
        let (_d, mut s) = open();
        let t = s.create(&create_req("c"), "T", ThreadKind::Api).unwrap();
        let SendBegin::New(acc, _) = s
            .send_begin(&send_req("s1", &t, "q1"), 0, |_| Ok(()))
            .unwrap()
        else {
            panic!()
        };
        s.mark_streaming(&acc.assistant.message_id).unwrap();
        s.flush(&acc.assistant.message_id, "par", 1).unwrap();
        let (thread, msg) = s
            .finish(
                &t.thread_id,
                &acc.assistant.message_id,
                &Finish {
                    state: MessageState::Stopped,
                    content: "partial",
                    stream_seq: 2,
                    stop_reason: Some("stop"),
                    error: None,
                },
            )
            .unwrap();
        assert!(!thread.streaming);
        assert_eq!(msg.state, MessageState::Stopped);
        let SendBegin::New(_, ctx) = s
            .send_begin(&send_req("s2", &thread, "q2"), 0, |_| Ok(()))
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(
            ctx,
            vec![
                ("user", "q1".to_string(), vec![]),
                ("assistant", "partial".to_string(), vec![]),
                ("user", "q2".to_string(), vec![])
            ]
        );
        let thread = s.thread(&t.thread_id).unwrap();
        let mut done = s.snapshot(&t.thread_id).unwrap().messages;
        let last = done.pop().unwrap();
        s.finish(
            &t.thread_id,
            &last.message_id,
            &Finish {
                state: MessageState::Failed,
                content: "",
                stream_seq: 0,
                stop_reason: None,
                error: Some("e"),
            },
        )
        .unwrap();
        let thread2 = s.thread(&t.thread_id).unwrap();
        assert!(thread2.revision > thread.revision);
        let err = s
            .send_begin(&send_req("s3", &thread2, "x"), MAX_CONTEXT, |_| Ok(()))
            .unwrap_err();
        assert!(err.contains("context budget"));
        assert!(s
            .send_begin(&send_req("s4", &thread2, ""), 0, |_| Ok(()))
            .is_err());
        let big = "x".repeat(MAX_TEXT + 1);
        assert!(s
            .send_begin(&send_req("s5", &thread2, &big), 0, |_| Ok(()))
            .is_err());
        // Failed sends commit no rows.
        assert_eq!(s.snapshot(&t.thread_id).unwrap().messages.len(), 4);
    }

    #[test]
    fn startup_marks_interrupted() {
        let dir = tempfile::tempdir().unwrap();
        let (tid, mid) = {
            let mut s = Store::open(dir.path()).unwrap();
            let t = s.create(&create_req("c"), "T", ThreadKind::Api).unwrap();
            let SendBegin::New(acc, _) = s
                .send_begin(&send_req("s1", &t, "q"), 0, |_| Ok(()))
                .unwrap()
            else {
                panic!()
            };
            s.mark_streaming(&acc.assistant.message_id).unwrap();
            (t.thread_id, acc.assistant.message_id)
        };
        let s = Store::open(dir.path()).unwrap();
        let snap = s.snapshot(&tid).unwrap();
        let m = snap.messages.iter().find(|m| m.message_id == mid).unwrap();
        assert_eq!(m.state, MessageState::Interrupted);
        assert!(!snap.thread.streaming);
    }

    #[test]
    fn update_delete_and_foreign_database() {
        let (dir, mut s) = open();
        let t = s.create(&create_req("c"), "T", ThreadKind::Api).unwrap();
        let upd = |title: Option<&str>, rev| ThreadUpdate {
            thread_id: t.thread_id.clone(),
            expected_revision: rev,
            title: title.map(Into::into),
            instance_id: None,
            model: None,
        };
        let renamed = s.update(&upd(Some(" New "), 1), |_| Ok(())).unwrap();
        assert_eq!(renamed.title, "New");
        assert!(s.update(&upd(Some("x"), 1), |_| Ok(())).is_err());
        assert!(s.update(&upd(Some("  "), 2), |_| Ok(())).is_err());
        s.delete(&ThreadDelete {
            thread_id: t.thread_id.clone(),
            expected_revision: 2,
        })
        .unwrap();
        assert_eq!(s.thread(&t.thread_id).unwrap_err(), GONE);
        drop(s);
        let other = tempfile::tempdir().unwrap();
        let path = other.path().join("threads.sqlite");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE x(a); PRAGMA user_version=7;")
            .unwrap();
        drop(db);
        assert!(Store::open(other.path()).unwrap_err().contains("preserved"));
        drop(dir);
    }

    fn begin(s: &mut Store, t: &ThreadSummary, op: &str, text: &str) -> ThreadSendAccepted {
        match s.send_begin(&send_req(op, t, text), 0, |_| Ok(())).unwrap() {
            SendBegin::New(a, _) => a,
            SendBegin::Replay(_) => panic!("expected new"),
        }
    }

    fn done(content: &str) -> Finish<'_> {
        Finish {
            state: MessageState::Completed,
            content,
            stream_seq: 1,
            stop_reason: Some("stop"),
            error: None,
        }
    }

    #[test]
    fn receipts_hold_no_text_and_follow_the_thread() {
        let (_d, mut s) = open();
        let t = s.create(&create_req("c"), "T", ThreadKind::Api).unwrap();
        begin(&mut s, &t, "s1", "secret-user-text");
        let rows: Vec<String> = {
            let mut stmt =
                s.db.prepare("SELECT request_hash||thread_id||result FROM receipts")
                    .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| !r.contains("secret-user-text")));
        let thread = s.thread(&t.thread_id).unwrap();
        s.interrupt_inflight(&t.thread_id, true).unwrap();
        let thread = s.thread(&thread.thread_id).unwrap();
        s.delete(&ThreadDelete {
            thread_id: t.thread_id.clone(),
            expected_revision: thread.revision,
        })
        .unwrap();
        let n: i64 =
            s.db.query_row("SELECT count(*) FROM receipts", [], |r| r.get(0))
                .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn replay_returns_live_state() {
        let (_d, mut s) = open();
        let t = s.create(&create_req("c"), "T", ThreadKind::Api).unwrap();
        let acc = begin(&mut s, &t, "s1", "q");
        s.finish(&t.thread_id, &acc.assistant.message_id, &done("answer"))
            .unwrap();
        let thread = s.thread(&t.thread_id).unwrap();
        let SendBegin::Replay(again) = s
            .send_begin(&send_req("s1", &t, "q"), 0, |_| Ok(()))
            .unwrap()
        else {
            panic!("expected replay");
        };
        assert_eq!(again.assistant.state, MessageState::Completed);
        assert_eq!(again.assistant.content, "answer");
        assert_eq!(again.thread, thread);
        assert_eq!(
            s.create(&create_req("c"), "T", ThreadKind::Api)
                .unwrap()
                .revision,
            1 + 2
        );
    }

    #[test]
    fn old_and_excess_receipts_are_pruned() {
        let (_d, mut s) = open();
        let t = s.create(&create_req("c"), "T", ThreadKind::Api).unwrap();
        for (i, age) in [(1, 0u64), (2, 1000), (3, 10)] {
            s.db.execute(
                "INSERT INTO receipts VALUES(?1,'h',?2,'[]',?3)",
                params![format!("o{i}"), t.thread_id, 10_000 - age as i64],
            )
            .unwrap();
        }
        let count = |s: &Store| -> i64 {
            s.db.query_row("SELECT count(*) FROM receipts", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(count(&s), 4);
        prune_receipts(&s.db, 10_000, 500, 100).unwrap();
        assert_eq!(count(&s), 3); // o2 is older than 500 ms; the create receipt is not.
        s.db.execute("UPDATE receipts SET created_ms=10", [])
            .unwrap();
        s.db.execute(
            "UPDATE receipts SET created_ms=20 WHERE operation_id='o1'",
            [],
        )
        .unwrap();
        prune_receipts(&s.db, 30, u64::MAX, 1).unwrap();
        assert_eq!(count(&s), 1);
    }

    #[test]
    fn context_drops_oldest_turns_instead_of_failing() {
        let mut h: Vec<Turn> = vec![
            ("user", "a".repeat(MAX_CONTEXT / 2), vec![]),
            ("assistant", "b".repeat(MAX_CONTEXT / 2), vec![]),
            ("user", "c".into(), vec![]),
            ("assistant", "d".into(), vec![]),
        ];
        fit_context(&mut h, 10);
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].1, "c");
        // An assistant turn left first is dropped too.
        let mut h: Vec<Turn> = vec![
            ("assistant", "x".to_string(), vec![]),
            ("user", "y".to_string(), vec![]),
        ];
        fit_context(&mut h, 0);
        assert_eq!(h, vec![("user", "y".to_string(), vec![])]);
    }

    #[test]
    fn stuck_rows_can_be_interrupted() {
        let (_d, mut s) = open();
        let t = s.create(&create_req("c"), "T", ThreadKind::Api).unwrap();
        let acc = begin(&mut s, &t, "s1", "q");
        s.flush(&acc.assistant.message_id, "part", 1).unwrap();
        let (thread, msg) = s
            .finish_minimal(&t.thread_id, &acc.assistant.message_id)
            .unwrap();
        assert_eq!(msg.state, MessageState::Interrupted);
        assert_eq!(msg.content, "part");
        assert!(!thread.streaming);
        let acc2 = begin(&mut s, &thread, "s2", "q2");
        assert!(s.thread(&t.thread_id).unwrap().streaming);
        assert!(s.interrupt_inflight(&t.thread_id, true).unwrap());
        assert!(!s.interrupt_inflight(&t.thread_id, true).unwrap());
        let snap = s.snapshot(&t.thread_id).unwrap();
        let m = snap
            .messages
            .iter()
            .find(|m| m.message_id == acc2.assistant.message_id)
            .unwrap();
        assert_eq!(m.state, MessageState::Interrupted);
    }

    #[test]
    fn cli_threads_keep_cwd_and_native_session() {
        let (_d, mut s) = open();
        let mut req = create_req("c");
        req.cwd = Some("/work".into());
        let cli = s.create(&req, "T", ThreadKind::Cli).unwrap();
        assert_eq!(
            (cli.kind, cli.cwd.as_deref()),
            (ThreadKind::Cli, Some("/work"))
        );
        assert_eq!(s.native_session(&cli.thread_id).unwrap(), None);
        s.set_native_session(&cli.thread_id, Some("sess-1"))
            .unwrap();
        assert_eq!(
            s.native_session(&cli.thread_id).unwrap().as_deref(),
            Some("sess-1")
        );
        s.set_native_session(&cli.thread_id, None).unwrap();
        assert_eq!(s.native_session(&cli.thread_id).unwrap(), None);
        s.set_native_session(&cli.thread_id, Some("sess-1"))
            .unwrap();
        // A CLI thread sends like an API thread and drops back to interrupted on restart.
        let acc = begin(&mut s, &cli, "s1", "hi");
        assert_eq!(acc.assistant.state, MessageState::Dispatching);
        // API threads never record a directory.
        req.operation_id = "c2".into();
        assert_eq!(s.create(&req, "T", ThreadKind::Api).unwrap().cwd, None);
    }

    #[test]
    fn list_is_scoped_to_the_workspace_and_keeps_legacy_threads() {
        let (_d, mut s) = open();
        let mut req = |op: &str, w: Option<&str>| {
            let mut r = create_req(op);
            r.workspace_id = w.map(Into::into);
            s.create(&r, "T", ThreadKind::Api).unwrap()
        };
        let a = req("a", Some("w1"));
        let b = req("b", Some("w2"));
        let legacy = req("c", None);
        assert_eq!(a.workspace_id.as_deref(), Some("w1"));
        let ids = |s: &Store, p: &str, w: &str| {
            let mut v: Vec<_> = s
                .list(p, w)
                .unwrap()
                .into_iter()
                .map(|t| t.thread_id)
                .collect();
            v.sort();
            v
        };
        let sorted = |mut v: Vec<String>| {
            v.sort();
            v
        };
        assert_eq!(
            ids(&s, "/p", "w1"),
            sorted(vec![a.thread_id, legacy.thread_id.clone()])
        );
        assert_eq!(
            ids(&s, "/p", "w2"),
            sorted(vec![b.thread_id, legacy.thread_id])
        );
        assert!(ids(&s, "/other", "w1").is_empty());
    }

    #[test]
    fn workspace_is_part_of_the_idempotency_hash() {
        let (_d, mut s) = open();
        s.create(&create_req("h"), "T", ThreadKind::Api).unwrap();
        let mut other = create_req("h");
        other.workspace_id = Some("w2".into());
        assert!(s.create(&other, "T", ThreadKind::Api).is_err());
    }

    #[test]
    fn old_database_gains_cli_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("threads.sqlite");
        {
            let db = Connection::open(&path).unwrap();
            db.execute_batch(&format!(
                "CREATE TABLE threads(id TEXT PRIMARY KEY, project TEXT NOT NULL,
                   kind TEXT NOT NULL CHECK(kind IN('api','cli')), instance_id TEXT NOT NULL,
                   model TEXT NOT NULL, title TEXT NOT NULL, task_id TEXT, revision INTEGER NOT NULL,
                   created_ms INTEGER NOT NULL, updated_ms INTEGER NOT NULL);
                 CREATE TABLE messages(id TEXT PRIMARY KEY,
                   thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
                   seq INTEGER NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL,
                   state TEXT NOT NULL, instance_id TEXT NOT NULL, model TEXT NOT NULL,
                   stream_seq INTEGER NOT NULL DEFAULT 0, stop_reason TEXT, error TEXT,
                   operation_id TEXT, created_ms INTEGER NOT NULL, UNIQUE(thread_id, seq));
                 CREATE TABLE receipts(operation_id TEXT PRIMARY KEY, request_hash TEXT NOT NULL,
                   thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
                   result TEXT NOT NULL, created_ms INTEGER NOT NULL);
                 INSERT INTO threads VALUES('t','/p','api','openai','gpt','Old',NULL,4,1,2);
                 PRAGMA application_id={APPLICATION_ID}; PRAGMA user_version=1;"
            ))
            .unwrap();
        }
        let mut s = Store::open(dir.path()).unwrap();
        let old = s.thread("t").unwrap();
        assert_eq!((old.title.as_str(), old.cwd), ("Old", None));
        assert_eq!(
            (old.effort, old.permissions, old.workspace),
            (None, Permissions::ReadOnly, Workspace::Project)
        );
        // Pre-workspace threads stay visible in every workspace of their project.
        assert_eq!(old.workspace_id, None);
        assert_eq!(s.list("/p", "any").unwrap().len(), 1);
        s.set_native_session("t", Some("x")).unwrap();
        let again = Store::open(dir.path()).unwrap();
        assert_eq!(again.native_session("t").unwrap().as_deref(), Some("x"));
        drop(again);
        let mut req = create_req("c");
        req.cwd = Some("/w".into());
        assert_eq!(
            s.create(&req, "N", ThreadKind::Cli).unwrap().cwd.as_deref(),
            Some("/w")
        );
    }

    #[test]
    fn cli_threads_store_effort_permissions_and_private_folder() {
        let (d, mut s) = open();
        let mut req = create_req("p");
        req.cwd = Some("/project".into());
        req.effort = Some("high".into());
        req.permissions = Permissions::Full;
        req.workspace = Workspace::Private;
        let t = s.create(&req, "T", ThreadKind::Cli).unwrap();
        let private = d.path().join("workspaces").join(&t.thread_id);
        assert!(private.is_dir());
        assert_eq!(t.cwd.as_deref(), private.to_str());
        assert_eq!(t.effort.as_deref(), Some("high"));
        assert_eq!(
            (t.permissions, t.workspace),
            (Permissions::Full, Workspace::Private)
        );
        assert_eq!(s.thread(&t.thread_id).unwrap(), t);
        // API threads ignore all of it.
        req.operation_id = "p2".into();
        let api = s.create(&req, "T", ThreadKind::Api).unwrap();
        assert_eq!(
            (api.cwd, api.effort, api.permissions, api.workspace),
            (None, None, Permissions::ReadOnly, Workspace::Project)
        );
    }

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n0000";

    fn count(s: &Store, table: &str) -> i64 {
        s.db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn attachment_lifecycle_binds_once() {
        let (d, mut s) = open();
        let t = s.create(&create_req("c"), "T", ThreadKind::Api).unwrap();
        let img = s.attachment_import("/x/../pic.png", PNG).unwrap();
        let txt = s.attachment_import("n.txt", b"hello").unwrap();
        assert_eq!((img.name.as_str(), img.size), ("pic.png", PNG.len() as u64));
        assert_eq!(img.id.len(), 32);
        assert!(s.attachment_import("x", b"\0\x01").is_err());
        let file = d.path().join("attachments").join(format!("{}.png", img.id));
        assert_eq!(std::fs::read(&file).unwrap(), PNG);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        // Discard removes an unsent file and its row.
        let gone = s.attachment_import("g.txt", b"g").unwrap();
        s.attachment_discard(&gone.id).unwrap();
        assert!(!d
            .path()
            .join("attachments")
            .join(format!("{}.txt", gone.id))
            .exists());
        assert!(s.attachment_discard(&gone.id).is_err());
        // Send with attachments only (empty text); unknown ids are refused.
        let mut req = send_req("s1", &t, "");
        req.attachments = vec!["nope".into()];
        assert!(s.send_begin(&req, 0, |_| Ok(())).is_err());
        req.attachments = vec![img.id.clone(), txt.id.clone()];
        let SendBegin::New(acc, ctx) = s.send_begin(&req, 0, |_| Ok(())).unwrap() else {
            panic!()
        };
        assert_eq!(acc.user.attachments, vec![img.clone(), txt.clone()]);
        assert_eq!(ctx[0].2.len(), 2);
        assert_eq!(
            s.snapshot(&t.thread_id).unwrap().messages[0]
                .attachments
                .len(),
            2
        );
        // The hash covers attachment ids, so a different set cannot reuse the operation.
        let mut other = req.clone();
        other.attachments.pop();
        assert!(matches!(s.send_begin(&other, 0, |_| Ok(())), Err(e) if e == REUSED));
        // A sent attachment can be neither discarded nor bound again.
        assert!(s
            .attachment_discard(&img.id)
            .unwrap_err()
            .contains("already sent"));
        let thread = s
            .interrupt_inflight(&t.thread_id, true)
            .map(|_| s.thread(&t.thread_id).unwrap())
            .unwrap();
        let mut again = send_req("s2", &thread, "x");
        again.attachments = vec![img.id.clone()];
        assert!(s.send_begin(&again, 0, |_| Ok(())).is_err());
        // Limits: count and total size.
        let many: Vec<String> = (0..11)
            .map(|i| s.attachment_import(&format!("{i}.txt"), b"x").unwrap().id)
            .collect();
        again.attachments = many;
        assert!(s
            .send_begin(&again, 0, |_| Ok(()))
            .unwrap_err()
            .contains("at most 10"));
        let big = s
            .attachment_import(
                "b.png",
                &[PNG, &vec![0; attachments::MAX_IMAGE - PNG.len()]].concat(),
            )
            .unwrap();
        let ids: Vec<String> = std::iter::once(big.id)
            .chain((0..4).map(|i| {
                let mut d = PNG.to_vec();
                d.resize(attachments::MAX_IMAGE, i);
                s.attachment_import("c.png", &d).unwrap().id
            }))
            .collect();
        again.attachments = ids;
        assert!(s.send_begin(&again, 0, |_| Ok(())).is_err());
        // Deleting the thread removes bound files and rows.
        let thread = s.thread(&t.thread_id).unwrap();
        s.delete(&ThreadDelete {
            thread_id: t.thread_id.clone(),
            expected_revision: thread.revision,
        })
        .unwrap();
        assert!(!file.exists());
        assert!(!d
            .path()
            .join("attachments")
            .join(format!("{}.txt", txt.id))
            .exists());
        assert_eq!(count(&s, "attachments"), 16); // only the unsent ones remain
    }

    #[test]
    fn unsent_attachments_are_pruned_after_a_day() {
        let dir = tempfile::tempdir().unwrap();
        let (old, fresh) = {
            let s = Store::open(dir.path()).unwrap();
            let old = s.attachment_import("o.txt", b"o").unwrap();
            let fresh = s.attachment_import("f.txt", b"f").unwrap();
            let day = (UNSENT_MAX_AGE_MS + 1000) as i64;
            s.db.execute(
                "UPDATE attachments SET created_ms=created_ms-?2 WHERE id=?1",
                params![old.id, day],
            )
            .unwrap();
            (old, fresh)
        };
        let s = Store::open(dir.path()).unwrap();
        assert!(s.attachment(&old.id).is_err());
        assert!(!dir
            .path()
            .join("attachments")
            .join(format!("{}.txt", old.id))
            .exists());
        assert!(s.attachment(&fresh.id).is_ok());
        assert!(dir
            .path()
            .join("attachments")
            .join(format!("{}.txt", fresh.id))
            .exists());
    }

    #[test]
    fn old_database_gains_attachments_table() {
        let dir = tempfile::tempdir().unwrap();
        {
            let s = Store::open(dir.path()).unwrap();
            s.db.execute_batch("DROP TABLE attachments;").unwrap();
        }
        let mut s = Store::open(dir.path()).unwrap();
        assert_eq!(count(&s, "attachments"), 0);
        let t = s.create(&create_req("c"), "T", ThreadKind::Api).unwrap();
        let a = s.attachment_import("n.txt", b"x").unwrap();
        let mut req = send_req("s", &t, "hi");
        req.attachments = vec![a.id.clone()];
        s.send_begin(&req, 0, |_| Ok(())).unwrap();
        assert_eq!(
            s.snapshot(&t.thread_id).unwrap().messages[0].attachments,
            vec![a]
        );
    }

    #[cfg(unix)]
    #[test]
    fn foreign_database_is_left_untouched() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("threads.sqlite");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE x(a); PRAGMA user_version=7;")
            .unwrap();
        drop(db);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(Store::open(dir.path()).unwrap_err().contains("preserved"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644);
        let db = Connection::open(&path).unwrap();
        let journal: String = db
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(journal, "delete");
    }
}
