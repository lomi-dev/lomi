//! Each source owner calls these helpers on its own connection/transaction.
//! They never open a database or acquire another source's lock.
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SCHEMA: &str = "CREATE TABLE notification_admissions(turn_id TEXT PRIMARY KEY,record TEXT NOT NULL); CREATE TABLE push_notification_intents(event_key TEXT PRIMARY KEY,record TEXT NOT NULL,state TEXT NOT NULL,expires_at INTEGER NOT NULL,next_attempt_at INTEGER NOT NULL); CREATE TABLE desktop_notification_intents(completion_key TEXT PRIMARY KEY,record TEXT NOT NULL,state TEXT NOT NULL,expires_at INTEGER NOT NULL,next_attempt_at INTEGER NOT NULL);";
const CAP: i64 = 1000;
const RETENTION: u64 = 7 * 86400;
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn storage(_: rusqlite::Error) -> String {
    "Notification ledger storage is unavailable.".into()
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Owner {
    pub user_id: String,
    pub session_id: String,
    pub process_id: String,
    pub generation: u64,
}
impl Owner {
    pub fn matches(&self, binding: &crate::auth::controller::RemoteBinding, process: &str) -> bool {
        self.user_id == binding.user_id
            && self.session_id == binding.session_id
            && (self.process_id != process || self.generation == binding.generation)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopStamp {
    pub owner: Owner,
    pub installation_id: String,
    pub consent_epoch: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudStamp {
    pub owner: Owner,
    pub source_id: String,
    pub consent_epoch: u64,
    pub source_epoch: u64,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Admission {
    pub desktop: Option<DesktopStamp>,
    pub cloud: Option<CloudStamp>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Intent {
    pub key: String,
    pub source_kind: String,
    pub logical_task_id: String,
    pub turn_id: String,
    pub completed_at: u64,
    pub expires_at: u64,
    pub admission: Admission,
    pub retry_count: u32,
    pub reason: Option<String>,
    pub server_event_id: Option<String>,
}
#[derive(Clone, Copy, Debug)]
pub enum Channel {
    Desktop,
    Cloud,
}
impl Channel {
    fn table(self) -> &'static str {
        match self {
            Self::Desktop => "desktop_notification_intents",
            Self::Cloud => "push_notification_intents",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Desktop => "completion_key",
            Self::Cloud => "event_key",
        }
    }
    pub fn ready(self) -> &'static str {
        match self {
            Self::Desktop => "ready",
            Self::Cloud => "pending",
        }
    }
}
pub fn canonical_key(identity: &str, kind: &str, logical: &str, turn: &str) -> String {
    // Fixed ordered names and UTF-8 JSON encoding, shared with the server.
    let canonical = format!("{{\"schemaVersion\":1,\"sourceId\":{},\"sourceKind\":{},\"logicalTaskId\":{},\"turnId\":{}}}", serde_json::to_string(identity).unwrap(), serde_json::to_string(kind).unwrap(), serde_json::to_string(logical).unwrap(), serde_json::to_string(turn).unwrap());
    format!("{:x}", Sha256::digest(canonical.as_bytes()))
}
pub fn desktop_key(stamp: &DesktopStamp, kind: &str, logical: &str, turn: &str) -> String {
    let bytes = serde_json::to_vec(&(
        1,
        &stamp.installation_id,
        &stamp.owner.user_id,
        kind,
        logical,
        turn,
    ))
    .unwrap();
    format!("{:x}", Sha256::digest(bytes))
}
pub fn admit(db: &Connection, turn: &str, admission: &Admission) -> Result<(), String> {
    db.execute(
        "INSERT OR IGNORE INTO notification_admissions VALUES(?1,?2)",
        params![
            turn,
            serde_json::to_string(admission).map_err(|_| "Invalid notification admission.")?
        ],
    )
    .map_err(storage)?;
    Ok(())
}
pub fn complete(db: &Connection, kind: &str, logical: &str, turn: &str) -> Result<(), String> {
    let record: Option<String> = db
        .query_row(
            "SELECT record FROM notification_admissions WHERE turn_id=?1",
            [turn],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    let Some(record) = record else { return Ok(()) };
    let admission: Admission =
        serde_json::from_str(&record).map_err(|_| "Invalid notification admission.")?;
    let time = now();
    for channel in [Channel::Desktop, Channel::Cloud] {
        let key = match channel {
            Channel::Desktop => admission
                .desktop
                .as_ref()
                .map(|s| desktop_key(s, kind, logical, turn)),
            Channel::Cloud => admission
                .cloud
                .as_ref()
                .map(|s| canonical_key(&s.source_id, kind, logical, turn)),
        };
        let Some(key) = key else { continue };
        let table = channel.table();
        db.execute(&format!("UPDATE {table} SET state='expired' WHERE expires_at<=?1 AND state IN ('pending','ready')"), [time as i64]).map_err(storage)?;
        db.execute(
            &format!("DELETE FROM {table} WHERE expires_at<?1"),
            [time.saturating_sub(RETENTION) as i64],
        )
        .map_err(storage)?;
        let count: i64 = db
            .query_row(
                &format!(
                    "SELECT count(*) FROM {table} WHERE state IN ('pending','ready','claimed') AND expires_at>?1"
                ),
                [time as i64],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if count >= CAP {
            continue;
        }
        let intent = Intent {
            key: key.clone(),
            source_kind: kind.into(),
            logical_task_id: logical.into(),
            turn_id: turn.into(),
            completed_at: time,
            expires_at: time + 900,
            admission: match channel {
                Channel::Desktop => Admission {
                    desktop: admission.desktop.clone(),
                    cloud: None,
                },
                Channel::Cloud => Admission {
                    desktop: None,
                    cloud: admission.cloud.clone(),
                },
            },
            retry_count: 0,
            reason: None,
            server_event_id: None,
        };
        db.execute(
            &format!("INSERT OR IGNORE INTO {table} VALUES(?1,?2,?3,?4,?5)"),
            params![
                key,
                serde_json::to_string(&intent).unwrap(),
                channel.ready(),
                intent.expires_at as i64,
                time as i64
            ],
        )
        .map_err(storage)?;
    }
    // Consuming the admission in this same transaction prevents a stale final
    // save from recreating an event after seven-day intent retention expires.
    db.execute(
        "DELETE FROM notification_admissions WHERE turn_id=?1",
        [turn],
    )
    .map_err(storage)?;
    Ok(())
}
pub fn ready(db: &Connection, channel: Channel) -> Result<Vec<Intent>, String> {
    // Idle applications still retire ledger metadata. Cleanup is bounded and
    // never touches source history or execution outbox rows.
    db.execute(
        &format!(
            "DELETE FROM {} WHERE {} IN (SELECT {} FROM {} WHERE expires_at<?1 LIMIT 128)",
            channel.table(),
            channel.key(),
            channel.key(),
            channel.table()
        ),
        [now().saturating_sub(RETENTION) as i64],
    )
    .map_err(storage)?;
    let mut query = db.prepare(&format!("SELECT record FROM {} WHERE state=?1 AND next_attempt_at<=?2 ORDER BY expires_at LIMIT 16", channel.table())).map_err(storage)?;
    let rows = query
        .query_map(params![channel.ready(), now() as i64], |r| {
            r.get::<_, String>(0)
        })
        .map_err(storage)?;
    let mut result = Vec::new();
    for row in rows {
        let row = row.map_err(storage)?;
        if row.len() > 16384 {
            return Err("Notification intent exceeds its bound.".into());
        }
        result.push(serde_json::from_str(&row).map_err(|_| "Invalid notification intent.")?);
    }
    Ok(result)
}
pub fn transition(
    db: &Connection,
    channel: Channel,
    intent: &Intent,
    expected: &str,
    state: &str,
    next: u64,
) -> Result<bool, String> {
    let changed = db
        .execute(
            &format!(
                "UPDATE {} SET record=?1,state=?2,next_attempt_at=?3 WHERE {}=?4 AND state=?5",
                channel.table(),
                channel.key()
            ),
            params![
                serde_json::to_string(intent).unwrap(),
                state,
                next as i64,
                intent.key,
                expected
            ],
        )
        .map_err(storage)?;
    Ok(changed == 1)
}
pub fn validate(db: &Connection) -> Result<(), String> {
    for (table, columns) in [
        ("notification_admissions", vec!["turn_id", "record"]),
        (
            "push_notification_intents",
            vec![
                "event_key",
                "record",
                "state",
                "expires_at",
                "next_attempt_at",
            ],
        ),
        (
            "desktop_notification_intents",
            vec![
                "completion_key",
                "record",
                "state",
                "expires_at",
                "next_attempt_at",
            ],
        ),
    ] {
        let mut q = db
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(storage)?;
        let actual = q
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        if actual != columns {
            return Err(
                "Unknown notification ledger schema; original storage was preserved.".into(),
            );
        }
        let (count, length): (i64, i64) = db
            .query_row(
                &format!("SELECT count(*),coalesce(max(length(record)),0) FROM {table}"),
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(storage)?;
        if count > 1000000 || length > 16384 {
            return Err("Notification ledger exceeds its bound.".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        c
    }
    fn admission(desktop: bool, cloud: bool) -> Admission {
        let owner = Owner {
            user_id: "user".into(),
            session_id: "session".into(),
            process_id: "process".into(),
            generation: 7,
        };
        Admission {
            desktop: desktop.then(|| DesktopStamp {
                owner: owner.clone(),
                installation_id: "installation".into(),
                consent_epoch: 2,
            }),
            cloud: cloud.then(|| CloudStamp {
                owner,
                source_id: "source".into(),
                consent_epoch: 3,
                source_epoch: 4,
            }),
        }
    }
    fn count(c: &Connection, table: &str) -> i64 {
        c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
    #[test]
    fn channels_are_independent_and_unadmitted_history_never_emits() {
        for (desktop, cloud) in [(false, false), (true, false), (false, true), (true, true)] {
            let c = fixture();
            admit(&c, "turn", &admission(desktop, cloud)).unwrap();
            complete(&c, "chat", "conversation", "turn").unwrap();
            complete(&c, "chat", "history", "never-admitted").unwrap();
            assert_eq!(
                count(&c, "desktop_notification_intents"),
                i64::from(desktop)
            );
            assert_eq!(count(&c, "push_notification_intents"), i64::from(cloud));
        }
    }
    #[test]
    fn failed_completion_rolls_back_both_channels_and_retry_deduplicates() {
        let mut c = fixture();
        admit(&c, "turn", &admission(true, true)).unwrap();
        c.execute_batch("CREATE TABLE result(id TEXT PRIMARY KEY); CREATE TRIGGER fail_cloud BEFORE INSERT ON push_notification_intents BEGIN SELECT RAISE(ABORT,'fail'); END;").unwrap();
        {
            let tx = c.transaction().unwrap();
            tx.execute("INSERT INTO result VALUES('completed')", [])
                .unwrap();
            assert!(complete(&tx, "chat", "conversation", "turn").is_err());
        }
        assert_eq!(count(&c, "result"), 0);
        assert_eq!(count(&c, "desktop_notification_intents"), 0);
        c.execute_batch("DROP TRIGGER fail_cloud").unwrap();
        let tx = c.transaction().unwrap();
        tx.execute("INSERT INTO result VALUES('completed')", [])
            .unwrap();
        complete(&tx, "chat", "conversation", "turn").unwrap();
        tx.commit().unwrap();
        complete(&c, "chat", "conversation", "turn").unwrap();
        assert_eq!(count(&c, "desktop_notification_intents"), 1);
        assert_eq!(count(&c, "push_notification_intents"), 1);
    }
    #[test]
    fn admission_cannot_rebind_and_new_turn_has_a_distinct_key() {
        let c = fixture();
        let a = admission(true, true);
        admit(&c, "turn", &a).unwrap();
        admit(&c, "turn", &Admission::default()).unwrap();
        complete(&c, "managed-task", "task", "turn").unwrap();
        admit(&c, "continue", &a).unwrap();
        complete(&c, "managed-task", "task", "continue").unwrap();
        assert_eq!(count(&c, "desktop_notification_intents"), 2);
        assert_eq!(count(&c, "push_notification_intents"), 2);
    }
    #[test]
    fn claimed_crash_never_replays_and_late_ack_cannot_unsuppress() {
        let c = fixture();
        admit(&c, "turn", &admission(true, false)).unwrap();
        complete(&c, "chat", "c", "turn").unwrap();
        let intent = ready(&c, Channel::Desktop).unwrap().pop().unwrap();
        assert!(transition(&c, Channel::Desktop, &intent, "ready", "claimed", 0).unwrap());
        assert!(ready(&c, Channel::Desktop).unwrap().is_empty());
        assert!(transition(&c, Channel::Desktop, &intent, "claimed", "suppressed", 0).unwrap());
        assert!(!transition(
            &c,
            Channel::Desktop,
            &intent,
            "claimed",
            "plugin_accepted",
            0
        )
        .unwrap());
    }
    #[test]
    fn queue_cap_and_retention_leave_source_history_alone() {
        let c = fixture();
        c.execute_batch(
            "CREATE TABLE history(id TEXT PRIMARY KEY); INSERT INTO history VALUES('kept');",
        )
        .unwrap();
        let a = admission(true, true);
        for i in 0..1001 {
            let turn = i.to_string();
            admit(&c, &turn, &a).unwrap();
            complete(&c, "chat", "c", &turn).unwrap();
        }
        assert_eq!(count(&c, "desktop_notification_intents"), 1000);
        assert_eq!(count(&c, "push_notification_intents"), 1000);
        assert_eq!(count(&c, "history"), 1);
        c.execute(
            "UPDATE desktop_notification_intents SET expires_at=?1",
            [(now() - RETENTION - 1) as i64],
        )
        .unwrap();
        admit(&c, "new", &a).unwrap();
        complete(&c, "chat", "c", "new").unwrap();
        assert_eq!(count(&c, "desktop_notification_intents"), 1);
        assert_eq!(count(&c, "history"), 1);
    }
    #[test]
    fn durable_owner_ignores_old_process_generation_but_not_new_session() {
        let owner = admission(true, false).desktop.unwrap().owner;
        let mut b = crate::auth::controller::RemoteBinding {
            user_id: "user".into(),
            session_id: "session".into(),
            expires_at: now() + 60,
            generation: 7,
        };
        assert!(owner.matches(&b, "process"));
        b.generation = 1;
        assert!(!owner.matches(&b, "process"));
        assert!(owner.matches(&b, "restarted-process"));
        b.session_id = "new-login".into();
        assert!(!owner.matches(&b, "restarted-process"));
        b.user_id = "other".into();
        assert!(!owner.matches(&b, "process"));
    }
    #[test]
    fn canonical_hash_matches_fixed_utf8_contract() {
        assert_eq!(canonical_key("source","chat","conversation","turn"),format!("{:x}",Sha256::digest(br#"{"schemaVersion":1,"sourceId":"source","sourceKind":"chat","logicalTaskId":"conversation","turnId":"turn"}"#)));
        assert_ne!(
            canonical_key("a", "bc", "d", "e"),
            canonical_key("ab", "c", "d", "e")
        );
    }
}
