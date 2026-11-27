//! Parent side of `lomi-stt-worker`: spawn, framing, timeouts, kill.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lomi_stt::protocol::{
    read_frame, write_frame, Acceleration, Request, Response, PROTOCOL_VERSION,
};

const HELLO_TIMEOUT: Duration = Duration::from_secs(60);
const LOAD_TIMEOUT: Duration = Duration::from_secs(180);
const STDERR_LINES: usize = 200;

pub struct Worker {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    responses: Receiver<Response>,
    /// Replies that arrived while only speech edges were being read.
    pending: VecDeque<Response>,
    /// Recent worker diagnostics, kept for error reports instead of a log file.
    stderr: Arc<Mutex<VecDeque<String>>>,
    loaded: Option<PathBuf>,
}

pub fn binary() -> Result<PathBuf, String> {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("LOMI_STT_WORKER") {
        return Ok(path.into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let name = if cfg!(windows) {
        "lomi-stt-worker.exe"
    } else {
        "lomi-stt-worker"
    };
    let path = exe.parent().ok_or("No executable directory.")?.join(name);
    if path.is_file() {
        Ok(path)
    } else {
        Err("Dictation is not available in this build.".into())
    }
}

impl Worker {
    pub fn spawn() -> Result<Self, String> {
        let mut command = Command::new(binary()?);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("Speech engine did not start: {e}"))?;
        let stdin = BufWriter::new(child.stdin.take().ok_or("no stdin")?);
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr_pipe = child.stderr.take().ok_or("no stderr")?;

        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        let lines = stderr.clone();
        // A full stderr pipe would block the worker, so it is always drained.
        std::thread::spawn(move || {
            for line in BufReader::new(stderr_pipe).lines().map_while(Result::ok) {
                #[cfg(debug_assertions)]
                eprintln!("[stt-worker] {line}");
                if let Ok(mut lines) = lines.lock() {
                    if lines.len() == STDERR_LINES {
                        lines.pop_front();
                    }
                    lines.push_back(line);
                }
            }
        });
        let (tx, responses) = mpsc::channel();
        std::thread::spawn(move || {
            let mut stdout = BufReader::new(stdout);
            while let Ok(Some((response, _))) = read_frame::<Response>(&mut stdout) {
                if tx.send(response).is_err() {
                    break;
                }
            }
        });

        let mut worker = Self {
            child,
            stdin,
            responses,
            pending: VecDeque::new(),
            stderr,
            loaded: None,
        };
        match worker.recv(HELLO_TIMEOUT)? {
            Response::Hello { protocol, .. } if protocol == PROTOCOL_VERSION => Ok(worker),
            other => Err(format!("Speech engine is incompatible: {other:?}")),
        }
    }

    /// Loads `model` unless it is already the loaded one.
    pub fn ensure_loaded(&mut self, model: &Path, vad: Option<&Path>) -> Result<(), String> {
        if self.loaded.as_deref() == Some(model) {
            return Ok(());
        }
        self.send(
            &Request::Load {
                model: model.to_string_lossy().into_owned(),
                vad: vad.map(|p| p.to_string_lossy().into_owned()),
                acceleration: Acceleration::Auto,
            },
            &[],
        )?;
        match self.recv(LOAD_TIMEOUT)? {
            Response::Loaded { .. } => {
                self.loaded = Some(model.to_path_buf());
                Ok(())
            }
            Response::Error { message, .. } => Err(message),
            other => Err(format!("Unexpected engine reply: {other:?}")),
        }
    }

    pub fn send(&mut self, request: &Request, payload: &[u8]) -> Result<(), String> {
        write_frame(&mut self.stdin, request, payload).map_err(|_| self.died())
    }

    /// Live speech edges received so far (oldest first), without blocking.
    pub fn speech_edges(&mut self) -> Vec<bool> {
        let mut edges = Vec::new();
        while let Ok(response) = self.responses.try_recv() {
            match response {
                Response::Speech { active } => edges.push(active),
                other => self.pending.push_back(other),
            }
        }
        edges
    }

    /// Next reply; live speech edges are skipped.
    pub fn recv(&mut self, timeout: Duration) -> Result<Response, String> {
        if let Some(response) = self.pending.pop_front() {
            return Ok(response);
        }
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.responses.recv_timeout(left) {
                Ok(Response::Speech { .. }) => {}
                other => return self.reply(other),
            }
        }
    }

    fn reply(&mut self, received: Result<Response, RecvTimeoutError>) -> Result<Response, String> {
        match received {
            Ok(response) => Ok(response),
            Err(RecvTimeoutError::Timeout) => {
                let _ = self.child.kill();
                Err("The speech engine stopped responding.".into())
            }
            Err(RecvTimeoutError::Disconnected) => Err(self.died()),
        }
    }

    fn died(&mut self) -> String {
        let status = self.child.wait().ok();
        let tail = self
            .stderr
            .lock()
            .map(|l| l.iter().rev().take(3).cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        eprintln!("lomi: speech engine exited ({status:?}): {tail:?}");
        "The speech engine stopped unexpectedly.".into()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Closing stdin already makes the worker exit; kill covers a hung one.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
