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
    sync::{Arc, Condvar, Mutex},
    thread,
};
use tauri::{
    ipc::{Channel, Response},
    State, Window,
};

const HIGH_WATER: usize = 128 * 1024;

pub mod clipboard;

#[derive(Default)]
struct Flow {
    pending: usize,
    closed: bool,
}

struct Session {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    flow: Mutex<Flow>,
    ready: Condvar,
    pid: Option<u32>,
    profile: Profile,
}

impl Session {
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
        #[cfg(target_os = "linux")]
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

#[derive(Default, Clone)]
pub struct Terminals {
    sessions: Arc<Mutex<HashMap<String, Arc<Session>>>>,
}

#[derive(Clone)]
pub struct Shells {
    pub profiles: Vec<Profile>,
    pub integration: PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRequest {
    id: String,
    profile_id: String,
    cwd: String,
    cols: u16,
    rows: u16,
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

    #[cfg(feature = "native-smoke")]
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

    pub fn close(&self, id: &str) {
        let session = self
            .sessions
            .lock()
            .ok()
            .and_then(|mut sessions| sessions.remove(id));
        if let Some(session) = session {
            session.stop();
        }
    }

    pub fn acknowledge(&self, id: &str, bytes: usize) {
        if let Ok(session) = self.get(id) {
            if let Ok(mut flow) = session.flow.lock() {
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
        if request.id.is_empty() || request.id.len() > 128 {
            return Err("Invalid terminal identifier.".into());
        }
        let profile = shells
            .profiles
            .iter()
            .find(|profile| profile.id == request.profile_id)
            .cloned()
            .ok_or("The selected shell is no longer installed. Choose another shell.")?;
        let (command, cwd) = shell::build(&profile, &request.cwd, &shells.integration)?;
        let pair = native_pty_system()
            .openpty(size(request.cols, request.rows)?)
            .map_err(|error| error.to_string())?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| error.to_string())?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| error.to_string())?;
        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| format!("Cannot start {}: {error}", profile.name))?;
        drop(pair.slave);
        let session = Arc::new(Session {
            pid: child.process_id(),
            profile: profile.clone(),
            master: Mutex::new(pair.master),
            writer: Mutex::new(Some(writer)),
            killer: Mutex::new(child.clone_killer()),
            flow: Mutex::new(Flow::default()),
            ready: Condvar::new(),
        });
        {
            let mut sessions = self.sessions.lock().map_err(|error| error.to_string())?;
            if sessions.contains_key(&request.id) {
                drop(sessions);
                session.stop();
                let _ = child.wait();
                return Err("A terminal with this identifier already exists.".into());
            }
            sessions.insert(request.id.clone(), session.clone());
        }
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
                    Ok(0) | Err(_) => break,
                    Ok(length) => length,
                };
                // Bound bytes in flight; acknowledge only after xterm has parsed them.
                session
                    .flow
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .pending += length;
                if output
                    .send(Response::new(buffer[..length].to_vec()))
                    .is_err()
                {
                    session.stop();
                    break;
                }
            }
            drop(reader);
            let code = child.wait().ok().map(|status| status.exit_code());
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

    fn write(&self, id: &str, data: &str) -> Result<(), String> {
        if data.len() > 256 * 1024 {
            return Err("A terminal input chunk exceeds 256 KiB.".into());
        }
        let session = self.get(id)?;
        let mut writer = session.writer.lock().map_err(|error| error.to_string())?;
        let writer = writer.as_mut().ok_or("The terminal has been closed.")?;
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
    state: State<'_, Terminals>,
    shells: State<'_, Shells>,
    request: StartRequest,
    output: Channel<Response>,
    exited: Channel<Exit>,
) -> Result<Started, String> {
    main_window(&window)?;
    let state = state.inner().clone();
    let shells = shells.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.start(&shells, request, output, exited))
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
    // ConPTY resize is synchronous and must not block the native event loop.
    tauri::async_runtime::spawn_blocking(move || {
        session
            .master
            .lock()
            .map_err(|error| error.to_string())?
            .resize(size)
            .map_err(|error| error.to_string())
    })
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
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn reset_terminals(window: Window, state: State<'_, Terminals>) -> Result<(), String> {
    main_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.stop_all())
        .await
        .map_err(|error| error.to_string())
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
        #[cfg(target_os = "linux")]
        if let Some(pid) = session.pid {
            context.cwd = std::fs::read_link(format!("/proc/{pid}/cwd"))
                .ok()
                .map(|path| path.to_string_lossy().into_owned());
            context.foreground_program = session.foreground_program();
            context.title_cli = session.title_process();
        }
        #[cfg(not(target_os = "linux"))]
        let _ = &session.pid;
        contexts.insert(id.clone(), context);
    }
    Ok(contexts)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_terminal_sizes() {
        assert!(size(0, 24).is_err());
        assert!(size(80, 1001).is_err());
        assert!(size(80, 24).is_ok());
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
            for (name, cli) in [
                ("codex", TitleCli::Codex),
                ("agy", TitleCli::Agy),
                ("claude", TitleCli::Claude),
                ("cursor-agent", TitleCli::Cursor),
            ] {
                let executable_path = directory.path().join(name);
                std::fs::copy("/usr/bin/sleep", &executable_path).unwrap();
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
}
