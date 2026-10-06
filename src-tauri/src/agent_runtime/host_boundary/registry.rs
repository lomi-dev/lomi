use super::{failure, kernel, Member, Scope};
use std::{
    collections::HashMap,
    sync::{Arc, Condvar, Mutex, OnceLock, Weak},
    time::{Duration, Instant},
};

#[derive(Default)]
struct State {
    accepting: bool,
    cancelled: bool,
    active: usize,
}
pub(crate) struct EffectScope {
    state: Mutex<State>,
    settled: Condvar,
    retirement_holds: Mutex<Vec<Arc<dyn Send + Sync>>>,
}
pub(crate) struct EffectGuard {
    scope: Arc<EffectScope>,
}
impl EffectScope {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
            settled: Condvar::new(),
            retirement_holds: Mutex::new(Vec::new()),
        })
    }
    pub(super) fn retain_resource<T: Send + Sync + 'static>(
        &self,
        resource: Arc<T>,
    ) -> Result<(), String> {
        let state = self.state.lock().map_err(|_| failure())?;
        if state.cancelled {
            return Err(failure());
        }
        let mut resources = self.retirement_holds.lock().map_err(|_| failure())?;
        if resources.len() >= 32 {
            return Err(failure());
        }
        resources.push(resource);
        Ok(())
    }
    pub(super) fn release_resources(&self) -> Result<(), String> {
        let resources = {
            let mut holds = self.retirement_holds.lock().map_err(|_| failure())?;
            std::mem::take(&mut *holds)
        };
        // Resource destructors may join host workers or release scope references.
        // Never run them under the holds mutex.
        drop(resources);
        Ok(())
    }
    pub(crate) fn enter(self: &Arc<Self>) -> Result<EffectGuard, String> {
        let mut state = self.state.lock().map_err(|_| failure())?;
        if !state.accepting || state.cancelled {
            return Err(failure());
        }
        state.active = state.active.checked_add(1).ok_or_else(failure)?;
        Ok(EffectGuard {
            scope: self.clone(),
        })
    }
    pub(crate) fn cancelled(&self) -> bool {
        self.state.lock().map(|s| s.cancelled).unwrap_or(true)
    }
    pub(super) fn release(&self) -> Result<(), String> {
        let mut s = self.state.lock().map_err(|_| failure())?;
        if s.cancelled {
            return Err(failure());
        }
        s.accepting = true;
        Ok(())
    }
    pub(crate) fn cancel(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.accepting = false;
            state.cancelled = true;
            self.settled.notify_all();
        }
    }
    pub(super) fn cancel_and_wait(&self, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        let mut s = self.state.lock().map_err(|_| failure())?;
        s.accepting = false;
        s.cancelled = true;
        while s.active != 0 {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(failure)?;
            let (next, result) = self
                .settled
                .wait_timeout(s, remaining)
                .map_err(|_| failure())?;
            s = next;
            if result.timed_out() && s.active != 0 {
                return Err(failure());
            }
        }
        Ok(())
    }
}
impl Drop for EffectGuard {
    fn drop(&mut self) {
        if let Ok(mut s) = self.scope.state.lock() {
            s.active -= 1;
            self.scope.settled.notify_all();
        }
    }
}
struct Registration {
    scope: Scope,
    member: Member,
    boot: String,
    effects: Weak<EffectScope>,
}
fn registry() -> &'static Mutex<HashMap<String, Registration>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Registration>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}
pub(super) fn register(
    scope: &Scope,
    member: &Member,
    effects: &Arc<EffectScope>,
) -> Result<(), String> {
    let registration = Registration {
        scope: scope.clone(),
        member: member.clone(),
        boot: kernel::HostWitness::admitted()?.boot,
        effects: Arc::downgrade(effects),
    };
    let mut registry = registry().lock().map_err(|_| failure())?;
    registry.retain(|_, r| r.effects.strong_count() != 0);
    registry.insert(scope.operation_id.clone(), registration);
    Ok(())
}
pub(crate) fn registered_scope(pid: u32) -> Result<Option<(Scope, Arc<EffectScope>)>, String> {
    let host = kernel::HostWitness::admitted()?;
    let member = kernel::member(pid)?;
    let registrations = registry().lock().map_err(|_| failure())?;
    for r in registrations.values() {
        if r.boot == host.boot && r.member.resource_coalition == member.resource_coalition {
            if let Some(effects) = r.effects.upgrade() {
                let s = effects.state.lock().map_err(|_| failure())?;
                if s.accepting
                    && !s.cancelled
                    && kernel::usage(member.resource_coalition)? == kernel::Usage::Alive
                {
                    drop(s);
                    return Ok(Some((r.scope.clone(), effects)));
                }
            }
        }
    }
    Ok(None)
}

pub(super) fn retained_effects(scope: &Scope) -> Result<Option<Arc<EffectScope>>, String> {
    let registrations = registry().lock().map_err(|_| failure())?;
    if let Some(r) = registrations.get(&scope.operation_id) {
        if serde_json::to_vec(&r.scope).map_err(|_| failure())?
            != serde_json::to_vec(scope).map_err(|_| failure())?
        {
            return Err(failure());
        }
        return Ok(r.effects.upgrade());
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retirement_resources_survive_cancellation_and_drop_outside_the_holds_lock() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Resource {
            scope: Weak<EffectScope>,
            dropped: Arc<AtomicUsize>,
        }
        impl Drop for Resource {
            fn drop(&mut self) {
                let scope = self.scope.upgrade().unwrap();
                let _lock = scope
                    .retirement_holds
                    .try_lock()
                    .expect("resource dropped under holds lock");
                self.dropped.fetch_add(1, Ordering::SeqCst);
            }
        }
        let scope = EffectScope::new();
        let dropped = Arc::new(AtomicUsize::new(0));
        scope
            .retain_resource(Arc::new(Resource {
                scope: Arc::downgrade(&scope),
                dropped: dropped.clone(),
            }))
            .unwrap();
        scope.cancel();
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
        scope.release_resources().unwrap();
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn cancellation_closes_admission_and_retains_active_effects() {
        let scope = EffectScope::new();
        assert!(scope.enter().is_err());
        scope.release().unwrap();
        let guard = scope.enter().unwrap();
        assert!(scope.cancel_and_wait(Duration::from_millis(1)).is_err());
        assert!(scope.cancelled());
        assert!(scope.enter().is_err());
        assert!(scope.release().is_err());
        drop(guard);
        scope.cancel_and_wait(Duration::from_millis(1)).unwrap();
    }
}
