use super::*;
use std::sync::mpsc::{sync_channel, SyncSender};

pub type SettingsReadDispatch = Arc<dyn Fn(SettingsReadRequest) -> io::Result<()> + Send + Sync>;
pub(super) struct PendingRead {
    epoch: String,
    answer: SyncSender<SettingsReadReply>,
}
fn revision(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Broker {
    pub fn set_settings_read_dispatch(&self, dispatch: SettingsReadDispatch) -> io::Result<()> {
        *self.settings_read_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    pub fn settings_read_reply(&self, reply: SettingsReadReply) -> io::Result<()> {
        if serde_json::to_vec(&reply).map_or(true, |v| v.len() > MAX_METADATA_BYTES) {
            return Err(failure());
        }
        let mut pending = self.settings_reads.lock().map_err(|_| failure())?;
        if pending
            .get(&reply.request_id)
            .is_none_or(|r| r.epoch != reply.ui_epoch)
        {
            return Err(failure());
        }
        pending
            .remove(&reply.request_id)
            .unwrap()
            .answer
            .try_send(reply)
            .map_err(|_| failure())
    }
    fn settings_read_access(
        state: &State,
        owner: &str,
        workspace: &str,
    ) -> Result<String, ErrorCode> {
        let session = state
            .sessions
            .get(owner)
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .ok_or(ErrorCode::ControlRevoked)?;
        if !session.grant.scopes.contains("settings.read") {
            return Err(ErrorCode::ScopeDenied);
        }
        state
            .projection
            .workspaces
            .iter()
            .find(|w| w.id == workspace && session.grant.permits(w))
            .map(|w| w.project_id.clone())
            .ok_or(ErrorCode::TargetNotFound)
    }
    pub(super) fn read_settings(&self, owner: &str, input: SettingsReadInput) -> Reply {
        if !(1..=200).contains(&input.limit)
            || input.offset > 4096
            || (input.offset != 0
                && (input.section != SettingsSection::Keybinds
                    || input.expected_revision.is_none()))
            || input
                .expected_revision
                .as_ref()
                .is_some_and(|r| !revision(r))
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let (project_id, epoch, connected, policy) = {
            let Ok(state) = self.lock_state() else {
                return error(ErrorCode::ControlRevoked);
            };
            let project = match Self::settings_read_access(&state, owner, &input.workspace_id) {
                Ok(p) => p,
                Err(e) => return error(e),
            };
            (
                project,
                state.projection.ui_epoch.clone(),
                state.sessions[owner].alive.clone(),
                state.policy_revision,
            )
        };
        let _producer = match self.file_reads.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => return error(ErrorCode::ResourceExhausted),
        };
        let Some(dispatch) = self
            .settings_read_dispatch
            .lock()
            .ok()
            .and_then(|d| d.clone())
        else {
            return error(ErrorCode::UiNotReady);
        };
        let Ok(id) = new_id() else {
            return error(ErrorCode::ResourceExhausted);
        };
        let (answer, receive) = sync_channel(1);
        {
            let Ok(mut pending) = self.settings_reads.lock() else {
                return error(ErrorCode::AppUnavailable);
            };
            if pending.len() >= 16 {
                return error(ErrorCode::ResourceExhausted);
            }
            pending.insert(
                id.clone(),
                PendingRead {
                    epoch: epoch.clone(),
                    answer,
                },
            );
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        let check = || {
            if !connected.load(Ordering::SeqCst)
                || self.authorization.load(Ordering::SeqCst) != policy
            {
                return Err(ErrorCode::ControlRevoked);
            }
            if Instant::now() >= deadline {
                return Err(ErrorCode::UiNotReady);
            }
            Ok(())
        };
        let read = || -> Result<SettingsReadReply, ErrorCode> {
            check()?;
            dispatch(SettingsReadRequest {
                request_id: id.clone(),
                ui_epoch: epoch,
                project_id: project_id.clone(),
                input: input.clone(),
            })
            .map_err(|_| ErrorCode::UiNotReady)?;
            loop {
                check()?;
                match receive.recv_timeout(Duration::from_millis(25)) {
                    Ok(reply) => return Ok(reply),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => return Err(ErrorCode::UiNotReady),
                }
            }
        };
        let result = read();
        if let Ok(mut pending) = self.settings_reads.lock() {
            pending.remove(&id);
        }
        let reply = match result {
            Ok(r) => r,
            Err(e) => return error(e),
        };
        if let Err(e) = check() {
            return error(e);
        }
        if let Some(e) = reply.error {
            return error(e);
        }
        let Some(snapshot) = reply.snapshot else {
            return error(ErrorCode::UiNotReady);
        };
        if snapshot.workspace_id != input.workspace_id
            || snapshot.section != input.section
            || snapshot.values.section() != input.section
            || !revision(&snapshot.revision)
        {
            return error(ErrorCode::OutcomeUnknown);
        }
        match &snapshot.values {
            SettingsValues::Keybinds {
                items,
                total,
                offset,
                next_offset,
                ..
            } => {
                let end = usize::from(*offset) + items.len();
                if *offset != input.offset
                    || *total > 4096
                    || end > usize::from(*total)
                    || items.len() > usize::from(input.limit)
                    || *next_offset != (end < usize::from(*total)).then_some(end as u16)
                    || (items.is_empty() && end < usize::from(*total))
                    || items.iter().any(|i| {
                        i.action.is_empty()
                            || i.action.len() > 160
                            || [&i.shortcut, &i.default_shortcut]
                                .iter()
                                .any(|v| v.as_ref().is_some_and(|s| s.len() > 64))
                    })
                {
                    return error(ErrorCode::OutcomeUnknown);
                }
            }
            SettingsValues::Editor { tab_size, .. } if !(1..=16).contains(tab_size) => {
                return error(ErrorCode::OutcomeUnknown)
            }
            _ => {}
        }
        if input
            .expected_revision
            .as_ref()
            .is_some_and(|r| *r != snapshot.revision)
        {
            return error(ErrorCode::RevisionConflict);
        }
        let Ok(state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        match Self::settings_read_access(&state, owner, &input.workspace_id) {
            Ok(p) if p == project_id => {}
            Ok(_) => return error(ErrorCode::ControlRevoked),
            Err(e) => return error(e),
        }
        if let Err(e) = check() {
            return error(e);
        }
        Reply::ok(Data::SettingsSnapshot(Box::new(snapshot)))
    }
}
