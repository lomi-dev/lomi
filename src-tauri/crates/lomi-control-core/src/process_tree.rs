use std::{
    io,
    process::{Child, Command, ExitStatus},
};

#[cfg(windows)]
const DESCENDANT_EXIT_GRACE: std::time::Duration = std::time::Duration::from_millis(500);

/// Own one explicitly spawned process group. On Unix the group
/// leader is not reaped until its descendants exit, preventing PGID reuse.
pub struct Tree {
    pub child: Child,
    exit: Option<ExitStatus>,
    killed: bool,
    pub abandoned_descendants: bool,
    #[cfg(windows)]
    job: std::sync::Arc<windows::Job>,
    #[cfg(windows)]
    leader_exited_at: Option<std::time::Instant>,
}

impl Tree {
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000004);
        }
        let child = command.spawn()?;
        #[cfg(windows)]
        let (child, job) = windows::attach(child)?;
        Ok(Self {
            child,
            exit: None,
            killed: false,
            abandoned_descendants: false,
            #[cfg(windows)]
            job: std::sync::Arc::new(job),
            #[cfg(windows)]
            leader_exited_at: None,
        })
    }

    /// Observe this tree without transferring termination authority. Dropping the
    /// tree still explicitly kills the job while observers retain their handles.
    #[cfg(windows)]
    pub fn membership(&self) -> Membership {
        Membership(self.job.clone())
    }

    pub fn kill(&mut self) -> io::Result<()> {
        if self.exit.is_some() || self.killed {
            return Ok(());
        }
        #[cfg(unix)]
        if unsafe { libc::kill(-(self.child.id() as i32), libc::SIGKILL) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        #[cfg(windows)]
        self.job.kill()?;
        self.killed = true;
        Ok(())
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if let Some(exit) = self.exit {
            return Ok(Some(exit));
        }
        #[cfg(unix)]
        {
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            if unsafe {
                libc::waitid(
                    libc::P_PID,
                    self.child.id(),
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            } != 0
            {
                return Err(io::Error::last_os_error());
            }
            if unsafe { info.si_pid() } == 0 {
                return Ok(None);
            }
            if group_has_descendants(self.child.id())? {
                if !self.killed {
                    self.abandoned_descendants = true;
                    self.kill()?;
                }
                return Ok(None);
            }
        }
        #[cfg(windows)]
        {
            if self.child.try_wait()?.is_none() {
                return Ok(None);
            }
            let exited_at = self
                .leader_exited_at
                .get_or_insert_with(std::time::Instant::now);
            if self.job.active()? != 0 {
                // Windows helpers (including console hosts) can finish shortly
                // after the launcher. Retain the job and caller's ownership
                // guards throughout one bounded grace period; never reset its
                // start on later polls. Explicit kill bypasses this grace.
                // Success still requires all job accounting to drain.
                if !self.killed
                    && exited_at.elapsed() >= DESCENDANT_EXIT_GRACE
                    && self.job.has_live_descendants(self.child.id())?
                {
                    self.abandoned_descendants = true;
                    self.kill()?;
                }
                return Ok(None);
            }
        }
        self.exit = Some(self.child.wait()?);
        Ok(self.exit)
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = self.kill();
    }
}

/// A read-only observer of the kernel-owned process tree.
#[cfg(windows)]
#[derive(Clone)]
pub struct Membership(std::sync::Arc<windows::Job>);

#[cfg(windows)]
impl Membership {
    pub fn pids(&self) -> io::Result<Vec<u32>> {
        self.0.pids()
    }

    pub fn contains(&self, pid: u32) -> io::Result<bool> {
        Ok(self.pids()?.contains(&pid))
    }
}

#[cfg(target_os = "macos")]
fn group_has_descendants(leader: u32) -> io::Result<bool> {
    unsafe extern "C" {
        fn proc_listpids(kind: u32, info: u32, buffer: *mut std::ffi::c_void, size: i32) -> i32;
    }
    const PROC_PGRP_ONLY: u32 = 2;
    let mut pids = vec![0_i32; 4096];
    let size = (pids.len() * std::mem::size_of::<i32>()) as i32;
    let count = unsafe { proc_listpids(PROC_PGRP_ONLY, leader, pids.as_mut_ptr().cast(), size) };
    if count <= 0 {
        return Err(io::Error::last_os_error());
    }
    if count >= size {
        return Err(io::Error::other("Owned process group exceeds its bound"));
    }
    for pid in pids
        .into_iter()
        .take(count as usize / 4)
        .filter(|pid| *pid > 0 && *pid as u32 != leader)
    {
        // Zombies have no remaining filesystem work or open output pipes.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let length = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                std::mem::size_of::<libc::proc_bsdinfo>() as i32,
            )
        };
        if length == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                continue;
            }
            return Err(error);
        }
        if length as usize != std::mem::size_of::<libc::proc_bsdinfo>() {
            return Err(io::Error::other("Incomplete owned process identity"));
        }
        if info.pbi_pgid == leader && info.pbi_status != 5 {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(target_os = "linux")]
fn group_has_descendants(leader: u32) -> io::Result<bool> {
    use std::io::Read;
    for (index, entry) in std::fs::read_dir("/proc")?.enumerate() {
        if index > 1_000_000 {
            return Err(io::Error::other("Process inventory exceeds its bound"));
        }
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == leader {
            continue;
        }
        let mut text = String::new();
        match std::fs::File::open(entry.path().join("stat"))
            .and_then(|file| file.take(16385).read_to_string(&mut text))
        {
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
                ) =>
            {
                continue
            }
            Err(error) => return Err(error),
        }
        if text.len() > 16384 {
            return Err(io::Error::other("Process metadata exceeds its bound"));
        }
        if let Some((_, fields)) = text.rsplit_once(") ") {
            let fields: Vec<_> = fields.split_whitespace().take(3).collect();
            if fields.len() == 3 && fields[2].parse::<u32>() == Ok(leader) && fields[0] != "Z" {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, HANDLE, INVALID_HANDLE_VALUE,
            WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD,
                THREADENTRY32,
            },
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob,
                JobObjectBasicAccountingInformation, JobObjectBasicProcessIdList,
                JobObjectExtendedLimitInformation, QueryInformationJobObject,
                SetInformationJobObject, TerminateJobObject,
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
            Threading::{
                OpenProcess, OpenThread, ResumeThread, WaitForSingleObject,
                PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, THREAD_SUSPEND_RESUME,
            },
        },
    };

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    pub struct Job(Handle);
    // Windows handles are kernel-owned and may be used by the operation's worker thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}
    impl Job {
        pub fn pids(&self) -> io::Result<Vec<u32>> {
            #[repr(C)]
            struct Processes {
                assigned: u32,
                count: u32,
                ids: [usize; 4096],
            }
            let mut processes: Processes = unsafe { std::mem::zeroed() };
            if unsafe {
                QueryInformationJobObject(
                    self.0 .0,
                    JobObjectBasicProcessIdList,
                    (&mut processes as *mut Processes).cast(),
                    std::mem::size_of_val(&processes) as u32,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            if processes.assigned > 4096 || processes.count > 4096 {
                return Err(io::Error::other("Owned process job exceeds its bound"));
            }
            processes.ids[..processes.count as usize]
                .iter()
                .map(|pid| u32::try_from(*pid).map_err(io::Error::other))
                .collect()
        }
        pub fn kill(&self) -> io::Result<()> {
            if unsafe { TerminateJobObject(self.0 .0, 1) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
        pub(super) fn has_live_descendants(&self, leader: u32) -> io::Result<bool> {
            for pid in self.pids()? {
                if pid == leader {
                    // The retained Child has already reported leader exit.
                    // Its job entry can outlive that observation; only other
                    // members can be abandoned descendants.
                    continue;
                }
                let raw = unsafe {
                    OpenProcess(
                        PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                        0,
                        pid,
                    )
                };
                if raw.is_null() {
                    if unsafe { GetLastError() } == ERROR_INVALID_PARAMETER {
                        // The enumerated member has already disappeared.
                        continue;
                    }
                    return Err(io::Error::last_os_error());
                }
                let process = Handle(raw);
                let mut member = 0;
                if unsafe { IsProcessInJob(process.0, self.0 .0, &mut member) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                if member == 0 {
                    // The PID was reused after the job snapshot. Never treat
                    // an unrelated process as a living owned descendant.
                    continue;
                }
                match unsafe { WaitForSingleObject(process.0, 0) } {
                    WAIT_TIMEOUT => return Ok(true),
                    WAIT_OBJECT_0 => {}
                    WAIT_FAILED => return Err(io::Error::last_os_error()),
                    _ => return Err(io::Error::other("Unexpected owned process wait result")),
                }
            }
            Ok(false)
        }
        pub fn active(&self) -> io::Result<u32> {
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
            if unsafe {
                QueryInformationJobObject(
                    self.0 .0,
                    JobObjectBasicAccountingInformation,
                    (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                    std::mem::size_of_val(&info) as u32,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(info.ActiveProcesses)
        }
    }
    pub fn attach(mut child: Child) -> io::Result<(Child, Job)> {
        let result = (|| {
            let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if raw.is_null() {
                return Err(io::Error::last_os_error());
            }
            let job = Job(Handle(raw));
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if unsafe {
                SetInformationJobObject(
                    raw,
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of_val(&limits) as u32,
                )
            } == 0
                || unsafe { AssignProcessToJobObject(raw, child.as_raw_handle().cast()) } == 0
            {
                return Err(io::Error::last_os_error());
            }
            let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            let snapshot = Handle(snapshot);
            let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
            entry.dwSize = std::mem::size_of_val(&entry) as u32;
            let mut found = false;
            let mut next = unsafe { Thread32First(snapshot.0, &mut entry) };
            while next != 0 {
                if entry.th32OwnerProcessID == child.id() {
                    let thread =
                        unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                    if thread.is_null() {
                        return Err(io::Error::last_os_error());
                    }
                    let thread = Handle(thread);
                    if unsafe { ResumeThread(thread.0) } == u32::MAX {
                        return Err(io::Error::last_os_error());
                    }
                    found = true;
                }
                next = unsafe { Thread32Next(snapshot.0, &mut entry) };
            }
            if !found {
                return Err(io::Error::other(
                    "The suspended owned process has no primary thread",
                ));
            }
            Ok(job)
        })();
        match result {
            Ok(job) => Ok((child, job)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::{
        path::Path,
        process::Stdio,
        time::{Duration, Instant},
    };

    const FIXTURE: &str = "process_tree::windows_tests::launcher_fixture";

    fn fixture(directory: &Path, role: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", FIXTURE, "--ignored", "--nocapture"])
            .env("LOMI_TREE_FIXTURE_DIRECTORY", directory)
            .env("LOMI_TREE_FIXTURE_ROLE", role)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    #[test]
    #[ignore = "subprocess fixture invoked by the ownership test"]
    fn launcher_fixture() {
        let directory = std::env::var_os("LOMI_TREE_FIXTURE_DIRECTORY").unwrap();
        let directory = Path::new(&directory);
        let role = std::env::var("LOMI_TREE_FIXTURE_ROLE").unwrap();
        if role == "output" {
            use std::io::Write;
            let mut output = std::io::stdout().lock();
            for _ in 0..32 {
                output.write_all(&[b'x'; 8192]).unwrap();
            }
            output.flush().unwrap();
        } else if role == "launcher" || role == "orphan-launcher" {
            let mut child = fixture(directory, "qemu").spawn().unwrap();
            std::fs::write(directory.join("qemu.pid"), child.id().to_string()).unwrap();
            if role == "orphan-launcher" {
                wait_for(|| directory.join("ready").is_file());
                // Model a failed launcher leaving its live child behind.
                return;
            }
            // Model emulator.exe's _wspawnv followed by an infinite child wait.
            child.wait().unwrap();
        } else {
            std::fs::write(directory.join("ready"), b"ready").unwrap();
            let deadline = Instant::now() + Duration::from_secs(60);
            while !directory.join("shutdown").is_file() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    fn wait_for(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !condition() {
            assert!(
                Instant::now() < deadline,
                "native process fixture timed out"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn windows_output_only_exit_never_marks_the_leader_as_abandoned() {
        use std::io::Read;
        let directory = tempfile::tempdir().unwrap();
        // Exercise signaling/accounting order across successive pipe-heavy
        // exits. Every iteration must succeed; failures are never retried.
        for _ in 0..16 {
            let mut command = fixture(directory.path(), "output");
            command.stdout(Stdio::piped());
            let mut tree = Tree::spawn(&mut command).unwrap();
            let mut pipe = tree.child.stdout.take().unwrap();
            let reader = std::thread::spawn(move || {
                let mut output = Vec::new();
                pipe.read_to_end(&mut output).unwrap();
                output
            });
            let mut exit = None;
            wait_for(|| {
                exit = tree.try_wait().unwrap();
                exit.is_some()
            });
            assert!(exit.unwrap().success());
            assert!(!tree.abandoned_descendants);
            let output = reader.join().unwrap();
            assert!(output.iter().filter(|byte| **byte == b'x').count() >= 32 * 8192);
        }
    }

    #[test]
    fn windows_launcher_tree_force_stop_and_drop_own_only_descendants() {
        let unrelated_directory = tempfile::tempdir().unwrap();
        let mut unrelated = fixture(unrelated_directory.path(), "qemu");
        let mut unrelated = Tree::spawn(&mut unrelated).unwrap();
        wait_for(|| unrelated_directory.path().join("ready").is_file());
        for force in [true, false] {
            let directory = tempfile::tempdir().unwrap();
            let mut command = fixture(directory.path(), "launcher");
            let mut tree = Tree::spawn(&mut command).unwrap();
            let membership = tree.membership();
            let mut qemu = None;
            wait_for(|| {
                qemu = std::fs::read_to_string(directory.path().join("qemu.pid"))
                    .ok()
                    .and_then(|text| text.parse::<u32>().ok());
                qemu.is_some() && directory.path().join("ready").is_file()
            });
            let qemu = qemu.unwrap();
            assert_ne!(qemu, tree.child.id());
            assert!(membership.contains(tree.child.id()).unwrap());
            assert!(membership.contains(qemu).unwrap());
            assert!(!membership.contains(unrelated.child.id()).unwrap());
            if force {
                tree.kill().unwrap();
                wait_for(|| tree.try_wait().unwrap().is_some());
            }
            drop(tree);
            // Retaining an observer must never retain a living owned process.
            wait_for(|| membership.pids().unwrap().is_empty());
            assert!(unrelated.try_wait().unwrap().is_none());
        }
        let graceful_directory = tempfile::tempdir().unwrap();
        let mut command = fixture(graceful_directory.path(), "launcher");
        let mut graceful = Tree::spawn(&mut command).unwrap();
        wait_for(|| graceful_directory.path().join("ready").is_file());
        std::fs::write(graceful_directory.path().join("shutdown"), b"shutdown").unwrap();
        wait_for(|| graceful.try_wait().unwrap().is_some());
        assert!(!graceful.abandoned_descendants);
        assert!(unrelated.try_wait().unwrap().is_none());
        let orphan_directory = tempfile::tempdir().unwrap();
        let mut command = fixture(orphan_directory.path(), "orphan-launcher");
        let mut orphan = Tree::spawn(&mut command).unwrap();
        let membership = orphan.membership();
        let mut qemu = None;
        wait_for(|| {
            qemu = std::fs::read_to_string(orphan_directory.path().join("qemu.pid"))
                .ok()
                .and_then(|text| text.parse::<u32>().ok());
            qemu.is_some() && orphan_directory.path().join("ready").is_file()
        });
        assert!(membership.contains(qemu.unwrap()).unwrap());
        let orphan_wait_started = Instant::now();
        wait_for(|| orphan.try_wait().unwrap().is_some());
        assert!(orphan_wait_started.elapsed() >= DESCENDANT_EXIT_GRACE);
        assert!(orphan.abandoned_descendants);
        assert!(membership.pids().unwrap().is_empty());
        assert!(unrelated.try_wait().unwrap().is_none());
        unrelated.kill().unwrap();
        wait_for(|| unrelated.try_wait().unwrap().is_some());
    }
}
