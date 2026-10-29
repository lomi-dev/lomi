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
