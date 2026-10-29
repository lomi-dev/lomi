use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Recovery must additionally verify the managed AVD and authenticated instance.
/// A PID alone can be reused after exit, including while a stale record remains.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Identity {
    pub pid: u32,
    pub executable: PathBuf,
    pub created: u64,
    pub boot: String,
}

impl Identity {
    pub fn read(pid: u32) -> Result<Self, String> {
        if pid == 0 || pid > i32::MAX as u32 {
            return Err("Invalid managed process ID".into());
        }
        native(pid).map_err(|error| format!("Cannot verify Android process identity: {error}"))
    }

    pub fn still_matches(&self) -> Result<bool, String> {
        Ok(self.matches(&Self::read(self.pid)?))
    }

    fn matches(&self, current: &Self) -> bool {
        if self.pid != current.pid || self.created != current.created || self.boot != current.boot {
            return false;
        }
        #[cfg(windows)]
        {
            // Older recovery records contain DOS paths, while native identity
            // now uses canonical extended-length paths. Compare native paths.
            self.executable
                .canonicalize()
                .ok()
                .is_some_and(|path| path == current.executable)
        }
        #[cfg(not(windows))]
        {
            self.executable == current.executable
        }
    }

    pub fn matches_or_exited(&self) -> Result<bool, String> {
        match Self::read(self.pid) {
            Ok(current) => Ok(self.matches(&current)),
            Err(_) if self.pid > 0 && self.pid <= i32::MAX as u32 && absent(self.pid) => Ok(false),
            Err(error) => Err(error),
        }
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
fn absent(pid: u32) -> bool {
    // Signal zero checks existence/permissions without delivering a signal.
    unsafe {
        libc::kill(pid as i32, 0) == -1
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }
}

#[cfg(target_os = "linux")]
fn absent(pid: u32) -> bool {
    let missing = unsafe { libc::kill(pid as i32, 0) } == -1
        && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
    // An unreaped child still answers signal zero, but cannot run or expose exe.
    // A failed/malformed proc read is not evidence that a live process exited.
    missing || linux_stat(pid).is_ok_and(|value| matches!(value.state, b'Z' | b'X'))
}

#[cfg(target_os = "linux")]
#[derive(Debug, PartialEq, Eq)]
struct LinuxStat {
    created: u64,
    state: u8,
}

#[cfg(target_os = "linux")]
fn parse_linux_stat(pid: u32, text: &str) -> Result<LinuxStat, String> {
    if pid == 0 || pid > i32::MAX as u32 || text.len() > 16384 {
        return Err("Invalid or oversized process stat".into());
    }
    // comm can contain spaces and parentheses; numeric fields follow its final ')'.
    let (command, fields) = text.rsplit_once(") ").ok_or("Malformed process stat")?;
    let fields: Vec<_> = fields.split_whitespace().collect();
    if !command.starts_with(&format!("{pid} ("))
        || fields.len() < 50
        || !matches!(
            fields[0],
            "R" | "S" | "D" | "Z" | "T" | "t" | "X" | "x" | "K" | "W" | "P" | "I"
        )
        || fields[1..].iter().any(|value| {
            !value
                .strip_prefix('-')
                .unwrap_or(value)
                .bytes()
                .all(|byte| byte.is_ascii_digit())
                || value.parse::<i128>().is_err()
        })
    {
        return Err("Malformed process stat".into());
    }
    Ok(LinuxStat {
        created: fields[19]
            .parse()
            .map_err(|_| "Invalid process start time")?,
        state: fields[0].as_bytes()[0],
    })
}

#[cfg(target_os = "linux")]
fn linux_stat(pid: u32) -> Result<LinuxStat, String> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(format!("/proc/{pid}/stat"))
        .map_err(|error| error.to_string())?
        .take(16385)
        .read_to_string(&mut text)
        .map_err(|error| error.to_string())?;
    parse_linux_stat(pid, &text)
}

#[cfg(windows)]
fn exited(handle: windows_sys::Win32::Foundation::HANDLE) -> Result<bool, String> {
    use windows_sys::Win32::{
        Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::WaitForSingleObject,
    };
    match unsafe { WaitForSingleObject(handle, 0) } {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        WAIT_FAILED => Err(std::io::Error::last_os_error().to_string()),
        _ => Err("Unexpected Android process wait result".into()),
    }
}

#[cfg(windows)]
fn absent(pid: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER},
        System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE},
    };
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    };
    if handle.is_null() {
        unsafe { GetLastError() == ERROR_INVALID_PARAMETER }
    } else {
        // An exited process can remain openable while its launcher still owns
        // a handle. Treat query/wait failures conservatively as not absent.
        let absent = exited(handle) == Ok(true);
        unsafe {
            CloseHandle(handle);
        }
        absent
    }
}

#[cfg(target_os = "linux")]
fn native(pid: u32) -> Result<Identity, String> {
    use std::fs;
    let root = PathBuf::from(format!("/proc/{pid}"));
    let before = linux_stat(pid)?;
    if matches!(before.state, b'Z' | b'X') {
        return Err("Process has exited".into());
    }
    let executable = fs::read_link(root.join("exe")).map_err(|e| e.to_string())?;
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").map_err(|e| e.to_string())?;
    let after = linux_stat(pid)?;
    if after.created != before.created || matches!(after.state, b'Z' | b'X') {
        return Err("Process changed during identity lookup".into());
    }
    Ok(Identity {
        pid,
        executable,
        created: before.created,
        boot: boot.trim().into(),
    })
}

#[cfg(target_os = "macos")]
fn native(pid: u32) -> Result<Identity, String> {
    use std::{
        ffi::{c_char, c_int, c_void, CStr},
        os::unix::ffi::OsStrExt,
    };
    // rusage_info_v0 from the macOS SDK: UUID followed by ten uint64_t counters.
    #[repr(C)]
    #[derive(Default)]
    struct Usage {
        uuid: [u8; 16],
        counters: [u64; 10],
    }
    unsafe extern "C" {
        fn proc_pidpath(pid: c_int, buffer: *mut c_void, size: u32) -> c_int;
        fn proc_pid_rusage(pid: c_int, flavor: c_int, buffer: *mut c_void) -> c_int;
        fn sysctlbyname(
            name: *const c_char,
            output: *mut c_void,
            size: *mut usize,
            input: *mut c_void,
            input_size: usize,
        ) -> c_int;
    }
    let read = || -> Result<Usage, String> {
        let mut usage = Usage::default();
        if unsafe { proc_pid_rusage(pid as c_int, 0, (&mut usage as *mut Usage).cast()) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if usage.counters[9] != 0 {
            return Err("Process has exited and must be reaped".into());
        }
        Ok(usage)
    };
    let before = read()?;
    let mut path = [0u8; 4096];
    if unsafe { proc_pidpath(pid as c_int, path.as_mut_ptr().cast(), path.len() as u32) } <= 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let path = CStr::from_bytes_until_nul(&path).map_err(|e| e.to_string())?;
    let executable = PathBuf::from(std::ffi::OsStr::from_bytes(path.to_bytes()));
    let mut boot = [0u8; 128];
    let mut length = boot.len();
    if unsafe {
        sysctlbyname(
            c"kern.bootsessionuuid".as_ptr(),
            boot.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    } != 0
        || length > boot.len()
    {
        return Err("Cannot read the host boot identity".into());
    }
    let boot = CStr::from_bytes_until_nul(&boot[..length])
        .map_err(|e| e.to_string())?
        .to_str()
        .map_err(|e| e.to_string())?
        .to_owned();
    let after = read()?;
    if before.counters[8] != after.counters[8] || before.uuid != after.uuid {
        return Err("Process changed during identity lookup".into());
    }
    Ok(Identity {
        pid,
        executable,
        created: before.counters[8],
        boot,
    })
}

#[cfg(windows)]
fn native(pid: u32) -> Result<Identity, String> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, FILETIME, HANDLE},
        System::Threading::{
            GetProcessTimes, OpenProcess, QueryFullProcessImageNameW,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
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
    let handle = Handle(unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    });
    if handle.0.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut created: FILETIME = unsafe { std::mem::zeroed() };
    let mut exit = created;
    let mut kernel = created;
    let mut user = created;
    if unsafe { GetProcessTimes(handle.0, &mut created, &mut exit, &mut kernel, &mut user) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if exited(handle.0)? {
        return Err("Process has exited".into());
    }
    let mut path = vec![0u16; 32768];
    let mut length = path.len() as u32;
    if unsafe { QueryFullProcessImageNameW(handle.0, 0, path.as_mut_ptr(), &mut length) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let executable = PathBuf::from(std::ffi::OsString::from_wide(&path[..length as usize]))
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if exited(handle.0)? {
        return Err("Process exited during identity lookup".into());
    }
    Ok(Identity {
        pid,
        // Match canonical SDK paths, including Windows extended-length prefixes.
        executable,
        created: (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
        // Windows creation FILETIME is an absolute timestamp, not boot-relative ticks.
        boot: String::new(),
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn native(_: u32) -> Result<Identity, String> {
    Err("Host process identity is unavailable".into())
}
