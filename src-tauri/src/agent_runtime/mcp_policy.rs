//! Native task MCP uses fresh audited cohort membership and durable owner fences.
//! Peers without captured membership retain the global ownership ceiling. Never
//! infer provenance from client labels, request fields, ancestry or environment.
use super::{process_identity::Identity, service::AgentRuntime};
use lomi_control_core::broker::{
    SessionEffectPermit, SessionRequestAdmission, SessionRequestPolicy,
};
use lomi_control_protocol::{control::Request, ErrorCode};
use std::{
    path::PathBuf,
    sync::{atomic::Ordering, Arc},
};
use tauri::AppHandle;

pub(super) type WorkspaceResolver = Arc<dyn Fn(&str) -> Result<PathBuf, ErrorCode> + Send + Sync>;
#[cfg(target_os = "macos")]
struct ManagedPeer {
    scope: super::host_boundary::Scope,
    effects: Arc<super::host_boundary::EffectScope>,
    project_identity: (u64, u64),
}
struct NativeOwnershipPolicy {
    runtime: AgentRuntime,
    peer: Identity,
    #[cfg(target_os = "macos")]
    resolver: Option<WorkspaceResolver>,
    #[cfg(target_os = "macos")]
    managed: Option<ManagedPeer>,
}

fn observation_only(request: &Request) -> bool {
    matches!(request, Request::Status(_) | Request::Connect(_))
}

impl NativeOwnershipPolicy {
    fn peer_matches(&self) -> Result<(), ErrorCode> {
        match self.peer.still_matches() {
            Ok(true) => Ok(()),
            _ => Err(ErrorCode::ControlRevoked),
        }
    }

    #[cfg(target_os = "macos")]
    fn managed_scope(&self) -> Result<Option<&ManagedPeer>, ErrorCode> {
        let Some(captured) = &self.managed else {
            return Ok(None);
        };
        let (scope, effects) = super::host_boundary::registered_scope(self.peer.pid)
            .map_err(|_| ErrorCode::ControlRevoked)?
            .ok_or(ErrorCode::ControlRevoked)?;
        if serde_json::to_vec(&scope).map_err(|_| ErrorCode::ControlRevoked)?
            != serde_json::to_vec(&captured.scope).map_err(|_| ErrorCode::ControlRevoked)?
            || !Arc::ptr_eq(&effects, &captured.effects)
            || effects.cancelled()
        {
            return Err(ErrorCode::ControlRevoked);
        }
        Ok(Some(captured))
    }
    #[cfg(target_os = "macos")]
    fn check_managed(
        &self,
        inner: &super::service::Inner,
        request: &Request,
        captured: &ManagedPeer,
    ) -> Result<(), ErrorCode> {
        use super::{host_boundary::Purpose, types::TaskState};
        let scope = &captured.scope;
        if scope.purpose != Purpose::Attempt || self.runtime.closing.load(Ordering::SeqCst) {
            return Err(ErrorCode::ControlRevoked);
        }
        let task_id = scope.task_id.as_ref().ok_or(ErrorCode::ScopeDenied)?;
        let attempt_id = scope.attempt_id.as_ref().ok_or(ErrorCode::ScopeDenied)?;
        let generation = scope.generation.ok_or(ErrorCode::ScopeDenied)?;
        let active = inner.active.get(task_id).ok_or(ErrorCode::ControlRevoked)?;
        if active.attempt != *attempt_id
            || active.account != scope.account_id
            || active.generation != generation
            || active.stop.load(Ordering::SeqCst)
            || active.abort.load(Ordering::SeqCst)
            || active.done.load(Ordering::SeqCst)
            || inner.closing_tasks.contains(task_id)
            || inner.pending_finishes.contains_key(task_id)
        {
            return Err(ErrorCode::ControlRevoked);
        }
        let store = inner.store.as_ref().ok_or(ErrorCode::AppUnavailable)?;
        let account = store
            .account(&scope.account_id)
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        let binding = store
            .binding(&scope.account_id)
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        if binding.account_id != scope.account_id
            || binding.auth_revision != scope.auth_revision
            || scope
                .physical_account_root
                .as_ref()
                .is_none_or(|root| root != &PathBuf::from(&binding.physical_root))
        {
            return Err(ErrorCode::ControlRevoked);
        }
        let task = store
            .task(task_id)
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        if !account.enabled
            || account.recovery.is_some()
            || account.auth_revision != scope.auth_revision
            || task.generation != generation
            || task.active_attempt_id.as_ref() != Some(attempt_id)
            || task.active_account_id.as_ref() != Some(&scope.account_id)
            || !matches!(task.state, TaskState::Starting | TaskState::Running)
        {
            return Err(ErrorCode::ControlRevoked);
        }
        let attempt = task
            .attempts
            .iter()
            .find(|a| a.attempt_id == *attempt_id)
            .ok_or(ErrorCode::ControlRevoked)?;
        if attempt.account_id != scope.account_id
            || attempt.auth_revision != scope.auth_revision
            || attempt.generation != generation
            || scope.parent_operation_id.as_deref() != Some(attempt.operation_id.as_str())
            || !matches!(attempt.state.as_str(), "spawn_intent" | "dispatched")
        {
            return Err(ErrorCode::ControlRevoked);
        }
        if store
            .directory_identity(task_id)
            .map_err(|_| ErrorCode::StorageUnavailable)?
            != captured.project_identity
        {
            return Err(ErrorCode::ScopeDenied);
        }
        let (physical, identity) =
            super::store::workspace(&scope.project_root).map_err(|_| ErrorCode::ScopeDenied)?;
        if std::path::Path::new(&task.cwd) != scope.project_root
            || std::path::Path::new(&physical) != scope.project_root
            || identity != captured.project_identity
        {
            return Err(ErrorCode::ScopeDenied);
        }
        if let Some(alias) = super::managed_mcp::observation_workspace(request)? {
            let resolver = self.resolver.as_ref().ok_or(ErrorCode::ScopeDenied)?;
            let root = resolver(alias)?;
            if root != scope.project_root {
                return Err(ErrorCode::ScopeDenied);
            }
        }
        Ok(())
    }
    fn check_owner(
        &self,
        inner: &super::service::Inner,
        request: &Request,
    ) -> Result<(), ErrorCode> {
        #[cfg(target_os = "macos")]
        if let Some(captured) = self.managed_scope()? {
            return self.check_managed(inner, request, captured);
        }
        if self.runtime.closing.load(Ordering::SeqCst) {
            return Err(ErrorCode::ControlRevoked);
        }
        let store = inner.store.as_ref().ok_or(ErrorCode::AppUnavailable)?;
        if observation_only(request) {
            return Ok(());
        }
        if !inner.active.is_empty()
            || !inner.pending_finishes.is_empty()
            || !store
                .unresolved_accounts()
                .map_err(|_| ErrorCode::StorageUnavailable)?
                .is_empty()
        {
            return Err(ErrorCode::ScopeDenied);
        }
        Ok(())
    }
}

impl SessionRequestPolicy for NativeOwnershipPolicy {
    fn check(&self, request: &Request) -> Result<(), ErrorCode> {
        self.peer_matches()?;
        let inner = self
            .runtime
            .inner
            .lock()
            .map_err(|_| ErrorCode::AppUnavailable)?;
        self.check_owner(&inner, request)
    }

    fn admit_effect(
        &self,
        request: &Request,
    ) -> Result<Option<Arc<dyn SessionEffectPermit>>, ErrorCode> {
        self.peer_matches()?;
        let inner = self
            .runtime
            .inner
            .lock()
            .map_err(|_| ErrorCode::AppUnavailable)?;
        self.check_owner(&inner, request)?;
        #[cfg(target_os = "macos")]
        if let Some(captured) = self.managed_scope()? {
            let effect = captured
                .effects
                .enter()
                .map_err(|_| ErrorCode::ControlRevoked)?;
            // Retain native effect admission even for observations: Stop waits
            // for descriptor reads/Git workers before retiring this cohort.
            return Ok(Some(Arc::new(effect)));
        }
        if observation_only(request) {
            return Ok(None);
        }
        // Native preparation reserves its project lease under this same owner
        // lock. An unscoped permit prevents that reservation while an accepted
        // MCP effect, including a deferred native commit, remains outstanding.
        let guard =
            crate::project_write_guard::admit_unscoped().map_err(|_| ErrorCode::TargetBusy)?;
        Ok(Some(Arc::new(guard)))
    }
}

pub(super) fn capture_with_workspace_resolver(
    runtime: &AgentRuntime,
    peer_pid: Option<u32>,
    resolver: Option<WorkspaceResolver>,
) -> Result<Arc<dyn SessionRequestPolicy>, ErrorCode> {
    #[cfg(not(target_os = "macos"))]
    let _ = &resolver;
    let peer = Identity::read(peer_pid.ok_or(ErrorCode::ScopeDenied)?)
        .map_err(|_| ErrorCode::ScopeDenied)?;
    #[cfg(target_os = "macos")]
    let managed = match super::host_boundary::registered_scope(peer.pid) {
        Ok(Some((scope, effects))) => {
            let (_, project_identity) =
                super::store::workspace(&scope.project_root).map_err(|_| ErrorCode::ScopeDenied)?;
            Some(ManagedPeer {
                scope,
                effects,
                project_identity,
            })
        }
        // Unresolved provenance retains the global ownership ceiling. It never
        // falls through to task observations or a client-selected tool mode.
        Ok(None) | Err(_) => None,
    };
    Ok(Arc::new(NativeOwnershipPolicy {
        runtime: runtime.clone(),
        peer,
        #[cfg(target_os = "macos")]
        resolver,
        #[cfg(target_os = "macos")]
        managed,
    }))
}

impl AgentRuntime {
    pub(crate) fn mcp_request_admission_with_resolver(
        &self,
        app: &AppHandle,
        resolver: Option<WorkspaceResolver>,
    ) -> Result<SessionRequestAdmission, String> {
        // Restore durable ownership and project fences before opening the
        // listener. A failed owner initialization must not expose an ordinary
        // admission path to native peers with unresolved provenance.
        self.with(app, |_| Ok(()))?;
        let runtime = self.clone();
        Ok(Arc::new(move |peer_pid| {
            capture_with_workspace_resolver(&runtime, peer_pid, resolver.clone()).map(Some)
        }))
    }
}
