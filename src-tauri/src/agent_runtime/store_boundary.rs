//! Durable native boundary provenance. This is a private Store child module so
//! ledger publication remains inside the existing database owner/transaction.
use super::{current_boot, decode, encode, error, workspace, Store};
use crate::agent_runtime::{
    host_boundary, host_child::Context, native_accounts, types::NativeOperation,
};
use rusqlite::{params, OptionalExtension};
use std::path::{Path, PathBuf};

impl Store {
    fn owned_operation(&self, id: &str) -> Result<NativeOperation, String> {
        let operation: NativeOperation = decode(
            self.db
                .query_row(
                    "SELECT record FROM native_operations WHERE id=?1",
                    [id],
                    |row| row.get(0),
                )
                .map_err(error)?,
        )?;
        if !operation.host_boundary_owned || operation.operation_id != id {
            return Err("Native operation has no durable host boundary ownership.".into());
        }
        Ok(operation)
    }
    pub(crate) fn host_context(&self, id: &str) -> Result<Context, String> {
        self.context_for_operation(self.owned_operation(id)?)
    }
    pub(super) fn context_for_operation(
        &self,
        operation: NativeOperation,
    ) -> Result<Context, String> {
        #[cfg(test)]
        if self.fail_next_context.replace(false) {
            return Err("Injected pre-intent context read failure.".into());
        }
        if operation.state == "reviewed" {
            return Err("Native operation is already complete; a new durable intent is required before launch.".into());
        }
        if operation.boot != current_boot()? {
            return Err(
                "Native operation belongs to an earlier OS boot; explicit recovery is required."
                    .into(),
            );
        }
        let account = self.account(&operation.account_id)?;
        let binding = self.binding(&operation.account_id)?;
        if account.cli != operation.cli
            || account.auth_revision != operation.auth_revision
            || binding.account_id != operation.account_id
            || binding.auth_revision != operation.auth_revision
        {
            return Err("Native operation account/auth binding changed.".into());
        }
        let project_root = PathBuf::from(&operation.cwd);
        let (physical, identity) = workspace(&project_root)?;
        if Path::new(&physical) != project_root {
            return Err("Native operation project root is no longer physical.".into());
        }
        if let Some(task_id) = &operation.task_id {
            let task = self.task(task_id)?;
            let attempt = task
                .attempts
                .iter()
                .find(|attempt| Some(&attempt.attempt_id) == operation.attempt_id.as_ref())
                .ok_or("Native operation attempt is missing.")?;
            if task.cwd != operation.cwd
                || attempt.account_id != operation.account_id
                || attempt.auth_revision != operation.auth_revision
                || Some(attempt.generation) != operation.generation
            {
                return Err("Native operation durable attempt changed.".into());
            }
            let saved: String = self
                .db
                .query_row(
                    "SELECT directory_identity FROM tasks WHERE id=?1",
                    [task_id],
                    |row| row.get(0),
                )
                .map_err(error)?;
            if decode::<(u64, u64)>(saved)? != identity {
                return Err("Native operation project directory was replaced.".into());
            }
        }
        let physical_account_root = PathBuf::from(
            operation
                .physical_account_root
                .as_ref()
                .ok_or("Owned native operation has no immutable physical account root.")?,
        );
        if physical_account_root != std::path::Path::new(&binding.physical_root)
            || !physical_account_root.is_absolute()
            || std::fs::symlink_metadata(&physical_account_root).is_err()
        {
            return Err(
                "Native operation account namespace no longer matches its stable durable binding."
                    .into(),
            );
        }
        native_accounts::check_private_directory(&physical_account_root)?;
        if physical_account_root
            .canonicalize()
            .map_err(|_| "Native account root cannot be resolved.")?
            != physical_account_root
        {
            return Err("Native account root must remain physical.".into());
        }
        Ok(Context {
            parent_operation_id: operation.operation_id,
            account_id: operation.account_id,
            auth_revision: operation.auth_revision,
            task_id: operation.task_id,
            attempt_id: operation.attempt_id,
            generation: operation.generation,
            project_root,
            physical_account_root,
            storage_root: self.root.clone(),
        })
    }
    fn owned_parent_completed(&self, id: &str) -> Result<bool, String> {
        let operation = self.owned_operation(id)?;
        if operation.boot != current_boot()? {
            return Ok(false);
        }
        let purpose = operation.purpose.clone();
        let context = Context {
            parent_operation_id: id.into(),
            account_id: operation.account_id,
            auth_revision: operation.auth_revision,
            task_id: operation.task_id,
            attempt_id: operation.attempt_id,
            generation: operation.generation,
            project_root: PathBuf::from(operation.cwd),
            physical_account_root: PathBuf::from(
                operation
                    .physical_account_root
                    .ok_or("Owned native receipt lacks immutable account root provenance.")?,
            ),
            storage_root: self.root.clone(),
        };
        let scopes = host_boundary::parent_scopes(&self.root, id)?;
        for scope in &scopes {
            use host_boundary::Purpose;
            let allowed = match purpose.as_str() {
                "attempt" => matches!(
                    scope.purpose,
                    Purpose::VersionProbe | Purpose::Resolver | Purpose::Attempt
                ),
                "verify" => matches!(
                    scope.purpose,
                    Purpose::VersionProbe | Purpose::Resolver | Purpose::Collector
                ),
                "account_terminal" => matches!(
                    scope.purpose,
                    Purpose::VersionProbe | Purpose::Resolver | Purpose::AccountTerminal
                ),
                "attempt_prepare" => {
                    matches!(scope.purpose, Purpose::VersionProbe | Purpose::Resolver)
                }
                _ => false,
            };
            if !allowed
                || scope.parent_operation_id.as_deref() != Some(id)
                || scope.account_id != context.account_id
                || scope.auth_revision != context.auth_revision
                || scope.task_id != context.task_id
                || scope.attempt_id != context.attempt_id
                || scope.generation != context.generation
                || scope.project_root != context.project_root
                || scope.physical_account_root.as_ref() != Some(&context.physical_account_root)
                || scope.storage_root.as_ref() != Some(&context.storage_root)
            {
                return Err(
                    "Native boundary receipt differs from its durable owner provenance.".into(),
                );
            }
        }
        // Empty journals prove zero execution only for explicitly owned intents:
        // every route commits intent before boundary publication and has no
        // unjournaled subprocess fallback. Legacy intents never reach this path.
        host_boundary::parent_completed(&self.root, id)
    }
    pub(crate) fn complete_owned_helper(&mut self, id: &str) -> Result<(), String> {
        let operation = self.owned_operation(id)?;
        if operation.state == "reviewed" {
            return Ok(());
        }
        self.host_context(id)?;
        // The caller settles children outside the runtime owner mutex. This
        // database phase only reads receipts, avoiding Stop/MCP lock inversion.
        if !self.owned_parent_completed(id)? {
            return Err("Native helper boundary retirement remains unconfirmed.".into());
        }
        self.mark_owned_reviewed(id)
    }
    fn mark_owned_reviewed(&mut self, id: &str) -> Result<(), String> {
        let mut operation = self.owned_operation(id)?;
        if operation.state == "reviewed" {
            return Ok(());
        }
        operation.state = "reviewed".into();
        let tx = self.db.transaction().map_err(error)?;
        // A sealed local cohort proves helper retirement only. Clearing an
        // attempt's generic effects marker requires a complete observed native
        // journal, or the explicit no-Attempt rejection path.
        tx.execute(
            "UPDATE native_operations SET record=?2 WHERE id=?1",
            params![id, encode(&operation)?],
        )
        .map_err(error)?;
        tx.execute("UPDATE metadata SET revision=revision+1 WHERE id=1", [])
            .map_err(error)?;
        tx.commit().map_err(error)
    }
    pub(crate) fn reconcile_owned_helpers(&mut self) -> Result<(), String> {
        let boot = current_boot()?;
        for operation in self.native_operations()? {
            if operation.host_boundary_owned
                && operation.state != "reviewed"
                && operation.boot == boot
                && host_boundary::parent_scopes(&self.root, &operation.operation_id)
                    .is_ok_and(|scopes| !scopes.is_empty())
                && self
                    .owned_parent_completed(&operation.operation_id)
                    .unwrap_or(false)
            {
                self.mark_owned_reviewed(&operation.operation_id)?;
            }
        }
        Ok(())
    }
    pub(super) fn owned_attempt_completed(&self, attempt: &str) -> Result<Option<bool>, String> {
        let raw:Option<String>=self.db.query_row("SELECT record FROM native_operations WHERE json_extract(record,'$.attemptId')=?1 AND json_extract(record,'$.purpose')='attempt'",[attempt],|row|row.get(0)).optional().map_err(error)?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        let operation: NativeOperation = decode(raw)?;
        if !operation.host_boundary_owned {
            return Ok(None);
        }
        if operation.state != "reviewed" || operation.boot != current_boot()? {
            return Ok(Some(false));
        }
        Ok(Some(
            self.owned_parent_completed(&operation.operation_id)
                .unwrap_or(false),
        ))
    }
}

#[cfg(all(test, target_arch = "aarch64"))]
mod tests {
    use super::*;
    use crate::{
        agent_runtime::types::{AccountInstance, CredentialBinding},
        cli_catalog::TitleCli,
    };
    fn fixture() -> (tempfile::TempDir, Store, AccountInstance, String) {
        let temporary = tempfile::tempdir().unwrap();
        let mut store = Store::open(temporary.path().join("owner")).unwrap();
        let account = AccountInstance {
            account_id: "owned".into(),
            cli: TitleCli::Codex,
            label: "Fixture".into(),
            enabled: true,
            revision: 1,
            auth_revision: 1,
            auth_state: "unverified".into(),
            availability_reason: None,
            accepted_version: None,
            recovery: None,
        };
        native_accounts::check_private_directory(&store.root.join("accounts/owned")).unwrap();
        let binding = CredentialBinding {
            account_id: account.account_id.clone(),
            auth_revision: 1,
            physical_root: store.root.join("accounts/owned").to_str().unwrap().into(),
            namespace: "native:Codex".into(),
            credential_reference: "native-managed".into(),
        };
        store
            .save_account(
                &serde_json::json!({}),
                "create-owned",
                &account,
                &binding,
                &serde_json::json!({}),
            )
            .unwrap();
        let cwd = temporary
            .path()
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        (temporary, store, account, cwd)
    }
    #[test]
    fn only_explicit_owned_intent_can_complete_with_zero_execution() {
        let (_temporary, mut store, account, cwd) = fixture();
        let legacy = store.helper_intent("verify", &account, &cwd, None).unwrap();
        assert!(
            !store
                .native_operations()
                .unwrap()
                .iter()
                .find(|op| op.operation_id == legacy)
                .unwrap()
                .host_boundary_owned
        );
        assert!(store.host_context(&legacy).is_err());
        assert!(store.complete_owned_helper(&legacy).is_err());
        let before = store.revision().unwrap();
        let owned = store
            .owned_helper_intent("verify", &account, &cwd, None)
            .unwrap();
        assert_eq!(store.revision().unwrap(), before + 1);
        store.reconcile_owned_helpers().unwrap();
        assert_eq!(
            store
                .native_operations()
                .unwrap()
                .iter()
                .find(|op| op.operation_id == owned)
                .unwrap()
                .state,
            "ownership_unknown"
        );
        let context = store.host_context(&owned).unwrap();
        assert_eq!(context.account_id, "owned");
        assert_eq!(context.auth_revision, 1);
        store.complete_owned_helper(&owned).unwrap();
        let operations = store.native_operations().unwrap();
        assert_eq!(
            operations
                .iter()
                .find(|op| op.operation_id == owned)
                .unwrap()
                .state,
            "reviewed"
        );
        assert_eq!(
            operations
                .iter()
                .find(|op| op.operation_id == legacy)
                .unwrap()
                .state,
            "ownership_unknown"
        );
        let revision = store.revision().unwrap();
        store.complete_owned_helper(&owned).unwrap();
        assert_eq!(store.revision().unwrap(), revision);
    }
    fn attempt_fixture() -> (tempfile::TempDir, Store, crate::agent_runtime::types::Task) {
        use crate::agent_runtime::types::Task;
        let (temporary, mut store, _account, cwd) = fixture();
        let task:Task=serde_json::from_value(serde_json::json!({
            "taskId":"task","cwd":cwd,"title":"Fixture","cli":"codex","availabilityReason":null,"model":"fixture","reasoningEffort":null,"revision":1,"historyRevision":0,"generation":1,"state":"starting","nextAccountId":"owned","activeAccountId":"owned","activeAttemptId":"attempt","statusMessage":"","attempts":[{"attemptId":"attempt","operationId":"attempt-op","accountId":"owned","authRevision":1,"generation":1,"input":"fixture","continuationMethod":"new","state":"spawn_intent","output":"","nativeRef":null,"version":null,"effectsState":"unsettled"}],"history":[],"grants":[],"switches":[]
        })).unwrap();
        let (_, identity) = workspace(Path::new(&task.cwd)).unwrap();
        store
            .create_task(
                &serde_json::json!({}),
                "create-task",
                &task,
                "fixture",
                identity,
            )
            .unwrap();
        store
            .owned_process_intent(&task, &task.attempts[0])
            .unwrap();
        (temporary, store, task)
    }
    #[test]
    #[ignore = "Darwin held launchd journals; run explicitly serially"]
    fn previous_boot_owned_journals_do_not_block_explicit_recovery() {
        use crate::agent_runtime::host_boundary::{
            Boundary, NativeIo, NativeSpec, Policy, Purpose, Scope,
        };
        use crate::agent_runtime::types::AccountRecover;
        let (_temporary, mut store, task) = attempt_fixture();
        let account = store.account("owned").unwrap();
        let helper = store
            .owned_helper_intent("verify", &account, &task.cwd, None)
            .unwrap();
        for (parent, purpose) in [
            ("attempt-op", Purpose::Attempt),
            (helper.as_str(), Purpose::VersionProbe),
        ] {
            let context = store.host_context(parent).unwrap();
            let base = host_boundary::prepare_parent(&store.root, parent).unwrap();
            let id = crate::agent_runtime::new_id().unwrap();
            let scope = Scope {
                operation_id: id.clone(),
                parent_operation_id: Some(parent.into()),
                physical_account_root: Some(context.physical_account_root.clone()),
                storage_root: Some(store.root.clone()),
                account_id: context.account_id,
                auth_revision: context.auth_revision,
                task_id: context.task_id,
                attempt_id: context.attempt_id,
                generation: context.generation,
                project_root: context.project_root.clone(),
                purpose,
            };
            let program = PathBuf::from("/usr/bin/true");
            let spec = NativeSpec {
                program: program.clone(),
                arguments: vec![],
                environment: vec![],
                cwd: context.project_root.clone(),
                io: NativeIo::Pipes,
                policy: Policy {
                    project_root: context.project_root,
                    account_root: context.physical_account_root,
                    temp_root: PathBuf::from(&task.cwd),
                    runtime_reads: vec![program],
                    protected_reads: vec![],
                    blocked_reads: vec![],
                    runtime_read_roots: vec![],
                    codex_preferences: false,
                    loopback_tcp_ports: vec![],
                    loopback_listener: false,
                    unix_sockets: vec![],
                    allow_native_tools: false,
                },
            };
            let mut held = Boundary::create(&base, scope, spec).unwrap();
            let record_path = base.join(id).join("record.json");
            let mut saved: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
            assert_eq!(saved["phase"], "held");
            held.stop().unwrap();
            drop(held);
            // Restore only this owned fixture's valid held record as an earlier
            // boot. Its old kernel IDs must never be consulted after restart.
            saved["host"]["boot"] = serde_json::json!("previous-owned-fixture-boot");
            std::fs::write(&record_path, serde_json::to_vec(&saved).unwrap()).unwrap();
        }
        for mut operation in store.native_operations().unwrap() {
            operation.boot = "previous-owned-fixture-boot".into();
            store
                .db
                .execute(
                    "UPDATE native_operations SET record=?2 WHERE id=?1",
                    params![operation.operation_id, encode(&operation).unwrap()],
                )
                .unwrap();
        }
        store.test_previous_boot("attempt").unwrap();
        store
            .process_tool_untracked("attempt", "unknown-remote")
            .unwrap();
        let root = store.root.clone();
        drop(store);
        let mut reopened = Store::open(root).unwrap();
        assert!(
            reopened
                .account_recovery("owned")
                .unwrap()
                .unwrap()
                .recoverable
        );
        assert_eq!(reopened.reconcile_processes().unwrap(), vec!["owned"]);
        assert!(reopened.processes_settled("task").unwrap());
        assert_eq!(
            reopened.task("task").unwrap().attempts[0].effects_state,
            "uncertain"
        );
        assert!(reopened
            .native_operations()
            .unwrap()
            .iter()
            .all(|o| o.state != "reviewed"));
        reopened
            .recover_helpers(&AccountRecover {
                operation_id: "explicit-review".into(),
                account_id: "owned".into(),
                expected_revision: 1,
                acknowledge_effects: true,
            })
            .unwrap();
        assert_eq!(reopened.account("owned").unwrap().auth_revision, 2);
        assert!(reopened.account_recovery("owned").unwrap().is_none());
        assert_eq!(
            reopened.task("task").unwrap().attempts[0].effects_state,
            "uncertain"
        );
    }
    #[test]
    fn owned_attempt_absence_and_unknown_tools_never_inherit_legacy_drain() {
        let (_temporary, mut store, _task) = attempt_fixture();
        store.reconcile_owned_helpers().unwrap();
        assert!(store.reconcile_processes().unwrap().is_empty());
        assert!(!store.processes_settled("task").unwrap());
        store.complete_owned_helper("attempt-op").unwrap();
        store
            .process_tool_untracked("attempt", "unknown-remote")
            .unwrap();
        assert!(store.reconcile_processes().unwrap().is_empty());
        assert!(store.process_not_spawned("attempt").is_err());
        assert!(!store.processes_settled("task").unwrap());
        assert_eq!(
            store.task("task").unwrap().attempts[0].effects_state,
            "unsettled"
        );
    }
    #[test]
    fn positively_finished_queue_repairs_two_storage_failures_without_restart() {
        let (_temporary, store, _task) = attempt_fixture();
        let context = store.host_context("attempt-op").unwrap();
        assert!(host_boundary::parent_completed(&store.root, "attempt-op").unwrap());
        let mut inner = crate::agent_runtime::service::Inner {
            store: Some(store),
            ..Default::default()
        };
        inner
            .finished_owned_operations
            .insert("attempt-op".into(), (context, true));
        inner.store.as_ref().unwrap().db.execute_batch("CREATE TRIGGER block_completion BEFORE UPDATE ON native_operations BEGIN SELECT RAISE(ABORT,'fixture completion failure'); END;").unwrap();
        let revision = inner.store.as_ref().unwrap().revision().unwrap();
        for _ in 0..2 {
            assert!(crate::agent_runtime::owned_operation::retry_finished(&mut inner).is_err());
            assert_eq!(inner.finished_owned_operations.len(), 1);
            assert_eq!(inner.store.as_ref().unwrap().revision().unwrap(), revision);
            assert_eq!(
                inner.store.as_ref().unwrap().task("task").unwrap().attempts[0].state,
                "spawn_intent"
            );
        }
        inner
            .store
            .as_ref()
            .unwrap()
            .db
            .execute_batch("DROP TRIGGER block_completion")
            .unwrap();
        crate::agent_runtime::owned_operation::retry_finished(&mut inner).unwrap();
        assert!(inner.finished_owned_operations.is_empty());
        let store = inner.store.as_ref().unwrap();
        assert_eq!(store.native_operations().unwrap()[0].state, "reviewed");
        let task = store.task("task").unwrap();
        assert_eq!(task.attempts[0].state, "rejected");
        assert_eq!(task.attempts[0].effects_state, "settled");
        assert!(store.processes_settled("task").unwrap());
        assert!(store.account_recovery("owned").unwrap().is_none());
    }
    #[test]
    fn context_validation_failure_creates_no_durable_helper_intent() {
        let (_temporary, mut store, account, cwd) = fixture();
        let revision = store.revision().unwrap();
        // Fail the same durable binding read used to validate a new context.
        store.fail_next_context.set(true);
        assert!(store
            .owned_helper_intent_context("verify", &account, &cwd)
            .is_err());
        assert!(store.native_operations().unwrap().is_empty());
        assert_eq!(store.revision().unwrap(), revision);
    }
    #[test]
    fn wire_tool_id_cannot_clear_owner_lifecycle_marker() {
        let (_temporary, mut store, _task) = attempt_fixture();
        store
            .process_tool_denied("attempt", "unqualified-boundary")
            .unwrap();
        store.process_untracked("attempt").unwrap();
        store
            .process_tool_untracked("attempt", "unqualified-dispatch")
            .unwrap();
        store
            .process_tool_denied("attempt", "unqualified-dispatch")
            .unwrap();
        let raw: String = store
            .db
            .query_row(
                "SELECT unknown_tools FROM process_ownership WHERE attempt_id='attempt'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            decode::<Vec<String>>(raw).unwrap(),
            vec!["unqualified-boundary", "unqualified-dispatch"]
        );
        store.process_tool_untracked("attempt", "x").unwrap();
        store.process_tool_untracked("attempt", "tool:x").unwrap();
        store.process_tool_denied("attempt", "tool:x").unwrap();
        let raw: String = store
            .db
            .query_row(
                "SELECT unknown_tools FROM process_ownership WHERE attempt_id='attempt'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(decode::<Vec<String>>(raw)
            .unwrap()
            .contains(&"tool:x".into()));
        store.db.execute("UPDATE process_ownership SET unknown_tools='[\"legacy-opaque\",\"unqualified-boundary\"]' WHERE attempt_id='attempt'",[]).unwrap();
        store
            .process_tool_denied("attempt", "legacy-opaque")
            .unwrap();
        store.clear_lifecycle_boundary("attempt").unwrap();
        let raw: String = store
            .db
            .query_row(
                "SELECT unknown_tools FROM process_ownership WHERE attempt_id='attempt'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(decode::<Vec<String>>(raw).unwrap(), vec!["legacy-opaque"]);
        assert!(!store.processes_settled("task").unwrap());
    }
    #[test]
    fn owned_review_ledger_preserves_unknown_effects_atomically() {
        let (_temporary, mut store, _task) = attempt_fixture();
        store
            .process_tool_untracked("attempt", "unknown-remote")
            .unwrap();
        let before = store.revision().unwrap();
        store.db.execute_batch("CREATE TRIGGER reject_owned_review BEFORE UPDATE ON native_operations BEGIN SELECT RAISE(ABORT,'fixture atomic failure'); END;").unwrap();
        assert!(store.complete_owned_helper("attempt-op").is_err());
        assert_eq!(store.revision().unwrap(), before);
        assert_eq!(
            store.owned_operation("attempt-op").unwrap().state,
            "ownership_unknown"
        );
        let raw: String = store
            .db
            .query_row(
                "SELECT unknown_tools FROM process_ownership WHERE attempt_id='attempt'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let tools: Vec<String> = decode(raw).unwrap();
        assert!(tools.iter().any(|tool| tool == "unqualified-boundary"));
        assert!(tools.iter().any(|tool| tool == "tool:unknown-remote"));
        store
            .db
            .execute_batch("DROP TRIGGER reject_owned_review;")
            .unwrap();
        store.complete_owned_helper("attempt-op").unwrap();
        assert_eq!(
            store.owned_operation("attempt-op").unwrap().state,
            "reviewed"
        );
        assert_eq!(store.revision().unwrap(), before + 1);
        let (raw, state): (String, String) = store
            .db
            .query_row(
                "SELECT unknown_tools,state FROM process_ownership WHERE attempt_id='attempt'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            decode::<Vec<String>>(raw).unwrap(),
            vec!["tool:unknown-remote", "unqualified-boundary"]
        );
        assert_eq!(state, "untracked");
        assert_eq!(
            store.task("task").unwrap().attempts[0].effects_state,
            "unsettled"
        );
    }
    #[test]
    fn incomplete_boundary_journal_never_releases_owned_intent() {
        use std::os::unix::fs::PermissionsExt;
        let (_temporary, mut store, account, cwd) = fixture();
        let owned = store
            .owned_helper_intent("verify", &account, &cwd, None)
            .unwrap();
        let base = store.root.join("host-boundaries");
        native_accounts::check_private_directory(&base).unwrap();
        let journal = base.join("unfinished");
        native_accounts::check_private_directory(&journal).unwrap();
        let path = journal.join("record.json");
        std::fs::write(&path, b"{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(store.complete_owned_helper(&owned).is_err());
        store.reconcile_owned_helpers().unwrap();
        assert_eq!(
            store
                .native_operations()
                .unwrap()
                .iter()
                .find(|op| op.operation_id == owned)
                .unwrap()
                .state,
            "ownership_unknown"
        );
    }
}
