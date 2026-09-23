use std::{
    io,
    process::{Child, Command, ExitStatus},
};

/// Own one explicitly spawned process group. On Unix the group
/// leader is not reaped until its descendants exit, preventing PGID reuse.
pub struct Tree {
    pub child: Child,
    exit: Option<ExitStatus>,
    killed: bool,
    pub abandoned_descendants: bool,
    #[cfg(windows)]
    job: windows::Job,
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
            job,
        })
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
            if self.job.active()? != 0 {
                if !self.killed {
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
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD,
                THREADENTRY32,
            },
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
                JobObjectExtendedLimitInformation, QueryInformationJobObject,
                SetInformationJobObject, TerminateJobObject,
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
            Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
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
    impl Job {
        pub fn kill(&self) -> io::Result<()> {
            if unsafe { TerminateJobObject(self.0 .0, 1) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
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
