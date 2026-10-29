use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

pub(super) const IDLE_TIMEOUT: Duration = Duration::from_secs(3600);

#[derive(Clone)]
pub(crate) struct Activity(Arc<Mutex<Clock>>);

struct Clock {
    latest: SystemTime,
    armed: bool,
    elapsed: bool,
}

impl Default for Activity {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Clock {
            latest: SystemTime::now(),
            armed: false,
            elapsed: false,
        })))
    }
}

impl Activity {
    pub(crate) fn record(&self) {
        self.record_at(SystemTime::now());
    }
    fn record_at(&self, now: SystemTime) {
        if let Ok(mut clock) = self.0.lock() {
            // A wake gesture must not erase an already elapsed idle deadline.
            clock.elapsed |= clock.armed && idle_since(clock.latest, now);
            clock.latest = now;
        }
    }
    pub(super) fn activate(&self) {
        self.activate_at(SystemTime::now());
    }
    fn activate_at(&self, now: SystemTime) {
        if let Ok(mut clock) = self.0.lock() {
            clock.latest = now;
            clock.armed = true;
            clock.elapsed = false;
        }
    }
    pub(super) fn restore(&self) {
        self.restore_at(SystemTime::now());
    }
    fn restore_at(&self, now: SystemTime) {
        if let Ok(mut clock) = self.0.lock() {
            // Background authorization recovery is not user activity.
            if !clock.armed {
                clock.latest = now;
                clock.armed = true;
            }
        }
    }
    pub(super) fn deactivate(&self) {
        if let Ok(mut clock) = self.0.lock() {
            clock.armed = false;
            clock.elapsed = false;
        }
    }

    pub(super) fn is_idle(&self, now: SystemTime) -> bool {
        self.0
            .lock()
            .is_ok_and(|clock| clock.armed && (clock.elapsed || idle_since(clock.latest, now)))
    }
}

fn idle_since(latest: SystemTime, now: SystemTime) -> bool {
    now.duration_since(latest)
        .is_ok_and(|idle| idle > IDLE_TIMEOUT)
}
