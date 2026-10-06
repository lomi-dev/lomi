#[cfg(target_os = "macos")]
#[path = "store_boundary.rs"]
mod boundary;
use super::types::*;

use rusqlite::{params, Connection, OptionalExtension};

use serde::{de::DeserializeOwned, Serialize};

use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) struct Store {
    pub root: PathBuf,
    db: Connection,
    _owner: crate::chat::storage::Owner,
    #[cfg(test)]
    pub(super) fail_next_update: bool,
    #[cfg(test)]
    pub(super) fail_next_read: std::cell::Cell<bool>,
    #[cfg(test)]
    pub(super) fail_next_context: std::cell::Cell<bool>,
}

struct PreparedIntent {
    key: String,
    #[cfg(target_os = "macos")]
    context: Option<super::host_child::Context>,
}

fn error(_: rusqlite::Error) -> String {
    "Agent runtime durable storage is unavailable; execution is blocked.".into()
}

pub(super) fn encode<T: Serialize>(value: &T) -> Result<String, String> {
    fn canonical(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let ordered = map
                    .into_iter()
                    .map(|(k, v)| (k, canonical(v)))
                    .collect::<std::collections::BTreeMap<_, _>>();
                serde_json::to_value(ordered).unwrap()
            }
            serde_json::Value::Array(v) => {
                serde_json::Value::Array(v.into_iter().map(canonical).collect())
            }
            v => v,
        }
    }
    let value = serde_json::to_value(value).map_err(|_| "Cannot encode runtime record.")?;
    let encoded =
        serde_json::to_string(&canonical(value)).map_err(|_| "Cannot encode runtime record.")?;
    if encoded.len() > 64 * 1024 * 1024 {
        return Err("Runtime record exceeds the 67108864-byte durable bound; original history remains retained.".into());
    }
    Ok(encoded)
}

fn decode<T: DeserializeOwned>(value: String) -> Result<T, String> {
    serde_json::from_str(&value).map_err(|_| "Agent runtime record requires recovery.".into())
}

impl Store {
    pub fn open(root: PathBuf) -> Result<Self, String> {
        if root.exists() {
            super::native_accounts::check_private_directory(&root)?;
        }

        let existing = root.join("runtime.sqlite");

        if existing.exists() {
            preflight(&root)?;
        }

        let owner = crate::chat::storage::Owner::acquire(root)?;

        let root = owner.root.clone();

        for name in ["accounts", "tasks"] {
            let path = root.join(name);

            crate::chat::storage::reject_link(&path)?;

            fs::create_dir_all(&path).map_err(|_| "Cannot create runtime namespace.")?;

            crate::chat::storage::private(&path, true)?;
        }

        let path = root.join("runtime.sqlite");

        for suffix in ["", "-wal", "-shm"] {
            crate::chat::storage::reject_link(&root.join(format!("runtime.sqlite{suffix}")))?;
        }

        if !path.exists() {
            publish_initialization_intent(&root)?;
        }
        let db = Connection::open(&path).map_err(error)?;

        crate::chat::storage::private(&path, false)?;

        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
        )
        .map_err(error)?;
        let version: i64 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(error)?;
        if version == 0 {
            validate_initialization_intent(&root)?;
            db.execute_batch("BEGIN IMMEDIATE;
   CREATE TABLE IF NOT EXISTS metadata(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL); INSERT OR IGNORE INTO metadata VALUES(1,0);
   CREATE TABLE IF NOT EXISTS accounts(id TEXT PRIMARY KEY, record TEXT NOT NULL, binding TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS tasks(id TEXT PRIMARY KEY, record TEXT NOT NULL, shell TEXT NOT NULL, directory_identity TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS checkpoints(task_id TEXT PRIMARY KEY REFERENCES tasks(id), record TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS receipts(operation_id TEXT PRIMARY KEY, request TEXT NOT NULL, result TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS outbox(operation_id TEXT PRIMARY KEY REFERENCES receipts(operation_id),task_id TEXT NOT NULL,attempt_id TEXT NOT NULL,generation INTEGER NOT NULL,state TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS process_ownership(task_id TEXT NOT NULL,attempt_id TEXT PRIMARY KEY,account_id TEXT NOT NULL,generation INTEGER NOT NULL,process_group INTEGER,identity TEXT,ownership_marker TEXT NOT NULL,boot TEXT NOT NULL,unknown_tools TEXT NOT NULL,state TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS imports(digest TEXT PRIMARY KEY, record TEXT NOT NULL);
   CREATE TABLE native_operations(id TEXT PRIMARY KEY,record TEXT NOT NULL); PRAGMA application_id=1280265545; PRAGMA user_version=2; COMMIT;").map_err(error)?;
        } else if version == 1 {
            prepare_schema_one_upgrade(&db, &root)?;
            db.execute_batch("BEGIN IMMEDIATE; CREATE TABLE native_operations(id TEXT PRIMARY KEY,record TEXT NOT NULL); PRAGMA user_version=2; COMMIT;").map_err(error)?;
        }

        for suffix in ["-wal", "-shm"] {
            let p = root.join(format!("runtime.sqlite{suffix}"));

            if p.exists() {
                crate::chat::storage::private(&p, false)?;
            }
        }

        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(error)?;

        let mut result = Self {
            root,
            db,
            _owner: owner,
            #[cfg(test)]
            fail_next_update: false,
            #[cfg(test)]
            fail_next_read: std::cell::Cell::new(false),
            #[cfg(test)]
            fail_next_context: std::cell::Cell::new(false),
        };

        result.validate_records()?;
        validate_native_operations(&result.db)?;

        #[cfg(target_os = "macos")]
        result.reconcile_owned_helpers()?;
        result.recover_restart()?;

        Ok(result)
    }

    pub fn command_receipt<R: Serialize, T: Serialize>(
        &mut self,
        request: &R,
        operation: &str,
        result: &T,
    ) -> Result<(), String> {
        self.db
            .execute(
                "INSERT INTO receipts VALUES(?1,?2,?3)",
                params![operation, encode(request)?, encode(result)?],
            )
            .map_err(error)?;

        Ok(())
    }

    pub fn adopt_directory(&mut self, id: &str, identity: (u64, u64)) -> Result<(), String> {
        self.db
            .execute(
                "UPDATE tasks SET directory_identity=?2 WHERE id=?1",
                params![id, encode(&identity)?],
            )
            .map_err(error)?;

        Ok(())
    }

    pub fn validate_records(&self) -> Result<(), String> {
        validate_database_records(&self.db)
    }

    pub fn receipt<T: DeserializeOwned>(&self, id: &str) -> Result<Option<T>, String> {
        self.db
            .query_row(
                "SELECT result FROM receipts WHERE operation_id=?1",
                [id],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(error)?
            .map(decode)
            .transpose()
    }

    pub fn revision(&self) -> Result<u64, String> {
        self.db
            .query_row("SELECT revision FROM metadata WHERE id=1", [], |r| {
                r.get::<_, i64>(0)
            })
            .map_err(error)
            .and_then(|v| u64::try_from(v).map_err(|_| "Invalid runtime revision.".into()))
    }

    pub fn account(&self, id: &str) -> Result<AccountInstance, String> {
        self.db
            .query_row("SELECT record FROM accounts WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .map_err(error)
            .and_then(decode)
    }

    pub fn binding(&self, id: &str) -> Result<CredentialBinding, String> {
        self.db
            .query_row("SELECT binding FROM accounts WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .map_err(error)
            .and_then(decode)
    }

    pub fn accounts(&self) -> Result<Vec<AccountInstance>, String> {
        let mut stmt = self
            .db
            .prepare("SELECT record FROM accounts ORDER BY id")
            .map_err(error)?;

        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(error)?;

        rows.map(|r| decode(r.map_err(error)?)).collect()
    }

    pub fn tasks(&self) -> Result<Vec<Task>, String> {
        let mut stmt = self
            .db
            .prepare("SELECT record FROM tasks ORDER BY id")
            .map_err(error)?;

        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(error)?;

        rows.map(|r| decode(r.map_err(error)?)).collect()
    }

    pub fn task(&self, id: &str) -> Result<Task, String> {
        #[cfg(test)]
        if self.fail_next_read.replace(false) {
            return Err("Injected final-record read failure.".into());
        }
        self.db
            .query_row("SELECT record FROM tasks WHERE id=?1", [id], |r| r.get(0))
            .map_err(|_| "The task record is missing or unavailable.".into())
            .and_then(decode)
    }

    pub fn shell(&self, id: &str) -> Result<String, String> {
        self.db
            .query_row("SELECT shell FROM tasks WHERE id=?1", [id], |r| r.get(0))
            .map_err(error)
    }

    pub fn directory_identity(&self, id: &str) -> Result<(u64, u64), String> {
        self.db
            .query_row(
                "SELECT directory_identity FROM tasks WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(error)
            .and_then(decode)
    }

    pub fn checkpoint(&self, id: &str) -> Result<Option<HistoryCheckpoint>, String> {
        self.db
            .query_row(
                "SELECT record FROM checkpoints WHERE task_id=?1",
                [id],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(error)?
            .map(decode)
            .transpose()
    }

    pub fn replay<R: Serialize, T: DeserializeOwned>(
        &self,
        id: &str,
        request: &R,
    ) -> Result<Option<T>, String> {
        super::valid_operation(id)?;

        let req = encode(request)?;

        let row = self
            .db
            .query_row(
                "SELECT request,result FROM receipts WHERE operation_id=?1",
                [id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(error)?;

        match row {
            Some((old, result)) if old == req => Ok(Some(decode(result)?)),
            Some(_) => {
                Err("The operation ID was already used for different command content.".into())
            }

            None => Ok(None),
        }
    }

    pub fn save_account<R: Serialize, T: Serialize>(
        &mut self,
        request: &R,
        operation: &str,
        account: &AccountInstance,
        binding: &CredentialBinding,
        result: &T,
    ) -> Result<(), String> {
        let tx = self.db.transaction().map_err(error)?;

        tx.execute("INSERT INTO accounts VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET record=excluded.record,binding=excluded.binding",params![account.account_id,encode(account)?,encode(binding)?]).map_err(error)?;

        tx.execute(
            "INSERT INTO receipts VALUES(?1,?2,?3)",
            params![operation, encode(request)?, encode(result)?],
        )
        .map_err(error)?;

        tx.execute("UPDATE metadata SET revision=revision+1 WHERE id=1", [])
            .map_err(error)?;

        tx.commit().map_err(error)
    }

    pub fn delete_account<R: Serialize, T: Serialize>(
        &mut self,
        request: &R,
        operation: &str,
        id: &str,
        result: &T,
    ) -> Result<(), String> {
        let tx = self.db.transaction().map_err(error)?;

        tx.execute("DELETE FROM accounts WHERE id=?1", [id])
            .map_err(error)?;

        tx.execute(
            "INSERT INTO receipts VALUES(?1,?2,?3)",
            params![operation, encode(request)?, encode(result)?],
        )
        .map_err(error)?;

        tx.execute("UPDATE metadata SET revision=revision+1 WHERE id=1", [])
            .map_err(error)?;

        tx.commit().map_err(error)
    }

    pub fn create_task<R: Serialize>(
        &mut self,
        request: &R,
        operation: &str,
        task: &Task,
        shell: &str,
        identity: (u64, u64),
    ) -> Result<(), String> {
        let tx = self.db.transaction().map_err(error)?;

        tx.execute(
            "INSERT INTO tasks VALUES(?1,?2,?3,?4)",
            params![task.task_id, encode(task)?, shell, encode(&identity)?],
        )
        .map_err(error)?;

        tx.execute(
            "INSERT INTO receipts VALUES(?1,?2,?3)",
            params![operation, encode(request)?, encode(task)?],
        )
        .map_err(error)?;

        tx.execute("UPDATE metadata SET revision=revision+1 WHERE id=1", [])
            .map_err(error)?;

        tx.commit().map_err(error)
    }

    pub fn mutate_task<R: Serialize>(
        &mut self,
        request: &R,
        operation: &str,
        task: &Task,
        intention: Option<&Attempt>,
    ) -> Result<(), String> {
        let tx = self.db.transaction().map_err(error)?;

        tx.execute(
            "UPDATE tasks SET record=?2 WHERE id=?1",
            params![task.task_id, encode(task)?],
        )
        .map_err(error)?;

        tx.execute(
            "INSERT INTO receipts VALUES(?1,?2,?3)",
            params![operation, encode(request)?, encode(task)?],
        )
        .map_err(error)?;

        if let Some(a) = intention {
            tx.execute(
                "INSERT INTO outbox VALUES(?1,?2,?3,?4,'pending')",
                params![
                    operation,
                    task.task_id,
                    a.attempt_id,
                    i64::try_from(a.generation).map_err(|_| "Generation exhausted.")?
                ],
            )
            .map_err(error)?;
        }

        tx.execute("UPDATE metadata SET revision=revision+1 WHERE id=1", [])
            .map_err(error)?;

        tx.commit().map_err(error)
    }

    pub fn update_task(
        &mut self,
        task: &Task,
        checkpoint: Option<&HistoryCheckpoint>,
        outbox_state: Option<(&str, &str)>,
    ) -> Result<(), String> {
        #[cfg(test)]
        if std::mem::take(&mut self.fail_next_update) {
            return Err("Injected durable finalization failure.".into());
        }
        let tx = self.db.transaction().map_err(error)?;

        tx.execute(
            "UPDATE tasks SET record=?2 WHERE id=?1",
            params![task.task_id, encode(task)?],
        )
        .map_err(error)?;

        if let Some(c) = checkpoint {
            tx.execute("INSERT INTO checkpoints VALUES(?1,?2) ON CONFLICT(task_id) DO UPDATE SET record=excluded.record",params![task.task_id,encode(c)?]).map_err(error)?;
        }

        if let Some((operation, state)) = outbox_state {
            tx.execute(
                "UPDATE outbox SET state=?2 WHERE operation_id=?1",
                params![operation, state],
            )
            .map_err(error)?;
        }

        tx.execute("UPDATE metadata SET revision=revision+1 WHERE id=1", [])
            .map_err(error)?;

        tx.commit().map_err(error)
    }

    pub fn task_root(&self, id: &str) -> Result<PathBuf, String> {
        super::valid_operation(id)?;

        let path = self.root.join("tasks").join(id);

        crate::chat::storage::reject_link(&path)?;

        fs::create_dir_all(&path).map_err(|_| "Cannot create task namespace.")?;

        crate::chat::storage::private(&path, true)?;

        let native = path.join("native");

        crate::chat::storage::reject_link(&native)?;

        fs::create_dir_all(&native).map_err(|_| "Cannot create native task history namespace.")?;

        crate::chat::storage::private(&native, true)?;

        Ok(native)
    }

    fn recover_restart(&mut self) -> Result<(), String> {
        for mut task in self.tasks()? {
            if matches!(
                task.state,
                TaskState::Starting | TaskState::Running | TaskState::Stopping
            ) {
                let uncertain = task.attempts.iter().any(|a| {
                    matches!(
                        a.state.as_str(),
                        "spawn_intent" | "dispatched" | "running" | "stopping"
                    )
                });

                task.state = if uncertain {
                    TaskState::DeliveryUncertain
                } else {
                    TaskState::Stopped
                };

                task.revision += 1;

                task.active_account_id = None;

                task.active_attempt_id = None;

                task.status_message="The app restarted with an unsettled dispatch intention. Review effects; input will never be replayed.".into();

                for a in &mut task.attempts {
                    if matches!(
                        a.state.as_str(),
                        "dispatch_intent" | "spawn_intent" | "dispatched" | "running" | "stopping"
                    ) {
                        a.state = if uncertain {
                            "delivery_uncertain"
                        } else {
                            "rejected"
                        }
                        .into();

                        a.effects_state = if uncertain { "uncertain" } else { "settled" }.into();
                    }
                }

                self.update_task(&task, None, None)?;
            } else if task.active_attempt_id.is_some() || task.active_account_id.is_some() {
                // The terminal/checkpoint transaction may precede actor cleanup.
                // Preserve that durable outcome while clearing process-local IDs.
                task.active_attempt_id = None;
                task.active_account_id = None;
                task.revision += 1;
                self.update_task(&task, None, None)?;
            }
        }

        Ok(())
    }
}

pub(crate) fn workspace(path: &Path) -> Result<(String, (u64, u64)), String> {
    let p = dunce::canonicalize(path).map_err(|_| "Choose an existing project directory.")?;

    let meta = fs::metadata(&p).map_err(|_| "Cannot inspect project directory.")?;

    if !meta.is_dir() {
        return Err("The task project must be a directory.".into());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        Ok((p.to_string_lossy().into_owned(), (meta.dev(), meta.ino())))
    }

    #[cfg(not(unix))]
    {
        let _ = meta;

        Err("Native task directories are not qualified on this platform.".into())
    }
}

impl Store {
    pub(crate) fn import_bundle(
        &mut self,
        bundle: &super::migration::ImportBundle,
    ) -> Result<String, String> {
        use serde_json::{json, Value};

        use sha2::{Digest, Sha256};

        let bytes =
            serde_json::to_vec(bundle).map_err(|_| "Cannot encode migration publication.")?;

        if bytes.len() > 64 * 1024 * 1024 {
            return Err("The migration publication exceeds its bound.".into());
        }

        let digest = format!("{:x}", Sha256::digest(&bytes));

        let old = self
            .db
            .query_row(
                "SELECT record FROM imports WHERE digest=?1",
                [&digest],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(error)?;

        if let Some(old) = old {
            if old
                != String::from_utf8(bytes).map_err(|_| "Cannot decode migration publication.")?
            {
                return Err("Migration receipt content changed.".into());
            }

            return Ok(digest);
        }

        let tx = self.db.transaction().map_err(error)?;

        let profiles = bundle.snapshot["profiles"]
            .as_array()
            .ok_or("Migration accounts are missing.")?;

        for p in profiles {
            let id = p["id"].as_str().ok_or("Migration account ID missing.")?;

            super::valid_operation(id)?;

            let Ok(cli) = serde_json::from_value::<crate::cli_catalog::TitleCli>(p["cli"].clone())
            else {
                // Retired legacy families are preserved in the import archive
                // and task journal, never invented as subscription accounts.
                continue;
            };

            let root = bundle.physical_auth_roots.get(id);

            let ready = root.is_some() && super::native_accounts::available(cli);

            let revision = p["revision"].as_u64().unwrap_or(1).max(1);

            let account=AccountInstance{
account_id:id.into(),cli,label:p["label"].as_str().unwrap_or("Imported account").into(),enabled:ready&&p["enabled"].as_bool().unwrap_or(false),revision,auth_revision:revision,auth_state:"unverified".into(),availability_reason:Some(if ready{
"Imported original native binding; verify or explicitly open account terminal before new execution."}
else{
"Archived legacy API/unsupported account; no native binding was adopted."}
.into()),accepted_version:None,recovery:None}
;

            let binding = CredentialBinding {
                account_id: id.into(),
                auth_revision: revision,
                physical_root: root
                    .map(|v| v.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                namespace: format!("legacy-native:{:?}", cli),
                credential_reference: if root.is_some() {
                    "native-managed".into()
                } else {
                    format!("archived:{}", bundle.keyring_service)
                },
            };

            let conflict = tx
                .query_row(
                    "SELECT record,binding FROM accounts WHERE id=?1",
                    [id],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(error)?;

            match conflict {
                Some((a, b)) if a == encode(&account)? && b == encode(&binding)? => {}

                Some(_) => {
                    return Err(
                        "Migration account ID conflicts with an existing runtime account.".into(),
                    )
                }

                None => {
                    tx.execute(
                        "INSERT INTO accounts VALUES(?1,?2,?3)",
                        params![id, encode(&account)?, encode(&binding)?],
                    )
                    .map_err(error)?;
                }
            }
        }

        let routers = bundle.snapshot["routers"]
            .as_array()
            .ok_or("Migration router descriptors are missing.")?;

        for run in bundle.snapshot["runs"]
            .as_array()
            .ok_or("Migration history is missing.")?
        {
            let old_id = run["id"].as_str().ok_or("Migration run ID missing.")?;

            let id = bundle
                .run_id_map
                .get(old_id)
                .ok_or("Migration task mapping missing.")?;

            let cli = routers
                .iter()
                .find(|r| r["id"] == run["routerId"])
                .and_then(|r| serde_json::from_value(r["cli"].clone()).ok())
                .or_else(|| {
                    run["attempts"]
                        .as_array()
                        .and_then(|v| v.last())
                        .and_then(|a| profiles.iter().find(|p| p["id"] == a["profileId"]))
                        .and_then(|p| serde_json::from_value(p["cli"].clone()).ok())
                })
                .or_else(|| {
                    run["allowedProfileIds"]
                        .as_array()
                        .and_then(|v| v.first())
                        .and_then(|id| profiles.iter().find(|p| p["id"] == *id))
                        .and_then(|p| serde_json::from_value(p["cli"].clone()).ok())
                });

            let mut task=Task{
task_id:id.clone(),cwd:run["cwd"].as_str().unwrap_or("").into(),title:run["title"].as_str().unwrap_or("Imported task").into(),cli,availability_reason:cli.is_none().then(||"The archived CLI family cannot be qualified because its original router/account provenance is missing.".into()),model:run["model"].as_str().unwrap_or("").into(),reasoning_effort:run["reasoningEffort"].as_str().map(str::to_owned),revision:1,history_revision:0,generation:run["generation"].as_u64().unwrap_or(0),state:TaskState::Archived,next_account_id:run["pinnedProfileId"].as_str().or_else(||run["allowedProfileIds"].as_array().and_then(|v|v.first()).and_then(Value::as_str)).unwrap_or("").into(),active_account_id:None,active_attempt_id:None,status_message:"Archived original history. Legacy delivery/effects were not adopted as settled; restore never executes.".into(),attempts:vec![],history:vec![],grants:vec![],switches:vec![]}
;

            for a in run["attempts"].as_array().into_iter().flatten() {
                let account = a["profileId"].as_str().unwrap_or("");

                let auth = profiles
                    .iter()
                    .find(|p| p["id"] == account)
                    .and_then(|p| p["revision"].as_u64())
                    .unwrap_or(1)
                    .max(1);

                let input_id = a["inputId"].as_str().unwrap_or("");

                let input = run["inputs"]
                    .as_array()
                    .and_then(|v| v.iter().find(|v| v["id"] == input_id))
                    .and_then(|v| v["text"].as_str())
                    .unwrap_or("");

                let attempt = Attempt {
                    attempt_id: a["id"].as_str().unwrap_or("legacy").into(),
                    operation_id: format!("legacy:{input_id}"),
                    account_id: account.into(),
                    auth_revision: auth,
                    generation: a["generation"].as_u64().unwrap_or(0),
                    input: input.into(),
                    continuation_method: "blocked".into(),
                    state: a["state"].as_str().unwrap_or("unknown").into(),
                    output: run["turns"]
                        .as_array()
                        .and_then(|v| v.iter().find(|v| v["attemptId"] == a["id"]))
                        .and_then(|v| v["text"].as_str())
                        .unwrap_or("")
                        .into(),
                    native_ref: None,
                    version: None,
                    effects_state: "archived_unknown".into(),
                };

                super::service::append(
                    &mut task,
                    &attempt,
                    "legacy_attempt",
                    "archived",
                    json!({
                    "original":a,"input":input,"output":attempt.output}
                    ),
                );

                task.attempts.push(attempt);
            }

            let provenance = task.attempts.last().cloned().unwrap_or(Attempt {
                attempt_id: format!("legacy:{old_id}"),
                operation_id: format!("legacy:{old_id}"),
                account_id: task.next_account_id.clone(),
                auth_revision: 1,
                generation: task.generation,
                input: String::new(),
                continuation_method: "blocked".into(),
                state: "archived".into(),
                output: String::new(),
                native_ref: None,
                version: None,
                effects_state: "archived_unknown".into(),
            });

            super::service::append(
                &mut task,
                &provenance,
                "legacy_record",
                "archived",
                run.clone(),
            );

            for profile in profiles.iter().filter(|p| {
                run["attempts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|a| a["profileId"] == p["id"])
                    || run["allowedProfileIds"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|id| *id == p["id"])
            }) {
                super::service::append(
                    &mut task,
                    &provenance,
                    "legacy_account_record",
                    "archived",
                    profile.clone(),
                );
            }

            let raw_root = bundle
                .archive
                .join("originals/runs")
                .join(old_id)
                .join("native");

            if raw_root.is_dir() {
                let mut paths = fs::read_dir(&raw_root)
                    .map_err(|_| "Cannot read archived native observations.")?
                    .map(|e| {
                        e.map(|e| e.path())
                            .map_err(|_| "Cannot inspect archived native record.")
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                paths.sort_by_key(|p| {
                    p.file_name()
                        .and_then(|s| s.to_str())
                        .and_then(|s| s.strip_prefix("attempt-"))
                        .and_then(|s| s.strip_suffix(".json"))
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(u64::MAX)
                });
                for path in paths {
                    let name = path.file_name().and_then(|v| v.to_str()).unwrap_or("");

                    if name.starts_with("attempt-") && name.ends_with(".json") {
                        crate::chat::storage::reject_link(&path)?;

                        let meta = fs::metadata(&path)
                            .map_err(|_| "Cannot read archived observation metadata.")?;

                        if meta.len() > 32 * 1024 * 1024 {
                            return Err("Archived observation exceeds its bound.".into());
                        }

                        let bytes =
                            fs::read(&path).map_err(|_| "Cannot read archived observation.")?;

                        let value = super::native_wire::strict_json(&bytes).or_else(|_| {
                            serde_json::from_slice(&bytes).map_err(|_| {
                                String::from("Archived observation requires recovery.")
                            })
                        })?;

                        let generation = name
                            .strip_prefix("attempt-")
                            .and_then(|s| s.strip_suffix(".json"))
                            .and_then(|s| s.parse::<u64>().ok());
                        let matched = generation.and_then(|g| {
                            task.attempts.iter().position(|a| {
                                a.generation == g
                                    && value["generation"].as_u64() == Some(g)
                                    && value["profileId"].as_str() == Some(a.account_id.as_str())
                                    && value["inputId"].as_str().is_some_and(|input| {
                                        a.operation_id == format!("legacy:{input}")
                                    })
                                    && value["runId"].as_str() == Some(old_id)
                                    && value["schema"].as_u64() == Some(1)
                                    && value["profileRevision"].as_u64().is_some_and(|v| v > 0)
                            })
                        });
                        let native_provenance = if let Some(index) = matched {
                            let a = &mut task.attempts[index];
                            a.auth_revision = value["profileRevision"].as_u64().unwrap();
                            a.native_ref = value["sessionId"].as_str().map(str::to_owned);
                            a.version = value["version"]
                                .as_str()
                                .filter(|s| !s.is_empty())
                                .map(str::to_owned);
                            // Earlier input/turn summaries now share the exact frozen
                            // original auth revision, never today's account revision.
                            for h in &mut task.history {
                                if h.attempt_id == a.attempt_id {
                                    h.auth_revision = a.auth_revision;
                                }
                            }
                            a.clone()
                        } else {
                            task.availability_reason=Some("Archived native observations have missing or conflicting attempt/account/generation provenance; same-task continuation requires explicit archive qualification.".into());
                            Attempt {
                                attempt_id: format!(
                                    "legacy-unknown:{old_id}:{}",
                                    generation.unwrap_or(0)
                                ),
                                operation_id: format!("legacy-unknown:{old_id}"),
                                account_id: String::new(),
                                auth_revision: 1,
                                generation: generation.unwrap_or(0),
                                input: String::new(),
                                continuation_method: "blocked".into(),
                                state: "archived".into(),
                                output: String::new(),
                                native_ref: None,
                                version: None,
                                effects_state: "archived_unknown".into(),
                            }
                        };
                        super::service::append(
                            &mut task,
                            &native_provenance,
                            "legacy_native_observation",
                            "archived",
                            value,
                        );
                    }
                }
            }

            let record = encode(&task)?;

            if record.len() > 64 * 1024 * 1024 {
                return Err("Imported task exceeds retained history bound.".into());
            }

            let conflict = tx
                .query_row("SELECT record FROM tasks WHERE id=?1", [id], |r| {
                    r.get::<_, String>(0)
                })
                .optional()
                .map_err(error)?;

            match conflict {
                Some(old) if old == record => {}

                Some(_) => return Err("Migration task ID conflicts with an existing task.".into()),
                None => {
                    tx.execute(
                        "INSERT INTO tasks VALUES(?1,?2,?3,?4)",
                        params![
                            id,
                            record,
                            run["shellProfileId"].as_str().unwrap_or(""),
                            "[0,0]"
                        ],
                    )
                    .map_err(error)?;
                }
            }
        }

        tx.execute(
            "INSERT INTO imports VALUES(?1,?2)",
            params![
                digest,
                String::from_utf8(bytes).map_err(|_| "Cannot retain migration receipt.")?
            ],
        )
        .map_err(error)?;

        tx.execute("UPDATE metadata SET revision=revision+1 WHERE id=1", [])
            .map_err(error)?;

        tx.commit().map_err(error)?;

        Ok(digest)
    }
}

fn preflight(root: &Path) -> Result<(), String> {
    preflight_named(root, "runtime.sqlite")
}
fn preflight_named(root: &Path, name: &str) -> Result<(), String> {
    use rusqlite::OpenFlags;

    for suffix in ["", "-wal", "-shm"] {
        let p = root.join(format!("{name}{suffix}"));

        if p.exists() {
            crate::chat::storage::reject_link(&p)?;

            let meta = fs::symlink_metadata(&p).map_err(|_| "Cannot inspect runtime storage.")?;

            if !meta.is_file() || meta.len() > 2 * 1024 * 1024 * 1024 {
                return Err("Runtime storage exceeds its admitted bound.".into());
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;

                if meta.uid() != unsafe { libc::geteuid() }
                    || meta.mode() & 0o077 != 0
                    || meta.nlink() != 1
                {
                    return Err("Runtime storage must be private and independently owned.".into());
                }
            }
        }
    }

    let db = Connection::open_with_flags(
        root.join(name),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(error)?;

    let version: i64 = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(error)?;

    let app: i64 = db
        .pragma_query_value(None, "application_id", |r| r.get(0))
        .map_err(error)?;

    if version == 0 && app == 0 && name == "runtime.sqlite" {
        validate_initialization_intent(root)?;
        let count: i64 = db
            .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))
            .map_err(error)?;
        if count != 0 {
            return Err(
                "Unknown interrupted initialization was preserved without adoption.".into(),
            );
        }
        return Ok(());
    }
    if !matches!(version, 1 | 2) || app != 1280265545 {
        return Err(
            "Unknown Agent runtime schema; storage was preserved without migration.".into(),
        );
    }

    let tables: Vec<String> = {
        let mut stmt = db
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .map_err(error)?;

        let rows = stmt
            .query_map([], |r| r.get(0))
            .map_err(error)?
            .collect::<Result<_, _>>()
            .map_err(error)?;

        rows
    };

    let mut expected_tables = vec![
        "accounts",
        "checkpoints",
        "imports",
        "metadata",
        "outbox",
        "process_ownership",
        "receipts",
        "tasks",
    ];
    if version == 2 {
        expected_tables.push("native_operations");
        expected_tables.sort();
    }
    if tables != expected_tables {
        return Err("Agent runtime schema does not match the owned contract.".into());
    }

    for (table, expected) in [
        ("accounts", vec!["id", "record", "binding"]),
        ("tasks", vec!["id", "record", "shell", "directory_identity"]),
        ("checkpoints", vec!["task_id", "record"]),
        ("receipts", vec!["operation_id", "request", "result"]),
        (
            "outbox",
            vec![
                "operation_id",
                "task_id",
                "attempt_id",
                "generation",
                "state",
            ],
        ),
        (
            "process_ownership",
            vec![
                "task_id",
                "attempt_id",
                "account_id",
                "generation",
                "process_group",
                "identity",
                "ownership_marker",
                "boot",
                "unknown_tools",
                "state",
            ],
        ),
        ("imports", vec!["digest", "record"]),
        ("metadata", vec!["id", "revision"]),
    ] {
        let mut stmt = db
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(error)?;
        let columns = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(error)?;
        if columns != expected {
            return Err(
                "Agent runtime column schema is unknown; original storage was preserved.".into(),
            );
        }
    }

    if version == 2 {
        let mut stmt = db
            .prepare("PRAGMA table_info(native_operations)")
            .map_err(error)?;
        let columns = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(error)?;
        if columns != ["id", "record"] {
            return Err("Unknown native helper ledger schema; storage was preserved.".into());
        }
        validate_native_operations(&db)?;
    }
    let integrity: String = db
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(error)?;

    if integrity != "ok" {
        return Err("Agent runtime storage is corrupt; the original was preserved.".into());
    }

    let _: i64 = db
        .query_row("SELECT revision FROM metadata WHERE id=1", [], |r| r.get(0))
        .map_err(error)?;

    for table in [
        "accounts",
        "tasks",
        "checkpoints",
        "receipts",
        "outbox",
        "imports",
        "process_ownership",
    ] {
        let count: i64 = db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .map_err(error)?;

        if count > 1000000 {
            return Err("Agent runtime record count exceeds its bound.".into());
        }
    }

    for table in ["accounts", "tasks", "checkpoints", "imports"] {
        let max: Option<i64> = db
            .query_row(
                &format!("SELECT max(length(CAST(record AS BLOB))) FROM {table}"),
                [],
                |r| r.get(0),
            )
            .map_err(error)?;

        if max.unwrap_or(0) > 64 * 1024 * 1024 {
            return Err("Agent runtime record exceeds its bound.".into());
        }
    }

    validate_database_records(&db)?;
    Ok(())
}

impl Store {
    #[cfg(any(test, not(target_os = "macos")))]
    pub fn process_intent(&mut self, t: &Task, a: &Attempt) -> Result<String, String> {
        self.process_intent_impl(t, a, false)
            .map(|intent| intent.key)
    }
    #[cfg(target_os = "macos")]
    #[cfg(any(test, feature = "native-smoke"))]
    pub(crate) fn owned_process_intent(&mut self, t: &Task, a: &Attempt) -> Result<String, String> {
        self.process_intent_impl(t, a, true)
            .map(|intent| intent.key)
    }
    #[cfg(target_os = "macos")]
    pub(crate) fn owned_process_intent_context(
        &mut self,
        task: &Task,
        attempt: &Attempt,
    ) -> Result<(String, super::host_child::Context), String> {
        let intent = self.process_intent_impl(task, attempt, true)?;
        Ok((
            intent.key,
            intent
                .context
                .ok_or("Owned intent lacks validated context.")?,
        ))
    }
    fn process_intent_impl(
        &mut self,
        t: &Task,
        a: &Attempt,
        owned: bool,
    ) -> Result<PreparedIntent, String> {
        let operation = if owned {
            let account = self.account(&a.account_id)?;
            let binding = self.binding(&a.account_id)?;
            if account.auth_revision != a.auth_revision
                || binding.auth_revision != a.auth_revision
                || !t.attempts.iter().any(|attempt| {
                    attempt.attempt_id == a.attempt_id
                        && attempt.operation_id == a.operation_id
                        && attempt.account_id == a.account_id
                        && attempt.auth_revision == a.auth_revision
                        && attempt.generation == a.generation
                })
            {
                return Err("Owned attempt provenance is inconsistent.".into());
            }
            Some(NativeOperation {
                host_boundary_owned: true,
                physical_account_root: Some(binding.physical_root.clone()),
                operation_id: a.operation_id.clone(),
                cli: account.cli,
                purpose: "attempt".into(),
                account_id: a.account_id.clone(),
                auth_revision: a.auth_revision,
                task_id: Some(t.task_id.clone()),
                attempt_id: Some(a.attempt_id.clone()),
                generation: Some(a.generation),
                cwd: t.cwd.clone(),
                boot: current_boot()?,
                state: "ownership_unknown".into(),
            })
        } else {
            None
        };
        #[cfg(target_os = "macos")]
        let context = operation
            .clone()
            .map(|op| self.context_for_operation(op))
            .transpose()?;
        let marker = format!("{}{}", super::new_id()?, super::new_id()?);
        #[cfg(unix)]
        let boot = super::process_identity::Identity::read(std::process::id())?.boot;
        #[cfg(not(unix))]
        let boot = String::from("unavailable");
        let tx = self.db.transaction().map_err(error)?;
        tx.execute(
            "INSERT INTO process_ownership VALUES(?1,?2,?3,?4,NULL,NULL,?5,?6,?7,'spawn_intent')",
            params![
                t.task_id,
                a.attempt_id,
                a.account_id,
                i64::try_from(a.generation).map_err(|_| "Generation exhausted.")?,
                marker,
                boot,
                encode(&["unqualified-boundary"])?
            ],
        )
        .map_err(error)?;
        tx.execute(
            "UPDATE tasks SET record=?2 WHERE id=?1",
            params![t.task_id, encode(t)?],
        )
        .map_err(error)?;
        if let Some(operation) = operation {
            tx.execute(
                "INSERT INTO native_operations VALUES(?1,?2)",
                params![operation.operation_id, encode(&operation)?],
            )
            .map_err(error)?;
            tx.execute("UPDATE metadata SET revision=revision+1 WHERE id=1", [])
                .map_err(error)?;
        }
        tx.commit().map_err(error)?;
        Ok(PreparedIntent {
            key: marker,
            #[cfg(target_os = "macos")]
            context,
        })
    }
    pub fn process_spawned(&mut self, attempt: &str, group: u32) -> Result<(), String> {
        self.db
            .execute(
                "UPDATE process_ownership SET process_group=?2,state=CASE WHEN unknown_tools='[]' THEN 'spawned' ELSE 'untracked' END WHERE attempt_id=?1",
                params![attempt, group],
            )
            .map_err(error)?;

        #[cfg(unix)]
        if let Ok(identity) = super::process_identity::Identity::read(group) {
            self.db
                .execute(
                    "UPDATE process_ownership SET identity=?2 WHERE attempt_id=?1",
                    params![attempt, encode(&identity)?],
                )
                .map_err(error)?;
        }

        Ok(())
    }

    pub fn process_untracked(&mut self, attempt: &str) -> Result<(), String> {
        self.insert_unknown_tool(attempt, "unqualified-dispatch")
    }
    pub fn process_tool_untracked(&mut self, attempt: &str, tool: &str) -> Result<(), String> {
        self.insert_unknown_tool(attempt, &format!("tool:{tool}"))
    }
    fn insert_unknown_tool(&mut self, attempt: &str, tool: &str) -> Result<(), String> {
        let raw: String = self
            .db
            .query_row(
                "SELECT unknown_tools FROM process_ownership WHERE attempt_id=?1",
                [attempt],
                |r| r.get(0),
            )
            .optional()
            .map_err(error)?
            .unwrap_or_else(|| "[]".into());
        let mut tools: std::collections::BTreeSet<String> = decode(raw)?;
        tools.insert(tool.into());
        self.db.execute("UPDATE process_ownership SET state='untracked',unknown_tools=?2 WHERE attempt_id=?1",params![attempt,encode(&tools)?]).map_err(error)?;
        Ok(())
    }
    pub fn process_tool_denied(&mut self, attempt: &str, tool: &str) -> Result<(), String> {
        // New wire IDs occupy an injective namespace. Never remove raw legacy
        // keys: they may be reserved lifecycle markers or old unknown effects.
        self.remove_unknown_tool(attempt, &format!("tool:{tool}"))
    }
    pub(super) fn clear_lifecycle_boundary(&mut self, attempt: &str) -> Result<(), String> {
        self.remove_unknown_tool(attempt, "unqualified-boundary")
    }
    fn remove_unknown_tool(&mut self, attempt: &str, tool: &str) -> Result<(), String> {
        let raw: Option<String> = self
            .db
            .query_row(
                "SELECT unknown_tools FROM process_ownership WHERE attempt_id=?1",
                [attempt],
                |r| r.get(0),
            )
            .optional()
            .map_err(error)?;
        if let Some(raw) = raw {
            let mut tools: std::collections::BTreeSet<String> = decode(raw)?;
            tools.remove(tool);
            self.db.execute("UPDATE process_ownership SET unknown_tools=?2,state=CASE WHEN ?3 THEN 'spawned' ELSE state END WHERE attempt_id=?1",params![attempt,encode(&tools)?,tools.is_empty()]).map_err(error)?;
        }
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn test_previous_boot(&mut self, attempt: &str) -> Result<(), String> {
        self.db
            .execute(
                "UPDATE process_ownership SET boot='previous-test-boot' WHERE attempt_id=?1",
                [attempt],
            )
            .map_err(error)?;
        Ok(())
    }

    pub fn process_not_spawned(&mut self, attempt: &str) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        if let Some(completed) = self.owned_attempt_completed(attempt)? {
            let (state, tools): (String, String) = self
                .db
                .query_row(
                    "SELECT state,unknown_tools FROM process_ownership WHERE attempt_id=?1",
                    [attempt],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(error)?;
            if !completed || state == "untracked" || !decode::<Vec<String>>(tools)?.is_empty() {
                return Err("Owned native intent has no verified retirement receipt or retains unknown tool effects.".into());
            }
        }
        self.db.execute("UPDATE process_ownership SET state='drained',unknown_tools='[]' WHERE attempt_id=?1 AND process_group IS NULL",[attempt]).map_err(error)?;

        Ok(())
    }

    pub fn processes_settled(&self, task: &str) -> Result<bool, String> {
        let count: i64 = self
            .db
            .query_row(
                "SELECT count(*) FROM process_ownership WHERE task_id=?1 AND state!='drained'",
                [task],
                |r| r.get(0),
            )
            .map_err(error)?;

        Ok(count == 0)
    }

    pub fn unresolved_accounts(&self) -> Result<Vec<String>, String> {
        let mut stmt = self
            .db
            .prepare("SELECT DISTINCT account_id FROM process_ownership WHERE state!='drained' UNION SELECT json_extract(record,'$.accountId') FROM native_operations WHERE json_extract(record,'$.state')!='reviewed'")
            .map_err(error)?;

        let rows = stmt
            .query_map([], |r| r.get(0))
            .map_err(error)?
            .collect::<Result<_, _>>()
            .map_err(error)?;

        Ok(rows)
    }

    pub fn reconcile_processes(&mut self) -> Result<Vec<String>, String> {
        let rows = {
            let mut stmt=self.db.prepare("SELECT attempt_id,account_id,process_group,ownership_marker,boot,state FROM process_ownership WHERE state!='drained'").map_err(error)?;
            let result = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<u32>>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                    ))
                })
                .map_err(error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(error)?;
            result
        };
        let mut released = vec![];
        #[cfg(unix)]
        let current_boot = super::process_identity::Identity::read(std::process::id())
            .ok()
            .map(|i| i.boot);
        for (attempt, account, group, marker, boot, state) in rows {
            // A verified OS restart proves local process absence. Do not read
            // old cohort journals or query recycled kernel identities; helper
            // and remote-effect review remain separate durable obligations.
            #[cfg(unix)]
            if current_boot
                .as_ref()
                .is_some_and(|current| &boot != current)
            {
                self.db
                    .execute(
                        "UPDATE process_ownership SET state='drained' WHERE attempt_id=?1",
                        [&attempt],
                    )
                    .map_err(error)?;
                released.push(account);
                continue;
            }
            #[cfg(target_os = "macos")]
            if let Some(completed) = self.owned_attempt_completed(&attempt)? {
                if completed && state != "untracked" {
                    let unknown: String = self
                        .db
                        .query_row(
                            "SELECT unknown_tools FROM process_ownership WHERE attempt_id=?1",
                            [&attempt],
                            |row| row.get(0),
                        )
                        .map_err(error)?;
                    if decode::<Vec<String>>(unknown)?.is_empty() {
                        self.db
                            .execute(
                                "UPDATE process_ownership SET state='drained' WHERE attempt_id=?1",
                                [&attempt],
                            )
                            .map_err(error)?;
                        released.push(account);
                    }
                }
                continue;
            }
            #[cfg(unix)]
            let absent = current_boot
                .as_ref()
                .is_some_and(|current| &boot != current)
                || state != "untracked"
                    && group.is_some_and(group_absent)
                    && super::process_supervision::remaining(&marker)
                        .is_ok_and(|remaining| remaining.is_empty());
            #[cfg(not(unix))]
            let absent = {
                let _ = (&group, &marker, &boot, &state);
                false
            };
            if absent {
                self.db
                    .execute(
                        "UPDATE process_ownership SET state='drained' WHERE attempt_id=?1",
                        [attempt],
                    )
                    .map_err(error)?;
                released.push(account);
            }
        }
        Ok(released)
    }
    pub fn process_markers(&self, task: &str) -> Result<Vec<String>, String> {
        let mut stmt=self.db.prepare("SELECT ownership_marker FROM process_ownership WHERE task_id=?1 AND state!='drained'").map_err(error)?;
        let result = stmt
            .query_map([task], |r| r.get(0))
            .map_err(error)?
            .collect::<Result<_, _>>()
            .map_err(error)?;
        Ok(result)
    }
}

fn group_absent(group: u32) -> bool {
    #[cfg(unix)]
    {
        if group < 2 || group > i32::MAX as u32 {
            return false;
        }

        let result = unsafe { libc::kill(-(group as i32), 0) };

        result < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }

    #[cfg(not(unix))]
    {
        let _ = group;

        false
    }
}

fn read_records<T: DeserializeOwned>(db: &Connection, table: &str) -> Result<Vec<T>, String> {
    let mut stmt = db
        .prepare(&format!("SELECT record FROM {table} ORDER BY id"))
        .map_err(error)?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(error)?;
    rows.into_iter().map(decode).collect()
}
fn validate_database_records(db: &Connection) -> Result<(), String> {
    let accounts: Vec<AccountInstance> = read_records(db, "accounts")?;

    let tasks: Vec<Task> = read_records(db, "tasks")?;

    if accounts.len() > 10000 || tasks.len() > 100000 {
        return Err("Runtime record count exceeds its admitted bound.".into());
    }

    for a in &accounts {
        super::valid_operation(&a.account_id)?;

        if a.revision == 0 || a.auth_revision == 0 {
            return Err("Invalid account revision.".into());
        }

        let b: CredentialBinding = decode(
            db.query_row(
                "SELECT binding FROM accounts WHERE id=?1",
                [&a.account_id],
                |r| r.get(0),
            )
            .map_err(error)?,
        )?;

        if b.account_id != a.account_id || b.auth_revision != a.auth_revision {
            return Err("Credential binding identity does not match its account.".into());
        }

        if !b.physical_root.is_empty() && !Path::new(&b.physical_root).is_absolute() {
            return Err("Credential namespace is not an owned absolute path.".into());
        }
    }

    for t in &tasks {
        super::valid_operation(&t.task_id)?;

        if t.revision == 0
            || t.history_revision != t.history.last().map(|h| h.sequence).unwrap_or(0)
            || t.history
                .iter()
                .enumerate()
                .any(|(n, h)| h.sequence != n as u64 + 1)
            || t.history.len() > 1000000
        {
            return Err("Task journal sequence requires recovery.".into());
        }

        let mut attempts = std::collections::HashSet::new();

        for a in &t.attempts {
            if !attempts.insert(&a.attempt_id)
                || a.generation > t.generation
                || a.auth_revision == 0
            {
                return Err("Attempt provenance requires recovery.".into());
            }
        }

        if let Some(c) = db
            .query_row(
                "SELECT record FROM checkpoints WHERE task_id=?1",
                [&t.task_id],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(error)?
            .map(decode::<HistoryCheckpoint>)
            .transpose()?
        {
            if c.task_id != t.task_id
                || !t.attempts.iter().any(|a| {
                    a.attempt_id == c.attempt_id
                        && a.account_id == c.account_id
                        && a.auth_revision == c.auth_revision
                        && a.generation == c.generation
                })
                || c.history_revision > t.history_revision
            {
                return Err("Checkpoint provenance requires recovery.".into());
            }
        }
    }

    let process_rows = {
        let mut stmt=db.prepare("SELECT task_id,attempt_id,account_id,generation,process_group,ownership_marker,state,boot FROM process_ownership").map_err(error)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<u32>>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                ))
            })
            .map_err(error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(error)?;
        rows
    };
    for (task, attempt, account, generation, group, marker, state, boot) in process_rows {
        if boot.is_empty()
            || boot.len() > 1024
            || marker.len() != 64
            || !marker.bytes().all(|b| b.is_ascii_hexdigit())
            || group.is_some_and(|p| p < 2 || p > i32::MAX as u32)
            || !matches!(
                state.as_str(),
                "spawn_intent" | "spawned" | "drained" | "untracked"
            )
            || !tasks.iter().any(|t| {
                t.task_id == task
                    && t.attempts.iter().any(|a| {
                        a.attempt_id == attempt
                            && a.account_id == account
                            && i64::try_from(a.generation).ok() == Some(generation)
                    })
            })
        {
            return Err(
                "Native process provenance requires recovery; original storage was preserved."
                    .into(),
            );
        }
    }

    Ok(())
}

fn current_boot() -> Result<String, String> {
    #[cfg(unix)]
    {
        Ok(super::process_identity::Identity::read(std::process::id())?.boot)
    }
    #[cfg(not(unix))]
    {
        Err("Native ownership inspection is unavailable on this platform.".into())
    }
}
fn validate_native_operations(db: &Connection) -> Result<(), String> {
    let mut stmt = db
        .prepare("SELECT id,record FROM native_operations")
        .map_err(error)?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(error)?;
    for (index, row) in rows.enumerate() {
        if index >= 1_000_000 {
            return Err("Native helper ledger exceeds its bound.".into());
        }
        let (id, record) = row.map_err(error)?;
        if record.len() > 64 * 1024 {
            return Err("Native helper record exceeds its bound.".into());
        }
        let op: NativeOperation = decode(record)?;
        super::valid_operation(&op.operation_id)?;
        super::valid_operation(&op.account_id)?;
        if let Some(task) = &op.task_id {
            super::valid_operation(task)?;
            if op.attempt_id.is_none() || op.generation.is_none_or(|g| g == 0) {
                return Err("Native helper has incomplete historical attempt provenance.".into());
            }
        }
        if let Some(attempt) = &op.attempt_id {
            super::valid_operation(attempt)?;
        }
        if op.host_boundary_owned
            && op.physical_account_root.as_ref().is_none_or(|root| {
                !Path::new(root).is_absolute() || root.chars().any(char::is_control)
            })
        {
            return Err("Owned native operation has no immutable physical account root.".into());
        }
        if op.operation_id != id
            || id.is_empty()
            || id.len() > 200
            || op.auth_revision == 0
            || op.boot.is_empty()
            || op.boot.len() > 1024
            || !Path::new(&op.cwd).is_absolute()
            || op.cwd.len() > 16 * 1024
            || !matches!(op.state.as_str(), "ownership_unknown" | "reviewed")
            || !matches!(
                op.purpose.as_str(),
                "attempt_prepare" | "verify" | "account_terminal" | "attempt"
            )
        {
            return Err(
                "Native helper provenance requires recovery; original storage was preserved."
                    .into(),
            );
        }
        let account: Option<String> = db
            .query_row(
                "SELECT record FROM accounts WHERE id=?1",
                [&op.account_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(error)?;
        if let Some(raw) = account {
            let a: AccountInstance = decode(raw)?;
            if op.auth_revision > a.auth_revision || op.cli != a.cli {
                return Err("Native helper auth or driver provenance is unknown.".into());
            }
        } else if op.state != "reviewed" {
            return Err("Unreviewed native helper account is missing.".into());
        }
        if let Some(task) = op.task_id.as_ref() {
            let raw: Option<String> = db
                .query_row("SELECT record FROM tasks WHERE id=?1", [task], |r| r.get(0))
                .optional()
                .map_err(error)?;
            if let Some(raw) = raw {
                let t: Task = decode(raw)?;
                if !t.attempts.iter().any(|a| {
                    Some(&a.attempt_id) == op.attempt_id.as_ref()
                        && Some(a.generation) == op.generation
                        && a.account_id == op.account_id
                        && a.auth_revision == op.auth_revision
                }) {
                    return Err("Native helper attempt provenance is unknown.".into());
                }
            } else if op.state != "reviewed" {
                return Err("Unreviewed native helper task is missing.".into());
            }
        } else if op.attempt_id.is_some() || op.generation.is_some() {
            return Err("Native helper has incomplete task provenance.".into());
        }
    }
    Ok(())
}
impl Store {
    pub(super) fn native_operations(&self) -> Result<Vec<NativeOperation>, String> {
        let mut stmt = self
            .db
            .prepare("SELECT record FROM native_operations ORDER BY id")
            .map_err(error)?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(error)?;
        rows.into_iter().map(decode).collect()
    }
    pub(super) fn helper_intent(
        &mut self,
        purpose: &str,
        account: &AccountInstance,
        cwd: &str,
        attempt: Option<(&str, &Attempt)>,
    ) -> Result<String, String> {
        self.helper_intent_impl(purpose, account, cwd, attempt, false)
            .map(|intent| intent.key)
    }
    #[cfg(target_os = "macos")]
    #[cfg(any(test, feature = "native-smoke"))]
    pub(crate) fn owned_helper_intent(
        &mut self,
        purpose: &str,
        account: &AccountInstance,
        cwd: &str,
        attempt: Option<(&str, &Attempt)>,
    ) -> Result<String, String> {
        self.helper_intent_impl(purpose, account, cwd, attempt, true)
            .map(|intent| intent.key)
    }
    #[cfg(target_os = "macos")]
    pub(crate) fn owned_helper_intent_context(
        &mut self,
        purpose: &str,
        account: &AccountInstance,
        cwd: &str,
    ) -> Result<super::host_child::Context, String> {
        let intent = self.helper_intent_impl(purpose, account, cwd, None, true)?;
        intent
            .context
            .ok_or_else(|| "Owned intent lacks validated context.".into())
    }
    fn helper_intent_impl(
        &mut self,
        purpose: &str,
        account: &AccountInstance,
        cwd: &str,
        attempt: Option<(&str, &Attempt)>,
        owned: bool,
    ) -> Result<PreparedIntent, String> {
        if owned {
            let durable = self.account(&account.account_id)?;
            let binding = self.binding(&account.account_id)?;
            if durable.cli != account.cli
                || durable.auth_revision != account.auth_revision
                || binding.auth_revision != account.auth_revision
            {
                return Err("Owned helper account binding changed before intent.".into());
            }
        }
        if !super::native_accounts::available(account.cli) {
            return Err("This native account is unavailable on the current platform; no helper was launched.".into());
        }
        if cwd.len() > 16 * 1024
            || cwd.chars().any(char::is_control)
            || !Path::new(cwd).is_absolute()
        {
            return Err("Native helper workspace is not admitted; no process was launched.".into());
        }
        let cwd = Path::new(cwd)
            .canonicalize()
            .map_err(|_| "Native helper workspace cannot be admitted.")?;
        let id = super::new_id()?;
        let op = NativeOperation {
            host_boundary_owned: owned,
            physical_account_root: if owned {
                Some(self.binding(&account.account_id)?.physical_root)
            } else {
                None
            },
            operation_id: id.clone(),
            cli: account.cli,
            purpose: purpose.into(),
            account_id: account.account_id.clone(),
            auth_revision: account.auth_revision,
            task_id: attempt.map(|(t, _)| t.into()),
            attempt_id: attempt.map(|(_, a)| a.attempt_id.clone()),
            generation: attempt.map(|(_, a)| a.generation),
            cwd: cwd.to_str().ok_or("Native workspace is not UTF-8.")?.into(),
            boot: current_boot()?,
            state: "ownership_unknown".into(),
        };
        #[cfg(target_os = "macos")]
        let context = if owned {
            Some(self.context_for_operation(op.clone())?)
        } else {
            None
        };
        let record = encode(&op)?;
        let tx = self.db.transaction().map_err(error)?;
        tx.execute(
            "INSERT INTO native_operations VALUES(?1,?2)",
            params![id, record],
        )
        .map_err(error)?;
        tx.execute("UPDATE metadata SET revision=revision+1 WHERE id=1", [])
            .map_err(error)?;
        tx.commit().map_err(error)?;
        Ok(PreparedIntent {
            key: id,
            #[cfg(target_os = "macos")]
            context,
        })
    }
    pub(super) fn helper_requires_live_fence(
        &self,
        operation: &NativeOperation,
    ) -> Result<bool, String> {
        Ok(operation.state != "reviewed" && operation.boot == current_boot()?)
    }
    pub(super) fn account_recovery(&self, id: &str) -> Result<Option<AccountRecovery>, String> {
        let ops = self
            .native_operations()?
            .into_iter()
            .filter(|o| o.account_id == id && o.state != "reviewed")
            .collect::<Vec<_>>();
        if ops.is_empty() {
            return Ok(None);
        }
        let boot = current_boot()?;
        let recoverable = ops.iter().all(|o| o.boot != boot);
        Ok(Some(AccountRecovery{state:if recoverable{"effects_review_required"}else{"ownership_unknown"}.into(),reason:if recoverable{"Review the interrupted native helper effects before using this account again."}else{"Native setup, verification or login may have started background work. This account and workspace remain protected until Lomi verifies an OS restart."}.into(),operation_ids:ops.into_iter().map(|o|o.operation_id).collect(),recoverable,requires_verified_boot_change:!recoverable}))
    }
    pub(super) fn helpers_settled(&self, task: Option<&str>) -> Result<bool, String> {
        Ok(!self
            .native_operations()?
            .iter()
            .any(|o| o.state != "reviewed" && task.is_none_or(|t| o.task_id.as_deref() == Some(t))))
    }
    pub(super) fn recover_helpers(
        &mut self,
        request: &AccountRecover,
    ) -> Result<AccountsSnapshot, String> {
        if let Some(result) = self.replay(&request.operation_id, request)? {
            return Ok(result);
        }
        let recovery = self
            .account_recovery(&request.account_id)?
            .ok_or("No native helper recovery is pending.")?;
        if !recovery.recoverable || !request.acknowledge_effects {
            return Err("A verified OS restart and explicit effects review are required; acknowledgement cannot release live or unknown ownership.".into());
        }
        let mut a = self.account(&request.account_id)?;
        if a.revision != request.expected_revision {
            return Err("The account changed; review recovery again.".into());
        }
        a.revision = a.revision.checked_add(1).ok_or("Revision exhausted.")?;
        a.auth_revision = a
            .auth_revision
            .checked_add(1)
            .ok_or("Auth revision exhausted.")?;
        a.auth_state = "unverified".into();
        a.recovery = None;
        let mut binding = self.binding(&a.account_id)?;
        binding.auth_revision = a.auth_revision;
        let mut tasks = self.tasks()?;
        for task in &mut tasks {
            let old = task.grants.len();
            task.grants.retain(|g| g.account_id != a.account_id);
            if old != task.grants.len() {
                task.revision = task
                    .revision
                    .checked_add(1)
                    .ok_or("Task revision exhausted.")?;
            }
        }
        let task_records = tasks
            .iter()
            .map(|t| Ok((t.task_id.clone(), encode(t)?)))
            .collect::<Result<Vec<_>, String>>()?;
        let mut accounts = self.accounts()?;
        *accounts
            .iter_mut()
            .find(|v| v.account_id == a.account_id)
            .unwrap() = a.clone();
        for account in &mut accounts {
            account.recovery = if account.account_id == a.account_id {
                None
            } else {
                self.account_recovery(&account.account_id)?
            };
        }
        let result = AccountsSnapshot {
            schema: 1,
            revision: self
                .revision()?
                .checked_add(1)
                .ok_or("Revision exhausted.")?,
            accounts,
            capabilities: super::service::capabilities(),
        };
        let record = encode(&a)?;
        let binding = encode(&binding)?;
        let request_record = encode(request)?;
        let result_record = encode(&result)?;
        let mut ops = self.native_operations()?;
        for op in &mut ops {
            if op.account_id == a.account_id {
                op.state = "reviewed".into();
            }
        }
        let op_records = ops
            .iter()
            .map(|o| Ok((o.operation_id.clone(), encode(o)?)))
            .collect::<Result<Vec<_>, String>>()?;
        let tx = self.db.transaction().map_err(error)?;
        tx.execute(
            "UPDATE accounts SET record=?2,binding=?3 WHERE id=?1",
            params![a.account_id, record, binding],
        )
        .map_err(error)?;
        for (id, record) in task_records {
            tx.execute(
                "UPDATE tasks SET record=?2 WHERE id=?1",
                params![id, record],
            )
            .map_err(error)?;
        }
        for (id, record) in op_records {
            tx.execute(
                "UPDATE native_operations SET record=?2 WHERE id=?1",
                params![id, record],
            )
            .map_err(error)?;
        }
        tx.execute(
            "UPDATE metadata SET revision=?1 WHERE id=1",
            [i64::try_from(result.revision).map_err(|_| "Revision exhausted.")?],
        )
        .map_err(error)?;
        tx.execute(
            "INSERT INTO receipts VALUES(?1,?2,?3)",
            params![request.operation_id, request_record, result_record],
        )
        .map_err(error)?;
        tx.commit().map_err(error)?;
        Ok(result)
    }
}

fn schema_one_digest(db: &Connection) -> Result<String, String> {
    use rusqlite::types::ValueRef;
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    fn add(context: &mut ring::digest::Context, bytes: &[u8]) {
        context.update(&(bytes.len() as u64).to_le_bytes());
        context.update(bytes);
    }
    let mut schema = db
        .prepare("SELECT name,sql FROM sqlite_master WHERE type='table' ORDER BY name")
        .map_err(error)?;
    let entries = schema
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(error)?;
    for (name, sql) in entries {
        if ![
            "accounts",
            "tasks",
            "checkpoints",
            "receipts",
            "outbox",
            "process_ownership",
            "imports",
            "metadata",
        ]
        .contains(&name.as_str())
        {
            return Err("Unexpected schema-one table; snapshot was preserved.".into());
        }
        add(&mut context, name.as_bytes());
        add(&mut context, sql.as_bytes());
        let mut stmt = db
            .prepare(&format!("SELECT * FROM {name} ORDER BY 1"))
            .map_err(error)?;
        let count = stmt.column_count();
        let mut rows = stmt.query([]).map_err(error)?;
        while let Some(row) = rows.next().map_err(error)? {
            for column in 0..count {
                match row.get_ref(column).map_err(error)? {
                    ValueRef::Null => add(&mut context, b"null"),
                    ValueRef::Integer(v) => {
                        add(&mut context, b"integer");
                        add(&mut context, &v.to_le_bytes());
                    }
                    ValueRef::Real(v) => {
                        add(&mut context, b"real");
                        add(&mut context, &v.to_bits().to_le_bytes());
                    }
                    ValueRef::Text(v) => {
                        add(&mut context, b"text");
                        add(&mut context, v);
                    }
                    ValueRef::Blob(v) => {
                        add(&mut context, b"blob");
                        add(&mut context, v);
                    }
                }
            }
        }
    }
    Ok(context
        .finish()
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
pub(super) fn prepare_schema_one_upgrade(db: &Connection, root: &Path) -> Result<(), String> {
    let backup = root.join("schema-1-backup.sqlite");
    let marker = root.join("schema-1-upgrade.json");
    let expected =
        serde_json::json!({"schema":1,"from":1,"to":2,"sourceDigest":schema_one_digest(db)?});
    if marker.exists() {
        crate::chat::storage::reject_link(&marker)?;
        let meta =
            fs::symlink_metadata(&marker).map_err(|_| "Cannot inspect schema upgrade intent.")?;
        if !meta.is_file() || meta.len() > 1024 {
            return Err("Unknown schema upgrade intent was preserved.".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.uid() != unsafe { libc::geteuid() }
                || meta.mode() & 0o077 != 0
                || meta.nlink() != 1
            {
                return Err("Schema upgrade intent is not private and owned.".into());
            }
        }
        let found: serde_json::Value = serde_json::from_slice(
            &fs::read(&marker).map_err(|_| "Cannot read schema upgrade intent.")?,
        )
        .map_err(|_| "Unknown schema upgrade intent was preserved.")?;
        if found != expected {
            return Err("Schema-one source changed after its retained upgrade intent; original data and backup were preserved.".into());
        }
    } else {
        if backup.exists() {
            return Err("Unidentified schema backup was preserved without adoption.".into());
        }
        crate::chat::storage::atomic(&marker, encode(&expected)?.as_bytes())?;
    }
    let staged = root.join("schema-1-backup.staged.sqlite");
    crate::chat::storage::reject_link(&staged)?;
    // persist_noclobber-style publication uses an exclusive hard-link. A crash
    // between link and unlink can leave the two journal-owned names linked;
    // validate the exact snapshot before completing that unlink on restart.
    #[cfg(unix)]
    if backup.exists() && staged.exists() {
        use std::os::unix::fs::MetadataExt;
        crate::chat::storage::reject_link(&backup)?;
        crate::chat::storage::reject_link(&staged)?;
        let a = fs::metadata(&backup).map_err(|_| "Cannot inspect schema backup publication.")?;
        let b = fs::metadata(&staged).map_err(|_| "Cannot inspect schema backup stage.")?;
        if a.is_file()
            && b.is_file()
            && a.dev() == b.dev()
            && a.ino() == b.ino()
            && a.nlink() == 2
            && a.uid() == unsafe { libc::geteuid() }
            && a.mode() & 0o077 == 0
        {
            let snapshot =
                Connection::open_with_flags(&backup, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                    .map_err(error)?;
            if schema_one_digest(&snapshot)? != schema_one_digest(db)? {
                return Err(
                    "Interrupted backup publication is unknown; original files were preserved."
                        .into(),
                );
            }
            drop(snapshot);
            fs::remove_file(&staged).map_err(|_| "Cannot complete exact backup publication.")?;
            fs::File::open(root)
                .and_then(|f| f.sync_all())
                .map_err(|_| "Cannot synchronize backup publication.")?;
        }
    }
    if !backup.exists() {
        if staged.exists() {
            crate::chat::storage::reject_link(&staged)?;
            let m = fs::symlink_metadata(&staged)
                .map_err(|_| "Cannot inspect interrupted backup stage.")?;
            if !m.is_file() || m.len() > 2 * 1024 * 1024 * 1024 {
                return Err("Unknown backup stage was preserved.".into());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if m.uid() != unsafe { libc::geteuid() } || m.nlink() != 1 || m.mode() & 0o022 != 0
                {
                    return Err("Unknown backup stage ownership was preserved.".into());
                }
            }
            let valid = preflight_named(root, "schema-1-backup.staged.sqlite").is_ok()
                && Connection::open_with_flags(&staged, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                    .ok()
                    .and_then(|snapshot| schema_one_digest(&snapshot).ok())
                    .is_some_and(|digest| schema_one_digest(db).ok() == Some(digest));
            if !valid {
                // Never overwrite/delete a partial snapshot. Its journal-owned
                // name is archived before creating a fresh complete snapshot.
                fs::rename(
                    &staged,
                    root.join(format!(
                        "schema-1-backup.partial-{}.sqlite",
                        super::new_id()?
                    )),
                )
                .map_err(|_| "Cannot preserve interrupted backup stage.")?;
                fs::File::open(root)
                    .and_then(|f| f.sync_all())
                    .map_err(|_| "Cannot synchronize preserved backup stage.")?;
            }
        }
        if !staged.exists() {
            db.execute(
                "VACUUM INTO ?1",
                [staged.to_str().ok_or("Runtime path is not UTF-8.")?],
            )
            .map_err(error)?;
            crate::chat::storage::private(&staged, false)?;
            fs::File::open(&staged)
                .and_then(|f| f.sync_all())
                .map_err(|_| "Cannot synchronize schema backup stage.")?;
        }
        preflight_named(root, "schema-1-backup.staged.sqlite")?;
        let snapshot =
            Connection::open_with_flags(&staged, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(error)?;
        if schema_one_digest(&snapshot)? != schema_one_digest(db)? {
            return Err(
                "Staged backup does not match its source; no publication was applied.".into(),
            );
        }
        drop(snapshot);
        fs::hard_link(&staged, &backup)
            .map_err(|_| "Schema backup publication refused to overwrite an existing file.")?;
        fs::File::open(root)
            .and_then(|f| f.sync_all())
            .map_err(|_| "Cannot synchronize schema backup publication.")?;
        fs::remove_file(&staged).map_err(|_| "Cannot complete schema backup publication.")?;
        fs::File::open(root)
            .and_then(|f| f.sync_all())
            .map_err(|_| "Cannot synchronize schema backup namespace.")?;
    }
    preflight_named(root, "schema-1-backup.sqlite")?;
    let snapshot = Connection::open_with_flags(&backup, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(error)?;
    let version: i64 = snapshot
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(error)?;
    if version != 1 || schema_one_digest(&snapshot)? != schema_one_digest(db)? {
        return Err(
            "Retained schema-one backup does not match its exact source; no upgrade was applied."
                .into(),
        );
    }
    Ok(())
}

fn initialization_intent(root: &Path) -> Result<serde_json::Value, String> {
    let canonical = root
        .canonicalize()
        .map_err(|_| "Cannot identify runtime initialization directory.")?;
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        let m = fs::metadata(root).map_err(|_| "Cannot inspect initialization directory.")?;
        (m.dev(), m.ino())
    };
    #[cfg(not(unix))]
    let identity = (0u64, 0u64);
    Ok(
        serde_json::json!({"schema":1,"targetVersion":2,"applicationId":1280265545,"directory":canonical,"identity":identity}),
    )
}
fn validate_initialization_intent(root: &Path) -> Result<(), String> {
    let path = root.join("initialization.json");
    crate::chat::storage::reject_link(&path)?;
    let m = fs::symlink_metadata(&path)
        .map_err(|_| "Unknown empty runtime schema was preserved without initialization intent.")?;
    if !m.is_file() || m.len() > 2048 {
        return Err("Unknown runtime initialization intent was preserved.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 || m.nlink() != 1 {
            return Err("Runtime initialization intent is not private and owned.".into());
        }
    }
    let found: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).map_err(|_| "Cannot read initialization intent.")?)
            .map_err(|_| "Unknown initialization intent was preserved.")?;
    if found != initialization_intent(root)? {
        return Err(
            "Runtime initialization directory identity changed; original bytes were preserved."
                .into(),
        );
    }
    Ok(())
}
pub(super) fn publish_initialization_intent(root: &Path) -> Result<(), String> {
    let path = root.join("initialization.json");
    if path.exists() {
        validate_initialization_intent(root)
    } else {
        crate::chat::storage::atomic(&path, encode(&initialization_intent(root)?)?.as_bytes())
    }
}
