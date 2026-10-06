use super::{types::*, AgentRuntime};

use crate::terminal::Shells;

use serde::{Deserialize, Serialize};

use serde_json::json;

use std::sync::atomic::Ordering;

use tauri::{AppHandle, Manager, State, Window};

fn trusted(w: &Window) -> Result<(), String> {
    if matches!(w.label(), "main" | "settings") {
        Ok(())
    } else {
        Err("Agent runtime commands require a trusted application view.".into())
    }
}

macro_rules! account_command {
    ($name:ident,$request:ty,$method:ident) => {
        #[tauri::command]
        pub(crate) fn $name(
            window: Window,
            app: AppHandle,
            state: State<'_, AgentRuntime>,
            request: $request,
        ) -> Result<AccountsSnapshot, String> {
            trusted(&window)?;

            let result = state.$method(&app, request)?;

            state.changed(&app, None);

            Ok(result)
        }
    };
}

macro_rules! task_command {
    ($name:ident,$request:ty,$method:ident) => {
        #[tauri::command]
        pub(crate) fn $name(
            window: Window,
            app: AppHandle,
            state: State<'_, AgentRuntime>,
            request: $request,
        ) -> Result<Task, String> {
            crate::files::main_window(&window)?;

            let result = state.$method(&app, request)?;

            state.changed(&app, Some(&result.task_id));

            Ok(result)
        }
    };
}

#[tauri::command]
pub(crate) fn agent_accounts_snapshot(
    window: Window,
    app: AppHandle,
    state: State<'_, AgentRuntime>,
) -> Result<AccountsSnapshot, String> {
    trusted(&window)?;

    state.accounts(&app)
}

#[tauri::command]
pub(crate) fn agent_tasks_snapshot(
    window: Window,
    app: AppHandle,
    state: State<'_, AgentRuntime>,
) -> Result<TasksSnapshot, String> {
    crate::files::main_window(&window)?;

    state.tasks(&app)
}

#[tauri::command]
pub(crate) fn agent_task_snapshot(
    window: Window,
    app: AppHandle,
    state: State<'_, AgentRuntime>,
    task_id: String,
) -> Result<Task, String> {
    crate::files::main_window(&window)?;

    state.task(&app, &task_id)
}

account_command!(agent_account_create, AccountCreate, account_create);

account_command!(agent_account_update, AccountUpdate, account_update);

account_command!(agent_account_remove, AccountRemove, account_remove);
account_command!(agent_account_recover, AccountRecover, account_recover);

#[tauri::command]
pub(crate) async fn agent_account_verify(
    window: Window,
    app: AppHandle,
    state: State<'_, AgentRuntime>,
    shells: State<'_, Shells>,
    request: AccountVerify,
) -> Result<AccountsSnapshot, String> {
    trusted(&window)?;

    let state = state.inner().clone();

    let shells = shells.inner().clone();

    tauri::async_runtime::spawn_blocking(move || state.verify(&app, &shells, request))
        .await
        .map_err(|_| "Native account verification worker failed.")?
}

task_command!(agent_task_create, TaskCreate, task_create);

task_command!(agent_task_update_history_grant, GrantUpdate, grant);

task_command!(agent_task_prepare_switch, SwitchPrepare, prepare_switch);

task_command!(agent_task_stop, TaskStop, stop);

task_command!(agent_task_recover, TaskRecover, recover);

#[tauri::command]
pub(crate) fn agent_task_send(
    window: Window,
    app: AppHandle,
    state: State<'_, AgentRuntime>,
    shells: State<'_, Shells>,
    request: TaskSend,
) -> Result<Task, String> {
    crate::files::main_window(&window)?;

    state.send(&app, &shells, request)
}

#[tauri::command]
pub(crate) fn agent_task_commit_switch(
    window: Window,
    app: AppHandle,
    state: State<'_, AgentRuntime>,
    shells: State<'_, Shells>,
    request: SwitchCommit,
) -> Result<Task, String> {
    crate::files::main_window(&window)?;

    state.commit_switch(&app, &shells, request)
}

#[tauri::command]
pub(crate) async fn agent_task_prepare_close(
    window: Window,
    app: AppHandle,
    state: State<'_, AgentRuntime>,
    task_ids: Vec<String>,
) -> Result<(), String> {
    crate::files::main_window(&window)?;

    let state = state.inner().clone();

    tauri::async_runtime::spawn_blocking(move || state.drain(&app, &task_ids))
        .await
        .map_err(|_| "Task drain worker failed.")?
}

#[tauri::command]
pub(crate) fn agent_task_close_release(
    window: Window,
    app: AppHandle,
    state: State<'_, AgentRuntime>,
    task_ids: Vec<String>,
) -> Result<(), String> {
    crate::files::main_window(&window)?;

    state.cancel_close(&app, Some(&task_ids))
}

#[tauri::command]
pub(crate) fn agent_runtime_prepare_close(
    window: Window,
    state: State<'_, AgentRuntime>,
) -> Result<(), String> {
    crate::files::main_window(&window)?;

    state.closing.store(true, Ordering::SeqCst);

    Ok(())
}

#[tauri::command]
pub(crate) fn agent_runtime_cancel_close(
    window: Window,
    app: AppHandle,
    state: State<'_, AgentRuntime>,
) -> Result<(), String> {
    crate::files::main_window(&window)?;

    state.cancel_close(&app, None)
}

#[tauri::command]
pub(crate) async fn agent_tasks_drain(
    window: Window,
    app: AppHandle,
    state: State<'_, AgentRuntime>,
    task_ids: Option<Vec<String>>,
) -> Result<(), String> {
    crate::files::main_window(&window)?;

    let state = state.inner().clone();

    tauri::async_runtime::spawn_blocking(move || {
        if let Some(ids) = task_ids {
            state.drain(&app, &ids)
        } else {
            state.drain_all(&app)
        }
    })
    .await
    .map_err(|_| "Runtime drain worker failed.")?
}

#[tauri::command]
pub(crate) async fn agent_runtime_shutdown(
    window: Window,
    app: AppHandle,
    state: State<'_, AgentRuntime>,
) -> Result<(), String> {
    crate::files::main_window(&window)?;

    let state = state.inner().clone();

    tauri::async_runtime::spawn_blocking(move || state.drain_all(&app))
        .await
        .map_err(|_| "Runtime shutdown worker failed.")?
}

#[tauri::command]
pub(crate) fn agent_permission_snapshot(
    window: Window,
    app: AppHandle,
    state: State<'_, AgentRuntime>,
    task_id: Option<String>,
) -> Result<Vec<PendingPermission>, String> {
    crate::files::main_window(&window)?;

    state.with(&app, |i| {
        Ok(i.permissions
            .values()
            .filter(|p| !p.consumed && task_id.as_ref().is_none_or(|id| &p.preview.task_id == id))
            .map(|p| p.preview.clone())
            .collect())
    })
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FreezeRequest {
    operation_id: String,
    task_id: String,
    attempt_id: String,
    generation: u64,
    approval_token: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FreezeResult {
    editor_freeze_token: String,
}

#[tauri::command]
pub(crate) fn agent_permission_freeze(
    window: Window,
    app: AppHandle,
    state: State<'_, AgentRuntime>,
    request: FreezeRequest,
) -> Result<FreezeResult, String> {
    crate::files::main_window(&window)?;

    state.with(&app, |i| {
        let s = i.store.as_mut().unwrap();

        if let Some(result) = s.replay(&request.operation_id, &request)? {
            return Ok(result);
        }

        let p = i
            .permissions
            .get(&request.approval_token)
            .ok_or("The native permission is no longer pending.")?;

        if p.preview.task_id != request.task_id
            || p.preview.attempt_id != request.attempt_id
            || p.preview.generation != request.generation
            || p.consumed
        {
            return Err("The native permission was fenced.".into());
        }

        let t = s.task(&request.task_id)?;

        if t.generation != request.generation
            || t.active_attempt_id.as_deref() != Some(&request.attempt_id)
        {
            return Err("The native attempt changed.".into());
        }

        let result = FreezeResult {
            editor_freeze_token: super::new_id()?,
        };

        i.freezes.insert(
            result.editor_freeze_token.clone(),
            (
                request.task_id.clone(),
                request.attempt_id.clone(),
                request.generation,
                false,
            ),
        );

        s.command_receipt(&request, &request.operation_id, &result)?;

        Ok(result)
    })
}

#[derive(Serialize)]
pub(crate) struct FreezeComplete {
    complete: bool,
}

#[tauri::command]
pub(crate) fn agent_permission_freeze_complete(
    window: Window,
    app: AppHandle,
    state: State<'_, AgentRuntime>,
    editor_freeze_token: String,
) -> Result<FreezeComplete, String> {
    crate::files::main_window(&window)?;

    state.with(&app, |i| {
        Ok(FreezeComplete {
            complete: i
                .freezes
                .get(&editor_freeze_token)
                .ok_or(
                    "Retained freeze token is unavailable; refresh task recovery before release.",
                )?
                .3,
        })
    })
}

#[tauri::command]
pub(crate) fn agent_permission_reply(
    window: Window,
    app: AppHandle,
    state: State<'_, AgentRuntime>,
    request: PermissionReply,
) -> Result<Task, String> {
    crate::files::main_window(&window)?;

    let t = state.with(&app, |i| {
        let s = i.store.as_mut().unwrap();

        if let Some(old) = s.replay(&request.operation_id, &request)? {
            return Ok(old);
        }

        let p = i
            .permissions
            .get_mut(&request.approval_token)
            .ok_or("The native permission is no longer pending.")?;

        let mut t = s.task(&request.task_id)?;

        if p.preview.task_id != request.task_id
            || p.preview.attempt_id != request.attempt_id
            || p.preview.generation != request.generation
            || t.active_attempt_id.as_deref() != Some(&request.attempt_id)
            || t.generation != request.generation
            || p.consumed
            || p.decision.is_some()
        {
            return Err("The native permission decision was fenced or already used.".into());
        }

        if i.active
            .get(&request.task_id)
            .is_none_or(|a| a.stop.load(Ordering::SeqCst))
        {
            return Err("The task is stopping; new approvals are fenced.".into());
        }

        let freeze = request.editor_freeze_token.clone().unwrap_or_default();

        if request.allow {
            let lease = i
                .freezes
                .get(&freeze)
                .ok_or("Freeze project editor buffers before approving the native tool.")?;

            if lease.0 != t.task_id
                || lease.1 != request.attempt_id
                || lease.2 != request.generation
                || lease.3
            {
                return Err("The project editor freeze lease changed.".into());
            }
        }

        let permission = &p.preview.permission;

        let choice = permission_choice(&request, permission)?;

        let attempt = t
            .attempts
            .iter()
            .find(|a| a.attempt_id == request.attempt_id)
            .cloned()
            .ok_or("Permission attempt provenance is missing.")?;

        super::service::append(
            &mut t,
            &attempt,
            "permission_decision",
            "committed",
            json!({
            "requestId":permission.request_id,"choice":choice,"allow":request.allow}
            ),
        );

        t.revision += 1;

        s.mutate_task(&request, &request.operation_id, &t, None)?;

        p.decision = Some((choice, request.allow, freeze));

        Ok(t)
    })?;

    state.changed(&app, Some(&t.task_id));

    Ok(t)
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MigrationPreview {
    pub migration_id: String,
    pub digest: String,
    pub account_count: usize,
    pub task_count: usize,
    pub missing_count: usize,
    pub phase: String,
}

#[tauri::command]
pub(crate) async fn agent_legacy_migration_preview(
    window: Window,
    app: AppHandle,
    state: State<'_, AgentRuntime>,
) -> Result<MigrationPreview, String> {
    crate::files::main_window(&window)?;

    let state = state.inner().clone();

    tauri::async_runtime::spawn_blocking(move || {
        let session = app.state::<crate::files::SessionFile>();

        let _session = session
            .0
            .lock()
            .map_err(|_| "Session publication is unavailable.")?;

        state.with(&app, |i| {
            if !i.active.is_empty() || !i.account_leases.is_empty() {
                return Err(
                    "Drain tasks and close account terminals before reviewing migration.".into(),
                );
            }

            let parent = app
                .path()
                .app_data_dir()
                .map_err(|_| "Cannot resolve migration storage.")?;

            let migration_id = format!("legacy-{}", super::new_id()?);

            let migration = super::migration::prepare_owned(&parent, &migration_id)?;

            let b = migration.bundle();

            let result = MigrationPreview {
                migration_id: b.migration_id.clone(),
                digest: migration.reviewed_digest().into(),
                account_count: b.snapshot["profiles"].as_array().map(Vec::len).unwrap_or(0),
                task_count: b.snapshot["runs"].as_array().map(Vec::len).unwrap_or(0),
                missing_count: b.missing_run_ids.len(),
                phase: format!("{:?}", migration.phase()).to_lowercase(),
            };

            i.store.as_mut().unwrap().command_receipt(
                &json!({
                "migrationId":migration_id}
                ),
                &format!("migration-preview:{migration_id}"),
                &result,
            )?;

            Ok(result)
        })
    })
    .await
    .map_err(|_| "Migration preview worker failed.")?
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MigrationApply {
    operation_id: String,
    migration_id: String,
    expected_digest: String,
}

#[tauri::command]
pub(crate) async fn agent_legacy_migration_apply(
    window: Window,
    app: AppHandle,
    state: State<'_, AgentRuntime>,
    request: MigrationApply,
) -> Result<MigrationPreview, String> {
    crate::files::main_window(&window)?;

    let state = state.inner().clone();

    tauri::async_runtime::spawn_blocking(move || {
        let session = app.state::<crate::files::SessionFile>();

        let _session = session
            .0
            .lock()
            .map_err(|_| "Session publication is unavailable.")?;

        let result = state.with(&app, |i| {
            let s = i.store.as_mut().unwrap();

            if let Some(old) = s.replay(&request.operation_id, &request)? {
                return Ok(old);
            }

            if !i.active.is_empty() || !i.account_leases.is_empty() {
                return Err("Drain runtime processes before archive publication.".into());
            }

            let parent = app
                .path()
                .app_data_dir()
                .map_err(|_| "Cannot resolve migration storage.")?;

            super::valid_operation(&request.migration_id)?;

            let review: MigrationPreview = s
                .receipt(&format!("migration-preview:{}", request.migration_id))?
                .ok_or("Review this exact migration generation before publication.")?;

            if review.digest != request.expected_digest {
                return Err("The reviewed migration digest changed.".into());
            }

            let mut migration = super::migration::prepare_owned(&parent, &request.migration_id)?;

            if migration.reviewed_digest() != request.expected_digest {
                return Err("The reviewed archive changed; preview migration again.".into());
            }

            let b = migration.bundle();

            let mut result = MigrationPreview {
                migration_id: b.migration_id.clone(),
                digest: migration.reviewed_digest().into(),
                account_count: b.snapshot["profiles"].as_array().map(Vec::len).unwrap_or(0),
                task_count: b.snapshot["runs"].as_array().map(Vec::len).unwrap_or(0),
                missing_count: b.missing_run_ids.len(),
                phase: "committed".into(),
            };

            migration.publish(|bundle| s.import_bundle(bundle))?;

            result.phase = "committed".into();

            s.command_receipt(&request, &request.operation_id, &result)?;

            Ok(result)
        })?;

        state.changed(&app, None);

        Ok(result)
    })
    .await
    .map_err(|_| "Migration publication worker failed.")?
}

#[cfg(unix)]
#[tauri::command]
pub(crate) fn agent_task_transfer_preview(
    window: Window,
    app: AppHandle,
    state: State<'_, AgentRuntime>,
    request: TransferPreview,
) -> Result<TransferReview, String> {
    crate::files::main_window(&window)?;

    state.transfer_preview(&app, request)
}

#[cfg(unix)]
#[tauri::command]
pub(crate) fn agent_task_transfer_apply(
    window: Window,
    app: AppHandle,
    state: State<'_, AgentRuntime>,
    request: TransferApply,
) -> Result<Task, String> {
    crate::files::main_window(&window)?;

    let task = state.transfer_apply(&app, request)?;

    state.changed(&app, Some(&task.task_id));

    Ok(task)
}

#[cfg(not(unix))]
#[tauri::command]
pub(crate) fn agent_task_transfer_preview(
    window: Window,
    request: TransferPreview,
) -> Result<TransferReview, String> {
    crate::files::main_window(&window)?;

    let _ = request;

    Err("Native reviewed transfer is unqualified on this platform.".into())
}

#[cfg(not(unix))]
#[tauri::command]
pub(crate) fn agent_task_transfer_apply(
    window: Window,
    request: TransferApply,
) -> Result<Task, String> {
    crate::files::main_window(&window)?;

    let _ = request;

    Err("Native reviewed transfer is unqualified on this platform.".into())
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MigrationRollback {
    operation_id: String,
    migration_id: String,
    expected_digest: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MigrationExport {
    export_id: String,
}

#[tauri::command]
pub(crate) async fn agent_legacy_migration_rollback(
    window: Window,
    app: AppHandle,
    state: State<'_, AgentRuntime>,
    request: MigrationRollback,
) -> Result<MigrationExport, String> {
    crate::files::main_window(&window)?;

    let state = state.inner().clone();

    tauri::async_runtime::spawn_blocking(move || {
        let session = app.state::<crate::files::SessionFile>();

        let _session = session
            .0
            .lock()
            .map_err(|_| "Session publication is unavailable.")?;

        state.with(&app, |i| {
            let s = i.store.as_mut().unwrap();

            if let Some(old) = s.replay(&request.operation_id, &request)? {
                return Ok(old);
            }

            if !i.active.is_empty() || !i.account_leases.is_empty() {
                return Err("Drain native ownership before exporting rollback data.".into());
            }

            super::valid_operation(&request.migration_id)?;

            let reviewed: MigrationPreview = s
                .receipt(&format!("migration-preview:{}", request.migration_id))?
                .ok_or("Review the exact native migration archive first.")?;

            if reviewed.digest != request.expected_digest {
                return Err("The reviewed archive digest changed.".into());
            }

            let parent = app
                .path()
                .app_data_dir()
                .map_err(|_| "Cannot resolve migration storage.")?;

            let prepared = super::migration::prepare_owned(&parent, &request.migration_id)?;

            let exports = parent.join("agent-runtime-rollback");

            crate::chat::storage::reject_link(&exports)?;

            std::fs::create_dir_all(&exports)
                .map_err(|_| "Cannot create rollback export namespace.")?;

            crate::chat::storage::private(&exports, true)?;

            let result = MigrationExport {
                export_id: super::new_id()?,
            };

            let target = exports.join(&result.export_id);

            std::fs::create_dir(&target).map_err(|_| "Cannot create rollback export.")?;

            crate::chat::storage::private(&target, true)?;

            prepared.rollback_to(&target, &s.root)?;

            s.command_receipt(&request, &request.operation_id, &result)?;

            Ok(result)
        })
    })
    .await
    .map_err(|_| "Migration rollback export worker failed.")?
}

#[tauri::command]
pub(crate) fn agent_legacy_migration_export_open(
    window: Window,
    app: AppHandle,
    export_id: String,
) -> Result<(), String> {
    crate::files::main_window(&window)?;

    if export_id.len() != 32 || !export_id.bytes().all(|v| v.is_ascii_hexdigit()) {
        return Err("Invalid native rollback export identity.".into());
    }

    let path = app
        .path()
        .app_data_dir()
        .map_err(|_| "Cannot resolve rollback export namespace.")?
        .join("agent-runtime-rollback")
        .join(export_id);

    super::native_accounts::check_private_directory(&path)?;

    use tauri_plugin_opener::OpenerExt;

    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|_| String::from("Cannot open the native rollback export."))
}

pub(super) fn permission_choice(
    request: &PermissionReply,
    permission: &super::native_wire::NativePermission,
) -> Result<serde_json::Value, String> {
    let choice = request.choice.clone().unwrap_or_else(|| {
        if matches!(
            permission.raw["method"].as_str(),
            Some("input" | "editor" | "select")
        ) {
            serde_json::Value::Null
        } else {
            json!(request.allow)
        }
    });

    let method = permission.raw["method"].as_str().unwrap_or("");

    let native_pi = matches!(method, "confirm" | "input" | "editor" | "select");

    let offered = permission
        .choices
        .iter()
        .any(|v| v == &choice || v.get("optionId") == Some(&choice));

    let pi_choice = native_pi
        && match method {
            "confirm" => choice.is_boolean(),
            "input" | "editor" => {
                choice.as_str().is_some_and(|s| s.len() <= 64 * 1024) || choice.is_null()
            }

            "select" => offered || choice.is_null(),
            _ => false,
        };

    if !offered && !pi_choice {
        return Err("Select an exact choice offered by this native permission protocol.".into());
    }

    let allow = choice.as_bool() == Some(true)
        || choice.as_str().is_some_and(|v| {
            matches!(
                v,
                "allow" | "accept" | "acceptForSession" | "once" | "always" | "approved"
            )
        })
        || permission.choices.iter().any(|v| {
            v.get("optionId") == Some(&choice)
                && v["kind"].as_str().is_some_and(|v| v.starts_with("allow"))
        })
        || native_pi && matches!(method, "input" | "editor" | "select") && !choice.is_null()
        || choice
            .as_object()
            .is_some_and(|v| v.keys().any(|k| k.starts_with("accept")));

    if allow != request.allow {
        return Err("The permission choice does not match its allow/deny decision.".into());
    }

    Ok(choice)
}
