use super::*;

/// A server-owned restriction captured for one authenticated connection.
/// Implementations must revalidate their captured process/attempt provenance on
/// every check and fail closed if that provenance can no longer be established.
/// This restriction supplements grants and never bypasses their authorization.
pub trait SessionRequestPolicy: Send + Sync {
    fn check(&self, request: &Request) -> Result<(), ErrorCode>;

    /// Hold host effect admission until all synchronous and deferred work ends.
    /// The host must serialize acquisition with native ownership transitions.
    fn admit_effect(
        &self,
        _request: &Request,
    ) -> Result<Option<Arc<dyn SessionEffectPermit>>, ErrorCode> {
        Ok(None)
    }
}

/// Host-owned effect admission retained through the actual effect completion.
pub trait SessionEffectPermit: Send + Sync {}
impl<T: Send + Sync> SessionEffectPermit for T {}

#[derive(Clone)]
pub(super) struct CapturedRequest {
    policy: Option<Arc<dyn SessionRequestPolicy>>,
    request: Request,
    connected: Arc<AtomicBool>,
    _effect: Option<Arc<dyn SessionEffectPermit>>,
}
impl CapturedRequest {
    pub(super) fn check(&self) -> Result<(), ErrorCode> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err(ErrorCode::ControlRevoked);
        }
        if let Some(policy) = &self.policy {
            policy.check(&self.request)?;
        }
        Ok(())
    }
}

thread_local! {
    // Request handlers are synchronous. Deferred work explicitly clones this
    // context before leaving the handler, never reading TLS from an async task.
    static CURRENT_REQUEST: std::cell::RefCell<Option<CapturedRequest>> = const { std::cell::RefCell::new(None) };
}
pub(super) fn current_request() -> Option<CapturedRequest> {
    CURRENT_REQUEST.with(|current| current.borrow().clone())
}
pub(super) struct RequestScope(Option<CapturedRequest>);
impl RequestScope {
    pub(super) fn enter(request: Option<CapturedRequest>) -> Self {
        Self(CURRENT_REQUEST.with(|current| current.replace(request)))
    }
}
impl Drop for RequestScope {
    fn drop(&mut self) {
        CURRENT_REQUEST.with(|current| current.replace(self.0.take()));
    }
}

/// Enrollment receives only the OS-observed peer PID, never a client label or
/// claimed request identity. `None` means ordinary admission only when returned
/// successfully at enrollment; errors cannot discard an enrolled restriction.
pub type SessionRequestAdmission = Arc<
    dyn Fn(Option<u32>) -> Result<Option<Arc<dyn SessionRequestPolicy>>, ErrorCode> + Send + Sync,
>;

impl Broker {
    pub(super) fn capture_request(
        &self,
        id: &str,
        request: &Request,
    ) -> Result<CapturedRequest, ErrorCode> {
        let (policy, connected) = {
            let state = self.lock_state().map_err(|_| ErrorCode::AppUnavailable)?;
            let session = state
                .sessions
                .get(id)
                .filter(|s| s.alive.load(Ordering::SeqCst))
                .ok_or(ErrorCode::ControlRevoked)?;
            (session.request_policy.clone(), session.alive.clone())
        };
        let mut captured = CapturedRequest {
            policy,
            connected,
            request: request.clone(),
            _effect: None,
        };
        captured.check()?;
        if let Some(policy) = &captured.policy {
            captured._effect = policy.admit_effect(request)?;
        }
        captured.check()?;
        Ok(captured)
    }

    /// Retain this permit across a native UI effect that outlives its ticket
    /// authorization call, including while revocation removes queued UI work.
    pub fn operation_effect_permit(&self, operation: &str) -> Result<NativePermit, ErrorCode> {
        self.check_work_request(operation)
    }

    pub(super) fn check_work_request(&self, operation: &str) -> Result<NativePermit, ErrorCode> {
        let permit = {
            let state = self.lock_state().map_err(|_| ErrorCode::AppUnavailable)?;
            state
                .work
                .get(operation)
                .ok_or(ErrorCode::TargetNotFound)?
                .native_permit
                .clone()
        };
        permit.revalidate()?;
        Ok(permit)
    }

    pub(super) fn check_session_request(
        &self,
        id: &str,
        request: &Request,
    ) -> Result<(), ErrorCode> {
        if self.session_request_admission.is_none() {
            return Ok(());
        }
        let policy = {
            let state = self.lock_state().map_err(|_| ErrorCode::AppUnavailable)?;
            let session = state
                .sessions
                .get(id)
                .filter(|session| session.alive.load(Ordering::SeqCst))
                .ok_or(ErrorCode::ControlRevoked)?;
            session.request_policy.clone()
        };
        // Process provenance checks may consult native state. Never hold the
        // broker policy mutex while calling host code.
        if let Some(policy) = policy {
            policy.check(request)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
