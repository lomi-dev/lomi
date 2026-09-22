use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const FRAME_BYTES: usize = 32 * 1024;
const MAX_CONTEXT: usize = 40 * 1024 * 1024;
const MAX_EVENT: usize = 3 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Event {
    pub protocol_version: u8,
    pub request_id: String,
    pub sequence: u64,
    pub r#type: String,
    pub payload: Value,
}

struct Transfer {
    id: String,
    data: Vec<u8>,
}

pub struct Process {
    child: Arc<Mutex<Child>>,
    data: SyncSender<Transfer>,
    control: SyncSender<String>,
    cancelled: Arc<Mutex<HashMap<String, Instant>>>,
    completed: Arc<Mutex<VecDeque<String>>>,
    stopped: Arc<AtomicBool>,
}

impl Process {
    pub fn start(node: &Path, bundle: &Path) -> Result<(Self, Receiver<Event>), String> {
        if !node.is_absolute() || !bundle.is_absolute() || !node.is_file() || !bundle.is_file() {
            return Err("The bundled AI runtime is unavailable.".into());
        }
        let mut command = Command::new(node);
        command
            .arg("--no-warnings")
            .arg(bundle)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // No PATH, NODE_OPTIONS, proxy configuration, preload hooks, or API keys.
        for name in ["SystemRoot", "WINDIR", "TEMP", "TMP", "TMPDIR", "LANG"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .spawn()
            .map_err(|_| "Cannot start the bundled AI runtime.")?;
        let mut stdin = child.stdin.take().ok_or("AI input pipe unavailable.")?;
        let stdout = child.stdout.take().ok_or("AI output pipe unavailable.")?;
        let child = Arc::new(Mutex::new(child));
        let stopped = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::new(Mutex::new(HashMap::<String, Instant>::new()));
        let completed = Arc::new(Mutex::new(VecDeque::<String>::new()));
        let (data, incoming) = mpsc::sync_channel::<Transfer>(4);
        let (control, commands) = mpsc::sync_channel::<String>(16);
        let (events, received) = mpsc::sync_channel::<Event>(16);
        let writer_stop = stopped.clone();
        let writer_cancel = cancelled.clone();
        thread::spawn(move || {
            let write = |stdin: &mut std::process::ChildStdin,
                         id: &str,
                         kind: &str,
                         payload: Value|
             -> std::io::Result<()> {
                serde_json::to_writer(
                    &mut *stdin,
                    &json!({"protocolVersion":1,"requestId":id,"type":kind,"payload":payload}),
                )?;
                stdin.write_all(b"\n")?;
                stdin.flush()
            };
            let result = (|| -> std::io::Result<()> {
                write(&mut stdin, "handshake", "hello", Value::Null)?;
                loop {
                    if writer_stop.load(Ordering::Acquire) {
                        break;
                    }
                    while let Ok(id) = commands.try_recv() {
                        write(&mut stdin, &id, "cancel", Value::Null)?;
                    }
                    let transfer = match incoming.recv_timeout(Duration::from_millis(10)) {
                        Ok(value) => value,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(_) => break,
                    };
                    if writer_cancel.lock().unwrap().contains_key(&transfer.id) {
                        continue;
                    }
                    write(&mut stdin, &transfer.id, "begin", Value::Null)?;
                    for chunk in transfer.data.chunks(FRAME_BYTES) {
                        while let Ok(id) = commands.try_recv() {
                            write(&mut stdin, &id, "cancel", Value::Null)?;
                        }
                        if writer_cancel.lock().unwrap().contains_key(&transfer.id) {
                            break;
                        }
                        write(
                            &mut stdin,
                            &transfer.id,
                            "append",
                            json!({"data":encode(chunk)}),
                        )?;
                    }
                    if !writer_cancel.lock().unwrap().contains_key(&transfer.id) {
                        write(&mut stdin, &transfer.id, "generate", Value::Null)?;
                    }
                }
                Ok(())
            })();
            if result.is_err() {
                writer_stop.store(true, Ordering::Release);
            }
        });
        let reader_cancel = cancelled.clone();
        let reader_completed = completed.clone();
        let reader_stop = stopped.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut sequences = HashMap::<String, u64>::new();
            let result = (|| -> Result<(), ()> {
                loop {
                    let line = bounded_line(&mut reader, MAX_EVENT).map_err(|_| ())?;
                    if line.is_empty() {
                        break;
                    }
                    let event: Event = serde_json::from_slice(&line).map_err(|_| ())?;
                    if event.protocol_version != 1 || event.request_id.len() > 100 {
                        return Err(());
                    }
                    if event.r#type != "ready" {
                        let sequence = sequences.entry(event.request_id.clone()).or_default();
                        if event.sequence != *sequence + 1 {
                            return Err(());
                        }
                        *sequence = event.sequence;
                    }
                    let terminal =
                        matches!(event.r#type.as_str(), "completed" | "failed" | "cancelled");
                    if terminal {
                        let mut completed = reader_completed.lock().unwrap();
                        if completed.len() >= 256 {
                            completed.pop_front();
                        }
                        completed.push_back(event.request_id.clone());
                        reader_cancel.lock().unwrap().remove(&event.request_id);
                        sequences.remove(&event.request_id);
                    }
                    events.send(event).map_err(|_| ())?;
                }
                Ok(())
            })();
            let _ = result;
            reader_stop.store(true, Ordering::Release);
        });
        let watchdog_child = child.clone();
        let watchdog_stop = stopped.clone();
        let watchdog_cancel = cancelled.clone();
        thread::spawn(move || loop {
            let expired = watchdog_cancel
                .lock()
                .unwrap()
                .values()
                .any(|at| at.elapsed() >= Duration::from_secs(3));
            let mut child = watchdog_child.lock().unwrap();
            if expired || watchdog_stop.load(Ordering::Acquire) {
                let _ = child.kill();
                let _ = child.wait();
                watchdog_stop.store(true, Ordering::Release);
                break;
            }
            if child.try_wait().ok().flatten().is_some() {
                watchdog_stop.store(true, Ordering::Release);
                break;
            }
            drop(child);
            thread::sleep(Duration::from_millis(25));
        });
        Ok((
            Self {
                child,
                data,
                control,
                cancelled,
                completed,
                stopped,
            },
            received,
        ))
    }

    pub fn generate(&self, id: &str, payload: &Value) -> Result<(), String> {
        if !valid_id(id) || self.stopped.load(Ordering::Acquire) {
            return Err("AI process unavailable.".into());
        }
        let data = serde_json::to_vec(payload).map_err(|_| "Invalid AI context.")?;
        if data.len() > MAX_CONTEXT {
            return Err("The conversation exceeds the 40 MiB context limit.".into());
        }
        self.data
            .try_send(Transfer {
                id: id.into(),
                data,
            })
            .map_err(|_| "AI input queue is full.".into())
    }

    pub fn cancel(&self, id: &str) -> Result<(), String> {
        if !valid_id(id) {
            return Err("Invalid request ID.".into());
        }
        let completed = self.completed.lock().map_err(|_| "AI state unavailable.")?;
        if completed.iter().any(|request| request == id) {
            return Ok(());
        }
        let mut cancelled = self.cancelled.lock().map_err(|_| "AI state unavailable.")?;
        if cancelled.len() >= 16 && !cancelled.contains_key(id) {
            return Err("Too many cancellation requests.".into());
        }
        cancelled.entry(id.into()).or_insert_with(Instant::now);
        // The deadline remains armed even if the writer is blocked or its queue is full.
        let _ = self.control.try_send(id.into());
        Ok(())
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }

    pub fn id(&self) -> u32 {
        self.child.lock().unwrap().id()
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub fn bounded_line(reader: &mut impl BufRead, limit: usize) -> std::io::Result<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return if line.is_empty() {
                Ok(line)
            } else {
                Err(std::io::ErrorKind::UnexpectedEof.into())
            };
        }
        let end = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1);
        let length = end.unwrap_or(buffer.len());
        if line.len() + length > limit {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        line.extend_from_slice(&buffer[..length]);
        reader.consume(length);
        if end.is_some() {
            return Ok(line);
        }
    }
}

pub(crate) fn encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let word = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        output.push(ALPHABET[((word >> 18) & 63) as usize] as char);
        output.push(ALPHABET[((word >> 12) & 63) as usize] as char);
        output.push(if chunk.len() > 1 {
            ALPHABET[((word >> 6) & 63) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            ALPHABET[(word & 63) as usize] as char
        } else {
            '='
        });
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_unicode_and_frame_limit() {
        let data = "Zażółć 日本語 👩🏽‍💻\nnext\n";
        let mut reader = BufReader::with_capacity(1, data.as_bytes());
        assert_eq!(
            String::from_utf8(bounded_line(&mut reader, 100).unwrap()).unwrap(),
            "Zażółć 日本語 👩🏽‍💻\n"
        );
        assert_eq!(bounded_line(&mut reader, 100).unwrap(), b"next\n");
        assert!(bounded_line(&mut BufReader::new(&b"123456"[..]), 5).is_err());
        assert_eq!(encode("日本語".as_bytes()), "5pel5pys6Kqe");
    }
    #[test]
    fn cancellation_deadline_survives_blocked_output_delivery() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let node = root.join(format!(
            "binaries/lomi-node-{}{}",
            env!("LOMI_AI_TARGET"),
            if cfg!(windows) { ".exe" } else { "" }
        ));
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("blocked-output.cjs");
        std::fs::write(&bundle,r#"const fs=require('fs');let sequence=0;setInterval(()=>{for(let i=0;i<64;i++)fs.writeSync(1,JSON.stringify({protocolVersion:1,requestId:'blocked',sequence:++sequence,type:'chunk',payload:{type:'text-delta',id:'t',delta:'x'.repeat(32768)}})+'\n');},1);"#).unwrap();
        let (process, events) = Process::start(&node, &bundle).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let start = Instant::now();
        process.cancel("blocked").unwrap();
        while !process.stopped.load(Ordering::Acquire) && start.elapsed() < Duration::from_secs(4) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(process.stopped.load(Ordering::Acquire));
        assert!(process.child.lock().unwrap().try_wait().unwrap().is_some());
        assert!(start.elapsed() < Duration::from_millis(3300));
        drop(events);
    }
    #[test]
    fn cancellation_deadline_survives_a_blocked_input_pipe() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let target = env!("LOMI_AI_TARGET");
        let suffix = if cfg!(windows) { ".exe" } else { "" };
        let node = root.join(format!("binaries/lomi-node-{target}{suffix}"));
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("blocked.cjs");
        std::fs::write(&bundle, "setInterval(() => {}, 1000)").unwrap();
        let (process, _events) = Process::start(&node, &bundle).unwrap();
        process
            .generate("blocked", &json!({"context":"x".repeat(8 * 1024 * 1024)}))
            .unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let start = Instant::now();
        process.cancel("blocked").unwrap();
        while !process.stopped.load(Ordering::Acquire) && start.elapsed() < Duration::from_secs(4) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(process.stopped.load(Ordering::Acquire));
        assert!(start.elapsed() < Duration::from_millis(3300));
        assert!(process.child.lock().unwrap().try_wait().unwrap().is_some());
    }
}
