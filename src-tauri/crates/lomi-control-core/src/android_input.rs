//! A device input lease has one in-flight sequence and a bounded retry ledger.
use crate::android::AndroidControl;
use lomi_control_protocol::ErrorCode;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
};

pub struct InputLease {
    pub id: String,
    pub view: String,
    pub generation: String,
    control: Weak<AndroidControl>,
    active: AtomicBool,
    ledger: Mutex<Ledger>,
}
#[derive(Default)]
struct Ledger {
    sequence: u64,
    pending: Option<u64>,
    receipts: BTreeMap<u64, ([u8; 32], String)>,
}
#[derive(Debug, PartialEq)]
pub enum Reservation {
    New,
    Replay(String),
}
impl InputLease {
    pub(crate) fn new(
        control: &Arc<AndroidControl>,
        view: String,
        generation: String,
        id: String,
    ) -> Self {
        Self {
            id,
            view,
            generation,
            control: Arc::downgrade(control),
            active: AtomicBool::new(true),
            ledger: Mutex::new(Ledger::default()),
        }
    }
    pub fn belongs_to(&self, control: &Arc<AndroidControl>) -> bool {
        self.control
            .upgrade()
            .is_some_and(|current| Arc::ptr_eq(&current, control))
    }
    pub fn check(&self) -> Result<(), ErrorCode> {
        if !self.active.load(Ordering::SeqCst) {
            return Err(ErrorCode::ControlRevoked);
        }
        let control = self.control.upgrade().ok_or(ErrorCode::ControlRevoked)?;
        control.check_generation(&self.generation)?;
        control.check_selected(&self.view)
    }
    pub fn revoke(&self) {
        self.active.store(false, Ordering::SeqCst);
    }
    pub fn lookup(&self, sequence: u64, hash: [u8; 32]) -> Result<Option<String>, ErrorCode> {
        self.check()?;
        let ledger = self.ledger.lock().map_err(|_| ErrorCode::AppUnavailable)?;
        if let Some((previous, operation)) = ledger.receipts.get(&sequence) {
            return if *previous == hash {
                Ok(Some(operation.clone()))
            } else {
                Err(ErrorCode::IdempotencyConflict)
            };
        }
        if sequence <= ledger.sequence {
            return Err(ErrorCode::RetryWindowExpired);
        }
        if ledger.pending.is_some() {
            return Err(ErrorCode::TargetBusy);
        }
        if sequence != ledger.sequence + 1 || sequence >= 1 << 53 {
            return Err(ErrorCode::RevisionConflict);
        }
        Ok(None)
    }
    pub fn reserve(
        &self,
        sequence: u64,
        hash: [u8; 32],
        operation: &str,
    ) -> Result<Reservation, ErrorCode> {
        self.check()?;
        if sequence == 0 || sequence >= 1 << 53 {
            return Err(ErrorCode::RevisionConflict);
        }
        let mut ledger = self.ledger.lock().map_err(|_| ErrorCode::AppUnavailable)?;
        if let Some((previous, operation)) = ledger.receipts.get(&sequence) {
            return if *previous == hash {
                Ok(Reservation::Replay(operation.clone()))
            } else {
                Err(ErrorCode::IdempotencyConflict)
            };
        }
        if sequence <= ledger.sequence {
            return Err(ErrorCode::RetryWindowExpired);
        }
        if ledger.pending.is_some() {
            return Err(ErrorCode::TargetBusy);
        }
        if sequence != ledger.sequence + 1 {
            return Err(ErrorCode::RevisionConflict);
        }
        self.check()?;
        ledger.sequence = sequence;
        ledger.pending = Some(sequence);
        ledger.receipts.insert(sequence, (hash, operation.into()));
        while ledger.receipts.len() > 128 {
            ledger.receipts.pop_first();
        }
        Ok(Reservation::New)
    }
    pub fn complete(&self, sequence: u64) -> Result<(), ErrorCode> {
        self.check()?;
        let mut ledger = self.ledger.lock().map_err(|_| ErrorCode::AppUnavailable)?;
        if ledger.pending != Some(sequence) {
            return Err(ErrorCode::RevisionConflict);
        }
        ledger.pending = None;
        Ok(())
    }
}
