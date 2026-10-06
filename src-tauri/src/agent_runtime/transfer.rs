use super::{
    native_transfer::ForkRequest,
    service::{expected, history_digest, target},
    store::Store,
    types::*,
};
use crate::cli_catalog::TitleCli;
pub(crate) fn request(
    s: &Store,
    t: &Task,
    account: &str,
    auth: u64,
) -> Result<ForkRequest, String> {
    let a = target(s, account, auth, Some(t))?;
    let c = s
        .checkpoint(&t.task_id)?
        .ok_or("No settled native checkpoint is available for reviewed transfer.")?;
    if t.cli != Some(TitleCli::Pi)
        || c.version != "1.0.1"
        || !c.settled
        || c.history_revision != t.history_revision
        || c.digest != history_digest(t)?
        || c.account_id == a.account_id
        || !t.model.starts_with("openai/")
    {
        return Err("Reviewed Pi transfer requires the exact latest settled 1.0.1 openai/* checkpoint and a different granted account.".into());
    }
    let source = target(s, &c.account_id, c.auth_revision, Some(t))?;
    Ok(ForkRequest {
        run_id: t.task_id.clone(),
        input_id: c.attempt_id,
        source_generation: c.generation,
        next_generation: t.generation + 1,
        source_file: c
            .session_file
            .ok_or("The owned Pi source file is missing.")?,
        source_session: c.native_ref,
        source_profile: source.account_id,
        source_revision: source.auth_revision,
        destination_profile: a.account_id,
        destination_revision: a.auth_revision,
        cwd: t.cwd.clone(),
        cwd_identity: c.cwd_identity,
        model: t.model.clone(),
        version: c.version,
    })
}
impl super::AgentRuntime {
    pub(crate) fn transfer_preview(
        &self,
        app: &tauri::AppHandle,
        r: TransferPreview,
    ) -> Result<TransferReview, String> {
        self.with(app, |i| {
            let s = i.store.as_mut().unwrap();
            if let Some(old) = s.replay(&r.operation_id, &r)? {
                return Ok(old);
            }
            if i.active.contains_key(&r.task_id) {
                return Err("Stop and settle the task before reviewing a native transfer.".into());
            }
            let t = s.task(&r.task_id)?;
            expected(t.revision, r.expected_revision)?;
            expected(t.history_revision, r.expected_history_revision)?;
            let request = request(s, &t, &r.account_id, r.auth_revision)?;
            if i.account_leases.contains(&request.source_profile)
                || i.account_leases.contains(&request.destination_profile)
            {
                return Err("Native source or target account namespace is leased.".into());
            }
            let root = s.task_root(&t.task_id)?;
            let (digest, bytes) = super::native_transfer::inspect(&root, &request)?;
            let result = TransferReview {
                digest,
                bytes,
                task_id: t.task_id,
                account_id: r.account_id.clone(),
                auth_revision: r.auth_revision,
                coverage: t.history_revision,
                source_attempt_id: request.input_id,
            };
            s.command_receipt(&r, &r.operation_id, &result)?;
            Ok(result)
        })
    }
    pub(crate) fn transfer_apply(
        &self,
        app: &tauri::AppHandle,
        r: TransferApply,
    ) -> Result<Task, String> {
        self.with(app, |i| {
            let s = i.store.as_mut().unwrap();
            if let Some(old) = s.replay(&r.operation_id, &r)? {
                return Ok(old);
            }
            if i.active.contains_key(&r.task_id) {
                return Err(
                    "Wait for native tool settlement and drain before applying transfer.".into(),
                );
            }
            let mut t = s.task(&r.task_id)?;
            expected(t.revision, r.expected_revision)?;
            expected(t.history_revision, r.expected_history_revision)?;
            let request = request(s, &t, &r.account_id, r.auth_revision)?;
            if i.account_leases.contains(&request.source_profile)
                || i.account_leases.contains(&request.destination_profile)
            {
                return Err("Native source or target account namespace is leased.".into());
            }
            let root = s.task_root(&t.task_id)?;
            super::native_transfer::prepare_reviewed(
                &root,
                &request,
                &r.expected_digest,
                r.expected_bytes,
            )?;
            t.switches.push(SwitchOperation {
                operation_id: r.operation_id.clone(),
                source_attempt_id: Some(request.input_id),
                account_id: r.account_id.clone(),
                auth_revision: r.auth_revision,
                history_revision: t.history_revision,
                mode: "reviewed_transfer".into(),
                phase: "prepared".into(),
                continuation_method: "reviewed_transfer".into(),
                reason: None,
                coverage: t.history_revision,
                budget_bytes: super::service::CONTEXT_BUDGET as u64,
                context_digest: Some(r.expected_digest.clone()),
                stop_supervision_qualified: true,
            });
            t.next_account_id = r.account_id.clone();
            t.revision += 1;
            s.mutate_task(&r, &r.operation_id, &t, None)?;
            Ok(t)
        })
    }
}
