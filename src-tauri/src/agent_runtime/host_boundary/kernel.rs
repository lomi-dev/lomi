//! Private coalition ABI is admitted only on the exact qualified host kernel.
use super::{failure, Identity};
use crate::agent_runtime::process_identity::AuditToken;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct HostWitness {
    pub boot: String,
    pub release: String,
    pub build: String,
    pub version: String,
}
impl HostWitness {
    pub(super) fn admitted() -> Result<Self, String> {
        fn read(name: &std::ffi::CStr) -> Result<String, String> {
            let mut out = [0u8; 4096];
            let mut n = out.len();
            if unsafe {
                libc::sysctlbyname(
                    name.as_ptr(),
                    out.as_mut_ptr().cast(),
                    &mut n,
                    std::ptr::null_mut(),
                    0,
                )
            } != 0
                || n == 0
                || n > out.len()
            {
                return Err(failure());
            }
            std::ffi::CStr::from_bytes_until_nul(&out[..n])
                .map_err(|_| failure())?
                .to_str()
                .map(str::to_owned)
                .map_err(|_| failure())
        }
        let value = Self {
            boot: read(c"kern.bootsessionuuid")?,
            release: read(c"kern.osrelease")?,
            build: read(c"kern.osversion")?,
            version: read(c"kern.version")?,
        };
        if !cfg!(target_arch = "aarch64")
            || value.release != "27.0.0"
            || value.build != "26A434"
            || value.version != "Darwin Kernel Version 27.0.0: Tue Aug 11 21:05:41 PDT 2026; root:xnu-13432.1.9~1/RELEASE_ARM64_T8122"
            || value.boot.is_empty()
        {
            return Err(
                "The experimental host boundary is not admitted on this kernel/ABI.".into(),
            );
        }
        Ok(value)
    }
}
#[cfg(test)]
pub(crate) fn production_qualified() -> bool {
    false
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Member {
    pub identity: Identity,
    pub pid_version: u32,
    pub resource_coalition: u64,
}
#[repr(C)]
#[derive(Default)]
struct CoalitionInfo {
    ids: [u64; 2],
    reserved: [u64; 3],
}
// Called between fork and exec: fixed stack storage and the OS query only.
pub(super) fn inherited_coalition(expected: u64) -> std::io::Result<()> {
    let mut info = CoalitionInfo::default();
    let bytes = unsafe {
        libc::proc_pidinfo(
            libc::getpid(),
            20,
            0,
            (&mut info as *mut CoalitionInfo).cast(),
            std::mem::size_of::<CoalitionInfo>() as i32,
        )
    };
    if bytes == 40 && expected > 1 && info.ids[0] == expected && info.reserved == [0; 3] {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(libc::EPERM))
    }
}
pub(crate) fn member(pid: u32) -> Result<Member, String> {
    let before = AuditToken::fresh(pid)?;
    let identity = Identity::read(pid)?;
    let mut info = CoalitionInfo::default();
    let size = std::mem::size_of::<CoalitionInfo>();
    if size != 40 {
        return Err(failure());
    }
    let bytes = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            20,
            0,
            (&mut info as *mut CoalitionInfo).cast(),
            size as i32,
        )
    };
    let after = AuditToken::fresh(pid)?;
    if bytes != 40 || info.ids[0] <= 1 || info.reserved != [0; 3] || !before.matches(&after) {
        return Err(failure());
    }
    Ok(Member {
        identity,
        pid_version: after.pid_version(),
        resource_coalition: info.ids[0],
    })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Usage {
    Alive,
    Retired,
}
pub(super) fn usage(cid: u64) -> Result<Usage, String> {
    if cid <= 1 {
        return Err(failure());
    }
    HostWitness::admitted()?;
    // Private symbol availability is checked dynamically. The explicit
    // libSystem reference remains live across the call; no unsupported symbol
    // can become a dyld requirement for the normal application.
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
        return Err(failure());
    }
    let symbol = unsafe { libc::dlsym(library.0, c"coalition_info_resource_usage".as_ptr()) };
    if symbol.is_null() {
        return Err(failure());
    }
    let query: unsafe extern "C" fn(u64, *mut std::ffi::c_void, usize) -> i32 =
        unsafe { std::mem::transmute(symbol) };
    // Only presence/ESRCH is read; counters are never a settlement proof.
    let mut ignored_counters = [0u64; 2];
    let status = unsafe {
        query(
            cid,
            ignored_counters.as_mut_ptr().cast(),
            std::mem::size_of_val(&ignored_counters),
        )
    };
    if status == 0 {
        return Ok(Usage::Alive);
    }
    if status == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
        Ok(Usage::Retired)
    } else {
        Err(failure())
    }
}
pub(crate) fn signal_member(expected: &Member, cid: u64) -> Result<(), String> {
    signal_member_with(expected, cid, libc::SIGKILL)
}
pub(crate) fn signal_member_with(expected: &Member, cid: u64, signal: i32) -> Result<(), String> {
    if ![libc::SIGKILL, libc::SIGTERM, libc::SIGINT].contains(&signal)
        || cid <= 1
        || cid != expected.resource_coalition
    {
        return Err(failure());
    }
    let before = AuditToken::fresh(expected.identity.pid)?;
    let current = member(expected.identity.pid)?;
    let after = AuditToken::fresh(expected.identity.pid)?;
    if current != *expected
        || !before.matches(&after)
        || after.pid_version() != expected.pid_version
    {
        return Err(failure());
    }
    after.signal(signal)
}
pub(super) fn gone(pid: u32) -> bool {
    unsafe {
        libc::kill(pid as i32, 0) < 0
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }
}
#[cfg(not(test))]
pub(super) fn enumerate() -> Result<Vec<u32>, String> {
    let mut cap = 4096usize;
    loop {
        let mut pids = vec![0u32; cap];
        let bytes = unsafe {
            libc::proc_listpids(
                4,
                libc::geteuid(),
                pids.as_mut_ptr().cast(),
                (cap * 4) as i32,
            )
        };
        if bytes < 0 || bytes % 4 != 0 {
            return Err(failure());
        }
        if (bytes as usize) < cap * 4 {
            pids.truncate(bytes as usize / 4);
            return Ok(pids);
        }
        cap *= 2;
        if cap > 262144 {
            return Err(failure());
        }
    }
}
#[cfg(test)]
pub(super) fn enumerate() -> Result<Vec<u32>, String> {
    Err("Tests must supply only owned fixture PIDs; broad inspection is forbidden.".into())
}
