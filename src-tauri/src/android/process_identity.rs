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

#[cfg(any(target_os = "linux", test))]
#[derive(Debug, PartialEq, Eq)]
struct LinuxStat {
    created: u64,
    state: u8,
}

#[cfg(any(target_os = "linux", test))]
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

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    #[ignore = "subprocess fixture for retained process handle test"]
    fn identity_fixture() {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }

    fn stat_fixture(state: &str, comm: &str) -> String {
        let mut fields = vec!["0"; 50];
        fields[0] = state;
        fields[6] = "18446744073709551615";
        fields[15] = "-5";
        fields[19] = "12345";
        format!("42 ({comm}) {}\n", fields.join(" "))
    }

    #[test]
    fn linux_stat_preserves_state_and_creation_with_comm_spaces_and_parentheses() {
        for comm in ["worker", "worker name (nested)", "worker ) (name) tail"] {
            for state in ["S", "Z", "X"] {
                assert_eq!(
                    parse_linux_stat(42, &stat_fixture(state, comm)).unwrap(),
                    LinuxStat {
                        created: 12345,
                        state: state.as_bytes()[0]
                    }
                );
            }
        }
    }

    #[test]
    fn linux_stat_rejects_wrong_pid_truncation_malformed_and_oversized_fields() {
        let valid = stat_fixture("Z", "worker");
        for invalid in [
            String::new(),
            valid.replace("42 (", "43 ("),
            valid.replace("42 (", "42 "),
            valid.replace(") Z ", ") ZZ "),
            valid.replace(") Z ", ") ? "),
            valid.replace("12345", "-1"),
            valid.replace("12345", "18446744073709551616"),
            valid.replace("-5", "unknown"),
            valid
                .split_whitespace()
                .take(51)
                .collect::<Vec<_>>()
                .join(" "),
            stat_fixture("Z", &"x".repeat(16384)),
        ] {
            assert!(parse_linux_stat(42, &invalid).is_err());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unreaped_child_zombie_is_exited_even_while_signal_zero_succeeds() {
        struct OwnedChild(std::process::Child);
        impl Drop for OwnedChild {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let mut child = OwnedChild(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "android::process_identity::tests::identity_fixture",
                    "--ignored",
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let identity = Identity::read(child.0.id()).unwrap();
        assert!(!absent(identity.pid));
        child.0.kill().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while linux_stat(identity.pid).unwrap().state != b'Z' {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(unsafe { libc::kill(identity.pid as i32, 0) }, 0);
        assert!(Identity::read(identity.pid).is_err());
        assert!(!identity.matches_or_exited().unwrap());
        drop(child);
        assert!(absent(identity.pid));
    }

    #[cfg(windows)]
    #[test]
    fn legacy_dos_executable_paths_match_canonical_native_identity() {
        let identity = Identity::read(std::process::id()).unwrap();
        let mut legacy = identity.clone();
        let path = identity.executable.to_str().unwrap();
        legacy.executable = PathBuf::from(path.strip_prefix(r"\\?\").unwrap_or(path));
        assert!(legacy.still_matches().unwrap());
        assert!(legacy.matches_or_exited().unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn exited_process_with_a_retained_handle_is_not_a_live_identity() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["process_identity::tests::identity_fixture", "--ignored"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let identity = Identity::read(child.id()).unwrap();
        child.kill().unwrap();
        // Keep Child's process handle open, as emulator.exe does for QEMU.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !absent(identity.pid) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!identity.matches_or_exited().unwrap());
        child.wait().unwrap();
    }

    #[test]
    fn native_process_creation_and_executable_are_required_for_a_match() {
        let identity = Identity::read(std::process::id()).unwrap();
        assert_eq!(
            std::fs::canonicalize(&identity.executable).unwrap(),
            std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap()
        );
        assert!(identity.created > 0);
        assert!(identity.still_matches().unwrap());
        let mut stale = identity.clone();
        stale.created += 1;
        assert!(!stale.still_matches().unwrap());
        stale = identity;
        stale.executable = PathBuf::from("not-the-managed-emulator");
        assert!(!stale.still_matches().unwrap());
        assert!(Identity::read(0).is_err());
        assert!(Identity::read(u32::MAX).is_err());
    }
}
