//! Settle host workers outside the durable runtime owner. A broker effect may
//! need that owner while completing; Stop must never hold it while waiting.
use super::{host_boundary, host_child::Context, service::AgentRuntime};
use tauri::AppHandle;

pub(super) struct OwnedOperation {
    state: AgentRuntime,
    app: AppHandle,
    context: Context,
    complete: bool,
}
impl OwnedOperation {
    pub(super) fn new(state: &AgentRuntime, app: &AppHandle, context: Context) -> Self {
        Self {
            state: state.clone(),
            app: app.clone(),
            context,
            complete: false,
        }
    }
    pub(super) fn finish(&mut self) -> Result<(), String> {
        if self.complete {
            return Ok(());
        }
        // A drained Process/PTY can still retain its boundary lock while its
        // immutable retirement receipt already proves ESRCH. Read that proof
        // first; reopening the live owner's lock would reject clean completion.
        if !host_boundary::parent_completed(
            &self.context.storage_root,
            &self.context.parent_operation_id,
        )? {
            host_boundary::settle_parent(
                &self.context.storage_root,
                &self.context.parent_operation_id,
            )?;
        }
        let no_attempt = self.no_attempt_created()?;
        // Retain only positively finished parents before any durable write.
        // Later owner entries can repair transient storage failures without
        // reopening or stopping a live version-stage parent.
        self.state
            .inner
            .lock()
            .map_err(|_| "Runtime owner unavailable.")?
            .finished_owned_operations
            .insert(
                self.context.parent_operation_id.clone(),
                (self.context.clone(), no_attempt),
            );
        self.state.with(&self.app, |_| Ok(()))?;
        self.complete = true;
        Ok(())
    }
    pub(super) fn no_attempt_created(&self) -> Result<bool, String> {
        Ok(!host_boundary::parent_scopes(
            &self.context.storage_root,
            &self.context.parent_operation_id,
        )?
        .iter()
        .any(|scope| scope.purpose == host_boundary::Purpose::Attempt))
    }
}
pub(super) fn retry_finished(inner: &mut super::service::Inner) -> Result<(), String> {
    let finished = inner
        .finished_owned_operations
        .values()
        .cloned()
        .collect::<Vec<_>>();
    for (context, no_attempt) in finished {
        finalize_finished(inner, &context, no_attempt)?;
        inner
            .finished_owned_operations
            .remove(&context.parent_operation_id);
    }
    if inner
        .account_terminal_finishes
        .values()
        .any(|done| done.load(std::sync::atomic::Ordering::SeqCst))
    {
        let reviewed = inner
            .store
            .as_ref()
            .ok_or("Runtime owner unavailable.")?
            .native_operations()?
            .into_iter()
            .filter(|op| op.state == "reviewed")
            .map(|op| op.operation_id)
            .collect::<std::collections::HashSet<_>>();
        inner.account_terminal_finishes.retain(|id, done| {
            !done.load(std::sync::atomic::Ordering::SeqCst) || !reviewed.contains(id)
        });
    }
    Ok(())
}
fn finalize_finished(
    inner: &mut super::service::Inner,
    context: &Context,
    no_attempt: bool,
) -> Result<(), String> {
    let store = inner.store.as_mut().ok_or("Runtime owner unavailable.")?;
    store.complete_owned_helper(&context.parent_operation_id)?;
    if let (Some(task_id), Some(attempt_id)) = (&context.task_id, &context.attempt_id) {
        if no_attempt {
            let mut task = store.task(task_id)?;
            if let Some(attempt) = task
                .attempts
                .iter_mut()
                .find(|a| a.attempt_id == *attempt_id)
            {
                if attempt.state == "spawn_intent" {
                    store.clear_lifecycle_boundary(attempt_id)?;
                    attempt.state = "rejected".into();
                    attempt.effects_state = "settled".into();
                    store.process_not_spawned(attempt_id)?;
                    store.update_task(&task, None, None)?;
                }
            }
        }
    }
    if context.task_id.is_none() {
        inner.recovery_projects.remove(&context.parent_operation_id);
    }
    Ok(())
}

impl Drop for OwnedOperation {
    fn drop(&mut self) {
        // Failure retains the journal, account and project fence. It is never
        // replaced with a success inferred from EOF or an absent PID marker.
        let _ = self.finish();
    }
}
