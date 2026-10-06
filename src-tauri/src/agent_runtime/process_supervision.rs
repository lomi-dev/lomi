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
        #[cfg(test)]
        inspection_point(pid, InspectionPhase::BeforeIdentity);
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
        #[cfg(test)]
        inspection_point(pid, InspectionPhase::BeforeEnvironment);
        let matched = match has_owner(pid, marker) {
            Ok(matched) => matched,
            Err(_) if !same_user_live(pid)? => continue,
            Err(_) => {
                return Err("Cannot inspect descendant ownership; recovery remains fenced.".into())
            }
        };
        if matched {
            #[cfg(test)]
            inspection_point(pid, InspectionPhase::BeforeIdentityFence);
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

#[cfg(test)]
fn stop_from_pids(marker: &str, pids: &[u32]) -> Result<(), String> {
    stop_using(marker, || Ok(pids.to_vec()))
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
    #[cfg(test)]
    inspection_point(identity.pid, InspectionPhase::BeforeSignalEnvironment);
    let matches = match has_owner(identity.pid, marker) {
        Ok(matches) => matches,
        Err(_) if !same_user_live(identity.pid)? => return Ok(()),
        Err(error) => return Err(error),
    };
    #[cfg(test)]
    inspection_point(identity.pid, InspectionPhase::BeforeSignalAuditFence);
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

#[cfg(all(not(test), target_os = "macos"))]
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

#[cfg(all(not(test), target_os = "linux"))]
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

#[cfg(all(not(test), not(any(target_os = "linux", target_os = "macos"))))]
fn enumerate() -> Result<Vec<u32>, String> {
    Err("Descendant ownership is not qualified on this platform.".into())
}

#[cfg(test)]
thread_local! {
    static TEST_PIDS: std::cell::RefCell<Option<Vec<u32>>> = const { std::cell::RefCell::new(None) };
    static INSPECTION_HOOK: std::cell::RefCell<Option<InspectionHook>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// Keep the before/after timing explicit across platform-specific inspection seams.
#[allow(clippy::enum_variant_names)]
enum InspectionPhase {
    BeforeIdentity,
    BeforeEnvironment,
    BeforeIdentityFence,
    #[cfg(target_os = "macos")]
    BeforeSignalEnvironment,
    #[cfg(target_os = "macos")]
    BeforeSignalAuditFence,
    #[cfg(target_os = "linux")]
    AfterLinuxMetadata,
}
#[cfg(test)]
struct InspectionHook {
    pid: u32,
    phase: InspectionPhase,
    run: Box<dyn FnOnce()>,
}
#[cfg(test)]
fn inspection_point(pid: u32, phase: InspectionPhase) {
    let hook = INSPECTION_HOOK.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot
            .as_ref()
            .is_some_and(|hook| hook.pid == pid && hook.phase == phase)
        {
            slot.take()
        } else {
            None
        }
    });
    if let Some(hook) = hook {
        (hook.run)();
    }
}
#[cfg(test)]
fn enumerate() -> Result<Vec<u32>, String> {
    TEST_PIDS.with(|pids| pids.borrow().clone().ok_or("Tests must inject explicitly owned fixture PIDs; scanning unrelated process environments is forbidden.".into()))
}
#[cfg(test)]
pub(super) fn with_test_pids<T>(pids: &[u32], run: impl FnOnce() -> T) -> T {
    struct Restore(Option<Vec<u32>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_PIDS.with(|pids| *pids.borrow_mut() = self.0.take());
        }
    }
    let old = TEST_PIDS.with(|list| list.replace(Some(pids.to_vec())));
    let _restore = Restore(old);
    run()
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
    #[cfg(test)]
    inspection_point(pid, InspectionPhase::AfterLinuxMetadata);
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

#[cfg(any(target_os = "macos", test))]
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        os::unix::process::CommandExt,
        process::{Command, Stdio},
    };

    #[test]
    fn environment_parser_matches_only_an_exact_entry_after_argv() {
        let marker = "a".repeat(64);
        let mut bytes = 2i32.to_ne_bytes().to_vec();
        bytes.extend_from_slice(b"/bin/sh\0\0/bin/sh\0-c\0");
        bytes.extend_from_slice(
            format!("OTHER={OWNER_ENV}={marker}\0{OWNER_ENV}={marker}\0").as_bytes(),
        );
        assert!(macos_owner_entry(&bytes, &marker).unwrap());
        assert!(!macos_owner_entry(&bytes, &"b".repeat(64)).unwrap());
        assert!(!env_contains(
            format!("OTHER={OWNER_ENV}={marker}\0").as_bytes(),
            &marker
        ));
        assert!(macos_owner_entry(&[0], &marker).is_err());
    }

    #[test]
    fn tests_refuse_unscoped_environment_enumeration() {
        assert!(remaining(&"a".repeat(64)).is_err());
        with_test_pids(&[], || {
            assert!(remaining(&"a".repeat(64)).unwrap().is_empty())
        });
        assert!(remaining(&"a".repeat(64)).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn platform_binary_without_environment_visibility_never_proves_marker_absent() {
        let marker = "f".repeat(64);
        let mut child = Command::new("/bin/sleep")
            .arg("60")
            .env_clear()
            .env(OWNER_ENV, &marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let result = has_owner(child.id(), &marker);
        child.kill().unwrap();
        child.wait().unwrap();
        // Current Darwin supplies argv only for this platform executable. If a
        // later OS supplies env too, the real marker is present. Neither case
        // permits a false negative ownership proof.
        assert!(!matches!(result, Ok(false)));
    }

    #[test]
    fn owned_fixture_exit_during_each_inspection_is_settled_without_scanning_other_pids() {
        let phases = [
            InspectionPhase::BeforeIdentity,
            InspectionPhase::BeforeEnvironment,
            InspectionPhase::BeforeIdentityFence,
            #[cfg(target_os = "linux")]
            InspectionPhase::AfterLinuxMetadata,
        ];
        for phase in phases {
            let marker = "d".repeat(64);
            let child = Command::new("/usr/bin/python3")
                .args(["-c", "import time;time.sleep(60)"])
                .env_clear()
                .env(OWNER_ENV, &marker)
                .process_group(0)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let pid = child.id();
            let child = std::sync::Arc::new(std::sync::Mutex::new(child));
            let ready = Instant::now() + Duration::from_secs(1);
            while !has_owner(pid, &marker).unwrap_or(false) && Instant::now() < ready {
                std::thread::sleep(Duration::from_millis(10));
            }
            let stopped = child.clone();
            INSPECTION_HOOK.with(|slot| {
                *slot.borrow_mut() = Some(InspectionHook {
                    pid,
                    phase,
                    run: Box::new(move || {
                        let mut child = stopped.lock().unwrap();
                        child.kill().unwrap();
                        child.wait().unwrap();
                    }),
                });
            });
            let result = remaining_from_pids(&marker, &[pid]);
            // Cleanup also covers assertion failures or a hook that did not run.
            let mut child = child.lock().unwrap();
            let _ = child.kill();
            let _ = child.wait();
            INSPECTION_HOOK.with(|slot| {
                assert!(
                    slot.borrow_mut().take().is_none(),
                    "inspection hook was not exercised: {phase:?}, {result:?}"
                );
            });
            assert!(result.unwrap().is_empty());
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn exit_during_audited_stop_never_signals_the_unrelated_fixture() {
        for phase in [
            InspectionPhase::BeforeSignalEnvironment,
            InspectionPhase::BeforeSignalAuditFence,
        ] {
            let marker = "e".repeat(64);
            let spawn = || {
                let mut child = Command::new("/usr/bin/python3")
                    .args(["-c", "import time;print('ready',flush=True);time.sleep(60)"])
                    .env_clear()
                    .env(OWNER_ENV, &marker)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                let mut ready = String::new();
                BufReader::new(child.stdout.take().unwrap())
                    .read_line(&mut ready)
                    .unwrap();
                assert_eq!(ready.trim(), "ready");
                child
            };
            let child = spawn();
            let mut unrelated = spawn();
            let unrelated_identity = Identity::read(unrelated.id()).unwrap();
            let pid = child.id();
            let child = std::sync::Arc::new(std::sync::Mutex::new(child));
            let ready = Instant::now() + Duration::from_secs(1);
            while !has_owner(pid, &marker).unwrap_or(false) && Instant::now() < ready {
                std::thread::sleep(Duration::from_millis(10));
            }
            let identity = Identity::read(pid).unwrap();
            let stopped = child.clone();
            INSPECTION_HOOK.with(|slot| {
                *slot.borrow_mut() = Some(InspectionHook {
                    pid,
                    phase,
                    run: Box::new(move || {
                        let mut child = stopped.lock().unwrap();
                        child.kill().unwrap();
                        child.wait().unwrap();
                    }),
                })
            });
            let result = signal_owned(&identity, &marker);
            let hook_exercised = INSPECTION_HOOK.with(|slot| slot.borrow_mut().take().is_none());
            let survived = unrelated_identity.still_matches().unwrap();
            let mut child = child.lock().unwrap();
            let _ = child.kill();
            let _ = child.wait();
            unrelated.kill().unwrap();
            unrelated.wait().unwrap();
            assert!(hook_exercised);
            assert!(result.is_ok());
            assert!(survived);
        }
    }
    #[test]
    fn detached_descendant_is_stopped_even_after_leader_and_pipes_exit() {
        let marker = "c".repeat(64);
        let python = "/usr/bin/python3";
        let code = "import os,time\nr,w=os.pipe()\npid=os.fork()\nif pid==0:\n os.close(r)\n os.setsid()\n os.close(0);os.close(1);os.close(2)\n os.write(w,b'1');os.close(w)\n time.sleep(60)\n os._exit(0)\nos.close(w);os.read(r,1);os.close(r)\nprint(pid,flush=True)\nos._exit(0)\n";
        let mut leader = Command::new(python)
            .args(["-c", code])
            .env_clear()
            .env(OWNER_ENV, &marker)
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut pid_line = String::new();
        BufReader::new(leader.stdout.take().unwrap())
            .read_line(&mut pid_line)
            .unwrap();
        let detached: u32 = pid_line.trim().parse().unwrap();
        struct Cleanup(Identity, String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                #[cfg(target_os = "macos")]
                {
                    let _ = signal_owned(&self.0, &self.1);
                }
                #[cfg(not(target_os = "macos"))]
                if self.0.matches_or_exited().unwrap_or(false)
                    && has_owner(self.0.pid, &self.1).unwrap_or(false)
                {
                    unsafe {
                        libc::kill(self.0.pid as i32, libc::SIGKILL);
                    }
                }
            }
        }
        let _cleanup = Cleanup(Identity::read(detached).unwrap(), marker.clone());
        leader.wait().unwrap();
        assert!(gone(leader.id()));
        assert!(unsafe { libc::kill(-(leader.id() as i32), 0) } < 0);
        assert!(has_owner(detached, &marker).unwrap());
        let owned = remaining_from_pids(&marker, &[detached]).unwrap();
        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0].pid, detached);
        with_test_pids(&[detached], || {
            assert_eq!(remaining(&marker).unwrap().len(), 1)
        });
        stop_from_pids(&marker, &[detached]).unwrap();
        assert!(remaining_from_pids(&marker, &[detached])
            .unwrap()
            .is_empty());
    }
}
