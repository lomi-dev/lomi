use super::{
    native_wire::NativeKind,
    store::{workspace, Store},
    types::*,
};

use crate::{cli_catalog::TitleCli, terminal::Shells};

use serde_json::{json, Value};

use sha2::{Digest, Sha256};

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use tauri::{AppHandle, Emitter, Manager};

pub(crate) const CONTEXT_BUDGET: usize = 128 * 1024;

#[derive(Clone)]
pub(crate) struct Active {
    pub attempt: String,
    pub generation: u64,
    pub account: String,
    pub stop: Arc<AtomicBool>,
    pub abort: Arc<AtomicBool>,
    pub done: Arc<AtomicBool>,
    pub control: Arc<Mutex<()>>,
    pub worker_finished: Arc<AtomicBool>,
}

pub(crate) struct Permission {
    pub preview: PendingPermission,
    pub decision: Option<(Value, bool, String)>,
    pub consumed: bool,
}

#[derive(Default)]
pub(crate) struct Inner {
    pub store: Option<Store>,
    pub active: HashMap<String, Active>,
    pub account_leases: HashSet<String>,
    pub finished_account_leases: HashSet<String>,
    pub closing_tasks: HashSet<String>,
    pub permissions: HashMap<String, Permission>,
    pub freezes: HashMap<String, (String, String, u64, bool)>,
    pub recovery_projects: HashMap<String, crate::project_write_guard::Lease>,
    pub pending_finishes: HashMap<String, (Active, Result<(), String>)>,
    pub pending_observations: HashMap<String, Task>,
    #[cfg(target_os = "macos")]
    pub finished_owned_operations: HashMap<String, (super::host_child::Context, bool)>,
    #[cfg(target_os = "macos")]
    pub account_terminal_finishes: HashMap<String, Arc<AtomicBool>>,
    pub pending_event_batches: HashMap<String, (Attempt, Vec<super::native_wire::NativeEvent>)>,
}

#[derive(Clone, Default)]
pub(crate) struct AgentRuntime {
    pub inner: Arc<Mutex<Inner>>,
    pub closing: Arc<AtomicBool>,
}

impl AgentRuntime {
    pub(crate) fn with<T>(
        &self,
        app: &AppHandle,
        f: impl FnOnce(&mut Inner) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "Runtime owner unavailable.")?;

        if inner.store.is_none() {
            let mut store = Store::open(
                app.path()
                    .app_data_dir()
                    .map_err(|_| "Runtime storage unavailable.")?
                    .join("agent-runtime"),
            )?;

            #[cfg(target_os = "macos")]
            store.reconcile_owned_helpers()?;
            store.reconcile_processes()?;

            for task in store.tasks()? {
                if !store.processes_settled(&task.task_id)? && task.generation > 0 {
                    let lease=crate::project_write_guard::activate(Path::new(&task.cwd),&task.task_id,task.generation).map_err(|_|"An interrupted native project's recovery fence could not be restored. Runtime initialization is blocked; original storage is preserved.")?;
                    inner.recovery_projects.insert(task.task_id, lease);
                }
            }

            let mut protected_roots = inner
                .recovery_projects
                .keys()
                .map(|id| store.task(id).map(|t| t.cwd))
                .collect::<Result<HashSet<_>, _>>()?;
            for operation in store.native_operations()? {
                if !store.helper_requires_live_fence(&operation)? {
                    continue;
                }
                if protected_roots.insert(operation.cwd.clone()) {
                    let key = operation
                        .task_id
                        .clone()
                        .unwrap_or_else(|| operation.operation_id.clone());
                    let lease=crate::project_write_guard::activate(Path::new(&operation.cwd),&key,operation.generation.unwrap_or(1)).map_err(|_|"An interrupted native helper workspace cannot be fenced; runtime initialization is blocked.")?;
                    inner.recovery_projects.insert(key, lease);
                }
            }
            inner.store = Some(store);
        }

        #[cfg(target_os = "macos")]
        super::owned_operation::retry_finished(&mut inner)?;

        #[cfg(unix)]
        {
            super::execution::retry_observations(&mut inner)?;
            super::execution::retry_finishes(&mut inner)?;
        }
        let dead = inner.store.as_mut().unwrap().reconcile_processes()?;

        let mut settled_workers = Vec::new();

        for (id, active) in &inner.active {
            if active.worker_finished.load(Ordering::SeqCst)
                && inner.store.as_ref().unwrap().processes_settled(id)?
            {
                settled_workers.push((id.clone(), active.clone()));
            }
        }

        for (id, active) in settled_workers {
            inner.active.remove(&id);

            inner.account_leases.remove(&active.account);

            active.done.store(true, Ordering::SeqCst);
        }

        for account in dead {
            if !inner.active.values().any(|a| a.account == account) {
                inner.account_leases.remove(&account);
            }
        }

        let unresolved = inner.store.as_ref().unwrap().unresolved_accounts()?;

        let finished = inner
            .finished_account_leases
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        for account in finished {
            if !unresolved.contains(&account)
                && !inner.active.values().any(|a| a.account == account)
            {
                inner.account_leases.remove(&account);
                inner.finished_account_leases.remove(&account);
            }
        }
        for account in unresolved {
            inner.account_leases.insert(account);
        }

        f(&mut inner)
    }

    pub(crate) fn changed(&self, app: &AppHandle, task_id: Option<&str>) {
        if let Ok(revision) = self.with(app, |inner| inner.store.as_ref().unwrap().revision()) {
            for label in ["main", "settings"] {
                let _ = app.emit_to(
                    tauri::EventTarget::webview(label),
                    "agent-runtime-changed",
                    json!({
                    "revision":revision,"taskId":task_id}
                    ),
                );
            }
        }
    }

    pub(crate) fn accounts(&self, app: &AppHandle) -> Result<AccountsSnapshot, String> {
        self.with(app, |i| snapshot(i.store.as_ref().unwrap()))
    }

    pub(crate) fn tasks(&self, app: &AppHandle) -> Result<TasksSnapshot, String> {
        self.with(app, |i| {
            let s = i.store.as_ref().unwrap();

            Ok(TasksSnapshot {
                schema: 1,
                revision: s.revision()?,
                tasks: s.tasks()?,
            })
        })
    }

    pub(crate) fn task(&self, app: &AppHandle, id: &str) -> Result<Task, String> {
        self.with(app, |i| i.store.as_ref().unwrap().task(id))
    }

    pub(crate) fn account_create(
        &self,
        app: &AppHandle,
        r: AccountCreate,
    ) -> Result<AccountsSnapshot, String> {
        self.with(app, |i| {
            let s = i.store.as_mut().unwrap();

            if let Some(old) = s.replay(&r.operation_id, &r)? {
                return Ok(old);
            }

            label(&r.label)?;

            let id = super::new_id()?;

            let root = s.root.join("accounts").join(&id);

            crate::chat::storage::reject_link(&root)?;

            std::fs::create_dir(&root).map_err(|_| "Cannot create native account namespace.")?;

            crate::chat::storage::private(&root, true)?;

            let capability = capabilities().into_iter().find(|c| c.cli == r.cli).unwrap();

            let a = AccountInstance {
                account_id: id.clone(),
                cli: r.cli,
                label: r.label.clone(),
                enabled: true,
                revision: 1,
                auth_revision: 1,
                auth_state: "unverified".into(),
                availability_reason: (!capability.account_terminal).then_some(capability.reason),
                accepted_version: None,
                recovery: None,
            };

            let b = CredentialBinding {
                account_id: id,
                auth_revision: 1,
                physical_root: root.to_string_lossy().into_owned(),
                namespace: format!("native:{:?}", r.cli),
                credential_reference: "native-managed".into(),
            };

            let mut result = snapshot(s)?;

            result.revision += 1;

            result.accounts.push(a.clone());

            result
                .accounts
                .sort_by(|a, b| a.account_id.cmp(&b.account_id));

            s.save_account(&r, &r.operation_id, &a, &b, &result)?;

            Ok(result)
        })
    }

    pub(crate) fn account_update(
        &self,
        app: &AppHandle,
        r: AccountUpdate,
    ) -> Result<AccountsSnapshot, String> {
        self.with(app,|i|{
let s=i.store.as_mut().unwrap();

if let Some(old)=s.replay(&r.operation_id,&r)?{
return Ok(old);

}
 if account_control_reserved(s,&i.account_leases,&r.account_id)?{
return Err("Close the account terminal or stop its active task before changing this account.".into());

}
let mut a=s.account(&r.account_id)?;

expected(a.revision,r.expected_revision)?;

if let Some(v)=&r.label{
label(v)?;

a.label=v.clone();

}
if let Some(v)=r.enabled{
a.enabled=v;

}
a.revision+=1;

let b=s.binding(&a.account_id)?;

let mut result=snapshot(s)?;

result.revision+=1;

*result.accounts.iter_mut().find(|v|v.account_id==a.account_id).unwrap()=a.clone();

s.save_account(&r,&r.operation_id,&a,&b,&result)?;

Ok(result)}
)
    }

    pub(crate) fn account_remove(
        &self,
        app: &AppHandle,
        r: AccountRemove,
    ) -> Result<AccountsSnapshot, String> {
        self.with(app, |i| {
            let s = i.store.as_mut().unwrap();

            if let Some(old) = s.replay(&r.operation_id, &r)? {
                return Ok(old);
            }

            if account_control_reserved(s, &i.account_leases, &r.account_id)? {
                return Err("This account has an active credential binding lease.".into());
            }

            expected(s.account(&r.account_id)?.revision, r.expected_revision)?;

            let mut result = snapshot(s)?;

            result.accounts.retain(|v| v.account_id != r.account_id);

            result.revision += 1;

            s.delete_account(&r, &r.operation_id, &r.account_id, &result)?;

            Ok(result)
        })
    }

    pub(crate) fn task_create(&self, app: &AppHandle, r: TaskCreate) -> Result<Task, String> {
        self.with(app, |i| {
            let s = i.store.as_mut().unwrap();

            if let Some(old) = s.replay(&r.operation_id, &r)? {
                return Ok(old);
            }

            let a = target(s, &r.account_id, r.auth_revision, None)?;

            model(a.cli, &r.model, r.reasoning_effort.as_deref())?;

            label(&r.title)?;

            let (cwd, identity) = workspace(Path::new(&r.cwd))?;

            let t = Task {
                task_id: super::new_id()?,
                cwd,
                title: r.title.clone(),
                cli: Some(a.cli),
                availability_reason: None,
                model: r.model.clone(),
                reasoning_effort: r.reasoning_effort.clone(),
                revision: 1,
                history_revision: 0,
                generation: 0,
                state: TaskState::Idle,
                next_account_id: a.account_id.clone(),
                active_account_id: None,
                active_attempt_id: None,
                status_message: "Ready. Choose an account for every Send or Continue.".into(),
                attempts: vec![],
                history: vec![],
                grants: vec![HistoryGrant {
                    account_id: a.account_id,
                    auth_revision: a.auth_revision,
                    revision: 1,
                }],
                switches: vec![],
            };

            s.create_task(
                &r,
                &r.operation_id,
                &t,
                r.shell_profile_id.as_deref().unwrap_or(""),
                identity,
            )?;

            Ok(t)
        })
    }

    pub(crate) fn grant(&self, app: &AppHandle, r: GrantUpdate) -> Result<Task, String> {
        let active = self.with(app, |i| Ok(i.active.get(&r.task_id).cloned()))?;

        let _control = active
            .as_ref()
            .map(|a| a.control.lock().map_err(|_| "Task control unavailable."))
            .transpose()?;

        self.with(app, |i| {
            let s = i.store.as_mut().unwrap();

            if let Some(old) = s.replay(&r.operation_id, &r)? {
                return Ok(old);
            }

            let a = target(s, &r.account_id, r.auth_revision, None)?;

            let mut t = s.task(&r.task_id)?;

            expected(t.revision, r.expected_revision)?;

            expected(t.history_revision, r.expected_history_revision)?;

            if Some(a.cli) != t.cli {
                return Err("History grants are restricted to this task's CLI family.".into());
            }

            t.grants.retain(|g| g.account_id != a.account_id);

            if r.allow {
                t.grants.push(HistoryGrant {
                    account_id: a.account_id,
                    auth_revision: a.auth_revision,
                    revision: t.revision + 1,
                });
            }

            t.revision += 1;

            s.mutate_task(&r, &r.operation_id, &t, None)?;

            Ok(t)
        })
    }

    pub(crate) fn send(
        &self,
        app: &AppHandle,
        shells: &Shells,
        r: TaskSend,
    ) -> Result<Task, String> {
        let (task, dispatch) = self.with(app, |i| {
            prepare_send(i, &r, self.closing.load(Ordering::SeqCst))
        })?;

        if dispatch {
            self.launch(app, shells, &task.task_id);
        }

        self.changed(app, Some(&task.task_id));

        Ok(task)
    }

    fn launch(&self, app: &AppHandle, shells: &Shells, id: &str) {
        let state = self.clone();

        let app = app.clone();

        let shells = shells.clone();

        let id = id.to_owned();

        std::thread::spawn(move || super::execution::execute(state, app, shells, id));
    }

    pub(crate) fn stop(&self, app: &AppHandle, r: TaskStop) -> Result<Task, String> {
        let active = self.with(app, |i| Ok(i.active.get(&r.task_id).cloned()))?;

        let _control = active
            .as_ref()
            .map(|a| a.control.lock().map_err(|_| "Task control unavailable."))
            .transpose()?;

        let t = self.with(app, |i| {
            let s = i.store.as_mut().unwrap();

            if let Some(old) = s.replay(&r.operation_id, &r)? {
                return Ok(old);
            }

            let mut t = s.task(&r.task_id)?;

            expected(t.revision, r.expected_revision)?;

            if let Some(a) = i
                .active
                .get(&t.task_id)
                .filter(|a| !a.worker_finished.load(Ordering::SeqCst))
            {
                a.stop.store(true, Ordering::SeqCst);

                t.state = TaskState::Stopping;

                t.status_message =
                    "Stopping source; native tool settlement and descendant drain are required."
                        .into();
            } else if !matches!(
                t.state,
                TaskState::RecoveryRequired | TaskState::DeliveryUncertain
            ) {
                t.state = TaskState::Stopped;
            }

            t.revision += 1;

            s.mutate_task(&r, &r.operation_id, &t, None)?;

            Ok(t)
        })?;

        // Restarted owners may have no live Rust worker. Cleanup uses only the
        // durable private witness and exact OS identity; it never resumes inference.
        #[cfg(unix)]
        if active
            .as_ref()
            .is_none_or(|a| a.worker_finished.load(Ordering::SeqCst))
        {
            let markers = self.with(app, |i| {
                i.store.as_ref().unwrap().process_markers(&r.task_id)
            })?;
            for marker in markers {
                super::process_supervision::stop(&marker)?;
            }
            self.with(app, |i| {
                let s = i.store.as_mut().unwrap();
                s.reconcile_processes()?;
                if !s.processes_settled(&r.task_id)? {
                    return Err("Process creation or descendant ownership remains unknown; recovery and binding changes remain fenced.".into());
                }
                Ok(())
            })?;
        }
        self.changed(app, Some(&t.task_id));

        Ok(t)
    }

    pub(crate) fn recover(&self, app: &AppHandle, r: TaskRecover) -> Result<Task, String> {
        self.with(app,|i|{

        let s=i.store.as_mut().unwrap();

        if let Some(old)=s.replay(&r.operation_id,&r)?{
return Ok(old);
}

        let mut t=s.task(&r.task_id)?;
        if let Some(reason)=&t.availability_reason {return Err(reason.clone());}

        expected(t.revision,r.expected_revision)?;

        expected(t.history_revision,r.expected_history_revision)?;

        if !s.processes_settled(&t.task_id)? || !s.helpers_settled(Some(&t.task_id))? {
return Err("The previous native process group is still alive or its identity is unknown. Effects acknowledgement cannot release its credential or project ownership.".into());
}

        if i.active.contains_key(&t.task_id)||!matches!(t.state,TaskState::RecoveryRequired|TaskState::DeliveryUncertain|TaskState::Archived)||!r.acknowledge_effects{
return Err("Review retained observations and acknowledge uncertain effects after process drain.".into());
}

        t.state=TaskState::Stopped;

        t.status_message="Effects explicitly reviewed. A new explicit Send or Continue may use complete observed context; no input was replayed.".into();

        for a in &mut t.attempts{
if a.effects_state!="settled"{
a.effects_state="reviewed".into();
}
}

        if s.directory_identity(&t.task_id)?==(0,0){
s.adopt_directory(&t.task_id,workspace(Path::new(&t.cwd))?.1)?;
}

        t.revision+=1;

        s.mutate_task(&r,&r.operation_id,&t,None)?;

        i.recovery_projects.remove(&t.task_id);

        for freeze in i.freezes.values_mut(){
if freeze.0==t.task_id{
freeze.3=true;
}
}

        Ok(t)
      }
)
    }

    pub(crate) fn prepare_switch(&self, app: &AppHandle, r: SwitchPrepare) -> Result<Task, String> {
        let active = self.with(app, |i| Ok(i.active.get(&r.task_id).cloned()))?;

        let _control = active
            .as_ref()
            .map(|a| a.control.lock().map_err(|_| "Task control unavailable."))
            .transpose()?;

        self.with(app, |i| {
            let s = i.store.as_mut().unwrap();

            if let Some(old) = s.replay(&r.operation_id, &r)? {
                return Ok(old);
            }

            let mut t = s.task(&r.task_id)?;

            if t.cli.is_none() {
                return Err(t.availability_reason.clone().unwrap_or(
                    "The archived CLI family cannot be qualified from its retained provenance."
                        .into(),
                ));
            }

            expected(t.revision, r.expected_revision)?;

            expected(t.history_revision, r.expected_history_revision)?;

            if !matches!(r.mode.as_str(), "next_turn" | "stop_and_continue") {
                return Err("Invalid switch mode.".into());
            }

            if r.source_attempt_id
                .as_ref()
                .is_some_and(|v| t.active_attempt_id.as_ref().is_some_and(|a| v != a))
            {
                return Err("The source attempt changed.".into());
            }

            t.next_account_id = r.account_id.clone();

            let eligibility = target(s, &r.account_id, r.auth_revision, Some(&t))
                .and_then(|a| continuation(s, &t, &a));

            let (mut method, mut reason) = match eligibility {
                Ok(v) => (v, None),
                Err(v) => ("blocked".into(), Some(v)),
            };

            let phase = if r.mode == "next_turn" {
                "prepared"
            } else if let Some(active) = i.active.get(&t.task_id) {
                active.stop.store(true, Ordering::SeqCst);

                method = "blocked".into();

                reason = None;

                t.state = TaskState::Stopping;

                "stopping_source"
            } else if matches!(
                t.state,
                TaskState::RecoveryRequired | TaskState::DeliveryUncertain
            ) {
                "recovery_required"
            } else {
                t.state = TaskState::Prepared;

                "prepared"
            };

            t.switches.push(SwitchOperation {
                operation_id: r.operation_id.clone(),
                source_attempt_id: r
                    .source_attempt_id
                    .clone()
                    .or_else(|| t.active_attempt_id.clone()),
                account_id: r.account_id.clone(),
                auth_revision: r.auth_revision,
                history_revision: r.expected_history_revision,
                mode: r.mode.clone(),
                phase: phase.into(),
                continuation_method: method,
                reason,
                coverage: t.history_revision,
                budget_bytes: CONTEXT_BUDGET as u64,
                context_digest: Some(history_digest(&t)?),
                stop_supervision_qualified: r.mode != "stop_and_continue"
                    || phase != "stopping_source",
            });

            t.revision += 1;

            s.mutate_task(&r, &r.operation_id, &t, None)?;

            Ok(t)
        })
    }

    pub(crate) fn commit_switch(
        &self,
        app: &AppHandle,
        shells: &Shells,
        r: SwitchCommit,
    ) -> Result<Task, String> {
        let (t, dispatch) = self.with(app, |i| {
            prepare_switch_commit(i, &r, self.closing.load(Ordering::SeqCst))
        })?;

        if dispatch {
            self.launch(app, shells, &t.task_id);
        }

        self.changed(app, Some(&t.task_id));

        Ok(t)
    }

    pub(crate) fn drain(&self, app: &AppHandle, ids: &[String]) -> Result<(), String> {
        let mut sorted_ids = ids.to_vec();

        sorted_ids.sort();

        sorted_ids.dedup();

        let ids = sorted_ids.as_slice();

        let controls = self.with(app, |i| {
            for id in ids {
                i.closing_tasks.insert(id.clone());
            }

            Ok(ids
                .iter()
                .filter_map(|id| i.active.get(id).cloned())
                .collect::<Vec<_>>())
        })?;

        let guards = controls
            .iter()
            .map(|a| a.control.lock().map_err(|_| "Task control unavailable."))
            .collect::<Result<Vec<_>, _>>()?;

        let active = self.with(app, |i| {
            let mut list = Vec::new();

            for id in ids {
                i.closing_tasks.insert(id.clone());

                if let Some(a) = i.active.get(id) {
                    a.stop.store(true, Ordering::SeqCst);

                    list.push(a.clone());
                }
            }

            Ok(list)
        })?;

        drop(guards);

        let deadline = Instant::now() + Duration::from_secs(40);

        while active.iter().any(|v| !v.done.load(Ordering::SeqCst)) {
            self.with(app, |_| Ok(()))?;

            if Instant::now() >= deadline {
                for a in &active {
                    a.abort.store(true, Ordering::SeqCst);
                }

                return Err(
                    "Native drain timed out. Recovery and retained close fences block shutdown."
                        .into(),
                );
            }

            std::thread::sleep(Duration::from_millis(20));
        }

        self.with(app,|i| {
            let s=i.store.as_ref().unwrap();
            for id in ids {
                if i.pending_finishes.contains_key(id)||!s.processes_settled(id)? || !s.helpers_settled(Some(id))? {
                    return Err("Native process ownership or final durable record remains unresolved; close is fenced.".into());
                }
            }
            Ok(())
        })
    }

    pub(crate) fn drain_all(&self, app: &AppHandle) -> Result<(), String> {
        self.closing.store(true, Ordering::SeqCst);

        // Login PTYs are helper operations, not task Active entries. Retire
        // only owned terminals outside the runtime owner before checking the
        // durable helper fence; ordinary terminals retain their quit flow.
        #[cfg(target_os = "macos")]
        app.state::<crate::terminal::Terminals>()
            .stop_owned_and_wait()?;
        #[cfg(target_os = "macos")]
        {
            let finished = self
                .inner
                .lock()
                .map_err(|_| "Runtime owner unavailable.")?
                .account_terminal_finishes
                .values()
                .cloned()
                .collect::<Vec<_>>();
            let deadline = Instant::now() + Duration::from_secs(8);
            while finished.iter().any(|done| !done.load(Ordering::SeqCst)) {
                if Instant::now() >= deadline {
                    return Err("Owned login helper finalization is still pending; the terminal view is retained.".into());
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }

        let ids = self
            .tasks(app)?
            .tasks
            .into_iter()
            .map(|v| v.task_id)
            .collect::<Vec<_>>();

        self.drain(app, &ids)?;
        self.with(app, |i| {
            if !i.store.as_ref().unwrap().helpers_settled(None)? {
                return Err("Native setup, verification or login ownership remains unresolved; shutdown is fenced.".into());
            }
            Ok(())
        })
    }

    fn release_account_lease(&self, id: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.finished_account_leases.insert(id.to_owned());
            if !inner.active.values().any(|a| a.account == id)
                && inner
                    .store
                    .as_ref()
                    .and_then(|s| s.unresolved_accounts().ok())
                    .is_some_and(|accounts| !accounts.iter().any(|a| a == id))
            {
                inner.account_leases.remove(id);
                inner.finished_account_leases.remove(id);
            }
        }
    }

    pub(crate) fn stop_all(&self) {
        self.closing.store(true, Ordering::SeqCst);

        if let Ok(i) = self.inner.lock() {
            for a in i.active.values() {
                a.stop.store(true, Ordering::SeqCst);

                a.abort.store(true, Ordering::SeqCst);
            }
        }
    }

    pub(crate) fn cancel_close(
        &self,
        app: &AppHandle,
        ids: Option<&[String]>,
    ) -> Result<(), String> {
        self.with(app, |i| {
            if let Some(ids) = ids {
                for id in ids {
                    i.closing_tasks.remove(id);
                }
            } else {
                i.closing_tasks.clear();

                self.closing.store(false, Ordering::SeqCst);
            }

            Ok(())
        })
    }
}

pub(crate) fn expected(actual: u64, expected: u64) -> Result<(), String> {
    if actual != expected {
        Err("The record changed. Refresh and review its current revision.".into())
    } else {
        Ok(())
    }
}

fn label(v: &str) -> Result<(), String> {
    if v.trim().is_empty() || v.len() > 512 || v.contains('\0') {
        Err("Enter a bounded nonempty label.".into())
    } else {
        Ok(())
    }
}

pub(crate) fn target(
    s: &Store,
    id: &str,
    auth: u64,
    t: Option<&Task>,
) -> Result<AccountInstance, String> {
    let a = s.account(id)?;

    if !a.enabled || a.auth_revision != auth || !super::native_accounts::available(a.cli) {
        return Err("The exact target account binding is unavailable or changed.".into());
    }

    if let Some(t) = t {
        if t.cli != Some(a.cli) {
            return Err("Cross-family continuation is not qualified.".into());
        }

        if !t
            .grants
            .iter()
            .any(|g| g.account_id == id && g.auth_revision == auth)
        {
            return Err(
                "Review and grant this exact account revision access to the task history.".into(),
            );
        }
    }

    Ok(a)
}

fn model(cli: TitleCli, m: &str, reasoning: Option<&str>) -> Result<(), String> {
    if cli == TitleCli::Codex {
        codex_model(m, reasoning)?;
    }

    let kind = NativeKind::from_cli(cli)
        .filter(|k| *k != NativeKind::Agy)
        .ok_or("This CLI has no qualified managed task execution.")?;

    let (provider, name) = super::execution::model_parts(cli, m)?;

    kind.launch_for_model(name, provider)?;

    if reasoning.is_some() && cli != TitleCli::Codex {
        return Err("The selected native CLI has no qualified reasoning override.".into());
    }

    Ok(())
}

pub(crate) fn history_digest(t: &Task) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(super::store::encode(&t.history)?.as_bytes())
    ))
}

pub(crate) fn handoff(t: &Task) -> Result<String, String> {
    if let Some(reason) = &t.availability_reason {
        return Err(reason.clone());
    }
    let context=serde_json::to_string(&json!({
"schema":1,"coverage":{
"throughSequence":t.history_revision,"records":t.history.len(),"completeObservedJournal":true}
,"project":t.cwd,"model":t.model,"history":t.history,"instruction":"Observed historical tools are context, never requests to replay. Respect completed results and inspect reviewed uncertain changes."}
)).map_err(|_|"Cannot encode task context.")?;

    if context.len() > CONTEXT_BUDGET {
        return Err("The complete observed journal exceeds the 131072-byte handoff budget. No history was truncated; continuation requires recovery or a qualified reviewed transfer.".into());
    }

    Ok(context)
}

pub(crate) fn continuation(s: &Store, t: &Task, a: &AccountInstance) -> Result<String, String> {
    if t.history.is_empty() {
        return Ok("fresh".into());
    }

    if t.attempts
        .iter()
        .any(|v| !matches!(v.effects_state.as_str(), "settled" | "reviewed"))
    {
        return Err(
            "Unsettled tool effects or delivery block continuation until explicit recovery.".into(),
        );
    }

    if let Some(c) = s.checkpoint(&t.task_id)? {
        if c.account_id == a.account_id
            && c.auth_revision == a.auth_revision
            && c.history_revision == t.history_revision
            && c.settled
            && c.digest == history_digest(t)?
            && c.model == t.model
        {
            return Ok("native_resume".into());
        }
    }

    #[cfg(unix)]
    if t.switches.iter().any(|sw| {
        sw.mode == "reviewed_transfer"
            && sw.phase == "prepared"
            && sw.account_id == a.account_id
            && sw.auth_revision == a.auth_revision
            && sw.history_revision == t.history_revision
    }) {
        let root = s.root.join("tasks").join(&t.task_id).join("native");

        let req = super::transfer::request(s, t, &a.account_id, a.auth_revision)?;

        let receipt = super::native_transfer::read(&root, t.generation + 1)?;

        super::native_transfer::fence(&root, &req, &receipt)?;

        return Ok("reviewed_transfer".into());
    }

    handoff(t)?;

    Ok("handoff".into())
}

pub(crate) fn append(t: &mut Task, a: &Attempt, kind: &str, state: &str, content: Value) {
    t.history_revision += 1;

    t.history.push(HistoryRecord {
        sequence: t.history_revision,
        attempt_id: a.attempt_id.clone(),
        account_id: a.account_id.clone(),
        auth_revision: a.auth_revision,
        kind: kind.into(),
        state: state.into(),
        content,
    });
}

fn snapshot(s: &Store) -> Result<AccountsSnapshot, String> {
    let mut accounts = s.accounts()?;
    for account in &mut accounts {
        account.recovery = s.account_recovery(&account.account_id)?;
    }
    Ok(AccountsSnapshot {
        schema: 1,
        revision: s.revision()?,
        accounts,
        capabilities: capabilities(),
    })
}

pub(super) fn capabilities() -> Vec<Capability> {
    [TitleCli::Codex,TitleCli::Claude,TitleCli::Pi,TitleCli::Kimi,TitleCli::Kilo,TitleCli::Opencode,TitleCli::Grok,TitleCli::Agy,TitleCli::Gemini,TitleCli::Cursor,TitleCli::Copilot,TitleCli::Openclaw,TitleCli::Hermes,TitleCli::Qwen,TitleCli::Kiro,TitleCli::Vibe].into_iter().map(|cli|{
let kind=NativeKind::from_cli(cli);

let available=super::native_accounts::available(cli);

Capability{
cli,account_terminal:available,managed_execution:available&&cli!=TitleCli::Agy&&(cli!=TitleCli::Pi||cfg!(all(target_os="macos",target_arch="aarch64"))),versions:kind.map(|k|k.versions().iter().map(|s|(*s).into()).collect()).unwrap_or_default(),cross_account_native_resume:false,reviewed_transfer:available&&cli==TitleCli::Pi,stop_and_continue_qualified:false,stop_and_continue_reason:Some("Remote and tool-specific effects are not qualified for stop and continue.".into()),reason:if available{
if cli==TitleCli::Agy{
"Qualified account terminal only; native PTY confirmations apply."}
else{
if cli==TitleCli::Claude {"Managed Claude uses held host containment and closed passive configuration while preserving native Bash, Read and Edit. When Lomi Control is enabled, private owned MCP permits task-scoped project observations. Sealed cohort receipts prove local retirement in the same OS boot; unknown remote or tool effects still require review."} else if cli==TitleCli::Codex {"Managed Codex uses held host containment, closed passive configuration and file credential storage. When Lomi Control is enabled, private owned MCP permits task-scoped project observations. Sealed cohort receipts prove local retirement in the same OS boot; unknown remote or tool effects still require review."} else if cli==TitleCli::Pi {"Managed Pi requires the pinned dependency closure and Node 22.22.3 on macOS ARM64, and disables extensions. Background, configuration and lifecycle helpers remain unqualified even for text-only turns; ownership stays protected until a verified OS restart."} else {"Native tools and permissions are supported, but this client's background or lifecycle effects remain unqualified. Ownership stays protected until a verified OS restart."}}
}
else{
"Managed native account isolation and execution are unqualified on this CLI/platform; independent terminal integrations remain available."}
.into()}
}
).collect()
}

pub(crate) fn account_terminal<T>(
    state: &AgentRuntime,
    app: &AppHandle,
    shells: &Shells,
    shell_id: &str,
    cwd: &str,
    id: &str,
    start: impl FnOnce(
        crate::terminal::NativeTerminalLaunch,
        Arc<AtomicBool>,
        (String, String),
    ) -> Result<T, String>,
) -> Result<T, String> {
    let (a, b, helper) = state.with(app, |i| {
        let s = i.store.as_mut().unwrap();
        if state.closing.load(Ordering::SeqCst)
            || account_control_reserved(s, &i.account_leases, id)? {
            return Err("The exact native account namespace is already leased or closing.".into());
        }
        let mut a = s.account(id)?;
        if !a.enabled || !super::native_accounts::available(a.cli) {
            return Err("This account has no admitted isolated terminal on this platform.".into());
        }
        let mut b = s.binding(id)?;
        // Invalidate the old auth/grants before preparing any login helper.
        a.auth_revision = a.auth_revision.checked_add(1).ok_or("Auth revision exhausted.")?;
        a.revision = a.revision.checked_add(1).ok_or("Revision exhausted.")?;
        a.auth_state = "unverified".into();
        b.auth_revision = a.auth_revision;
        let mutation = super::new_id()?;
        let request = json!({"operationId":mutation,"accountId":id,"terminalLogin":true,"authRevision":a.auth_revision});
        let mut result = snapshot(s)?;
        result.revision += 1;
        *result.accounts.iter_mut().find(|v|v.account_id==id).unwrap()=a.clone();
        s.save_account(&request,&mutation,&a,&b,&result)?;
        for mut task in s.tasks()? {
            let old = task.grants.len();
            task.grants.retain(|grant|grant.account_id!=id);
            if old != task.grants.len() {
                task.revision += 1;
                s.update_task(&task,None,None)?;
            }
        }
        let operation = begin_managed_helper(i,state.closing.load(Ordering::SeqCst),&a,cwd,"account_terminal")?;
        Ok((a,b,operation))
    })?;
    #[cfg(not(target_os = "macos"))]
    let operation = helper.operation;
    #[cfg(target_os = "macos")]
    let context = helper
        .context
        .expect("owned helper context was validated before commit");
    #[cfg(target_os = "macos")]
    let helper_finished = helper
        .finished
        .expect("owned login finalization token was retained");
    #[cfg(target_os = "macos")]
    let mut operation_guard =
        super::owned_operation::OwnedOperation::new(state, app, context.clone());
    let done = Arc::new(AtomicBool::new(false));
    let result = (|| {
        let prepared = super::native_process::Prepared::prepare(
            app,
            shells,
            shell_id,
            cwd,
            a.cli,
            Path::new(&b.physical_root),
            Arc::new(AtomicBool::new(false)),
            #[cfg(target_os = "macos")]
            Some(context),
            #[cfg(not(target_os = "macos"))]
            None,
        )?;
        let version = prepared.version().to_owned();
        let command = prepared.terminal_command()?;
        state.with(app, |i| {
            let s = i.store.as_mut().unwrap();
            let mut current = s.account(id)?;
            expected(current.revision, a.revision)?;
            current.accepted_version = Some(version);
            current.revision += 1;
            let mutation = super::new_id()?;
            let request = json!({"operationId":mutation,"accountId":id,"terminalVersion":true});
            let mut result = snapshot(s)?;
            result.revision += 1;
            *result
                .accounts
                .iter_mut()
                .find(|v| v.account_id == id)
                .unwrap() = current.clone();
            s.save_account(&request, &mutation, &current, &b, &result)
        })?;
        start(
            command,
            done.clone(),
            (a.account_id.clone(), a.label.clone()),
        )
    })();
    if result.is_ok() {
        let state = state.clone();
        let app = app.clone();
        let id = id.to_owned();
        std::thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(25));
            }
            #[cfg(target_os = "macos")]
            let _ = operation_guard.finish();
            #[cfg(target_os = "macos")]
            helper_finished.store(true, Ordering::SeqCst);
            state.release_account_lease(&id);
            state.changed(&app, None);
        });
    } else {
        #[cfg(target_os = "macos")]
        let _ = operation_guard.finish();
        #[cfg(target_os = "macos")]
        helper_finished.store(true, Ordering::SeqCst);
        state.release_account_lease(id);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = operation;
    state.changed(app, None);
    result
}

impl AgentRuntime {
    pub(crate) fn verify(
        &self,
        app: &AppHandle,
        shells: &Shells,
        r: AccountVerify,
    ) -> Result<AccountsSnapshot, String> {
        let (a, b) = self.with(app, |i| {
            let s = i.store.as_ref().unwrap();

            if let Some(result) = s.replay(&r.operation_id, &r)? {
                return Ok((None, Some(result)));
            }

            let a = s.account(&r.account_id)?;

            expected(a.revision, r.expected_revision)?;

            if account_control_reserved(s, &i.account_leases, &a.account_id)?
                || !a.enabled
                || !super::native_accounts::available(a.cli)
            {
                return Err("Close active account processes before verification.".into());
            }

            let b = s.binding(&a.account_id)?;

            let operation =
                begin_managed_helper(i, self.closing.load(Ordering::SeqCst), &a, &r.cwd, "verify")?;
            Ok((Some((a, b, operation)), None))
        })?;

        if let Some(snapshot) = b {
            return Ok(snapshot);
        }

        let (a, b, helper) = a.unwrap();
        #[cfg(not(target_os = "macos"))]
        let operation = helper.operation;
        #[cfg(target_os = "macos")]
        let context = helper
            .context
            .expect("owned helper context was validated before commit");
        #[cfg(target_os = "macos")]
        let mut operation_guard =
            super::owned_operation::OwnedOperation::new(self, app, context.clone());
        #[cfg(not(target_os = "macos"))]
        let _ = operation;
        let result = (|| {
            let prepared = super::native_process::Prepared::prepare(
                app,
                shells,
                &r.shell_profile_id,
                &r.cwd,
                a.cli,
                Path::new(&b.physical_root),
                Arc::new(AtomicBool::new(false)),
                #[cfg(target_os = "macos")]
                Some(context),
                #[cfg(not(target_os = "macos"))]
                None,
            )?;

            let version = prepared.version().to_owned();

            let auth = match a.cli {
                TitleCli::Codex => {
                    let output =
                        prepared.collect_command(&["login".into(), "status".into()], || Ok(()))?;

                    let text = std::str::from_utf8(&output)
                        .map_err(|_| "Native auth status was not verified.")?;

                    text.trim().starts_with("Logged in")
                }

                TitleCli::Claude => {
                    let output =
                        prepared.collect_command(&["auth".into(), "status".into()], || Ok(()))?;

                    let value = super::native_wire::strict_json(&output)?;

                    value["loggedIn"]
                        .as_bool()
                        .ok_or("Native auth status schema was not recognized.")?
                }

                _ => false,
            };

            #[cfg(target_os = "macos")]
            operation_guard.finish()?;
            self.with(app, |i| {
                let s = i.store.as_mut().unwrap();
                let mut current = s.account(&a.account_id)?;

                expected(current.revision, a.revision)?;

                current.accepted_version = Some(version);

                current.auth_state = if auth { "verified" } else { "unverified" }.into();

                current.availability_reason =
                    if matches!(a.cli, TitleCli::Codex | TitleCli::Claude) && !auth {
                        Some("Native read-only auth status did not confirm login.".into())
                    } else {
                        None
                    };

                current.revision += 1;

                let mut result = snapshot(s)?;

                result.revision += 1;

                *result
                    .accounts
                    .iter_mut()
                    .find(|v| v.account_id == a.account_id)
                    .unwrap() = current.clone();

                for account in &mut result.accounts {
                    account.recovery = s.account_recovery(&account.account_id)?;
                }
                s.save_account(&r, &r.operation_id, &current, &b, &result)?;

                Ok(result)
            })
        })();

        #[cfg(target_os = "macos")]
        let settlement = operation_guard.finish();
        self.release_account_lease(&a.account_id);
        self.changed(app, None);
        #[cfg(target_os = "macos")]
        settlement?;

        result
    }
}

pub(super) fn prepare_switch_commit(
    i: &mut Inner,
    r: &SwitchCommit,
    closing: bool,
) -> Result<(Task, bool), String> {
    let s = i.store.as_mut().unwrap();

    if let Some(old) = s.replay(&r.operation_id, r)? {
        return Ok((old, false));
    }
    let mut t = s.task(&r.task_id)?;

    expected(t.revision, r.expected_revision)?;

    if closing
        || i.closing_tasks.contains(&t.task_id)
        || i.active.contains_key(&t.task_id)
        || matches!(
            t.state,
            TaskState::RecoveryRequired | TaskState::DeliveryUncertain | TaskState::Archived
        )
    {
        return Err("Wait for source settlement and drain before committing the switch.".into());
    }
    let index = t
        .switches
        .iter()
        .position(|v| v.operation_id == r.switch_operation_id)
        .ok_or("The switch intention is missing.")?;

    let switch = t.switches[index].clone();
    switch_dispatch_allowed(&switch)?;

    if switch.account_id != r.account_id
        || switch.auth_revision != r.auth_revision
        || switch.mode != "stop_and_continue"
        || switch.phase != "prepared"
    {
        return Err("Review a prepared switch to this exact account revision.".into());
    }
    let a = target(s, &r.account_id, r.auth_revision, Some(&t))?;

    if account_control_reserved(s, &i.account_leases, &a.account_id)? {
        return Err("The target account namespace is leased.".into());
    }
    crate::project_write_guard::ensure_no_unscoped_effects()?;

    let method = continuation(s, &t, &a)?;

    let input="Continue the unfinished task using the complete retained history. Respect completed tools and inspect reviewed changes.".into();

    let attempt = Attempt {
        attempt_id: super::new_id()?,
        operation_id: r.operation_id.clone(),
        account_id: a.account_id.clone(),
        auth_revision: a.auth_revision,
        generation: t.generation + 1,
        input,
        continuation_method: method,
        state: "dispatch_intent".into(),
        output: String::new(),
        native_ref: None,
        version: None,
        effects_state: "unsettled".into(),
    };

    t.generation = attempt.generation;

    t.state = TaskState::Starting;

    t.active_account_id = Some(a.account_id.clone());

    t.active_attempt_id = Some(attempt.attempt_id.clone());

    t.next_account_id = a.account_id.clone();

    t.switches[index].phase = "dispatch_pending".into();

    t.switches[index].history_revision = t.history_revision;

    t.switches[index].coverage = t.history_revision;

    t.switches[index].context_digest = Some(history_digest(&t)?);

    append(&mut t, &attempt, "user", "committed", json!(attempt.input));

    t.attempts.push(attempt.clone());

    t.revision += 1;

    s.mutate_task(r, &r.operation_id, &t, Some(&attempt))?;

    i.active.insert(
        t.task_id.clone(),
        Active {
            attempt: attempt.attempt_id,
            generation: attempt.generation,
            account: a.account_id.clone(),
            stop: Arc::new(AtomicBool::new(false)),
            abort: Arc::new(AtomicBool::new(false)),
            done: Arc::new(AtomicBool::new(false)),
            control: Arc::new(Mutex::new(())),
            worker_finished: Arc::new(AtomicBool::new(false)),
        },
    );

    i.account_leases.insert(a.account_id);

    Ok((t, true))
}

pub(super) fn prepare_send(
    i: &mut Inner,
    r: &TaskSend,
    closing: bool,
) -> Result<(Task, bool), String> {
    let s = i.store.as_mut().unwrap();

    if let Some(old) = s.replay(&r.operation_id, &r)? {
        return Ok((old, false));
    }

    if closing || i.closing_tasks.contains(&r.task_id) {
        return Err("This task is preparing to close.".into());
    }

    let mut t = s.task(&r.task_id)?;

    expected(t.revision, r.expected_revision)?;

    t.next_account_id = r.account_id.clone();

    if i.active.contains_key(&t.task_id)
        || matches!(
            t.state,
            TaskState::RecoveryRequired | TaskState::DeliveryUncertain | TaskState::Archived
        )
    {
        return Err(
            "Stop and settle or recover this task before dispatching another attempt.".into(),
        );
    }

    let a = target(s, &r.account_id, r.auth_revision, Some(&t))?;

    if account_control_reserved(s, &i.account_leases, &a.account_id)? {
        return Err("The target account namespace is leased by another process.".into());
    }

    if r.text.len() > CONTEXT_BUDGET
        || r.text.contains('\0')
        || (r.text.trim().is_empty() && !r.continue_requested.unwrap_or(false))
    {
        return Err("Enter a bounded message or explicitly choose Continue.".into());
    }

    crate::project_write_guard::ensure_no_unscoped_effects()?;

    let continuation = continuation(s, &t, &a)?;

    let input = if r.continue_requested.unwrap_or(false) && r.text.trim().is_empty() {
        "Continue the unfinished task. Respect observed completed tools; inspect uncertain changes before acting.".to_string()
    } else {
        r.text.clone()
    };

    let intention = Attempt {
        attempt_id: super::new_id()?,
        operation_id: r.operation_id.clone(),
        account_id: a.account_id.clone(),
        auth_revision: a.auth_revision,
        generation: t.generation + 1,
        input: input.clone(),
        continuation_method: continuation,
        state: "dispatch_intent".into(),
        output: String::new(),
        native_ref: None,
        version: None,
        effects_state: "unsettled".into(),
    };

    append(&mut t, &intention, "user", "committed", json!(input));

    t.generation = intention.generation;

    t.state = TaskState::Starting;

    t.active_account_id = Some(a.account_id.clone());

    t.active_attempt_id = Some(intention.attempt_id.clone());

    t.attempts.push(intention.clone());

    t.revision += 1;

    t.status_message = "Dispatch intention committed. Native initialization follows.".into();

    s.mutate_task(&r, &r.operation_id, &t, Some(&intention))?;

    let active = Active {
        attempt: intention.attempt_id.clone(),
        generation: intention.generation,
        account: a.account_id.clone(),
        stop: Arc::new(AtomicBool::new(false)),
        abort: Arc::new(AtomicBool::new(false)),
        done: Arc::new(AtomicBool::new(false)),
        control: Arc::new(Mutex::new(())),
        worker_finished: Arc::new(AtomicBool::new(false)),
    };

    i.active.insert(t.task_id.clone(), active);

    i.account_leases.insert(a.account_id);

    Ok((t, true))
}

fn codex_model(model: &str, effort: Option<&str>) -> Result<(), String> {
    let catalog: Value = serde_json::from_str(include_str!("codex-models-0.160.0.json"))
        .map_err(|_| "Pinned Codex model metadata is unavailable.")?;

    let entry = catalog["models"]
        .as_array()
        .and_then(|v| v.iter().find(|v| v["slug"] == model))
        .ok_or("Choose a Codex model admitted by the pinned 0.160.0 metadata.")?;

    if let Some(effort) = effort {
        if !entry["supported_reasoning_levels"]
            .as_array()
            .is_some_and(|v| v.iter().any(|v| v["effort"] == effort))
        {
            return Err(
                "This exact Codex model does not support the selected reasoning effort.".into(),
            );
        }
    }

    Ok(())
}

pub(crate) fn settle_switches(s: &Store, t: &mut Task) -> Result<(), String> {
    for index in 0..t.switches.len() {
        if t.switches[index].phase == "stopping_source" {
            if matches!(t.state, TaskState::Stopped | TaskState::Completed) {
                let sw = t.switches[index].clone();

                let eligibility = target(s, &sw.account_id, sw.auth_revision, Some(t))
                    .and_then(|a| continuation(s, t, &a));

                let (method, reason) = match eligibility {
                    Ok(_) if !sw.stop_supervision_qualified => ("blocked".into(),Some("Active Stop→Continue is unqualified for this native driver/version/configuration and descendant boundary. Source stopped; select a completed-turn handoff only after qualified settlement.".into())),
                    Ok(method) => (method, None),
                    Err(reason) => ("blocked".into(), Some(reason)),
                };

                let digest = history_digest(t)?;

                let sw = &mut t.switches[index];

                sw.phase = "prepared".into();

                sw.continuation_method = method;

                sw.reason = reason;

                sw.history_revision = t.history_revision;

                sw.coverage = t.history_revision;

                sw.context_digest = Some(digest);

                t.state = TaskState::Prepared;
            } else {
                t.switches[index].phase = "recovery_required".into();

                t.switches[index].reason = Some(t.status_message.clone());
            }
        }
    }

    Ok(())
}

pub(super) fn switch_dispatch_allowed(switch: &SwitchOperation) -> Result<(), String> {
    if !switch.stop_supervision_qualified
        || switch.continuation_method == "blocked"
        || switch.reason.is_some()
    {
        return Err(switch.reason.clone().unwrap_or_else(||"Active Stop→Continue lacks qualified native descendant supervision; target dispatch is blocked.".into()));
    }
    if switch.phase != "prepared" {
        return Err("Switch is not prepared for target dispatch.".into());
    }
    Ok(())
}

impl AgentRuntime {
    pub(crate) fn account_recover(
        &self,
        app: &AppHandle,
        request: AccountRecover,
    ) -> Result<AccountsSnapshot, String> {
        let result = self.with(app, |i| {
            let s = i.store.as_mut().unwrap();
            let result = s.recover_helpers(&request)?;
            for op in s.native_operations()? {
                if op.account_id == request.account_id && op.state == "reviewed" {
                    i.recovery_projects.remove(&op.operation_id);
                    if let Some(task) = op.task_id {
                        if s.processes_settled(&task)? && s.helpers_settled(Some(&task))? {
                            i.recovery_projects.remove(&task);
                        }
                    }
                }
            }
            if !s.unresolved_accounts()?.contains(&request.account_id) {
                i.account_leases.remove(&request.account_id);
                i.finished_account_leases.remove(&request.account_id);
            }
            Ok(result)
        })?;
        self.changed(app, None);
        Ok(result)
    }
}

pub(super) fn account_control_reserved(
    store: &Store,
    leases: &HashSet<String>,
    account: &str,
) -> Result<bool, String> {
    Ok(leases.contains(account) || store.unresolved_accounts()?.iter().any(|id| id == account))
}

#[cfg(test)]
pub(super) fn begin_helper(
    inner: &mut Inner,
    closing: bool,
    account: &AccountInstance,
    cwd: &str,
    purpose: &str,
) -> Result<(), String> {
    begin_helper_impl(inner, closing, account, cwd, purpose, false).map(|_| ())
}
struct AdmittedHelper {
    #[cfg(not(target_os = "macos"))]
    operation: String,
    #[cfg(target_os = "macos")]
    context: Option<super::host_child::Context>,
    #[cfg(target_os = "macos")]
    finished: Option<Arc<AtomicBool>>,
}
fn begin_managed_helper(
    inner: &mut Inner,
    closing: bool,
    account: &AccountInstance,
    cwd: &str,
    purpose: &str,
) -> Result<AdmittedHelper, String> {
    begin_helper_impl(
        inner,
        closing,
        account,
        cwd,
        purpose,
        cfg!(target_os = "macos"),
    )
}
fn begin_helper_impl(
    inner: &mut Inner,
    closing: bool,
    account: &AccountInstance,
    cwd: &str,
    purpose: &str,
    owned: bool,
) -> Result<AdmittedHelper, String> {
    if closing {
        return Err("The app is preparing to close; no native helper was admitted.".into());
    }
    if account_control_reserved(
        inner.store.as_ref().unwrap(),
        &inner.account_leases,
        &account.account_id,
    )? || !account.enabled
        || !super::native_accounts::available(account.cli)
    {
        return Err("The native account namespace is unavailable or reserved.".into());
    }
    let lease = crate::project_write_guard::activate(Path::new(cwd), &account.account_id, 1)?;
    let store = inner.store.as_mut().unwrap();
    #[cfg(target_os = "macos")]
    let (operation, context) = if owned {
        let context = store.owned_helper_intent_context(purpose, account, cwd)?;
        (context.parent_operation_id.clone(), Some(context))
    } else {
        (store.helper_intent(purpose, account, cwd, None)?, None)
    };
    #[cfg(not(target_os = "macos"))]
    let operation = {
        let _ = owned;
        store.helper_intent(purpose, account, cwd, None)?
    };
    #[cfg(target_os = "macos")]
    let finished = if owned && purpose == "account_terminal" {
        let flag = Arc::new(AtomicBool::new(false));
        inner
            .account_terminal_finishes
            .insert(operation.clone(), flag.clone());
        Some(flag)
    } else {
        None
    };
    inner.recovery_projects.insert(operation.clone(), lease);
    inner.account_leases.insert(account.account_id.clone());
    Ok(AdmittedHelper {
        #[cfg(not(target_os = "macos"))]
        operation,
        #[cfg(target_os = "macos")]
        context,
        #[cfg(target_os = "macos")]
        finished,
    })
}
