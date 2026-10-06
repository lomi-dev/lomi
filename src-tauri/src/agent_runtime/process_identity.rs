use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Recovery must additionally verify the owned process group and immutable account binding.
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
        native(pid).map_err(|error| format!("Cannot verify native process identity: {error}"))
    }

    pub fn still_matches(&self) -> Result<bool, String> {
        Ok(Self::read(self.pid)? == *self)
    }

    pub fn matches_or_exited(&self) -> Result<bool, String> {
        match Self::read(self.pid) {
            Ok(current) => Ok(current == *self),
            Err(_) if self.pid > 0 && self.pid <= i32::MAX as u32 && absent(self.pid) => Ok(false),
            Err(error) => Err(error),
        }
    }
}

#[cfg(unix)]
fn absent(pid: u32) -> bool {
    // Signal zero checks existence/permissions without delivering a signal.
    unsafe {
        libc::kill(pid as i32, 0) == -1
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }
}

#[cfg(windows)]
fn absent(pid: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER},
        System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        unsafe { GetLastError() == ERROR_INVALID_PARAMETER }
    } else {
        unsafe {
            CloseHandle(handle);
        }
        false
    }
}

#[cfg(target_os = "linux")]
fn native(pid: u32) -> Result<Identity, String> {
    use std::{fs, io::Read};
    let root = PathBuf::from(format!("/proc/{pid}"));
    let read = || -> Result<u64, String> {
        let mut text = String::new();
        fs::File::open(root.join("stat"))
            .map_err(|e| e.to_string())?
            .take(16385)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
        if text.len() > 16384 {
            return Err("Process stat exceeds its bound".into());
        }
        // comm can contain spaces and parentheses; numeric fields follow its final ')'.
        text.rsplit_once(") ")
            .and_then(|(_, fields)| fields.split_whitespace().nth(19))
            .and_then(|value| value.parse().ok())
            .ok_or("Missing process start time".into())
    };
    let created = read()?;
    let executable = fs::read_link(root.join("exe")).map_err(|e| e.to_string())?;
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").map_err(|e| e.to_string())?;
    if read()? != created {
        return Err("Process changed during identity lookup".into());
    }
    Ok(Identity {
        pid,
        executable,
        created,
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
    let before_audit = AuditToken::fresh(pid)?;
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
    let after_audit = AuditToken::fresh(pid)?;
    if !before_audit.matches(&after_audit)
        || before.counters[8] != after.counters[8]
        || before.uuid != after.uuid
    {
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
            PROCESS_QUERY_LIMITED_INFORMATION,
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
    let handle = Handle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) });
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
    if exit.dwHighDateTime != 0 || exit.dwLowDateTime != 0 {
        return Err("Process has exited".into());
    }
    let mut path = vec![0u16; 32768];
    let mut length = path.len() as u32;
    if unsafe { QueryFullProcessImageNameW(handle.0, 0, path.as_mut_ptr(), &mut length) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(Identity {
        pid,
        executable: PathBuf::from(std::ffi::OsString::from_wide(&path[..length as usize])),
        created: (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
        // Windows creation FILETIME is an absolute timestamp, not boot-relative ticks.
        boot: String::new(),
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn native(_: u32) -> Result<Identity, String> {
    Err("Host process identity is unavailable".into())
}

/// A fresh kernel audit identity, used only for macOS PID-reuse-safe signals.
/// Never obtain both inspection brackets from one retained task-name port:
/// a stale port continues to describe its old task after numeric PID reuse.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub(super) struct AuditToken {
    values: [u32; 8],
}
#[cfg(target_os = "macos")]
impl AuditToken {
    pub(super) fn fresh(pid: u32) -> Result<Self, String> {
        if pid == 0 || pid > i32::MAX as u32 {
            return Err("Invalid audited process ID.".into());
        }
        unsafe extern "C" {
            static mach_task_self_: u32;
            fn task_name_for_pid(target: u32, pid: i32, port: *mut u32) -> i32;
            fn mach_port_deallocate(task: u32, port: u32) -> i32;
        }
        struct Port(u32);
        impl Drop for Port {
            fn drop(&mut self) {
                unsafe {
                    mach_port_deallocate(mach_task_self_, self.0);
                }
            }
        }
        let mut port = 0;
        let status = unsafe { task_name_for_pid(mach_task_self_, pid as i32, &mut port) };
        if status != 0 || port == 0 {
            return Err("Cannot obtain a fresh descendant audit identity.".into());
        }
        let port = Port(port);
        let mut token = Self { values: [0; 8] };
        // SDK mach/task_info.h: TASK_AUDIT_TOKEN returns eight natural_t words.
        const TASK_AUDIT_TOKEN: u32 = 15;
        let mut count = 8;
        let status = unsafe {
            libc::task_info(
                port.0,
                TASK_AUDIT_TOKEN,
                token.values.as_mut_ptr().cast(),
                &mut count,
            )
        };
        if status != 0 || count != 8 || token.pid() != pid || !token.owned() {
            return Err("Cannot establish the audited descendant owner.".into());
        }
        Ok(token)
    }
    pub(super) fn pid(&self) -> u32 {
        unsafe { audit_token_to_pid(*self) as u32 }
    }
    pub(super) fn pid_version(&self) -> u32 {
        unsafe { audit_token_to_pidversion(*self) as u32 }
    }
    fn owned(&self) -> bool {
        unsafe {
            audit_token_to_euid(*self) == libc::geteuid()
                && audit_token_to_ruid(*self) == libc::getuid()
        }
    }
    pub(super) fn matches(&self, other: &Self) -> bool {
        self.pid() == other.pid()
            && self.pid_version() == other.pid_version()
            && self.owned()
            && other.owned()
            && self == other
    }
    /// XNU verifies PID version while retaining the target process reference;
    /// no raw PID signal is used after the ownership inspection.
    pub(super) fn signal(&self, signal: i32) -> Result<(), String> {
        if self.pid() == 0 || self.pid() > i32::MAX as u32 || !self.owned() {
            return Err("Refusing to signal an unauthenticated process identity.".into());
        }
        self.signal_using(signal, c"proc_signal_with_audittoken")
    }
    fn signal_using(&self, signal: i32, name: &std::ffi::CStr) -> Result<(), String> {
        struct Library(*mut std::ffi::c_void);
        impl Drop for Library {
            fn drop(&mut self) {
                unsafe {
                    libc::dlclose(self.0);
                }
            }
        }
        let library = Library(unsafe {
            libc::dlopen(
                c"/usr/lib/libSystem.B.dylib".as_ptr(),
                libc::RTLD_NOW | libc::RTLD_LOCAL,
            )
        });
        if library.0.is_null() {
            return Err(
                "Exact audited signalling is unavailable; ownership remains protected.".into(),
            );
        }
        let symbol = unsafe { libc::dlsym(library.0, name.as_ptr()) };
        if symbol.is_null() {
            return Err(
                "Exact audited signalling is unavailable; ownership remains protected.".into(),
            );
        }
        let send: unsafe extern "C" fn(*mut AuditToken, i32) -> i32 =
            unsafe { std::mem::transmute(symbol) };
        let mut token = *self;
        let error = unsafe { send(&mut token, signal) };
        if error != 0 {
            return Err(format!(
                "Cannot signal the exact audited descendant: OS error {error}."
            ));
        }
        Ok(())
    }
}
#[cfg(target_os = "macos")]
#[link(name = "bsm")]
unsafe extern "C" {
    fn audit_token_to_pid(token: AuditToken) -> i32;
    fn audit_token_to_pidversion(token: AuditToken) -> i32;
    fn audit_token_to_euid(token: AuditToken) -> u32;
    fn audit_token_to_ruid(token: AuditToken) -> u32;
}

#[cfg(all(test, target_os = "macos"))]
mod audit_tests {
    use super::*;
    use std::process::{Command, Stdio};
    fn child() -> std::process::Child {
        Command::new("/bin/sleep")
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }
    #[test]
    fn exact_audit_signal_stops_only_the_owned_fixture() {
        let mut target = child();
        let mut unrelated = child();
        let target_identity = Identity::read(target.id()).unwrap();
        let unrelated_identity = Identity::read(unrelated.id()).unwrap();
        let token = AuditToken::fresh(target.id()).unwrap();
        let fresh = AuditToken::fresh(target.id()).unwrap();
        assert!(token.matches(&fresh));
        token.signal(libc::SIGKILL).unwrap();
        let result = target.wait().unwrap();
        let survived = unrelated_identity.still_matches().unwrap();
        unrelated.kill().unwrap();
        unrelated.wait().unwrap();
        assert!(!result.success());
        assert!(survived);
        assert!(!target_identity.matches_or_exited().unwrap());
    }
    #[test]
    fn wrong_pid_version_is_rejected_by_kernel_without_signalling_live_fixture() {
        let mut target = child();
        let identity = Identity::read(target.id()).unwrap();
        let valid = AuditToken::fresh(target.id()).unwrap();
        let mut wrong = valid;
        wrong.values[7] ^= 1;
        assert_ne!(wrong.pid_version(), valid.pid_version());
        let result = wrong.signal(libc::SIGKILL);
        let survived = identity.still_matches().unwrap();
        target.kill().unwrap();
        target.wait().unwrap();
        assert!(result.is_err());
        assert!(survived);
    }
    #[test]
    fn a_retained_exited_token_cannot_replace_a_fresh_second_bracket() {
        let mut target = child();
        let token = AuditToken::fresh(target.id()).unwrap();
        target.kill().unwrap();
        target.wait().unwrap();
        assert!(AuditToken::fresh(target.id()).is_err());
        assert!(token.signal(libc::SIGKILL).is_err());
    }
    #[test]
    fn unavailable_audit_signal_symbol_fails_closed_without_pid_fallback() {
        let mut target = child();
        let identity = Identity::read(target.id()).unwrap();
        let token = AuditToken::fresh(target.id()).unwrap();
        let result = token.signal_using(libc::SIGKILL, c"lomi_fixture_missing_audit_signal_symbol");
        let survived = identity.still_matches().unwrap();
        target.kill().unwrap();
        target.wait().unwrap();
        assert!(result.is_err());
        assert!(survived);
    }
}
