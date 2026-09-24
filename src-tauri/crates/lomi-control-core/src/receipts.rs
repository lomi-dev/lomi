//! Durable reservations precede dispatch. This store never replays a command and
//! does not grant access: the broker must authorize admission, dispatch and reads.
use std::{fs, io, path::Path, time::Duration};

use lomi_control_protocol::control::OperationResult;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const DAY: i64 = 24 * 60 * 60;
const MAX_RECEIPTS: i64 = 4096;
const MAX_INPUT_BYTES: usize = 64 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InvalidInput,
    StorageUnavailable,
    RetryWindowExpired,
    IdempotencyConflict,
    ResourceExhausted,
    TargetNotFound,
    InvalidTransition,
}

impl From<rusqlite::Error> for Error {
    fn from(_: rusqlite::Error) -> Self {
        Self::StorageUnavailable
    }
}
impl From<io::Error> for Error {
    fn from(_: io::Error) -> Self {
        Self::StorageUnavailable
    }
}
type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Queued,
    AwaitingUser,
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
    OutcomeUnknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    None,
    Partial,
    Complete,
    Unknown,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::AwaitingUser => "awaiting_user",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::OutcomeUnknown => "outcome_unknown",
        }
    }
    fn terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::OutcomeUnknown
        )
    }
    fn permits(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Queued,
                Self::AwaitingUser | Self::Running | Self::Cancelled | Self::Failed
            ) | (
                Self::AwaitingUser,
                Self::Queued | Self::Cancelled | Self::Failed
            ) | (
                Self::Running,
                Self::Succeeded | Self::Failed | Self::Cancelling | Self::OutcomeUnknown
            ) | (
                Self::Cancelling,
                Self::Cancelled | Self::Succeeded | Self::Failed | Self::OutcomeUnknown
            )
        )
    }
}
impl Effect {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Partial => "partial",
            Self::Complete => "complete",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Receipt {
    pub operation_id: String,
    pub state: State,
    pub effect_state: Effect,
    pub workspace_id: Option<String>,
    pub result: Option<OperationResult>,
}

/// All fields come from authenticated resolution, not from focus or client hints.
#[derive(Debug, Serialize)]
pub struct Target<'a> {
    pub workspace_id: &'a str,
    pub resource_id: &'a str,
    pub generation: &'a str,
    pub revision: &'a str,
}

pub struct Key<'a> {
    pub pairing_id: &'a str,
    pub retry_epoch: &'a str,
    pub project_id: &'a str,
    pub tool: &'a str,
    pub request_key: &'a str,
}

#[derive(Debug)]
pub struct Reservation {
    pub receipt: Receipt,
    pub created: bool,
}

fn identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.:".contains(&c))
    {
        return Err(Error::InvalidInput);
    }
    Ok(())
}

fn random_id() -> Result<String> {
    let mut bytes = [0; 20];
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(&mut bytes)
        .map_err(|_| Error::StorageUnavailable)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Values must already have passed their closed, domain-specific DTO validator.
/// Canonical object ordering keeps retries independent of JSON key order.
pub fn fingerprint<T: Serialize>(input: &T, target: &Target<'_>) -> Result<[u8; 32]> {
    for value in [target.workspace_id, target.resource_id, target.generation] {
        identifier(value)?;
    }
    if target.revision.len() > 128 || target.revision.contains('\0') {
        return Err(Error::InvalidInput);
    }
    struct Bounded(Vec<u8>);
    impl io::Write for Bounded {
        fn write(&mut self, value: &[u8]) -> io::Result<usize> {
            if value.len() > MAX_INPUT_BYTES.saturating_sub(self.0.len()) {
                return Err(io::Error::other("Input too large"));
            }
            self.0.extend_from_slice(value);
            Ok(value.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut bytes = Bounded(Vec::new());
    serde_json::to_writer(&mut bytes, &(input, target)).map_err(|_| Error::InvalidInput)?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes.0).map_err(|_| Error::InvalidInput)?;
    fn canonical(value: serde_json::Value, depth: usize) -> Result<serde_json::Value> {
        if depth > 32 {
            return Err(Error::InvalidInput);
        }
        Ok(match value {
            serde_json::Value::Object(map) => {
                let sorted: std::collections::BTreeMap<_, _> = map.into_iter().collect();
                let mut out = serde_json::Map::new();
                for (key, value) in sorted {
                    if key.contains('\0') {
                        return Err(Error::InvalidInput);
                    }
                    out.insert(key, canonical(value, depth + 1)?);
                }
                serde_json::Value::Object(out)
            }
            serde_json::Value::Array(values) => serde_json::Value::Array(
                values
                    .into_iter()
                    .map(|v| canonical(v, depth + 1))
                    .collect::<Result<_>>()?,
            ),
            serde_json::Value::String(ref text) if text.contains('\0') => {
                return Err(Error::InvalidInput);
            }
            value => value,
        })
    }
    bytes.0.clear();
    serde_json::to_writer(&mut bytes, &canonical(value, 0)?).map_err(|_| Error::InvalidInput)?;
    Ok(Sha256::digest(bytes.0).into())
}

pub struct Store {
    pub(crate) connection: Connection,
    pub(crate) root: std::path::PathBuf,
    instance_id: String,
    pub(crate) artifact_pins: std::collections::HashMap<String, std::sync::Weak<()>>,
    // Kept until after the SQLite connection is dropped.
    _owner: OwnerLock,
}

struct OwnerLock(fs::File);
impl Drop for OwnerLock {
    fn drop(&mut self) {
        // Release explicitly after SQLite closes, even if a concurrently spawned
        // child briefly inherited the open file description before exec.
        let _ = self.0.unlock();
    }
}

pub(crate) fn private(path: &Path, directory: bool) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = path.symlink_metadata()?;
    if metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != if directory { 0o700 } else { 0o600 }
        || if directory {
            !metadata.is_dir()
        } else {
            !metadata.is_file() || metadata.nlink() != 1
        }
    {
        return Err(Error::StorageUnavailable);
    }
    Ok(())
}

impl Store {
    pub(crate) fn trash_recovery_root(
        &self,
    ) -> std::result::Result<std::path::PathBuf, lomi_control_protocol::ErrorCode> {
        use lomi_control_protocol::ErrorCode;
        use std::os::unix::fs::DirBuilderExt;
        private(&self.root, true).map_err(|_| ErrorCode::StorageUnavailable)?;
        let path = self.root.join("trash-recovery");
        match fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => {
                fs::File::open(&self.root)
                    .and_then(|file| file.sync_all())
                    .map_err(|_| ErrorCode::StorageUnavailable)?;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(ErrorCode::StorageUnavailable),
        }
        private(&path, true).map_err(|_| ErrorCode::StorageUnavailable)?;
        // Never prune unresolved recovery data to satisfy the quota.
        let count = fs::read_dir(&path)
            .map_err(|_| ErrorCode::StorageUnavailable)?
            .take(129)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| ErrorCode::StorageUnavailable)?
            .len();
        if count >= 128 {
            return Err(ErrorCode::ResourceExhausted);
        }
        path.canonicalize()
            .map_err(|_| ErrorCode::StorageUnavailable)
    }
    /// The caller supplies its application-owned parent, never a client path.
    /// Windows must implement and qualify its ACL policy before exposing this API.
    pub fn open(root: &Path, now: i64) -> Result<Self> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        if now < 0 {
            return Err(Error::InvalidInput);
        }
        match fs::DirBuilder::new().mode(0o700).create(root) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error.into()),
        }
        private(root, true)?;
        let canonical_root = root.canonicalize()?;
        let root = canonical_root.as_path();
        let owner = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(root.join("owner.lock"))?;
        private(&root.join("owner.lock"), false)?;
        owner.try_lock().map_err(|_| Error::StorageUnavailable)?;
        let path = root.join("control.sqlite3");
        for name in [
            "control.sqlite3",
            "control.sqlite3-wal",
            "control.sqlite3-shm",
        ] {
            let p = root.join(name);
            match p.symlink_metadata() {
                Ok(_) => private(&p, false)?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                Err(error) => return Err(error.into()),
            }
        }
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)?;
        private(&path, false)?;
        file.sync_all()?;
        drop(file);
        let mut connection = Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        connection.busy_timeout(Duration::from_secs(2))?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > 5 {
            return Err(Error::StorageUnavailable);
        }
        connection.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA fullfsync=ON; PRAGMA checkpoint_fullfsync=ON; PRAGMA max_page_count=16384;")?;
        if version == 0 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch("CREATE TABLE epochs(id TEXT PRIMARY KEY, pairing TEXT NOT NULL, instance TEXT NOT NULL, expires INTEGER NOT NULL);
                CREATE TABLE receipts(id TEXT PRIMARY KEY, pairing TEXT NOT NULL, epoch TEXT NOT NULL REFERENCES epochs(id), project TEXT NOT NULL, tool TEXT NOT NULL, request_key TEXT NOT NULL, fingerprint BLOB NOT NULL CHECK(length(fingerprint)=32), state TEXT NOT NULL, effect TEXT NOT NULL, completed INTEGER, retain_until INTEGER NOT NULL, UNIQUE(pairing,epoch,project,tool,request_key));
                CREATE TABLE history(operation TEXT NOT NULL REFERENCES receipts(id), state TEXT NOT NULL, effect TEXT NOT NULL, at INTEGER NOT NULL);
                PRAGMA user_version=1;")?;
            transaction.commit()?;
        }
        if version < 2 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch("ALTER TABLE receipts ADD COLUMN workspace TEXT; ALTER TABLE receipts ADD COLUMN result TEXT; PRAGMA user_version=2;")?;
            transaction.commit()?;
        }
        if version < 3 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch("CREATE TABLE artifacts(id TEXT PRIMARY KEY, pairing TEXT NOT NULL, project TEXT NOT NULL, reserved_bytes INTEGER NOT NULL, state TEXT NOT NULL CHECK(state IN ('reserved','ready')), expires INTEGER NOT NULL, metadata TEXT); PRAGMA user_version=3;")?;
            transaction.commit()?;
        }
        if version < 4 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch("ALTER TABLE artifacts ADD COLUMN class TEXT NOT NULL DEFAULT 'image' CHECK(class IN ('image','apk')); PRAGMA user_version=4;")?;
            transaction.commit()?;
        }
        if version < 5 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch("CREATE TABLE artifacts_v5(id TEXT PRIMARY KEY, pairing TEXT NOT NULL, project TEXT NOT NULL, reserved_bytes INTEGER NOT NULL, state TEXT NOT NULL CHECK(state IN ('reserved','ready')), expires INTEGER NOT NULL, metadata TEXT, class TEXT NOT NULL DEFAULT 'image' CHECK(class IN ('image','apk','file')));
                INSERT INTO artifacts_v5 SELECT id,pairing,project,reserved_bytes,state,expires,metadata,class FROM artifacts;
                DROP TABLE artifacts; ALTER TABLE artifacts_v5 RENAME TO artifacts;
                PRAGMA user_version=5;")?;
            transaction.commit()?;
        }
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute("INSERT INTO history SELECT id,CASE WHEN state IN ('queued','awaiting_user') THEN 'cancelled' ELSE 'outcome_unknown' END,CASE WHEN state IN ('queued','awaiting_user') THEN 'none' ELSE 'unknown' END,?1 FROM receipts WHERE state IN ('queued','awaiting_user','running','cancelling')", [now])?;
        transaction.execute("UPDATE receipts SET state='cancelled',effect='none',completed=?1,retain_until=MAX(retain_until,?2) WHERE state IN ('queued','awaiting_user')", params![now, now.checked_add(DAY).ok_or(Error::InvalidInput)?])?;
        transaction.execute("UPDATE receipts SET state='outcome_unknown',effect='unknown',completed=?1,retain_until=MAX(retain_until,?2) WHERE state IN ('running','cancelling')", params![now, now + DAY])?;
        // Expired instances keep their receipts, but no retry epoch authorizes dispatch.
        transaction.commit()?;
        for name in [
            "control.sqlite3",
            "control.sqlite3-wal",
            "control.sqlite3-shm",
        ] {
            private(&root.join(name), false)?;
        }
        fs::File::open(root)?.sync_all()?;
        let mut store = Self {
            connection,
            root: root.to_path_buf(),
            instance_id: random_id()?,
            artifact_pins: Default::default(),
            _owner: OwnerLock(owner),
        };
        store.recover_artifacts(now)?;
        Ok(store)
    }

    pub fn issue_epoch(&mut self, pairing: &str, now: i64) -> Result<String> {
        identifier(pairing)?;
        let expires = now
            .checked_add(DAY)
            .filter(|_| now >= 0)
            .ok_or(Error::InvalidInput)?;
        let count: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM epochs", [], |row| row.get(0))?;
        if count >= MAX_RECEIPTS {
            return Err(Error::ResourceExhausted);
        }
        let id = random_id()?;
        self.connection.execute(
            "INSERT INTO epochs VALUES(?1,?2,?3,?4)",
            params![id, pairing, self.instance_id, expires],
        )?;
        Ok(id)
    }

    pub fn existing(&self, key: &Key<'_>, now: i64) -> Result<Option<(Receipt, Vec<u8>)>> {
        for value in [
            key.pairing_id,
            key.retry_epoch,
            key.project_id,
            key.tool,
            key.request_key,
        ] {
            identifier(value)?;
        }
        if now < 0 {
            return Err(Error::InvalidInput);
        }
        let valid:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM epochs WHERE id=?1 AND pairing=?2 AND instance=?3 AND expires>?4)",params![key.retry_epoch,key.pairing_id,self.instance_id,now],|row|row.get(0))?;
        if !valid {
            return Err(Error::RetryWindowExpired);
        }
        self.connection.query_row("SELECT id,state,effect,workspace,result,fingerprint FROM receipts WHERE pairing=?1 AND epoch=?2 AND project=?3 AND tool=?4 AND request_key=?5",params![key.pairing_id,key.retry_epoch,key.project_id,key.tool,key.request_key],|row|Ok((read_receipt(row)?,row.get(5)?))).optional().map_err(Into::into)
    }
    pub fn reserve(
        &mut self,
        key: &Key<'_>,
        fingerprint: [u8; 32],
        now: i64,
    ) -> Result<Reservation> {
        for value in [
            key.pairing_id,
            key.retry_epoch,
            key.project_id,
            key.tool,
            key.request_key,
        ] {
            identifier(value)?;
        }
        if now < 0 {
            return Err(Error::InvalidInput);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let expires: Option<i64> = tx.query_row("SELECT expires FROM epochs WHERE id=?1 AND pairing=?2 AND instance=?3 AND expires>?4", params![key.retry_epoch,key.pairing_id,self.instance_id,now], |row| row.get(0)).optional()?;
        let expires = expires.ok_or(Error::RetryWindowExpired)?;
        let existing = tx.query_row("SELECT id,state,effect,workspace,result,fingerprint FROM receipts WHERE pairing=?1 AND epoch=?2 AND project=?3 AND tool=?4 AND request_key=?5", params![key.pairing_id,key.retry_epoch,key.project_id,key.tool,key.request_key], |row| Ok((read_receipt(row)?, row.get::<_,Vec<u8>>(5)?))).optional()?;
        if let Some((receipt, hash)) = existing {
            if hash != fingerprint {
                return Err(Error::IdempotencyConflict);
            }
            return Ok(Reservation {
                receipt,
                created: false,
            });
        }
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM receipts", [], |row| row.get(0))?;
        if count >= MAX_RECEIPTS {
            return Err(Error::ResourceExhausted);
        }
        let id = random_id()?;
        tx.execute(
            "INSERT INTO receipts(id,pairing,epoch,project,tool,request_key,fingerprint,state,effect,completed,retain_until) VALUES(?1,?2,?3,?4,?5,?6,?7,'queued','none',NULL,?8)",
            params![
                id,
                key.pairing_id,
                key.retry_epoch,
                key.project_id,
                key.tool,
                key.request_key,
                fingerprint.as_slice(),
                expires
            ],
        )?;
        tx.execute(
            "INSERT INTO history VALUES(?1,'queued','none',?2)",
            params![id, now],
        )?;
        tx.commit()?;
        Ok(Reservation {
            receipt: Receipt {
                operation_id: id,
                state: State::Queued,
                effect_state: Effect::None,
                workspace_id: None,
                result: None,
            },
            created: true,
        })
    }

    /// Both the authenticated pairing and current authorized project are required.
    /// An operation ID alone cannot disclose another subject's receipt.
    pub fn get(&self, pairing: &str, project: &str, id: &str) -> Result<Receipt> {
        self.connection
            .query_row(
                "SELECT id,state,effect,workspace,result FROM receipts WHERE pairing=?1 AND project=?2 AND id=?3",
                params![pairing, project, id],
                read_receipt,
            )
            .optional()?
            .ok_or(Error::TargetNotFound)
    }
    pub fn lookup(&self, key: &Key<'_>) -> Result<Receipt> {
        self.connection.query_row("SELECT id,state,effect,workspace,result FROM receipts WHERE pairing=?1 AND project=?2 AND epoch=?3 AND tool=?4 AND request_key=?5",params![key.pairing_id,key.project_id,key.retry_epoch,key.tool,key.request_key],read_receipt).optional()?.ok_or(Error::TargetNotFound)
    }

    pub fn bind_workspace(
        &mut self,
        pairing: &str,
        project: &str,
        id: &str,
        workspace: &str,
    ) -> Result<()> {
        identifier(workspace)?;
        let changed=self.connection.execute("UPDATE receipts SET workspace=?1 WHERE pairing=?2 AND project=?3 AND id=?4 AND state='queued' AND workspace IS NULL",params![workspace,pairing,project,id])?;
        if changed != 1 {
            return Err(Error::InvalidTransition);
        }
        Ok(())
    }
    pub fn events(
        &self,
        pairing: &str,
        project: &str,
        workspace: &str,
        offset: usize,
        limit: u16,
    ) -> Result<Vec<lomi_control_protocol::control::ControlEvent>> {
        identifier(pairing)?;
        identifier(project)?;
        identifier(workspace)?;
        if limit == 0 || limit > 501 || i64::try_from(offset).is_err() {
            return Err(Error::InvalidInput);
        }
        let mut query=self.connection.prepare("SELECT h.operation,h.state,h.effect,h.at,r.tool FROM history h JOIN receipts r ON r.id=h.operation WHERE r.pairing=?1 AND r.project=?2 AND r.workspace=?3 ORDER BY h.rowid LIMIT ?4 OFFSET ?5")?;
        let rows = query.query_map(
            params![pairing, project, workspace, limit, offset as i64],
            |row| {
                Ok(lomi_control_protocol::control::ControlEvent {
                    sequence: String::new(),
                    kind: "operation_state".into(),
                    operation_id: row.get(0)?,
                    workspace_id: workspace.into(),
                    state: row.get(1)?,
                    effect_state: row.get(2)?,
                    timestamp_seconds: row.get::<_, i64>(3)?.to_string(),
                    tool: row.get(4)?,
                })
            },
        )?;
        rows.enumerate()
            .map(|(index, row)| {
                let mut event = row?;
                event.sequence = (offset + index + 1).to_string();
                Ok(event)
            })
            .collect()
    }
    /// A result is durable before final success is reported. A crash between
    /// this write and the terminal transition remains outcome_unknown.
    pub fn record_result(
        &mut self,
        pairing: &str,
        project: &str,
        id: &str,
        result: &OperationResult,
    ) -> Result<()> {
        let value = serde_json::to_string(result).map_err(|_| Error::InvalidInput)?;
        if value.len() > MAX_INPUT_BYTES {
            return Err(Error::ResourceExhausted);
        }
        let changed=self.connection.execute("UPDATE receipts SET result=?1 WHERE pairing=?2 AND project=?3 AND id=?4 AND state IN ('queued','running','cancelling') AND result IS NULL",params![value,pairing,project,id])?;
        if changed != 1 {
            return Err(Error::InvalidTransition);
        }
        Ok(())
    }

    /// Replace a reserved send identity once, preserving every target and ID.
    pub(crate) fn finish_chat_send(
        &mut self,
        pairing: &str,
        project: &str,
        id: &str,
        result: &lomi_control_protocol::chat::ChatSent,
    ) -> Result<()> {
        if result.draft_revision.is_some() == result.rejection.is_some() {
            return Err(Error::InvalidInput);
        }
        let mut reserved = result.clone();
        reserved.draft_revision = None;
        reserved.rejection = None;
        let expected = serde_json::to_string(&OperationResult::ChatSent(Box::new(reserved)))
            .map_err(|_| Error::InvalidInput)?;
        let value = serde_json::to_string(&OperationResult::ChatSent(Box::new(result.clone())))
            .map_err(|_| Error::InvalidInput)?;
        if value.len() > MAX_INPUT_BYTES {
            return Err(Error::ResourceExhausted);
        }
        if self.connection.execute("UPDATE receipts SET result=?1 WHERE pairing=?2 AND project=?3 AND id=?4 AND result=?5 AND state IN ('queued','running','awaiting_user','cancelling')", params![value, pairing, project, id, expected])? != 1 {
            return Err(Error::InvalidTransition);
        }
        Ok(())
    }

    /// Advance only an immutable project lifecycle preparation to verified publication.
    /// Other result types keep the one-write rule in record_result.
    pub(crate) fn finish_project_lifecycle(
        &mut self,
        pairing: &str,
        project: &str,
        id: &str,
        result: &OperationResult,
        now: i64,
    ) -> Result<()> {
        if now < 0 || now.checked_add(DAY).is_none() {
            return Err(Error::InvalidInput);
        }
        let mut prepared = result.clone();
        let workspace = match &mut prepared {
            OperationResult::ProjectOpened(p)
                if p.opened == Some(true) && p.project_id != project =>
            {
                p.opened = None;
                p.anchor_workspace_id.clone()
            }
            OperationResult::WorkspaceClosure(p)
                if p.closed == Some(true)
                    && p.project_closed.is_some()
                    && p.project_id == project =>
            {
                p.closed = None;
                p.project_closed = None;
                p.workspace_id.clone()
            }
            OperationResult::ProjectClosure(p)
                if p.closed == Some(true)
                    && p.project_id == project
                    && !p.workspace_ids.is_empty()
                    && p.workspace_ids.contains(&p.workspace_id) =>
            {
                p.closed = None;
                p.workspace_id.clone()
            }
            _ => return Err(Error::InvalidInput),
        };
        let expected = serde_json::to_string(&prepared).map_err(|_| Error::InvalidInput)?;
        let value = serde_json::to_string(result).map_err(|_| Error::InvalidInput)?;
        if value.len() > MAX_INPUT_BYTES {
            return Err(Error::ResourceExhausted);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute("UPDATE receipts SET result=?1,state='succeeded',effect='complete',completed=?2,retain_until=MAX(retain_until,?3) WHERE pairing=?4 AND project=?5 AND id=?6 AND workspace=?7 AND result=?8 AND state IN ('running','cancelling')", params![value,now,now+DAY,pairing,project,id,workspace,expected])?;
        if changed != 1 {
            return Err(Error::InvalidTransition);
        }
        tx.execute(
            "INSERT INTO history VALUES(?1,'succeeded','complete',?2)",
            params![id, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The broker rechecks grant, deadline, generation and lease before Running.
    /// Only a successful durable Running transition permits domain dispatch.
    pub fn transition(
        &mut self,
        pairing: &str,
        project: &str,
        id: &str,
        next: State,
        effect: Effect,
        now: i64,
    ) -> Result<Receipt> {
        if now < 0 || now.checked_add(DAY).is_none() {
            return Err(Error::InvalidInput);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = tx
            .query_row(
                "SELECT id,state,effect,workspace,result FROM receipts WHERE pairing=?1 AND project=?2 AND id=?3",
                params![pairing, project, id],
                read_receipt,
            )
            .optional()?
            .ok_or(Error::TargetNotFound)?;
        if !current.state.permits(next)
            || (matches!(current.state, State::Queued | State::AwaitingUser)
                && effect != Effect::None)
            || (next == State::OutcomeUnknown && effect != Effect::Unknown)
            || (next == State::Succeeded && effect != Effect::Complete)
        {
            return Err(Error::InvalidTransition);
        }
        if next == State::Running {
            let valid: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM receipts r JOIN epochs e ON e.id=r.epoch WHERE r.id=?1 AND e.instance=?2 AND e.expires>?3)", params![id,self.instance_id,now], |row| row.get(0))?;
            if !valid {
                return Err(Error::RetryWindowExpired);
            }
        }
        let completed = next.terminal().then_some(now);
        tx.execute("UPDATE receipts SET state=?1,effect=?2,completed=?3,retain_until=MAX(retain_until,?4) WHERE id=?5", params![next.as_str(),effect.as_str(),completed,completed.map(|at|at+DAY).unwrap_or(0),id])?;
        tx.execute(
            "INSERT INTO history VALUES(?1,?2,?3,?4)",
            params![id, next.as_str(), effect.as_str(), now],
        )?;
        tx.commit()?;
        Ok(Receipt {
            operation_id: id.into(),
            state: next,
            effect_state: effect,
            ..current
        })
    }
}

fn read_receipt(row: &rusqlite::Row<'_>) -> rusqlite::Result<Receipt> {
    fn enum_value<T: serde::de::DeserializeOwned>(
        row: &rusqlite::Row<'_>,
        index: usize,
    ) -> rusqlite::Result<T> {
        serde_json::from_value(serde_json::Value::String(row.get(index)?)).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                index,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
    }
    Ok(Receipt {
        operation_id: row.get(0)?,
        state: enum_value(row, 1)?,
        effect_state: enum_value(row, 2)?,
        workspace_id: row.get(3)?,
        result: row
            .get::<_, Option<String>>(4)?
            .map(|value| {
                serde_json::from_str(&value).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        4,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })
            })
            .transpose()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn key(epoch: &str) -> Key<'_> {
        Key {
            pairing_id: "paired-client",
            retry_epoch: epoch,
            project_id: "project-one",
            tool: "lomi_terminal_run",
            request_key: "once",
        }
    }
    fn target() -> Target<'static> {
        Target {
            workspace_id: "workspace-one",
            resource_id: "terminal-one",
            generation: "generation-one",
            revision: "1",
        }
    }

    #[test]
    fn releasing_store_does_not_leave_its_lock_on_an_inherited_descriptor() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("control");
        let store = Store::open(&root, 100).unwrap();
        let inherited = store._owner.0.try_clone().unwrap();
        assert!(Store::open(&root, 100).is_err());
        drop(store);
        let reopened = Store::open(&root, 101).unwrap();
        drop(inherited);
        assert!(
            Store::open(&root, 101).is_err(),
            "Closing the old descriptor must not unlock the new owner"
        );
        drop(reopened);
        Store::open(&root, 102).unwrap();
    }

    #[test]
    fn canonical_arguments_bind_the_resolved_generation_and_obey_limits() {
        let a = serde_json::from_str::<serde_json::Value>(r#"{"z":2,"a":{"y":1,"b":0}}"#).unwrap();
        let b = serde_json::from_str::<serde_json::Value>(r#"{"a":{"b":0,"y":1},"z":2}"#).unwrap();
        assert_eq!(fingerprint(&a, &target()), fingerprint(&b, &target()));
        let mut newer = target();
        newer.generation = "generation-two";
        assert_ne!(fingerprint(&a, &target()), fingerprint(&a, &newer));
        assert_eq!(
            fingerprint(&"x".repeat(MAX_INPUT_BYTES), &target()),
            Err(Error::InvalidInput)
        );
        assert_eq!(
            fingerprint(&"hidden\0input", &target()),
            Err(Error::InvalidInput)
        );
    }

    #[test]
    fn concurrent_retries_reserve_one_receipt_and_changed_input_conflicts() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(&temp.path().join("control"), 100).unwrap();
        let epoch = store.issue_epoch("paired-client", 100).unwrap();
        let shared = Arc::new(Mutex::new(store));
        let workers: Vec<_> = (0..16)
            .map(|_| {
                let store = shared.clone();
                let epoch = epoch.clone();
                std::thread::spawn(move || {
                    store
                        .lock()
                        .unwrap()
                        .reserve(&key(&epoch), [1; 32], 101)
                        .unwrap()
                })
            })
            .collect();
        let reservations: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        assert_eq!(reservations.iter().filter(|r| r.created).count(), 1);
        let id = &reservations[0].receipt.operation_id;
        assert!(reservations.iter().all(|r| r.receipt.operation_id == *id));
        let mut store = shared.lock().unwrap();
        assert_eq!(
            store.reserve(&key(&epoch), [2; 32], 101).unwrap_err(),
            Error::IdempotencyConflict
        );
        assert_eq!(
            store.get("other-client", "project-one", id).unwrap_err(),
            Error::TargetNotFound
        );
        assert_eq!(
            store.get("paired-client", "project-two", id).unwrap_err(),
            Error::TargetNotFound
        );
        store
            .transition(
                "paired-client",
                "project-one",
                id,
                State::Running,
                Effect::None,
                102,
            )
            .unwrap();
        assert_eq!(
            store
                .transition(
                    "paired-client",
                    "project-one",
                    id,
                    State::Running,
                    Effect::None,
                    102
                )
                .unwrap_err(),
            Error::InvalidTransition
        );
        assert_eq!(
            store.reserve(&key(&epoch), [1; 32], 100 + DAY).unwrap_err(),
            Error::RetryWindowExpired
        );
    }

    #[test]
    fn failed_durable_write_never_produces_a_dispatchable_receipt() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(&temp.path().join("control"), 100).unwrap();
        let epoch = store.issue_epoch("paired-client", 100).unwrap();
        store
            .connection
            .execute_batch("PRAGMA query_only=ON")
            .unwrap();
        assert_eq!(
            store.reserve(&key(&epoch), [1; 32], 101).unwrap_err(),
            Error::StorageUnavailable
        );
        store
            .connection
            .execute_batch("PRAGMA query_only=OFF")
            .unwrap();
        let reservation = store.reserve(&key(&epoch), [1; 32], 101).unwrap();
        assert!(reservation.created);
        store
            .connection
            .execute_batch("PRAGMA query_only=ON")
            .unwrap();
        assert_eq!(
            store
                .transition(
                    "paired-client",
                    "project-one",
                    &reservation.receipt.operation_id,
                    State::Running,
                    Effect::None,
                    102
                )
                .unwrap_err(),
            Error::StorageUnavailable
        );
        assert_eq!(
            store
                .get(
                    "paired-client",
                    "project-one",
                    &reservation.receipt.operation_id
                )
                .unwrap()
                .state,
            State::Queued
        );
    }

    #[test]
    fn cancellation_does_not_overwrite_a_completed_effect() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(&temp.path().join("control"), 100).unwrap();
        let epoch = store.issue_epoch("paired-client", 100).unwrap();
        let id = store
            .reserve(&key(&epoch), [1; 32], 101)
            .unwrap()
            .receipt
            .operation_id;
        store
            .transition(
                "paired-client",
                "project-one",
                &id,
                State::Running,
                Effect::None,
                102,
            )
            .unwrap();
        store
            .transition(
                "paired-client",
                "project-one",
                &id,
                State::Cancelling,
                Effect::Partial,
                103,
            )
            .unwrap();
        store
            .transition(
                "paired-client",
                "project-one",
                &id,
                State::Succeeded,
                Effect::Complete,
                104,
            )
            .unwrap();
        assert_eq!(
            store
                .transition(
                    "paired-client",
                    "project-one",
                    &id,
                    State::Cancelled,
                    Effect::None,
                    105
                )
                .unwrap_err(),
            Error::InvalidTransition
        );
        assert_eq!(
            store
                .get("paired-client", "project-one", &id)
                .unwrap()
                .effect_state,
            Effect::Complete
        );
    }

    #[test]
    fn restart_invalidates_retry_and_preserves_uncertain_effects() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("control");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "receipts::tests::crash_child",
                "--ignored",
                "--nocapture",
            ])
            .env("LOMI_RECEIPT_CRASH_ROOT", &root)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let marker = temp.path().join("committed.json");
        while !marker.exists() {
            assert!(
                child.try_wait().unwrap().is_none(),
                "Child exited before committing"
            );
            assert!(std::time::Instant::now() < deadline, "Child did not commit");
            std::thread::sleep(Duration::from_millis(10));
        }
        // SIGKILL leaves SQLite's WAL in place; no Rust destructors/checkpoint run.
        child.kill().unwrap();
        child.wait().unwrap();
        let (epoch, id): (String, String) =
            serde_json::from_slice(&fs::read(marker).unwrap()).unwrap();
        let mut store = Store::open(&root, 200).unwrap();
        let recovered = store.get("paired-client", "project-one", &id).unwrap();
        assert_eq!(recovered.state, State::OutcomeUnknown);
        assert_eq!(recovered.effect_state, Effect::Unknown);
        assert_eq!(
            store.reserve(&key(&epoch), [1; 32], 201).unwrap_err(),
            Error::RetryWindowExpired
        );
        assert_eq!(
            store
                .transition(
                    "paired-client",
                    "project-one",
                    &id,
                    State::Running,
                    Effect::None,
                    201
                )
                .unwrap_err(),
            Error::InvalidTransition
        );
        let history: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM history WHERE operation=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(history, 3);
    }

    #[test]
    #[ignore = "Owned subprocess killed by the crash-recovery test"]
    fn crash_child() {
        let root = std::path::PathBuf::from(std::env::var_os("LOMI_RECEIPT_CRASH_ROOT").unwrap());
        let mut store = Store::open(&root, 100).unwrap();
        let epoch = store.issue_epoch("paired-client", 100).unwrap();
        let id = store
            .reserve(&key(&epoch), [1; 32], 101)
            .unwrap()
            .receipt
            .operation_id;
        store
            .transition(
                "paired-client",
                "project-one",
                &id,
                State::Running,
                Effect::None,
                102,
            )
            .unwrap();
        fs::write(
            root.parent().unwrap().join("committed.tmp"),
            serde_json::to_vec(&(epoch, id)).unwrap(),
        )
        .unwrap();
        fs::rename(
            root.parent().unwrap().join("committed.tmp"),
            root.parent().unwrap().join("committed.json"),
        )
        .unwrap();
        loop {
            std::thread::park();
        }
    }

    #[test]
    fn storage_rejects_another_owner_links_and_future_schemas() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("control");
        let store = Store::open(&root, 100).unwrap();
        assert!(Store::open(&root, 100).is_err());
        store
            .connection
            .pragma_update(None, "user_version", 6)
            .unwrap();
        drop(store);
        let original = fs::read(root.join("control.sqlite3")).unwrap();
        assert!(Store::open(&root, 101).is_err());
        assert_eq!(fs::read(root.join("control.sqlite3")).unwrap(), original);
        let alias = temp.path().join("alias");
        symlink(&root, &alias).unwrap();
        assert!(Store::open(&alias, 101).is_err());
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(Store::open(&root, 101).is_err());
    }
}
