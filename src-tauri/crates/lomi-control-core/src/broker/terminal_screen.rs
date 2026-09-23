use super::*;
use std::sync::mpsc::{sync_channel, SyncSender};

pub type ScreenDispatch = Arc<dyn Fn(ScreenRequest) -> io::Result<()> + Send + Sync>;
pub(super) struct PendingScreen {
    epoch: String,
    answer: SyncSender<Option<TerminalScreen>>,
}
impl Broker {
    pub fn set_screen_dispatch(&self, dispatch: ScreenDispatch) -> io::Result<()> {
        *self.screen_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    pub fn screen_reply(&self, reply: ScreenReply) -> io::Result<()> {
        let mut screens = self.screens.lock().map_err(|_| failure())?;
        let pending = screens.get(&reply.request_id).ok_or_else(failure)?;
        if pending.epoch != reply.ui_epoch {
            return Err(failure());
        }
        if reply.screen.as_ref().is_some_and(|s| s.text.len() > 65536) {
            return Err(failure());
        }
        screens
            .remove(&reply.request_id)
            .unwrap()
            .answer
            .try_send(reply.screen)
            .map_err(|_| failure())
    }
    pub(super) fn read_screen(&self, owner: &str, input: TerminalReadInput) -> Reply {
        let max_bytes = input.max_bytes.unwrap_or(16 * 1024);
        if !(1..=65536).contains(&max_bytes)
            || input.cursor.is_some()
            || input.operation_id.is_some()
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let (epoch, minimum, authorization) = {
            let Ok(state) = self.lock_state() else {
                return error(ErrorCode::ControlRevoked);
            };
            let target = match Self::terminal_target(
                &state,
                owner,
                &input.workspace_id,
                &input.panel_id,
                &input.terminal_session_id,
                "terminal.read",
            ) {
                Ok(t) => t,
                Err(e) => return error(e),
            };
            let Ok(control) = target.control.lock() else {
                return error(ErrorCode::AppUnavailable);
            };
            let stream = control.watermarks().0;
            let minimum = match input
                .minimum_parsed_sequence
                .as_ref()
                .map(|s| s.parse::<u64>())
                .transpose()
            {
                Ok(m) if m.is_none_or(|m| m <= stream) => m.unwrap_or(stream),
                _ => return error(ErrorCode::RevisionConflict),
            };
            (
                state.projection.ui_epoch.clone(),
                minimum,
                state.policy_revision,
            )
        };
        let Some(dispatch) = self.screen_dispatch.lock().ok().and_then(|d| d.clone()) else {
            return error(ErrorCode::UiNotReady);
        };
        let Ok(request_id) = new_id() else {
            return error(ErrorCode::ResourceExhausted);
        };
        let (answer, receive) = sync_channel(1);
        {
            let Ok(mut screens) = self.screens.lock() else {
                return error(ErrorCode::AppUnavailable);
            };
            if screens.len() >= 16 {
                return error(ErrorCode::ResourceExhausted);
            }
            screens.insert(
                request_id.clone(),
                PendingScreen {
                    epoch: epoch.clone(),
                    answer,
                },
            );
        }
        let request = ScreenRequest {
            request_id: request_id.clone(),
            ui_epoch: epoch.clone(),
            workspace_id: input.workspace_id.clone(),
            panel_id: input.panel_id.clone(),
            terminal_session_id: input.terminal_session_id.clone(),
            minimum_parsed_sequence: minimum.to_string(),
            max_bytes,
        };
        let response = if dispatch(request).is_ok() {
            receive.recv_timeout(Duration::from_secs(2)).ok().flatten()
        } else {
            None
        };
        if let Ok(mut screens) = self.screens.lock() {
            screens.remove(&request_id);
        }
        let Some(mut screen) = response else {
            return error(ErrorCode::UiNotReady);
        };
        let Ok(state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        if authorization != state.policy_revision || state.projection.ui_epoch != epoch {
            return error(ErrorCode::ControlRevoked);
        }
        let target = match Self::terminal_target(
            &state,
            owner,
            &input.workspace_id,
            &input.panel_id,
            &input.terminal_session_id,
            "terminal.read",
        ) {
            Ok(t) => t,
            Err(e) => return error(e),
        };
        let Ok(control) = target.control.lock() else {
            return error(ErrorCode::AppUnavailable);
        };
        let stream = control.watermarks().0;
        let parsed = match screen.parsed_sequence.parse::<u64>() {
            Ok(p) if p <= stream => p,
            _ => return error(ErrorCode::StaleGeneration),
        };
        if screen.workspace_id != input.workspace_id
            || screen.panel_id != input.panel_id
            || screen.terminal_session_id != input.terminal_session_id
            || screen.text.len() > max_bytes as usize
            || screen.rows == 0
            || screen.columns == 0
            || screen.rows > 1024
            || screen.columns > 1024
            || screen.cursor_column >= screen.columns
            || screen.cursor_row >= screen.rows
        {
            return error(ErrorCode::StaleGeneration);
        }
        screen.stream_sequence = stream.to_string();
        screen.parser_pending = parsed < stream;
        Reply::ok(Data::TerminalScreen(Box::new(screen)))
    }
}
