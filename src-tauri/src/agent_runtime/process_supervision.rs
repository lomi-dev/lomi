//! Ownership witnesses for descendants which leave the original process group.
//!
//! The admitted client/tool launch must preserve the owner environment entry.
//! This is not OS containment of processes which deliberately scrub ownership.
//! Enumeration failures are unknown ownership, never evidence of settlement.
use super::process_identity::Identity;
use std::time::{Duration, Instant};

pub(super) const OWNER_ENV: &str = "LOMI_AGENT_ATTEMPT_OWNER";

fn validate(marker: &str) -> Result<(), String> {
    if marker.len() != 64 || !marker.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Invalid native process ownership witness.".into());
    }
    Ok(())
}

pub(super) fn remaining(marker: &str) -> Result<Vec<Identity>, String> {
    remaining_from_pids(marker, &enumerate()?)
}

pub(super) fn remaining_from_pids(marker: &str, pids: &[u32]) -> Result<Vec<Identity>, String> {
    validate(marker)?;
    let mut owned = Vec::new();
    for &pid in pids {
        if pid == 0 || !same_user_live(pid)? {
            continue;
        }

        #[cfg(target_os = "macos")]
        let before_audit = match super::process_identity::AuditToken::fresh(pid) {
            Ok(token) => token,
            Err(_) if !same_user_live(pid)? => continue,
            Err(error) => return Err(error),
        };
        let before = match Identity::read(pid) {
            Ok(identity) => identity,
            Err(_) if !same_user_live(pid)? => continue,
            Err(_) => return Err("Cannot establish descendant process identity.".into()),
        };

        let matched = match has_owner(pid, marker) {
            Ok(matched) => matched,
            Err(_) if !same_user_live(pid)? => continue,
            Err(_) => {
                return Err("Cannot inspect descendant ownership; recovery remains fenced.".into())
            }
        };
        if matched {
            match before.still_matches() {
                Ok(true) => {}
                Err(_) if !same_user_live(pid)? => continue,
                _ => return Err("Descendant identity changed during ownership inspection.".into()),
            }
            #[cfg(target_os = "macos")]
            {
                let after_audit = match super::process_identity::AuditToken::fresh(pid) {
                    Ok(token) => token,
                    Err(_) if !same_user_live(pid)? => continue,
                    Err(error) => return Err(error),
                };
                if !before_audit.matches(&after_audit) {
                    return Err(
                        "Descendant audit identity changed during ownership inspection.".into(),
                    );
                }
            }
            owned.push(before);
        }
    }
    Ok(owned)
}

pub(super) fn stop(marker: &str) -> Result<(), String> {
    stop_using(marker, enumerate)
}

fn stop_using(marker: &str, discover: impl Fn() -> Result<Vec<u32>, String>) -> Result<(), String> {
    validate(marker)?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let remaining = remaining_from_pids(marker, &discover()?)?;
        if remaining.is_empty() {
            return Ok(());
        }
        for identity in remaining {
            #[cfg(target_os = "macos")]
            signal_owned(&identity, marker)?;
            #[cfg(not(target_os = "macos"))]
            {
                // Preserve the separately qualified non-Darwin path.
                if !identity.matches_or_exited()? {
                    continue;
                }
                let matches = match has_owner(identity.pid, marker) {
                    Ok(matches) => matches,
                    Err(_) if !same_user_live(identity.pid)? => continue,
                    Err(error) => return Err(error),
                };
                if matches {
                    let result = unsafe { libc::kill(identity.pid as i32, libc::SIGKILL) };
                    if result != 0 && !gone(identity.pid) {
                        return Err(
                            "Cannot stop an owned descendant; recovery remains fenced.".into()
                        );
                    }
                }
            }
        }
        if Instant::now() >= deadline {
            return Err("Owned descendants did not settle before the deadline.".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(target_os = "macos")]
fn signal_owned(identity: &Identity, marker: &str) -> Result<(), String> {
    use super::process_identity::AuditToken;
    let before = match AuditToken::fresh(identity.pid) {
        Ok(token) => token,
        Err(_) if !same_user_live(identity.pid)? => return Ok(()),
        Err(error) => return Err(error),
    };
    if !identity.matches_or_exited()? {
        return Ok(());
    }
    if !same_user_live(identity.pid)? {
        return Ok(());
    }

    let matches = match has_owner(identity.pid, marker) {
        Ok(matches) => matches,
        Err(_) if !same_user_live(identity.pid)? => return Ok(()),
        Err(error) => return Err(error),
    };

    let after = match AuditToken::fresh(identity.pid) {
        Ok(token) => token,
        Err(_) if !same_user_live(identity.pid)? => return Ok(()),
        Err(error) => return Err(error),
    };
    if !before.matches(&after) {
        return Err("Descendant changed during audited stop; recovery remains fenced.".into());
    }
    if matches {
        if let Err(error) = after.signal(libc::SIGKILL) {
            if same_user_live(identity.pid)? {
                return Err(error);
            }
        }
    }
    Ok(())
}

fn gone(pid: u32) -> bool {
    unsafe {
        libc::kill(pid as i32, 0) < 0
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }
}

#[cfg(target_os = "macos")]
fn enumerate() -> Result<Vec<u32>, String> {
    let mut capacity = 4096usize;
    loop {
        let mut pids = vec![0u32; capacity];
        let size = pids.len() * std::mem::size_of::<u32>();
        let bytes = unsafe { libc::proc_listpids(1, 0, pids.as_mut_ptr().cast(), size as i32) };
        if bytes < 0 || !(bytes as usize).is_multiple_of(std::mem::size_of::<u32>()) {
            return Err("Cannot enumerate supervised processes.".into());
        }
        if (bytes as usize) < size {
            pids.truncate(bytes as usize / std::mem::size_of::<u32>());
            return Ok(pids);
        }
        capacity *= 2;
        if capacity > 262144 {
            return Err("Process enumeration exceeds its qualification bound.".into());
        }
    }
}

#[cfg(target_os = "linux")]
fn enumerate() -> Result<Vec<u32>, String> {
    let entries =
        std::fs::read_dir("/proc").map_err(|_| "Cannot enumerate supervised processes.")?;
    let mut pids = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| "Cannot enumerate supervised processes.")?;
        if let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse().ok())
        {
            pids.push(pid);
        }
        if pids.len() > 262144 {
            return Err("Process enumeration exceeds its qualification bound.".into());
        }
    }
    Ok(pids)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn enumerate() -> Result<Vec<u32>, String> {
    Err("Descendant ownership is not qualified on this platform.".into())
}

#[cfg(target_os = "macos")]
fn same_user_live(pid: u32) -> Result<bool, String> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    let result = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size as i32,
        )
    };
    if result != size as i32 {
        return if gone(pid) {
            Ok(false)
        } else {
            Err("Cannot establish process ownership.".into())
        };
    }
    let info = unsafe { info.assume_init() };
    Ok(info.pbi_uid == unsafe { libc::geteuid() } && info.pbi_status != 5)
}

#[cfg(target_os = "linux")]
fn same_user_live(pid: u32) -> Result<bool, String> {
    use std::os::unix::fs::MetadataExt;
    let root = std::path::PathBuf::from(format!("/proc/{pid}"));
    let metadata = match std::fs::metadata(&root) {
        Ok(metadata) => metadata,
        Err(_) if gone(pid) => return Ok(false),
        Err(_) => return Err("Cannot establish process ownership.".into()),
    };
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Ok(false);
    }

    let stat = match std::fs::read_to_string(root.join("stat")) {
        Ok(stat) => stat,
        Err(_) if gone(pid) => return Ok(false),
        Err(_) => return Err("Cannot inspect descendant state.".into()),
    };
    Ok(!matches!(
        stat.rsplit_once(") ")
            .and_then(|(_, tail)| tail.as_bytes().first()),
        Some(b'Z' | b'X')
    ))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn same_user_live(_: u32) -> Result<bool, String> {
    Err("Descendant ownership is not qualified on this platform.".into())
}

#[cfg(target_os = "macos")]
fn has_owner(pid: u32, marker: &str) -> Result<bool, String> {
    // KERN_PROCARGS2 returns argc, executable, argv, then nul-delimited env.
    // Never expose or log that buffer; only the exact ownership entry escapes.
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as i32];
    let capacity = unsafe { libc::sysconf(libc::_SC_ARG_MAX) };
    if !(4096..=2 * 1024 * 1024).contains(&capacity) {
        return Err("Cannot establish the process environment bound.".into());
    }
    let mut bytes = vec![0u8; capacity as usize];
    let mut size = bytes.len();
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as u32,
            bytes.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return Err(format!(
            "Cannot inspect descendant ownership: {}",
            std::io::Error::last_os_error()
        ));
    }
    if size > bytes.len() {
        return Err("Process environment exceeds its qualification bound.".into());
    }
    bytes.truncate(size);
    let result = macos_owner_entry(&bytes, marker);
    bytes.fill(0);
    result
}

#[cfg(target_os = "macos")]
fn macos_owner_entry(bytes: &[u8], marker: &str) -> Result<bool, String> {
    let argc = i32::from_ne_bytes(
        bytes
            .get(..4)
            .ok_or("Invalid process argument record.")?
            .try_into()
            .map_err(|_| "Invalid process argument record.")?,
    );
    if !(0..=262144).contains(&argc) {
        return Err("Invalid process argument record.".into());
    }
    let mut rest = bytes.get(4..).ok_or("Invalid process argument record.")?;
    let executable_end = rest
        .iter()
        .position(|byte| *byte == 0)
        .ok_or("Incomplete process argument record.")?;
    rest = &rest[executable_end..];
    while rest.first() == Some(&0) {
        rest = &rest[1..];
    }
    for _ in 0..argc {
        let end = rest
            .iter()
            .position(|byte| *byte == 0)
            .ok_or("Incomplete process argument record.")?;
        rest = &rest[end + 1..];
    }
    if rest.iter().all(|byte| *byte == 0) {
        return Err("The OS omitted this process environment; ownership is unknown.".into());
    }
    Ok(env_contains(rest, marker))
}

fn env_contains(bytes: &[u8], marker: &str) -> bool {
    let entry = format!("{OWNER_ENV}={marker}");
    bytes
        .split(|byte| *byte == 0)
        .any(|value| value == entry.as_bytes())
}

#[cfg(target_os = "linux")]
fn has_owner(pid: u32, marker: &str) -> Result<bool, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(format!("/proc/{pid}/environ"))
        .and_then(|file| file.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes))
        .map_err(|_| "Cannot inspect descendant ownership.")?;
    if bytes.len() > 2 * 1024 * 1024 {
        bytes.fill(0);
        return Err("Process environment exceeds its qualification bound.".into());
    }
    let matched = env_contains(&bytes, marker);
    bytes.fill(0);
    Ok(matched)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn has_owner(_: u32, _: &str) -> Result<bool, String> {
    Err("Descendant ownership is not qualified on this platform.".into())
}
