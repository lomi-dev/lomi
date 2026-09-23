//! Device authority is shared by every view of one managed Android generation.
use lomi_control_protocol::ErrorCode;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};

pub struct AndroidControl {
    pub device: String,
    policy: Arc<AtomicU64>,
    revision: u64,
    connected: Arc<AtomicBool>,
    active: AtomicBool,
    observing: AtomicBool,
    generation: Mutex<Option<String>>,
    selected: Mutex<Option<String>>,
    input: Mutex<Option<Arc<crate::android_input::InputLease>>>,
}
impl AndroidControl {
    pub fn new(
        device: String,
        policy: Arc<AtomicU64>,
        revision: u64,
        connected: Arc<AtomicBool>,
    ) -> Self {
        Self {
            device,
            policy,
            revision,
            connected,
            active: AtomicBool::new(true),
            observing: AtomicBool::new(false),
            generation: Mutex::new(None),
            selected: Mutex::new(None),
            input: Mutex::new(None),
        }
    }
    pub fn check(&self) -> Result<(), ErrorCode> {
        if !self.active.load(Ordering::SeqCst)
            || !self.connected.load(Ordering::SeqCst)
            || self.policy.load(Ordering::SeqCst) != self.revision
        {
            Err(ErrorCode::ControlRevoked)
        } else {
            Ok(())
        }
    }
    pub fn begin_observation(self: &Arc<Self>) -> Result<Observation, ErrorCode> {
        self.check()?;
        if self.observing.swap(true, Ordering::SeqCst) {
            return Err(ErrorCode::TargetBusy);
        }
        Ok(Observation(self.clone()))
    }
    pub fn revoke(&self) {
        self.active.store(false, Ordering::SeqCst);
        self.revoke_input();
    }
    pub fn generation(&self) -> Result<Option<String>, ErrorCode> {
        self.check()?;
        self.generation
            .lock()
            .map(|g| g.clone())
            .map_err(|_| ErrorCode::AppUnavailable)
    }
    pub fn bind(&self, generation: &str) -> Result<(), ErrorCode> {
        self.check()?;
        if !lomi_control_protocol::android::valid_device_id(generation) {
            return Err(ErrorCode::StaleGeneration);
        }
        let mut current = self
            .generation
            .lock()
            .map_err(|_| ErrorCode::AppUnavailable)?;
        if current.as_deref().is_some_and(|old| old != generation) {
            return Err(ErrorCode::StaleGeneration);
        }
        *current = Some(generation.into());
        self.check()
    }
    pub fn check_generation(&self, generation: &str) -> Result<(), ErrorCode> {
        if self.generation()?.as_deref() != Some(generation) {
            return Err(ErrorCode::StaleGeneration);
        }
        self.check()
    }
}

pub struct Observation(Arc<AndroidControl>);
impl Drop for Observation {
    fn drop(&mut self) {
        self.0.observing.store(false, Ordering::SeqCst);
    }
}

impl AndroidControl {
    pub fn select(&self, view: Option<String>) {
        if let Ok(mut selected) = self.selected.lock() {
            if *selected != view {
                self.revoke_input();
                *selected = view;
            }
        }
    }
    pub fn check_selected(&self, view: &str) -> Result<(), ErrorCode> {
        self.check()?;
        if self
            .selected
            .lock()
            .map_err(|_| ErrorCode::AppUnavailable)?
            .as_deref()
            != Some(view)
        {
            return Err(ErrorCode::PanelNotRenderable);
        }
        Ok(())
    }
    pub fn revoke_input(&self) {
        if let Ok(mut input) = self.input.lock() {
            if let Some(input) = input.take() {
                input.revoke();
            }
        }
    }
    pub fn acquire_input(
        self: &Arc<Self>,
        view: &str,
        generation: &str,
        id: &str,
    ) -> Result<Arc<crate::android_input::InputLease>, ErrorCode> {
        self.check_generation(generation)?;
        self.check_selected(view)?;
        if !lomi_control_protocol::android::valid_device_id(id) {
            return Err(ErrorCode::ResourceExhausted);
        }
        let lease = Arc::new(crate::android_input::InputLease::new(
            self,
            view.into(),
            generation.into(),
            id.into(),
        ));
        {
            let mut current = self.input.lock().map_err(|_| ErrorCode::AppUnavailable)?;
            if let Some(previous) = current.replace(lease.clone()) {
                previous.revoke();
            }
        }
        lease.check()?;
        Ok(lease)
    }
    pub fn input(&self) -> Option<Arc<crate::android_input::InputLease>> {
        let lease = self.input.lock().ok()?.clone()?;
        lease.check().ok()?;
        Some(lease)
    }
}
