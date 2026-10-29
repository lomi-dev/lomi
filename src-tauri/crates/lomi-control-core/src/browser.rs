//! Native browser authority is independent of receipt and UI locks. Profiles
//! remain isolated after a human takes over; takeover never restores an agent lease.
use crate::broker::new_id;
use lomi_control_protocol::browser::{address, Origin};
use lomi_control_protocol::{control::BrowserWaitUntil, ErrorCode};
use std::{
    io,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};

pub struct BrowserControl {
    pub generation: String,
    pub profile_id: String,
    pub panel_id: String,
    lease: String,
    origins: Vec<Origin>,
    allow_any_http_origin: bool,
    epoch: Arc<AtomicU64>,
    expected_epoch: u64,
    connection: Arc<AtomicBool>,
    active: AtomicBool,
    human: AtomicBool,
    started: AtomicBool,
    selected_tab: AtomicBool,
    navigation: AtomicU64,
    loading: AtomicBool,
    pending: Mutex<Option<Navigation>>,
    dom_busy: AtomicBool,
    native_dom_busy: AtomicBool,
    snapshot: Mutex<Option<(String, String)>>,
    document_url: Mutex<Option<String>>,
    interactions: Mutex<std::collections::VecDeque<(String, InteractionObservation)>>,
}

type InteractionObservation =
    Result<lomi_control_protocol::control::BrowserInteractionResult, ErrorCode>;

pub struct NativeDomGuard(Arc<BrowserControl>);
impl Drop for NativeDomGuard {
    fn drop(&mut self) {
        self.0.native_dom_busy.store(false, Ordering::SeqCst);
    }
}
pub struct DomGuard(Arc<BrowserControl>);
impl Drop for DomGuard {
    fn drop(&mut self) {
        self.0.dom_busy.store(false, Ordering::SeqCst);
    }
}

struct Navigation {
    operation: String,
    expected: String,
    requested: bool,
    url: String,
    committed: bool,
    loaded: bool,
    error: Option<ErrorCode>,
}
#[derive(Clone)]
pub struct NavigationObservation {
    pub navigation_id: String,
    pub url: String,
    pub committed: bool,
    pub loaded: bool,
}

impl BrowserControl {
    #[allow(
        clippy::too_many_arguments,
        reason = "Keep isolated authority and revocation inputs explicit at the construction boundary."
    )]
    pub fn new(
        generation: String,
        panel_id: String,
        profile_id: String,
        origins: Vec<Origin>,
        allow_any_http_origin: bool,
        epoch: Arc<AtomicU64>,
        expected_epoch: u64,
        connection: Arc<AtomicBool>,
    ) -> io::Result<Self> {
        if (!allow_any_http_origin && origins.is_empty())
            || origins.len() > 16
            || profile_id.len() != 32
            || !profile_id.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(io::Error::other("Invalid isolated browser authority"));
        }
        Ok(Self {
            generation,
            panel_id,
            profile_id,
            lease: new_id()?,
            origins,
            allow_any_http_origin,
            epoch,
            expected_epoch,
            connection,
            active: AtomicBool::new(true),
            human: AtomicBool::new(false),
            started: AtomicBool::new(false),
            selected_tab: AtomicBool::new(false),
            navigation: AtomicU64::new(0),
            loading: AtomicBool::new(true),
            pending: Mutex::new(None),
            dom_busy: AtomicBool::new(false),
            native_dom_busy: AtomicBool::new(false),
            snapshot: Mutex::new(None),
            document_url: Mutex::new(None),
            interactions: Mutex::new(std::collections::VecDeque::new()),
        })
    }
    pub fn authorized(&self) -> bool {
        self.active.load(Ordering::SeqCst)
            && self.connection.load(Ordering::SeqCst)
            && self.epoch.load(Ordering::SeqCst) == self.expected_epoch
    }
    pub(crate) fn set_selected_tab(&self, selected: bool) {
        self.selected_tab.store(selected, Ordering::SeqCst);
    }
    pub fn require_renderable(&self) -> Result<(), ErrorCode> {
        if !self.authorized() {
            return Err(ErrorCode::ControlRevoked);
        }
        if !self.selected_tab.load(Ordering::SeqCst) {
            return Err(ErrorCode::PanelNotRenderable);
        }
        Ok(())
    }
    pub fn record_interaction(
        &self,
        operation: &str,
        result: lomi_control_protocol::control::BrowserInteractionResult,
    ) -> Result<(), ErrorCode> {
        if !self.authorized() {
            return Err(ErrorCode::ControlRevoked);
        }
        let mut results = self
            .interactions
            .lock()
            .map_err(|_| ErrorCode::AppUnavailable)?;
        if results.len() == 32 {
            results.pop_front();
        }
        results.push_back((operation.into(), Ok(result)));
        Ok(())
    }
    pub fn record_interaction_rejection(
        &self,
        operation: &str,
        code: ErrorCode,
    ) -> Result<(), ErrorCode> {
        if !self.authorized() {
            return Err(ErrorCode::ControlRevoked);
        }
        let mut results = self
            .interactions
            .lock()
            .map_err(|_| ErrorCode::AppUnavailable)?;
        if results.len() == 32 {
            results.pop_front();
        }
        results.push_back((operation.into(), Err(code)));
        Ok(())
    }
    pub fn interaction_rejection(&self, operation: &str) -> Option<ErrorCode> {
        self.interactions
            .lock()
            .ok()?
            .iter()
            .find(|(op, _)| op == operation)?
            .1
            .as_ref()
            .err()
            .copied()
    }
    pub fn interaction_result(
        &self,
        operation: &str,
    ) -> Result<lomi_control_protocol::control::BrowserInteractionResult, ErrorCode> {
        if !self.authorized() {
            return Err(ErrorCode::ControlRevoked);
        }
        self.interactions
            .lock()
            .map_err(|_| ErrorCode::AppUnavailable)?
            .iter()
            .find(|(op, _)| op == operation)
            .map(|(_, r)| r.clone())
            .unwrap_or(Err(ErrorCode::OutcomeUnknown))
    }
    pub fn begin_native_dom(self: &Arc<Self>) -> Result<NativeDomGuard, ErrorCode> {
        if !self.authorized() {
            return Err(ErrorCode::ControlRevoked);
        }
        self.native_dom_busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| ErrorCode::TargetBusy)?;
        Ok(NativeDomGuard(self.clone()))
    }
    pub fn begin_dom(self: &Arc<Self>) -> Result<DomGuard, ErrorCode> {
        if !self.authorized() {
            return Err(ErrorCode::ControlRevoked);
        }
        if self.loading.load(Ordering::SeqCst) {
            return Err(ErrorCode::TargetBusy);
        }
        self.dom_busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| ErrorCode::TargetBusy)?;
        Ok(DomGuard(self.clone()))
    }
    pub fn retain_snapshot(&self, snapshot: &str, navigation: &str) -> Result<(), ErrorCode> {
        self.check_document(navigation)?;
        *self
            .snapshot
            .lock()
            .map_err(|_| ErrorCode::AppUnavailable)? = Some((snapshot.into(), navigation.into()));
        Ok(())
    }
    pub fn check_document(&self, navigation: &str) -> Result<(), ErrorCode> {
        if !self.authorized() {
            return Err(ErrorCode::ControlRevoked);
        }
        if self.loading.load(Ordering::SeqCst) || self.navigation_id() != navigation {
            return Err(ErrorCode::StaleSnapshot);
        }
        Ok(())
    }
    pub fn check_snapshot(&self, snapshot: &str, navigation: &str) -> Result<(), ErrorCode> {
        self.check_document(navigation)?;
        if self
            .snapshot
            .lock()
            .map_err(|_| ErrorCode::AppUnavailable)?
            .as_ref()
            .is_none_or(|s| s.0 != snapshot || s.1 != navigation)
        {
            return Err(ErrorCode::StaleSnapshot);
        }
        Ok(())
    }
    pub fn lease(&self) -> Option<&str> {
        self.authorized().then_some(&self.lease)
    }
    pub fn revoke(&self) {
        self.active.store(false, Ordering::SeqCst);
    }
    /// Only trusted native UI may permit human navigation beyond the grant.
    pub fn take_over(&self) {
        self.revoke();
        self.human.store(true, Ordering::SeqCst);
    }
    pub fn permits(&self, value: &str) -> bool {
        let Ok(url) = address(value) else {
            return false;
        };
        self.authorized()
            && (self.allow_any_http_origin
                || self.origins.iter().any(|origin| origin.permits(&url)))
    }
    pub fn native_navigation(&self, value: &str) -> bool {
        if !(self.permits(value) || self.human.load(Ordering::SeqCst) && address(value).is_ok()) {
            if let Ok(mut pending) = self.pending.lock() {
                if let Some(pending) = pending.as_mut().filter(|p| !p.loaded) {
                    pending.error = Some(ErrorCode::ScopeDenied);
                    self.loading.store(false, Ordering::SeqCst);
                }
            }
            return false;
        }
        self.navigation.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut pending) = self.pending.lock() {
            if let Some(pending) = pending
                .as_mut()
                .filter(|p| !p.committed && p.error.is_none())
            {
                if pending.requested || pending.expected == value {
                    pending.expected = value.into();
                    pending.requested = true;
                }
            }
        }
        true
    }
    pub fn prepare_navigation(&self, operation: &str, url: &str) -> Result<(), ErrorCode> {
        if !self.permits(url) {
            return Err(ErrorCode::ControlRevoked);
        }
        if self.loading.load(Ordering::SeqCst) {
            return Err(ErrorCode::TargetBusy);
        }
        let mut pending = self.pending.try_lock().map_err(|_| ErrorCode::TargetBusy)?;
        if pending
            .as_ref()
            .is_some_and(|p| !p.loaded && p.error.is_none())
        {
            return Err(ErrorCode::TargetBusy);
        }
        *pending = Some(Navigation {
            operation: operation.into(),
            expected: url.into(),
            requested: false,
            url: url.into(),
            committed: false,
            loaded: false,
            error: None,
        });
        self.loading.store(true, Ordering::SeqCst);
        Ok(())
    }
    /// The application reads this URL from the native registered child; a page
    /// signal is only a hint to refresh it, never the source of the URL.
    pub fn document_url(&self) -> Result<String, ErrorCode> {
        if !self.authorized() {
            return Err(ErrorCode::ControlRevoked);
        }
        self.document_url
            .lock()
            .map_err(|_| ErrorCode::AppUnavailable)?
            .clone()
            .ok_or(ErrorCode::TargetBusy)
    }
    pub fn document_changed(&self, url: &str) {
        if !self.permits(url) {
            return;
        }
        if let Ok(mut current) = self.document_url.lock() {
            if current.as_ref().is_some_and(|old| old != url) {
                self.navigation.fetch_add(1, Ordering::SeqCst);
            }
            *current = Some(url.into());
        }
    }
    pub fn document_committed(&self, url: &str) {
        if !self.permits(url) {
            return;
        }
        if let Ok(mut pending) = self.pending.lock() {
            if let Some(pending) = pending
                .as_mut()
                .filter(|p| p.requested && p.expected == url && p.error.is_none())
            {
                // A commit is supplied only by the registered native child delegate.
                pending.url = url.into();
                pending.committed = true;
            }
        }
    }
    pub fn document_loaded(&self, url: &str) {
        if !self.permits(url) {
            return;
        }
        self.document_changed(url);
        self.loading.store(false, Ordering::SeqCst);
        if let Ok(mut pending) = self.pending.lock() {
            if let Some(pending) = pending
                .as_mut()
                .filter(|p| p.committed && p.url == url && p.error.is_none())
            {
                pending.loaded = true;
            }
        }
    }
    pub fn navigation_observation(
        &self,
        operation: &str,
        wait: BrowserWaitUntil,
    ) -> Result<Option<NavigationObservation>, ErrorCode> {
        if !self.authorized() {
            return Err(ErrorCode::ControlRevoked);
        }
        let pending = self.pending.lock().map_err(|_| ErrorCode::AppUnavailable)?;
        let pending = pending
            .as_ref()
            .filter(|p| p.operation == operation)
            .ok_or(ErrorCode::TargetNotFound)?;
        if let Some(error) = pending.error {
            return Err(error);
        }
        let complete = pending.committed && (wait == BrowserWaitUntil::Commit || pending.loaded);
        Ok(complete.then(|| NavigationObservation {
            navigation_id: self.navigation_id(),
            url: pending.url.clone(),
            committed: pending.committed,
            loaded: pending.loaded,
        }))
    }
    pub fn fail_navigation(&self, operation: &str, error: ErrorCode) {
        if let Ok(mut pending) = self.pending.lock() {
            if let Some(pending) = pending.as_mut().filter(|p| p.operation == operation) {
                pending.error = Some(error);
                self.loading.store(false, Ordering::SeqCst);
            }
        }
    }
    pub fn may_stop_navigation(&self, operation: &str) -> bool {
        !self.human.load(Ordering::SeqCst)
            && self.pending.lock().ok().is_some_and(|pending| {
                pending
                    .as_ref()
                    .is_some_and(|p| p.operation == operation && !p.loaded)
            })
    }
    pub fn navigation_id(&self) -> String {
        format!(
            "{}:{}",
            self.generation,
            self.navigation.load(Ordering::SeqCst)
        )
    }
    pub fn started(&self) -> bool {
        self.started.load(Ordering::SeqCst)
    }
    pub fn mark_started(&self) {
        self.started.store(true, Ordering::SeqCst);
    }
    pub fn profile_identifier(&self) -> [u8; 16] {
        let mut bytes = [0; 16];
        for (byte, digits) in bytes
            .iter_mut()
            .zip(self.profile_id.as_bytes().as_chunks::<2>().0)
        {
            *byte = u8::from_str_radix(std::str::from_utf8(digits).unwrap(), 16).unwrap();
        }
        bytes
    }
}
