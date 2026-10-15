//! Ordinary terminal alerts are observations, never verified task completion.
//! This private owner stores only opaque identities and captured cloud consent.
use super::ledger::{self, Admission, Channel, Intent};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};
use tauri::{AppHandle, Manager};

const OBSERVATIONS: &str = "CREATE TABLE terminal_notification_observations(notification_id TEXT PRIMARY KEY,observed_at INTEGER NOT NULL);";
// Seven-day UUID receipts are small metadata, independent of the shared
// 1000 pending-intent bound; normal dispatch must not consume a weekly quota.
const RECEIPT_CAP: i64 = 100_000;
const RETENTION: u64 = 7 * 86400;
fn storage(_: rusqlite::Error) -> String {
    "Terminal notification ledger is unavailable.".into()
}
pub(crate) fn validate_id(id: &str) -> Result<(), String> {
    super::valid_id(id)?;
    if id.bytes().any(|b| b.is_ascii_uppercase()) {
        return Err("INVALID_REQUEST".into());
    }
    if id.as_bytes()[14] != b'4' || !matches!(id.as_bytes()[19], b'8' | b'9' | b'a' | b'b') {
        return Err("INVALID_REQUEST".into());
    }
    Ok(())
}
pub(super) struct Store {
    connection: Connection,
}
impl Store {
    pub(super) fn open(path: &Path) -> Result<Self, String> {
        crate::chat::storage::reject_link(path)?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|_| "Terminal notification ledger is unavailable.")?;
        crate::chat::storage::private(path, false)?;
        drop(file);
        let mut connection = Connection::open(path).map_err(storage)?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(storage)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA temp_store=MEMORY;",
            )
            .map_err(storage)?;
        let mode: String = connection
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .map_err(storage)?;
        let sync: i64 = connection
            .query_row("PRAGMA synchronous", [], |r| r.get(0))
            .map_err(storage)?;
        if mode != "delete" || sync != 2 {
            return Err("Durable terminal notification storage is unavailable.".into());
        }
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(storage)?;
        if version == 0 {
            let count: i64 = connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'", [], |r| r.get(0)).map_err(storage)?;
            if count != 0 {
                return Err("Unknown terminal notification schema; storage was preserved.".into());
            }
            let tx = connection.transaction().map_err(storage)?;
            tx.execute_batch(ledger::SCHEMA).map_err(storage)?;
            tx.execute_batch(OBSERVATIONS).map_err(storage)?;
            tx.execute_batch("PRAGMA user_version=1").map_err(storage)?;
            tx.commit().map_err(storage)?;
        } else if version != 1 {
            return Err("Unknown terminal notification schema; storage was preserved.".into());
        }
        ledger::validate(&connection)?;
        let mut q = connection
            .prepare("PRAGMA table_info(terminal_notification_observations)")
            .map_err(storage)?;
        let cols = q
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        if cols != ["notification_id", "observed_at"] {
            return Err("Unknown terminal observation schema.".into());
        }
        drop(q);
        Ok(Self { connection })
    }
    fn observe(&mut self, terminal: &str, id: &str, admission: &Admission) -> Result<(), String> {
        self.observe_bounded(terminal, id, admission, RECEIPT_CAP)
    }
    fn observe_bounded(
        &mut self,
        terminal: &str,
        id: &str,
        admission: &Admission,
        receipt_cap: i64,
    ) -> Result<(), String> {
        validate_id(id)?;
        let tx = self.connection.transaction().map_err(storage)?;
        let now = ledger::now();
        tx.execute(
            "DELETE FROM terminal_notification_observations WHERE observed_at<?1",
            [now.saturating_sub(RETENTION) as i64],
        )
        .map_err(storage)?;
        let duplicate: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM terminal_notification_observations WHERE notification_id=?1)", [id], |r| r.get(0)).map_err(storage)?;
        if duplicate {
            return tx.commit().map_err(storage);
        }
        let count: i64 = tx
            .query_row(
                "SELECT count(*) FROM terminal_notification_observations",
                [],
                |r| r.get(0),
            )
            .map_err(storage)?;
        // Bound all retained metadata, including disabled observations and
        // successful/suppressed intents, not just the pending queue.
        if count >= receipt_cap {
            return Err("Terminal notification receipt capacity is exhausted.".into());
        }
        if tx
            .execute(
                "INSERT OR IGNORE INTO terminal_notification_observations VALUES(?1,?2)",
                params![id, now as i64],
            )
            .map_err(storage)?
            == 1
        {
            if let Some(cloud) = admission.cloud.as_ref() {
                let admission = Admission {
                    desktop: None,
                    cloud: Some(cloud.clone()),
                };
                // Shared envelope names are transport identity only: logicalTaskId
                // is an opaque terminal hash, turnId is this observation UUID,
                // and completedAt is receipt time, not a completed task claim.
                let opaque = format!("{:x}", Sha256::digest(terminal.as_bytes()));
                ledger::admit(&tx, id, &admission)?;
                ledger::complete(&tx, "terminal-notification", &opaque, id)?;
            }
        }
        tx.commit().map_err(storage)
    }
    pub(super) fn ready(&self, channel: Channel) -> Result<Vec<Intent>, String> {
        ledger::ready(&self.connection, channel)
    }
    pub(super) fn transition(
        &self,
        channel: Channel,
        intent: &Intent,
        expected: &str,
        state: &str,
        next: u64,
    ) -> Result<bool, String> {
        ledger::transition(&self.connection, channel, intent, expected, state, next)
    }
}
pub(crate) fn observe(app: &AppHandle, session: &str, id: &str) -> Result<(), String> {
    // Capture current authority at receipt, never refresh or revive historical
    // observations. Dispatcher rechecks this exact owner/epoch before sending.
    let admission = super::capture(app);
    let push = app.state::<super::Push>();
    let mut owner = push
        .terminal
        .lock()
        .map_err(|_| "Terminal notification ledger is unavailable.")?;
    owner
        .as_mut()
        .ok_or("Terminal notification ledger is unavailable.")?
        .observe(session, id, &admission)
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "12345678-1234-4234-8234-123456789abc";
    fn admission() -> Admission {
        Admission {
            desktop: None,
            cloud: Some(ledger::CloudStamp {
                owner: ledger::Owner {
                    user_id: "user".into(),
                    session_id: "login".into(),
                    process_id: "process".into(),
                    generation: 1,
                },
                source_id: "source".into(),
                consent_epoch: 1,
                source_epoch: 2,
            }),
        }
    }
    #[test]
    fn duplicate_receipt_is_cloud_only_and_persists_only_opaque_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("terminal.sqlite3");
        let mut store = Store::open(&path).unwrap();
        store
            .observe("secret-terminal-session", ID, &admission())
            .unwrap();
        store
            .observe("secret-terminal-session", ID, &admission())
            .unwrap();
        assert!(store.ready(Channel::Desktop).unwrap().is_empty());
        let intents = store.ready(Channel::Cloud).unwrap();
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].source_kind, "terminal-notification");
        assert_eq!(intents[0].turn_id, ID);
        assert!(!String::from_utf8_lossy(&std::fs::read(path).unwrap())
            .contains("secret-terminal-session"));
    }
    #[test]
    fn disabled_observation_cannot_be_revived_by_duplicate_after_consent() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("ledger.sqlite3")).unwrap();
        store
            .observe("terminal", ID, &Admission::default())
            .unwrap();
        store.observe("terminal", ID, &admission()).unwrap();
        assert!(store.ready(Channel::Cloud).unwrap().is_empty());
        assert!(store.observe("terminal", "invalid", &admission()).is_err());
    }
    #[test]
    fn retained_disabled_receipts_do_not_consume_pending_queue_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("ledger.sqlite3")).unwrap();
        for i in 0..1000 {
            let id = format!("{i:08x}-1234-4234-8234-123456789abc");
            store
                .observe("terminal", &id, &Admission::default())
                .unwrap();
        }
        store.observe("terminal", ID, &admission()).unwrap();
        assert_eq!(store.ready(Channel::Cloud).unwrap().len(), 1);
    }
    #[test]
    fn receipt_saturation_is_explicit_and_duplicates_remain_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("ledger.sqlite3")).unwrap();
        store
            .observe_bounded("terminal", ID, &Admission::default(), 1)
            .unwrap();
        store
            .observe_bounded("terminal", ID, &admission(), 1)
            .unwrap();
        assert!(store
            .observe_bounded(
                "terminal",
                "ffffffff-1234-4234-8234-123456789abc",
                &admission(),
                1
            )
            .unwrap_err()
            .contains("capacity"));
        assert!(store.ready(Channel::Cloud).unwrap().is_empty());
    }
    #[test]
    fn enqueue_failure_rolls_back_observation_and_admission() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("ledger.sqlite3")).unwrap();
        store.connection.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON push_notification_intents BEGIN SELECT RAISE(ABORT,'failure'); END;").unwrap();
        assert!(store.observe("terminal", ID, &admission()).is_err());
        for table in [
            "terminal_notification_observations",
            "notification_admissions",
            "push_notification_intents",
        ] {
            assert_eq!(
                store
                    .connection
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        store.connection.execute_batch("DROP TRIGGER fail").unwrap();
        store.observe("terminal", ID, &admission()).unwrap();
        assert_eq!(store.ready(Channel::Cloud).unwrap().len(), 1);
    }
}
