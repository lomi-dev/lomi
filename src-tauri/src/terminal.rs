use crate::{
    files::main_window,
    shell::{self, Profile},
};
use portable_pty::{native_pty_system, ChildKiller, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io::{Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc, Condvar, Mutex,
    },
    thread,
    time::Duration,
};
use tauri::{
    ipc::{Channel, Response},
    Manager, State, Window,
};

#[cfg(unix)]
use lomi_control_core::{terminal::TerminalControl, terminal_io};
#[cfg(unix)]
use std::os::fd::BorrowedFd;

const HIGH_WATER: usize = 128 * 1024;

pub mod clipboard;

#[derive(Default)]
struct Flow {
    pending: usize,
    closed: bool,
    #[cfg(unix)]
    control: Option<Arc<Mutex<TerminalControl>>>,
    #[cfg(unix)]
    sequence: u64,
    #[cfg(unix)]
    nonblocking: bool,
}

struct Session {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    flow: Mutex<Flow>,
    // Serializes Remote producers and records the final Exit boundary.
    remote_events: Arc<Mutex<bool>>,
    ready: Condvar,
    pid: Option<u32>,
    profile: Profile,
    human_generation: AtomicU64,
    human_waiters: AtomicUsize,
    remote_lease: Mutex<Option<RemoteLease>>,
}

struct HumanInputGuard(Arc<Session>);
impl Drop for HumanInputGuard {
    fn drop(&mut self) {
        self.0.human_waiters.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Session {
    fn resize(&self, id: String, size: PtySize, sink: Option<RemoteSink>) -> Result<(), String> {
        let remote_closed = self
            .remote_events
            .lock()
            .map_err(|error| error.to_string())?;
        if *remote_closed {
            return Err("Terminal closed.".into());
        }
        {
            let flow = self.flow.lock().map_err(|error| error.to_string())?;
            if flow.closed {
                return Err("Terminal closed.".into());
            }
            self.master
                .lock()
                .map_err(|error| error.to_string())?
                .resize(size)
                .map_err(|error| error.to_string())?;
        }
        if let Some(sink) = sink {
            sink.send(RemoteTerminalEvent::Resize {
                id,
                cols: size.cols,
                rows: size.rows,
            });
        }
        Ok(())
    }

    #[cfg(unix)]
    fn control(&self) -> Option<Arc<Mutex<TerminalControl>>> {
        self.flow.lock().ok().and_then(|f| f.control.clone())
    }
    #[cfg(unix)]
    fn nonblocking(&self) -> bool {
        self.flow.lock().map(|f| f.nonblocking).unwrap_or(true)
    }

    #[cfg(unix)]
    fn control_fd(&self) -> Result<BorrowedFd<'_>, String> {
        let fd = self
            .master
            .lock()
            .map_err(|e| e.to_string())?
            .as_raw_fd()
            .ok_or("Terminal closed.")?;
        // Session owns this immutable master for the whole borrow. Do not hold
        // its resize/context mutex while polling the duplicated descriptor.
        Ok(unsafe { BorrowedFd::borrow_raw(fd) })
    }

    #[cfg(unix)]
    fn protected_origin(&self, peers: &[u32]) -> bool {
        #[cfg(target_os = "macos")]
        {
            let Some(shell_pid) = self.pid else {
                return true;
            };
            if self
                .foreground_program()
                .is_some_and(|p| matches!(p.as_str(), "codex" | "claude" | "agy" | "cursor-agent"))
            {
                return true;
            }
            for &peer in peers {
                let mut pid = peer;
                let mut resolved = false;
                for _ in 0..128 {
                    if pid == shell_pid {
                        return true;
                    }
                    if pid <= 1 {
                        resolved = true;
                        break;
                    }
                    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
                    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
                    let count = unsafe {
                        libc::proc_pidinfo(
                            pid as i32,
                            libc::PROC_PIDTBSDINFO,
                            // Include retained/exiting ancestors. XNU only looks
                            // up zombie BSD identities when this argument is set.
                            1,
                            info.as_mut_ptr().cast(),
                            size,
                        )
                    };
                    if count != size {
                        return true;
                    }
                    let info = unsafe { info.assume_init() };
                    if info.pbi_pid != pid || info.pbi_ppid == pid {
                        return true;
                    }
                    pid = info.pbi_ppid;
                }
                if !resolved {
                    return true;
                }
            }
            peers.is_empty()
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = peers;
            true
        }
    }

    fn has_foreground_process(&self) -> bool {
        #[cfg(unix)]
        if let Ok(master) = self.master.lock() {
            if master
                .process_group_leader()
                .is_some_and(|group| Some(group as u32) != self.pid)
            {
                return true;
            }
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if self.foreground_program().is_some() {
            // exec can replace the shell without changing its PID or process group.
            return true;
        }
        false
    }

    fn title_process(&self) -> Option<crate::cli_titles::TitleProcess> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if self.profile.distro.is_none() {
            let group = self.master.lock().ok()?.process_group_leader()? as u32;
            return crate::cli_titles::process_in_group(group);
        }
        None
    }

    #[cfg(target_os = "linux")]
    fn foreground_program(&self) -> Option<String> {
        let pid = self.master.lock().ok()?.process_group_leader()?;
        if Some(pid as u32) == self.pid {
            let executable = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
            if executable == std::fs::canonicalize(&self.profile.program).ok()? {
                return None;
            }
        }
        // Read only the process name, never command arguments or CLI state files.
        std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .ok()
            .map(|name| name.trim_end_matches('\n').to_owned())
    }

    #[cfg(target_os = "macos")]
    fn foreground_program(&self) -> Option<String> {
        use std::{ffi::c_void, os::unix::ffi::OsStringExt};

        #[link(name = "proc")]
        unsafe extern "C" {
            fn proc_pidpath(pid: i32, buffer: *mut c_void, size: u32) -> i32;
        }

        let pid = self.master.lock().ok()?.process_group_leader()?;
        // libproc requires at most PROC_PIDPATHINFO_MAXSIZE (4 * MAXPATHLEN).
        // Read only the executable path, never the process's arguments or environment.
        let mut buffer = vec![0u8; 4096];
        let count = unsafe { proc_pidpath(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
        if count <= 0 {
            return None;
        }
        buffer.truncate(buffer.iter().position(|byte| *byte == 0)?);
        let executable = PathBuf::from(std::ffi::OsString::from_vec(buffer));
        if Some(pid as u32) == self.pid
            && executable == std::fs::canonicalize(&self.profile.program).ok()?
        {
            return None;
        }
        Some(executable.file_name()?.to_string_lossy().into_owned())
    }

    fn stop(&self) {
        #[cfg(unix)]
        if let Some(control) = self.control() {
            if let Ok(mut control) = control.lock() {
                control.revoke();
            }
        }
        if let Ok(mut flow) = self.flow.lock() {
            flow.closed = true;
        }
        self.ready.notify_all();
        if let Ok(mut killer) = self.killer.lock() {
            let _ = killer.kill();
        }
        if let Ok(mut writer) = self.writer.lock() {
            writer.take();
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum RemoteTerminalEvent {
    Start { id: String, cols: u16, rows: u16 },
    Output { id: String, data: Vec<u8> },
    Resize { id: String, cols: u16, rows: u16 },
    Exit { id: String, code: Option<u32> },
}

pub(crate) struct RemoteObservedEvent {
    pub event: RemoteTerminalEvent,
    pub generation: u64,
}

#[derive(Default)]
struct RemoteObservationState {
    next_generation: u64,
    sessions: HashMap<String, RemoteObservationSession>,
}
struct RemoteObservationSession {
    generation: u64,
    lost: bool,
    exited: bool,
    dimensions: Option<(u16, u16)>,
}
#[derive(Clone, Default)]
pub(crate) struct RemoteObserver {
    state: Arc<Mutex<RemoteObservationState>>,
}
type RemoteSessionObservation = (String, u64, bool, bool, Option<(u16, u16)>);

impl RemoteObserver {
    pub(crate) fn sessions(&self) -> Vec<RemoteSessionObservation> {
        self.state
            .lock()
            .map(|state| {
                state
                    .sessions
                    .iter()
                    .map(|(id, s)| (id.clone(), s.generation, s.lost, s.exited, s.dimensions))
                    .collect()
            })
            .unwrap_or_default()
    }
    pub(crate) fn current(&self, event: &RemoteObservedEvent) -> Option<bool> {
        self.current_generation(event.event.session_id(), event.generation)
    }
    pub(crate) fn current_generation(&self, id: &str, generation: u64) -> Option<bool> {
        self.state
            .lock()
            .ok()?
            .sessions
            .get(id)
            .filter(|s| s.generation == generation)
            .map(|s| s.lost)
    }
    pub(crate) fn retire(&self, id: &str, generation: u64) {
        if let Ok(mut state) = self.state.lock() {
            if state
                .sessions
                .get(id)
                .is_some_and(|s| s.generation == generation)
            {
                state.sessions.remove(id);
            }
        }
    }
}
impl RemoteTerminalEvent {
    pub(crate) fn session_id(&self) -> &str {
        match self {
            Self::Start { id, .. }
            | Self::Output { id, .. }
            | Self::Resize { id, .. }
            | Self::Exit { id, .. } => id,
        }
    }
}

#[derive(Clone)]
struct RemoteSink {
    sender: SyncSender<RemoteObservedEvent>,
    healthy: Arc<std::sync::atomic::AtomicBool>,
    observer: RemoteObserver,
    activity: crate::activity::Activity,
}

impl RemoteSink {
    fn send(&self, event: RemoteTerminalEvent) {
        if !matches!(&event, RemoteTerminalEvent::Output { data, .. } if data.is_empty()) {
            self.activity.record();
        }
        let id = event.session_id().to_owned();
        let generation = {
            let Ok(mut state) = self.observer.state.lock() else {
                self.healthy.store(false, Ordering::SeqCst);
                return;
            };
            if matches!(&event, RemoteTerminalEvent::Start { .. })
                || !state.sessions.contains_key(&id)
            {
                state.next_generation += 1;
                let generation = state.next_generation;
                let dimensions = match &event {
                    RemoteTerminalEvent::Start { cols, rows, .. } => Some((*cols, *rows)),
                    _ => None,
                };
                state.sessions.insert(
                    id.clone(),
                    RemoteObservationSession {
                        generation,
                        lost: false,
                        exited: false,
                        dimensions,
                    },
                );
            }
            let session = state.sessions.get_mut(&id).unwrap();
            if session.lost {
                if matches!(&event, RemoteTerminalEvent::Exit { .. }) {
                    session.exited = true;
                }
                return;
            }
            session.generation
        };
        let exited = matches!(&event, RemoteTerminalEvent::Exit { .. });
        // Local PTY readers and resize handlers must never wait for Remote.
        // A full observer queue fences this generation instead of delaying
        // local output or silently continuing an incomplete Remote model.
        let observed = RemoteObservedEvent { event, generation };
        match self.sender.try_send(observed) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.healthy.store(false, Ordering::SeqCst);
            }
            Err(mpsc::TrySendError::Full(_)) => {
                if let Ok(mut state) = self.observer.state.lock() {
                    if let Some(session) = state
                        .sessions
                        .get_mut(&id)
                        .filter(|s| s.generation == generation)
                    {
                        session.lost = true;
                        session.exited = exited;
                    }
                } else {
                    self.healthy.store(false, Ordering::SeqCst);
                }
            }
        }
    }
}

struct RemoteLease {
    id: String,
    owner: String,
    generation: u64,
    deadline: std::time::Instant,
}

#[derive(serde::Serialize)]
pub(crate) struct RemoteReceipt {
    pub written: usize,
    pub status: &'static str,
}

#[derive(Default, Clone)]
pub struct Terminals {
    sessions: Arc<Mutex<HashMap<String, Arc<Session>>>>,
    remote_sink: Arc<Mutex<Option<RemoteSink>>>,
    pub(crate) activity: crate::activity::Activity,
}

#[derive(Clone)]
pub struct Shells {
    pub profiles: Vec<Profile>,
    pub integration: PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartRequest {
    id: String,
    profile_id: String,
    cwd: String,
    cols: u16,
    rows: u16,
    #[serde(default)]
    agent_ticket: Option<AgentTicket>,
    #[serde(default)]
    cli_launch: Option<crate::cli_catalog::TitleCli>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentTicket {
    operation_id: String,
    nonce: String,
}

fn validate_start_request(request: &StartRequest) -> Result<(), String> {
    if request.agent_ticket.is_some() && request.cli_launch.is_some() {
        return Err("Agent control terminals cannot launch a separate CLI.".into());
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Started {
    cwd: String,
    profile_id: String,
}

#[derive(Serialize, Clone)]
pub struct Exit {
    code: Option<u32>,
}

fn size(cols: u16, rows: u16) -> Result<PtySize, String> {
    if cols == 0 || rows == 0 || cols > 1000 || rows > 1000 {
        return Err("Terminal dimensions must be between 1 and 1000 cells.".into());
    }
    Ok(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })
}

impl Terminals {
    pub(crate) fn observe_remote(
        &self,
    ) -> Result<
        (
            Receiver<RemoteObservedEvent>,
            Arc<std::sync::atomic::AtomicBool>,
            RemoteObserver,
        ),
        String,
    > {
        let mut slot = self
            .remote_sink
            .lock()
            .map_err(|_| "Terminal observer unavailable.")?;
        if slot.is_some()
            || !self
                .sessions
                .lock()
                .map_err(|_| "Terminal unavailable.")?
                .is_empty()
        {
            return Err("Remote observer must attach before terminal startup.".into());
        }
        let (sender, receiver) = mpsc::sync_channel(256);
        let healthy = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let observer = RemoteObserver::default();
        *slot = Some(RemoteSink {
            sender,
            observer: observer.clone(),
            healthy: healthy.clone(),
            activity: self.activity.clone(),
        });
        Ok((receiver, healthy, observer))
    }

    pub(crate) fn remote_claim(&self, id: &str, owner: &str, lease_id: &str) -> Result<(), String> {
        let session = self.get(id)?;
        let _writer = session.writer.lock().map_err(|_| "Terminal unavailable.")?;
        if session.human_waiters.load(Ordering::SeqCst) > 0 {
            return Err("Local input has terminal priority.".into());
        }
        #[cfg(unix)]
        {
            if session
                .control()
                .is_some_and(|c| c.lock().map(|c| c.lease().is_some()).unwrap_or(true))
            {
                return Err("An agent currently owns terminal control.".into());
            }
            let mut flow = session.flow.lock().map_err(|_| "Terminal unavailable.")?;
            terminal_io::make_nonblocking(session.control_fd()?)
                .map_err(|_| "Terminal control unavailable.")?;
            flow.nonblocking = true;
        }
        #[cfg(not(unix))]
        return Err("Remote control is not qualified on this platform.".into());
        #[cfg(unix)]
        {
            let mut lease = session
                .remote_lease
                .lock()
                .map_err(|_| "Terminal unavailable.")?;
            if lease
                .as_ref()
                .is_some_and(|l| l.deadline > std::time::Instant::now() && l.owner != owner)
            {
                return Err("Another remote peer owns terminal control.".into());
            }
            *lease = Some(RemoteLease {
                id: lease_id.into(),
                owner: owner.into(),
                generation: session.human_generation.load(Ordering::SeqCst),
                deadline: std::time::Instant::now() + Duration::from_secs(5),
            });
            Ok(())
        }
    }

    pub(crate) fn remote_renew(&self, id: &str, owner: &str, lease_id: &str) -> Result<(), String> {
        let session = self.get(id)?;
        let writer = session.writer.lock().map_err(|_| "Terminal unavailable.")?;
        if writer.is_none() {
            return Err("The terminal has been closed.".into());
        }
        #[cfg(not(unix))]
        return Err("Remote control is not qualified on this platform.".into());
        #[cfg(unix)]
        {
            let control = session.control();
            let control = control
                .as_ref()
                .map(|c| c.lock())
                .transpose()
                .map_err(|_| "Terminal unavailable.")?;
            let mut lease = session
                .remote_lease
                .lock()
                .map_err(|_| "Terminal unavailable.")?;
            let now = std::time::Instant::now();
            let lease = lease.as_mut().ok_or("Remote lease expired.")?;
            if session.human_waiters.load(Ordering::SeqCst) > 0
                || control.as_ref().is_some_and(|c| c.lease().is_some())
                || lease.id != lease_id
                || lease.owner != owner
                || lease.deadline <= now
                || lease.generation != session.human_generation.load(Ordering::SeqCst)
            {
                return Err("Remote lease expired or preempted.".into());
            }
            // Renewal never creates a lease or adopts a newer human generation.
            lease.deadline = now + Duration::from_secs(5);
            Ok(())
        }
    }

    pub(crate) fn remote_lease_live(&self, id: &str, owner: &str, lease_id: &str) -> bool {
        self.get(id).ok().is_some_and(|session| {
            session
                .remote_lease
                .lock()
                .map(|lease| {
                    lease.as_ref().is_some_and(|l| {
                        l.id == lease_id
                            && l.owner == owner
                            && l.deadline > std::time::Instant::now()
                            && l.generation == session.human_generation.load(Ordering::SeqCst)
                    })
                })
                .unwrap_or(false)
        })
    }

    pub(crate) fn remote_revoke_owner(&self, id: &str, owner: &str) {
        if let Ok(session) = self.get(id) {
            if let Ok(mut lease) = session.remote_lease.lock() {
                if lease.as_ref().is_some_and(|l| l.owner == owner) {
                    session.human_generation.fetch_add(1, Ordering::SeqCst);
                    lease.take();
                }
            }
        }
    }

    pub(crate) fn remote_revoke(&self, id: &str) {
        if let Ok(session) = self.get(id) {
            session.human_generation.fetch_add(1, Ordering::SeqCst);
            if let Ok(mut lease) = session.remote_lease.lock() {
                lease.take();
            }
        }
    }

    #[cfg(unix)]
    pub(crate) fn remote_input(
        &self,
        id: &str,
        owner: &str,
        lease_id: &str,
        data: &[u8],
        permit: impl Fn() -> bool,
    ) -> RemoteReceipt {
        let rejected = || RemoteReceipt {
            written: 0,
            status: "rejected",
        };
        if data.is_empty() || data.len() > 16 * 1024 {
            return rejected();
        }
        let Ok(session) = self.get(id) else {
            return rejected();
        };
        let Ok(writer) = session.writer.lock() else {
            return rejected();
        };
        if writer.is_none() {
            return rejected();
        }
        let allowed = || {
            permit()
                && session
                    .remote_lease
                    .lock()
                    .map(|lease| {
                        lease.as_ref().is_some_and(|l| {
                            l.id == lease_id
                                && l.owner == owner
                                && l.deadline > std::time::Instant::now()
                                && l.generation == session.human_generation.load(Ordering::SeqCst)
                        })
                    })
                    .unwrap_or(false)
        };
        if !allowed() {
            return rejected();
        }
        let Ok(fd) = session.control_fd() else {
            return rejected();
        };
        let receipt = match terminal_io::write(fd, data, Duration::from_secs(2), allowed) {
            Ok(written) => RemoteReceipt {
                written,
                status: "accepted",
            },
            Err(failure) => RemoteReceipt {
                written: failure.written,
                status: if failure.written == 0 {
                    "rejected"
                } else {
                    "partial"
                },
            },
        };
        if receipt.written > 0 {
            self.activity.record();
        }
        receipt
    }

    #[cfg(not(unix))]
    pub(crate) fn remote_input(
        &self,
        _id: &str,
        _owner: &str,
        _lease_id: &str,
        _data: &[u8],
        _permit: impl Fn() -> bool,
    ) -> RemoteReceipt {
        RemoteReceipt {
            written: 0,
            status: "rejected",
        }
    }

    fn busy(&self, ids: &[String]) -> Result<Vec<String>, String> {
        let sessions: Vec<_> = self
            .sessions
            .lock()
            .map_err(|error| error.to_string())?
            .iter()
            .filter(|(id, _)| ids.contains(id))
            .map(|(id, session)| (id.clone(), session.clone()))
            .collect();
        if sessions.is_empty() {
            return Ok(Vec::new());
        }
        let parents = process_parents()?;
        Ok(sessions
            .into_iter()
            .filter(|(_, session)| {
                // WSL processes are outside the host process tree, so these sessions
                // require close confirmation without a host-side child lookup.
                session.profile.distro.is_some()
                    || session.has_foreground_process()
                    || session.pid.is_none_or(|pid| parents.contains(&pid))
            })
            .map(|(id, _)| id)
            .collect())
    }

    pub(crate) fn validate_notification_session(&self, id: Option<&str>) -> Result<(), String> {
        let session = self.get(id.ok_or("A live terminal session is required.")?)?;
        let closed = session
            .remote_events
            .lock()
            .map_err(|_| "Terminal unavailable.")?;
        if *closed
            || session
                .flow
                .lock()
                .map_err(|_| "Terminal unavailable.")?
                .closed
        {
            return Err("The terminal session is no longer running.".into());
        }
        // Membership in the native registry authenticates ownership; CLI names
        // and renderer labels are never used as notification authority.
        Ok(())
    }

    pub(crate) fn notification_agent(
        &self,
        session_id: Option<&str>,
    ) -> Option<crate::cli_catalog::TitleCli> {
        self.get(session_id?)
            .ok()?
            .title_process()
            .map(|process| process.cli)
    }

    pub fn check_title_process(
        &self,
        id: &str,
        process: crate::cli_titles::TitleProcess,
    ) -> Result<(), String> {
        if self.get(id)?.title_process() != Some(process) {
            return Err(
                "The CLI is no longer running in this terminal. Check the settings again.".into(),
            );
        }
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Arc<Session>, String> {
        self.sessions
            .lock()
            .map_err(|error| error.to_string())?
            .get(id)
            .cloned()
            .ok_or_else(|| "The terminal session is no longer running.".into())
    }

    pub fn stop_all(&self) {
        let sessions: Vec<_> = self
            .sessions
            .lock()
            .map(|mut sessions| sessions.drain().map(|(_, session)| session).collect())
            .unwrap_or_default();
        for session in sessions {
            session.stop();
        }
    }

    pub fn close(&self, id: &str) -> Result<(), String> {
        let session = self
            .sessions
            .lock()
            .map_err(|_| "Terminal state is unavailable.")?
            .get(id)
            .cloned();
        if let Some(session) = session {
            session.stop();
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| "Terminal state is unavailable.")?;
            if sessions
                .get(id)
                .is_some_and(|current| Arc::ptr_eq(current, &session))
            {
                sessions.remove(id);
            }
        }
        Ok(())
    }

    pub fn acknowledge(&self, id: &str, bytes: usize) {
        if let Ok(session) = self.get(id) {
            if let Ok(mut flow) = session.flow.lock() {
                #[cfg(unix)]
                if let Some(control) = &flow.control {
                    if let Ok(mut control) = control.lock() {
                        control.acknowledge(bytes);
                    }
                }
                flow.pending = flow.pending.saturating_sub(bytes);
            }
            session.ready.notify_one();
        }
    }

    fn start(
        &self,
        shells: &Shells,
        request: StartRequest,
        output: Channel<Response>,
        exited: Channel<Exit>,
    ) -> Result<Started, String> {
        self.start_control(
            shells,
            request,
            output,
            exited,
            #[cfg(unix)]
            None,
        )
    }
    fn start_control(
        &self,
        shells: &Shells,
        request: StartRequest,
        output: Channel<Response>,
        exited: Channel<Exit>,
        #[cfg(unix)] control: Option<Arc<Mutex<TerminalControl>>>,
    ) -> Result<Started, String> {
        validate_start_request(&request)?;
        if request.id.is_empty() || request.id.len() > 128 {
            return Err("Invalid terminal identifier.".into());
        }
        let profile = shells
            .profiles
            .iter()
            .find(|profile| profile.id == request.profile_id)
            .cloned()
            .ok_or("The selected shell is no longer installed. Choose another shell.")?;
        let (command, cwd) = if let Some(cli) = request.cli_launch {
            let resolved =
                crate::cli_launch::resolve_cli(&profile, &request.cwd, &shells.integration, cli)?;
            shell::build_with_cli(
                &profile,
                &request.cwd,
                &shells.integration,
                &resolved.program,
                resolved.argument,
            )?
        } else {
            shell::build(&profile, &request.cwd, &shells.integration)?
        };
        #[cfg(unix)]
        if let Some(control) = &control {
            if !control
                .lock()
                .map_err(|_| "Control unavailable")?
                .authorized()
            {
                return Err("Control revoked before terminal start".into());
            }
        }
        let pair = native_pty_system()
            .openpty(size(request.cols, request.rows)?)
            .map_err(|error| error.to_string())?;
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| format!("Cannot start {}: {error}", profile.name))?;
        let (master, mut child) = (pair.master, child);
        #[cfg(unix)]
        if control.is_some() {
            let fd = master.as_raw_fd().ok_or("This PTY cannot be controlled.")?;
            terminal_io::make_nonblocking(unsafe { BorrowedFd::borrow_raw(fd) })
                .map_err(|e| e.to_string())?;
        }
        let mut reader = master
            .try_clone_reader()
            .map_err(|error| error.to_string())?;
        let writer = master.take_writer().map_err(|error| error.to_string())?;
        let session = Arc::new(Session {
            pid: child.process_id(),
            profile: profile.clone(),
            master: Mutex::new(master),
            writer: Mutex::new(Some(writer)),
            killer: Mutex::new(child.clone_killer()),
            flow: Mutex::new(Flow {
                #[cfg(unix)]
                nonblocking: control.is_some(),
                #[cfg(unix)]
                control,
                ..Flow::default()
            }),
            remote_events: Arc::new(Mutex::new(false)),
            ready: Condvar::new(),
            human_generation: AtomicU64::new(0),
            human_waiters: AtomicUsize::new(0),
            remote_lease: Mutex::new(None),
        });
        // Remote producers serialize separately from Flow so queue backpressure
        // cannot block local input, acknowledgements or terminal shutdown.
        let remote_events = session.remote_events.clone();
        let remote_order = remote_events.lock().map_err(|error| error.to_string())?;
        {
            let mut sessions = self.sessions.lock().map_err(|error| error.to_string())?;
            if sessions.contains_key(&request.id) {
                drop(sessions);
                session.stop();
                drop(reader);
                drop(session);
                let _ = child.wait();
                return Err("A terminal with this identifier already exists.".into());
            }
            sessions.insert(request.id.clone(), session.clone());
        }
        let remote_sink = self.remote_sink.lock().ok().and_then(|s| s.clone());
        if let Some(sink) = &remote_sink {
            sink.send(RemoteTerminalEvent::Start {
                id: request.id.clone(),
                cols: request.cols,
                rows: request.rows,
            });
        }
        drop(remote_order);
        let sessions = Arc::downgrade(&self.sessions);
        thread::spawn(move || {
            let mut buffer = [0_u8; 16 * 1024];

            loop {
                let mut flow = session
                    .flow
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                while !flow.closed && flow.pending >= HIGH_WATER {
                    flow = session
                        .ready
                        .wait(flow)
                        .unwrap_or_else(|error| error.into_inner());
                }
                if flow.closed {
                    break;
                }
                drop(flow);
                let length = match reader.read(&mut buffer) {
                    #[cfg(unix)]
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && session.nonblocking() =>
                    {
                        let Ok(fd) = session.control_fd() else {
                            break;
                        };
                        if terminal_io::wait_readable(fd).is_err() {
                            break;
                        }
                        continue;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Ok(0) => break,
                    Err(_) => {
                        break;
                    }
                    Ok(length) => length,
                };

                let remote_order = remote_events
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                {
                    let mut flow = session
                        .flow
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    #[cfg(unix)]
                    {
                        flow.sequence = flow.sequence.saturating_add(1);
                        if let Some(control) = &flow.control {
                            if let Ok(mut control) = control.lock() {
                                control.observe_sequence(&buffer[..length], flow.sequence);
                            }
                        }
                    }
                    flow.pending += length;
                }
                if let Some(sink) = &remote_sink {
                    sink.send(RemoteTerminalEvent::Output {
                        id: request.id.clone(),
                        data: buffer[..length].to_vec(),
                    });
                }
                drop(remote_order);
                let local_result = output.send(Response::new(buffer[..length].to_vec()));
                if local_result.is_err() {
                    session.stop();
                    break;
                }
            }
            drop(reader);
            #[cfg(unix)]
            let exit_control = session.control();
            let code = child.wait().ok().map(|status| status.exit_code());
            let mut remote_closed = remote_events
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            *remote_closed = true;
            if let Some(sink) = &remote_sink {
                sink.send(RemoteTerminalEvent::Exit {
                    id: request.id.clone(),
                    code,
                });
            }
            drop(remote_closed);
            #[cfg(unix)]
            if let Some(control) = exit_control {
                if let Ok(mut control) = control.lock() {
                    control.exit(code);
                }
            }
            if let Some(sessions) = sessions.upgrade() {
                if let Ok(mut sessions) = sessions.lock() {
                    if sessions
                        .get(&request.id)
                        .is_some_and(|current| Arc::ptr_eq(current, &session))
                    {
                        sessions.remove(&request.id);
                    }
                }
            }
            let _ = exited.send(Exit { code });
            // An ordered EOF marker keeps the exit notice behind all output chunks.
            let _ = output.send(Response::new(Vec::<u8>::new()));
        });
        Ok(Started {
            cwd,
            profile_id: profile.id,
        })
    }

    #[cfg(unix)]
    pub(crate) fn agent_attach(
        &self,
        generation: &str,
        control: &Arc<Mutex<TerminalControl>>,
        profile: &lomi_control_protocol::control::TerminalProfile,
        peers: &[u32],
    ) -> Result<(), lomi_control_protocol::ErrorCode> {
        use lomi_control_protocol::ErrorCode;
        let session = self
            .get(generation)
            .map_err(|_| ErrorCode::StaleGeneration)?;
        let writer = session
            .writer
            .try_lock()
            .map_err(|_| ErrorCode::TargetBusy)?;
        if writer.is_none() {
            return Err(ErrorCode::StaleGeneration);
        }
        if session.profile.id != profile.id
            || lomi_control_core::broker::certificate_hash(
                &serde_json::to_vec(&session.profile).map_err(|_| ErrorCode::HostUnqualified)?,
            ) != profile.revision
        {
            return Err(ErrorCode::HostUnqualified);
        }
        if session.protected_origin(peers) {
            return Err(ErrorCode::ProtectedOriginTerminal);
        }
        let mut flow = session.flow.try_lock().map_err(|_| ErrorCode::TargetBusy)?;
        if flow.closed {
            return Err(ErrorCode::StaleGeneration);
        }
        if flow.pending != 0 {
            return Err(ErrorCode::TargetBusy);
        }
        let mut observer = control.try_lock().map_err(|_| ErrorCode::TargetBusy)?;
        if !observer.authorized() {
            return Err(ErrorCode::ControlRevoked);
        }
        // Only attach at an acknowledged producer boundary. No second PTY reader.
        terminal_io::make_nonblocking(
            session
                .control_fd()
                .map_err(|_| ErrorCode::HostUnqualified)?,
        )
        .map_err(|_| ErrorCode::HostUnqualified)?;
        flow.nonblocking = true;
        observer.set_sequence_base(flow.sequence);
        if let Some(previous) = &flow.control {
            if let Ok(mut previous) = previous.try_lock() {
                previous.detach();
            } else {
                return Err(ErrorCode::TargetBusy);
            }
        }
        flow.control = Some(control.clone());
        Ok(())
    }

    #[cfg(unix)]
    pub(crate) fn agent_close(
        &self,
        generation: &str,
        control: &Arc<Mutex<TerminalControl>>,
        peers: &[u32],
        commit: bool,
    ) -> Result<(), lomi_control_protocol::ErrorCode> {
        use lomi_control_protocol::ErrorCode;
        #[cfg(target_os = "macos")]
        {
            let mut sessions = self
                .sessions
                .try_lock()
                .map_err(|_| ErrorCode::TargetBusy)?;
            let Some(session) = sessions.get(generation).cloned() else {
                let observation = control.try_lock().map_err(|_| ErrorCode::TargetBusy)?;
                return if observation.exited && !observation.human_owned && observation.authorized()
                {
                    Ok(())
                } else {
                    Err(ErrorCode::StaleGeneration)
                };
            };
            if session.control().is_none_or(|c| !Arc::ptr_eq(&c, control)) {
                return Err(ErrorCode::StaleGeneration);
            }
            let mut writer = session
                .writer
                .try_lock()
                .map_err(|_| ErrorCode::TargetBusy)?;
            let mut observation = control.try_lock().map_err(|_| ErrorCode::TargetBusy)?;
            if observation.human_owned || !observation.authorized() {
                return Err(ErrorCode::ControlRevoked);
            }
            if session.protected_origin(peers) {
                return Err(ErrorCode::ProtectedOriginTerminal);
            }
            if observation.prompt() != lomi_control_core::terminal::Prompt::Ready
                || session.has_foreground_process()
            {
                return Err(ErrorCode::TargetBusy);
            }
            let pid = session.pid.ok_or(ErrorCode::TargetBusy)?;
            let mut child: libc::pid_t = 0;
            let child_count = unsafe {
                libc::proc_listchildpids(
                    pid as libc::pid_t,
                    (&mut child as *mut libc::pid_t).cast(),
                    std::mem::size_of_val(&child) as i32,
                )
            };
            if child_count != 0 {
                return Err(ErrorCode::TargetBusy);
            }
            if !commit {
                return Ok(());
            }
            let mut killer = session
                .killer
                .try_lock()
                .map_err(|_| ErrorCode::TargetBusy)?;
            if !observation.authorized() {
                return Err(ErrorCode::ControlRevoked);
            }
            let mut flow = session.flow.try_lock().map_err(|_| ErrorCode::TargetBusy)?;
            observation.revoke();
            killer.kill().map_err(|_| ErrorCode::OutcomeUnknown)?;
            flow.closed = true;
            session.ready.notify_all();
            writer.take();
            sessions.remove(generation);
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (generation, control, peers, commit);
            Err(ErrorCode::HostUnqualified)
        }
    }

    #[cfg(unix)]
    pub(crate) fn agent_run(
        &self,
        request: lomi_control_core::broker::TerminalDispatchRequest,
    ) -> Result<(), lomi_control_core::terminal_io::WriteFailure> {
        use lomi_control_protocol::ErrorCode;
        let failure = |code| terminal_io::WriteFailure { code, written: 0 };
        let session = self
            .get(&request.generation)
            .map_err(|_| failure(ErrorCode::StaleGeneration))?;
        if session
            .control()
            .is_none_or(|c| !Arc::ptr_eq(&c, &request.control))
        {
            return Err(failure(ErrorCode::StaleGeneration));
        }
        let writer = session
            .writer
            .lock()
            .map_err(|_| failure(ErrorCode::AppUnavailable))?;
        if writer.is_none() {
            return Err(failure(ErrorCode::StaleGeneration));
        }
        if session
            .remote_lease
            .lock()
            .map(|l| {
                l.as_ref()
                    .is_some_and(|l| l.deadline > std::time::Instant::now())
            })
            .unwrap_or(true)
        {
            return Err(failure(ErrorCode::ControlRevoked));
        }
        let permitted = || {
            let peers = (request.permit)().ok_or(ErrorCode::ControlRevoked)?;
            if session.protected_origin(&peers) {
                return Err(ErrorCode::ProtectedOriginTerminal);
            }
            Ok(())
        };
        permitted().map_err(failure)?;
        let bytes = {
            let mut control = request
                .control
                .lock()
                .map_err(|_| failure(ErrorCode::AppUnavailable))?;
            if let Some(target) = &request.interrupt {
                control.prepare_interrupt(&request.lease, target)
            } else {
                control.prepare_run(
                    &request.lease,
                    &request.operation,
                    &request.command,
                    session.has_foreground_process(),
                )
            }
        }
        .map_err(failure)?;
        let fd = session
            .control_fd()
            .map_err(|_| failure(ErrorCode::StaleGeneration))?;
        terminal_io::write(fd, &bytes, Duration::from_secs(2), || permitted().is_ok()).map(|_| ())
    }

    #[cfg(unix)]
    pub(crate) fn agent_input(
        &self,
        request: lomi_control_core::broker::TerminalInputRequest,
    ) -> Result<lomi_control_core::terminal::InputReceipt, lomi_control_protocol::ErrorCode> {
        use lomi_control_core::terminal::InputReceipt;
        use lomi_control_protocol::ErrorCode;
        let session = self
            .get(&request.generation)
            .map_err(|_| ErrorCode::StaleGeneration)?;
        if session
            .control()
            .is_none_or(|c| !Arc::ptr_eq(&c, &request.control))
        {
            return Err(ErrorCode::StaleGeneration);
        }
        let writer = session
            .writer
            .lock()
            .map_err(|_| ErrorCode::AppUnavailable)?;
        if writer.is_none() {
            return Err(ErrorCode::StaleGeneration);
        }
        if session
            .remote_lease
            .lock()
            .map(|l| {
                l.as_ref()
                    .is_some_and(|l| l.deadline > std::time::Instant::now())
            })
            .unwrap_or(true)
        {
            return Err(ErrorCode::ControlRevoked);
        }
        let permitted = || {
            let peers = (request.permit)().ok_or(ErrorCode::ControlRevoked)?;
            if session.protected_origin(&peers) {
                return Err(ErrorCode::ProtectedOriginTerminal);
            }
            Ok(())
        };
        permitted()?;
        if let Some(ack) = request
            .control
            .lock()
            .map_err(|_| ErrorCode::AppUnavailable)?
            .prepare_input(&request.lease, request.sequence, &request.payload)?
        {
            return Ok(ack);
        }
        let result = terminal_io::write(
            session
                .control_fd()
                .map_err(|_| ErrorCode::StaleGeneration)?,
            &request.payload,
            Duration::from_secs(2),
            || permitted().is_ok(),
        );
        request
            .control
            .lock()
            .map_err(|_| ErrorCode::AppUnavailable)?
            .finish_input(request.sequence, result.is_ok());
        Ok(if result.is_ok() {
            InputReceipt::Dispatched
        } else {
            InputReceipt::OutcomeUnknown
        })
    }

    fn write(&self, id: &str, data: &str) -> Result<(), String> {
        self.write_internal(id, data, true)
    }
    fn write_response(&self, id: &str, data: &str) -> Result<(), String> {
        self.write_internal(id, data, false)
    }
    fn write_internal(&self, id: &str, data: &str, human: bool) -> Result<(), String> {
        if data.len() > 256 * 1024 {
            return Err("A terminal input chunk exceeds 256 KiB.".into());
        }
        let session = self.get(id)?;
        let _human_guard = if human {
            if !data.is_empty() {
                self.activity.record();
            }
            session.human_waiters.fetch_add(1, Ordering::SeqCst);
            Some(HumanInputGuard(session.clone()))
        } else {
            None
        };
        if human {
            session.human_generation.fetch_add(1, Ordering::SeqCst);
        }
        if human {
            if let Ok(mut lease) = session.remote_lease.lock() {
                lease.take();
            }
        }
        #[cfg(unix)]
        if let Some(control) = session.control().filter(|_| human) {
            control.lock().map_err(|e| e.to_string())?.manual_input();
        }
        let mut writer = session.writer.lock().map_err(|error| error.to_string())?;
        let writer = writer.as_mut().ok_or("The terminal has been closed.")?;
        // A competing claim may have acquired the writer after the early invalidation.
        if human {
            session.human_generation.fetch_add(1, Ordering::SeqCst);
        }
        if human {
            if let Ok(mut lease) = session.remote_lease.lock() {
                lease.take();
            }
        }
        #[cfg(unix)]
        if let Some(control) = session.control().filter(|_| human) {
            control.lock().map_err(|e| e.to_string())?.manual_input();
        }
        #[cfg(unix)]
        if session.nonblocking() {
            return terminal_io::write(
                session.control_fd()?,
                data.as_bytes(),
                Duration::from_secs(2),
                || true,
            )
            .map(|_| ())
            .map_err(|e| {
                format!(
                    "Terminal input stopped after {} bytes: {:?}",
                    e.written, e.code
                )
            });
        }
        writer
            .write_all(data.as_bytes())
            .and_then(|_| writer.flush())
            .map_err(|error| error.to_string())
    }
}

#[cfg(unix)]
fn process_parents() -> Result<HashSet<u32>, String> {
    let output = shell::quiet_command("/bin/ps")
        .args(["-A", "-o", "ppid=,stat="])
        .env("LC_ALL", "C")
        .output();
    let output = output.map_err(|error| format!("Cannot check terminal processes: {error}"))?;
    if !output.status.success() {
        return Err("Cannot check terminal processes.".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let parent = fields.next()?.parse().ok()?;
            (!fields.next()?.starts_with('Z')).then_some(parent)
        })
        .collect())
}

#[cfg(windows)]
fn process_parents() -> Result<HashSet<u32>, String> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::{ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
    };

    // A native snapshot avoids starting PowerShell and WMI for every close check.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(format!(
            "Cannot check terminal processes: {}",
            std::io::Error::last_os_error()
        ));
    }
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut parents = HashSet::new();
    let mut found = unsafe { Process32FirstW(snapshot.as_raw_handle(), &mut entry) };
    while found != 0 {
        let length = entry
            .szExeFile
            .iter()
            .position(|&character| character == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..length]);
        if !name.eq_ignore_ascii_case("conhost.exe")
            && !name.eq_ignore_ascii_case("OpenConsole.exe")
        {
            parents.insert(entry.th32ParentProcessID);
        }
        found = unsafe { Process32NextW(snapshot.as_raw_handle(), &mut entry) };
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() != Some(ERROR_NO_MORE_FILES as i32) {
        return Err(format!("Cannot check terminal processes: {error}"));
    }
    Ok(parents)
}

#[tauri::command]
pub async fn busy_terminals(
    window: Window,
    state: State<'_, Terminals>,
    ids: Vec<String>,
) -> Result<Vec<String>, String> {
    main_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.busy(&ids))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn start_terminal(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Terminals>,
    shells: State<'_, Shells>,
    request: StartRequest,
    output: Channel<Response>,
    exited: Channel<Exit>,
) -> Result<Started, String> {
    main_window(&window)?;

    validate_start_request(&request)?;
    let state = state.inner().clone();
    let shells = shells.inner().clone();
    #[cfg(unix)]
    let broker = app.state::<crate::agent_control::Control>().current()?;
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(unix)]
        {
            if let Some(ticket) = &request.agent_ticket {
                let broker = broker.ok_or("Agent control is unavailable.")?;
                let profile = crate::agent_control::qualified_profile(&shells, &request.profile_id)
                    .ok_or("This shell is not qualified for agent control.")?;
                let operation = ticket.operation_id.clone();
                let nonce = ticket.nonce.clone();
                let generation = request.id.clone();
                let cwd = request.cwd.clone();
                if request.profile_id != profile.id {
                    return Err("Shell profile changed.".into());
                }
                return broker.start_terminal(
                    &operation,
                    &nonce,
                    &generation,
                    &profile,
                    &cwd,
                    |monitor| state.start_control(&shells, request, output, exited, Some(monitor)),
                );
            }
            if broker.is_some_and(|broker| broker.reserved_terminal(&request.id)) {
                return Err("Agent terminal requires its native start ticket.".into());
            }
        }
        #[cfg(not(unix))]
        if request.agent_ticket.is_some() {
            return Err("Agent terminals are not supported on this host.".into());
        }
        state.start(&shells, request, output, exited)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn write_terminal(
    window: Window,
    state: State<'_, Terminals>,
    id: String,
    data: String,
) -> Result<(), String> {
    main_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.write(&id, &data))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn acknowledge_terminal(
    window: Window,
    state: State<'_, Terminals>,
    id: String,
    bytes: usize,
) -> Result<(), String> {
    main_window(&window)?;
    state.acknowledge(&id, bytes);
    Ok(())
}

#[tauri::command]
pub async fn write_terminal_response(
    window: Window,
    state: State<'_, Terminals>,
    id: String,
    data: String,
) -> Result<(), String> {
    main_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.write_response(&id, &data))
        .await
        .map_err(|_| "Terminal response unavailable.")?
}

#[tauri::command]
pub async fn resize_terminal(
    window: Window,
    state: State<'_, Terminals>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    main_window(&window)?;
    let session = state.get(&id)?;
    let size = size(cols, rows)?;
    let sink = state.remote_sink.lock().ok().and_then(|s| s.clone());
    // ConPTY resize is synchronous and must not block the native event loop.
    tauri::async_runtime::spawn_blocking(move || session.resize(id, size, sink))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn close_terminal(
    window: Window,
    state: State<'_, Terminals>,
    id: String,
) -> Result<(), String> {
    main_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.close(&id))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn reset_terminals(window: Window, state: State<'_, Terminals>) -> Result<(), String> {
    main_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.stop_all();
        Ok::<(), String>(())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn quote_paths(
    window: Window,
    shells: State<'_, Shells>,
    profile_id: String,
    paths: Vec<String>,
) -> Result<String, String> {
    main_window(&window)?;
    let profile = shells
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .cloned()
        .ok_or("Unknown terminal environment.")?;
    tauri::async_runtime::spawn_blocking(move || {
        paths
            .iter()
            .map(|path| {
                let path = if let Some(distro) = &profile.distro {
                    shell::wsl_path(distro, path)?
                } else {
                    path.clone()
                };
                shell::quote(&path, &profile.kind)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|paths| paths.join(" "))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TerminalContext {
    cwd: Option<String>,
    foreground_program: Option<String>,
    title_cli: Option<crate::cli_titles::TitleProcess>,
    agent_controlled: bool,
}

#[tauri::command]
pub async fn take_terminal_control(
    window: Window,
    state: State<'_, Terminals>,
    id: String,
) -> Result<(), String> {
    main_window(&window)?;
    state.remote_revoke(&id);
    let session = state.get(&id)?;
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(unix)]
        if let Some(control) = session.control() {
            control
                .lock()
                .map_err(|_| "Terminal control unavailable")?
                .manual_input();
        }
        #[cfg(not(unix))]
        let _ = session;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

impl Session {
    fn context(&self) -> TerminalContext {
        #[allow(unused_mut)]
        let mut context = TerminalContext::default();
        #[cfg(unix)]
        {
            context.agent_controlled = self
                .control()
                .and_then(|c| c.lock().ok().map(|c| c.lease().is_some()))
                .unwrap_or(false);
        }
        #[cfg(target_os = "linux")]
        if let Some(pid) = self.pid {
            context.cwd = std::fs::read_link(format!("/proc/{pid}/cwd"))
                .ok()
                .map(|path| path.to_string_lossy().into_owned());
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            context.foreground_program = self.foreground_program();
            context.title_cli = self.title_process();
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let _ = &self.pid;
        context
    }
}

impl Terminals {
    fn contexts_with(
        &self,
        mut inspect: impl FnMut(&Session) -> TerminalContext,
    ) -> Result<HashMap<String, TerminalContext>, String> {
        let sessions: HashMap<_, _> = self
            .sessions
            .lock()
            .map_err(|error| error.to_string())?
            .iter()
            .filter(|(_, session)| session.profile.distro.is_none())
            .map(|(id, session)| (id.clone(), session.clone()))
            .collect();
        let mut contexts = HashMap::new();
        for (id, session) in &sessions {
            let context = inspect(session);
            if !session
                .flow
                .lock()
                .map_err(|error| error.to_string())?
                .closed
            {
                contexts.insert(id.clone(), context);
            }
        }
        // A close or restart can replace a session while process inspection runs.
        // Only publish contexts for the exact sessions still in the registry.
        let current = self.sessions.lock().map_err(|error| error.to_string())?;
        contexts.retain(|id, _| {
            current.get(id).is_some_and(|live| {
                sessions
                    .get(id)
                    .is_some_and(|snapshot| Arc::ptr_eq(live, snapshot))
            })
        });
        Ok(contexts)
    }
}

#[tauri::command]
pub async fn terminal_contexts(
    window: Window,
    state: State<'_, Terminals>,
) -> Result<HashMap<String, TerminalContext>, String> {
    main_window(&window)?;
    let state = state.inner().clone();
    // Process discovery and CLI title reads can be slow; keep them off the
    // native event loop and outside the terminal registry lock.
    tauri::async_runtime::spawn_blocking(move || state.contexts_with(Session::context))
        .await
        .map_err(|error| error.to_string())?
}

#[cfg(all(test, unix))]
mod local_runtime_tests {
    use super::*;
    use std::time::Instant;
    use tauri::ipc::InvokeResponseBody;

    enum Event {
        Output(Vec<u8>),
        Exited(Option<u32>),
    }

    struct LocalTerminal {
        terminals: Terminals,
        events: Receiver<Event>,
        _directory: tempfile::TempDir,
    }

    impl Drop for LocalTerminal {
        fn drop(&mut self) {
            self.terminals.stop_all();
        }
    }

    fn start_local_shell() -> LocalTerminal {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let cwd = root.to_str().unwrap().to_owned();
        let profile = Profile {
            id: "local-test-shell".into(),
            name: "Local test shell".into(),
            kind: "sh".into(),
            program: "/bin/sh".into(),
            distro: None,
            home: cwd.clone(),
        };
        let shells = Shells {
            profiles: vec![profile],
            integration: root.join("shell-integration"),
        };
        let (sender, events) = mpsc::channel();
        let output_sender = sender.clone();
        let output = Channel::<Response>::new(move |body| {
            let InvokeResponseBody::Raw(bytes) = body else {
                panic!("Terminal output must remain binary");
            };
            let _ = output_sender.send(Event::Output(bytes));
            Ok(())
        });
        let exited = Channel::<Exit>::new(move |body| {
            let InvokeResponseBody::Json(json) = body else {
                panic!("Terminal exit must carry its status");
            };
            let value: serde_json::Value = serde_json::from_str(&json).unwrap();
            let code = value["code"].as_u64().map(|code| code as u32);
            let _ = sender.send(Event::Exited(code));
            Ok(())
        });
        let terminals = Terminals::default();
        assert!(terminals.remote_sink.lock().unwrap().is_none());
        let started = terminals
            .start_control(
                &shells,
                StartRequest {
                    id: "local-terminal".into(),
                    profile_id: "local-test-shell".into(),
                    cwd: cwd.clone(),
                    cols: 80,
                    rows: 24,
                    agent_ticket: None,
                    cli_launch: None,
                },
                output,
                exited,
                None,
            )
            .unwrap();
        assert_eq!(started.cwd, cwd);
        assert_eq!(started.profile_id, "local-test-shell");
        LocalTerminal {
            terminals,
            events,
            _directory: directory,
        }
    }

    fn drain(terminal: &LocalTerminal) -> (Vec<u8>, Option<u32>) {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut output = Vec::new();
        let mut eof = false;
        let mut exit = None;
        loop {
            let event = terminal
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("The local PTY did not finish");
            match event {
                Event::Output(bytes) if bytes.is_empty() => eof = true,
                Event::Output(bytes) => {
                    assert!(!eof, "Output arrived after EOF");
                    terminal
                        .terminals
                        .acknowledge("local-terminal", bytes.len());
                    output.extend(bytes);
                    assert!(output.len() < 1024 * 1024);
                }
                Event::Exited(code) => {
                    assert!(exit.is_none(), "The exit status was sent twice");
                    exit = Some(code);
                }
            }
            if eof {
                if let Some(code) = exit {
                    return (output, code);
                }
            }
        }
    }

    #[test]
    fn local_pty_streams_input_resizes_and_drains_without_a_host() {
        let terminal = start_local_shell();
        assert!(terminal
            .terminals
            .contexts_with(Session::context)
            .unwrap()
            .contains_key("local-terminal"));
        terminal
            .terminals
            .get("local-terminal")
            .unwrap()
            .resize("local-terminal".into(), size(101, 41).unwrap(), None)
            .unwrap();
        terminal
            .terminals
            .write(
                "local-terminal",
                "stty size; printf '\\114\\117\\115\\111\\055\\114\\117\\103\\101\\114\\055\\117\\113\\n'; i=0; while [ \"$i\" -lt 5000 ]; do printf '0123456789abcdef0123456789abcdef\\n'; i=$((i+1)); done; exit 7\n",
            )
            .unwrap();
        let (bytes, code) = drain(&terminal);
        let output = String::from_utf8_lossy(&bytes);
        assert!(output.contains("41 101"));
        assert!(output.contains("LOMI-LOCAL-OK"));
        assert!(bytes.len() > HIGH_WATER);
        assert_eq!(code, Some(7));
        assert!(terminal.terminals.get("local-terminal").is_err());
        assert!(terminal.terminals.remote_sink.lock().unwrap().is_none());
    }

    #[test]
    fn closing_a_local_terminal_retires_its_session_without_host_ipc() {
        let terminal = start_local_shell();
        terminal.terminals.close("local-terminal").unwrap();
        let _ = drain(&terminal);
        assert!(terminal.terminals.get("local-terminal").is_err());
        assert!(terminal.terminals.sessions.lock().unwrap().is_empty());
    }
}
