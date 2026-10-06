use super::{types::TaskState, AgentRuntime};
use crate::{cli_catalog::TitleCli, terminal::Shells};
use std::sync::atomic::Ordering;
use tauri::AppHandle;
pub(crate) fn model_parts(cli: TitleCli, model: &str) -> Result<(Option<&str>, &str), String> {
    if matches!(cli, TitleCli::Pi | TitleCli::Kilo | TitleCli::Opencode) {
        let (p, m) = model
            .split_once('/')
            .filter(|(p, m)| !p.is_empty() && !m.is_empty())
            .ok_or("Choose an exact provider/model identifier.")?;
        Ok((Some(p), m))
    } else {
        Ok((None, model))
    }
}
pub(crate) fn execute(state: AgentRuntime, app: AppHandle, _: Shells, id: String) {
    let _ = state.with(&app, |i| {
        let s = i.store.as_mut().unwrap();
        let mut t = s.task(&id)?;
        t.state = TaskState::Stopped;
        t.active_account_id = None;
        t.active_attempt_id = None;
        t.status_message = "Managed native execution is unavailable on this platform.".into();
        t.revision += 1;
        s.update_task(&t, None, None)?;
        if let Some(active) = i.active.remove(&id) {
            i.account_leases.remove(&active.account);
            active.done.store(true, Ordering::SeqCst);
        }
        Ok(())
    });
    state.changed(&app, Some(&id));
}
