//! Production driver for the exact host boundary, with no direct-spawn fallback.
use super::{
    host_boundary::{Boundary, NativeSpec, Purpose, RetiredReceipt, Scope},
    process_identity::Identity,
};
use std::{
    fs::File,
    path::PathBuf,
    process::ExitStatus,
    sync::atomic::{AtomicBool, Ordering},
};

#[cfg(test)]
use super::host_boundary::EffectScope;
#[cfg(test)]
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(crate) struct Context {
    pub parent_operation_id: String,
    pub account_id: String,
    pub auth_revision: u64,
    pub task_id: Option<String>,
    pub attempt_id: Option<String>,
    pub generation: Option<u64>,
    pub project_root: PathBuf,
    pub physical_account_root: PathBuf,
    pub storage_root: PathBuf,
}
impl Context {
    #[cfg(test)]
    pub(crate) fn spawn(&self, purpose: Purpose, spec: NativeSpec) -> Result<HostChild, String> {
        self.spawn_checked(purpose, spec, &AtomicBool::new(false), || Ok(()))
    }
    #[cfg(test)]
    pub(crate) fn spawn_checked(
        &self,
        purpose: Purpose,
        spec: NativeSpec,
        cancel: &AtomicBool,
        before_release: impl FnOnce() -> Result<(), String>,
    ) -> Result<HostChild, String> {
        self.spawn_with_boundary(purpose, spec, cancel, |_| before_release())
    }
    pub(crate) fn spawn_with_boundary(
        &self,
        purpose: Purpose,
        spec: NativeSpec,
        cancel: &AtomicBool,
        before_release: impl FnOnce(&Boundary) -> Result<(), String>,
    ) -> Result<HostChild, String> {
        if cancel.load(Ordering::SeqCst) {
            return Err("The native client is stopping.".into());
        }
        if self.parent_operation_id.is_empty()
            || self.parent_operation_id.len() > 200
            || self.parent_operation_id.chars().any(char::is_control)
            || spec.policy.project_root != self.project_root
            || spec.policy.account_root != self.physical_account_root
        {
            return Err("The native boundary context does not match its immutable policy.".into());
        }
        for path in [
            &self.project_root,
            &self.physical_account_root,
            &self.storage_root,
        ] {
            if path
                .canonicalize()
                .map_err(|_| "Cannot verify native context roots.")?
                != *path
            {
                return Err("Native context roots must be canonical.".into());
            }
        }
        let base =
            super::host_boundary::prepare_parent(&self.storage_root, &self.parent_operation_id)?;
        let scope = Scope {
            operation_id: super::new_id()?,
            parent_operation_id: Some(self.parent_operation_id.clone()),
            physical_account_root: Some(self.physical_account_root.clone()),
            storage_root: Some(self.storage_root.clone()),
            account_id: self.account_id.clone(),
            auth_revision: self.auth_revision,
            task_id: self.task_id.clone(),
            attempt_id: self.attempt_id.clone(),
            generation: self.generation,
            project_root: self.project_root.clone(),
            purpose,
        };
        let mut boundary = Boundary::create(&base, scope, spec)?;
        let preparation = (|| {
            if cancel.load(Ordering::SeqCst) {
                return Err("The native client is stopping.".into());
            }
            boundary.release_with_gate(|boundary| {
                if cancel.load(Ordering::SeqCst) {
                    return Err("The native client is stopping.".into());
                }
                before_release(boundary)?;
                if cancel.load(Ordering::SeqCst) {
                    return Err("The native client is stopping.".into());
                }
                Ok(())
            })
        })();
        if let Err(error) = preparation {
            // The durable journal remains even when cleanup cannot prove retirement.
            let _ = boundary.stop();
            return Err(error);
        }
        let identity = boundary.identity()?.clone();
        let transport = boundary.take_transport()?;
        Ok(HostChild {
            stdin: Some(transport.stdin),
            stdout: Some(transport.stdout),
            stderr: Some(transport.stderr),
            boundary,
            identity,
            status: None,
            retired: false,
        })
    }
}
pub(crate) struct HostChild {
    pub stdin: Option<File>,
    pub stdout: Option<File>,
    pub stderr: Option<File>,
    boundary: Boundary,
    identity: Identity,
    status: Option<ExitStatus>,
    retired: bool,
}
impl HostChild {
    pub(crate) fn id(&self) -> u32 {
        self.identity.pid
    }
    #[cfg(test)]
    pub(crate) fn effect_scope(&self) -> Arc<EffectScope> {
        self.boundary.effects()
    }
    pub(crate) fn try_wait(&mut self) -> Result<Option<ExitStatus>, String> {
        if self.status.is_none() {
            self.status = self.boundary.try_wait()?;
        }
        Ok(self.status)
    }
    pub(crate) fn exit_pending(&mut self) -> Result<bool, String> {
        Ok(self.try_wait()?.is_some())
    }
    pub(crate) fn signal(&self, signal: i32) -> Result<(), String> {
        self.boundary.signal(signal)
    }
    pub(crate) fn resize(&self, rows: u16, cols: u16) -> Result<(), String> {
        self.boundary.resize(rows, cols)
    }
    pub(crate) fn close_stdin(&mut self) {
        self.stdin.take();
    }
    pub(crate) fn stop_and_wait(&mut self) -> Result<RetiredReceipt, String> {
        self.stdin.take();
        let receipt = self.boundary.stop()?;
        self.retired = true;
        Ok(receipt)
    }
}
impl Drop for HostChild {
    fn drop(&mut self) {
        if !self.retired {
            let _ = self.stop_and_wait();
        }
    }
}
