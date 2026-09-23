use super::*;
use crate::terminal::{InputReceipt, TerminalControl};

pub type TerminalInputDispatch =
    Arc<dyn Fn(TerminalInputRequest) -> Result<InputReceipt, ErrorCode> + Send + Sync>;
pub struct TerminalInputRequest {
    pub generation: String,
    pub lease: String,
    pub sequence: u64,
    pub payload: Vec<u8>,
    pub control: Arc<Mutex<TerminalControl>>,
    pub permit: Arc<dyn Fn() -> Option<Vec<u32>> + Send + Sync>,
}
impl Broker {
    pub fn set_terminal_input_dispatch(&self, dispatch: TerminalInputDispatch) -> io::Result<()> {
        *self.terminal_input_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    pub(super) async fn call_async(self: &Arc<Self>, id: &str, request: Request) -> Reply {
        let broker = self.clone();
        let id = id.to_string();
        self.spawn_worker(move || match request {
            Request::InputTerminal(input) => broker.input_terminal(&id, input),
            request => broker.call(&id, request),
        })
        .await
        .unwrap_or_else(|_| error(ErrorCode::OutcomeUnknown))
    }
    fn input_terminal(self: &Arc<Self>, id: &str, input: TerminalInput) -> Reply {
        let Ok(sequence) = input.input_sequence.parse::<u64>() else {
            return error(ErrorCode::RevisionConflict);
        };
        if sequence == 0 {
            return error(ErrorCode::RevisionConflict);
        }
        let payload = match &input.input {
            TerminalPayload::Text { text } => {
                if text.is_empty() || text.len() > 64 * 1024 || text.contains('\0') {
                    return error(ErrorCode::ResourceExhausted);
                }
                text.as_bytes().to_vec()
            }
            TerminalPayload::Key { key } => vec![match key {
                TerminalKey::Enter => b'\r',
                TerminalKey::Tab => b'\t',
                TerminalKey::Escape => 27,
                TerminalKey::Backspace => 127,
                TerminalKey::CtrlC => 3,
                TerminalKey::CtrlD => 4,
            }],
        };
        let (control, dispatch) = {
            let Ok(state) = self.lock_state() else {
                return error(ErrorCode::AppUnavailable);
            };
            let target = match Self::terminal_target(
                &state,
                id,
                &input.workspace_id,
                &input.panel_id,
                &input.terminal_session_id,
                "terminal.execute",
            ) {
                Ok(t) => t,
                Err(e) => return error(e),
            };
            let Some(dispatch) = self
                .terminal_input_dispatch
                .lock()
                .ok()
                .and_then(|d| d.clone())
            else {
                return error(ErrorCode::HostUnqualified);
            };
            (target.control.clone(), dispatch)
        };
        let weak = Arc::downgrade(self);
        let owner = id.to_string();
        let target = input.clone();
        let deadline = Instant::now() + Duration::from_secs(5);
        let permit = Arc::new(move || {
            if Instant::now() >= deadline {
                return None;
            }
            let broker = weak.upgrade()?;
            let state = broker.writer_state()?;
            let terminal = Self::terminal_target(
                &state,
                &owner,
                &target.workspace_id,
                &target.panel_id,
                &target.terminal_session_id,
                "terminal.execute",
            )
            .ok()?;
            if bounded_lock(&terminal.control, || broker.check_policy(&state).is_ok())?.lease()
                != Some(&target.lease_id)
            {
                return None;
            }
            state.sessions.values().map(|s| s.peer_pid).collect()
        });
        let request = TerminalInputRequest {
            generation: input.terminal_session_id.clone(),
            lease: input.lease_id,
            sequence,
            payload,
            control,
            permit,
        };
        let outcome = dispatch(request);
        let dispatch = match outcome {
            Ok(ack) => match ack {
                InputReceipt::Pending => "pending",
                InputReceipt::Dispatched => "dispatched",
                InputReceipt::OutcomeUnknown => "outcome_unknown",
            },
            Err(code) => return error(code),
        };
        Reply::ok(Data::TerminalInputAck {
            workspace_id: input.workspace_id,
            panel_id: input.panel_id,
            terminal_session_id: input.terminal_session_id,
            input_sequence: sequence.to_string(),
            dispatch: dispatch.into(),
        })
    }
}
