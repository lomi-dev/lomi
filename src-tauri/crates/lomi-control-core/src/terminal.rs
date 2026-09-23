//! Bounded observation and input coordination, independent of the PTY reader.
//! Callers hold the native writer lock across admission and dispatch. OSC is an
//! observation from an untrusted process; it never creates or restores a lease.
use crate::broker::new_id;
use lomi_control_protocol::ErrorCode;
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
};

const RING_BYTES: usize = 512 * 1024; // Two rings share the 1 MiB observer budget.
const MAX_OBSERVERS: usize = 32;
static OBSERVERS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prompt {
    Unknown,
    Ready,
    Editing,
    Running,
}
#[derive(Clone, Debug)]
pub struct Command {
    pub operation_id: String,
    pub started_observed: bool,
    pub exit_code: Option<i32>,
    pub completed: bool,
    pub start_cursor: u64,
    pub end_cursor: Option<u64>,
}
#[derive(Debug)]
pub struct Output {
    pub text: String,
    pub next_cursor: u64,
    pub gap: bool,
    pub available_from: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputReceipt {
    Pending,
    Dispatched,
    OutcomeUnknown,
}
struct Ack {
    sequence: u64,
    hash: [u8; 32],
    receipt: InputReceipt,
}
enum Parser {
    Text,
    Escape,
    Csi(Vec<u8>),
    Osc(Vec<u8>, bool),
    String(bool),
}

pub struct TerminalControl {
    pub generation: String,
    pub owner: String,
    pub exited: bool,
    pub human_owned: bool,
    pub shell_exit_code: Option<u32>,
    lease: Option<String>,
    authorization: Option<(Arc<AtomicU64>, u64)>,
    connection_alive: Option<Arc<AtomicBool>>,
    observing: bool,
    prompt: Prompt,
    bracketed: bool,
    parser: Parser,
    output: VecDeque<u8>,
    raw: VecDeque<u8>,
    end: u64,
    stream_sequence: u64,
    parsed_sequence: u64,
    received_bytes: u64,
    parsed_bytes: u64,
    parser_boundaries: VecDeque<(u64, u64)>,
    last_sequence: u64,
    acknowledgments: VecDeque<Ack>,
    commands: VecDeque<Command>,
}
impl Drop for TerminalControl {
    fn drop(&mut self) {
        if self.observing {
            OBSERVERS.fetch_sub(1, Ordering::SeqCst);
        }
    }
}
impl TerminalControl {
    pub fn new(owner: String, generation: String) -> Result<Self, ErrorCode> {
        let lease = new_id().map_err(|_| ErrorCode::ResourceExhausted)?;
        OBSERVERS
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                (count < MAX_OBSERVERS).then_some(count + 1)
            })
            .map_err(|_| ErrorCode::ResourceExhausted)?;
        Ok(Self {
            generation,
            owner,
            exited: false,
            human_owned: false,
            shell_exit_code: None,
            lease: Some(lease),
            authorization: None,
            connection_alive: None,
            observing: true,
            prompt: Prompt::Unknown,
            bracketed: false,
            parser: Parser::Text,
            output: VecDeque::with_capacity(RING_BYTES),
            raw: VecDeque::with_capacity(RING_BYTES),
            end: 0,
            stream_sequence: 0,
            parsed_sequence: 0,
            received_bytes: 0,
            parsed_bytes: 0,
            parser_boundaries: VecDeque::new(),
            last_sequence: 0,
            acknowledgments: VecDeque::new(),
            commands: VecDeque::new(),
        })
    }
    pub fn bind_authorization(&mut self, epoch: Arc<AtomicU64>, expected: u64) {
        self.authorization = Some((epoch, expected));
    }
    pub fn bind_connection(&mut self, alive: Arc<AtomicBool>) {
        self.connection_alive = Some(alive);
    }
    pub fn authorized(&self) -> bool {
        self.connection_alive
            .as_ref()
            .is_none_or(|alive| alive.load(Ordering::SeqCst))
            && self
                .authorization
                .as_ref()
                .is_none_or(|(epoch, expected)| epoch.load(Ordering::SeqCst) == *expected)
    }
    pub fn lease(&self) -> Option<&str> {
        self.authorized().then_some(self.lease.as_deref()).flatten()
    }
    pub fn prompt(&self) -> Prompt {
        self.prompt
    }
    pub fn watermarks(&self) -> (u64, u64) {
        (self.stream_sequence, self.parsed_sequence)
    }
    pub fn acknowledge(&mut self, bytes: usize) {
        if !self.observing {
            return;
        }
        self.parsed_bytes = self
            .parsed_bytes
            .saturating_add(bytes as u64)
            .min(self.received_bytes);
        while self
            .parser_boundaries
            .front()
            .is_some_and(|(_, end)| *end <= self.parsed_bytes)
        {
            self.parsed_sequence = self.parser_boundaries.pop_front().unwrap().0;
        }
    }
    pub fn input_sequence(&self) -> u64 {
        self.last_sequence
    }
    pub fn cursor(&self) -> u64 {
        self.end
    }
    pub fn manual_input(&mut self) {
        self.human_owned = true;
        self.lease = None;
        self.prompt = Prompt::Editing;
    }
    pub fn exit(&mut self, code: Option<u32>) {
        self.exited = true;
        self.shell_exit_code = code;
        self.revoke();
    }
    pub fn detach(&mut self) {
        self.revoke();
        self.output.clear();
        self.output.shrink_to_fit();
        self.raw.clear();
        self.raw.shrink_to_fit();
        self.commands.clear();
        self.acknowledgments.clear();
        self.parser = Parser::Text;
        self.parser_boundaries.clear();
        if self.observing {
            self.observing = false;
            OBSERVERS.fetch_sub(1, Ordering::SeqCst);
        }
    }
    pub fn revoke(&mut self) {
        self.lease = None;
    }
    pub fn command(&self, id: &str) -> Option<&Command> {
        self.commands
            .iter()
            .find(|command| command.operation_id == id)
    }
    fn check_lease(&self, lease: &str) -> Result<(), ErrorCode> {
        if self.lease() != Some(lease) {
            return Err(ErrorCode::ControlRevoked);
        }
        Ok(())
    }
    pub fn prepare_interrupt(&self, lease: &str, operation: &str) -> Result<Vec<u8>, ErrorCode> {
        self.check_lease(lease)?;
        let command = self
            .commands
            .back()
            .filter(|c| c.operation_id == operation && !c.completed && c.started_observed)
            .ok_or(ErrorCode::TargetBusy)?;
        let _ = command;
        Ok(vec![3])
    }
    pub fn prepare_run(
        &mut self,
        lease: &str,
        operation_id: &str,
        command: &str,
        foreground_busy: bool,
    ) -> Result<Vec<u8>, ErrorCode> {
        self.check_lease(lease)?;
        if command.trim().is_empty()
            || command.len() > 16 * 1024
            || command.chars().any(char::is_control)
        {
            return Err(ErrorCode::ResourceExhausted);
        }
        if foreground_busy || self.prompt == Prompt::Running {
            return Err(ErrorCode::TargetBusy);
        }
        if self.prompt != Prompt::Ready {
            return Err(ErrorCode::PromptStateUnknown);
        }
        if self.commands.iter().any(|c| c.operation_id == operation_id) {
            return Err(ErrorCode::IdempotencyConflict);
        }
        if self.commands.len() >= 64 {
            self.commands.pop_front();
        }
        self.commands.push_back(Command {
            operation_id: operation_id.into(),
            started_observed: false,
            exit_code: None,
            completed: false,
            start_cursor: self.end,
            end_cursor: None,
        });
        self.prompt = Prompt::Running;
        Ok(if self.bracketed {
            format!("\x1b[200~{command}\x1b[201~\r")
        } else {
            format!("{command}\r")
        }
        .into_bytes())
    }
    /// A duplicate returns its recorded dispatch state and no input bytes.
    pub fn prepare_input(
        &mut self,
        lease: &str,
        sequence: u64,
        payload: &[u8],
    ) -> Result<Option<InputReceipt>, ErrorCode> {
        self.check_lease(lease)?;
        if payload.len() > 64 * 1024 || payload.is_empty() {
            return Err(ErrorCode::ResourceExhausted);
        }
        let hash: [u8; 32] = Sha256::digest(payload).into();
        if sequence <= self.last_sequence {
            let ack = self
                .acknowledgments
                .iter()
                .find(|ack| ack.sequence == sequence)
                .ok_or(ErrorCode::CursorExpired)?;
            if ack.hash != hash {
                return Err(ErrorCode::IdempotencyConflict);
            }
            return Ok(Some(ack.receipt));
        }
        if self.last_sequence.checked_add(1) != Some(sequence) {
            return Err(ErrorCode::RevisionConflict);
        }
        self.last_sequence = sequence;
        if self.acknowledgments.len() >= 64 {
            self.acknowledgments.pop_front();
        }
        self.acknowledgments.push_back(Ack {
            sequence,
            hash,
            receipt: InputReceipt::Pending,
        });
        // Arbitrary input invalidates automatic run readiness until a fresh prompt.
        self.prompt = Prompt::Unknown;
        Ok(None)
    }
    pub fn finish_input(&mut self, sequence: u64, success: bool) {
        if let Some(ack) = self
            .acknowledgments
            .iter_mut()
            .find(|ack| ack.sequence == sequence)
        {
            if ack.receipt == InputReceipt::Pending {
                ack.receipt = if success {
                    InputReceipt::Dispatched
                } else {
                    InputReceipt::OutcomeUnknown
                };
            }
        }
    }
    fn push(&mut self, byte: u8) {
        if self.output.len() == RING_BYTES {
            self.output.pop_front();
        }
        self.output.push_back(byte);
        self.end = self.end.saturating_add(1);
    }
    fn osc(&mut self, bytes: &[u8]) {
        let Ok(text) = std::str::from_utf8(bytes) else {
            return;
        };
        let Some(event) = text.strip_prefix("133;") else {
            return;
        };
        let mut fields = event.split(';');
        match fields.next() {
            Some("A") => {
                if self.lease.is_some() {
                    self.prompt = Prompt::Unknown;
                }
            }
            Some("B") => {
                if self.lease.is_some() {
                    self.prompt = Prompt::Ready;
                }
            }
            Some("C") => {
                self.prompt = Prompt::Running;
                if let Some(command) = self
                    .commands
                    .back_mut()
                    .filter(|command| !command.completed)
                {
                    command.started_observed = true;
                }
            }
            Some("D") => {
                if let Some(command) = self
                    .commands
                    .back_mut()
                    .filter(|command| !command.completed)
                {
                    command.completed = true;
                    command.exit_code = fields.next().and_then(|value| value.parse().ok());
                    command.end_cursor = Some(self.end);
                }
                if self.lease.is_some() {
                    self.prompt = Prompt::Unknown;
                }
            }
            _ => {}
        }
    }
    pub fn set_sequence_base(&mut self, sequence: u64) {
        self.stream_sequence = sequence;
        self.parsed_sequence = sequence;
    }
    pub fn observe(&mut self, bytes: &[u8]) {
        self.observe_sequence(bytes, self.stream_sequence.saturating_add(1));
    }
    pub fn observe_sequence(&mut self, bytes: &[u8], sequence: u64) {
        if !self.observing || bytes.is_empty() {
            return;
        }
        self.stream_sequence = sequence;
        self.received_bytes = self.received_bytes.saturating_add(bytes.len() as u64);
        if self.parser_boundaries.len() == 128 {
            self.parser_boundaries.pop_front();
        }
        self.parser_boundaries
            .push_back((self.stream_sequence, self.received_bytes));
        for &byte in bytes {
            if self.raw.len() == RING_BYTES {
                self.raw.pop_front();
            }
            self.raw.push_back(byte);
            let parser = std::mem::replace(&mut self.parser, Parser::Text);
            self.parser = match parser {
                Parser::Text => match byte {
                    0x1b => Parser::Escape,
                    b'\n' | b'\r' | b'\t' | 0x20..=0x7e | 0x80..=0xff => {
                        self.push(byte);
                        Parser::Text
                    }
                    _ => Parser::Text,
                },
                Parser::Escape => match byte {
                    b'[' => Parser::Csi(Vec::new()),
                    b']' => Parser::Osc(Vec::new(), false),
                    b'P' | b'_' | b'^' | b'X' => Parser::String(false),
                    _ => Parser::Text,
                },
                Parser::Csi(mut value) => {
                    if (0x40..=0x7e).contains(&byte) {
                        if value == b"?2004" {
                            if byte == b'h' {
                                self.bracketed = true;
                            } else if byte == b'l' {
                                self.bracketed = false;
                            }
                        }
                        Parser::Text
                    } else if value.len() < 64 {
                        value.push(byte);
                        Parser::Csi(value)
                    } else {
                        Parser::String(false)
                    }
                }
                Parser::Osc(mut value, escaped) => {
                    if byte == 7 || (escaped && byte == b'\\') {
                        self.osc(&value);
                        Parser::Text
                    } else if value.len() >= 512 {
                        Parser::String(byte == 0x1b)
                    } else {
                        if byte != 0x1b {
                            value.push(byte);
                        }
                        Parser::Osc(value, byte == 0x1b)
                    }
                }
                Parser::String(escaped) => {
                    if byte == 7 || (escaped && byte == b'\\') {
                        Parser::Text
                    } else {
                        Parser::String(byte == 0x1b)
                    }
                }
            };
        }
    }
    pub fn read_raw(
        &self,
        cursor: Option<u64>,
        max_bytes: usize,
    ) -> Result<(Vec<u8>, u64, u64, bool), ErrorCode> {
        if !(1..=65536).contains(&max_bytes) {
            return Err(ErrorCode::ResourceExhausted);
        }
        let start = self.received_bytes - self.raw.len() as u64;
        let requested = cursor.unwrap_or(start);
        if requested > self.received_bytes {
            return Err(ErrorCode::CursorExpired);
        }
        let offset = requested.max(start);
        let bytes: Vec<_> = self
            .raw
            .iter()
            .skip((offset - start) as usize)
            .take(max_bytes)
            .copied()
            .collect();
        let next = offset + bytes.len() as u64;
        Ok((bytes, next, start, requested < start))
    }
    pub fn read_command(
        &self,
        id: &str,
        cursor: Option<u64>,
        max_bytes: usize,
    ) -> Result<Output, ErrorCode> {
        let command = self.command(id).ok_or(ErrorCode::TargetNotFound)?;
        let cursor = cursor.unwrap_or(command.start_cursor);
        if cursor < command.start_cursor {
            return Err(ErrorCode::CursorExpired);
        }
        self.read_until(
            Some(cursor),
            max_bytes,
            command.end_cursor.unwrap_or(self.end),
        )
    }
    pub fn read(&self, cursor: Option<u64>, max_bytes: usize) -> Result<Output, ErrorCode> {
        self.read_until(cursor, max_bytes, self.end)
    }
    fn read_until(
        &self,
        cursor: Option<u64>,
        max_bytes: usize,
        end: u64,
    ) -> Result<Output, ErrorCode> {
        if max_bytes == 0 || max_bytes > 64 * 1024 {
            return Err(ErrorCode::ResourceExhausted);
        }
        let start = self.end - self.output.len() as u64;
        let requested = cursor.unwrap_or(start);
        if requested > end {
            return Err(ErrorCode::CursorExpired);
        }
        let mut offset = requested.max(start) - start;
        // A wrapped ring or user-supplied cursor can land inside a UTF-8 scalar.
        while self
            .output
            .get(offset as usize)
            .is_some_and(|b| b & 0xc0 == 0x80)
        {
            offset += 1;
        }
        let mut bytes: Vec<u8> = self
            .output
            .iter()
            .skip(offset as usize)
            .take(max_bytes.min(end.saturating_sub(start + offset) as usize))
            .copied()
            .collect();
        if let Err(error) = std::str::from_utf8(&bytes) {
            if error.error_len().is_none() && !self.exited {
                if error.valid_up_to() == 0 && bytes.len() == max_bytes {
                    return Err(ErrorCode::ResourceExhausted);
                }
                bytes.truncate(error.valid_up_to());
            }
        }
        let text = String::from_utf8_lossy(&bytes).into_owned();
        // Invalid UTF-8 expands to replacement characters; honor the UTF-8 budget.
        let mut text_end = text.len().min(max_bytes);
        while !text.is_char_boundary(text_end) {
            text_end -= 1;
        }
        if text_end < text.len() {
            return Err(ErrorCode::ResourceExhausted);
        }
        Ok(Output {
            text,
            next_cursor: start + offset + bytes.len() as u64,
            gap: requested < start || offset != requested.max(start) - start,
            available_from: start,
        })
    }
}

impl From<&Command> for lomi_control_protocol::control::CommandObservation {
    fn from(command: &Command) -> Self {
        Self {
            operation_id: command.operation_id.clone(),
            started_observed: command.started_observed,
            completed: command.completed,
            exit_code: command.exit_code,
            source: if command.started_observed || command.completed {
                "shell_integration"
            } else {
                "unknown"
            }
            .into(),
            start_cursor: command.start_cursor.to_string(),
            end_cursor: command.end_cursor.map(|v| v.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parser_watermarks_follow_only_actual_xterm_acknowledgments() {
        let mut monitor = TerminalControl::new("owner".into(), "generation".into()).unwrap();
        monitor.observe("🙂".as_bytes());
        monitor.observe(b"next");
        assert_eq!(monitor.watermarks(), (2, 0));
        monitor.acknowledge(3);
        assert_eq!(monitor.watermarks(), (2, 0));
        monitor.acknowledge(1);
        assert_eq!(monitor.watermarks(), (2, 1));
        monitor.acknowledge(4);
        assert_eq!(monitor.watermarks(), (2, 2));
        assert!(matches!(
            monitor.read(Some(0), 1),
            Err(ErrorCode::ResourceExhausted)
        ));
        for _ in 0..200 {
            monitor.observe(b"x");
        }
        assert!(monitor.parser_boundaries.len() <= 128);
        monitor.acknowledge(200);
        assert_eq!(monitor.watermarks(), (202, 202));
        monitor.detach();
        assert_eq!(monitor.output.capacity(), 0);
        assert_eq!(monitor.raw.capacity(), 0);
        monitor.observe(b"private human output");
        assert_eq!(monitor.watermarks(), (202, 202));
    }
    #[test]
    fn raw_and_command_cursors_do_not_mix_or_disclose_later_output() {
        let mut control = TerminalControl::new("owner".into(), "generation".into()).unwrap();
        control.observe(b"\x1b]133;A\x07\x1b]133;B\x07");
        let lease = control.lease().unwrap().to_string();
        control
            .prepare_run(&lease, "first", "echo one", false)
            .unwrap();
        control.observe(b"\x1b]133;C\x07one\x1b]133;D;0\x07");
        control.observe(b"later-secret");
        assert_eq!(control.read_command("first", None, 64).unwrap().text, "one");
        let (raw, next, _, gap) = control.read_raw(None, 64).unwrap();
        assert!(raw.starts_with(b"\x1b]133;A\x07"));
        assert!(!gap);
        assert!(control.read_raw(Some(next + 1), 64).is_err());
        control.observe(&vec![b'x'; RING_BYTES + 128]);
        assert!(control.read_raw(Some(0), 64).unwrap().3);
        let lost = control.read_command("first", None, 64).unwrap();
        assert!(lost.gap);
        assert_eq!(lost.text, "");
        assert!(control.output.capacity() + control.raw.capacity() <= 1024 * 1024);
    }
    #[test]
    fn fragmented_output_has_bounded_work_and_utf8_cursors() {
        let mut monitor = TerminalControl::new("fixture".into(), "generation".into()).unwrap();
        for byte in "\x1b]133;A\x07prompt\x1b]133;B\x07Zażółć 🙂\n\x1b[31mred\x1b[0m".as_bytes()
        {
            monitor.observe(&[*byte]);
        }
        assert_eq!(monitor.prompt(), Prompt::Ready);
        let output = monitor.read(None, 64).unwrap();
        assert_eq!(output.text, "promptZażółć 🙂\nred");
        let cursor = monitor.cursor();
        monitor.observe(&vec![b'x'; RING_BYTES + 128]);
        let read = monitor.read(Some(cursor), 1024).unwrap();
        assert!(read.gap);
        assert_eq!(read.text.len(), 1024);
        monitor.observe(b"\x1b]");
        monitor.observe(&vec![b'x'; 1024 * 1024]);
        monitor.observe(b"\x07safe");
        assert_eq!(
            monitor.read(Some(monitor.cursor() - 4), 16).unwrap().text,
            "safe"
        );
    }
    #[test]
    fn manual_takeover_cannot_be_undone_by_osc_or_duplicate_input() {
        let mut monitor = TerminalControl::new("fixture".into(), "generation".into()).unwrap();
        let lease = monitor.lease().unwrap().to_string();
        assert_eq!(
            monitor
                .prepare_run(&lease, "op", "echo hi", false)
                .unwrap_err(),
            ErrorCode::PromptStateUnknown
        );
        monitor.observe(b"\x1b]133;A\x07\x1b]133;B\x07");
        assert_eq!(
            monitor
                .prepare_run(&lease, "op", "echo hi\x1b", false)
                .unwrap_err(),
            ErrorCode::ResourceExhausted
        );
        assert_eq!(monitor.prepare_input(&lease, 1, b"text").unwrap(), None);
        monitor.finish_input(1, true);
        assert_eq!(
            monitor.prepare_input(&lease, 1, b"text").unwrap(),
            Some(InputReceipt::Dispatched)
        );
        assert_eq!(
            monitor.prepare_input(&lease, 1, b"other").unwrap_err(),
            ErrorCode::IdempotencyConflict
        );
        assert_eq!(
            monitor.prepare_input(&lease, 3, b"text").unwrap_err(),
            ErrorCode::RevisionConflict
        );
        monitor.manual_input();
        monitor.observe(b"\x1b]133;A\x07\x1b]133;B\x07");
        assert_eq!(
            monitor
                .prepare_run(&lease, "op2", "echo hi", false)
                .unwrap_err(),
            ErrorCode::ControlRevoked
        );
        assert_eq!(
            monitor.prepare_input(&lease, 2, b"text").unwrap_err(),
            ErrorCode::ControlRevoked
        );
    }
}
