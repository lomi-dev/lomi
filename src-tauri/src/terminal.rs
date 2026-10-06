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
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc, Condvar, Mutex,
    },
    thread,
};
use tauri::{
    ipc::{Channel, Response},
    Manager, State, Window,
};

#[cfg(unix)]
use lomi_control_core::{terminal::TerminalControl, terminal_io};
#[cfg(unix)]
use std::{
    os::fd::BorrowedFd,
    time::{Duration, Instant},
};

const HIGH_WATER: usize = 128 * 1024;

pub(crate) type FinalNativeAdmission = Box<dyn FnOnce() -> Result<(), String> + Send>;
pub(crate) struct NativeTerminalLaunch {
    pub command: (portable_pty::CommandBuilder, String),
    pub admission: FinalNativeAdmission,
    #[cfg(target_os = "macos")]
    pub owned_boundary: Option<crate::agent_runtime::native_launch::PtyFactory>,
}
struct PreparedTerminal {
    command: (portable_pty::CommandBuilder, String),
    completion: Arc<AtomicBool>,
    drain_healthy: Option<Arc<AtomicBool>>,
    admission: Option<FinalNativeAdmission>,
    #[cfg(target_os = "macos")]
    owned_boundary: Option<crate::agent_runtime::native_launch::PtyFactory>,
}

#[cfg(unix)]
#[cfg(all(test, unix))]
struct OwnedAccountTerminal {
    command: (portable_pty::CommandBuilder, String),
    completion: Arc<AtomicBool>,
    drain_healthy: Arc<AtomicBool>,
}

#[cfg(target_os = "macos")]
fn account_group_contains_only_zombies(group: u32) -> bool {
    // Darwin killpg returns EPERM if a retained group has no signalable live
    // member. Admit that case only with two complete, stable native snapshots
    // and an exact zombie status for every member, never for an unknown PID.
    // proc_pidinfo's arg=1 includes zombies (XNU proc_info.c).
    fn members(group: u32) -> Option<Vec<i32>> {
        let mut pids = [0i32; 1024];
        let capacity = std::mem::size_of_val(&pids) as i32;
        let count = unsafe {
            libc::proc_listpids(
                2, /* PROC_PGRP_ONLY */
                group,
                pids.as_mut_ptr().cast(),
                capacity,
            )
        };
        if count <= 0 || count >= capacity || count % 4 != 0 {
            return None;
        }
        let mut result = pids[..count as usize / 4].to_vec();
        if result.iter().any(|pid| *pid <= 0) {
            return None;
        }
        result.sort_unstable();
        Some(result)
    }
    let Some(pids) = members(group) else {
        return false;
    };
    for pid in &pids {
        let mut info = std::mem::MaybeUninit::<libc::proc_bsdshortinfo>::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdshortinfo>() as i32;
        let read = unsafe {
            libc::proc_pidinfo(
                *pid,
                libc::PROC_PIDT_SHORTBSDINFO,
                1,
                info.as_mut_ptr().cast(),
                size,
            )
        };
        if read != size {
            return false;
        }
        let info = unsafe { info.assume_init() };
        if info.pbsi_pid != *pid as u32
            || info.pbsi_pgid != group
            || info.pbsi_status != libc::SZOMB
        {
            return false;
        }
    }
    members(group).is_some_and(|current| current == pids)
}

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
    owned_account: bool,
    owned_boundary: bool,
    completion: Option<Arc<AtomicBool>>,
    transport_finished: AtomicBool,
    account_reap_started: Mutex<bool>,
    account_kill_sent: AtomicBool,
    account_signal_failed: AtomicBool,
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
                            0,
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

    #[cfg(unix)]
    fn kill_account_group(&self, reap_started: bool) {
        if self.owned_boundary {
            if self.account_kill_sent.load(Ordering::SeqCst) {
                return;
            }
            match self
                .killer
                .lock()
                .map_err(|_| ())
                .and_then(|mut killer| killer.kill().map_err(|_| ()))
            {
                Ok(()) => {
                    self.account_signal_failed.store(false, Ordering::SeqCst);
                    self.account_kill_sent.store(true, Ordering::SeqCst);
                }
                Err(()) => self.account_signal_failed.store(true, Ordering::SeqCst),
            }
            return;
        }
        // Once a group-wide SIGKILL succeeds, its members cannot create new
        // descendants. Do not signal again after reaping begins or after a
        // successful kill: Darwin can retain an exiting child until the PTY
        // descriptors close, and PID ownership ends when wait reaps it.
        if reap_started || self.account_kill_sent.load(Ordering::SeqCst) {
            return;
        }
        let Some(pid) = self.pid else {
            self.account_signal_failed.store(true, Ordering::SeqCst);
            return;
        };
        let killed = unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
        let error = if killed != 0 {
            std::io::Error::last_os_error().raw_os_error()
        } else {
            None
        };
        #[cfg(target_os = "macos")]
        let only_zombies = error == Some(libc::EPERM) && account_group_contains_only_zombies(pid);
        #[cfg(not(target_os = "macos"))]
        let only_zombies = false;
        if killed == 0 || error == Some(libc::ESRCH) || only_zombies {
            self.account_kill_sent.store(true, Ordering::SeqCst);
        } else {
            self.account_signal_failed.store(true, Ordering::SeqCst);
        }
    }

    fn stop(&self) {
        #[cfg(unix)]
        if self.owned_account {
            // portable-pty creates an owned session/process group. Fence its
            // descendants before the leader can be reaped and its PID reused.
            if let Ok(reap_started) = self.account_reap_started.lock() {
                self.kill_account_group(*reap_started);
            }
        }
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
        if !self.owned_account {
            if let Ok(mut killer) = self.killer.lock() {
                let _ = killer.kill();
            }
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
    activity: crate::remote::activity::Activity,
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
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
        let mut observed = RemoteObservedEvent { event, generation };
        loop {
            match self.sender.try_send(observed) {
                Ok(()) => return,
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    self.healthy.store(false, Ordering::SeqCst);
                    return;
                }
                Err(mpsc::TrySendError::Full(event)) => observed = event,
            }
            if std::time::Instant::now() >= deadline {
                // Local PTY readers must keep draining even when the helper can
                // never restart. Only this terminal's Remote model loses fidelity.
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
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
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
    pub(crate) activity: crate::remote::activity::Activity,
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
    #[serde(default)]
    account_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentTicket {
    operation_id: String,
    nonce: String,
}

fn validate_start_request(request: &StartRequest) -> Result<(), String> {
    if request.account_id.is_some()
        && (request.agent_ticket.is_some() || request.cli_launch.is_some())
    {
        return Err(
            "A native account terminal resolves its own CLI and cannot combine another launch."
                .into(),
        );
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_label: Option<String>,
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

    #[cfg(any(feature = "native-smoke", feature = "mcp-probe"))]
    pub fn smoke_sessions(&self) -> serde_json::Value {
        serde_json::json!(self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .map(|(id, session)| (id.clone(), session.pid))
            .collect::<std::collections::BTreeMap<_, _>>())
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

    pub(crate) fn stop_owned_and_wait(&self) -> Result<(), String> {
        let sessions: Vec<_> = self
            .sessions
            .lock()
            .map_err(|_| "Owned terminal state is unavailable.")?
            .values()
            .filter(|session| session.owned_boundary)
            .cloned()
            .collect();
        Self::stop_owned_sessions(&sessions)
    }

    fn stop_owned_sessions(sessions: &[Arc<Session>]) -> Result<(), String> {
        // Keep the registered views until each owned killer proves retirement.
        // Never hold the terminal registry or runtime owner lock across Stop.
        for session in sessions {
            #[cfg(unix)]
            {
                let reap_started = session
                    .account_reap_started
                    .lock()
                    .map_err(|_| "Owned terminal retirement state is unavailable.")?;
                session.kill_account_group(*reap_started);
            }
            if !session.account_kill_sent.load(Ordering::SeqCst)
                || session.account_signal_failed.load(Ordering::SeqCst)
            {
                return Err("An owned login terminal has not proved retirement.".into());
            }
            session.stop();
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        while sessions
            .iter()
            .any(|session| !session.transport_finished.load(Ordering::SeqCst))
        {
            if Instant::now() >= deadline {
                return Err("Owned login terminal transport has not finished draining.".into());
            }
            thread::sleep(Duration::from_millis(5));
        }
        for session in sessions {
            session
                .completion
                .as_ref()
                .ok_or("Owned login completion state is unavailable.")?
                .store(true, Ordering::SeqCst);
        }
        // Keep owned views until durable helper finalization also succeeds.
        // Explicit close or the successful application's Exit removes them.
        Ok(())
    }

    pub fn close(&self, id: &str) -> Result<(), String> {
        let session = self
            .sessions
            .lock()
            .map_err(|_| "Terminal state is unavailable.")?
            .get(id)
            .cloned();
        if let Some(session) = session {
            if session.owned_boundary {
                Self::stop_owned_sessions(std::slice::from_ref(&session))?;
            } else {
                session.stop();
            }
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
        self.start_prepared(
            shells,
            request,
            output,
            exited,
            #[cfg(unix)]
            control,
            None,
        )
    }

    fn start_account(
        &self,
        shells: &Shells,
        request: StartRequest,
        output: Channel<Response>,
        exited: Channel<Exit>,
        launch: NativeTerminalLaunch,
        completion: Arc<AtomicBool>,
    ) -> Result<Started, String> {
        self.start_prepared(
            shells,
            request,
            output,
            exited,
            #[cfg(unix)]
            None,
            Some(PreparedTerminal {
                command: launch.command,
                completion,
                drain_healthy: None,
                admission: Some(launch.admission),
                #[cfg(target_os = "macos")]
                owned_boundary: launch.owned_boundary,
            }),
        )
    }

    #[cfg(feature = "native-smoke")]
    pub(crate) fn start_owned_smoke(
        &self,
        app: &tauri::AppHandle,
        launch: NativeTerminalLaunch,
        completion: Arc<AtomicBool>,
    ) -> Result<String, String> {
        let shells = app.state::<Shells>();
        let profile_id = shells
            .profiles
            .first()
            .ok_or("No smoke shell profile is available.")?
            .id
            .clone();
        let id = format!("owned-smoke-{}", crate::agent_runtime::new_id()?);
        let cwd = launch.command.1.clone();
        self.start_account(
            &shells,
            StartRequest {
                id: id.clone(),
                profile_id,
                cwd,
                cols: 80,
                rows: 24,
                agent_ticket: None,
                account_id: Some("offline-owned-smoke".into()),
                cli_launch: None,
            },
            Channel::new(|_| Ok(())),
            Channel::new(|_| Ok(())),
            launch,
            completion,
        )?;
        Ok(id)
    }

    #[cfg(all(test, unix))]
    fn start_owned_account(
        &self,
        shells: &Shells,
        request: StartRequest,
        output: Channel<Response>,
        exited: Channel<Exit>,
        launch: OwnedAccountTerminal,
    ) -> Result<Started, String> {
        self.start_prepared(
            shells,
            request,
            output,
            exited,
            None,
            Some(PreparedTerminal {
                command: launch.command,
                completion: launch.completion,
                drain_healthy: Some(launch.drain_healthy),
                admission: None,
                #[cfg(target_os = "macos")]
                owned_boundary: None,
            }),
        )
    }

    fn start_prepared(
        &self,
        shells: &Shells,
        request: StartRequest,
        output: Channel<Response>,
        exited: Channel<Exit>,
        #[cfg(unix)] control: Option<Arc<Mutex<TerminalControl>>>,
        prepared: Option<PreparedTerminal>,
    ) -> Result<Started, String> {
        #[cfg(target_os = "macos")]
        let mut owned_boundary = None;
        let (prepared, completion, drain_healthy, admission) = match prepared {
            Some(PreparedTerminal {
                command,
                completion,
                drain_healthy,
                admission,
                #[cfg(target_os = "macos")]
                    owned_boundary: factory,
            }) => {
                #[cfg(target_os = "macos")]
                {
                    owned_boundary = factory;
                }
                (Some(command), Some(completion), drain_healthy, admission)
            }
            None => (None, None, None, None),
        };
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
        let (command, cwd) = if let Some(prepared) = prepared {
            prepared
        } else if let Some(cli) = request.cli_launch {
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
        #[cfg(target_os = "macos")]
        let is_owned_boundary = owned_boundary.is_some();
        #[cfg(not(target_os = "macos"))]
        let is_owned_boundary = false;
        if let Some(admission) = admission {
            admission()?;
        }
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
        #[cfg(target_os = "macos")]
        let owned = owned_boundary
            .map(|factory| factory(size(request.cols, request.rows)?))
            .transpose()?;
        #[cfg(not(target_os = "macos"))]
        let owned: Option<(
            Box<dyn MasterPty + Send>,
            Box<dyn portable_pty::Child + Send + Sync>,
        )> = None;
        #[cfg(target_os = "macos")]
        let owned_read_fd = owned.as_ref().map(|owned| owned.read_fd);
        #[cfg(not(target_os = "macos"))]
        let owned_read_fd: Option<i32> = None;
        #[cfg(target_os = "macos")]
        let owned = owned.map(|owned| (owned.master, owned.child));
        let (master, mut child) = if let Some(owned) = owned {
            owned
        } else {
            let pair = native_pty_system()
                .openpty(size(request.cols, request.rows)?)
                .map_err(|error| error.to_string())?;
            let child = pair
                .slave
                .spawn_command(command)
                .map_err(|error| format!("Cannot start {}: {error}", profile.name))?;
            (pair.master, child)
        };
        #[cfg(unix)]
        if control.is_some() || request.account_id.is_some() {
            let fd = master.as_raw_fd().ok_or("This PTY cannot be controlled.")?;
            terminal_io::make_nonblocking(unsafe { BorrowedFd::borrow_raw(fd) })
                .map_err(|e| e.to_string())?;
        }
        let mut reader = master
            .try_clone_reader()
            .map_err(|error| error.to_string())?;
        let writer = master.take_writer().map_err(|error| error.to_string())?;
        let session = Arc::new(Session {
            owned_account: request.account_id.is_some(),
            owned_boundary: is_owned_boundary,
            completion: completion.clone(),
            transport_finished: AtomicBool::new(false),
            account_reap_started: Mutex::new(false),
            account_kill_sent: AtomicBool::new(false),
            account_signal_failed: AtomicBool::new(false),
            pid: child.process_id(),
            profile: profile.clone(),
            master: Mutex::new(master),
            writer: Mutex::new(Some(writer)),
            killer: Mutex::new(child.clone_killer()),
            flow: Mutex::new(Flow {
                #[cfg(unix)]
                nonblocking: control.is_some() || request.account_id.is_some(),
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
                if session.owned_account {
                    *session
                        .account_reap_started
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = true;
                }
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
        let session_retirement = Arc::downgrade(&session);
        thread::spawn(move || {
            let mut buffer = [0_u8; 16 * 1024];
            let mut clean_drain = true;
            #[cfg(unix)]
            let mut account_tail: Option<(Instant, usize)> = None;
            'read: loop {
                #[cfg(unix)]
                if session.owned_account && !session.owned_boundary {
                    if let Some(pid) = session.pid {
                        let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
                        let observed = unsafe {
                            libc::waitid(
                                libc::P_PID,
                                pid,
                                info.as_mut_ptr(),
                                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                            )
                        };
                        if observed != 0 {
                            clean_drain = false;
                        }
                        if (observed != 0 || unsafe { info.assume_init().si_pid() } != 0)
                            && account_tail.is_none()
                        {
                            let reap_started = session
                                .account_reap_started
                                .lock()
                                .unwrap_or_else(|error| error.into_inner());
                            session.kill_account_group(*reap_started);
                            account_tail = Some((Instant::now() + Duration::from_secs(5), 0));
                        }
                    }
                    if account_tail.is_some_and(|(deadline, bytes)| {
                        Instant::now() >= deadline || bytes > 1024 * 1024
                    }) {
                        clean_drain = false;
                        break;
                    }
                }
                let mut flow = session
                    .flow
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                #[cfg(unix)]
                let draining_tail = account_tail.is_some();
                #[cfg(not(unix))]
                let draining_tail = false;
                while !flow.closed && flow.pending >= HIGH_WATER && !draining_tail {
                    #[cfg(unix)]
                    if session.owned_account {
                        let (next, timeout) = session
                            .ready
                            .wait_timeout(flow, Duration::from_millis(50))
                            .unwrap_or_else(|error| error.into_inner());
                        flow = next;
                        if timeout.timed_out() {
                            continue 'read;
                        }
                        continue;
                    }
                    flow = session
                        .ready
                        .wait(flow)
                        .unwrap_or_else(|error| error.into_inner());
                }
                if flow.closed && !session.owned_account {
                    break;
                }
                #[cfg(unix)]
                if flow.closed && session.owned_account && account_tail.is_none() {
                    account_tail = Some((Instant::now() + Duration::from_secs(5), 0));
                }
                drop(flow);
                let length = match reader.read(&mut buffer) {
                    #[cfg(unix)]
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && session.nonblocking() =>
                    {
                        if let Some(read_fd) = owned_read_fd {
                            let fd = unsafe { BorrowedFd::borrow_raw(read_fd) };
                            if terminal_io::wait_readable(fd).is_err() {
                                clean_drain = false;
                                break;
                            }
                            continue;
                        }
                        let Ok(fd) = session.control_fd() else {
                            clean_drain = false;
                            break;
                        };
                        if terminal_io::wait_readable(fd).is_err() {
                            clean_drain = false;
                            break;
                        }
                        continue;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Ok(0) => break,
                    #[cfg(unix)]
                    Err(error)
                        if session.owned_account && error.raw_os_error() == Some(libc::EIO) =>
                    {
                        break
                    }
                    Err(_) => {
                        clean_drain = false;
                        break;
                    }
                    Ok(length) => length,
                };
                #[cfg(unix)]
                if let Some((_, bytes)) = &mut account_tail {
                    *bytes = bytes.saturating_add(length);
                }
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
                if output
                    .send(Response::new(buffer[..length].to_vec()))
                    .is_err()
                {
                    session.stop();
                    clean_drain = false;
                    break;
                }
            }
            drop(reader);
            #[cfg(unix)]
            let exit_control = session.control();
            let owned_account = session.owned_account;
            let owned_boundary = session.owned_boundary;
            if owned_boundary {
                let deadline = Instant::now() + Duration::from_millis(500);
                while Instant::now() < deadline {
                    match child.try_wait() {
                        Ok(Some(_)) | Err(_) => break,
                        Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                    }
                }
            }
            let mut retired_owned_boundary = false;
            let mut retained_session = Some(session);
            if owned_account {
                let session = retained_session.as_ref().unwrap();
                {
                    let mut reap_started = session
                        .account_reap_started
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    #[cfg(unix)]
                    session.kill_account_group(*reap_started);
                    // Disarm stale Session holders before wait can reuse PID.
                    // Never hold this mutex across a blocking process wait.
                    *reap_started = true;
                }
                clean_drain &= !session.account_signal_failed.load(Ordering::SeqCst);
                retired_owned_boundary = owned_boundary
                    && session.account_kill_sent.load(Ordering::SeqCst)
                    && !session.account_signal_failed.load(Ordering::SeqCst);
                session.stop();
                if let Some(sessions) = sessions.upgrade().filter(|_| !owned_boundary) {
                    if let Ok(mut sessions) = sessions.lock() {
                        if sessions
                            .get(&request.id)
                            .is_some_and(|current| Arc::ptr_eq(current, session))
                        {
                            sessions.remove(&request.id);
                        }
                    }
                }
                // Closing our writer and master after the output tail is
                // drained also lets Darwin finish an exiting PTY child.
                drop(retained_session.take());
            }
            let code = child.wait().ok().map(|status| status.exit_code());
            if let Some(session) = session_retirement.upgrade() {
                session.transport_finished.store(true, Ordering::SeqCst);
            }
            if let Some(healthy) = drain_healthy {
                healthy.store(clean_drain && code.is_some(), Ordering::SeqCst);
            }
            if retired_owned_boundary || (!owned_boundary && code.is_some()) {
                if let Some(completion) = completion {
                    completion.store(true, Ordering::SeqCst);
                }
            }
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
            if let Some(session) = retained_session {
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
            }
            let _ = exited.send(Exit { code });
            // An ordered EOF marker keeps the exit notice behind all output chunks.
            let _ = output.send(Response::new(Vec::<u8>::new()));
        });
        Ok(Started {
            cwd,
            profile_id: profile.id,
            account_id: None,
            account_label: None,
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
    let runtime = app
        .state::<crate::agent_runtime::AgentRuntime>()
        .inner()
        .clone();
    #[cfg(unix)]
    let broker = app.state::<crate::agent_control::Control>().current()?;
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(account_id) = request.account_id.clone() {
            let shell_id = request.profile_id.clone();
            let cwd = request.cwd.clone();
            return crate::agent_runtime::account_terminal(
                &runtime,
                &app,
                &shells,
                &shell_id,
                &cwd,
                &account_id,
                |command, completion, (chosen_id, chosen_label)| {
                    let mut started = state
                        .start_account(&shells, request, output, exited, command, completion)?;
                    started.account_id = Some(chosen_id);
                    started.account_label = Some(chosen_label);
                    Ok(started)
                },
            );
        }
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
        state.stop_owned_and_wait()?;
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

#[tauri::command]
pub fn terminal_contexts(
    window: Window,
    state: State<'_, Terminals>,
) -> Result<HashMap<String, TerminalContext>, String> {
    main_window(&window)?;
    let sessions = state.sessions.lock().map_err(|error| error.to_string())?;
    let mut contexts = HashMap::new();
    for (id, session) in sessions.iter() {
        if session.profile.distro.is_some() {
            continue;
        }
        #[allow(unused_mut)]
        let mut context = TerminalContext::default();
        #[cfg(unix)]
        {
            context.agent_controlled = session
                .control()
                .and_then(|c| c.lock().ok().map(|c| c.lease().is_some()))
                .unwrap_or(false);
        }
        #[cfg(target_os = "linux")]
        if let Some(pid) = session.pid {
            context.cwd = std::fs::read_link(format!("/proc/{pid}/cwd"))
                .ok()
                .map(|path| path.to_string_lossy().into_owned());
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            context.foreground_program = session.foreground_program();
            context.title_cli = session.title_process();
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let _ = &session.pid;
        contexts.insert(id.clone(), context);
    }
    Ok(contexts)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    fn account_shell_fixture(directory: &std::path::Path) -> Shells {
        Shells {
            profiles: vec![Profile {
                id: "account-fixture-shell".into(),
                name: "Fixture sh".into(),
                kind: "sh".into(),
                program: "/bin/sh".into(),
                distro: None,
                home: directory.to_string_lossy().into_owned(),
            }],
            integration: directory.to_owned(),
        }
    }
    #[cfg(unix)]
    fn account_request(directory: &std::path::Path, id: &str) -> StartRequest {
        StartRequest {
            id: id.into(),
            profile_id: "account-fixture-shell".into(),
            cwd: directory.to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
            agent_ticket: None,
            account_id: Some("account-fixture".into()),

            cli_launch: None,
        }
    }
    #[cfg(unix)]
    fn account_command(
        directory: &std::path::Path,
        script: &str,
    ) -> (portable_pty::CommandBuilder, String) {
        let mut command = portable_pty::CommandBuilder::new("/bin/sh");
        command.env_clear();
        command.cwd(directory);
        command.arg("-c");
        command.arg(script);
        (command, directory.to_string_lossy().into_owned())
    }
    #[cfg(unix)]
    struct FixtureTerminalCleanup(Terminals);
    #[cfg(unix)]
    impl Drop for FixtureTerminalCleanup {
        fn drop(&mut self) {
            self.0.stop_all();
        }
    }
    #[cfg(unix)]
    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "Owned PTY explicit close retains a failed effect-drain retry"]
    fn owned_close_failure_retains_the_session_until_a_positive_retry() {
        use crate::agent_runtime::native_launch::{self, LaunchContext};
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        for name in ["project", "account", "storage"] {
            let path = root.join(name);
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let context = LaunchContext {
            parent_operation_id: "owned-pty-close-retry".into(),
            account_id: "account-fixture".into(),
            auth_revision: 1,
            task_id: None,
            attempt_id: None,
            generation: None,
            project_root: root.join("project"),
            physical_account_root: root.join("account"),
            storage_root: root.join("storage"),
        };
        let shells = account_shell_fixture(&context.project_root);
        let manager = Terminals::default();
        let done = Arc::new(AtomicBool::new(false));
        let effect = Arc::new(Mutex::new(None));
        let held = effect.clone();
        let launch_context = context.clone();
        let mut portable = portable_pty::CommandBuilder::new("/bin/cat");
        portable.cwd(&context.project_root);
        let launch = NativeTerminalLaunch {
            command: (
                portable,
                context.project_root.to_string_lossy().into_owned(),
            ),
            admission: Box::new(|| Ok(())),
            owned_boundary: Some(Box::new(move |size| {
                let mut command = std::process::Command::new("/bin/cat");
                command
                    .env_clear()
                    .env("HOME", &launch_context.physical_account_root)
                    .current_dir(&launch_context.project_root);
                let owned = native_launch::spawn_pty(
                    command,
                    &launch_context,
                    crate::cli_catalog::TitleCli::Pi,
                    None,
                    &[],
                    &[],
                    &AtomicBool::new(false),
                    size,
                    || Ok(()),
                )?;
                *held.lock().unwrap() = Some(owned.effects.enter()?);
                Ok(owned)
            })),
        };
        manager
            .start_account(
                &shells,
                account_request(&context.project_root, "owned-close-retry"),
                Channel::new(|_| Ok(())),
                Channel::new(|_| Ok(())),
                launch,
                done.clone(),
            )
            .unwrap();
        assert!(manager.close("owned-close-retry").is_err());
        assert!(manager.get("owned-close-retry").is_ok());
        assert!(!done.load(Ordering::SeqCst));
        drop(effect.lock().unwrap().take());
        manager.close("owned-close-retry").unwrap();
        assert!(done.load(Ordering::SeqCst));
        assert!(manager.get("owned-close-retry").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn quit_owned_drain_preserves_ordinary_terminal_sessions() {
        let directory = tempfile::tempdir().unwrap();
        let shells = account_shell_fixture(directory.path());
        let manager = Terminals::default();
        let _cleanup = FixtureTerminalCleanup(manager.clone());
        let done = Arc::new(AtomicBool::new(false));
        manager
            .start_owned_account(
                &shells,
                account_request(directory.path(), "ordinary-quit-fixture"),
                Channel::new(|_| Ok(())),
                Channel::new(|_| Ok(())),
                OwnedAccountTerminal {
                    command: account_command(directory.path(), "sleep 60"),
                    completion: done.clone(),
                    drain_healthy: Arc::new(AtomicBool::new(false)),
                },
            )
            .unwrap();
        manager.stop_owned_and_wait().unwrap();
        let session = manager.get("ordinary-quit-fixture").unwrap();
        assert!(!session.owned_boundary);
        assert_eq!(unsafe { libc::kill(session.pid.unwrap() as i32, 0) }, 0);
        assert!(!done.load(Ordering::SeqCst));
    }

    #[cfg(unix)]
    #[test]
    fn owned_account_fast_exit_delivers_final_tail_before_healthy_completion() {
        let directory = tempfile::tempdir().unwrap();
        let shells = account_shell_fixture(directory.path());
        let manager = Terminals::default();
        let _cleanup = FixtureTerminalCleanup(manager.clone());
        let done = Arc::new(AtomicBool::new(false));
        let healthy = Arc::new(AtomicBool::new(false));
        let (send, receive) = mpsc::channel();
        let ack = manager.clone();
        let output = Channel::new(move |body| {
            if let tauri::ipc::InvokeResponseBody::Raw(bytes) = body {
                ack.acknowledge("account-tail-fixture", bytes.len());
                let _ = send.send(bytes);
            }
            Ok(())
        });
        manager
            .start_owned_account(
                &shells,
                account_request(directory.path(), "account-tail-fixture"),
                output,
                Channel::new(|_| Ok(())),
                OwnedAccountTerminal {
                    command: account_command(
                        directory.path(),
                        "printf '__owned_account_final_tail__\\n'",
                    ),
                    completion: done.clone(),
                    drain_healthy: healthy.clone(),
                },
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut bytes = Vec::new();
        let mut eof = false;
        while Instant::now() < deadline && !eof {
            if let Ok(chunk) = receive.recv_timeout(Duration::from_millis(100)) {
                eof = chunk.is_empty();
                bytes.extend(chunk);
            }
        }
        assert!(eof, "Owned PTY did not deliver its ordered EOF marker");
        assert!(String::from_utf8_lossy(&bytes).contains("__owned_account_final_tail__"));
        assert!(done.load(Ordering::SeqCst));
        assert!(healthy.load(Ordering::SeqCst));
    }
    #[cfg(unix)]
    #[test]
    fn owned_account_cancel_drains_group_and_output_failure_is_unhealthy() {
        let directory = tempfile::tempdir().unwrap();
        let shells = account_shell_fixture(directory.path());
        for failed_channel in [false, true] {
            let manager = Terminals::default();
            let _cleanup = FixtureTerminalCleanup(manager.clone());
            let done = Arc::new(AtomicBool::new(false));
            let healthy = Arc::new(AtomicBool::new(false));
            let id = if failed_channel {
                "account-output-failure"
            } else {
                "account-cancel-fixture"
            };
            let (send, receive) = mpsc::channel();
            let ack = manager.clone();
            let output = Channel::new(move |body| {
                if let tauri::ipc::InvokeResponseBody::Raw(bytes) = body {
                    if failed_channel {
                        return Err(tauri::Error::Io(std::io::Error::new(
                            std::io::ErrorKind::BrokenPipe,
                            "fixture output channel failure",
                        )));
                    }
                    ack.acknowledge(id, bytes.len());
                    let _ = send.send(bytes);
                }
                Ok(())
            });
            manager
                .start_owned_account(
                    &shells,
                    account_request(directory.path(), id),
                    output,
                    Channel::new(|_| Ok(())),
                    OwnedAccountTerminal {
                        command: account_command(
                            directory.path(),
                            "printf '__owned_account_ready__\\n'; exec /bin/sleep 30",
                        ),
                        completion: done.clone(),
                        drain_healthy: healthy.clone(),
                    },
                )
                .unwrap();
            let mut pid = None;
            if !failed_channel {
                let ready_deadline = Instant::now() + Duration::from_secs(5);
                let mut ready = Vec::new();
                while Instant::now() < ready_deadline
                    && !String::from_utf8_lossy(&ready).contains("__owned_account_ready__")
                {
                    if let Ok(chunk) = receive.recv_timeout(Duration::from_millis(100)) {
                        ready.extend(chunk);
                    }
                }
                assert!(String::from_utf8_lossy(&ready).contains("__owned_account_ready__"));
                pid = manager.get(id).unwrap().pid;
                #[cfg(target_os = "macos")]
                assert!(!account_group_contains_only_zombies(pid.unwrap()));
                manager.close(id).unwrap();
            }
            let deadline = Instant::now() + Duration::from_secs(8);
            while !done.load(Ordering::SeqCst) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            assert!(
                done.load(Ordering::SeqCst),
                "Owned PTY group did not drain within its bound"
            );
            assert_eq!(healthy.load(Ordering::SeqCst), !failed_channel);
            if let Some(pid) = pid {
                assert_eq!(unsafe { libc::kill(-(pid as i32), 0) }, -1);
                assert_eq!(
                    std::io::Error::last_os_error().raw_os_error(),
                    Some(libc::ESRCH)
                );
            }
        }
    }
    #[test]
    fn rejects_invalid_terminal_sizes() {
        assert!(size(0, 24).is_err());
        assert!(size(80, 1001).is_err());
        assert!(size(80, 24).is_ok());
    }
    #[test]
    fn agent_control_ticket_cannot_be_combined_with_cli_launch() {
        let request = StartRequest {
            id: "test".into(),
            profile_id: "local:bash".into(),
            cwd: "/tmp".into(),
            cols: 80,
            rows: 24,
            agent_ticket: Some(AgentTicket {
                operation_id: "operation".into(),
                nonce: "nonce".into(),
            }),
            account_id: None,

            cli_launch: Some(crate::cli_catalog::TitleCli::Codex),
        };
        assert!(validate_start_request(&request).is_err());
    }
    #[test]
    fn start_request_accepts_camel_case_cli_launch_without_agent_ticket() {
        let request: StartRequest = serde_json::from_value(serde_json::json!({
            "id": "test",
            "profileId": "local:bash",
            "cwd": "/tmp",
            "cols": 80,
            "rows": 24,
            "cliLaunch": "codex"
        }))
        .unwrap();
        assert_eq!(
            request.cli_launch,
            Some(crate::cli_catalog::TitleCli::Codex)
        );
        assert!(request.agent_ticket.is_none());
    }
    #[test]
    fn account_terminal_request_requires_its_own_launch_and_rejects_legacy_routing() {
        assert!(serde_json::from_value::<StartRequest>(serde_json::json!({
            "id":"retired", "profileId":"local:bash", "cwd":"/tmp", "cols":80, "rows":24,
            "cliLaunch":"codex", "routerId":"pool"
        }))
        .is_err());
        let mut request: StartRequest = serde_json::from_value(serde_json::json!({
            "id":"account-terminal", "profileId":"local:bash", "cwd":"/tmp", "cols":80, "rows":24,
            "accountId":"account-fixture"
        }))
        .unwrap();
        assert_eq!(request.account_id.as_deref(), Some("account-fixture"));
        assert!(validate_start_request(&request).is_ok());
        request.cli_launch = Some(crate::cli_catalog::TitleCli::Codex);
        assert!(validate_start_request(&request).is_err());
        request.cli_launch = None;
        request.agent_ticket = Some(AgentTicket {
            operation_id: "operation".into(),
            nonce: "nonce".into(),
        });
        assert!(validate_start_request(&request).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn cli_launch_resolves_rc_path_and_starts_once_in_the_pty() {
        use std::{
            fs,
            os::unix::fs::PermissionsExt,
            sync::mpsc,
            time::{Duration, Instant},
        };

        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("test home");
        let bin = root.path().join("cli path with 'quote");
        let cwd = root.path().join("project");
        let integration = root.path().join("integration");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&cwd).unwrap();
        shell::prepare(&integration).unwrap();
        fs::write(
            home.join(".bashrc"),
            format!(
                "export PATH={}:\"$PATH\"\n",
                shell::quote(&bin.to_string_lossy(), "bash").unwrap()
            ),
        )
        .unwrap();

        let marker = root.path().join("launch-count");
        let cli = bin.join("codex");
        fs::write(
            &cli,
            format!(
                "#!/bin/sh\nprintf x >> {}\nprintf '__LOMI_CLI_LAUNCH_READY__\\n'\nexec /bin/sleep 30\n",
                shell::quote(&marker.to_string_lossy(), "bash").unwrap()
            ),
        )
        .unwrap();
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o755)).unwrap();

        let wrapper = root.path().join("selected-bash");
        fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nexport HOME={}\nexec /bin/bash \"$@\"\n",
                shell::quote(&home.to_string_lossy(), "bash").unwrap()
            ),
        )
        .unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
        let profile = Profile {
            id: "test:bash".into(),
            name: "bash".into(),
            kind: "bash".into(),
            program: wrapper.to_string_lossy().into_owned(),
            distro: None,
            home: home.to_string_lossy().into_owned(),
        };
        let shells = Shells {
            profiles: vec![profile.clone()],
            integration,
        };
        let manager = Terminals::default();
        let (send, receive) = mpsc::channel();
        let ack = manager.clone();
        let output = Channel::new(move |body| {
            if let tauri::ipc::InvokeResponseBody::Raw(bytes) = body {
                ack.acknowledge("cli-test", bytes.len());
                let _ = send.send(bytes);
            }
            Ok(())
        });
        let exited = Channel::new(|_| Ok(()));
        manager
            .start(
                &shells,
                StartRequest {
                    id: "cli-test".into(),
                    profile_id: profile.id,
                    cwd: cwd.to_string_lossy().into_owned(),
                    cols: 80,
                    rows: 24,
                    agent_ticket: None,
                    account_id: None,

                    cli_launch: Some(crate::cli_catalog::TitleCli::Codex),
                },
                output,
                exited,
            )
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut received = Vec::new();
        while !String::from_utf8_lossy(&received).contains("__LOMI_CLI_LAUNCH_READY__")
            && Instant::now() < deadline
        {
            if let Ok(bytes) = receive.recv_timeout(Duration::from_millis(100)) {
                received.extend(bytes);
            }
        }
        assert!(
            String::from_utf8_lossy(&received).contains("__LOMI_CLI_LAUNCH_READY__"),
            "CLI did not write to its PTY: {}",
            String::from_utf8_lossy(&received)
        );
        assert_eq!(fs::read_to_string(marker).unwrap(), "x");
        manager.stop_all();
    }
    #[cfg(windows)]
    #[test]
    fn process_snapshot_detects_child_without_powershell() {
        use std::process::Stdio;

        let mut child = shell::quiet_command("cmd.exe")
            .args(["/D", "/Q"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let parents = process_parents();
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(parents.unwrap().contains(&std::process::id()));
    }
    #[test]
    fn remote_observer_backpressure_preserves_bytes_and_remains_healthy() {
        let manager = Terminals::default();
        let (receive, healthy, _) = manager.observe_remote().unwrap();
        let sink = manager.remote_sink.lock().unwrap().clone().unwrap();
        for index in 0..256 {
            sink.send(RemoteTerminalEvent::Output {
                id: "test".into(),
                data: vec![index as u8; 16 * 1024],
            });
        }
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let producer = thread::spawn(move || {
            started_tx.send(()).unwrap();
            sink.send(RemoteTerminalEvent::Output {
                id: "test".into(),
                data: b"after the burst".to_vec(),
            });
            done_tx.send(()).unwrap();
        });
        started_rx.recv().unwrap();
        assert!(done_rx
            .recv_timeout(std::time::Duration::from_millis(50))
            .is_err());
        assert!(healthy.load(Ordering::SeqCst));
        for index in 0..256 {
            let RemoteTerminalEvent::Output { id, data } = receive.recv().unwrap().event else {
                panic!("Expected ordered terminal output");
            };
            assert_eq!(id, "test");
            assert_eq!(data, vec![index as u8; 16 * 1024]);
        }
        let RemoteTerminalEvent::Output { data, .. } = receive.recv().unwrap().event else {
            panic!("Expected the output queued after the burst");
        };
        assert_eq!(data, b"after the burst");
        done_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        producer.join().unwrap();
        assert!(healthy.load(Ordering::SeqCst));
    }

    #[test]
    fn remote_observer_sustained_outage_bounds_local_producers_and_tracks_only_lost_session() {
        let manager = Terminals::default();
        let (receive, healthy, observer) = manager.observe_remote().unwrap();
        let sink = manager.remote_sink.lock().unwrap().clone().unwrap();
        for _ in 0..256 {
            sink.send(RemoteTerminalEvent::Output {
                id: "lost".into(),
                data: vec![0; 16 * 1024],
            });
        }
        let started = std::time::Instant::now();
        for _ in 0..4096 {
            sink.send(RemoteTerminalEvent::Output {
                id: "lost".into(),
                data: vec![1; 16 * 1024],
            });
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        assert!(healthy.load(Ordering::SeqCst));
        assert!(observer
            .sessions()
            .iter()
            .any(|(id, _, lost, _, _)| id == "lost" && *lost));
        for _ in 0..256 {
            receive.recv().unwrap();
        }
        sink.send(RemoteTerminalEvent::Start {
            id: "survivor".into(),
            cols: 80,
            rows: 24,
        });
        sink.send(RemoteTerminalEvent::Output {
            id: "survivor".into(),
            data: b"exact".to_vec(),
        });
        assert_eq!(observer.current(&receive.recv().unwrap()), Some(false));
        let output = receive.recv().unwrap();
        assert_eq!(observer.current(&output), Some(false));
        let RemoteTerminalEvent::Output { data, .. } = output.event else {
            panic!("Expected survivor output");
        };
        assert_eq!(data, b"exact");
        sink.send(RemoteTerminalEvent::Exit {
            id: "lost".into(),
            code: Some(0),
        });
        assert!(observer
            .sessions()
            .iter()
            .any(|(id, _, lost, exited, _)| id == "lost" && *lost && *exited));
    }

    #[test]
    fn remote_observer_lost_start_retains_dimensions_and_restart_fences_old_events() {
        let manager = Terminals::default();
        let (receive, healthy, observer) = manager.observe_remote().unwrap();
        let sink = manager.remote_sink.lock().unwrap().clone().unwrap();
        for _ in 0..256 {
            sink.send(RemoteTerminalEvent::Output {
                id: "filler".into(),
                data: vec![0],
            });
        }
        sink.send(RemoteTerminalEvent::Start {
            id: "lost-start".into(),
            cols: 120,
            rows: 40,
        });
        let lost = observer
            .sessions()
            .into_iter()
            .find(|(id, _, _, _, _)| id == "lost-start")
            .unwrap();
        assert!(lost.2);
        assert_eq!(lost.4, Some((120, 40)));
        assert!(healthy.load(Ordering::SeqCst));
        for _ in 0..256 {
            receive.recv().unwrap();
        }
        sink.send(RemoteTerminalEvent::Start {
            id: "lost-start".into(),
            cols: 80,
            rows: 24,
        });
        let old_start = receive.recv().unwrap();
        sink.send(RemoteTerminalEvent::Start {
            id: "lost-start".into(),
            cols: 100,
            rows: 30,
        });
        let new_start = receive.recv().unwrap();
        assert_eq!(observer.current(&old_start), None);
        assert_eq!(observer.current(&new_start), Some(false));
        assert_ne!(old_start.generation, new_start.generation);
    }

    #[test]
    fn remote_observer_disconnect_releases_backpressured_producers() {
        let manager = Terminals::default();
        let (receive, healthy, _) = manager.observe_remote().unwrap();
        let sink = manager.remote_sink.lock().unwrap().clone().unwrap();
        for _ in 0..256 {
            sink.send(RemoteTerminalEvent::Output {
                id: "test".into(),
                data: vec![42],
            });
        }
        let (done_tx, done_rx) = mpsc::channel();
        let producer = thread::spawn(move || {
            sink.send(RemoteTerminalEvent::Output {
                id: "test".into(),
                data: vec![43],
            });
            done_tx.send(()).unwrap();
        });
        assert!(done_rx
            .recv_timeout(std::time::Duration::from_millis(50))
            .is_err());
        drop(receive);
        done_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        producer.join().unwrap();
        assert!(!healthy.load(Ordering::SeqCst));
    }

    #[cfg(unix)]
    #[test]
    fn remote_backpressure_does_not_hold_local_terminal_flow_or_prevent_close() {
        let directory = tempfile::tempdir().unwrap();
        shell::prepare(directory.path()).unwrap();
        let profile = shell::discover()
            .into_iter()
            .find(|p| p.kind == "bash")
            .unwrap();
        let shells = Shells {
            profiles: vec![profile.clone()],
            integration: directory.path().to_owned(),
        };
        let manager = Terminals::default();
        let (receive, healthy, _) = manager.observe_remote().unwrap();
        let sink = manager.remote_sink.lock().unwrap().clone().unwrap();
        let ack = manager.clone();
        manager
            .start(
                &shells,
                StartRequest {
                    id: "remote-backpressure-test".into(),
                    profile_id: profile.id,
                    cwd: directory.path().to_string_lossy().into_owned(),
                    cols: 80,
                    rows: 24,
                    agent_ticket: None,
                    account_id: None,

                    cli_launch: None,
                },
                Channel::new(move |body| {
                    if let tauri::ipc::InvokeResponseBody::Raw(bytes) = body {
                        ack.acknowledge("remote-backpressure-test", bytes.len());
                    }
                    Ok(())
                }),
                Channel::new(|_| Ok(())),
            )
            .unwrap();
        let session = manager.get("remote-backpressure-test").unwrap();
        while sink
            .sender
            .try_send(RemoteObservedEvent {
                generation: 0,
                event: RemoteTerminalEvent::Output {
                    id: "test".into(),
                    data: vec![42],
                },
            })
            .is_ok()
        {}
        manager
            .write("remote-backpressure-test", "printf 'burst-output'\r")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while session.remote_events.try_lock().is_ok() {
            if Instant::now() >= deadline {
                manager.close("remote-backpressure-test").unwrap();
                panic!("The PTY producer did not encounter Remote backpressure");
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(session.flow.try_lock().is_ok());
        assert!(healthy.load(Ordering::SeqCst));
        let resizing_session = session.clone();
        let (resize_started_tx, resize_started_rx) = mpsc::channel();
        let (resize_done_tx, resize_done_rx) = mpsc::channel();
        let resizing = thread::spawn(move || {
            resize_started_tx.send(()).unwrap();
            resize_done_tx
                .send(resizing_session.resize(
                    "remote-backpressure-test".into(),
                    size(100, 30).unwrap(),
                    Some(sink),
                ))
                .unwrap();
        });
        resize_started_rx.recv().unwrap();
        assert!(resize_done_rx
            .recv_timeout(Duration::from_millis(50))
            .is_err());
        let closer = manager.clone();
        let (done_tx, done_rx) = mpsc::channel();
        let close = thread::spawn(move || {
            closer.close("remote-backpressure-test").unwrap();
            done_tx.send(()).unwrap();
        });
        let closed = done_rx.recv_timeout(Duration::from_secs(2));
        drop(receive);
        close.join().unwrap();
        // With bounded waiting, resize may finish before close wins the race.
        let _ = resize_done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        resizing.join().unwrap();
        assert!(closed.is_ok(), "Local close waited for the Remote consumer");
        assert!(session.flow.lock().unwrap().closed);
    }

    #[cfg(unix)]
    #[test]
    fn remote_native_pty_writer_checks_grant_and_local_priority() {
        let directory = tempfile::tempdir().unwrap();
        shell::prepare(directory.path()).unwrap();
        let profile = shell::discover()
            .into_iter()
            .find(|p| p.kind == "bash")
            .unwrap();
        let shells = Shells {
            profiles: vec![profile.clone()],
            integration: directory.path().to_owned(),
        };
        let manager = Terminals::default();
        let (send, receive) = mpsc::channel();
        let ack = manager.clone();
        let output = Channel::new(move |body| {
            if let tauri::ipc::InvokeResponseBody::Raw(bytes) = body {
                ack.acknowledge("remote-test", bytes.len());
                let _ = send.send(bytes);
            }
            Ok(())
        });
        manager
            .start(
                &shells,
                StartRequest {
                    id: "remote-test".into(),
                    profile_id: profile.id,
                    cwd: directory.path().to_string_lossy().into_owned(),
                    cols: 80,
                    rows: 24,
                    agent_ticket: None,
                    account_id: None,

                    cli_launch: None,
                },
                output,
                Channel::new(|_| Ok(())),
            )
            .unwrap();
        manager
            .remote_claim("remote-test", "browser", "lease")
            .unwrap();
        let session = manager.get("remote-test").unwrap();
        let before = {
            let mut slot = session.remote_lease.lock().unwrap();
            let lease = slot.as_mut().unwrap();
            lease.deadline = std::time::Instant::now() + Duration::from_secs(2);
            (
                lease.id.clone(),
                lease.owner.clone(),
                lease.generation,
                lease.deadline,
            )
        };
        manager
            .remote_renew("remote-test", "browser", "lease")
            .unwrap();
        let renewed_deadline = {
            let slot = session.remote_lease.lock().unwrap();
            let lease = slot.as_ref().unwrap();
            assert_eq!(
                (&lease.id, &lease.owner, lease.generation),
                (&before.0, &before.1, before.2)
            );
            assert!(lease.deadline > before.3);
            lease.deadline
        };
        assert!(manager
            .remote_renew("remote-test", "other-browser", "lease")
            .is_err());
        assert!(manager
            .remote_renew("remote-test", "browser", "wrong-lease")
            .is_err());
        assert_eq!(
            session
                .remote_lease
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .deadline,
            renewed_deadline
        );
        session.flow.lock().unwrap().control = Some(Arc::new(Mutex::new(
            TerminalControl::new("agent".into(), "remote-test".into()).unwrap(),
        )));
        assert!(manager
            .remote_renew("remote-test", "browser", "lease")
            .is_err());
        session.flow.lock().unwrap().control = None;
        let denied = manager.remote_input(
            "remote-test",
            "browser",
            "lease",
            b"printf 'UNAUTHORIZED'\r",
            || false,
        );
        assert_eq!((denied.written, denied.status), (0, "rejected"));
        let accepted = manager.remote_input(
            "remote-test",
            "browser",
            "lease",
            b"printf '__REMOTE_NATIVE_OK__\n'\r",
            || true,
        );
        assert_eq!(accepted.status, "accepted");
        assert!(accepted.written > 0);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut text = Vec::new();
        while std::time::Instant::now() < deadline
            && !String::from_utf8_lossy(&text).contains("__REMOTE_NATIVE_OK__")
        {
            if let Ok(data) = receive.recv_timeout(Duration::from_millis(100)) {
                text.extend(data);
            }
        }
        assert!(String::from_utf8_lossy(&text).contains("__REMOTE_NATIVE_OK__"));
        manager.write_response("remote-test", "\x1b[1;1R").unwrap();
        assert!(manager.remote_lease_live("remote-test", "browser", "lease"));
        manager.remote_revoke_owner("remote-test", "unrelated-observer");
        assert!(manager.remote_lease_live("remote-test", "browser", "lease"));
        let checks = std::sync::atomic::AtomicUsize::new(0);
        let partial = manager.remote_input(
            "remote-test",
            "browser",
            "lease",
            &vec![b'x'; 16384],
            || checks.fetch_add(1, Ordering::SeqCst) < 2,
        );
        assert_eq!(partial.status, "partial");
        assert!(partial.written > 0 && partial.written < 16384);
        manager.write("remote-test", "\u{3}").unwrap();
        manager
            .write("remote-test", "printf '__LOCAL_NATIVE_OK__\n'\r")
            .unwrap();
        assert!(!manager.remote_lease_live("remote-test", "browser", "lease"));
        assert!(manager
            .remote_renew("remote-test", "browser", "lease")
            .is_err());
        assert!(session.remote_lease.lock().unwrap().is_none());
        let stale = manager.remote_input(
            "remote-test",
            "browser",
            "lease",
            b"printf 'STALE'\r",
            || true,
        );
        assert_eq!((stale.written, stale.status), (0, "rejected"));
        // A queued human writer must also prevent a competing Remote claim.
        manager
            .remote_claim("remote-test", "browser", "queued-lease")
            .unwrap();
        session.human_waiters.fetch_add(1, Ordering::SeqCst);
        let human = HumanInputGuard(session.clone());
        assert!(manager
            .remote_renew("remote-test", "browser", "queued-lease")
            .is_err());
        assert!(manager
            .remote_claim("remote-test", "browser", "queued-human")
            .is_err());
        drop(human);
        assert_eq!(session.human_waiters.load(Ordering::SeqCst), 0);
        manager
            .remote_claim("remote-test", "browser", "lease-new")
            .unwrap();
        // Hold the writer until local input has invalidated the lease and queued.
        // Renewal then races for the writer, but neither ordering may restore control.
        let writer = session.writer.lock().unwrap();
        let human_barrier = Arc::new(std::sync::Barrier::new(2));
        let local_manager = manager.clone();
        let local_barrier = human_barrier.clone();
        let local = thread::spawn(move || {
            local_barrier.wait();
            local_manager.write("remote-test", "\u{3}")
        });
        human_barrier.wait();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while session.human_waiters.load(Ordering::SeqCst) == 0
            || session.remote_lease.lock().unwrap().is_some()
        {
            assert!(
                std::time::Instant::now() < deadline,
                "Local input did not queue"
            );
            thread::yield_now();
        }
        let renew_barrier = Arc::new(std::sync::Barrier::new(2));
        let renew_manager = manager.clone();
        let start_renew = renew_barrier.clone();
        let renew = thread::spawn(move || {
            start_renew.wait();
            renew_manager.remote_renew("remote-test", "browser", "lease-new")
        });
        renew_barrier.wait();
        drop(writer);
        assert!(renew.join().unwrap().is_err());
        local.join().unwrap().unwrap();
        assert!(!manager.remote_lease_live("remote-test", "browser", "lease-new"));
        assert!(session.remote_lease.lock().unwrap().is_none());

        manager
            .remote_claim("remote-test", "browser", "lease-new")
            .unwrap();
        let generation = session
            .remote_lease
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .generation;
        session.human_generation.fetch_add(1, Ordering::SeqCst);
        assert!(manager
            .remote_renew("remote-test", "browser", "lease-new")
            .is_err());
        assert_eq!(
            session
                .remote_lease
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .generation,
            generation
        );
        manager
            .remote_claim("remote-test", "browser", "lease-new")
            .unwrap();
        manager
            .get("remote-test")
            .unwrap()
            .remote_lease
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .deadline = std::time::Instant::now() - Duration::from_millis(1);
        assert!(manager
            .remote_renew("remote-test", "browser", "lease-new")
            .is_err());
        assert_eq!(
            manager
                .remote_input("remote-test", "browser", "lease-new", b"expired", || true)
                .status,
            "rejected"
        );
        manager.stop_all();
    }

    #[cfg(unix)]
    #[test]
    fn pty_streams_utf8_resizes_and_exits() {
        use std::{
            sync::mpsc,
            time::{Duration, Instant},
        };
        let directory = tempfile::tempdir().unwrap();
        shell::prepare(directory.path()).unwrap();
        let profile = shell::discover()
            .into_iter()
            .find(|profile| profile.kind == "bash")
            .unwrap();
        let shells = Shells {
            profiles: vec![profile.clone()],
            integration: directory.path().to_owned(),
        };
        let manager = Terminals::default();
        let (send, receive) = mpsc::channel();
        let ack = manager.clone();
        let output = Channel::new(move |body| {
            if let tauri::ipc::InvokeResponseBody::Raw(bytes) = body {
                ack.acknowledge("test", bytes.len());
                send.send(bytes).unwrap();
            }
            Ok(())
        });
        let (exit_send, exit_receive) = mpsc::channel();
        let exited = Channel::new(move |_| {
            exit_send.send(()).unwrap();
            Ok(())
        });
        manager
            .start(
                &shells,
                StartRequest {
                    id: "test".into(),
                    profile_id: profile.id,
                    cwd: directory.path().to_string_lossy().into_owned(),
                    cols: 80,
                    rows: 24,
                    agent_ticket: None,
                    account_id: None,

                    cli_launch: None,
                },
                output,
                exited,
            )
            .unwrap();
        manager
            .get("test")
            .unwrap()
            .master
            .lock()
            .unwrap()
            .resize(size(101, 31).unwrap())
            .unwrap();
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let session = manager.get("test").unwrap();
            manager.write("test", "sleep 30\r").unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while session.foreground_program().as_deref() != Some("sleep")
                && Instant::now() < deadline
            {
                thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(session.foreground_program().as_deref(), Some("sleep"));
            #[cfg(target_os = "macos")]
            {
                let child_pid = session
                    .master
                    .lock()
                    .unwrap()
                    .process_group_leader()
                    .unwrap() as u32;
                assert!(
                    session.protected_origin(&[child_pid]),
                    "A peer descended from the PTY must be protected"
                );
                assert!(
                    !session.protected_origin(&[std::process::id()]),
                    "The external application is not a descendant of its PTY"
                );
                assert!(
                    session.protected_origin(&[i32::MAX as u32]),
                    "Unresolved process identity fails closed"
                );
            }
            assert_eq!(manager.busy(&["test".into()]).unwrap(), ["test"]);
            assert!(manager.busy(&["other".into()]).unwrap().is_empty());
            manager.write("test", "\u{3}").unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while session.foreground_program().is_some() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(session.foreground_program(), None);
            assert!(manager.busy(&["test".into()]).unwrap().is_empty());
            manager.write("test", "sleep 30 &\r").unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while manager.busy(&["test".into()]).unwrap().is_empty() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(session.foreground_program(), None);
            assert_eq!(manager.busy(&["test".into()]).unwrap(), ["test"]);
            manager.write("test", "kill %1; wait\r").unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while !manager.busy(&["test".into()]).unwrap().is_empty() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            assert!(manager.busy(&["test".into()]).unwrap().is_empty());
        }
        #[cfg(target_os = "linux")]
        {
            let session = manager.get("test").unwrap();
            use crate::cli_titles::TitleCli;
            let sleep_path = "/usr/bin/sleep";
            for (name, cli) in [
                ("codex", TitleCli::Codex),
                ("agy", TitleCli::Agy),
                ("claude", TitleCli::Claude),
                ("cursor-agent", TitleCli::Cursor),
            ] {
                let executable_path = directory.path().join(name);
                std::fs::copy(sleep_path, &executable_path).unwrap();
                let executable = shell::quote(&executable_path.to_string_lossy(), "bash").unwrap();
                for command in [
                    format!("{executable} 30\r"),
                    format!(
                        "sh -c 'trap \"kill \\$! 2>/dev/null; exit\" INT TERM; \"$1\" 30 & wait' sh {executable}\r"
                    ),
                ] {
                    manager.write("test", &command).unwrap();
                    let deadline = Instant::now() + Duration::from_secs(5);
                    while session.title_process().is_none() && Instant::now() < deadline {
                        thread::sleep(Duration::from_millis(10));
                    }
                    assert_eq!(session.title_process().unwrap().cli, cli);
                    let process = session.title_process().unwrap();
                    manager.check_title_process("test", process).unwrap();
                    assert!(manager
                        .check_title_process(
                            "test",
                            crate::cli_titles::TitleProcess {
                                pid: process.pid + 1,
                                ..process
                            }
                        )
                        .is_err());
                    manager.write("test", "\u{3}").unwrap();
                    let deadline = Instant::now() + Duration::from_secs(5);
                    while session.foreground_program().is_some() && Instant::now() < deadline {
                        thread::sleep(Duration::from_millis(10));
                    }
                    assert!(session.title_process().is_none());
                }
            }
        }
        manager
            .write(
                "test",
                "printf 'UTF8: zażółć\\n'; stty size; exec sleep 1\r",
            )
            .unwrap();
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let session = manager.get("test").unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while session.foreground_program().as_deref() != Some("sleep")
                && Instant::now() < deadline
            {
                thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(session.foreground_program().as_deref(), Some("sleep"));
            assert_eq!(
                session
                    .master
                    .lock()
                    .unwrap()
                    .process_group_leader()
                    .map(|pid| pid as u32),
                session.pid,
            );
            assert_eq!(manager.busy(&["test".into()]).unwrap(), ["test"]);
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut bytes = Vec::new();
        let mut eof = false;
        while Instant::now() < deadline {
            if let Ok(chunk) = receive.recv_timeout(Duration::from_millis(100)) {
                if chunk.is_empty() {
                    assert!(exit_receive.try_recv().is_ok());
                    eof = true;
                    break;
                }
                bytes.extend(chunk);
            }
        }
        manager.stop_all();
        assert!(eof, "The terminal did not send its ordered EOF marker.");
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("UTF8: zażółć"), "{text}");
        assert!(text.contains("31 101"), "{text}");
        assert!(manager.sessions.lock().unwrap().is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_pty_detects_unsigned_cli_child_and_resolves_process_config() {
        use crate::cli_titles::TitleCli;
        use std::{
            sync::mpsc,
            time::{Duration, Instant},
        };

        let directory = tempfile::tempdir().unwrap();
        shell::prepare(directory.path()).unwrap();
        let profile = shell::discover()
            .into_iter()
            .find(|profile| profile.kind == "bash")
            .unwrap();
        let shells = Shells {
            profiles: vec![profile.clone()],
            integration: directory.path().to_owned(),
        };
        let manager = Terminals::default();
        let (send, receive) = mpsc::channel();
        let ack = manager.clone();
        let output = Channel::new(move |body| {
            if let tauri::ipc::InvokeResponseBody::Raw(bytes) = body {
                ack.acknowledge("mac-fixture", bytes.len());
                let _ = send.send(bytes);
            }
            Ok(())
        });
        let (exit_send, _exit_receive) = mpsc::channel();
        let exited = Channel::new(move |_| {
            exit_send.send(()).unwrap();
            Ok(())
        });
        manager
            .start(
                &shells,
                StartRequest {
                    id: "mac-fixture".into(),
                    profile_id: profile.id,
                    cwd: directory.path().to_string_lossy().into_owned(),
                    cols: 80,
                    rows: 24,
                    agent_ticket: None,
                    account_id: None,

                    cli_launch: None,
                },
                output,
                exited,
            )
            .unwrap();

        let marker = b"__LOMI_FIXTURE_SHELL_READY__";
        manager
            .write("mac-fixture", "printf '__LOMI_FIXTURE_SHELL_READY__\\n'\r")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut output = Vec::new();
        while !output.windows(marker.len()).any(|window| window == marker)
            && Instant::now() < deadline
        {
            let Ok(chunk) = receive.recv_timeout(Duration::from_millis(100)) else {
                continue;
            };
            assert!(!chunk.is_empty(), "PTY closed before its readiness marker");
            output.extend(chunk);
        }
        assert!(
            output.windows(marker.len()).any(|window| window == marker),
            "interactive shell did not reach the fixture readiness marker"
        );

        let fixture = directory.path().join("codex");
        std::fs::copy(std::env::current_exe().unwrap(), &fixture).unwrap();
        let home = directory.path().join("fixture-home");
        let codex_home = directory.path().join("fixture-codex-home");
        let canonical_codex_home = directory
            .path()
            .canonicalize()
            .unwrap()
            .join("fixture-codex-home");
        let command = format!(
            "HOME={} CODEX_HOME={} LOMI_CLI_PROCESS_FIXTURE=1 sh -c '\"$1\" --exact cli_titles::tests::fixture_cli_process --nocapture & wait' sh {}\r",
            shell::quote(&home.to_string_lossy(), "bash").unwrap(),
            shell::quote(&codex_home.to_string_lossy(), "bash").unwrap(),
            shell::quote(&fixture.to_string_lossy(), "bash").unwrap(),
        );
        manager.write("mac-fixture", &command).unwrap();
        let session = manager.get("mac-fixture").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let process = loop {
            if let Some(process) = session.title_process() {
                break process;
            }
            assert!(
                Instant::now() < deadline,
                "fake Codex child was not detected; foreground {:?}",
                session.foreground_program()
            );
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(process.cli, TitleCli::Codex);
        assert_ne!(
            Some(process.pid as i32),
            session.master.lock().unwrap().process_group_leader(),
            "fixture should be discovered as a descendant of its shell wrapper"
        );
        manager.check_title_process("mac-fixture", process).unwrap();
        assert_eq!(
            crate::cli_titles::configuration(process).unwrap(),
            (canonical_codex_home.join("config.toml"), false)
        );
        assert_eq!(
            crate::cli_titles::mcp_configuration(process).unwrap(),
            canonical_codex_home.join("config.toml")
        );
        assert!(
            !codex_home.exists(),
            "inspection must not create user config directories"
        );
        manager.write("mac-fixture", "\u{3}").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while session.title_process().is_some() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        manager.stop_all();
    }
}
#[cfg(all(test, unix))]
mod native_final_admission_tests {
    use super::*;
    #[test]
    fn changed_artifact_after_builder_is_refused_before_actual_pty_spawn() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("artifact");
        std::fs::write(&executable, b"original pinned bytes").unwrap();
        let shells = Shells {
            profiles: vec![Profile {
                id: "fixture".into(),
                name: "Fixture".into(),
                kind: "sh".into(),
                program: "/bin/sh".into(),
                distro: None,
                home: root.path().to_string_lossy().into_owned(),
            }],
            integration: root.path().into(),
        };
        let marker = root.path().join("must-not-execute");
        let mut command = portable_pty::CommandBuilder::new("/bin/sh");
        command.env_clear();
        command.arg("-c");
        command.arg("touch must-not-execute");
        command.cwd(root.path());
        let admission_path = executable.clone();
        let checked = Arc::new(AtomicBool::new(false));
        let observed = checked.clone();
        let launch = NativeTerminalLaunch {
            command: (command, root.path().to_string_lossy().into_owned()),
            #[cfg(target_os = "macos")]
            owned_boundary: None,
            admission: Box::new(move || {
                observed.store(true, Ordering::SeqCst);
                if std::fs::read(admission_path).unwrap() != b"original pinned bytes" {
                    Err("Reviewed executable changed before spawn".into())
                } else {
                    Ok(())
                }
            }),
        };
        std::fs::write(executable, b"replacement wrapper").unwrap();
        let manager = Terminals::default();
        let result = manager.start_account(
            &shells,
            StartRequest {
                id: "final-admission-fixture".into(),
                profile_id: "fixture".into(),
                cwd: root.path().to_string_lossy().into_owned(),
                cols: 80,
                rows: 24,
                agent_ticket: None,
                account_id: Some("fixture".into()),
                cli_launch: None,
            },
            Channel::new(|_| Ok(())),
            Channel::new(|_| Ok(())),
            launch,
            Arc::new(AtomicBool::new(false)),
        );
        assert!(result.is_err());
        assert!(checked.load(Ordering::SeqCst));
        assert!(!marker.exists());
    }
}
