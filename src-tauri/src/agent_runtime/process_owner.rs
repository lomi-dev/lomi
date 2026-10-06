use crate::{shell::Profile as ShellProfile, terminal::Shells};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Child,
};
pub(crate) fn shell_profile<'a>(shells: &'a Shells, id: &str) -> Result<&'a ShellProfile, String> {
    let profile = shells.profiles.iter().find(|p| p.id == id && p.distro.is_none() && matches!(p.kind.as_str(), "bash" | "zsh" | "fish" | "sh")).ok_or("Choose an installed local shell. Native execution is not qualified for Windows or WSL.")?;
    if !cfg!(any(target_os = "macos", target_os = "linux")) {
        return Err("Native execution is not qualified on this platform.".into());
    }
    Ok(profile)
}

fn terminate(child: &mut Child) {
    #[cfg(unix)]
    {
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub(super) struct OwnedChild {
    child: Child,
    reaped: bool,
}
impl std::ops::Deref for OwnedChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.child
    }
}
impl std::ops::DerefMut for OwnedChild {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}
impl OwnedChild {
    pub(super) fn new(child: Child) -> Self {
        Self {
            child,
            reaped: false,
        }
    }
    pub(super) fn stop_group(&self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.id() as i32), libc::SIGKILL);
        }
    }
    pub(super) fn stop_and_wait(&mut self) -> Result<std::process::ExitStatus, String> {
        self.stop_group();
        let _ = self.child.kill();
        let status = self
            .child
            .wait()
            .map_err(|_| "Could not reap the CLI process.")?;
        self.reaped = true;
        Ok(status)
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.reaped {
            terminate(&mut self.child);
        }
    }
}

#[cfg(any(test, not(target_os = "macos")))]
pub(super) fn exit_pending(child: &Child) -> Result<bool, String> {
    #[cfg(unix)]
    {
        let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
        // Keep the leader unreaped so its PID cannot be reused before the owned
        // process group is stopped, including pipes held open by descendants.
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                child.id(),
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result != 0 {
            return Err("Could not verify CLI termination.".into());
        }
        let info = unsafe { info.assume_init() };
        Ok(unsafe { info.si_pid() } != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = child;
        Err("CLI process ownership is unqualified on this platform.".into())
    }
}

pub(crate) fn executable(path: &Path) -> Result<(PathBuf, u64, u64, u64, i64, i64), String> {
    let path = path
        .canonicalize()
        .map_err(|_| "Cannot resolve the native client executable.")?;
    let metadata =
        fs::symlink_metadata(&path).map_err(|_| "Cannot inspect the client executable.")?;
    if !metadata.is_file()
        || metadata.mode() & 0o022 != 0
        || ![0, unsafe { libc::geteuid() }].contains(&metadata.uid())
    {
        return Err("The native client executable is not privately owned or trusted.".into());
    }
    Ok((
        path,
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
    ))
}
